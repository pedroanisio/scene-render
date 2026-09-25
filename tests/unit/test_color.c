/* SPDX-License-Identifier: Apache-2.0 */
#include "scene_render/color.h"
#include "scene_render/compositor.h"

#include <stdlib.h>

#include "harness.h"

static void test_space_parse_and_name(sr_test_ctx *t)
{
    SrColorSpace parsed = SR_COLOR_SRGB;
    CHECK(t, sr_color_space_parse("rec709", &parsed));
    CHECK(t, parsed == SR_COLOR_REC709);
    CHECK_STR(t, sr_color_space_name(parsed), "rec709");
}

static void test_rec709_decode_differs_from_srgb(sr_test_ctx *t)
{
    CHECK(t, fabs(sr_color_decode(.05, SR_COLOR_REC709) -
                  sr_color_decode(.05, SR_COLOR_SRGB)) > 1e-3);
}

static SrProject project(bool linear, SrColorSpace working)
{
    SrProject p = {0};
    p.linear_light = linear;
    p.working_color_space = working;
    return p;
}

/* Converts rgba8 through blend space and back; returns false on failure. */
static bool round_trip(sr_test_ctx *t, const SrProject *p, SrColorSpace source,
                       SrColorSpace target, const uint8_t *in, uint32_t count,
                       uint8_t *out, unsigned threads)
{
    SrImage image = {0};
    SrColorOutput output = {0};
    bool ok = sr_color_image_from_rgba8(p, source, in, (size_t)count * 4,
                                        count, 1, &image) == SR_OK &&
              sr_color_output_init(&output, p, target) == SR_OK;
    CHECK(t, ok);
    if (ok) {
        SrFrame frame = {image.width, image.height, image.px};
        ok = sr_color_convert_frame(&output, &frame, out, threads) == SR_OK;
        CHECK(t, ok);
    }
    sr_color_output_free(&output);
    free(image.px);
    return ok;
}

static void test_grey_round_trip_identity(sr_test_ctx *t)
{
    uint8_t in[256 * 4], out[256 * 4];
    for (int i = 0; i < 256; ++i) {
        in[i * 4] = in[i * 4 + 1] = in[i * 4 + 2] = (uint8_t)i;
        in[i * 4 + 3] = 255;
    }
    for (int linear = 0; linear < 2; ++linear) {
        SrProject p = project(linear != 0, SR_COLOR_SRGB);
        memset(out, 0, sizeof(out));
        if (!round_trip(t, &p, SR_COLOR_SRGB, SR_COLOR_SRGB, in, 256, out, 3))
            continue;
        int mismatches = 0;
        for (int i = 0; i < 256 * 4; ++i) mismatches += in[i] != out[i];
        if (mismatches) SR_FAIL(t, "linear=%d: %d channel mismatches", linear,
                                mismatches);
    }
}

static void test_alpha_zero_safe(sr_test_ctx *t)
{
    const uint8_t in[8] = {200, 100, 50, 0, 10, 20, 30, 128};
    uint8_t out[8] = {1, 1, 1, 1, 1, 1, 1, 1};
    SrProject p = project(true, SR_COLOR_SRGB);
    SrImage image = {0};
    CHECK(t, sr_color_image_from_rgba8(&p, SR_COLOR_SRGB, in, 8, 2, 1,
                                       &image) == SR_OK);
    if (image.px) {
        for (int c = 0; c < 4; ++c) CHECK(t, image.px[c] == 0.0f);
        CHECK_NEAR(t, image.px[7], 128.0 / 255.0, 1e-7);
        free(image.px);
    }
    if (round_trip(t, &p, SR_COLOR_SRGB, SR_COLOR_SRGB, in, 2, out, 1)) {
        for (int c = 0; c < 4; ++c) CHECK_INT(t, out[c], 0);
        /* Straight color survives premultiply/unpremultiply at alpha 0.5. */
        CHECK(t, abs(out[4] - 10) <= 1 && abs(out[5] - 20) <= 1 &&
                 abs(out[6] - 30) <= 1);
        CHECK_INT(t, out[7], 128);
    }
}

static void test_gamut_conversion(sr_test_ctx *t)
{
    /* Pure P3 red is outside sRGB: it clips to full red. */
    const uint8_t in[4] = {255, 0, 0, 255};
    uint8_t out[4];
    SrProject p = project(false, SR_COLOR_DISPLAY_P3);
    if (round_trip(t, &p, SR_COLOR_DISPLAY_P3, SR_COLOR_SRGB, in, 1, out, 1)) {
        CHECK_INT(t, out[0], 255);
        CHECK_INT(t, out[1], 0);
        CHECK_INT(t, out[3], 255);
    }
    /* sRGB white stays white through a P3 working space. */
    const uint8_t white[4] = {255, 255, 255, 255};
    if (round_trip(t, &p, SR_COLOR_SRGB, SR_COLOR_SRGB, white, 1, out, 1)) {
        CHECK(t, out[0] >= 254 && out[1] >= 254 && out[2] >= 254);
    }
}

static void test_convert_frame_thread_invariant(sr_test_ctx *t)
{
    SrFrame frame = {0};
    CHECK(t, sr_frame_init(&frame, 17, 11) == SR_OK);
    if (!frame.px) return;
    for (size_t i = 0; i < (size_t)17 * 11 * 4; ++i)
        frame.px[i] = (float)((i * 73U) % 257U) / 200.0f;
    SrProject p = project(true, SR_COLOR_DISPLAY_P3);
    SrColorOutput output;
    CHECK(t, sr_color_output_init(&output, &p, SR_COLOR_SRGB) == SR_OK);
    uint8_t single[17 * 11 * 4], parallel[17 * 11 * 4];
    CHECK(t, sr_color_convert_frame(&output, &frame, single, 1) == SR_OK);
    CHECK(t, sr_color_convert_frame(&output, &frame, parallel, 4) == SR_OK);
    CHECK(t, memcmp(single, parallel, sizeof(single)) == 0);
    sr_color_output_free(&output);
    sr_frame_free(&frame);
}

static void test_to_blend(sr_test_ctx *t)
{
    float out[4];
    SrProject p = project(true, SR_COLOR_SRGB);
    sr_color_to_blend(&p, (SrColor){0.5, 1.0, 0.0, 0.5}, out);
    CHECK_NEAR(t, out[0], sr_color_decode(0.5, SR_COLOR_SRGB) * 0.5, 1e-7);
    CHECK_NEAR(t, out[1], 0.5, 1e-7);
    CHECK_NEAR(t, out[3], 0.5, 1e-7);
    p.linear_light = false;
    sr_color_to_blend(&p, (SrColor){0.5, 1.0, 0.0, 0.5}, out);
    CHECK_NEAR(t, out[0], 0.25, 1e-7);
}

const sr_test_case sr_tests_color[] = {
    {"space_parse_and_name", test_space_parse_and_name},
    {"rec709_decode_differs_from_srgb", test_rec709_decode_differs_from_srgb},
    {"grey_round_trip_identity", test_grey_round_trip_identity},
    {"alpha_zero_safe", test_alpha_zero_safe},
    {"gamut_conversion", test_gamut_conversion},
    {"convert_frame_thread_invariant", test_convert_frame_thread_invariant},
    {"to_blend", test_to_blend},
    {NULL, NULL},
};
