#define _POSIX_C_SOURCE 200809L
#include "scene_render/video.h"
#include "scene_render/color.h"

#include "media_internal.h"
#include "video_internal.h"

#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/pixdesc.h>
#include <libswscale/swscale.h>

#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* Decoding forward is cheaper than seeking for short gaps; beyond this many
 * frames the source seeks to the nearest earlier keyframe instead. */
enum { FORWARD_DECODE_LIMIT = 48 };

__extension__ typedef __int128 SrI128;

typedef struct {
    int64_t index;        /* -1 = empty */
    uint64_t stamp;       /* last use, for LRU eviction */
    SrImage image;
} CacheSlot;

struct SrVideoSource {
    AVFormatContext *fmt;
    AVCodecContext *dec;
    AVPacket *pkt;
    AVFrame *frame;       /* decoder output */
    AVFrame *prev;        /* latest decoded frame shown at or before the target */
    AVFrame *ahead;       /* decoded frame first shown after the target */
    int64_t prev_index;   /* first output index of prev / ahead */
    int64_t ahead_index;
    struct SwsContext *sws;
    int sws_key[5];
    uint8_t *rgba;        /* width * height * 4 conversion buffer */
    const SrProject *project;
    SrColorSpace source_space;
    char *path;           /* reopened to rewind a stream without timestamps */
    const char *failure;  /* stage of the last frame failure, or NULL */
    int stream;
    AVRational tb;        /* stream time base */
    AVRational frame_tb;  /* 1 / declared fps */
    int64_t origin;       /* timeline origin in tb (media_internal.h) */
    int64_t first_pts;    /* earliest frame pts */
    SrI128 eps_num;       /* selection tolerance, in frames */
    SrI128 eps_den;
    int64_t counted;      /* frames decoded since the start (no timestamps) */
    bool no_timestamps;   /* no packet timestamps: count frames, never seek */
    bool positioned;      /* decoder position follows prev/ahead */
    bool at_start;        /* decoding began at the first frame */
    bool matrix_warned;
    CacheSlot *slots;
    size_t slot_count;
    uint64_t clock;
    SrVideoInfo info;
    SrVideoStats stats;
};

static void set_err(char *err, size_t len, const char *what, int averr) {
    if (!err || !len) return;
    char message[AV_ERROR_MAX_STRING_SIZE] = "";
    if (averr < 0) av_strerror(averr, message, sizeof(message));
    snprintf(err, len, "%s%s%s", what, averr < 0 ? ": " : "", message);
}

static SrStatus averr_status(int rc) {
    return rc == AVERROR(ENOMEM) ? SR_ERR_MEMORY : SR_ERR_ASSET;
}

/* Deprecated full-range "J" formats become their plain twins + full range. */
static enum AVPixelFormat plain_format(enum AVPixelFormat format, bool *full) {
    switch (format) {
    case AV_PIX_FMT_YUVJ420P: *full = true; return AV_PIX_FMT_YUV420P;
    case AV_PIX_FMT_YUVJ422P: *full = true; return AV_PIX_FMT_YUV422P;
    case AV_PIX_FMT_YUVJ444P: *full = true; return AV_PIX_FMT_YUV444P;
    case AV_PIX_FMT_YUVJ440P: *full = true; return AV_PIX_FMT_YUV440P;
    case AV_PIX_FMT_YUVJ411P: *full = true; return AV_PIX_FMT_YUV411P;
    default: return format;
    }
}

int sr_video_sws_matrix(int colorspace, int height, bool *approximated) {
    if (approximated) *approximated = false;
    switch (colorspace) {
    case AVCOL_SPC_BT709: return SWS_CS_ITU709;
    case AVCOL_SPC_UNSPECIFIED: return height >= 720 ? SWS_CS_ITU709 : SWS_CS_ITU601;
    case AVCOL_SPC_SMPTE240M: return SWS_CS_SMPTE240M;
    case AVCOL_SPC_FCC: return SWS_CS_FCC;
    case AVCOL_SPC_BT2020_NCL: return SWS_CS_BT2020;
    case AVCOL_SPC_BT2020_CL:
        /* swscale has no constant-luminance path: the non-constant matrix
         * is the closest it offers. */
        if (approximated) *approximated = true;
        return SWS_CS_BT2020;
    default: return SWS_CS_DEFAULT;   /* BT.601 (BT.470BG, SMPTE 170M) */
    }
}

/* Builds (or reuses) a scaler from frame f to width x height RGBA using the
 * frame's own matrix and range. Untagged streams follow the usual
 * convention: BT.709 from 720 lines up, BT.601 below. */
static int build_scaler(struct SwsContext **sws, int *cached, const AVFrame *f,
                        int width, int height, int flags, bool *approximated) {
    bool full = f->color_range == AVCOL_RANGE_JPEG;
    enum AVPixelFormat format = plain_format((enum AVPixelFormat)f->format, &full);
    int space = f->colorspace;
    int matrix = sr_video_sws_matrix(space, f->height, approximated);
    int key[5] = {format, full, space, f->width, f->height};
    if (*sws && !memcmp(cached, key, sizeof(key))) return 0;
    sws_freeContext(*sws);
    memset(cached, 0, sizeof(key));
    *sws = sws_getContext(f->width, f->height, format, width, height,
                          AV_PIX_FMT_RGBA, flags | SWS_ACCURATE_RND |
                              SWS_FULL_CHR_H_INT | SWS_BITEXACT,
                          NULL, NULL, NULL);
    if (!*sws) return AVERROR(EINVAL);
    const int *coefficients = sws_getCoefficients(matrix);
    if (sws_setColorspaceDetails(*sws, coefficients, full ? 1 : 0,
                                 coefficients, 1, 0, 1 << 16, 1 << 16) < 0) {
        sws_freeContext(*sws);
        *sws = NULL;
        return AVERROR(EINVAL);
    }
    memcpy(cached, key, sizeof(key));
    return 0;
}

static int open_decoder(AVFormatContext *fmt, int stream, AVCodecContext **out) {
    AVStream *st = fmt->streams[stream];
    const AVCodec *codec = avcodec_find_decoder(st->codecpar->codec_id);
    if (!codec) return AVERROR_DECODER_NOT_FOUND;
    AVCodecContext *dec = avcodec_alloc_context3(codec);
    if (!dec) return AVERROR(ENOMEM);
    *out = dec;
    int rc = avcodec_parameters_to_context(dec, st->codecpar);
    if (rc < 0) return rc;
    dec->pkt_timebase = st->time_base;
    /* One thread: frame threading changes nothing in the output but adds
     * latency frames that complicate exact seeking. */
    dec->thread_count = 1;
    return avcodec_open2(dec, codec, NULL);
}

/* First output index that shows a frame with this pts: the smallest i with
 * t <= i / fps + eps, where t is the pts relative to the origin in seconds
 * and eps is 1e-6 of a frame, or half a stream tick when that is larger
 * (timestamps are rounded to ticks: a 30 fps frame at 66.67 ms is stored
 * as 67 ms in Matroska), at most half a frame. Exact in 128 bits. */
static int64_t first_index(const SrVideoSource *v, int64_t pts) {
    SrI128 n = ((SrI128)pts - v->origin) * v->tb.num * v->frame_tb.den;
    SrI128 d = (SrI128)v->tb.den * v->frame_tb.num;
    SrI128 q = n / d, r = n % d;
    if (r < 0) {
        q -= 1;
        r += d;
    }
    /* ceil(q + r/d - eps) for 0 < eps <= 1/2. */
    if (r * v->eps_den > v->eps_num * d) q += 1;
    return q > INT64_MAX ? INT64_MAX : q < INT64_MIN ? INT64_MIN : (int64_t)q;
}

static void set_tolerance(SrVideoSource *v) {
    SrI128 d = (SrI128)v->tb.den * v->frame_tb.num;
    v->eps_num = (SrI128)v->tb.num * v->frame_tb.den;   /* half a tick */
    v->eps_den = 2 * d;
    if (2 * v->eps_num > v->eps_den) {
        v->eps_num = 1;
        v->eps_den = 2;
    }
    if (v->eps_num * 1000000 < v->eps_den) {
        v->eps_num = 1;
        v->eps_den = 1000000;
    }
}

/* Closes and reopens the demuxer: the only way back to the first frame of
 * a stream without timestamps, which demuxers cannot seek. */
static int reopen(SrVideoSource *v) {
    avformat_close_input(&v->fmt);
    int rc = avformat_open_input(&v->fmt, v->path, NULL, NULL);
    if (rc >= 0) rc = avformat_find_stream_info(v->fmt, NULL);
    if (rc >= 0 && (v->stream >= (int)v->fmt->nb_streams ||
                    v->fmt->streams[v->stream]->codecpar->codec_type !=
                        AVMEDIA_TYPE_VIDEO))
        rc = AVERROR_INVALIDDATA;
    if (rc < 0) avformat_close_input(&v->fmt);
    return rc;
}

/* Demuxes every packet once to find the pts of the first and last frames in
 * presentation order, which container metadata does not reliably give, and
 * the timeline origin (media_internal.h). */
static int scan(SrVideoSource *v) {
    int64_t packets = 0, first = INT64_MAX, last = INT64_MIN;
    int64_t origin_us = sr_media_origin_us(v->fmt);
    int audio = -1;
    if (origin_us == AV_NOPTS_VALUE)
        audio = av_find_best_stream(v->fmt, AVMEDIA_TYPE_AUDIO, -1, -1, NULL, 0);
    int64_t fallback = AV_NOPTS_VALUE;
    int rc;
    while ((rc = av_read_frame(v->fmt, v->pkt)) >= 0) {
        int s = v->pkt->stream_index;
        int64_t ts = v->pkt->pts != AV_NOPTS_VALUE ? v->pkt->pts : v->pkt->dts;
        if (s == v->stream) {
            ++packets;
            if (ts != AV_NOPTS_VALUE) {
                first = ts < first ? ts : first;
                last = ts > last ? ts : last;
            }
        }
        if (origin_us == AV_NOPTS_VALUE && (s == v->stream || (audio >= 0 && s == audio))) {
            int64_t us = sr_media_start_us(v->fmt->streams[s], ts);
            if (us != AV_NOPTS_VALUE && (fallback == AV_NOPTS_VALUE || us < fallback))
                fallback = us;
        }
        av_packet_unref(v->pkt);
    }
    if (rc != AVERROR_EOF) return rc;
    if (packets == 0) return AVERROR_INVALIDDATA;
    if (first == INT64_MAX) {
        /* No timestamps: one frame per packet, counted from the start. */
        v->no_timestamps = true;
        v->info.frame_count = packets;
        return reopen(v);
    }
    if (origin_us == AV_NOPTS_VALUE) origin_us = fallback != AV_NOPTS_VALUE ? fallback : 0;
    v->origin = sr_media_origin_in(v->fmt->streams[v->stream], origin_us);
    v->first_pts = first;
    int64_t last_index = first_index(v, last);
    v->info.frame_count = last_index < 0 ? 1 : last_index + 1;
    return avformat_seek_file(v->fmt, v->stream, INT64_MIN, first, first, 0);
}

static int alloc_cache(SrVideoSource *v, size_t cache_bytes) {
    size_t pixels = (size_t)v->info.width * v->info.height;
    size_t per_frame = pixels * 4 * sizeof(float);
    size_t count = per_frame ? cache_bytes / per_frame : 0;
    if (count < SR_VIDEO_CACHE_MIN_FRAMES) count = SR_VIDEO_CACHE_MIN_FRAMES;
    v->slots = calloc(count, sizeof(*v->slots));
    v->rgba = sr_alloc(pixels * 4);
    if (!v->slots || !v->rgba) return AVERROR(ENOMEM);
    v->slot_count = count;
    for (size_t i = 0; i < count; ++i) v->slots[i].index = -1;
    return 0;
}

SrStatus sr_video_open(const char *path, const SrProject *project,
                       SrColorSpace source_space, uint32_t fps_num,
                       uint32_t fps_den, size_t cache_bytes,
                       SrVideoSource **out, char *err, size_t errlen) {
    if (out) *out = NULL;
    if (!path || !project || !out || !fps_num || !fps_den ||
        fps_num > INT32_MAX || fps_den > INT32_MAX) {
        set_err(err, errlen, "invalid argument", 0);
        return SR_ERR_ARGUMENT;
    }
    SrVideoSource *v = calloc(1, sizeof(*v));
    if (!v) return SR_ERR_MEMORY;
    v->project = project;
    v->source_space = source_space;
    v->frame_tb = (AVRational){(int)fps_den, (int)fps_num};
    v->path = sr_strdup(path);
    const char *stage = "out of memory";
    int rc = v->path ? 0 : AVERROR(ENOMEM);
    if (rc >= 0) {
        stage = "cannot open file";
        rc = avformat_open_input(&v->fmt, path, NULL, NULL);
    }
    if (rc >= 0) {
        stage = "cannot read stream information";
        rc = avformat_find_stream_info(v->fmt, NULL);
    }
    if (rc >= 0) {
        stage = "no video stream";
        rc = av_find_best_stream(v->fmt, AVMEDIA_TYPE_VIDEO, -1, -1, NULL, 0);
        v->stream = rc;
    }
    if (rc >= 0) {
        stage = "cannot open decoder";
        rc = open_decoder(v->fmt, v->stream, &v->dec);
    }
    if (rc >= 0) {
        stage = "invalid video stream";
        AVStream *st = v->fmt->streams[v->stream];
        AVRational rate = st->avg_frame_rate.num > 0 ? st->avg_frame_rate
                                                     : st->r_frame_rate;
        v->info.rate_num = rate.num;
        v->info.rate_den = rate.den;
        v->info.matrix_approximated =
            st->codecpar->color_space == AVCOL_SPC_BT2020_CL;
        v->matrix_warned = v->info.matrix_approximated;   /* caller reports it */
        v->tb = st->time_base;
        if (v->dec->width <= 0 || v->dec->height <= 0 || v->dec->width > 16384 ||
            v->dec->height > 16384 || v->tb.num <= 0 || v->tb.den <= 0)
            rc = AVERROR_INVALIDDATA;
        v->info.width = (uint32_t)(v->dec->width > 0 ? v->dec->width : 0);
        v->info.height = (uint32_t)(v->dec->height > 0 ? v->dec->height : 0);
    }
    if (rc >= 0) {
        set_tolerance(v);
        stage = "out of memory";
        v->pkt = av_packet_alloc();
        v->frame = av_frame_alloc();
        v->prev = av_frame_alloc();
        v->ahead = av_frame_alloc();
        if (!v->pkt || !v->frame || !v->prev || !v->ahead) rc = AVERROR(ENOMEM);
    }
    if (rc >= 0) {
        stage = "cannot index frames";
        rc = scan(v);
    }
    if (rc >= 0) {
        stage = "out of memory";
        rc = alloc_cache(v, cache_bytes);
    }
    if (rc < 0) {
        set_err(err, errlen, stage, rc);
        sr_video_close(v);
        return averr_status(rc);
    }
    *out = v;
    return SR_OK;
}

void sr_video_close(SrVideoSource *v) {
    if (!v) return;
    for (size_t i = 0; i < v->slot_count; ++i) free(v->slots[i].image.px);
    free(v->slots);
    free(v->rgba);
    free(v->path);
    sws_freeContext(v->sws);
    av_frame_free(&v->frame);
    av_frame_free(&v->prev);
    av_frame_free(&v->ahead);
    av_packet_free(&v->pkt);
    avcodec_free_context(&v->dec);
    avformat_close_input(&v->fmt);
    free(v);
}

const SrVideoInfo *sr_video_info(const SrVideoSource *v) { return &v->info; }
const SrVideoStats *sr_video_stats(const SrVideoSource *v) { return &v->stats; }

size_t sr_video_minimum_bytes(const SrVideoSource *v) {
    if (!v) return 0;
    return (size_t)v->info.width * v->info.height * 4 * sizeof(float) *
           SR_VIDEO_CACHE_MIN_FRAMES;
}

size_t sr_video_scene_minimum_bytes(const SrScene *scene) {
    size_t total = 0;
    for (size_t i = 0; scene && i < scene->asset_count; ++i) {
        size_t bytes = sr_video_minimum_bytes(scene->assets[i].video);
        total = bytes > SIZE_MAX - total ? SIZE_MAX : total + bytes;
    }
    return total;
}

/* First output index of decoded frame f. */
static int64_t frame_index(SrVideoSource *v, const AVFrame *f) {
    if (v->no_timestamps) return v->counted++;
    int64_t pts = f->best_effort_timestamp != AV_NOPTS_VALUE
                      ? f->best_effort_timestamp : f->pts;
    if (pts == AV_NOPTS_VALUE)   /* a lone untimed frame follows the last */
        return v->prev->buf[0] ? v->prev_index + 1 : 0;
    return first_index(v, pts);
}

/* Next decoded frame into v->frame: 0, AVERROR_EOF at the end, or error. */
static int decode_next(SrVideoSource *v) {
    if (!v->fmt) return AVERROR(EINVAL);
    for (;;) {
        int rc = avcodec_receive_frame(v->dec, v->frame);
        if (rc == 0) {
            v->stats.decoded++;
            return 0;
        }
        if (rc != AVERROR(EAGAIN)) return rc;
        rc = av_read_frame(v->fmt, v->pkt);
        if (rc == AVERROR_EOF) {
            rc = avcodec_send_packet(v->dec, NULL);
            if (rc < 0 && rc != AVERROR_EOF) return rc;
            continue;
        }
        if (rc < 0) return rc;
        if (v->pkt->stream_index == v->stream) rc = avcodec_send_packet(v->dec, v->pkt);
        av_packet_unref(v->pkt);
        if (rc < 0) return rc;
    }
}

/* Positions the decoder so that decoding forward reaches the frame shown
 * at `index`: a seek to the latest keyframe at or before its time, or to
 * the first frame; a stream without timestamps is reopened instead. */
static int reposition(SrVideoSource *v, int64_t index) {
    av_frame_unref(v->prev);
    av_frame_unref(v->ahead);
    v->positioned = false;
    v->stats.seeks++;
    int rc;
    if (v->no_timestamps) {
        rc = reopen(v);
        v->at_start = true;
        v->counted = 0;
    } else {
        int64_t offset = av_rescale_q_rnd(index, v->frame_tb, v->tb,
                                          AV_ROUND_DOWN | AV_ROUND_PASS_MINMAX);
        int64_t target = offset > INT64_MAX - v->origin ? INT64_MAX : v->origin + offset;
        v->at_start = index <= 0 || target <= v->first_pts;
        if (v->at_start) target = v->first_pts;
        rc = avformat_seek_file(v->fmt, v->stream, INT64_MIN, target, target, 0);
    }
    if (rc < 0) return rc;
    avcodec_flush_buffers(v->dec);
    v->positioned = true;
    return 0;
}

static CacheSlot *cache_find(SrVideoSource *v, int64_t index) {
    for (size_t i = 0; i < v->slot_count; ++i)
        if (v->slots[i].index == index) return &v->slots[i];
    return NULL;
}

static CacheSlot *cache_victim(SrVideoSource *v) {
    CacheSlot *best = &v->slots[0];
    for (size_t i = 0; i < v->slot_count; ++i) {
        CacheSlot *slot = &v->slots[i];
        if (slot->index < 0) return slot;
        if (slot->stamp < best->stamp) best = slot;
    }
    return best;
}

/* Converts f to blend space and stores it in the cache as `index`. */
static int convert(SrVideoSource *v, const AVFrame *f, int64_t index,
                   CacheSlot **out) {
    bool approximated = false;
    int rc = build_scaler(&v->sws, v->sws_key, f, (int)v->info.width,
                          (int)v->info.height, SWS_BICUBIC, &approximated);
    if (rc < 0) {
        v->failure = "cannot set up color conversion";
        return rc;
    }
    if (approximated && !v->matrix_warned) {
        v->matrix_warned = true;
        av_log(NULL, AV_LOG_WARNING,
               "%s: BT.2020 constant-luminance frames are decoded with the "
               "non-constant-luminance matrix (approximation)\n", v->path);
    }
    uint8_t *dst[4] = {v->rgba, NULL, NULL, NULL};
    int dst_stride[4] = {(int)v->info.width * 4, 0, 0, 0};
    rc = sws_scale(v->sws, (const uint8_t *const *)f->data, f->linesize, 0,
                   f->height, dst, dst_stride);
    if (rc <= 0) {
        v->failure = "color conversion failed";
        return rc < 0 ? rc : AVERROR_EXTERNAL;
    }
    CacheSlot *slot = cache_victim(v);
    free(slot->image.px);
    slot->image = (SrImage){0};
    slot->index = -1;
    if (sr_color_image_from_rgba8(v->project, v->source_space, v->rgba,
                                  (size_t)v->info.width * 4, v->info.width,
                                  v->info.height, &slot->image) != SR_OK)
        return AVERROR(ENOMEM);
    slot->index = index;
    *out = slot;
    return 0;
}

/* Decodes until the frame shown at `index` is known and converts it. The
 * shown frame is the latest one whose first index (first_index) is at most
 * `index`, never a later one; forward decoding and seeking agree because
 * a seek lands on a keyframe at or before that frame. */
static int produce(SrVideoSource *v, int64_t index, CacheSlot **out) {
    bool forward = v->positioned &&
                   (v->prev->buf[0] ? v->prev_index <= index : v->at_start);
    if (forward && !v->no_timestamps) {
        int64_t reached = v->ahead->buf[0] ? v->ahead_index
                        : v->prev->buf[0] ? v->prev_index : 0;
        if (index - reached > FORWARD_DECODE_LIMIT) forward = false;
    }
    int rc;
    if (!forward && (rc = reposition(v, index)) < 0) return rc;
    bool restarted = false;
    for (;;) {
        while (!v->ahead->buf[0] || v->ahead_index <= index) {
            if (v->ahead->buf[0]) {
                av_frame_unref(v->prev);
                av_frame_move_ref(v->prev, v->ahead);
                v->prev_index = v->ahead_index;
                continue;
            }
            rc = decode_next(v);
            if (rc == AVERROR_EOF) break;    /* past the last frame: hold it */
            if (rc < 0) return rc;
            v->ahead_index = frame_index(v, v->frame);
            av_frame_move_ref(v->ahead, v->frame);
        }
        const AVFrame *shown = v->prev->buf[0] ? v->prev : NULL;
        if (!shown && v->ahead->buf[0]) {
            if (!v->at_start && !restarted) {
                /* The seek landed after the target: start from frame 0. */
                restarted = true;
                if ((rc = reposition(v, 0)) < 0) return rc;
                continue;
            }
            shown = v->ahead;   /* before the first frame: the first frame */
        }
        if (!shown) return AVERROR_INVALIDDATA;
        return convert(v, shown, index, out);
    }
}

SrStatus sr_video_frame(SrVideoSource *v, int64_t index, const SrImage **out,
                        char *err, size_t errlen) {
    if (out) *out = NULL;
    if (!v || !out) return SR_ERR_ARGUMENT;
    if (index < 0) index = 0;
    else if (index >= v->info.frame_count) index = v->info.frame_count - 1;
    v->stats.requests++;
    CacheSlot *slot = cache_find(v, index);
    if (slot) {
        v->stats.cache_hits++;
    } else {
        v->failure = NULL;
        int rc = produce(v, index, &slot);
        if (rc < 0) {
            /* Unknown decoder position: the next request repositions. */
            v->positioned = false;
            set_err(err, errlen, v->failure ? v->failure : "cannot decode frame", rc);
            return averr_status(rc);
        }
    }
    slot->stamp = ++v->clock;
    *out = &slot->image;
    return SR_OK;
}

/* ---- still images ------------------------------------------------------- */

SrStatus sr_image_decode_rgba8(const char *path, uint32_t width,
                               uint32_t height, bool scale, uint8_t **rgba,
                               uint32_t *file_width, uint32_t *file_height,
                               char *err, size_t errlen) {
    if (rgba) *rgba = NULL;
    if (!path || !rgba || !width || !height || width > 16384 || height > 16384) {
        set_err(err, errlen, "invalid argument", 0);
        return SR_ERR_ARGUMENT;
    }
    AVFormatContext *fmt = NULL;
    AVCodecContext *dec = NULL;
    AVPacket *pkt = NULL;
    AVFrame *frame = NULL;
    struct SwsContext *sws = NULL;
    uint8_t *pixels = NULL;
    const char *stage = "cannot open file";
    int stream = -1;
    int rc = avformat_open_input(&fmt, path, NULL, NULL);
    if (rc >= 0) {
        stage = "cannot read stream information";
        rc = avformat_find_stream_info(fmt, NULL);
    }
    if (rc >= 0) {
        stage = "no image stream";
        rc = stream = av_find_best_stream(fmt, AVMEDIA_TYPE_VIDEO, -1, -1, NULL, 0);
    }
    if (rc >= 0) {
        stage = "cannot open decoder";
        rc = open_decoder(fmt, stream, &dec);
    }
    if (rc >= 0) {
        stage = "out of memory";
        pkt = av_packet_alloc();
        frame = av_frame_alloc();
        if (!pkt || !frame) rc = AVERROR(ENOMEM);
    }
    bool got = false, flushed = false;
    if (rc >= 0) stage = "cannot decode image";
    while (rc >= 0 && !got) {
        rc = avcodec_receive_frame(dec, frame);
        if (rc == 0) {
            got = true;
            break;
        }
        if (rc != AVERROR(EAGAIN)) break;
        if (flushed) {
            rc = AVERROR_INVALIDDATA;
            break;
        }
        rc = av_read_frame(fmt, pkt);
        if (rc == AVERROR_EOF) {
            flushed = true;
            rc = avcodec_send_packet(dec, NULL);
            continue;
        }
        if (rc < 0) break;
        if (pkt->stream_index == stream) rc = avcodec_send_packet(dec, pkt);
        av_packet_unref(pkt);
    }
    if (got) {
        if (file_width) *file_width = (uint32_t)frame->width;
        if (file_height) *file_height = (uint32_t)frame->height;
        if (!scale && ((uint32_t)frame->width != width ||
                       (uint32_t)frame->height != height)) {
            if (err && errlen)
                snprintf(err, errlen, "file is %dx%d, declared %ux%u",
                         frame->width, frame->height, width, height);
            rc = AVERROR_INVALIDDATA;
            stage = NULL;
        } else {
            int key[5] = {0};
            bool approximated = false;
            stage = "cannot set up color conversion";
            rc = build_scaler(&sws, key, frame, (int)width, (int)height,
                              SWS_LANCZOS, &approximated);
            static atomic_flag warned = ATOMIC_FLAG_INIT;
            if (rc >= 0 && approximated && !atomic_flag_test_and_set(&warned))
                av_log(NULL, AV_LOG_WARNING,
                       "%s: BT.2020 constant-luminance image decoded with the "
                       "non-constant-luminance matrix (approximation)\n", path);
        }
    }
    if (got && rc >= 0) {
        stage = "out of memory";
        pixels = sr_alloc((size_t)width * height * 4);
        if (!pixels) {
            rc = AVERROR(ENOMEM);
        } else {
            uint8_t *dst[4] = {pixels, NULL, NULL, NULL};
            int dst_stride[4] = {(int)width * 4, 0, 0, 0};
            stage = "color conversion failed";
            rc = sws_scale(sws, (const uint8_t *const *)frame->data,
                           frame->linesize, 0, frame->height, dst, dst_stride);
            if (rc == 0) rc = AVERROR_EXTERNAL;
        }
    }
    sws_freeContext(sws);
    av_frame_free(&frame);
    av_packet_free(&pkt);
    avcodec_free_context(&dec);
    avformat_close_input(&fmt);
    if (rc < 0 || !got) {
        if (stage) set_err(err, errlen, stage, rc < 0 ? rc : AVERROR_INVALIDDATA);
        free(pixels);
        return averr_status(rc);
    }
    *rgba = pixels;
    return SR_OK;
}
