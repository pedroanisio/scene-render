/* SPDX-License-Identifier: Apache-2.0 */
#include "scene_render/camera.h"
#include "scene_render/compositor.h"

#include "harness.h"

/* Extracts a 1x1 viewport (a single ray along the camera's forward axis)
 * from `panorama` with the given orientation in degrees. */
static bool extract_center(sr_test_ctx *t, const SrFrame *panorama,
                           double yaw, double pitch, uint8_t out[4])
{
    SrCamera camera = {0};
    camera.id = "cam";
    camera.active = true;
    camera.fov.base = 90.0;
    camera.yaw.base = yaw;
    camera.pitch.base = pitch;
    SrScene scene = {0};
    scene.cameras = &camera;
    scene.camera_count = 1;
    SrFrame viewport = {0};
    if (sr_frame_init(&viewport, 1, 1) != SR_OK) {
        SR_FAIL(t, "viewport allocation failed");
        return false;
    }
    FILE *sink = tmpfile();
    SrDiagnostics diag;
    sr_diag_init(&diag, "unit-camera", sink ? sink : stderr);
    SrStatus status = sr_camera_extract_viewport(&scene, 0.0, panorama,
                                                 &viewport, 1, &diag);
    CHECK(t, status == SR_OK);
    memcpy(out, viewport.rgba, 4);
    sr_frame_free(&viewport);
    if (sink) fclose(sink);
    return status == SR_OK;
}

static void fill(SrFrame *frame, size_t pixel, uint8_t r, uint8_t g, uint8_t b)
{
    uint8_t *p = &frame->rgba[pixel * 4];
    p[0] = r; p[1] = g; p[2] = b; p[3] = 255;
}

/* Pixel centers of the equirectangular source sit at integer + 0.5 in the
 * continuous panorama coordinate. A ray through a pixel center must return
 * that pixel unblended. */
static void test_pixel_center_is_exact(sr_test_ctx *t)
{
    SrFrame panorama = {0};
    CHECK(t, sr_frame_init(&panorama, 2, 1) == SR_OK);
    if (!panorama.rgba) return;
    fill(&panorama, 0, 200, 10, 30);
    fill(&panorama, 1, 20, 100, 250);
    uint8_t out[4];
    /* yaw -90 looks at longitude -pi/2: the center of pixel 0. */
    if (extract_center(t, &panorama, -90.0, 0.0, out)) {
        CHECK_INT(t, out[0], 200);
        CHECK_INT(t, out[1], 10);
        CHECK_INT(t, out[2], 30);
        CHECK_INT(t, out[3], 255);
    }
    /* yaw +90 looks at longitude +pi/2: the center of pixel 1. */
    if (extract_center(t, &panorama, 90.0, 0.0, out)) {
        CHECK_INT(t, out[0], 20);
        CHECK_INT(t, out[1], 100);
        CHECK_INT(t, out[2], 250);
    }
    /* yaw 0 lands exactly on the boundary between the two centers. */
    if (extract_center(t, &panorama, 0.0, 0.0, out)) {
        CHECK_INT(t, out[0], 110);
        CHECK_INT(t, out[1], 55);
        CHECK_INT(t, out[2], 140);
    }
    /* yaw 180 lands on the seam; x wraps so it blends the same pair. */
    if (extract_center(t, &panorama, 180.0, 0.0, out)) {
        CHECK_INT(t, out[0], 110);
        CHECK_INT(t, out[1], 55);
        CHECK_INT(t, out[2], 140);
    }
    sr_frame_free(&panorama);
}

/* Documents the sign convention: positive pitch looks down (toward the
 * bottom rows of the panorama); y clamps instead of wrapping. */
static void test_positive_pitch_looks_down(sr_test_ctx *t)
{
    SrFrame panorama = {0};
    CHECK(t, sr_frame_init(&panorama, 1, 2) == SR_OK);
    if (!panorama.rgba) return;
    fill(&panorama, 0, 255, 0, 0);   /* top row: above the horizon */
    fill(&panorama, 1, 0, 0, 255);   /* bottom row: below the horizon */
    uint8_t out[4];
    if (extract_center(t, &panorama, 0.0, 45.0, out)) {
        CHECK_INT(t, out[0], 0);
        CHECK_INT(t, out[2], 255);
    }
    if (extract_center(t, &panorama, 0.0, -45.0, out)) {
        CHECK_INT(t, out[0], 255);
        CHECK_INT(t, out[2], 0);
    }
    /* Straight down is past the last row center and must clamp. */
    if (extract_center(t, &panorama, 0.0, 90.0, out)) {
        CHECK_INT(t, out[0], 0);
        CHECK_INT(t, out[2], 255);
    }
    sr_frame_free(&panorama);
}

const sr_test_case sr_tests_camera[] = {
    {"pixel_center_is_exact", test_pixel_center_is_exact},
    {"positive_pitch_looks_down", test_positive_pitch_looks_down},
    {NULL, NULL},
};
