/* SPDX-License-Identifier: Apache-2.0 */
#include "scene_render/color.h"
#include "scene_render/compositor.h"

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

static void test_convert_frame_thread_invariant(sr_test_ctx *t)
{
    SrFrame single = {0}, parallel = {0};
    CHECK(t, sr_frame_init(&single, 17, 11) == SR_OK);
    CHECK(t, sr_frame_init(&parallel, 17, 11) == SR_OK);
    if (!single.rgba || !parallel.rgba) {
        sr_frame_free(&single);
        sr_frame_free(&parallel);
        return;
    }
    for (size_t i = 0; i < (size_t)17 * 11 * 4; ++i)
        single.rgba[i] = parallel.rgba[i] = (uint8_t)((i * 73U) & 255U);
    CHECK(t, sr_color_convert_frame(&single, SR_COLOR_DISPLAY_P3,
                                    SR_COLOR_SRGB, 1) == SR_OK);
    CHECK(t, sr_color_convert_frame(&parallel, SR_COLOR_DISPLAY_P3,
                                    SR_COLOR_SRGB, 4) == SR_OK);
    CHECK(t, memcmp(single.rgba, parallel.rgba, (size_t)17 * 11 * 4) == 0);
    sr_frame_free(&single);
    sr_frame_free(&parallel);
}

const sr_test_case sr_tests_color[] = {
    {"space_parse_and_name", test_space_parse_and_name},
    {"rec709_decode_differs_from_srgb", test_rec709_decode_differs_from_srgb},
    {"convert_frame_thread_invariant", test_convert_frame_thread_invariant},
    {NULL, NULL},
};
