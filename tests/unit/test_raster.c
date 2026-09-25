/* SPDX-License-Identifier: Apache-2.0 */
#include "fixture.h"

static const float clear[4] = {0, 0, 0, 0};

static void test_edge_half_coverage(sr_test_ctx *t)
{
    CHECK_NEAR(t, sr_shape_coverage(SR_MASK_RECT, 0, 0, 10.5, 20, 0,
                                    10.5, 5.5, 1.0), 0.5, 1e-6);
    SrScene scene;
    fx_scene(&scene, 16, 8);
    fx_rect(&scene, NULL, 0, 0, 10.5, 8, (SrColor){1, 1, 1, 1}, 1.0);
    SrFrame frame = {0};
    if (fx_render(t, &scene, 0.0, clear, &frame)) {
        CHECK_NEAR(t, fx_px(&frame, 9, 4)[3], 1.0, 1e-6);
        CHECK_NEAR(t, fx_px(&frame, 10, 4)[3], 0.5, 1e-6);
        CHECK_NEAR(t, fx_px(&frame, 11, 4)[3], 0.0, 1e-6);
    }
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

static void test_rotated_rect_fractional(sr_test_ctx *t)
{
    SrScene scene;
    fx_scene(&scene, 48, 48);
    SrNode *rect = fx_rect(&scene, NULL, 24, 24, 20, 20, (SrColor){1, 1, 1, 1}, 1.0);
    if (rect) {
        rect->transform.rotation.base = 30.0;
        rect->transform.anchor_x.base = 10.0;
        rect->transform.anchor_y.base = 10.0;
    }
    SrFrame frame = {0};
    if (fx_render(t, &scene, 0.0, clear, &frame)) {
        size_t partial = 0;
        double area = 0.0;
        for (size_t i = 0; i < (size_t)48 * 48; ++i) {
            float a = frame.px[i * 4 + 3];
            partial += a > 0.05f && a < 0.95f;
            area += a;
        }
        CHECK(t, partial > 40);
        CHECK_NEAR(t, area, 400.0, 400.0 * 0.005);
    }
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

static void test_ellipse_area(sr_test_ctx *t)
{
    SrScene scene;
    fx_scene(&scene, 128, 96);
    SrNode *ellipse = fx_rect(&scene, NULL, 20.3, 17.7, 80, 50,
                              (SrColor){1, 1, 1, 1}, 1.0);
    if (ellipse) ellipse->shape = SR_SHAPE_ELLIPSE;
    SrFrame frame = {0};
    if (fx_render(t, &scene, 0.0, clear, &frame)) {
        double area = 0.0;
        for (size_t i = 0; i < (size_t)128 * 96; ++i) area += frame.px[i * 4 + 3];
        double expected = SR_PI * 40.0 * 25.0;
        CHECK_NEAR(t, area, expected, expected * 0.005);
    }
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

/* The stroke is centred on the outline: a 4 px stroke on a 20 px square
 * covers 2 px on each side of the edge. */
static void test_stroke_centred(sr_test_ctx *t)
{
    SrScene scene;
    fx_scene(&scene, 40, 40);
    SrNode *rect = fx_rect(&scene, NULL, 10, 10, 20, 20, (SrColor){0, 0, 1, 1}, 1.0);
    if (rect) {
        rect->stroke.base = (SrColor){1, 0, 0, 1};
        rect->stroke_width = 4.0;
    }
    SrFrame frame = {0};
    if (fx_render(t, &scene, 0.0, clear, &frame)) {
        CHECK_NEAR(t, fx_px(&frame, 7, 20)[3], 0.0, 1e-6);   /* outside */
        CHECK_NEAR(t, fx_px(&frame, 8, 20)[0], 1.0, 1e-6);   /* outer half */
        CHECK_NEAR(t, fx_px(&frame, 11, 20)[0], 1.0, 1e-6);  /* inner half */
        CHECK_NEAR(t, fx_px(&frame, 12, 20)[2], 1.0, 1e-6);  /* fill */
        CHECK_NEAR(t, fx_px(&frame, 12, 20)[0], 0.0, 1e-6);
    }
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

const sr_test_case sr_tests_raster[] = {
    {"edge_half_coverage", test_edge_half_coverage},
    {"rotated_rect_fractional", test_rotated_rect_fractional},
    {"ellipse_area", test_ellipse_area},
    {"stroke_centred", test_stroke_centred},
    {NULL, NULL},
};
