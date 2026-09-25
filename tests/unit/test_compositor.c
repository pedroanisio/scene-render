/* SPDX-License-Identifier: Apache-2.0 */
#include "scene_render/compositor.h"

#include "harness.h"

#define NEAR_EPS 1e-6

static void test_blend_normal(sr_test_ctx *t)
{
    SrColor black = {0, 0, 0, 1};
    SrColor white = {1, 1, 1, 1};
    SrColor result = sr_blend_pixel(black, white, 0.5, SR_BLEND_NORMAL, false);
    CHECK_NEAR(t, result.r, 0.5, NEAR_EPS);
    CHECK_NEAR(t, result.g, 0.5, NEAR_EPS);
    CHECK_NEAR(t, result.b, 0.5, NEAR_EPS);
    CHECK_NEAR(t, result.a, 1, NEAR_EPS);
}

static void test_blend_multiply(sr_test_ctx *t)
{
    SrColor result = sr_blend_pixel((SrColor){0.2, 0.4, 0.8, 1},
                                    (SrColor){0.5, 0.5, 0.5, 1}, 1,
                                    SR_BLEND_MULTIPLY, false);
    CHECK_NEAR(t, result.r, 0.1, NEAR_EPS);
    CHECK_NEAR(t, result.g, 0.2, NEAR_EPS);
    CHECK_NEAR(t, result.b, 0.4, NEAR_EPS);
}

static void test_blend_add(sr_test_ctx *t)
{
    SrColor result = sr_blend_pixel((SrColor){0.2, 0.2, 0.2, 1},
                                    (SrColor){0.5, 0.5, 0.5, 1}, 1,
                                    SR_BLEND_ADD, false);
    CHECK_NEAR(t, result.r, 0.7, NEAR_EPS);
}

static void test_blend_screen(sr_test_ctx *t)
{
    SrColor result = sr_blend_pixel((SrColor){0.2, 0.2, 0.2, 1},
                                    (SrColor){0.5, 0.5, 0.5, 1}, 1,
                                    SR_BLEND_SCREEN, false);
    CHECK_NEAR(t, result.r, 0.6, NEAR_EPS);
}

static void test_blend_overlay(sr_test_ctx *t)
{
    SrColor result = sr_blend_pixel((SrColor){0.2, 0.2, 0.2, 1},
                                    (SrColor){0.8, 0.8, 0.8, 1}, 1,
                                    SR_BLEND_OVERLAY, false);
    CHECK_NEAR(t, result.r, 0.32, NEAR_EPS);
}

static void test_blend_difference(sr_test_ctx *t)
{
    SrColor result = sr_blend_pixel((SrColor){0.2, 0.4, 0.8, 1},
                                    (SrColor){0.5, 0.5, 0.5, 1}, 1,
                                    SR_BLEND_DIFFERENCE, false);
    CHECK_NEAR(t, result.r, 0.3, NEAR_EPS);
    CHECK_NEAR(t, result.g, 0.1, NEAR_EPS);
    CHECK_NEAR(t, result.b, 0.3, NEAR_EPS);
}

const sr_test_case sr_tests_compositor[] = {
    {"blend_normal", test_blend_normal},
    {"blend_multiply", test_blend_multiply},
    {"blend_add", test_blend_add},
    {"blend_screen", test_blend_screen},
    {"blend_overlay", test_blend_overlay},
    {"blend_difference", test_blend_difference},
    {NULL, NULL},
};
