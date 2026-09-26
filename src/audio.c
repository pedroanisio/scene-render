#define _POSIX_C_SOURCE 200809L
#include "scene_render/audio.h"

#include "audio_internal.h"
#include "media_internal.h"

#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/channel_layout.h>
#include <libavutil/samplefmt.h>
#include <libswresample/swresample.h>

#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

__extension__ typedef unsigned __int128 SrU128;

uint64_t sr_frame_to_sample(uint64_t frame, uint32_t rate, uint32_t fps_num,
                            uint32_t fps_den) {
    if (!fps_num) return 0;
    SrU128 product = (SrU128)frame * rate * fps_den;
    SrU128 sample = product / fps_num;
    return sample > UINT64_MAX ? UINT64_MAX : (uint64_t)sample;
}

/* ---- decoding ----------------------------------------------------------- */

typedef struct {
    AVFormatContext *fmt;
    AVCodecContext *dec;
    SwrContext *swr;
    AVPacket *pkt;
    AVFrame *frame;
    int stream;
    uint32_t rate;
    uint32_t channels;
    float *pcm;
    int64_t samples;     /* per channel in pcm */
    int64_t capacity;    /* samples allocated in pcm */
    int64_t limit;       /* most samples accepted */
    bool placed;         /* first frame's timestamp has been applied */
    int64_t trim;        /* output samples still to drop (priming before t=0) */
    int64_t origin;      /* timeline origin in the stream time base */
    int64_t next_in;     /* timeline position of the next input sample, in
                            1/decoder-rate units; INT64_MIN = unknown */
    uint8_t **silence;   /* SILENCE_CHUNK input samples of silence */
} Decoder;

/* Timestamp jitter up to this is ignored (samples are concatenated); larger
 * gaps are filled with silence and larger overlaps trimmed. */
#define SR_AUDIO_JITTER_SECONDS 0.020
enum { SILENCE_CHUNK = 4096 };

static void set_err(char *err, size_t len, const char *what, int averr) {
    if (!err || !len) return;
    char message[AV_ERROR_MAX_STRING_SIZE] = "";
    if (averr < 0) av_strerror(averr, message, sizeof(message));
    snprintf(err, len, "%s%s%s", what, averr < 0 ? ": " : "", message);
}

/* The file's timeline origin (media_internal.h), shared with its video.
 * Without any stream start time, the earliest packet timestamp of the used
 * streams is found by demuxing once, and the file is then reopened. */
static int find_origin(Decoder *d, const char *path) {
    AVStream *st = d->fmt->streams[d->stream];
    int64_t origin_us = sr_media_origin_us(d->fmt);
    if (origin_us == AV_NOPTS_VALUE) {
        int video = av_find_best_stream(d->fmt, AVMEDIA_TYPE_VIDEO, -1, -1, NULL, 0);
        int rc = sr_media_scan_origin(d->fmt, d->pkt, d->stream, video, &origin_us);
        if (rc < 0) return rc;
        avformat_close_input(&d->fmt);
        rc = avformat_open_input(&d->fmt, path, NULL, NULL);
        if (rc >= 0) rc = avformat_find_stream_info(d->fmt, NULL);
        if (rc < 0) return rc;
        if (d->stream >= (int)d->fmt->nb_streams) return AVERROR_INVALIDDATA;
        st = d->fmt->streams[d->stream];
    }
    d->origin = sr_media_origin_in(st, origin_us);
    return 0;
}

static int open_all(Decoder *d, const char *path, const char **stage) {
    *stage = "cannot open file";
    int rc = avformat_open_input(&d->fmt, path, NULL, NULL);
    if (rc < 0) return rc;
    *stage = "cannot read stream information";
    rc = avformat_find_stream_info(d->fmt, NULL);
    if (rc < 0) return rc;
    *stage = "no audio stream";
    rc = av_find_best_stream(d->fmt, AVMEDIA_TYPE_AUDIO, -1, -1, NULL, 0);
    if (rc < 0) return rc;
    d->stream = rc;
    *stage = "cannot open decoder";
    const AVCodecParameters *par = d->fmt->streams[d->stream]->codecpar;
    const AVCodec *codec = avcodec_find_decoder(par->codec_id);
    if (!codec) return AVERROR_DECODER_NOT_FOUND;
    d->dec = avcodec_alloc_context3(codec);
    if (!d->dec) return AVERROR(ENOMEM);
    rc = avcodec_parameters_to_context(d->dec, par);
    if (rc < 0) return rc;
    /* Needed for the decoder to advance a frame's timestamp past samples it
     * skips (priming signalled as skip-samples side data, as in MP4). */
    d->dec->pkt_timebase = d->fmt->streams[d->stream]->time_base;
    d->dec->thread_count = 1;
    rc = avcodec_open2(d->dec, codec, NULL);
    if (rc < 0) return rc;
    *stage = "cannot set up resampling";
    AVChannelLayout out = {0}, in = {0};
    av_channel_layout_default(&out, (int)d->channels);
    if (d->dec->ch_layout.order == AV_CHANNEL_ORDER_UNSPEC) {
        av_channel_layout_default(&in, d->dec->ch_layout.nb_channels);
    } else {
        rc = av_channel_layout_copy(&in, &d->dec->ch_layout);
        if (rc < 0) return rc;
    }
    rc = swr_alloc_set_opts2(&d->swr, &out, AV_SAMPLE_FMT_FLT, (int)d->rate,
                             &in, d->dec->sample_fmt, d->dec->sample_rate, 0,
                             NULL);
    av_channel_layout_uninit(&in);
    av_channel_layout_uninit(&out);
    if (rc < 0) return rc;
    rc = swr_init(d->swr);
    if (rc < 0) return rc;
    *stage = "out of memory";
    d->pkt = av_packet_alloc();
    d->frame = av_frame_alloc();
    if (!d->pkt || !d->frame) return AVERROR(ENOMEM);
    *stage = "cannot read timestamps";
    return find_origin(d, path);
}

static int reserve(Decoder *d, int64_t samples) {
    if (samples > d->limit) return AVERROR(E2BIG);
    if (samples <= d->capacity) return 0;
    int64_t capacity = d->capacity ? d->capacity : 1 << 16;
    while (capacity < samples) capacity *= 2;
    float *pcm = realloc(d->pcm, (size_t)capacity * d->channels * sizeof(float));
    if (!pcm) return AVERROR(ENOMEM);
    d->pcm = pcm;
    d->capacity = capacity;
    return 0;
}

/* Resamples `count` input samples (in == NULL flushes) and appends the
 * result. */
static int append_input(Decoder *d, const uint8_t **in, int count) {
    int room = swr_get_out_samples(d->swr, in ? count : 0);
    if (room < 0) return room;
    int rc = reserve(d, d->samples + room);
    if (rc < 0) return rc;
    uint8_t *out = (uint8_t *)(d->pcm + d->samples * d->channels);
    int got = swr_convert(d->swr, &out, room, in, in ? count : 0);
    if (got < 0) return got;
    d->samples += got;
    return 0;
}

static int append(Decoder *d, const AVFrame *frame) {
    return frame ? append_input(d, (const uint8_t **)frame->extended_data,
                                frame->nb_samples)
                 : append_input(d, NULL, 0);
}

/* Feeds `count` input samples of silence through the resampler, so a
 * timestamp gap keeps its length on the output timeline. */
static int append_silence(Decoder *d, int64_t count) {
    int channels = d->dec->ch_layout.nb_channels;
    if (!d->silence) {
        int rc = av_samples_alloc_array_and_samples(&d->silence, NULL, channels,
                                                    SILENCE_CHUNK,
                                                    d->dec->sample_fmt, 0);
        if (rc < 0) return rc;
        av_samples_set_silence(d->silence, 0, SILENCE_CHUNK, channels,
                               d->dec->sample_fmt);
    }
    while (count > 0) {
        int chunk = count < SILENCE_CHUNK ? (int)count : SILENCE_CHUNK;
        int rc = append_input(d, (const uint8_t **)d->silence, chunk);
        if (rc < 0) return rc;
        count -= chunk;
    }
    return 0;
}

/* Appends the samples of `f` from input sample `skip` on. */
static int append_from(Decoder *d, const AVFrame *f, int skip) {
    if (skip <= 0) return append(d, f);
    if (skip >= f->nb_samples) return 0;
    int channels = f->ch_layout.nb_channels;
    int planes = av_sample_fmt_is_planar((enum AVSampleFormat)f->format) ? channels : 1;
    int step = av_get_bytes_per_sample((enum AVSampleFormat)f->format) *
               (planes == 1 ? channels : 1);
    const uint8_t **in = malloc((size_t)planes * sizeof(*in));
    if (!in) return AVERROR(ENOMEM);
    for (int p = 0; p < planes; ++p) in[p] = f->extended_data[p] + (size_t)skip * step;
    int rc = append_input(d, in, f->nb_samples - skip);
    free(in);
    return rc;
}

/* Clip sample k plays at k / rate seconds of the file's timeline (from the
 * origin shared with its video): a stream that starts late is preceded by
 * silence, and encoder priming placed before zero (AAC in Matroska) is
 * dropped. When the time base is too coarse to express the priming exactly
 * and the codec declares it, the declared padding is used. */
static int place(Decoder *d, const AVFrame *f) {
    d->placed = true;
    int64_t pts = f->best_effort_timestamp;
    if (pts == AV_NOPTS_VALUE) return 0;
    const AVStream *st = d->fmt->streams[d->stream];
    AVRational out_tb = {1, (int)d->rate};
    int64_t at = av_rescale_q_rnd(av_sat_sub64(pts, d->origin), st->time_base,
                                  out_tb, AV_ROUND_NEAR_INF | AV_ROUND_PASS_MINMAX);
    int64_t tick = av_rescale_q(1, st->time_base, out_tb);
    int pad = st->codecpar->initial_padding;
    if (at < 0 && pad > 0 && d->dec->sample_rate > 0) {
        int64_t declared = -av_rescale(pad, d->rate, d->dec->sample_rate);
        if (llabs(at - declared) <= tick) at = declared;
    }
    if (at < 0) {
        d->trim = -at;
        return 0;
    }
    int rc = reserve(d, at > 0 ? at : 1);
    if (rc < 0) return rc;
    memset(d->pcm, 0, (size_t)at * d->channels * sizeof(float));
    d->samples = at;
    return 0;
}

static void apply_trim(Decoder *d) {
    int64_t n = d->trim < d->samples ? d->trim : d->samples;
    memmove(d->pcm, d->pcm + n * d->channels,
            (size_t)(d->samples - n) * d->channels * sizeof(float));
    d->samples -= n;
    d->trim -= n;
}

/* Input-sample position of frame f on the timeline (1/decoder rate), or
 * INT64_MIN without a timestamp. */
static int64_t frame_position(const Decoder *d, const AVFrame *f) {
    int64_t pts = f->best_effort_timestamp;
    int rate = f->sample_rate > 0 ? f->sample_rate : d->dec->sample_rate;
    if (pts == AV_NOPTS_VALUE || rate <= 0) return INT64_MIN;
    return av_rescale_q_rnd(av_sat_sub64(pts, d->origin),
                            d->fmt->streams[d->stream]->time_base,
                            (AVRational){1, rate},
                            AV_ROUND_NEAR_INF | AV_ROUND_PASS_MINMAX);
}

/* Every frame after the first plays at its own timestamp: a gap beyond
 * SR_AUDIO_JITTER_SECONDS becomes silence, an overlap beyond it trims the
 * start of the new frame, and smaller jitter is ignored. Returns the input
 * samples of f to skip, or an error. */
static int64_t follow_timestamps(Decoder *d, const AVFrame *f) {
    int64_t at = frame_position(d, f);
    if (at == INT64_MIN || d->next_in == INT64_MIN) {
        if (d->next_in != INT64_MIN) d->next_in += f->nb_samples;
        return 0;
    }
    int rate = f->sample_rate > 0 ? f->sample_rate : d->dec->sample_rate;
    int64_t jitter = (int64_t)(SR_AUDIO_JITTER_SECONDS * rate);
    int64_t skip = 0;
    if (at - d->next_in > jitter) {
        int64_t gap = at - d->next_in;
        if (av_rescale(gap, d->rate, rate) > d->limit - d->samples)
            return AVERROR(E2BIG);   /* before feeding it all to swr */
        int rc = append_silence(d, gap);
        if (rc < 0) return rc;
        d->next_in = at;
    } else if (d->next_in - at > jitter) {
        skip = d->next_in - at;
        if (skip > f->nb_samples) skip = f->nb_samples;
    }
    d->next_in += f->nb_samples - skip;
    return skip;
}

static int drain(Decoder *d) {
    for (;;) {
        int rc = avcodec_receive_frame(d->dec, d->frame);
        if (rc == AVERROR(EAGAIN) || rc == AVERROR_EOF) return 0;
        if (rc < 0) return rc;
        int64_t skip = 0;
        if (!d->placed) {
            rc = place(d, d->frame);
            if (rc < 0) {
                av_frame_unref(d->frame);
                return rc;
            }
            int64_t at = frame_position(d, d->frame);
            d->next_in = at == INT64_MIN ? INT64_MIN : at + d->frame->nb_samples;
        } else {
            skip = follow_timestamps(d, d->frame);
            if (skip < 0) {
                av_frame_unref(d->frame);
                return (int)skip;
            }
        }
        rc = append_from(d, d->frame, (int)skip);
        if (rc == 0 && d->trim > 0) apply_trim(d);
        av_frame_unref(d->frame);
        if (rc < 0) return rc;
    }
}

static int decode_all(Decoder *d) {
    int rc;
    while ((rc = av_read_frame(d->fmt, d->pkt)) >= 0) {
        if (d->pkt->stream_index == d->stream) {
            rc = avcodec_send_packet(d->dec, d->pkt);
            if (rc >= 0) rc = drain(d);
        }
        av_packet_unref(d->pkt);
        if (rc < 0) return rc;
    }
    if (rc != AVERROR_EOF) return rc;
    rc = avcodec_send_packet(d->dec, NULL);
    if (rc < 0) return rc;
    rc = drain(d);
    if (rc < 0) return rc;
    rc = append(d, NULL);   /* resampler tail */
    if (rc == 0 && d->trim > 0) apply_trim(d);
    return rc;
}

SrStatus sr_audio_decode_file(const char *path, uint32_t rate,
                              uint32_t channels, double max_seconds,
                              float **pcm, uint64_t *samples, char *err,
                              size_t errlen) {
    if (pcm) *pcm = NULL;
    if (samples) *samples = 0;
    if (!path || !pcm || !samples || rate < 1 || rate > INT32_MAX ||
        channels < 1 || channels > 2 || !(max_seconds > 0.0)) {
        set_err(err, errlen, "invalid argument", 0);
        return SR_ERR_ARGUMENT;
    }
    Decoder d = {.rate = rate, .channels = channels,
                 .limit = (int64_t)(max_seconds * rate), .next_in = INT64_MIN};
    const char *stage = "";
    int rc = open_all(&d, path, &stage);
    if (rc >= 0) {
        stage = "cannot decode";
        rc = decode_all(&d);
        if (rc == AVERROR(E2BIG)) stage = "clip is longer than the 4-hour limit";
    }
    if (rc >= 0 && d.samples == 0) {
        stage = "no audio samples";
        rc = AVERROR_INVALIDDATA;
    }
    av_frame_free(&d.frame);
    av_packet_free(&d.pkt);
    if (d.silence) av_freep(&d.silence[0]);
    av_freep(&d.silence);
    swr_free(&d.swr);
    avcodec_free_context(&d.dec);
    avformat_close_input(&d.fmt);
    if (rc < 0) {
        set_err(err, errlen, stage, rc == AVERROR(E2BIG) ? 0 : rc);
        free(d.pcm);
        return rc == AVERROR(ENOMEM) ? SR_ERR_MEMORY : SR_ERR_ASSET;
    }
    *pcm = d.pcm;
    *samples = (uint64_t)d.samples;
    return SR_OK;
}

SrStatus sr_audio_load(SrScene *scene, SrDiagnostics *diag) {
    for (size_t i = 0; i < scene->audio.track_count; ++i) {
        SrAsset *asset = scene->audio.tracks[i].asset;
        if (!asset || asset->audio_decoded) continue;
        char *path = sr_path_join(scene->base_dir, asset->source);
        if (!path) return SR_ERR_MEMORY;
        char err[256] = "";
        sr_diag_info(diag, "decoding audio of asset '%s' from %s", asset->id, path);
        SrStatus status = sr_audio_decode_file(
            path, scene->audio.sample_rate, scene->audio.channels,
            SR_AUDIO_MAX_SECONDS, &asset->audio_pcm, &asset->audio_frames, err,
            sizeof(err));
        free(path);
        if (status != SR_OK) {
            sr_diag_error(diag, asset->source_line,
                          asset->type == SR_ASSET_VIDEO ? "video" : "audio",
                          "src", "cannot decode audio of asset '%s': %s",
                          asset->id, err);
            return status;
        }
        asset->audio_decoded = true;
    }
    return SR_OK;
}

/* ---- mixing ------------------------------------------------------------- */

typedef struct {
    const float *pcm;
    const SrAudioTrack *automation; /* borrowed, NULL for static gains */
    uint64_t start;       /* first output sample */
    uint64_t clip_first;  /* source sample range [clip_first, clip_end) */
    uint64_t clip_end;
    uint64_t span;
    uint64_t total;       /* output samples the track plays */
    uint64_t fade_in;     /* output samples */
    uint64_t fade_out;
    double speed;
    bool reverse;
    double volume;
    double gain[2];       /* volume x pan per output channel */
} SrVoice;

struct SrMixer {
    SrVoice *voices;
    size_t count;
    uint32_t channels;
    uint32_t sample_rate;
};

uint64_t sr_audio_seconds_to_samples(double seconds, uint32_t rate) {
    if (!(seconds > 0.0)) return 0;
    double value = seconds * rate;
    /* Saturate before llround, whose result must fit in a long long. */
    if (!(value < 0x1p63)) return UINT64_MAX;
    return (uint64_t)llround(value);
}

SrStatus sr_mixer_create(const SrScene *scene, SrMixer **out) {
    if (!scene || !out) return SR_ERR_ARGUMENT;
    *out = NULL;
    SrMixer *mixer = calloc(1, sizeof(*mixer));
    if (!mixer) return SR_ERR_MEMORY;
    mixer->channels = scene->audio.channels;
    mixer->sample_rate = scene->audio.sample_rate;
    if (scene->audio.track_count) {
        mixer->voices = calloc(scene->audio.track_count, sizeof(SrVoice));
        if (!mixer->voices) {
            free(mixer);
            return SR_ERR_MEMORY;
        }
    }
    const uint32_t rate = scene->audio.sample_rate;
    for (size_t i = 0; i < scene->audio.track_count; ++i) {
        const SrAudioTrack *track = &scene->audio.tracks[i];
        const SrAsset *asset = track->asset;
        if (!asset || !asset->audio_pcm || !asset->audio_frames) continue;
        SrVoice voice = {.pcm = asset->audio_pcm};
        if (track->volume.track.count || track->pan.track.count) {
            if (!mixer->sample_rate) {
                sr_mixer_destroy(mixer);
                return SR_ERR_ARGUMENT;
            }
            voice.automation = track;
        }
        voice.start = sr_audio_seconds_to_samples(track->start, rate);
        voice.clip_first = sr_audio_seconds_to_samples(track->clip_in, rate);
        voice.clip_end = track->clip_out >= 0.0
            ? sr_audio_seconds_to_samples(track->clip_out, rate) : asset->audio_frames;
        if (voice.clip_end > asset->audio_frames) voice.clip_end = asset->audio_frames;
        if (voice.clip_first >= voice.clip_end) continue;
        voice.span = voice.clip_end - voice.clip_first;
        uint64_t plays = track->loop_count > 0 ? (uint64_t)track->loop_count : 1;
        uint64_t source_total = plays > UINT64_MAX / voice.span
            ? UINT64_MAX : voice.span * plays;
        voice.speed = track->speed > 0.0 ? track->speed : 1.0;
        voice.reverse = track->reverse;
        if (voice.speed == 1.0) {
            voice.total = source_total;
        } else {
            /* Output samples l with l * speed < source_total. */
            double estimate = ceil((double)source_total / voice.speed);
            uint64_t total = estimate >= 1.8e19 ? UINT64_MAX : (uint64_t)estimate;
            while (total > 0 && (double)(total - 1) * voice.speed >= (double)source_total)
                --total;
            while (total < UINT64_MAX && (double)total * voice.speed < (double)source_total)
                ++total;
            voice.total = total;
        }
        voice.fade_in = sr_audio_seconds_to_samples(track->fade_in, rate);
        voice.fade_out = sr_audio_seconds_to_samples(track->fade_out, rate);
        voice.volume = track->volume.base;
        voice.gain[0] = voice.gain[1] = track->volume.base;
        if (mixer->channels == 2 && track->pan.base != 0.0) {
            /* Equal-power pan: gains sqrt(2)*cos/sin of (pan+1)*pi/4, so the
             * summed power of the two gains is constant, pan 0 is unity on
             * both channels, and a mono source (upmixed at -3 dB) panned
             * hard to one side reaches unity on that side. */
            double angle = (track->pan.base + 1.0) * SR_PI / 4.0;
            voice.gain[0] = track->volume.base * sqrt(2.0) * cos(angle);
            voice.gain[1] = track->volume.base * sqrt(2.0) * sin(angle);
        }
        mixer->voices[mixer->count++] = voice;
    }
    *out = mixer;
    return SR_OK;
}

void sr_mixer_destroy(SrMixer *mixer) {
    if (!mixer) return;
    free(mixer->voices);
    free(mixer);
}

/* Source value of channel c at output sample l (l < total) of a voice. */
static float voice_sample(const SrVoice *v, uint32_t channels, uint64_t l,
                          uint32_t c) {
    if (v->speed == 1.0) {
        uint64_t q = l % v->span;
        uint64_t index = v->reverse ? v->clip_end - 1 - q : v->clip_first + q;
        return v->pcm[index * channels + c];
    }
    /* Varispeed: linear interpolation between neighbouring source samples
     * (not band-limited: speeds above 1 can alias). */
    double q = fmod((double)l * v->speed, (double)v->span);
    double position = v->reverse ? (double)(v->clip_end - 1) - q
                                 : (double)v->clip_first + q;
    if (position < (double)v->clip_first) position = (double)v->clip_first;
    uint64_t j0 = (uint64_t)floor(position);
    if (j0 >= v->clip_end) j0 = v->clip_end - 1;
    uint64_t j1 = j0 + 1 < v->clip_end ? j0 + 1 : j0;
    float f = (float)(position - (double)j0);
    float a = v->pcm[j0 * channels + c], b = v->pcm[j1 * channels + c];
    return a + (b - a) * f;
}

void sr_mixer_mix(const SrMixer *mixer, uint64_t first, size_t count,
                  float *out) {
    const uint32_t channels = mixer->channels;
    memset(out, 0, count * channels * sizeof(float));
    for (size_t k = 0; k < mixer->count; ++k) {
        const SrVoice *v = &mixer->voices[k];
        for (size_t i = 0; i < count; ++i) {
            uint64_t s = first + i;
            if (s < v->start) continue;
            uint64_t l = s - v->start;
            if (l >= v->total) break;
            double fade = 1.0;
            if (v->fade_in && l < v->fade_in)
                fade *= (double)l / (double)v->fade_in;
            if (v->fade_out && v->total - l < v->fade_out)
                fade *= (double)(v->total - l) / (double)v->fade_out;
            double gains[2] = {v->gain[0], v->gain[1]};
            if (v->automation) {
                double time = (double)s / mixer->sample_rate;
                double volume = fmax(0.0, fmin(1.0,
                    sr_anim_eval(&v->automation->volume, time)));
                double pan = fmax(-1.0, fmin(1.0,
                    sr_anim_eval(&v->automation->pan, time)));
                gains[0] = gains[1] = volume;
                if (channels == 2 && pan != 0.0) {
                    double angle = (pan + 1.0) * SR_PI / 4.0;
                    gains[0] = volume * sqrt(2.0) * cos(angle);
                    gains[1] = volume * sqrt(2.0) * sin(angle);
                }
            }
            for (uint32_t c = 0; c < channels; ++c) {
                float gain = (float)(fade == 1.0 ? gains[c] : gains[c] * fade);
                out[i * channels + c] += voice_sample(v, channels, l, c) * gain;
            }
        }
    }
    for (size_t i = 0; i < count * channels; ++i)
        out[i] = fmaxf(-1.0f, fminf(1.0f, out[i]));
}
