/* SPDX-License-Identifier: Apache-2.0 */
#include "scene_render/raster.h"

#include "harness.h"

#define EPS 1e-6

/* Hand-computed W3C separable results for premultiplied inputs:
 * backdrop straight (0.2, 0.4, 0.8) at alpha 0.5, source straight
 * (0.5, 0.5, 0.5) at alpha 0.5, so co = 0.25*cs + 0.25*cb + 0.25*B. */
static const float backdrop[4] = {0.1f, 0.2f, 0.4f, 0.5f};
static const float source[4] = {0.25f, 0.25f, 0.25f, 0.5f};

static void expect(sr_test_ctx *t, SrBlendMode mode, const double expected[3])
{
    float dst[4];
    memcpy(dst, backdrop, sizeof(dst));
    sr_blend_px(mode, dst, source);
    CHECK_NEAR(t, dst[0], expected[0], EPS);
    CHECK_NEAR(t, dst[1], expected[1], EPS);
    CHECK_NEAR(t, dst[2], expected[2], EPS);
    CHECK_NEAR(t, dst[3], 0.75, EPS);
}

static double w3c(double cb, double b)
{
    return 0.25 * 0.5 + 0.25 * cb + 0.25 * b;
}

static void test_normal(sr_test_ctx *t)
{
    const double e[3] = {0.25 + 0.05, 0.25 + 0.1, 0.25 + 0.2};
    expect(t, SR_BLEND_NORMAL, e);
}

static void test_multiply(sr_test_ctx *t)
{
    const double e[3] = {w3c(.2, .1), w3c(.4, .2), w3c(.8, .4)};
    expect(t, SR_BLEND_MULTIPLY, e);
}

static void test_screen(sr_test_ctx *t)
{
    const double e[3] = {w3c(.2, .6), w3c(.4, .7), w3c(.8, .9)};
    expect(t, SR_BLEND_SCREEN, e);
}

static void test_overlay(sr_test_ctx *t)
{
    /* cb <= 0.5: 2*cb*cs; cb > 0.5: screen(cs, 2cb-1). */
    const double e[3] = {w3c(.2, .2), w3c(.4, .4), w3c(.8, .5 + .6 - .3)};
    expect(t, SR_BLEND_OVERLAY, e);
}

static void test_difference(sr_test_ctx *t)
{
    const double e[3] = {w3c(.2, .3), w3c(.4, .1), w3c(.8, .3)};
    expect(t, SR_BLEND_DIFFERENCE, e);
}

static void test_add(sr_test_ctx *t)
{
    /* ADD reduces to cs' + cb'. */
    const double e[3] = {0.35, 0.45, 0.65};
    expect(t, SR_BLEND_ADD, e);
}

static void test_add_unclamped(sr_test_ctx *t)
{
    float dst[4] = {0.9f, 0.8f, 0.7f, 1.0f};
    const float src[4] = {0.6f, 0.6f, 0.6f, 1.0f};
    sr_blend_px(SR_BLEND_ADD, dst, src);
    CHECK_NEAR(t, dst[0], 1.5, EPS);
    CHECK_NEAR(t, dst[1], 1.4, EPS);
    CHECK_NEAR(t, dst[2], 1.3, EPS);
    CHECK_NEAR(t, dst[3], 1.0, EPS);
}

static void test_transparent_backdrop_identity(sr_test_ctx *t)
{
    for (int mode = SR_BLEND_NORMAL; mode <= SR_BLEND_DIFFERENCE; ++mode) {
        float dst[4] = {0, 0, 0, 0};
        sr_blend_px((SrBlendMode)mode, dst, source);
        for (int c = 0; c < 4; ++c) CHECK_NEAR(t, dst[c], source[c], EPS);
    }
}

static void test_transparent_source_noop(sr_test_ctx *t)
{
    for (int mode = SR_BLEND_NORMAL; mode <= SR_BLEND_DIFFERENCE; ++mode) {
        float dst[4];
        memcpy(dst, backdrop, sizeof(dst));
        const float clear[4] = {0, 0, 0, 0};
        sr_blend_px((SrBlendMode)mode, dst, clear);
        CHECK(t, memcmp(dst, backdrop, sizeof(dst)) == 0);
    }
}

static void test_tiny_alpha_keeps_backdrop(sr_test_ctx *t)
{
    float dst[4] = {0.5f, 0.5f, 0.5f, 1.0f};
    const float src[4] = {1e-40f, 1e-40f, 1e-40f, 1e-40f};
    for (int mode = SR_BLEND_ADD; mode <= SR_BLEND_DIFFERENCE; ++mode) {
        float d[4] = {dst[0], dst[1], dst[2], dst[3]};
        sr_blend_px((SrBlendMode)mode, d, src);
        CHECK_NEAR(t, d[0], 0.5, 1e-6);
        CHECK_NEAR(t, d[3], 1.0, 1e-6);
    }
}

const sr_test_case sr_tests_blend[] = {
    {"tiny_alpha_keeps_backdrop", test_tiny_alpha_keeps_backdrop},
    {"normal", test_normal},
    {"multiply", test_multiply},
    {"screen", test_screen},
    {"overlay", test_overlay},
    {"difference", test_difference},
    {"add", test_add},
    {"add_unclamped", test_add_unclamped},
    {"transparent_backdrop_identity", test_transparent_backdrop_identity},
    {"transparent_source_noop", test_transparent_source_noop},
    {NULL, NULL},
};
