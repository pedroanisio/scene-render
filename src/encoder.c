#define _POSIX_C_SOURCE 200809L
#include "scene_render/encoder.h"
#include "scene_render/parallel.h"

#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/channel_layout.h>
#include <libavutil/opt.h>
#include <libavutil/pixdesc.h>
#include <libavutil/spherical.h>
#include <libswresample/swresample.h>
#include <libswscale/swscale.h>

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>

struct SrEncoder {
    AVFormatContext *fmt;
    AVPacket *pkt;
    /* Video. */
    AVCodecContext *video;
    AVStream *vstream;
    struct SwsContext *sws;
    AVFrame *frame;
    int64_t next_pts;
    unsigned bits;
    uint32_t width, height;
    /* Audio: interleaved float is staged until one codec frame is full,
     * then converted to the codec's sample format by swresample. */
    AVCodecContext *audio;
    AVStream *astream;
    AVFrame *aframe;
    SwrContext *swr;
    float *stage;
    int staged;
    int stage_capacity;
    int64_t next_apts;
    uint32_t channels;
    bool header_written;
    bool finished;
    double seconds;
};

static SrStatus av_fail(SrDiagnostics *diag, int rc, const char *what) {
    char message[AV_ERROR_MAX_STRING_SIZE] = "";
    if (rc < 0) av_strerror(rc, message, sizeof(message));
    sr_diag_error(diag, 0, NULL, NULL, "%s%s%s", what, rc < 0 ? ": " : "",
                  message);
    return rc == AVERROR(ENOMEM) ? SR_ERR_MEMORY : SR_ERR_ENCODER;
}

static const char *video_encoder_name(SrCodec codec) {
    switch (codec) {
    case SR_CODEC_H265: return "libx265";
    case SR_CODEC_FFV1: return "ffv1";
    case SR_CODEC_H264:
    default: return "libx264";
    }
}

bool sr_encoder_available(const char *name) {
    return name && avcodec_find_encoder_by_name(name) != NULL;
}

unsigned sr_encoder_input_bits(const char *pixel_format) {
    enum AVPixelFormat format = pixel_format ? av_get_pix_fmt(pixel_format)
                                             : AV_PIX_FMT_NONE;
    const AVPixFmtDescriptor *desc = av_pix_fmt_desc_get(format);
    if (!desc) return 8;
    for (int i = 0; i < desc->nb_components; ++i)
        if (desc->comp[i].depth > 8) return 16;
    return 8;
}

/* Supported pixel/sample formats of an encoder; NULL = anything. */
static const void *supported(const AVCodec *codec, int which) {
#if LIBAVCODEC_VERSION_INT >= AV_VERSION_INT(61, 13, 100)
    const void *list = NULL;
    int count = 0;
    enum AVCodecConfig config = which == 0 ? AV_CODEC_CONFIG_PIX_FORMAT
                              : which == 1 ? AV_CODEC_CONFIG_SAMPLE_FORMAT
                                           : AV_CODEC_CONFIG_SAMPLE_RATE;
    if (avcodec_get_supported_config(NULL, codec, config, 0, &list, &count) < 0)
        return NULL;
    return list;
#else
    return which == 0 ? (const void *)codec->pix_fmts
         : which == 1 ? (const void *)codec->sample_fmts
                      : (const void *)codec->supported_samplerates;
#endif
}

static bool pixel_format_supported(const AVCodec *codec,
                                   enum AVPixelFormat format,
                                   char *list_text, size_t capacity) {
    const enum AVPixelFormat *list = supported(codec, 0);
    if (!list) return true;
    bool found = false;
    size_t used = 0;
    list_text[0] = '\0';
    for (size_t i = 0; list[i] != AV_PIX_FMT_NONE; ++i) {
        if (list[i] == format) found = true;
        const char *name = av_get_pix_fmt_name(list[i]);
        if (name && used + strlen(name) + 2 < capacity) {
            used += (size_t)snprintf(list_text + used, capacity - used, "%s%s",
                                     used ? " " : "", name);
        }
    }
    return found;
}

static const char *container_for(const char *path) {
    const char *dot = strrchr(path, '.');
    const char *slash = strrchr(path, '/');
    if (!dot || (slash && dot < slash)) return NULL;
    if (!strcasecmp(dot, ".mp4")) return "mp4";
    if (!strcasecmp(dot, ".mov")) return "mov";
    if (!strcasecmp(dot, ".mkv")) return "matroska";
    return NULL;
}

/* Tags and Y'CbCr matrix per output color space; the same values the
 * FFmpeg CLI arguments of earlier releases produced. */
static void color_tags(SrColorSpace space, AVCodecContext *c, int *sws_matrix) {
    switch (space) {
    case SR_COLOR_DISPLAY_P3:
        c->color_primaries = AVCOL_PRI_SMPTE432;
        c->color_trc = AVCOL_TRC_IEC61966_2_1;
        c->colorspace = AVCOL_SPC_BT709;
        *sws_matrix = SWS_CS_ITU709;
        break;
    case SR_COLOR_REC2020:
        c->color_primaries = AVCOL_PRI_BT2020;
        c->color_trc = AVCOL_TRC_BT2020_10;
        c->colorspace = AVCOL_SPC_BT2020_NCL;
        *sws_matrix = SWS_CS_BT2020;
        break;
    case SR_COLOR_REC709:
        c->color_primaries = AVCOL_PRI_BT709;
        c->color_trc = AVCOL_TRC_BT709;
        c->colorspace = AVCOL_SPC_BT709;
        *sws_matrix = SWS_CS_ITU709;
        break;
    case SR_COLOR_SRGB:
    default:
        c->color_primaries = AVCOL_PRI_BT709;
        c->color_trc = AVCOL_TRC_IEC61966_2_1;
        c->colorspace = AVCOL_SPC_BT709;
        *sws_matrix = SWS_CS_ITU709;
        break;
    }
}

static SrStatus open_video(SrEncoder *e, const SrScene *scene, unsigned threads,
                           SrDiagnostics *diag) {
    const SrOutput *output = &scene->output;
    const char *name = video_encoder_name(output->codec);
    const AVCodec *codec = avcodec_find_encoder_by_name(name);
    if (!codec) {
        sr_diag_error(diag, output->source_line, "output", "codec",
                      "libavcodec encoder '%s' is unavailable", name);
        return SR_ERR_ENCODER;
    }
    enum AVPixelFormat format = av_get_pix_fmt(output->pixel_format);
    char formats[1024];
    if (format == AV_PIX_FMT_NONE) {
        sr_diag_error(diag, output->source_line, "output", "pixelFormat",
                      "unknown pixel format '%s'", output->pixel_format);
        return SR_ERR_ENCODER;
    }
    if (!pixel_format_supported(codec, format, formats, sizeof(formats))) {
        sr_diag_error(diag, output->source_line, "output", "pixelFormat",
                      "encoder '%s' does not support pixel format '%s'; "
                      "supported: %s", name, output->pixel_format, formats);
        return SR_ERR_ENCODER;
    }
    e->bits = sr_encoder_input_bits(output->pixel_format);
    AVCodecContext *c = avcodec_alloc_context3(codec);
    if (!c) return av_fail(diag, AVERROR(ENOMEM), "cannot allocate video encoder");
    e->video = c;
    c->width = (int)scene->project.width;
    c->height = (int)scene->project.height;
    c->time_base = (AVRational){(int)scene->project.fps_den,
                                (int)scene->project.fps_num};
    c->framerate = (AVRational){(int)scene->project.fps_num,
                                (int)scene->project.fps_den};
    c->pix_fmt = format;
    c->thread_count = (int)threads;
    c->color_range = output->full_range ? AVCOL_RANGE_JPEG : AVCOL_RANGE_MPEG;
    int matrix;
    color_tags(output->color_space, c, &matrix);
    c->flags |= AV_CODEC_FLAG_BITEXACT;
    if (e->fmt->oformat->flags & AVFMT_GLOBALHEADER)
        c->flags |= AV_CODEC_FLAG_GLOBAL_HEADER;
    if (output->codec == SR_CODEC_FFV1) {
        /* Version 3 with per-slice CRCs: the threaded, checksummed layout. */
        c->level = 3;
        av_opt_set_int(c->priv_data, "slicecrc", 1, 0);
    } else {
        av_opt_set(c->priv_data, "preset", output->preset, 0);
        if (output->bitrate)
            c->bit_rate = (int64_t)output->bitrate;
        else
            av_opt_set_int(c->priv_data, "crf", output->crf, 0);
        if (output->codec == SR_CODEC_H265) {
            /* A fixed pool size and one frame thread keep x265 output
             * independent of scheduling. */
            char params[96];
            snprintf(params, sizeof(params),
                     "pools=%u:frame-threads=1:log-level=%s", threads,
                     diag && diag->verbose ? "info" : "error");
            av_opt_set(c->priv_data, "x265-params", params, 0);
        }
    }
    int rc = avcodec_open2(c, codec, NULL);
    if (rc < 0) {
        char what[160];
        snprintf(what, sizeof(what), "cannot open video encoder '%s'", name);
        return av_fail(diag, rc, what);
    }
    e->sws = sws_getContext(c->width, c->height,
                            e->bits == 16 ? AV_PIX_FMT_RGBA64 : AV_PIX_FMT_RGBA,
                            c->width, c->height, c->pix_fmt,
                            SWS_BICUBIC | SWS_ACCURATE_RND |
                                SWS_FULL_CHR_H_INT | SWS_BITEXACT,
                            NULL, NULL, NULL);
    if (!e->sws) return av_fail(diag, 0, "cannot set up RGB to Y'CbCr conversion");
    const int *coefficients = sws_getCoefficients(matrix);
    /* Source is full-range RGB; the destination range follows colorRange.
     * The return value only reports "not a YUV destination", harmless. */
    (void)sws_setColorspaceDetails(e->sws, coefficients, 1, coefficients,
                                   output->full_range ? 1 : 0, 0, 1 << 16,
                                   1 << 16);
    e->frame = av_frame_alloc();
    if (!e->frame) return av_fail(diag, AVERROR(ENOMEM), "cannot allocate frame");
    e->frame->format = c->pix_fmt;
    e->frame->width = c->width;
    e->frame->height = c->height;
    e->frame->color_range = c->color_range;
    e->frame->colorspace = c->colorspace;
    e->frame->color_primaries = c->color_primaries;
    e->frame->color_trc = c->color_trc;
    rc = av_frame_get_buffer(e->frame, 0);
    if (rc < 0) return av_fail(diag, rc, "cannot allocate frame");
    return SR_OK;
}

static SrStatus open_audio(SrEncoder *e, const SrScene *scene,
                           const SrEncoderAudio *audio, SrDiagnostics *diag) {
    const SrOutput *output = &scene->output;
    const char *name = output->audio_codec && *output->audio_codec
                           ? output->audio_codec : "aac";
    const AVCodec *codec = avcodec_find_encoder_by_name(name);
    if (!codec) {
        sr_diag_error(diag, output->source_line, "output", "audioCodec",
                      "libavcodec encoder '%s' is unavailable", name);
        return SR_ERR_ENCODER;
    }
    if (codec->type != AVMEDIA_TYPE_AUDIO) {
        sr_diag_error(diag, output->source_line, "output", "audioCodec",
                      "'%s' is not an audio encoder", name);
        return SR_ERR_ENCODER;
    }
    const int *rates = supported(codec, 2);
    if (rates) {
        bool found = false;
        for (size_t i = 0; rates[i]; ++i) found |= rates[i] == (int)audio->sample_rate;
        if (!found) {
            sr_diag_error(diag, output->source_line, "output", "audioCodec",
                          "encoder '%s' does not support %u Hz", name,
                          audio->sample_rate);
            return SR_ERR_ENCODER;
        }
    }
    AVCodecContext *a = avcodec_alloc_context3(codec);
    if (!a) return av_fail(diag, AVERROR(ENOMEM), "cannot allocate audio encoder");
    e->audio = a;
    const enum AVSampleFormat *formats = supported(codec, 1);
    a->sample_fmt = AV_SAMPLE_FMT_FLTP;
    if (formats) {
        a->sample_fmt = formats[0];
        for (size_t i = 0; formats[i] != AV_SAMPLE_FMT_NONE; ++i)
            if (formats[i] == AV_SAMPLE_FMT_FLTP) a->sample_fmt = formats[i];
    }
    a->sample_rate = (int)audio->sample_rate;
    av_channel_layout_default(&a->ch_layout, (int)audio->channels);
    a->bit_rate = (int64_t)output->audio_bitrate;
    a->time_base = (AVRational){1, (int)audio->sample_rate};
    a->flags |= AV_CODEC_FLAG_BITEXACT;
    if (e->fmt->oformat->flags & AVFMT_GLOBALHEADER)
        a->flags |= AV_CODEC_FLAG_GLOBAL_HEADER;
    int rc = avcodec_open2(a, codec, NULL);
    if (rc < 0) {
        char what[160];
        snprintf(what, sizeof(what), "cannot open audio encoder '%s'", name);
        return av_fail(diag, rc, what);
    }
    e->channels = audio->channels;
    e->stage_capacity = a->frame_size > 0 &&
                        !(codec->capabilities & AV_CODEC_CAP_VARIABLE_FRAME_SIZE)
                            ? a->frame_size : 1024;
    rc = swr_alloc_set_opts2(&e->swr, &a->ch_layout, a->sample_fmt,
                             a->sample_rate, &a->ch_layout, AV_SAMPLE_FMT_FLT,
                             a->sample_rate, 0, NULL);
    if (rc >= 0) rc = swr_init(e->swr);
    if (rc < 0) return av_fail(diag, rc, "cannot set up audio sample conversion");
    e->aframe = av_frame_alloc();
    e->stage = sr_alloc((size_t)e->stage_capacity * audio->channels * sizeof(float));
    if (!e->aframe || !e->stage)
        return av_fail(diag, AVERROR(ENOMEM), "cannot allocate audio frame");
    e->aframe->format = a->sample_fmt;
    e->aframe->nb_samples = e->stage_capacity;
    e->aframe->sample_rate = a->sample_rate;
    rc = av_channel_layout_copy(&e->aframe->ch_layout, &a->ch_layout);
    if (rc >= 0) rc = av_frame_get_buffer(e->aframe, 0);
    if (rc < 0) return av_fail(diag, rc, "cannot allocate audio frame");
    return SR_OK;
}

static SrStatus add_spherical(SrEncoder *e, SrDiagnostics *diag) {
    size_t size = 0;
    AVSphericalMapping *map = av_spherical_alloc(&size);
    if (!map) return av_fail(diag, AVERROR(ENOMEM), "cannot allocate spherical metadata");
    map->projection = AV_SPHERICAL_EQUIRECTANGULAR;
#if LIBAVCODEC_VERSION_INT >= AV_VERSION_INT(60, 31, 102)
    AVCodecParameters *par = e->vstream->codecpar;
    if (!av_packet_side_data_add(&par->coded_side_data, &par->nb_coded_side_data,
                                 AV_PKT_DATA_SPHERICAL, map, size, 0)) {
        av_free(map);
        return av_fail(diag, AVERROR(ENOMEM), "cannot attach spherical metadata");
    }
#else
    int rc = av_stream_add_side_data(e->vstream, AV_PKT_DATA_SPHERICAL,
                                     (uint8_t *)map, size);
    if (rc < 0) {
        av_free(map);
        return av_fail(diag, rc, "cannot attach spherical metadata");
    }
#endif
    /* MP4 sv3d/st3d are written only at "unofficial" compliance (they are
     * not part of ISO BMFF); Matroska writes its Projection element anyway. */
    e->fmt->strict_std_compliance = FF_COMPLIANCE_UNOFFICIAL;
    return SR_OK;
}

static SrStatus open_streams(SrEncoder *e, const SrScene *scene,
                             const char *path, const char *container,
                             SrDiagnostics *diag) {
    e->vstream = avformat_new_stream(e->fmt, NULL);
    if (!e->vstream) return av_fail(diag, AVERROR(ENOMEM), "cannot add video stream");
    e->vstream->time_base = e->video->time_base;
    e->vstream->avg_frame_rate = e->video->framerate;
    int rc = avcodec_parameters_from_context(e->vstream->codecpar, e->video);
    if (rc < 0) return av_fail(diag, rc, "cannot configure video stream");
    if (scene->output.codec == SR_CODEC_H265 && strcmp(container, "matroska"))
        e->vstream->codecpar->codec_tag = MKTAG('h', 'v', 'c', '1');
    if (scene->project.mode == SR_MODE_EQUIRECTANGULAR &&
        scene->output.spherical_metadata) {
        SrStatus status = add_spherical(e, diag);
        if (status != SR_OK) return status;
    }
    if (e->audio) {
        e->astream = avformat_new_stream(e->fmt, NULL);
        if (!e->astream) return av_fail(diag, AVERROR(ENOMEM), "cannot add audio stream");
        e->astream->time_base = e->audio->time_base;
        rc = avcodec_parameters_from_context(e->astream->codecpar, e->audio);
        if (rc < 0) return av_fail(diag, rc, "cannot configure audio stream");
    }
    rc = avio_open(&e->fmt->pb, path, AVIO_FLAG_WRITE);
    if (rc < 0) {
        char message[AV_ERROR_MAX_STRING_SIZE];
        av_strerror(rc, message, sizeof(message));
        sr_diag_error(diag, 0, NULL, NULL, "cannot open output file '%s': %s",
                      path, message);
        return rc == AVERROR(ENOMEM) ? SR_ERR_MEMORY : SR_ERR_IO;
    }
    AVDictionary *options = NULL;
    if (strcmp(container, "matroska"))
        av_dict_set(&options, "movflags", "+faststart", 0);
    rc = avformat_write_header(e->fmt, &options);
    av_dict_free(&options);
    if (rc < 0) return av_fail(diag, rc, "cannot write container header");
    e->header_written = true;
    return SR_OK;
}

SrStatus sr_encoder_open(SrEncoder **out, const SrScene *scene,
                         const char *path, unsigned threads,
                         const SrEncoderAudio *audio, SrDiagnostics *diag) {
    if (!out) return SR_ERR_ARGUMENT;
    *out = NULL;
    if (!scene || !path || !scene->project.width || !scene->project.height ||
        scene->project.width > INT32_MAX || scene->project.height > INT32_MAX ||
        !scene->project.fps_num || !scene->project.fps_den ||
        scene->project.fps_num > INT32_MAX || scene->project.fps_den > INT32_MAX ||
        !scene->output.pixel_format || !scene->output.preset ||
        (audio && audio->channels &&
         (audio->channels > 2 || audio->sample_rate < 8000 ||
          audio->sample_rate > INT32_MAX)))
        return SR_ERR_ARGUMENT;
    const char *container = container_for(path);
    if (!container) {
        sr_diag_error(diag, 0, NULL, NULL,
                      "output '%s' needs a .mp4, .mov or .mkv extension", path);
        return SR_ERR_ARGUMENT;
    }
    if (scene->output.codec == SR_CODEC_FFV1 && strcmp(container, "matroska")) {
        sr_diag_error(diag, scene->output.source_line, "output", "codec",
                      "ffv1 requires a .mkv (Matroska) output");
        return SR_ERR_ARGUMENT;
    }
    if (threads == 0) threads = sr_parallel_thread_count(0, SIZE_MAX);
    SrEncoder *e = calloc(1, sizeof(*e));
    if (!e) return SR_ERR_MEMORY;
    e->width = scene->project.width;
    e->height = scene->project.height;
    SrStatus status = SR_OK;
    int rc = avformat_alloc_output_context2(&e->fmt, NULL, container, path);
    if (rc < 0 || !e->fmt)
        status = av_fail(diag, rc < 0 ? rc : AVERROR(ENOMEM),
                         "cannot allocate output container");
    if (status == SR_OK) {
        e->fmt->flags |= AVFMT_FLAG_BITEXACT;
        e->pkt = av_packet_alloc();
        if (!e->pkt) status = av_fail(diag, AVERROR(ENOMEM), "cannot allocate packet");
    }
    if (status == SR_OK) status = open_video(e, scene, threads, diag);
    if (status == SR_OK && audio && audio->channels)
        status = open_audio(e, scene, audio, diag);
    if (status == SR_OK) status = open_streams(e, scene, path, container, diag);
    if (status != SR_OK) {
        sr_encoder_destroy(e);
        return status;
    }
    sr_diag_info(diag, "encoding in-process: %s %s -> %s", e->video->codec->name,
                 scene->output.pixel_format, container);
    *out = e;
    return SR_OK;
}

unsigned sr_encoder_bits(const SrEncoder *encoder) {
    return encoder ? encoder->bits : 8;
}

double sr_encoder_seconds(const SrEncoder *encoder) {
    return encoder ? encoder->seconds : 0.0;
}

const char *sr_encoder_name(const SrEncoder *encoder) {
    return encoder && encoder->video && encoder->video->codec
               ? encoder->video->codec->name : "none";
}

/* Writes every packet the codec has ready. Video packets last one frame:
 * encoders leave the duration unset, and muxers then shorten the last one. */
static int drain(SrEncoder *e, AVCodecContext *codec, AVStream *stream) {
    for (;;) {
        int rc = avcodec_receive_packet(codec, e->pkt);
        if (rc == AVERROR(EAGAIN) || rc == AVERROR_EOF) return 0;
        if (rc < 0) return rc;
        if (codec == e->video) e->pkt->duration = 1;
        av_packet_rescale_ts(e->pkt, codec->time_base, stream->time_base);
        e->pkt->stream_index = stream->index;
        rc = av_interleaved_write_frame(e->fmt, e->pkt);
        if (rc < 0) return rc;
    }
}

SrStatus sr_encoder_write_video(SrEncoder *e, const void *rgba,
                                SrDiagnostics *diag) {
    if (!e || e->finished || !rgba) return SR_ERR_ARGUMENT;
    double start = sr_monotonic_seconds();
    int rc = av_frame_make_writable(e->frame);
    if (rc < 0) return av_fail(diag, rc, "cannot prepare video frame");
    const uint8_t *source[4] = {rgba, NULL, NULL, NULL};
    const int stride[4] = {(int)(e->width * 4 * (e->bits / 8)), 0, 0, 0};
    sws_scale(e->sws, source, stride, 0, (int)e->height, e->frame->data,
              e->frame->linesize);
    e->frame->pts = e->next_pts++;
    rc = avcodec_send_frame(e->video, e->frame);
    if (rc >= 0) rc = drain(e, e->video, e->vstream);
    e->seconds += sr_monotonic_seconds() - start;
    return rc < 0 ? av_fail(diag, rc, "video encoding failed") : SR_OK;
}

/* Encodes the staged samples as one audio frame (the last may be short). */
static int flush_stage(SrEncoder *e) {
    int rc = av_frame_make_writable(e->aframe);
    if (rc < 0) return rc;
    const uint8_t *input[1] = {(const uint8_t *)e->stage};
    rc = swr_convert(e->swr, e->aframe->data, e->staged, input, e->staged);
    if (rc < 0) return rc;
    e->aframe->nb_samples = e->staged;
    e->aframe->pts = e->next_apts;
    e->next_apts += e->staged;
    e->staged = 0;
    rc = avcodec_send_frame(e->audio, e->aframe);
    return rc < 0 ? rc : drain(e, e->audio, e->astream);
}

SrStatus sr_encoder_write_audio(SrEncoder *e, const float *pcm, size_t samples,
                                SrDiagnostics *diag) {
    if (!e || e->finished || !e->audio || (!pcm && samples)) return SR_ERR_ARGUMENT;
    double start = sr_monotonic_seconds();
    size_t channels = e->channels;
    for (size_t i = 0; i < samples;) {
        size_t room = (size_t)(e->stage_capacity - e->staged);
        size_t count = samples - i < room ? samples - i : room;
        float *stage = e->stage + (size_t)e->staged * channels;
        const float *in = pcm + i * channels;
        for (size_t k = 0; k < count * channels; ++k)
            stage[k] = in[k] > 1.0f ? 1.0f : (in[k] < -1.0f ? -1.0f : in[k]);
        e->staged += (int)count;
        i += count;
        if (e->staged == e->stage_capacity) {
            int rc = flush_stage(e);
            if (rc < 0) {
                e->seconds += sr_monotonic_seconds() - start;
                return av_fail(diag, rc, "audio encoding failed");
            }
        }
    }
    e->seconds += sr_monotonic_seconds() - start;
    return SR_OK;
}

SrStatus sr_encoder_finish(SrEncoder *e, SrDiagnostics *diag) {
    if (!e || e->finished) return SR_ERR_ARGUMENT;
    double start = sr_monotonic_seconds();
    e->finished = true;
    int rc = avcodec_send_frame(e->video, NULL);
    if (rc >= 0) rc = drain(e, e->video, e->vstream);
    if (rc < 0) return av_fail(diag, rc, "cannot flush video encoder");
    if (e->audio) {
        if (e->staged > 0) rc = flush_stage(e);
        if (rc >= 0) rc = avcodec_send_frame(e->audio, NULL);
        if (rc >= 0) rc = drain(e, e->audio, e->astream);
        if (rc < 0) return av_fail(diag, rc, "cannot flush audio encoder");
    }
    rc = av_write_trailer(e->fmt);
    if (rc < 0) return av_fail(diag, rc, "cannot finish container");
    rc = avio_closep(&e->fmt->pb);
    e->seconds += sr_monotonic_seconds() - start;
    if (rc < 0) {
        av_fail(diag, rc, "cannot close output file");
        return SR_ERR_IO;
    }
    return SR_OK;
}

void sr_encoder_destroy(SrEncoder *e) {
    if (!e) return;
    if (e->fmt && e->fmt->pb) avio_closep(&e->fmt->pb);
    avformat_free_context(e->fmt);
    avcodec_free_context(&e->video);
    avcodec_free_context(&e->audio);
    sws_freeContext(e->sws);
    swr_free(&e->swr);
    av_frame_free(&e->frame);
    av_frame_free(&e->aframe);
    av_packet_free(&e->pkt);
    free(e->stage);
    free(e);
}
