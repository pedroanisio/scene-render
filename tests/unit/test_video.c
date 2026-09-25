/* SPDX-License-Identifier: Apache-2.0 */
/* Persistent video decoding: sequential and seeking access reach identical
 * converted frames; cache statistics; the SrAsset-level API; still images. */
#include <stdlib.h>

#include "harness.h"
#include "scene_render/assets.h"
#include "scene_render/video.h"
#include "scene_render/xml.h"

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
    /* A 0-byte budget still keeps the two-frame minimum. */
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

const sr_test_case sr_tests_video[] = {
    {"sequential_matches_seeking", sequential_matches_seeking},
    {"asset_frames_through_scene", asset_frames_through_scene},
    {"declared_dimensions_must_match", declared_dimensions_must_match},
    {"still_images", still_images},
    {NULL, NULL},
};
