/* SPDX-License-Identifier: Apache-2.0 */
/* In-process encoder: round trips through libavformat, color tags,
 * bit-exact output, spherical side data and the >8-bit input path. */
#include <stdlib.h>
#include <unistd.h>

#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/pixdesc.h>
#include <libavutil/spherical.h>
#include <libswscale/swscale.h>

#include "harness.h"
#include "scene_render/encoder.h"

typedef struct {
    int frames, width, height;
    bool monotonic;
    AVRational rate;
    enum AVPixelFormat format;
    enum AVColorRange range;
    enum AVColorSpace space;
    enum AVColorTransferCharacteristic transfer;
    enum AVColorPrimaries primaries;
    bool spherical;
    uint8_t center[3];      /* first frame's center pixel, BT.709 decode */
    int audio_streams;
} Probe;

static bool has_spherical(const AVStream *st) {
    const AVPacketSideData *sd = av_packet_side_data_get(
        st->codecpar->coded_side_data, st->codecpar->nb_coded_side_data,
        AV_PKT_DATA_SPHERICAL);
    return sd && ((const AVSphericalMapping *)sd->data)->projection ==
                     AV_SPHERICAL_EQUIRECTANGULAR;
}

/* Decodes every frame of `path` and records what the container says. */
static bool probe_file(const char *path, Probe *p) {
    memset(p, 0, sizeof(*p));
    p->monotonic = true;
    AVFormatContext *fmt = NULL;
    if (avformat_open_input(&fmt, path, NULL, NULL) < 0) return false;
    avformat_find_stream_info(fmt, NULL);
    int si = av_find_best_stream(fmt, AVMEDIA_TYPE_VIDEO, -1, -1, NULL, 0);
    if (si < 0) {
        avformat_close_input(&fmt);
        return false;
    }
    for (unsigned i = 0; i < fmt->nb_streams; ++i)
        p->audio_streams += fmt->streams[i]->codecpar->codec_type == AVMEDIA_TYPE_AUDIO;
    const AVCodecParameters *par = fmt->streams[si]->codecpar;
    p->rate = fmt->streams[si]->avg_frame_rate;
    p->format = (enum AVPixelFormat)par->format;
    p->range = par->color_range;
    p->space = par->color_space;
    p->transfer = par->color_trc;
    p->primaries = par->color_primaries;
    p->spherical = has_spherical(fmt->streams[si]);
    const AVCodec *dec = avcodec_find_decoder(par->codec_id);
    AVCodecContext *c = avcodec_alloc_context3(dec);
    avcodec_parameters_to_context(c, par);
    avcodec_open2(c, dec, NULL);
    AVPacket *pkt = av_packet_alloc();
    AVFrame *fr = av_frame_alloc();
    int64_t last = INT64_MIN;
    bool draining = false;
    for (;;) {
        if (!draining) {
            if (av_read_frame(fmt, pkt) < 0) {
                draining = true;
                avcodec_send_packet(c, NULL);
            } else {
                if (pkt->stream_index == si) avcodec_send_packet(c, pkt);
                av_packet_unref(pkt);
            }
        }
        int rc;
        while ((rc = avcodec_receive_frame(c, fr)) == 0) {
            if (p->frames == 0) {
                uint8_t rgb[3];
                struct SwsContext *s = sws_getContext(
                    fr->width, fr->height, fr->format, 1, 1, AV_PIX_FMT_RGB24,
                    SWS_POINT, NULL, NULL, NULL);
                const int *m = sws_getCoefficients(SWS_CS_ITU709);
                sws_setColorspaceDetails(s, m, fr->color_range == AVCOL_RANGE_JPEG,
                                         m, 1, 0, 1 << 16, 1 << 16);
                uint8_t *dst[1] = {rgb};
                int ds[1] = {3};
                sws_scale(s, (const uint8_t *const *)fr->data, fr->linesize, 0,
                          fr->height, dst, ds);
                sws_freeContext(s);
                memcpy(p->center, rgb, 3);
            }
            p->width = fr->width;
            p->height = fr->height;
            if (fr->pts <= last) p->monotonic = false;
            last = fr->pts;
            p->frames++;
        }
        if (draining && rc == AVERROR_EOF) break;
    }
    av_frame_free(&fr);
    av_packet_free(&pkt);
    avcodec_free_context(&c);
    avformat_close_input(&fmt);
    return true;
}

static void set_str(char **field, const char *value) {
    free(*field);
    *field = sr_strdup(value);
}

static void enc_scene(SrScene *scene, SrCodec codec, const char *pix,
                      uint32_t width, uint32_t height) {
    sr_scene_init(scene);
    scene->project.width = width;
    scene->project.height = height;
    scene->project.fps_num = 24;
    scene->project.fps_den = 1;
    scene->output.codec = codec;
    scene->output.crf = 10;
    set_str(&scene->output.pixel_format, pix);
    set_str(&scene->output.preset, "ultrafast");
}

/* Diagnostics captured in a temporary file, readable with diag_text. */
static FILE *g_diag_file;
static char g_diag_text[4096];

static SrDiagnostics quiet_diag(void) {
    if (!g_diag_file) g_diag_file = tmpfile();
    else rewind(g_diag_file);
    if (g_diag_file) {
        int fd = fileno(g_diag_file);
        if (ftruncate(fd, 0) != 0) {}
    }
    SrDiagnostics diag;
    sr_diag_init(&diag, "test", g_diag_file ? g_diag_file : stderr);
    return diag;
}

static const char *diag_text(void) {
    g_diag_text[0] = '\0';
    if (!g_diag_file) return g_diag_text;
    fflush(g_diag_file);
    rewind(g_diag_file);
    size_t n = fread(g_diag_text, 1, sizeof(g_diag_text) - 1, g_diag_file);
    g_diag_text[n] = '\0';
    return g_diag_text;
}

/* Encodes `frames` frames of flat (200,100,50) and returns the status. */
static SrStatus encode_flat(const SrScene *scene, const char *path, int frames,
                            const SrEncoderAudio *audio, SrDiagnostics *diag) {
    SrEncoder *e = NULL;
    SrStatus st = sr_encoder_open(&e, scene, path, 2, audio, diag);
    if (st != SR_OK) return st;
    unsigned bits = sr_encoder_bits(e);
    size_t pixels = (size_t)scene->project.width * scene->project.height;
    void *buffer = malloc(pixels * 4 * (bits / 8));
    for (size_t i = 0; i < pixels; ++i) {
        const unsigned v[4] = {200, 100, 50, 255};
        for (int c = 0; c < 4; ++c) {
            if (bits == 16) ((uint16_t *)buffer)[i * 4 + c] = (uint16_t)(v[c] * 257);
            else ((uint8_t *)buffer)[i * 4 + c] = (uint8_t)v[c];
        }
    }
    float pcm[2000 * 2] = {0};
    for (int f = 0; f < frames && st == SR_OK; ++f) {
        st = sr_encoder_write_video(e, buffer, diag);
        if (st == SR_OK && audio && audio->channels)
            st = sr_encoder_write_audio(e, pcm, 2000, diag);
    }
    if (st == SR_OK) st = sr_encoder_finish(e, diag);
    free(buffer);
    sr_encoder_destroy(e);
    return st;
}

static void round_trip(sr_test_ctx *t, SrCodec codec, const char *pix,
                       const char *name, int tolerance) {
    SrScene scene;
    enc_scene(&scene, codec, pix, 64, 48);
    SrDiagnostics diag = quiet_diag();
    char path[1024];
    snprintf(path, sizeof(path), "%s", sr_test_tmp_path(name));
    CHECK_INT(t, encode_flat(&scene, path, 12, NULL, &diag), SR_OK);
    Probe p;
    CHECK(t, probe_file(path, &p));
    CHECK_INT(t, p.frames, 12);
    CHECK_INT(t, p.width, 64);
    CHECK_INT(t, p.height, 48);
    CHECK(t, p.monotonic);
    CHECK_INT(t, p.rate.num, 24 * p.rate.den);
    CHECK_STR(t, av_get_pix_fmt_name(p.format), pix);
    CHECK_INT(t, p.range, AVCOL_RANGE_MPEG);
    CHECK_INT(t, p.space, AVCOL_SPC_BT709);
    CHECK_INT(t, p.primaries, AVCOL_PRI_BT709);
    CHECK_INT(t, p.transfer, AVCOL_TRC_IEC61966_2_1);
    CHECK_NEAR(t, p.center[0], 200, tolerance);
    CHECK_NEAR(t, p.center[1], 100, tolerance);
    CHECK_NEAR(t, p.center[2], 50, tolerance);
    CHECK(t, !p.spherical);
    sr_scene_free(&scene);
}

static void h264_mp4_round_trip(sr_test_ctx *t) {
    round_trip(t, SR_CODEC_H264, "yuv420p", "enc-h264.mp4", 3);
}

static void h265_mp4_round_trip(sr_test_ctx *t) {
    round_trip(t, SR_CODEC_H265, "yuv420p", "enc-h265.mp4", 3);
}

static void ffv1_mkv_round_trip(sr_test_ctx *t) {
    round_trip(t, SR_CODEC_FFV1, "yuv444p", "enc-ffv1.mkv", 2);
}

/* 10-bit output takes 16-bit RGBA input; the round trip stays accurate. */
static void yuv420p10le_path(sr_test_ctx *t) {
    CHECK_INT(t, sr_encoder_input_bits("yuv420p10le"), 16);
    CHECK_INT(t, sr_encoder_input_bits("yuv420p"), 8);
    CHECK_INT(t, sr_encoder_input_bits("no-such-format"), 8);
    round_trip(t, SR_CODEC_H265, "yuv420p10le", "enc-h265-10.mp4", 3);
    round_trip(t, SR_CODEC_FFV1, "yuv420p10le", "enc-ffv1-10.mkv", 2);
}

static void color_tags_follow_output(sr_test_ctx *t) {
    static const struct {
        SrColorSpace space;
        bool full;
        enum AVColorPrimaries primaries;
        enum AVColorTransferCharacteristic transfer;
        enum AVColorSpace matrix;
    } rows[] = {
        {SR_COLOR_REC709, false, AVCOL_PRI_BT709, AVCOL_TRC_BT709, AVCOL_SPC_BT709},
        {SR_COLOR_DISPLAY_P3, true, AVCOL_PRI_SMPTE432, AVCOL_TRC_IEC61966_2_1,
         AVCOL_SPC_BT709},
        {SR_COLOR_REC2020, false, AVCOL_PRI_BT2020, AVCOL_TRC_BT2020_10,
         AVCOL_SPC_BT2020_NCL},
    };
    for (size_t i = 0; i < sizeof(rows) / sizeof(rows[0]); ++i) {
        SrScene scene;
        enc_scene(&scene, SR_CODEC_H264, "yuv420p", 32, 16);
        scene.output.color_space = rows[i].space;
        scene.output.full_range = rows[i].full;
        SrDiagnostics diag = quiet_diag();
        char name[64], path[1024];
        snprintf(name, sizeof(name), "enc-tags-%zu.mp4", i);
        snprintf(path, sizeof(path), "%s", sr_test_tmp_path(name));
        CHECK_INT(t, encode_flat(&scene, path, 3, NULL, &diag), SR_OK);
        Probe p;
        CHECK(t, probe_file(path, &p));
        CHECK_INT(t, p.primaries, rows[i].primaries);
        CHECK_INT(t, p.transfer, rows[i].transfer);
        CHECK_INT(t, p.space, rows[i].matrix);
        CHECK_INT(t, p.range, rows[i].full ? AVCOL_RANGE_JPEG : AVCOL_RANGE_MPEG);
        /* The matrix actually used matches the tag: (200,100,50) decodes
         * back with the BT.709 probe only when the tag says 709. */
        if (rows[i].matrix == AVCOL_SPC_BT709) {
            CHECK_NEAR(t, p.center[0], 200, 3);
            CHECK_NEAR(t, p.center[2], 50, 3);
        }
        sr_scene_free(&scene);
    }
}

static bool read_all(const char *path, uint8_t **buf, size_t *len) {
    FILE *f = fopen(path, "rb");
    if (!f) return false;
    fseek(f, 0, SEEK_END);
    *len = (size_t)ftell(f);
    rewind(f);
    *buf = malloc(*len ? *len : 1);
    bool ok = *buf && fread(*buf, 1, *len, f) == *len;
    fclose(f);
    return ok;
}

/* Identical frames give byte-identical files (no dates, versions or
 * scheduling-dependent choices in stream or container). */
static void bitexact_output(sr_test_ctx *t) {
    static const struct { SrCodec codec; const char *ext; bool audio; } rows[] = {
        {SR_CODEC_H264, "mp4", true},
        {SR_CODEC_H265, "mp4", false},
        {SR_CODEC_FFV1, "mkv", true},
    };
    for (size_t r = 0; r < sizeof(rows) / sizeof(rows[0]); ++r) {
        uint8_t *data[2] = {NULL, NULL};
        size_t len[2] = {0, 0};
        for (int run = 0; run < 2; ++run) {
            SrScene scene;
            enc_scene(&scene, rows[r].codec, "yuv420p", 32, 16);
            SrDiagnostics diag = quiet_diag();
            char name[64], path[1024];
            snprintf(name, sizeof(name), "enc-bitexact-%zu-%d.%s", r, run, rows[r].ext);
            snprintf(path, sizeof(path), "%s", sr_test_tmp_path(name));
            SrEncoderAudio audio = {48000, rows[r].audio ? 2 : 0};
            CHECK_INT(t, encode_flat(&scene, path, 10, &audio, &diag), SR_OK);
            CHECK(t, read_all(path, &data[run], &len[run]));
            sr_scene_free(&scene);
        }
        if (len[0] != len[1] || !data[0] || !data[1] ||
            memcmp(data[0], data[1], len[0]) != 0)
            SR_FAIL(t, "%s run differs: %zu vs %zu bytes", rows[r].ext, len[0], len[1]);
        free(data[0]);
        free(data[1]);
    }
}

static void spherical_side_data(sr_test_ctx *t) {
    const char *names[] = {"enc-360.mp4", "enc-360.mkv"};
    for (int i = 0; i < 2; ++i) {
        SrScene scene;
        enc_scene(&scene, i ? SR_CODEC_FFV1 : SR_CODEC_H264, "yuv420p", 64, 32);
        scene.project.mode = SR_MODE_EQUIRECTANGULAR;
        scene.output.spherical_metadata = true;
        SrDiagnostics diag = quiet_diag();
        char path[1024];
        snprintf(path, sizeof(path), "%s", sr_test_tmp_path(names[i]));
        CHECK_INT(t, encode_flat(&scene, path, 2, NULL, &diag), SR_OK);
        Probe p;
        CHECK(t, probe_file(path, &p));
        CHECK(t, p.spherical);
        /* Off switch: no side data. */
        scene.output.spherical_metadata = false;
        CHECK_INT(t, encode_flat(&scene, path, 2, NULL, &diag), SR_OK);
        CHECK(t, probe_file(path, &p));
        CHECK(t, !p.spherical);
        sr_scene_free(&scene);
    }
}

static void open_rejects_bad_configuration(sr_test_ctx *t) {
    SrScene scene;
    enc_scene(&scene, SR_CODEC_FFV1, "yuv420p", 32, 16);
    SrEncoder *e = (SrEncoder *)&scene;   /* must be reset to NULL */
    SrDiagnostics diag = quiet_diag();
    CHECK_INT(t, sr_encoder_open(&e, &scene, sr_test_tmp_path("enc-bad.mp4"), 1,
                                 NULL, &diag), SR_ERR_ARGUMENT);
    CHECK(t, e == NULL);
    CHECK_CONTAINS(t, diag_text(), "ffv1 requires a .mkv");
    diag = quiet_diag();
    CHECK_INT(t, sr_encoder_open(&e, &scene, sr_test_tmp_path("enc-bad.avi"), 1,
                                 NULL, &diag), SR_ERR_ARGUMENT);
    CHECK_CONTAINS(t, diag_text(), ".mp4, .mov or .mkv");
    scene.output.codec = SR_CODEC_H264;
    set_str(&scene.output.pixel_format, "bogus");
    diag = quiet_diag();
    CHECK_INT(t, sr_encoder_open(&e, &scene, sr_test_tmp_path("enc-bad.mp4"), 1,
                                 NULL, &diag), SR_ERR_ENCODER);
    CHECK_CONTAINS(t, diag_text(), "unknown pixel format 'bogus'");
    /* A real format the encoder cannot take lists the supported ones. */
    set_str(&scene.output.pixel_format, "rgb565le");
    diag = quiet_diag();
    CHECK_INT(t, sr_encoder_open(&e, &scene, sr_test_tmp_path("enc-bad.mp4"), 1,
                                 NULL, &diag), SR_ERR_ENCODER);
    CHECK_CONTAINS(t, diag_text(), "supported: ");
    CHECK_CONTAINS(t, diag_text(), "yuv420p");
    set_str(&scene.output.pixel_format, "yuv420p");
    set_str(&scene.output.audio_codec, "no-such-audio-codec");
    SrEncoderAudio audio = {48000, 2};
    diag = quiet_diag();
    CHECK_INT(t, sr_encoder_open(&e, &scene, sr_test_tmp_path("enc-bad.mp4"), 1,
                                 &audio, &diag), SR_ERR_ENCODER);
    CHECK_CONTAINS(t, diag_text(), "no-such-audio-codec");
    diag = quiet_diag();
    CHECK_INT(t, sr_encoder_open(&e, &scene,
                                 sr_test_tmp_path("no-such-dir/x.mp4"), 1, NULL,
                                 &diag), SR_ERR_IO);
    CHECK_CONTAINS(t, diag_text(), "cannot open output file");
    CHECK(t, e == NULL);
    CHECK(t, sr_encoder_available("libx264"));
    CHECK(t, !sr_encoder_available("no-such-encoder"));
    CHECK_STR(t, sr_encoder_name(NULL), "none");
    CHECK_INT(t, sr_encoder_write_video(NULL, NULL, &diag), SR_ERR_ARGUMENT);
    CHECK_INT(t, sr_encoder_finish(NULL, &diag), SR_ERR_ARGUMENT);
    sr_encoder_destroy(NULL);
    sr_scene_free(&scene);
}

static void finish_once_and_destroy_unfinished(sr_test_ctx *t) {
    SrScene scene;
    enc_scene(&scene, SR_CODEC_FFV1, "yuv420p", 16, 16);
    SrDiagnostics diag = quiet_diag();
    SrEncoder *e = NULL;
    CHECK_INT(t, sr_encoder_open(&e, &scene, sr_test_tmp_path("enc-once.mkv"), 1,
                                 NULL, &diag), SR_OK);
    CHECK_STR(t, sr_encoder_name(e), "ffv1");
    CHECK_INT(t, sr_encoder_bits(e), 8);
    uint8_t rgba[16 * 16 * 4] = {0};
    CHECK_INT(t, sr_encoder_write_video(e, rgba, &diag), SR_OK);
    /* No audio stream was opened. */
    CHECK_INT(t, sr_encoder_write_audio(e, NULL, 0, &diag), SR_ERR_ARGUMENT);
    CHECK_INT(t, sr_encoder_finish(e, &diag), SR_OK);
    CHECK_INT(t, sr_encoder_finish(e, &diag), SR_ERR_ARGUMENT);
    CHECK_INT(t, sr_encoder_write_video(e, rgba, &diag), SR_ERR_ARGUMENT);
    sr_encoder_destroy(e);
    e = NULL;
    CHECK_INT(t, sr_encoder_open(&e, &scene, sr_test_tmp_path("enc-once.mkv"), 1,
                                 NULL, &diag), SR_OK);
    sr_encoder_destroy(e);   /* without finish: no leak, no crash */
    sr_scene_free(&scene);
}

const sr_test_case sr_tests_encode[] = {
    {"h264_mp4_round_trip", h264_mp4_round_trip},
    {"h265_mp4_round_trip", h265_mp4_round_trip},
    {"ffv1_mkv_round_trip", ffv1_mkv_round_trip},
    {"yuv420p10le_path", yuv420p10le_path},
    {"color_tags_follow_output", color_tags_follow_output},
    {"bitexact_output", bitexact_output},
    {"spherical_side_data", spherical_side_data},
    {"open_rejects_bad_configuration", open_rejects_bad_configuration},
    {"finish_once_and_destroy_unfinished", finish_once_and_destroy_unfinished},
    {NULL, NULL},
};
