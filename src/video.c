#define _POSIX_C_SOURCE 200809L
#include "scene_render/video.h"
#include "scene_render/color.h"

#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/pixdesc.h>
#include <libswscale/swscale.h>

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* Decoding forward is cheaper than seeking for short gaps; beyond this many
 * frames the source seeks to the nearest earlier keyframe instead. */
enum { FORWARD_DECODE_LIMIT = 48 };

typedef struct {
    int64_t index;        /* -1 = empty */
    uint64_t stamp;       /* last use, for LRU eviction */
    SrImage image;
} CacheSlot;

struct SrVideoSource {
    AVFormatContext *fmt;
    AVCodecContext *dec;
    AVPacket *pkt;
    AVFrame *frame;
    AVFrame *prev;        /* latest decoded frame before the target */
    struct SwsContext *sws;
    int sws_format, sws_range, sws_space, sws_width, sws_height;
    uint8_t *rgba;        /* width * height * 4 conversion buffer */
    const SrProject *project;
    SrColorSpace source_space;
    int stream;
    AVRational tb;        /* stream time base */
    AVRational frame_tb;  /* 1 / declared fps */
    int64_t start_pts;    /* pts of frame 0 */
    int64_t next_index;   /* index the decoder produces next; -1 unknown */
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

/* Builds (or reuses) a scaler from frame f to width x height RGBA using the
 * frame's own matrix and range. Untagged streams follow the usual
 * convention: BT.709 from 720 lines up, BT.601 below. */
static int build_scaler(struct SwsContext **sws, int *cached, const AVFrame *f,
                        int width, int height, int flags) {
    bool full = f->color_range == AVCOL_RANGE_JPEG;
    enum AVPixelFormat format = plain_format((enum AVPixelFormat)f->format, &full);
    int space = f->colorspace;
    int key[5] = {format, full, space, f->width, f->height};
    if (*sws && !memcmp(cached, key, sizeof(key))) return 0;
    sws_freeContext(*sws);
    *sws = sws_getContext(f->width, f->height, format, width, height,
                          AV_PIX_FMT_RGBA, flags | SWS_ACCURATE_RND |
                              SWS_FULL_CHR_H_INT | SWS_BITEXACT,
                          NULL, NULL, NULL);
    if (!*sws) return AVERROR(EINVAL);
    int matrix = space == AVCOL_SPC_BT709 ? SWS_CS_ITU709
               : space == AVCOL_SPC_UNSPECIFIED
                   ? (f->height >= 720 ? SWS_CS_ITU709 : SWS_CS_ITU601)
                   : SWS_CS_DEFAULT;
    if (space == AVCOL_SPC_BT2020_NCL || space == AVCOL_SPC_BT2020_CL)
        matrix = SWS_CS_BT2020;
    const int *coefficients = sws_getCoefficients(matrix);
    (void)sws_setColorspaceDetails(*sws, coefficients, full ? 1 : 0,
                                   coefficients, 1, 0, 1 << 16, 1 << 16);
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
    /* One thread: frame threading changes nothing in the output but adds
     * latency frames that complicate exact seeking. */
    dec->thread_count = 1;
    return avcodec_open2(dec, codec, NULL);
}

/* Demuxes every packet once to find the pts of the first and last frames in
 * presentation order, which container metadata does not reliably give. */
static int scan(SrVideoSource *v) {
    int64_t packets = 0, first = INT64_MAX, last = INT64_MIN;
    int rc;
    while ((rc = av_read_frame(v->fmt, v->pkt)) >= 0) {
        if (v->pkt->stream_index == v->stream) {
            ++packets;
            int64_t ts = v->pkt->pts != AV_NOPTS_VALUE ? v->pkt->pts : v->pkt->dts;
            if (ts != AV_NOPTS_VALUE) {
                first = ts < first ? ts : first;
                last = ts > last ? ts : last;
            }
        }
        av_packet_unref(v->pkt);
    }
    if (rc != AVERROR_EOF) return rc;
    if (packets == 0) return AVERROR_INVALIDDATA;
    if (first == INT64_MAX) {
        v->start_pts = 0;   /* no timestamps: one frame per packet */
        v->info.frame_count = packets;
    } else {
        v->start_pts = first;
        v->info.frame_count = 1 + av_rescale_q_rnd(last - first, v->tb, v->frame_tb,
                                                   AV_ROUND_NEAR_INF | AV_ROUND_PASS_MINMAX);
    }
    return avformat_seek_file(v->fmt, v->stream, INT64_MIN, v->start_pts,
                              v->start_pts, 0);
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
    v->next_index = -1;
    v->frame_tb = (AVRational){(int)fps_den, (int)fps_num};
    const char *stage = "cannot open file";
    int rc = avformat_open_input(&v->fmt, path, NULL, NULL);
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
        v->tb = st->time_base;
        if (v->dec->width <= 0 || v->dec->height <= 0 || v->dec->width > 16384 ||
            v->dec->height > 16384 || v->tb.num <= 0 || v->tb.den <= 0)
            rc = AVERROR_INVALIDDATA;
        v->info.width = (uint32_t)(v->dec->width > 0 ? v->dec->width : 0);
        v->info.height = (uint32_t)(v->dec->height > 0 ? v->dec->height : 0);
    }
    if (rc >= 0) {
        stage = "out of memory";
        v->pkt = av_packet_alloc();
        v->frame = av_frame_alloc();
        v->prev = av_frame_alloc();
        if (!v->pkt || !v->frame || !v->prev) rc = AVERROR(ENOMEM);
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
    sws_freeContext(v->sws);
    av_frame_free(&v->frame);
    av_frame_free(&v->prev);
    av_packet_free(&v->pkt);
    avcodec_free_context(&v->dec);
    avformat_close_input(&v->fmt);
    free(v);
}

const SrVideoInfo *sr_video_info(const SrVideoSource *v) { return &v->info; }
const SrVideoStats *sr_video_stats(const SrVideoSource *v) { return &v->stats; }

static int64_t frame_index_of(const SrVideoSource *v, const AVFrame *f) {
    int64_t pts = f->best_effort_timestamp != AV_NOPTS_VALUE
                      ? f->best_effort_timestamp : f->pts;
    if (pts == AV_NOPTS_VALUE) return v->next_index >= 0 ? v->next_index : 0;
    return av_rescale_q_rnd(pts - v->start_pts, v->tb, v->frame_tb,
                            AV_ROUND_NEAR_INF | AV_ROUND_PASS_MINMAX);
}

/* Next decoded frame into v->frame: 0, AVERROR_EOF at the end, or error. */
static int decode_next(SrVideoSource *v) {
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

static int seek_to(SrVideoSource *v, int64_t index) {
    int64_t target = v->start_pts + av_rescale_q(index, v->frame_tb, v->tb);
    int rc = avformat_seek_file(v->fmt, v->stream, INT64_MIN, target, target, 0);
    if (rc < 0) return rc;
    avcodec_flush_buffers(v->dec);
    av_frame_unref(v->prev);
    v->next_index = -1;
    v->stats.seeks++;
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
    int key[5] = {v->sws_format, v->sws_range, v->sws_space, v->sws_width,
                  v->sws_height};
    int rc = build_scaler(&v->sws, key, f, (int)v->info.width,
                          (int)v->info.height, SWS_BICUBIC);
    v->sws_format = key[0]; v->sws_range = key[1]; v->sws_space = key[2];
    v->sws_width = key[3]; v->sws_height = key[4];
    if (rc < 0) return rc;
    uint8_t *dst[4] = {v->rgba, NULL, NULL, NULL};
    int dst_stride[4] = {(int)v->info.width * 4, 0, 0, 0};
    sws_scale(v->sws, (const uint8_t *const *)f->data, f->linesize, 0,
              f->height, dst, dst_stride);
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

static int keep_as_prev(SrVideoSource *v) {
    av_frame_unref(v->prev);
    return av_frame_ref(v->prev, v->frame);
}

/* Decodes until the frame shown at `index` is known and converts it. */
static int produce(SrVideoSource *v, int64_t index, CacheSlot **out) {
    bool from_start = false;
    if (v->next_index < 0 || index < v->next_index ||
        index - v->next_index > FORWARD_DECODE_LIMIT) {
        int rc = seek_to(v, index);
        if (rc < 0) return rc;
    }
    for (;;) {
        int rc = decode_next(v);
        if (rc == AVERROR_EOF) {
            /* Past the last frame: hold the latest one decoded. */
            return v->prev->buf[0] ? convert(v, v->prev, index, out)
                                   : AVERROR_INVALIDDATA;
        }
        if (rc < 0) return rc;
        int64_t got = frame_index_of(v, v->frame);
        v->next_index = got + 1;
        if (got < index) {
            rc = keep_as_prev(v);
            if (rc < 0) return rc;
            continue;
        }
        if (got > index && !v->prev->buf[0] && !from_start && index > 0) {
            /* The seek landed after the target: restart from frame 0. */
            from_start = true;
            rc = seek_to(v, 0);
            if (rc < 0) return rc;
            continue;
        }
        /* got == index shows this frame; a gap shows the one before it. */
        const AVFrame *shown = got > index && v->prev->buf[0] ? v->prev : v->frame;
        rc = convert(v, shown, index, out);
        if (rc == 0) rc = keep_as_prev(v);
        return rc;
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
        int rc = produce(v, index, &slot);
        if (rc < 0) {
            /* Unknown decoder position: the next request seeks. */
            v->next_index = -1;
            set_err(err, errlen, "cannot decode frame", rc);
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
            stage = "cannot set up color conversion";
            rc = build_scaler(&sws, key, frame, (int)width, (int)height,
                              SWS_LANCZOS);
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
            sws_scale(sws, (const uint8_t *const *)frame->data, frame->linesize,
                      0, frame->height, dst, dst_stride);
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
