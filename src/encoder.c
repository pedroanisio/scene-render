#define _POSIX_C_SOURCE 200809L
#include "scene_render/encoder.h"
#include "scene_render/parallel.h"
#include "scene_render/outputs.h"
#include "gif_palette_internal.h"
#include "metadata.h"

#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/channel_layout.h>
#include <libavutil/intreadwrite.h>
#include <libavutil/opt.h>
#include <libavutil/pixdesc.h>
#include <libavutil/spherical.h>
#include <libswresample/swresample.h>
#include <libswscale/swscale.h>

#include <errno.h>
#include <fcntl.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <unistd.h>

/* How the caller's frame reaches the codec's AVFrame. */
typedef enum {
    CONVERT_SCALE,          /* swscale from RGBA/RGBA64 */
    CONVERT_PALETTE,        /* per-frame palette (gif) */
    CONVERT_FLOAT_PLANES    /* linear float RGBA -> GBR(A) planes (exr) */
} ConvertMode;

struct SrEncoder {
    const SrOutput *output; /* borrowed; outlives the encoder */
    const char *muxer;      /* NULL for image sequences */
    ConvertMode convert;
    /* Image sequences: one file per packet, named from `pattern` with the
     * absolute frame number first_number + pts. */
    char *pattern;
    uint64_t first_number;
    SrGifPalette *palette;
    char failed_file[512];  /* sequence file that could not be written */
    AVFormatContext *fmt;
    AVPacket *pkt;
    /* Video. */
    AVCodecContext *video;
    AVStream *vstream;
    struct SwsContext *sws;
    AVFrame *src;           /* threaded scaler: view of the caller's RGBA */
    enum AVPixelFormat src_format;
    bool sws_threaded;      /* slice-threaded scaler (sws_scale_frame) */
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
    int audio_frame_size;   /* fixed codec frame (padded tail); 0 = none */
    bool matroska;
    /* Pass-through video (sr_encoder_open_copy): no video codec; packets
     * of finished segments are copied with shifted timestamps. */
    AVCodecParameters *copy_par;
    AVRational frame_tb;    /* 1/fps: the frame index time base */
    bool header_written;
    bool trailer_written;
    bool closing;           /* sr_encoder_finish has started */
    bool finished;          /* sr_encoder_finish succeeded */
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
    return sr_codec_info(codec)->encoder;
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

static const char *output_muxer(const SrOutput *output, const char *path) {
    return path ? sr_output_muxer(output, path, NULL) : NULL;
}

SrStatus sr_encoder_validate_metadata(const SrScene *scene, const char *path,
                                       SrDiagnostics *diag) {
    if (!scene) return SR_ERR_ARGUMENT;
    return sr_encoder_validate_output_metadata(scene, &scene->output, path, diag);
}

SrStatus sr_encoder_validate_output_metadata(const SrScene *scene,
                                              const SrOutput *output,
                                              const char *path,
                                              SrDiagnostics *diag) {
    if (!scene || !output) return SR_ERR_ARGUMENT;
    if (!output->embed_metadata || !scene->metadata_count) return SR_OK;
    const char *container = output_muxer(output, path);
    if (container && (!strcmp(container, "gif") || !strcmp(container, "apng")))
        container = NULL;
    if (!container) {
        if (sr_codec_info(output->codec)->legacy)
            sr_diag_error(diag, output->source_line, "output", "path",
                          "metadata output needs a .mp4, .mov or .mkv extension");
        else
            sr_diag_error(diag, output->source_line, "output", "embedMetadata",
                          "codec %s cannot carry metadata; set "
                          "embedMetadata=\"false\" on this output",
                          sr_codec_info(output->codec)->name);
        return SR_ERR_ARGUMENT;
    }
    return sr_metadata_validate_embed(scene, true, container, diag);
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

/* The RGB -> Y'CbCr scaler. With several threads swscale converts
 * horizontal slices of the output in parallel, each slice context reading
 * the whole source, so every output row is computed exactly as by one
 * thread: the bytes do not depend on the thread count (verified for 8- and
 * 16-bit input, 4:2:0/4:2:2/4:4:4 output, odd sizes, both ranges). */
static int open_scaler(SrEncoder *e, const AVCodecContext *c, unsigned threads) {
    const int flags = SWS_BICUBIC | SWS_ACCURATE_RND | SWS_FULL_CHR_H_INT |
                      SWS_BITEXACT;
    if (threads <= 1) {
        e->sws = sws_getContext(c->width, c->height, e->src_format, c->width,
                                c->height, c->pix_fmt, flags, NULL, NULL, NULL);
        return e->sws ? 0 : AVERROR(EINVAL);
    }
    e->sws = sws_alloc_context();
    if (!e->sws) return AVERROR(ENOMEM);
    e->sws_threaded = true;
    int rc = av_opt_set_int(e->sws, "srcw", c->width, 0);
    if (rc >= 0) rc = av_opt_set_int(e->sws, "srch", c->height, 0);
    if (rc >= 0) rc = av_opt_set_int(e->sws, "src_format", e->src_format, 0);
    if (rc >= 0) rc = av_opt_set_int(e->sws, "dstw", c->width, 0);
    if (rc >= 0) rc = av_opt_set_int(e->sws, "dsth", c->height, 0);
    if (rc >= 0) rc = av_opt_set_int(e->sws, "dst_format", c->pix_fmt, 0);
    if (rc >= 0) rc = av_opt_set_int(e->sws, "sws_flags", flags, 0);
    if (rc >= 0) rc = av_opt_set_int(e->sws, "threads", (int64_t)threads, 0);
    if (rc >= 0) rc = sws_init_context(e->sws, NULL, NULL);
    if (rc >= 0) {
        e->src = av_frame_alloc();
        if (!e->src) return AVERROR(ENOMEM);
        e->src->format = e->src_format;
        e->src->width = c->width;
        e->src->height = c->height;
    }
    return rc;
}

static bool rgb_format(enum AVPixelFormat format) {
    const AVPixFmtDescriptor *desc = av_pix_fmt_desc_get(format);
    return desc && (desc->flags & AV_PIX_FMT_FLAG_RGB);
}

/* Options of the codecs added in 1.1 (B1-6). Every worker count is pinned
 * to one so the bytes do not depend on --threads or the machine. */
static SrStatus configure_new_codec(SrEncoder *e, AVCodecContext *c,
                                    SrDiagnostics *diag) {
    const SrOutput *output = e->output;
    int rc = 0;
    int level = sr_output_preset_level(output->codec, output->preset);
    switch (output->codec) {
    case SR_CODEC_PRORES: {
        static const int profiles[] = {2, 0, 1, 2, 3, 4, 5};
        rc = av_opt_set_int(c->priv_data, "profile",
                            profiles[output->prores_profile], 0);
        break;
    }
    case SR_CODEC_VP9:
        rc = av_opt_set(c->priv_data, "deadline", "good", 0);
        if (rc >= 0) rc = av_opt_set_int(c->priv_data, "cpu-used", level, 0);
        if (rc >= 0) rc = av_opt_set_int(c->priv_data, "row-mt", 0, 0);
        if (rc >= 0 && !output->bitrate) {
            c->bit_rate = 0;
            rc = av_opt_set_int(c->priv_data, "crf", output->crf, 0);
        }
        break;
    case SR_CODEC_AV1:
        rc = av_opt_set_int(c->priv_data, "preset", level, 0);
        if (rc >= 0 && !output->bitrate)
            rc = av_opt_set_int(c->priv_data, "crf", output->crf, 0);
        if (rc >= 0) rc = av_opt_set(c->priv_data, "svtav1-params", "lp=1", 0);
        break;
    case SR_CODEC_EXR_SEQUENCE:
        rc = av_opt_set(c->priv_data, "compression", "zip16", 0);
        if (rc >= 0) rc = av_opt_set(c->priv_data, "format", "float", 0);
        break;
    default:
        break;
    }
    if (rc >= 0 && output->bitrate && sr_codec_info(output->codec)->rate_control)
        c->bit_rate = (int64_t)output->bitrate;
    return rc < 0 ? av_fail(diag, rc, "cannot configure video encoder") : SR_OK;
}

static SrStatus open_video(SrEncoder *e, const SrScene *scene, unsigned threads,
                           SrDiagnostics *diag) {
    const SrOutput *output = e->output;
    const SrCodecInfo *info = sr_codec_info(output->codec);
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
    e->bits = sr_output_input_bits(output);
    e->convert = output->codec == SR_CODEC_GIF ? CONVERT_PALETTE
               : e->bits == 32 ? CONVERT_FLOAT_PLANES : CONVERT_SCALE;
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
    /* 1.1 fixes codec workers independently of render/scaler workers. Keep
     * the legacy policy for 1.0 byte identity. Resume fingerprints this.
     * Codecs added in 1.1 are pinned whatever the document version. */
    unsigned codec_threads = scene->format_version >= 11 || !info->legacy
        ? 1 : threads;
    c->thread_count = (int)codec_threads;
    c->color_range = output->full_range ? AVCOL_RANGE_JPEG : AVCOL_RANGE_MPEG;
    int matrix;
    color_tags(output->color_space, c, &matrix);
    if (!info->legacy && (rgb_format(format) || format == AV_PIX_FMT_PAL8)) {
        /* RGB carries no Y'CbCr matrix and is full range (1.0 codecs keep
         * their 1.0 tags). */
        c->colorspace = AVCOL_SPC_RGB;
        c->color_range = AVCOL_RANGE_JPEG;
    }
    c->flags |= AV_CODEC_FLAG_BITEXACT;
    if (e->fmt && e->fmt->oformat->flags & AVFMT_GLOBALHEADER)
        c->flags |= AV_CODEC_FLAG_GLOBAL_HEADER;
    if (output->keyframe_interval > 0.0 && info->gop) {
        double frames = floor(output->keyframe_interval * scene->project.fps_num /
                              scene->project.fps_den + 0.5);
        c->gop_size = frames < 1.0 ? 1 : frames > 1e6 ? 1000000 : (int)frames;
    }
    if (output->b_frames >= 0 && info->b_frames) c->max_b_frames = output->b_frames;
    if (output->codec == SR_CODEC_FFV1) {
        /* Version 3 with per-slice CRCs: the threaded, checksummed layout. */
        c->level = 3;
        av_opt_set_int(c->priv_data, "slicecrc", 1, 0);
    } else if (info->legacy) {
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
                     "pools=%u:frame-threads=1:log-level=%s", codec_threads,
                     diag && diag->verbose ? "info" : "error");
            int rc = av_opt_set(c->priv_data, "x265-params", params, 0);
            if (rc < 0) return av_fail(diag, rc, "cannot configure x265 workers");
        }
    } else {
        SrStatus status = configure_new_codec(e, c, diag);
        if (status != SR_OK) return status;
    }
    if (output->codec == SR_CODEC_AV1 && !(diag && diag->verbose))
        setenv("SVT_LOG", "1", 0);    /* SVT-AV1 prints its banner otherwise */
    int rc = avcodec_open2(c, codec, NULL);
    if (rc < 0) {
        char what[160];
        snprintf(what, sizeof(what), "cannot open video encoder '%s'", name);
        return av_fail(diag, rc, what);
    }
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
    if (e->convert == CONVERT_PALETTE) {
        SrStatus status = sr_gif_palette_create(&e->palette, e->width, e->height);
        return status == SR_OK ? SR_OK
            : av_fail(diag, AVERROR(ENOMEM), "cannot allocate palette");
    }
    if (e->convert == CONVERT_FLOAT_PLANES) return SR_OK;
    e->src_format = e->bits == 16 ? AV_PIX_FMT_RGBA64 : AV_PIX_FMT_RGBA;
    rc = open_scaler(e, c, threads);
    if (rc < 0) return av_fail(diag, rc, "cannot set up RGB to Y'CbCr conversion");
    const int *coefficients = sws_getCoefficients(matrix);
    /* Source is full-range RGB; the destination range follows colorRange. */
    if (sws_setColorspaceDetails(e->sws, coefficients, 1, coefficients,
                                 c->color_range == AVCOL_RANGE_JPEG ? 1 : 0, 0,
                                 1 << 16, 1 << 16) < 0)
        return av_fail(diag, 0, "cannot configure RGB to Y'CbCr conversion");
    return SR_OK;
}

static SrStatus open_audio(SrEncoder *e, const SrScene *scene,
                           const SrEncoderAudio *audio, SrDiagnostics *diag) {
    (void)scene;
    const SrOutput *output = e->output;
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
    bool fixed_frames = a->frame_size > 0 &&
                        !(codec->capabilities & AV_CODEC_CAP_VARIABLE_FRAME_SIZE);
    e->stage_capacity = fixed_frames ? a->frame_size : 1024;
    e->audio_frame_size = fixed_frames ? a->frame_size : 0;
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
                             bool final_output, SrDiagnostics *diag) {
    e->vstream = avformat_new_stream(e->fmt, NULL);
    if (!e->vstream) return av_fail(diag, AVERROR(ENOMEM), "cannot add video stream");
    e->vstream->time_base = e->frame_tb;
    e->vstream->avg_frame_rate = (AVRational){e->frame_tb.den, e->frame_tb.num};
    int rc = e->copy_par
        ? avcodec_parameters_copy(e->vstream->codecpar, e->copy_par)
        : avcodec_parameters_from_context(e->vstream->codecpar, e->video);
    if (rc < 0) return av_fail(diag, rc, "cannot configure video stream");
    /* A copied tag belongs to the segment's container; let the muxer pick. */
    e->vstream->codecpar->codec_tag = 0;
    if (e->output->codec == SR_CODEC_H265 && strcmp(container, "matroska"))
        e->vstream->codecpar->codec_tag = MKTAG('h', 'v', 'c', '1');
    if (final_output && scene->project.mode == SR_MODE_EQUIRECTANGULAR &&
        e->output->spherical_metadata &&
        (e->matroska || !strcmp(container, "mp4") || !strcmp(container, "mov"))) {
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
    bool embed = final_output && e->output->embed_metadata && scene->metadata_count;
    if (embed) {
        SrStatus status = sr_metadata_apply(scene, e->fmt, container, diag);
        if (status != SR_OK) return status;
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
    rc = 0;
    if (!strcmp(container, "mp4") || !strcmp(container, "mov")) {
        const char *flags = e->output->faststart
            ? (embed ? "+faststart+use_metadata_tags" : "+faststart")
            : (embed ? "+use_metadata_tags" : NULL);
        if (flags) rc = av_dict_set(&options, "movflags", flags, 0);
        if (rc >= 0 && embed) rc = av_dict_set(&options, "write_tmcd", "0", 0);
    } else if (!strcmp(container, "gif")) {
        /* loopCount counts plays; the GIF loop value counts repeats. */
        uint32_t plays = e->output->loop_count;
        rc = av_dict_set_int(&options, "loop", plays == 0 ? 0 :
                             plays == 1 ? -1 : (int64_t)plays - 1, 0);
    } else if (!strcmp(container, "apng")) {
        rc = av_dict_set_int(&options, "plays", e->output->loop_count, 0);
    }
    if (rc < 0) {
        av_dict_free(&options);
        return av_fail(diag, rc, "cannot allocate container options");
    }
    rc = avformat_write_header(e->fmt, &options);
    av_dict_free(&options);
    if (rc < 0) return av_fail(diag, rc, "cannot write container header");
    e->header_written = true;
    return embed ? sr_metadata_check(scene, e->fmt, container, diag) : SR_OK;
}

typedef enum { OPEN_FULL, OPEN_SEGMENT, OPEN_COPY } OpenMode;

/* Stream parameters of the video stream of `path` (a finished segment). */
static SrStatus read_video_params(SrEncoder *e, const SrScene *scene,
                                  const char *path, SrDiagnostics *diag) {
    const SrOutput *output = e->output;
    AVFormatContext *in = NULL;
    int rc = avformat_open_input(&in, path, NULL, NULL);
    if (rc >= 0) rc = avformat_find_stream_info(in, NULL);
    int index = rc >= 0 ? av_find_best_stream(in, AVMEDIA_TYPE_VIDEO, -1, -1,
                                              NULL, 0) : rc;
    SrStatus status = SR_OK;
    if (index < 0) {
        char message[AV_ERROR_MAX_STRING_SIZE];
        av_strerror(index, message, sizeof(message));
        sr_diag_error(diag, 0, NULL, NULL, "cannot read segment '%s': %s",
                      path, message);
        status = index == AVERROR(ENOMEM) ? SR_ERR_MEMORY : SR_ERR_ENCODER;
    } else {
        const AVCodecParameters *par = in->streams[index]->codecpar;
        if (par->width != (int)scene->project.width ||
            par->height != (int)scene->project.height) {
            sr_diag_error(diag, 0, NULL, NULL,
                          "segment '%s' is %dx%d, expected %ux%u", path,
                          par->width, par->height, scene->project.width,
                          scene->project.height);
            status = SR_ERR_ENCODER;
        } else {
            e->copy_par = avcodec_parameters_alloc();
            rc = e->copy_par ? avcodec_parameters_copy(e->copy_par, par)
                             : AVERROR(ENOMEM);
            if (rc < 0) status = av_fail(diag, rc, "cannot copy segment parameters");
            /* The demuxer measured the segment's bit rate; declare what the
             * encoder itself declares (the configured rate, or none). */
            else
                e->copy_par->bit_rate = output->codec == SR_CODEC_FFV1
                    ? 0 : (int64_t)output->bitrate;
        }
    }
    avformat_close_input(&in);
    return status;
}

static SrStatus open_encoder(SrEncoder **out, const SrScene *scene,
                             const SrOutput *output, const char *path,
                             unsigned threads, const SrEncoderAudio *audio,
                             OpenMode mode, const char *video_template,
                             SrDiagnostics *diag) {
    if (!out) return SR_ERR_ARGUMENT;
    *out = NULL;
    if (!scene || !output || !path || !scene->project.width ||
        !scene->project.height ||
        scene->project.width > INT32_MAX || scene->project.height > INT32_MAX ||
        !scene->project.fps_num || !scene->project.fps_den ||
        scene->project.fps_num > INT32_MAX || scene->project.fps_den > INT32_MAX ||
        !output->pixel_format || !output->preset ||
        (audio && audio->channels &&
         (audio->channels > 2 || audio->sample_rate < 8000 ||
          audio->sample_rate > INT32_MAX)))
        return SR_ERR_ARGUMENT;
    const SrCodecInfo *info = sr_codec_info(output->codec);
    bool sequence = info->sequence;
    const char *why = NULL;
    const char *container = sequence ? NULL : sr_output_muxer(output, path, &why);
    if (sequence && (mode != OPEN_FULL || !sr_sequence_pattern_valid(path))) {
        sr_diag_error(diag, output->source_line, "output", "path",
                      mode != OPEN_FULL ? "image sequences cannot be resumed"
                                        : "invalid sequence pattern '%s'", path);
        return SR_ERR_ARGUMENT;
    }
    if (!sequence && !container) {
        if (info->legacy && output->container == SR_CONTAINER_AUTO)
            sr_diag_error(diag, 0, NULL, NULL,
                          "output '%s' needs a .mp4, .mov or .mkv extension", path);
        else
            sr_diag_error(diag, output->source_line, "output", "container",
                          "output '%s' %s", path, why ? why : "has no container");
        return SR_ERR_ARGUMENT;
    }
    if (output->codec == SR_CODEC_FFV1 && strcmp(container, "matroska")) {
        sr_diag_error(diag, output->source_line, "output", "codec",
                      "ffv1 requires a .mkv (Matroska) output");
        return SR_ERR_ARGUMENT;
    }
    if (info->muxer && mode != OPEN_FULL) {
        sr_diag_error(diag, output->source_line, "output", "codec",
                      "%s output cannot be resumed", info->name);
        return SR_ERR_ARGUMENT;
    }
    if (mode != OPEN_SEGMENT) {
        SrStatus status = sr_encoder_validate_output_metadata(scene, output, path,
                                                              diag);
        if (status != SR_OK) return status;
    }
    if (threads == 0) threads = sr_parallel_thread_count(0, SIZE_MAX);
    SrEncoder *e = calloc(1, sizeof(*e));
    if (!e) return SR_ERR_MEMORY;
    e->output = output;
    e->muxer = container;
    e->width = scene->project.width;
    e->height = scene->project.height;
    e->matroska = container && (!strcmp(container, "matroska") ||
                                !strcmp(container, "webm"));
    e->frame_tb = (AVRational){(int)scene->project.fps_den,
                               (int)scene->project.fps_num};
    SrStatus status = SR_OK;
    if (sequence) {
        e->pattern = sr_strdup(path);
        if (!e->pattern) status = SR_ERR_MEMORY;
    } else {
        int rc = avformat_alloc_output_context2(&e->fmt, NULL, container, path);
        if (rc < 0 || !e->fmt)
            status = av_fail(diag, rc < 0 ? rc : AVERROR(ENOMEM),
                             "cannot allocate output container");
        if (status == SR_OK) e->fmt->flags |= AVFMT_FLAG_BITEXACT;
    }
    if (status == SR_OK) {
        e->pkt = av_packet_alloc();
        if (!e->pkt) status = av_fail(diag, AVERROR(ENOMEM), "cannot allocate packet");
    }
    if (status == SR_OK)
        status = mode == OPEN_COPY
            ? read_video_params(e, scene, video_template, diag)
            : open_video(e, scene, threads, diag);
    if (status == SR_OK && mode != OPEN_SEGMENT && audio && audio->channels &&
        info->audio)
        status = open_audio(e, scene, audio, diag);
    if (status == SR_OK && !sequence)
        status = open_streams(e, scene, path, container, mode != OPEN_SEGMENT,
                              diag);
    if (status != SR_OK) {
        sr_encoder_destroy(e);
        return status;
    }
    sr_diag_info(diag, "encoding in-process: %s %s -> %s", sr_encoder_name(e),
                 output->pixel_format, container ? container : "image sequence");
    *out = e;
    return SR_OK;
}

SrStatus sr_encoder_open(SrEncoder **out, const SrScene *scene,
                         const char *path, unsigned threads,
                         const SrEncoderAudio *audio, SrDiagnostics *diag) {
    return open_encoder(out, scene, scene ? &scene->output : NULL, path, threads,
                        audio, OPEN_FULL, NULL, diag);
}

SrStatus sr_encoder_open_output(SrEncoder **out, const SrScene *scene,
                                const SrOutput *output, const char *path,
                                unsigned threads, const SrEncoderAudio *audio,
                                SrDiagnostics *diag) {
    return open_encoder(out, scene, output, path, threads, audio, OPEN_FULL,
                        NULL, diag);
}

SrStatus sr_encoder_open_segment(SrEncoder **out, const SrScene *scene,
                                 const char *path, unsigned threads,
                                 SrDiagnostics *diag) {
    return open_encoder(out, scene, scene ? &scene->output : NULL, path, threads,
                        NULL, OPEN_SEGMENT, NULL, diag);
}

SrStatus sr_encoder_open_segment_output(SrEncoder **out, const SrScene *scene,
                                        const SrOutput *output,
                                        const char *path, unsigned threads,
                                        SrDiagnostics *diag) {
    return open_encoder(out, scene, output, path, threads, NULL, OPEN_SEGMENT,
                        NULL, diag);
}

SrStatus sr_encoder_open_copy(SrEncoder **out, const SrScene *scene,
                              const char *path, const char *video_template,
                              const SrEncoderAudio *audio, SrDiagnostics *diag) {
    return sr_encoder_open_copy_output(out, scene, scene ? &scene->output : NULL,
                                       path, video_template, audio, diag);
}

SrStatus sr_encoder_open_copy_output(SrEncoder **out, const SrScene *scene,
                                     const SrOutput *output, const char *path,
                                     const char *video_template,
                                     const SrEncoderAudio *audio,
                                     SrDiagnostics *diag) {
    if (!video_template) {
        if (out) *out = NULL;
        return SR_ERR_ARGUMENT;
    }
    return open_encoder(out, scene, output, path, 1, audio, OPEN_COPY,
                        video_template, diag);
}

void sr_encoder_set_first_number(SrEncoder *encoder, uint64_t number) {
    if (encoder) encoder->first_number = number;
}

bool sr_encoder_has_audio(const SrEncoder *encoder) {
    return encoder && encoder->audio;
}

SrStatus sr_encoder_copy_video(SrEncoder *e, const char *segment_path,
                               uint64_t first_frame, uint64_t frame_count,
                               SrDiagnostics *diag) {
    if (!e || e->closing || !e->copy_par || !segment_path ||
        first_frame > (uint64_t)INT64_MAX / 2 || frame_count > (uint64_t)INT32_MAX)
        return SR_ERR_ARGUMENT;
    double start = sr_monotonic_seconds();
    AVFormatContext *in = NULL;
    int rc = avformat_open_input(&in, segment_path, NULL, NULL);
    if (rc >= 0) rc = avformat_find_stream_info(in, NULL);
    int index = rc >= 0 ? av_find_best_stream(in, AVMEDIA_TYPE_VIDEO, -1, -1,
                                              NULL, 0) : rc;
    if (index < 0) {
        char message[AV_ERROR_MAX_STRING_SIZE];
        av_strerror(index, message, sizeof(message));
        sr_diag_error(diag, 0, NULL, NULL, "cannot read segment '%s': %s",
                      segment_path, message);
        avformat_close_input(&in);
        e->seconds += sr_monotonic_seconds() - start;
        return index == AVERROR(ENOMEM) ? SR_ERR_MEMORY : SR_ERR_ENCODER;
    }
    const AVCodecParameters *par = in->streams[index]->codecpar;
    const AVCodecParameters *want = e->copy_par;
    SrStatus status = SR_OK;
    if (par->codec_id != want->codec_id || par->width != want->width ||
        par->height != want->height || par->format != want->format ||
        par->extradata_size != want->extradata_size ||
        (want->extradata_size &&
         memcmp(par->extradata, want->extradata, (size_t)want->extradata_size))) {
        sr_diag_error(diag, 0, NULL, NULL,
                      "segment '%s' has different stream parameters than the first",
                      segment_path);
        status = SR_ERR_ENCODER;
    }
    AVRational in_tb = in->streams[index]->time_base;
    uint64_t packets = 0;
    /* Timestamps go to frame units first (rounding absorbs the segment
     * container's time base), are shifted by the segment's first frame,
     * then land in the output stream's time base exactly as encoded
     * packets do; every packet lasts one frame. */
    while (status == SR_OK && (rc = av_read_frame(in, e->pkt)) >= 0) {
        if (e->pkt->stream_index != index) {
            av_packet_unref(e->pkt);
            continue;
        }
        int64_t offset = (int64_t)first_frame;
        if (e->pkt->pts != AV_NOPTS_VALUE)
            e->pkt->pts = av_rescale_q(e->pkt->pts, in_tb, e->frame_tb) + offset;
        if (e->pkt->dts != AV_NOPTS_VALUE)
            e->pkt->dts = av_rescale_q(e->pkt->dts, in_tb, e->frame_tb) + offset;
        e->pkt->duration = 1;
        e->pkt->pos = -1;
        av_packet_rescale_ts(e->pkt, e->frame_tb, e->vstream->time_base);
        e->pkt->stream_index = e->vstream->index;
        ++packets;
        rc = av_interleaved_write_frame(e->fmt, e->pkt);
        if (rc < 0) status = av_fail(diag, rc, "cannot write copied video packet");
    }
    if (status == SR_OK && rc != AVERROR_EOF)
        status = av_fail(diag, rc, "cannot read segment packets");
    if (status == SR_OK && packets != frame_count) {
        sr_diag_error(diag, 0, NULL, NULL,
                      "segment '%s' holds %llu frames, expected %llu", segment_path,
                      (unsigned long long)packets, (unsigned long long)frame_count);
        status = SR_ERR_ENCODER;
    }
    av_packet_unref(e->pkt);
    avformat_close_input(&in);
    e->seconds += sr_monotonic_seconds() - start;
    return status;
}

SrStatus sr_encoder_check_segment(const SrScene *scene, const char *path,
                                  uint64_t frame_count, char *why,
                                  size_t why_size) {
    return sr_encoder_check_segment_output(scene, scene ? &scene->output : NULL,
                                           path, frame_count, why, why_size);
}

SrStatus sr_encoder_check_segment_output(const SrScene *scene,
                                         const SrOutput *output,
                                         const char *path, uint64_t frame_count,
                                         char *why, size_t why_size) {
    if (why && why_size) why[0] = '\0';
    if (!scene || !output || !path) return SR_ERR_ARGUMENT;
    const AVCodec *codec =
        avcodec_find_encoder_by_name(video_encoder_name(output->codec));
    enum AVPixelFormat format = output->pixel_format
        ? av_get_pix_fmt(output->pixel_format) : AV_PIX_FMT_NONE;
    AVFormatContext *in = NULL;
    AVPacket *pkt = NULL;
    int rc = avformat_open_input(&in, path, NULL, NULL);
    if (rc >= 0) rc = avformat_find_stream_info(in, NULL);
    SrStatus status = SR_OK;
    if (rc < 0) {
        char message[AV_ERROR_MAX_STRING_SIZE];
        av_strerror(rc, message, sizeof(message));
        snprintf(why, why_size, "cannot be read: %s", message);
        status = rc == AVERROR(ENOMEM) ? SR_ERR_MEMORY : SR_ERR_ENCODER;
    } else if (in->nb_streams != 1 ||
               in->streams[0]->codecpar->codec_type != AVMEDIA_TYPE_VIDEO) {
        snprintf(why, why_size, "does not hold exactly one video stream");
        status = SR_ERR_ENCODER;
    } else {
        const AVCodecParameters *par = in->streams[0]->codecpar;
        if (!codec || par->codec_id != codec->id ||
            par->width != (int)scene->project.width ||
            par->height != (int)scene->project.height || par->format != format) {
            snprintf(why, why_size, "stream parameters differ from the output "
                     "settings");
            status = SR_ERR_ENCODER;
        }
    }
    if (status == SR_OK) {
        pkt = av_packet_alloc();
        if (!pkt) status = SR_ERR_MEMORY;
    }
    uint64_t packets = 0;
    while (status == SR_OK && (rc = av_read_frame(in, pkt)) >= 0) {
        if (packets == 0 && !(pkt->flags & AV_PKT_FLAG_KEY)) {
            snprintf(why, why_size, "does not start with a keyframe");
            status = SR_ERR_ENCODER;
        }
        ++packets;
        av_packet_unref(pkt);
    }
    if (status == SR_OK && rc != AVERROR_EOF) {
        char message[AV_ERROR_MAX_STRING_SIZE];
        av_strerror(rc, message, sizeof(message));
        snprintf(why, why_size, "cannot be read to the end: %s", message);
        status = rc == AVERROR(ENOMEM) ? SR_ERR_MEMORY : SR_ERR_ENCODER;
    }
    if (status == SR_OK && packets != frame_count) {
        snprintf(why, why_size, "holds %llu frames, expected %llu",
                 (unsigned long long)packets, (unsigned long long)frame_count);
        status = SR_ERR_ENCODER;
    }
    av_packet_free(&pkt);
    avformat_close_input(&in);
    return status;
}

unsigned sr_encoder_bits(const SrEncoder *encoder) {
    return encoder ? encoder->bits : 8;
}

double sr_encoder_seconds(const SrEncoder *encoder) {
    return encoder ? encoder->seconds : 0.0;
}

const char *sr_encoder_name(const SrEncoder *encoder) {
    if (encoder && encoder->copy_par) return "copy";
    return encoder && encoder->video && encoder->video->codec
               ? encoder->video->codec->name : "none";
}

/* A fixed-frame audio codec pads the last frame to a whole frame. Matroska
 * only drops that padding on playback when the block carries DiscardPadding,
 * which the muxer writes from skip-samples side data: every packet whose
 * nominal frame [pts, pts + frame_size) extends past the last real sample
 * (e->next_apts, in 1/sample_rate) says how much of its end to discard.
 * MP4 needs nothing: the packet duration the encoder set (kept by the
 * rescale below) ends the track at the last real sample. */
static int mark_audio_padding(SrEncoder *e, AVPacket *pkt) {
    if (!e->matroska || !e->audio_frame_size || pkt->pts == AV_NOPTS_VALUE)
        return 0;
    int64_t end = pkt->pts + e->audio_frame_size;
    if (end <= e->next_apts) return 0;
    int64_t discard = end - e->next_apts;
    if (discard > e->audio_frame_size) discard = e->audio_frame_size;
    uint8_t *side = av_packet_new_side_data(pkt, AV_PKT_DATA_SKIP_SAMPLES, 10);
    if (!side) return AVERROR(ENOMEM);
    AV_WL32(side, 0);
    AV_WL32(side + 4, (uint32_t)discard);
    side[8] = side[9] = 0;
    return 0;
}

/* One image-sequence file: written under a temporary name next to it and
 * renamed, so a frame file exists only when complete. */
static int write_sequence_packet(SrEncoder *e, const AVPacket *pkt) {
    char name[4096], temporary[4200];
    uint64_t number = e->first_number + (uint64_t)(pkt->pts < 0 ? 0 : pkt->pts);
    if (pkt->pts < 0 || !sr_sequence_format(e->pattern, number, name, sizeof(name)))
        return AVERROR(EINVAL);
    snprintf(temporary, sizeof(temporary), "%s.%ld.tmp", name, (long)getpid());
    int fd = open(temporary, O_WRONLY | O_CREAT | O_TRUNC | O_NOFOLLOW | O_CLOEXEC,
                  0644);
    bool ok = fd >= 0;
    size_t done = 0, size = (size_t)pkt->size;
    while (ok && done < size) {
        ssize_t count = write(fd, pkt->data + done, size - done);
        if (count < 0 && errno == EINTR) continue;
        if (count <= 0) ok = false;
        else done += (size_t)count;
    }
    if (fd >= 0 && close(fd) != 0) ok = false;
    if (ok && rename(temporary, name) != 0) ok = false;
    if (!ok) {
        int error = errno ? errno : EIO;
        if (fd >= 0) unlink(temporary);
        snprintf(e->failed_file, sizeof(e->failed_file), "%.500s", name);
        return AVERROR(error);
    }
    return 0;
}

/* Writes every packet the codec has ready. Video packets last one frame:
 * encoders leave the duration unset, and muxers then shorten the last one.
 * Audio packets keep the duration the encoder gave them (the last one is
 * short when the final frame was). */
static int drain(SrEncoder *e, AVCodecContext *codec, AVStream *stream) {
    for (;;) {
        int rc = avcodec_receive_packet(codec, e->pkt);
        if (rc == AVERROR(EAGAIN) || rc == AVERROR_EOF) return 0;
        if (rc < 0) return rc;
        if (codec == e->video) {
            e->pkt->duration = 1;
            /* x265 4.2 initializes its reorder delay at the third input
             * frame. Short flushes can return an uninitialized DTS; these
             * one/two-frame streams have no reordered pictures. */
            if (codec->codec_id == AV_CODEC_ID_HEVC && e->closing &&
                e->next_pts <= 2)
                e->pkt->dts = e->pkt->pts;
        } else if (e->closing) {
            rc = mark_audio_padding(e, e->pkt);
            if (rc < 0) {
                av_packet_unref(e->pkt);
                return rc;
            }
        }
        if (e->pattern) {
            rc = write_sequence_packet(e, e->pkt);
            av_packet_unref(e->pkt);
            if (rc < 0) return rc;
            continue;
        }
        av_packet_rescale_ts(e->pkt, codec->time_base, stream->time_base);
        e->pkt->stream_index = stream->index;
        rc = av_interleaved_write_frame(e->fmt, e->pkt);
        if (rc < 0) return rc;
    }
}

static void borrowed_free(void *opaque, uint8_t *data) {
    (void)opaque;
    (void)data;
}

/* Linear float RGBA (premultiplied) into the GBR(A) float planes of an
 * EXR frame: plane 0 G, 1 B, 2 R, 3 A. */
static void fill_float_planes(SrEncoder *e, const float *rgba) {
    AVFrame *f = e->frame;
    bool alpha = f->data[3] != NULL;
    for (uint32_t y = 0; y < e->height; ++y) {
        const float *s = rgba + (size_t)y * e->width * 4;
        float *g = (float *)(f->data[0] + (ptrdiff_t)f->linesize[0] * y);
        float *b = (float *)(f->data[1] + (ptrdiff_t)f->linesize[1] * y);
        float *r = (float *)(f->data[2] + (ptrdiff_t)f->linesize[2] * y);
        float *a = alpha ? (float *)(f->data[3] + (ptrdiff_t)f->linesize[3] * y)
                         : NULL;
        for (uint32_t x = 0; x < e->width; ++x, s += 4) {
            r[x] = s[0];
            g[x] = s[1];
            b[x] = s[2];
            if (a) a[x] = s[3];
        }
    }
}

SrStatus sr_encoder_write_video(SrEncoder *e, const void *rgba,
                                SrDiagnostics *diag) {
    if (!e || e->closing || !rgba || !e->video) return SR_ERR_ARGUMENT;
    double start = sr_monotonic_seconds();
    int rc = av_frame_make_writable(e->frame);
    if (rc < 0) return av_fail(diag, rc, "cannot prepare video frame");
    int stride = (int)(e->width * 4 * (e->bits / 8));
    if (e->convert == CONVERT_FLOAT_PLANES) {
        fill_float_planes(e, rgba);
        rc = 1;
    } else if (e->convert == CONVERT_PALETTE) {
        sr_gif_palette_quantize(e->palette, rgba, e->frame->data[0],
                                e->frame->linesize[0],
                                (uint32_t *)e->frame->data[1]);
        rc = 1;
    } else if (e->sws_threaded) {
        /* A borrowed view of the caller's pixels: a buffer reference whose
         * free does nothing, so swscale references it instead of copying. */
        e->src->buf[0] = av_buffer_create((uint8_t *)rgba,
                                          (size_t)stride * e->height,
                                          borrowed_free, NULL,
                                          AV_BUFFER_FLAG_READONLY);
        if (!e->src->buf[0]) {
            e->seconds += sr_monotonic_seconds() - start;
            return av_fail(diag, AVERROR(ENOMEM), "cannot prepare video frame");
        }
        e->src->data[0] = (uint8_t *)rgba;
        e->src->linesize[0] = stride;
        rc = sws_scale_frame(e->sws, e->frame, e->src);
        av_buffer_unref(&e->src->buf[0]);
        e->src->data[0] = NULL;
        if (rc == 0) rc = 1;    /* success: 0 or more */
    } else {
        const uint8_t *source[4] = {rgba, NULL, NULL, NULL};
        const int strides[4] = {stride, 0, 0, 0};
        rc = sws_scale(e->sws, source, strides, 0, (int)e->height, e->frame->data,
                       e->frame->linesize);   /* success: rows written */
    }
    if (rc <= 0) {
        e->seconds += sr_monotonic_seconds() - start;
        return av_fail(diag, rc < 0 ? rc : 0, "RGB to Y'CbCr conversion failed");
    }
    e->frame->pts = e->next_pts++;
    rc = avcodec_send_frame(e->video, e->frame);
    if (rc >= 0) rc = drain(e, e->video, e->vstream);
    e->seconds += sr_monotonic_seconds() - start;
    if (rc < 0 && e->failed_file[0]) {
        char what[640];
        snprintf(what, sizeof(what), "cannot write frame file '%s'", e->failed_file);
        SrStatus status = av_fail(diag, rc, what);
        return status == SR_ERR_ENCODER ? SR_ERR_IO : status;
    }
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
    if (!e || e->closing || !e->audio || (!pcm && samples)) return SR_ERR_ARGUMENT;
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

/* Writes the trailer once (MP4 needs it for its moov box). */
static int write_trailer(SrEncoder *e) {
    if (!e->header_written || e->trailer_written) return 0;
    e->trailer_written = true;
    return av_write_trailer(e->fmt);
}

SrStatus sr_encoder_finish(SrEncoder *e, SrDiagnostics *diag) {
    if (!e || e->finished) return SR_ERR_ARGUMENT;
    if (e->closing) {
        sr_diag_error(diag, 0, NULL, NULL,
                      "encoder cannot be finished again after a failed finish");
        return SR_ERR_ENCODER;
    }
    double start = sr_monotonic_seconds();
    e->closing = true;
    /* Every step runs even after an earlier one failed, so the container
     * is still closed properly; the first failure is the one returned. */
    SrStatus status = SR_OK;
    int rc = 0;
    if (e->video) {
        rc = avcodec_send_frame(e->video, NULL);
        if (rc >= 0) rc = drain(e, e->video, e->vstream);
        if (rc < 0 && e->failed_file[0]) {
            sr_diag_error(diag, 0, NULL, NULL, "cannot write frame file '%s'",
                          e->failed_file);
            status = SR_ERR_IO;
        } else if (rc < 0) {
            status = av_fail(diag, rc, "cannot flush video encoder");
        }
    }
    if (e->audio) {
        rc = e->staged > 0 ? flush_stage(e) : 0;
        if (rc >= 0) rc = avcodec_send_frame(e->audio, NULL);
        if (rc >= 0) rc = drain(e, e->audio, e->astream);
        if (rc < 0) {
            SrStatus audio = av_fail(diag, rc, "cannot flush audio encoder");
            if (status == SR_OK) status = audio;
        }
    }
    rc = e->fmt ? write_trailer(e) : 0;
    if (rc < 0) {
        SrStatus trailer = av_fail(diag, rc, "cannot finish container");
        if (status == SR_OK) status = trailer;
    }
    rc = e->fmt && e->fmt->pb ? avio_closep(&e->fmt->pb) : 0;
    e->seconds += sr_monotonic_seconds() - start;
    if (rc < 0) {
        av_fail(diag, rc, "cannot close output file");
        if (status == SR_OK) status = SR_ERR_IO;
    }
    if (status == SR_OK) e->finished = true;
    return status;
}

void sr_encoder_destroy(SrEncoder *e) {
    if (!e) return;
    /* An unfinished file still gets its trailer (an MP4 without moov is
     * unreadable); errors here have nowhere to go. */
    if (e->fmt && e->fmt->pb) (void)write_trailer(e);
    if (e->fmt && e->fmt->pb) avio_closep(&e->fmt->pb);
    avformat_free_context(e->fmt);
    avcodec_free_context(&e->video);
    avcodec_free_context(&e->audio);
    sws_freeContext(e->sws);
    swr_free(&e->swr);
    av_frame_free(&e->frame);
    av_frame_free(&e->src);
    av_frame_free(&e->aframe);
    av_packet_free(&e->pkt);
    avcodec_parameters_free(&e->copy_par);
    sr_gif_palette_free(e->palette);
    free(e->pattern);
    free(e->stage);
    free(e);
}
