/* SPDX-License-Identifier: Apache-2.0 */
/* Persistent video decoding: sequential and seeking access reach identical
 * converted frames; cache statistics; the SrAsset-level API; still images. */
#include <stdlib.h>

#include <libswscale/swscale.h>

#include "harness.h"
#include "media_synth.h"
#include "scene_render/assets.h"
#include "scene_render/audio.h"
#include "scene_render/video.h"
#include "scene_render/xml.h"
#include "../../src/video_internal.h"

enum { CLIP_FRAMES = 24, CLIP_W = 160, CLIP_H = 90 };
#define CLIP_BYTES ((size_t)CLIP_W * CLIP_H * 4 * sizeof(float))

static FILE *g_sink;

static SrDiagnostics sink_diag(void) {
    if (!g_sink) g_sink = fopen("/dev/null", "w");
    SrDiagnostics diag;
    sr_diag_init(&diag, "video", g_sink ? g_sink : stderr);
    return diag;
}

/* The project of tests/data-media.xml, which declares clip.mp4. */
static bool media_scene(SrScene *scene) {
    SrDiagnostics diag = sink_diag();
    return sr_scene_load_xml(sr_test_data_path("tests/data-media.xml"), scene,
                             &diag) == SR_OK;
}

static void sequential_matches_seeking(sr_test_ctx *t) {
    SrScene scene;
    if (!media_scene(&scene)) {
        SR_FAIL(t, "cannot load tests/data-media.xml");
        return;
    }
    const char *path = sr_test_data_path("examples/assets/clip.mp4");
    SrVideoSource *seq = NULL, *jump = NULL;
    char err[256] = "";
    CHECK_INT(t, sr_video_open(path, &scene.project, SR_COLOR_SRGB, 12, 1,
                               SR_VIDEO_CACHE_DEFAULT_BYTES, &seq, err,
                               sizeof(err)), SR_OK);
    /* A 0-byte budget still keeps the one-frame minimum. */
    CHECK_INT(t, sr_video_open(path, &scene.project, SR_COLOR_SRGB, 12, 1, 0,
                               &jump, err, sizeof(err)), SR_OK);
    if (!seq || !jump) {
        SR_FAIL(t, "open failed: %s", err);
        sr_video_close(seq);
        sr_video_close(jump);
        sr_scene_free(&scene);
        return;
    }
    const SrVideoInfo *info = sr_video_info(seq);
    CHECK_INT(t, info->width, CLIP_W);
    CHECK_INT(t, info->height, CLIP_H);
    CHECK_INT(t, info->frame_count, CLIP_FRAMES);
    float *frames = malloc(CLIP_FRAMES * CLIP_BYTES);
    for (int i = 0; i < CLIP_FRAMES; ++i) {
        const SrImage *image = NULL;
        CHECK_INT(t, sr_video_frame(seq, i, &image, NULL, 0), SR_OK);
        if (image) memcpy(frames + (size_t)i * CLIP_W * CLIP_H * 4, image->px, CLIP_BYTES);
    }
    const SrVideoStats *stats = sr_video_stats(seq);
    CHECK_INT(t, stats->requests, CLIP_FRAMES);
    CHECK_INT(t, stats->cache_hits, 0);
    CHECK_INT(t, stats->seeks, 1);          /* only the initial positioning */
    CHECK(t, stats->decoded >= CLIP_FRAMES);
    /* Frames differ from each other (the clip moves). */
    CHECK(t, memcmp(frames, frames + (size_t)12 * CLIP_W * CLIP_H * 4, CLIP_BYTES) != 0);
    /* Repeats are cache hits and decode nothing. */
    uint64_t decoded = stats->decoded;
    const SrImage *image = NULL;
    CHECK_INT(t, sr_video_frame(seq, 5, &image, NULL, 0), SR_OK);
    CHECK_INT(t, sr_video_frame(seq, 5, &image, NULL, 0), SR_OK);
    CHECK_INT(t, stats->cache_hits, 2);
    CHECK_INT(t, stats->decoded, decoded);
    /* Out-of-range indices clamp to the first / last frame. */
    CHECK_INT(t, sr_video_frame(seq, -3, &image, NULL, 0), SR_OK);
    CHECK(t, image && !memcmp(image->px, frames, CLIP_BYTES));
    CHECK_INT(t, sr_video_frame(seq, 1000, &image, NULL, 0), SR_OK);
    CHECK(t, image && !memcmp(image->px,
                              frames + (size_t)(CLIP_FRAMES - 1) * CLIP_W * CLIP_H * 4,
                              CLIP_BYTES));
    /* Backward and far jumps (seek + decode forward) give identical pixels. */
    static const int order[] = {23, 0, 12, 5, 22, 1, 13, 11, 10, 3, 17, 16, 2,
                                20, 7, 19, 4, 21, 6, 18, 8, 15, 9, 14};
    int mismatches = 0;
    for (size_t k = 0; k < sizeof(order) / sizeof(order[0]); ++k) {
        CHECK_INT(t, sr_video_frame(jump, order[k], &image, NULL, 0), SR_OK);
        if (!image || memcmp(image->px,
                             frames + (size_t)order[k] * CLIP_W * CLIP_H * 4,
                             CLIP_BYTES) != 0)
            ++mismatches;
    }
    CHECK_INT(t, mismatches, 0);
    CHECK(t, sr_video_stats(jump)->seeks > 1);
    CHECK_INT(t, sr_video_stats(jump)->cache_hits, 0);
    free(frames);
    CHECK_INT(t, sr_video_minimum_bytes(jump), CLIP_BYTES);
    CHECK_INT(t, sr_video_minimum_bytes(NULL), 0);
    sr_video_close(seq);
    sr_video_close(jump);
    sr_scene_free(&scene);
}

/* The compositor-facing API maps source time at the declared rate. */
static void asset_frames_through_scene(sr_test_ctx *t) {
    SrScene scene;
    if (!media_scene(&scene)) {
        SR_FAIL(t, "cannot load tests/data-media.xml");
        return;
    }
    SrDiagnostics diag = sink_diag();
    CHECK_INT(t, sr_assets_load(&scene, &diag), SR_OK);
    SrAsset *clip = sr_scene_find_asset(&scene, "clip");
    CHECK(t, clip && clip->video);
    if (!clip || !clip->video) {
        sr_scene_free(&scene);
        return;
    }
    SrVideoSource *direct = NULL;
    CHECK_INT(t, sr_video_open(sr_test_data_path("examples/assets/clip.mp4"),
                               &scene.project, SR_COLOR_SRGB, 12, 1, 0, &direct,
                               NULL, 0), SR_OK);
    const SrImage *expected = NULL;
    SrImage *got = sr_asset_get_frame(&scene, clip, 0.5, &diag);   /* index 6 */
    CHECK(t, got != NULL);
    float *copy = got ? malloc(CLIP_BYTES) : NULL;
    if (copy) memcpy(copy, got->px, CLIP_BYTES);
    CHECK_INT(t, sr_video_frame(direct, 6, &expected, NULL, 0), SR_OK);
    CHECK(t, copy && expected && !memcmp(copy, expected->px, CLIP_BYTES));
    /* Past the declared duration: the last declared frame. */
    got = sr_asset_get_frame(&scene, clip, 99.0, &diag);
    CHECK_INT(t, sr_video_frame(direct, 23, &expected, NULL, 0), SR_OK);
    CHECK(t, got && expected && !memcmp(got->px, expected->px, CLIP_BYTES));
    size_t sources = 0;
    uint64_t totals[4];
    sr_assets_video_stats(&scene, &sources, totals);
    CHECK_INT(t, sources, 1);
    CHECK_INT(t, totals[0], 2);
    /* One kept frame per open source, far inside the default budget. */
    CHECK_INT(t, sr_video_scene_minimum_bytes(&scene), CLIP_BYTES);
    free(copy);
    sr_video_close(direct);
    CHECK_INT(t, diag.errors, 0);
    sr_scene_free(&scene);
}

static void declared_dimensions_must_match(sr_test_ctx *t) {
    SrScene scene;
    if (!media_scene(&scene)) {
        SR_FAIL(t, "cannot load tests/data-media.xml");
        return;
    }
    SrAsset *clip = sr_scene_find_asset(&scene, "clip");
    if (clip) clip->width = 150;
    SrDiagnostics diag = sink_diag();
    CHECK_INT(t, sr_assets_load(&scene, &diag), SR_ERR_ASSET);
    CHECK(t, clip && !clip->video);
    /* A wrong declared rate only warns. */
    if (clip) {
        clip->width = CLIP_W;
        clip->fps_num = 24;
        clip->duration = 1.0;
    }
    diag = sink_diag();
    CHECK_INT(t, sr_assets_load(&scene, &diag), SR_OK);
    CHECK_INT(t, diag.errors, 0);
    CHECK(t, diag.warnings >= 1);
    /* At a declared 24 fps the 12 fps stream's frames land on even indices;
     * odd indices resolve to the latest earlier frame. */
    if (clip && clip->video) {
        const SrImage *a = NULL, *b = NULL;
        float *copy = malloc(CLIP_BYTES);
        CHECK_INT(t, sr_video_frame(clip->video, 4, &a, NULL, 0), SR_OK);
        if (a && copy) memcpy(copy, a->px, CLIP_BYTES);
        CHECK_INT(t, sr_video_frame(clip->video, 5, &b, NULL, 0), SR_OK);
        CHECK(t, b && copy && !memcmp(copy, b->px, CLIP_BYTES));
        free(copy);
    }
    sr_scene_free(&scene);
}

static void still_images(sr_test_ctx *t) {
    const char *path = sr_test_data_path("examples/assets/checker.ppm");
    uint8_t *rgba = NULL;
    uint32_t w = 0, h = 0;
    char err[256] = "";
    CHECK_INT(t, sr_image_decode_rgba8(path, 8, 8, false, &rgba, &w, &h, err,
                                       sizeof(err)), SR_OK);
    CHECK_INT(t, w, 8);
    CHECK_INT(t, h, 8);
    CHECK(t, rgba && rgba[3] == 255);
    free(rgba);
    CHECK_INT(t, sr_image_decode_rgba8(path, 9, 8, false, &rgba, NULL, NULL,
                                       err, sizeof(err)), SR_ERR_ASSET);
    CHECK_CONTAINS(t, err, "declared 9x8");
    CHECK_INT(t, sr_image_decode_rgba8(path, 16, 16, true, &rgba, NULL, NULL,
                                       err, sizeof(err)), SR_OK);
    free(rgba);
    CHECK_INT(t, sr_image_decode_rgba8(sr_test_data_path("no-such.png"), 8, 8,
                                       true, &rgba, NULL, NULL, err, sizeof(err)),
              SR_ERR_ASSET);
    CHECK(t, rgba == NULL);
}

static SrProject plain_project(void) {
    SrScene scene;
    sr_scene_init(&scene);
    SrProject project = scene.project;
    sr_scene_free(&scene);
    return project;
}

/* First pixel of frame `index` of `v` (blend space), or -1. */
static float frame_value(SrVideoSource *v, int64_t index) {
    const SrImage *image = NULL;
    if (sr_video_frame(v, index, &image, NULL, 0) != SR_OK || !image) return -1.0f;
    return image->px[0];
}

/* Video and audio of one Matroska file both start at pts 10 s: both are
 * measured from the container start, so video frame 0 and audio sample 0
 * are the first content, without leading silence. */
static void video_and_audio_share_origin(sr_test_ctx *t) {
    enum { FRAMES = 10, PACKETS = 100 };
    int64_t vpts[FRAMES], apts[PACKETS];
    uint8_t levels[FRAMES];
    int counts[PACKETS];
    int16_t values[PACKETS];
    for (int k = 0; k < FRAMES; ++k) {
        vpts[k] = 10000 + 100 * k;     /* 10 fps, ms */
        levels[k] = (uint8_t)(20 + 20 * k);
    }
    for (int k = 0; k < PACKETS; ++k) {
        apts[k] = 10000 + 10 * k;      /* 480 samples = 10 ms */
        counts[k] = 480;
        values[k] = 8000;
    }
    SynthMedia m = {.format = "matroska", .video_tb = {1, 1000},
                    .video_pts = vpts, .video_level = levels,
                    .video_frames = FRAMES, .width = 16, .height = 16,
                    .audio_tb = {1, 1000}, .audio_rate = 48000,
                    .audio_pts = apts, .audio_samples = counts,
                    .audio_value = values, .audio_packets = PACKETS};
    char path[1024];
    snprintf(path, sizeof(path), "%s", sr_test_tmp_path("origin10.mkv"));
    CHECK(t, synth_media(path, &m));
    SrProject project = plain_project();
    SrVideoSource *v = NULL;
    char err[256] = "";
    CHECK_INT(t, sr_video_open(path, &project, SR_COLOR_SRGB, 10, 1, 0, &v, err,
                               sizeof(err)), SR_OK);
    if (v) {
        CHECK_INT(t, sr_video_info(v)->frame_count, FRAMES);
        float first = frame_value(v, 0), second = frame_value(v, 1);
        CHECK(t, first >= 0.0f && second > first);
        CHECK(t, frame_value(v, FRAMES - 1) > second);
        sr_video_close(v);
    }
    float *pcm = NULL;
    uint64_t samples = 0;
    CHECK_INT(t, sr_audio_decode_file(path, 48000, 1, 60.0, &pcm, &samples, err,
                                      sizeof(err)), SR_OK);
    CHECK_INT(t, samples, 48000);
    if (pcm) CHECK_NEAR(t, pcm[0], 8000.0f / 32768.0f, 1e-4);
    free(pcm);
}

/* Frames at 0, 20, 33.4 and 40 ms at a declared 30 fps: index 1 (33.33 ms)
 * shows the 20 ms frame (the 33.4 ms one is in its future), sequentially
 * and after a seek; index 2 shows the 40 ms frame. A 10 kHz reference
 * source addresses each frame by its own tick. */
static void vfr_never_shows_future_frames(sr_test_ctx *t) {
    static const int64_t pts[] = {0, 200, 334, 400};    /* 1/10000 s */
    static const uint8_t levels[] = {10, 60, 110, 160};
    SynthMedia m = {.format = "nut", .video_tb = {1, 10000}, .video_pts = pts,
                    .video_level = levels, .video_frames = 4, .width = 16,
                    .height = 16};
    char path[1024];
    snprintf(path, sizeof(path), "%s", sr_test_tmp_path("vfr.nut"));
    CHECK(t, synth_media(path, &m));
    SrProject project = plain_project();
    SrVideoSource *seq = NULL, *jump = NULL, *ref = NULL;
    CHECK_INT(t, sr_video_open(path, &project, SR_COLOR_SRGB, 30, 1, 0, &seq, NULL, 0),
              SR_OK);
    CHECK_INT(t, sr_video_open(path, &project, SR_COLOR_SRGB, 30, 1, 0, &jump, NULL, 0),
              SR_OK);
    CHECK_INT(t, sr_video_open(path, &project, SR_COLOR_SRGB, 10000, 1, 0, &ref, NULL, 0),
              SR_OK);
    if (seq && jump && ref) {
        CHECK_INT(t, sr_video_info(seq)->frame_count, 3);
        float f0 = frame_value(ref, 0), f20 = frame_value(ref, 200),
              f33 = frame_value(ref, 334), f40 = frame_value(ref, 400);
        CHECK(t, f0 < f20 && f20 < f33 && f33 < f40);
        CHECK(t, frame_value(ref, 333) == f20);   /* never early */
        CHECK(t, frame_value(seq, 0) == f0);
        CHECK(t, frame_value(seq, 1) == f20);
        CHECK(t, frame_value(seq, 2) == f40);
        CHECK(t, frame_value(jump, 1) == f20);    /* first request: a seek */
        CHECK(t, sr_video_stats(jump)->seeks >= 1);
        CHECK(t, frame_value(jump, 0) == f0);     /* backward */
        CHECK(t, frame_value(jump, 2) == f40);
    }
    sr_video_close(seq);
    sr_video_close(jump);
    sr_video_close(ref);
}

/* Tagged matrices map to their swscale coefficients. */
static void input_matrices(sr_test_ctx *t) {
    bool approx = true;
    CHECK_INT(t, sr_video_sws_matrix(AVCOL_SPC_BT709, 480, &approx), SWS_CS_ITU709);
    CHECK(t, !approx);
    CHECK_INT(t, sr_video_sws_matrix(AVCOL_SPC_SMPTE240M, 1080, &approx), SWS_CS_SMPTE240M);
    CHECK_INT(t, sr_video_sws_matrix(AVCOL_SPC_FCC, 480, &approx), SWS_CS_FCC);
    CHECK_INT(t, sr_video_sws_matrix(AVCOL_SPC_BT470BG, 1080, &approx), SWS_CS_ITU601);
    CHECK_INT(t, sr_video_sws_matrix(AVCOL_SPC_SMPTE170M, 1080, &approx), SWS_CS_ITU601);
    CHECK_INT(t, sr_video_sws_matrix(AVCOL_SPC_UNSPECIFIED, 720, &approx), SWS_CS_ITU709);
    CHECK_INT(t, sr_video_sws_matrix(AVCOL_SPC_UNSPECIFIED, 576, &approx), SWS_CS_ITU601);
    CHECK_INT(t, sr_video_sws_matrix(AVCOL_SPC_BT2020_NCL, 2160, &approx), SWS_CS_BT2020);
    CHECK(t, !approx);
    CHECK_INT(t, sr_video_sws_matrix(AVCOL_SPC_BT2020_CL, 2160, &approx), SWS_CS_BT2020);
    CHECK(t, approx);
    CHECK_INT(t, sr_video_sws_matrix(AVCOL_SPC_BT2020_CL, 2160, NULL), SWS_CS_BT2020);
}

const sr_test_case sr_tests_video[] = {
    {"sequential_matches_seeking", sequential_matches_seeking},
    {"asset_frames_through_scene", asset_frames_through_scene},
    {"declared_dimensions_must_match", declared_dimensions_must_match},
    {"still_images", still_images},
    {"video_and_audio_share_origin", video_and_audio_share_origin},
    {"vfr_never_shows_future_frames", vfr_never_shows_future_frames},
    {"input_matrices", input_matrices},
    {NULL, NULL},
};
