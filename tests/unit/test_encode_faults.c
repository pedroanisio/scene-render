/* SPDX-License-Identifier: Apache-2.0 */
/* Fault injection at the libav boundary. The test binary links with
 * -Wl,--wrap for every libav function the core calls (CMakeLists.txt,
 * Makefile), so a chosen call can be made to fail. Each failure must come
 * back as a clean status with a diagnostic, and the sanitizer build proves
 * the partially built encoder/decoder is released on every path. */
#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/frame.h>
#include <libswresample/swresample.h>
#include <libswscale/swscale.h>

#include <stdlib.h>

#include "harness.h"
#include "scene_render/audio.h"
#include "scene_render/encoder.h"
#include "scene_render/video.h"

static const char *g_fault;     /* function to fail, NULL = none */
static int g_skip;              /* matching calls to let through first */
static int g_rc;                /* avcodec_receive_frame: forced code */

static bool inject(const char *fn) {
    if (!g_fault || strcmp(g_fault, fn) != 0) return false;
    if (g_skip > 0) {
        --g_skip;
        return false;
    }
    g_fault = NULL;
    return true;
}

static void arm(const char *fn, int skip) {
    g_fault = fn;
    g_skip = skip;
}

static void disarm(void) {
    g_fault = NULL;
    g_skip = 0;
    g_rc = 0;
}

#define WRAP(ret, name, params, args, fail_value)                        \
    ret __real_##name params;                                             \
    ret __wrap_##name params;                                             \
    ret __wrap_##name params {                                            \
        return inject(#name) ? (fail_value) : __real_##name args;         \
    }

WRAP(const AVCodec *, avcodec_find_encoder_by_name, (const char *n), (n), NULL)
WRAP(AVCodecContext *, avcodec_alloc_context3, (const AVCodec *c), (c), NULL)
WRAP(int, avcodec_open2, (AVCodecContext *c, const AVCodec *k, AVDictionary **o),
     (c, k, o), AVERROR(EINVAL))
WRAP(AVStream *, avformat_new_stream, (AVFormatContext *s, const AVCodec *c), (s, c), NULL)
WRAP(int, avcodec_parameters_from_context,
     (AVCodecParameters *p, const AVCodecContext *c), (p, c), AVERROR(EINVAL))
WRAP(int, avio_open, (AVIOContext **s, const char *u, int f), (s, u, f), AVERROR(EACCES))
WRAP(int, avformat_write_header, (AVFormatContext *s, AVDictionary **o), (s, o),
     AVERROR(EIO))
WRAP(struct SwsContext *, sws_getContext,
     (int sw, int sh, enum AVPixelFormat sf, int dw, int dh, enum AVPixelFormat df,
      int fl, SwsFilter *a, SwsFilter *b, const double *p),
     (sw, sh, sf, dw, dh, df, fl, a, b, p), NULL)
WRAP(AVFrame *, av_frame_alloc, (void), (), NULL)
WRAP(AVPacket *, av_packet_alloc, (void), (), NULL)
WRAP(int, av_frame_get_buffer, (AVFrame *f, int align), (f, align), AVERROR(ENOMEM))
WRAP(int, av_frame_make_writable, (AVFrame *f), (f), AVERROR(ENOMEM))
WRAP(int, avcodec_send_frame, (AVCodecContext *c, const AVFrame *f), (c, f),
     AVERROR(EINVAL))
WRAP(int, avcodec_receive_packet, (AVCodecContext *c, AVPacket *p), (c, p), AVERROR(EIO))
WRAP(int, av_interleaved_write_frame, (AVFormatContext *s, AVPacket *p), (s, p),
     AVERROR(EIO))
WRAP(int, av_write_trailer, (AVFormatContext *s), (s), AVERROR(EIO))
WRAP(int, avformat_open_input,
     (AVFormatContext **ps, const char *u, const AVInputFormat *f, AVDictionary **o),
     (ps, u, f, o), AVERROR(ENOENT))
WRAP(int, avformat_find_stream_info, (AVFormatContext *s, AVDictionary **o), (s, o),
     AVERROR_INVALIDDATA)
WRAP(int, av_find_best_stream,
     (AVFormatContext *s, enum AVMediaType ty, int w, int r, const AVCodec **d, int fl),
     (s, ty, w, r, d, fl), AVERROR_STREAM_NOT_FOUND)
WRAP(const AVCodec *, avcodec_find_decoder, (enum AVCodecID id), (id), NULL)
WRAP(int, avcodec_parameters_to_context,
     (AVCodecContext *c, const AVCodecParameters *p), (c, p), AVERROR(EINVAL))
WRAP(int, av_read_frame, (AVFormatContext *s, AVPacket *p), (s, p), AVERROR(EIO))
WRAP(int, avformat_seek_file,
     (AVFormatContext *s, int i, int64_t a, int64_t b, int64_t c, int f),
     (s, i, a, b, c, f), AVERROR(EIO))
WRAP(int, avcodec_send_packet, (AVCodecContext *c, const AVPacket *p), (c, p),
     AVERROR_INVALIDDATA)
WRAP(int, av_frame_ref, (AVFrame *d, const AVFrame *s), (d, s), AVERROR(ENOMEM))
WRAP(int, swr_alloc_set_opts2,
     (struct SwrContext **ps, const AVChannelLayout *ol, enum AVSampleFormat of,
      int orate, const AVChannelLayout *il, enum AVSampleFormat inf, int irate,
      int lo, void *lc),
     (ps, ol, of, orate, il, inf, irate, lo, lc), AVERROR(EINVAL))
WRAP(int, swr_init, (struct SwrContext *s), (s), AVERROR(EINVAL))
WRAP(int, swr_convert,
     (struct SwrContext *s, uint8_t *const *o, int oc, const uint8_t *const *i, int ic),
     (s, o, oc, i, ic), AVERROR(EINVAL))
WRAP(int, swr_get_out_samples, (struct SwrContext *s, int n), (s, n), AVERROR(EINVAL))

int __real_avcodec_receive_frame(AVCodecContext *c, AVFrame *f);
int __wrap_avcodec_receive_frame(AVCodecContext *c, AVFrame *f);
int __wrap_avcodec_receive_frame(AVCodecContext *c, AVFrame *f) {
    if (inject("avcodec_receive_frame")) return g_rc ? g_rc : AVERROR_INVALIDDATA;
    return __real_avcodec_receive_frame(c, f);
}

int __real_avformat_alloc_output_context2(AVFormatContext **ctx,
                                          const AVOutputFormat *of,
                                          const char *fmt, const char *file);
int __wrap_avformat_alloc_output_context2(AVFormatContext **ctx,
                                          const AVOutputFormat *of,
                                          const char *fmt, const char *file);
int __wrap_avformat_alloc_output_context2(AVFormatContext **ctx,
                                          const AVOutputFormat *of,
                                          const char *fmt, const char *file) {
    if (inject("avformat_alloc_output_context2")) {
        *ctx = NULL;
        return AVERROR(ENOMEM);
    }
    return __real_avformat_alloc_output_context2(ctx, of, fmt, file);
}

static FILE *g_sink;

static SrDiagnostics sink_diag(void) {
    if (!g_sink) g_sink = fopen("/dev/null", "w");
    SrDiagnostics diag;
    sr_diag_init(&diag, "faults", g_sink ? g_sink : stderr);
    return diag;
}

static void fault_scene(SrScene *scene) {
    sr_scene_init(scene);
    scene->project.width = 16;
    scene->project.height = 16;
    scene->project.fps_num = 24;
    scene->project.fps_den = 1;
    scene->output.codec = SR_CODEC_FFV1;
}

static void open_faults(sr_test_ctx *t) {
    static const struct {
        const char *fn;
        int skip;
        bool audio;
        SrStatus want;
    } rows[] = {
        {"avformat_alloc_output_context2", 0, false, SR_ERR_MEMORY},
        {"av_packet_alloc", 0, false, SR_ERR_MEMORY},
        {"avcodec_find_encoder_by_name", 0, false, SR_ERR_ENCODER},
        {"avcodec_alloc_context3", 0, false, SR_ERR_MEMORY},
        {"avcodec_open2", 0, false, SR_ERR_ENCODER},
        {"sws_getContext", 0, false, SR_ERR_ENCODER},
        {"av_frame_alloc", 0, false, SR_ERR_MEMORY},
        {"av_frame_get_buffer", 0, false, SR_ERR_MEMORY},
        {"avformat_new_stream", 0, false, SR_ERR_MEMORY},
        {"avcodec_parameters_from_context", 0, false, SR_ERR_ENCODER},
        {"avio_open", 0, false, SR_ERR_IO},
        {"avformat_write_header", 0, false, SR_ERR_ENCODER},
        /* The video half opens first, so skip 1 targets the audio call. */
        {"avcodec_find_encoder_by_name", 1, true, SR_ERR_ENCODER},
        {"avcodec_alloc_context3", 1, true, SR_ERR_MEMORY},
        {"avcodec_open2", 1, true, SR_ERR_ENCODER},
        {"swr_alloc_set_opts2", 0, true, SR_ERR_ENCODER},
        {"swr_init", 0, true, SR_ERR_ENCODER},
        {"av_frame_alloc", 1, true, SR_ERR_MEMORY},
        {"av_frame_get_buffer", 1, true, SR_ERR_MEMORY},
        {"avformat_new_stream", 1, true, SR_ERR_MEMORY},
        {"avcodec_parameters_from_context", 1, true, SR_ERR_ENCODER},
    };
    for (size_t i = 0; i < sizeof(rows) / sizeof(rows[0]); ++i) {
        SrScene scene;
        fault_scene(&scene);
        SrDiagnostics diag = sink_diag();
        SrEncoderAudio audio = {48000, rows[i].audio ? 2 : 0};
        SrEncoder *e = NULL;
        arm(rows[i].fn, rows[i].skip);
        SrStatus st = sr_encoder_open(&e, &scene, sr_test_tmp_path("fault.mkv"),
                                      1, &audio, &diag);
        bool fired = g_fault == NULL;
        disarm();
        if (st != rows[i].want)
            SR_FAIL(t, "%s (skip %d): status %d, expected %d", rows[i].fn,
                    rows[i].skip, (int)st, (int)rows[i].want);
        CHECK(t, fired);
        CHECK(t, e == NULL);
        CHECK(t, diag.errors > 0);
        sr_encoder_destroy(e);
        sr_scene_free(&scene);
    }
}

static void write_and_finish_faults(sr_test_ctx *t) {
    static const struct {
        const char *fn;
        int skip;
        int stage;           /* 0 write video, 1 write audio, 2 finish */
        int64_t samples;     /* audio written before the faulting stage */
        SrStatus want;
    } rows[] = {
        {"av_frame_make_writable", 0, 0, 0, SR_ERR_MEMORY},
        {"avcodec_send_frame", 0, 0, 0, SR_ERR_ENCODER},
        {"avcodec_receive_packet", 0, 0, 0, SR_ERR_ENCODER},
        {"av_interleaved_write_frame", 0, 0, 0, SR_ERR_ENCODER},
        {"av_frame_make_writable", 0, 1, 0, SR_ERR_MEMORY},
        {"swr_convert", 0, 1, 0, SR_ERR_ENCODER},
        {"avcodec_send_frame", 0, 1, 0, SR_ERR_ENCODER},
        {"avcodec_send_frame", 0, 2, 100, SR_ERR_ENCODER},    /* video flush */
        {"avcodec_send_frame", 1, 2, 100, SR_ERR_ENCODER},    /* audio tail */
        {"avcodec_send_frame", 1, 2, 0, SR_ERR_ENCODER},      /* audio flush */
        {"av_write_trailer", 0, 2, 0, SR_ERR_ENCODER},
    };
    uint8_t rgba[16 * 16 * 4];
    memset(rgba, 90, sizeof(rgba));
    static float pcm[4096 * 2];
    for (size_t i = 0; i < sizeof(rows) / sizeof(rows[0]); ++i) {
        SrScene scene;
        fault_scene(&scene);
        SrDiagnostics diag = sink_diag();
        SrEncoderAudio audio = {48000, 2};
        SrEncoder *e = NULL;
        if (sr_encoder_open(&e, &scene, sr_test_tmp_path("fault-io.mkv"), 1,
                            &audio, &diag) != SR_OK) {
            SR_FAIL(t, "open failed");
            sr_scene_free(&scene);
            continue;
        }
        SrStatus st;
        if (rows[i].stage == 0) {
            arm(rows[i].fn, rows[i].skip);
            st = sr_encoder_write_video(e, rgba, &diag);
        } else if (rows[i].stage == 1) {
            arm(rows[i].fn, rows[i].skip);
            st = sr_encoder_write_audio(e, pcm, 4096, &diag);
        } else {
            CHECK_INT(t, sr_encoder_write_video(e, rgba, &diag), SR_OK);
            CHECK_INT(t, sr_encoder_write_audio(e, pcm, (size_t)rows[i].samples,
                                                &diag), SR_OK);
            arm(rows[i].fn, rows[i].skip);
            st = sr_encoder_finish(e, &diag);
        }
        bool fired = g_fault == NULL;
        disarm();
        if (st != rows[i].want)
            SR_FAIL(t, "row %zu (%s): status %d, expected %d", i, rows[i].fn,
                    (int)st, (int)rows[i].want);
        CHECK(t, fired);
        sr_encoder_destroy(e);
        sr_scene_free(&scene);
    }
}

/* A 16x16, 12-frame FFV1 clip written once for the decoder cases. */
static const char *video_fixture(void) {
    static char path[1024];
    if (!path[0]) {
        snprintf(path, sizeof(path), "%s", sr_test_tmp_path("fault-src.mkv"));
        SrScene scene;
        fault_scene(&scene);
        SrDiagnostics diag = sink_diag();
        SrEncoder *e = NULL;
        uint8_t rgba[16 * 16 * 4];
        if (sr_encoder_open(&e, &scene, path, 1, NULL, &diag) == SR_OK) {
            for (int i = 0; i < 12; ++i) {
                memset(rgba, 20 * i, sizeof(rgba));
                sr_encoder_write_video(e, rgba, &diag);
            }
            sr_encoder_finish(e, &diag);
        }
        sr_encoder_destroy(e);
        sr_scene_free(&scene);
    }
    return path;
}

static SrProject fault_project(void) {
    SrScene scene;
    sr_scene_init(&scene);
    SrProject project = scene.project;
    sr_scene_free(&scene);
    return project;
}

static void video_open_faults(sr_test_ctx *t) {
    static const struct {
        const char *fn;
        int skip;
        SrStatus want;
    } rows[] = {
        {"avformat_open_input", 0, SR_ERR_ASSET},
        {"avformat_find_stream_info", 0, SR_ERR_ASSET},
        {"av_find_best_stream", 0, SR_ERR_ASSET},
        {"avcodec_find_decoder", 0, SR_ERR_ASSET},
        {"avcodec_alloc_context3", 0, SR_ERR_MEMORY},
        {"avcodec_parameters_to_context", 0, SR_ERR_ASSET},
        {"avcodec_open2", 0, SR_ERR_ASSET},
        {"av_packet_alloc", 0, SR_ERR_MEMORY},
        {"av_frame_alloc", 0, SR_ERR_MEMORY},
        {"av_frame_alloc", 1, SR_ERR_MEMORY},
        {"av_read_frame", 3, SR_ERR_ASSET},
        {"avformat_seek_file", 0, SR_ERR_ASSET},
    };
    const char *path = video_fixture();
    SrProject project = fault_project();
    for (size_t i = 0; i < sizeof(rows) / sizeof(rows[0]); ++i) {
        SrVideoSource *v = (SrVideoSource *)&project;
        char err[256] = "";
        arm(rows[i].fn, rows[i].skip);
        SrStatus st = sr_video_open(path, &project, SR_COLOR_SRGB, 24, 1, 0, &v,
                                    err, sizeof(err));
        bool fired = g_fault == NULL;
        disarm();
        if (st != rows[i].want)
            SR_FAIL(t, "%s (skip %d): %d, expected %d", rows[i].fn, rows[i].skip,
                    (int)st, (int)rows[i].want);
        CHECK(t, fired);
        CHECK(t, v == NULL);
        CHECK(t, err[0] != '\0');
    }
}

static void video_frame_faults(sr_test_ctx *t) {
    static const struct {
        const char *fn;
        int skip, rc;
        SrStatus want;
    } rows[] = {
        /* The decoder ends early: before any frame there is nothing to
         * show; after some, the latest decoded one is held. */
        {"avcodec_receive_frame", 0, AVERROR_EOF, SR_ERR_ASSET},
        {"avcodec_receive_frame", 4, AVERROR_EOF, SR_OK},
        {"avcodec_receive_frame", 0, 0, SR_ERR_ASSET},
        {"avformat_seek_file", 0, 0, SR_ERR_ASSET},
        {"av_read_frame", 0, 0, SR_ERR_ASSET},
        {"avcodec_send_packet", 0, 0, SR_ERR_ASSET},
        {"sws_getContext", 0, 0, SR_ERR_ASSET},
        {"av_frame_ref", 0, 0, SR_ERR_MEMORY},
    };
    const char *path = video_fixture();
    SrProject project = fault_project();
    for (size_t i = 0; i < sizeof(rows) / sizeof(rows[0]); ++i) {
        SrVideoSource *v = NULL;
        if (sr_video_open(path, &project, SR_COLOR_SRGB, 24, 1, 0, &v, NULL, 0) !=
            SR_OK) {
            SR_FAIL(t, "fixture open failed");
            return;
        }
        const SrImage *image = NULL;
        char err[256] = "";
        arm(rows[i].fn, rows[i].skip);
        g_rc = rows[i].rc;
        SrStatus st = sr_video_frame(v, 3, &image, err, sizeof(err));
        disarm();
        if (st != rows[i].want)
            SR_FAIL(t, "%s (skip %d): %d, expected %d", rows[i].fn, rows[i].skip,
                    (int)st, (int)rows[i].want);
        CHECK(t, (st == SR_OK) == (image != NULL));
        /* The source recovers: the next request succeeds. */
        CHECK_INT(t, sr_video_frame(v, 5, &image, NULL, 0), SR_OK);
        CHECK(t, image && image->width == 16);
        sr_video_close(v);
    }
}

/* One second of 48 kHz stereo 16-bit PCM in a WAV file. */
static const char *audio_fixture(void) {
    static char path[1024];
    if (!path[0]) {
        snprintf(path, sizeof(path), "%s", sr_test_tmp_path("fault.wav"));
        FILE *f = fopen(path, "wb");
        if (!f) return path;
        const uint32_t data = 48000 * 4, riff = 36 + data, fmt_len = 16,
                       rate = 48000, bps = 48000 * 4;
        const uint16_t pcm = 1, ch = 2, align = 4, bits = 16;
        uint8_t h[44];
        memcpy(h, "RIFF", 4);
        memcpy(h + 4, &riff, 4);
        memcpy(h + 8, "WAVEfmt ", 8);
        memcpy(h + 16, &fmt_len, 4);
        memcpy(h + 20, &pcm, 2);
        memcpy(h + 22, &ch, 2);
        memcpy(h + 24, &rate, 4);
        memcpy(h + 28, &bps, 4);
        memcpy(h + 32, &align, 2);
        memcpy(h + 34, &bits, 2);
        memcpy(h + 36, "data", 4);
        memcpy(h + 40, &data, 4);
        fwrite(h, 1, 44, f);
        for (uint32_t i = 0; i < data / 2; ++i) {
            int16_t v = (int16_t)(i % 100);
            fwrite(&v, 2, 1, f);
        }
        fclose(f);
    }
    return path;
}

static void audio_decoder_faults(sr_test_ctx *t) {
    static const struct {
        const char *fn;
        SrStatus want;
    } rows[] = {
        {"avformat_open_input", SR_ERR_ASSET}, {"avformat_find_stream_info", SR_ERR_ASSET},
        {"av_find_best_stream", SR_ERR_ASSET}, {"avcodec_find_decoder", SR_ERR_ASSET},
        {"avcodec_alloc_context3", SR_ERR_MEMORY},
        {"avcodec_parameters_to_context", SR_ERR_ASSET},
        {"avcodec_open2", SR_ERR_ASSET},       {"swr_alloc_set_opts2", SR_ERR_ASSET},
        {"swr_init", SR_ERR_ASSET},            {"av_packet_alloc", SR_ERR_MEMORY},
        {"av_frame_alloc", SR_ERR_MEMORY},     {"av_read_frame", SR_ERR_ASSET},
        {"avcodec_send_packet", SR_ERR_ASSET}, {"avcodec_receive_frame", SR_ERR_ASSET},
        {"swr_get_out_samples", SR_ERR_ASSET}, {"swr_convert", SR_ERR_ASSET},
    };
    const char *path = audio_fixture();
    for (size_t i = 0; i < sizeof(rows) / sizeof(rows[0]); ++i) {
        float *pcm = (float *)rows;
        uint64_t samples = 1;
        char err[256] = "";
        arm(rows[i].fn, 0);
        SrStatus st = sr_audio_decode_file(path, 48000, 2, 60.0, &pcm, &samples,
                                           err, sizeof(err));
        bool fired = g_fault == NULL;
        disarm();
        if (st != rows[i].want)
            SR_FAIL(t, "%s: %d, expected %d", rows[i].fn, (int)st, (int)rows[i].want);
        CHECK(t, fired);
        CHECK(t, pcm == NULL && samples == 0);
        CHECK(t, err[0] != '\0');
    }
    float *pcm = NULL;
    uint64_t samples = 0;
    CHECK_INT(t, sr_audio_decode_file(path, 48000, 2, 60.0, &pcm, &samples, NULL, 0),
              SR_OK);
    CHECK_INT(t, samples, 48000);
    free(pcm);
    /* The length guard: 1 s of audio against a 0.5 s limit. */
    char err[256] = "";
    CHECK_INT(t, sr_audio_decode_file(path, 48000, 2, 0.5, &pcm, &samples, err,
                                      sizeof(err)), SR_ERR_ASSET);
    CHECK_CONTAINS(t, err, "4-hour limit");
}

static void image_decoder_faults(sr_test_ctx *t) {
    static const char *const rows[] = {
        "avformat_open_input", "avformat_find_stream_info", "av_find_best_stream",
        "avcodec_find_decoder", "avcodec_open2", "av_packet_alloc",
        "av_read_frame", "avcodec_send_packet", "avcodec_receive_frame",
        "sws_getContext",
    };
    const char *path = sr_test_data_path("examples/assets/checker.ppm");
    for (size_t i = 0; i < sizeof(rows) / sizeof(rows[0]); ++i) {
        uint8_t *rgba = (uint8_t *)rows;
        char err[256] = "";
        arm(rows[i], 0);
        SrStatus st = sr_image_decode_rgba8(path, 8, 8, false, &rgba, NULL, NULL,
                                            err, sizeof(err));
        bool fired = g_fault == NULL;
        disarm();
        CHECK(t, st != SR_OK);
        CHECK(t, fired);
        CHECK(t, rgba == NULL);
        CHECK(t, err[0] != '\0');
    }
}

const sr_test_case sr_tests_encode_faults[] = {
    {"open_faults", open_faults},
    {"write_and_finish_faults", write_and_finish_faults},
    {"video_open_faults", video_open_faults},
    {"video_frame_faults", video_frame_faults},
    {"audio_decoder_faults", audio_decoder_faults},
    {"image_decoder_faults", image_decoder_faults},
    {NULL, NULL},
};
