/* SPDX-License-Identifier: Apache-2.0 */
#include "scene_render/vector_path.h"

#include "harness.h"

#define W 100
#define H 100

static float fill[W * H], stroke[W * H];

static double total(const float *coverage, size_t count)
{
    double sum = 0.0;
    for (size_t i = 0; i < count; ++i) sum += coverage[i];
    return sum;
}

static void test_unit_square_exact(sr_test_ctx *t)
{
    CHECK(t, sr_vector_path_coverage("M 2 2 L 3 2 L 3 3 L 2 3 Z",
                                     SR_FILL_NONZERO, 0.0, 8, 8, fill,
                                     NULL) == SR_OK);
    CHECK_NEAR(t, total(fill, 64), 1.0, 0.0);
    CHECK_NEAR(t, fill[2 * 8 + 2], 1.0, 0.0);
    /* Offset by half a pixel: four quarter-covered pixels, same area. */
    CHECK(t, sr_vector_path_coverage("M 2.5 2.5 l 1 0 l 0 1 l -1 0 z",
                                     SR_FILL_EVENODD, 0.0, 8, 8, fill,
                                     NULL) == SR_OK);
    CHECK_NEAR(t, total(fill, 64), 1.0, 1e-6);
    CHECK_NEAR(t, fill[2 * 8 + 2], 0.25, 1e-6);
    CHECK_NEAR(t, fill[3 * 8 + 3], 0.25, 1e-6);
    /* A path reaching past the grid is clipped, not wrapped. */
    CHECK(t, sr_vector_path_coverage("M -4 0 L 4 0 L 4 8 L -4 8 Z",
                                     SR_FILL_NONZERO, 0.0, 8, 8, fill,
                                     NULL) == SR_OK);
    CHECK_NEAR(t, total(fill, 64), 32.0, 1e-5);
}

static const char star[] = "M 50 5 L 79 95 L 2 39 L 98 39 L 21 95 Z";

static void test_star_fill_rules(sr_test_ctx *t)
{
    CHECK(t, sr_vector_path_coverage(star, SR_FILL_NONZERO, 0.0, W, H, fill,
                                     NULL) == SR_OK);
    double nonzero = total(fill, W * H);
    CHECK_NEAR(t, fill[55 * W + 50], 1.0, 1e-6);
    CHECK(t, sr_vector_path_coverage(star, SR_FILL_EVENODD, 0.0, W, H, fill,
                                     NULL) == SR_OK);
    double evenodd = total(fill, W * H);
    CHECK_NEAR(t, fill[55 * W + 50], 0.0, 1e-6);
    CHECK_NEAR(t, fill[20 * W + 50], 1.0, 1e-6);  /* a point of the star */
    CHECK(t, nonzero - evenodd > 500.0);
}

static void test_stroke_width_coverage(sr_test_ctx *t)
{
    /* An open 40 px horizontal line stroked 4 px wide: a 40 x 4 band plus
     * two round caps, each half of a radius-2 disc, i.e. one disc. */
    CHECK(t, sr_vector_path_coverage("M 30 50 L 70 50", SR_FILL_NONZERO, 4.0,
                                     W, H, fill, stroke) == SR_OK);
    CHECK_NEAR(t, total(fill, W * H), 0.0, 1e-6);
    double disc = SR_PI * 2.0 * 2.0;
    CHECK_NEAR(t, total(stroke, W * H), 40.0 * 4.0 + disc, 0.5);
    CHECK_NEAR(t, stroke[48 * W + 50], 1.0, 1e-6);
    CHECK_NEAR(t, stroke[51 * W + 50], 1.0, 1e-6);
    CHECK_NEAR(t, stroke[46 * W + 50], 0.0, 1e-6);
    CHECK_NEAR(t, stroke[53 * W + 50], 0.0, 1e-6);
}

static void test_render_premultiplied(sr_test_ctx *t)
{
    float px[16 * 16 * 4];
    SrVectorStyle style = {SR_FILL_NONZERO, {1, 0, 0, 1}, {0, 0, 1, 1}, 2.0};
    const float red[4] = {1, 0, 0, 1}, blue[4] = {0, 0, 1, 1};
    FILE *sink = tmpfile();
    SrDiagnostics diag;
    sr_diag_init(&diag, "unit-path", sink ? sink : stderr);
    CHECK(t, sr_vector_path_render("M 4 4 H 12 V 12 H 4 Z", &style, red, blue,
                                   16, 16, px, 1, &diag) == SR_OK);
    const float *center = &px[(8 * 16 + 8) * 4];
    CHECK_NEAR(t, center[0], 1.0, 1e-6);
    CHECK_NEAR(t, center[2], 0.0, 1e-6);
    CHECK_NEAR(t, center[3], 1.0, 1e-6);
    const float *edge = &px[(8 * 16 + 4) * 4];  /* on the outline */
    CHECK_NEAR(t, edge[2], 1.0, 1e-6);
    CHECK_NEAR(t, edge[3], 1.0, 1e-6);
    CHECK_NEAR(t, px[(1 * 16 + 1) * 4 + 3], 0.0, 1e-6);
    if (sink) fclose(sink);
}

static void test_stroke_joins_not_double_counted(sr_test_ctx *t)
{
    /* A collinear vertex inside a 0.5 px wide stroke: the round join at
     * (4.5, 2.5) must not add coverage on top of the segment band. */
    CHECK(t, sr_vector_path_coverage("M 2 2.5 L 4.5 2.5 L 8 2.5",
                                     SR_FILL_NONZERO, 0.5, W, H, fill,
                                     stroke) == SR_OK);
    CHECK_NEAR(t, stroke[2 * W + 4], 0.5, 1e-6);
}

static void test_left_edge_rounding_stays_in_bounds(sr_test_ctx *t)
{
    /* Slope rounding puts the first intersection a hair left of x = 0. */
    float small_fill[2];
    CHECK(t, sr_vector_path_coverage("M .1 .1 L 0 .3 L 1 .3 Z",
                                     SR_FILL_NONZERO, 0.0, 2, 1, small_fill,
                                     NULL) == SR_OK);
    CHECK(t, small_fill[0] >= 0.0f && small_fill[0] <= 1.0f);
    /* Coordinates far beyond int range are clipped, not converted. */
    CHECK(t, sr_vector_path_coverage("M 0 0 L 1 3000000000 L 0 3000000000 Z",
                                     SR_FILL_NONZERO, 0.0, W, H, fill,
                                     NULL) == SR_OK);
    CHECK(t, fill[(H - 1) * W] > 0.0f);
}

const sr_test_case sr_tests_path[] = {
    {"unit_square_exact", test_unit_square_exact},
    {"star_fill_rules", test_star_fill_rules},
    {"stroke_width_coverage", test_stroke_width_coverage},
    {"render_premultiplied", test_render_premultiplied},
    {"stroke_joins_not_double_counted", test_stroke_joins_not_double_counted},
    {"left_edge_rounding_stays_in_bounds", test_left_edge_rounding_stays_in_bounds},
    {NULL, NULL},
};
