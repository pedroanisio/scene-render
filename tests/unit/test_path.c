/* SPDX-License-Identifier: Apache-2.0 */
#include "scene_render/vector_path.h"
#include "scene_render/parallel.h"
#include "vector_path_internal.h"

#include <stdlib.h>

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

static void prepared_path_geometry_and_lifetime(sr_test_ctx *t) {
    SrPreparedPath path = {0};
    const char *text = "M .5 .5 H 2.5 V 2.5 H .5 Z M 4 0 q 2 2 4 0 c 1 1 2 1 3 0";
    CHECK_INT(t, sr_prepared_path_parse(text, &path), SR_OK);
    CHECK_INT(t, path.count, 2);
    if (path.count == 2) {
        CHECK_INT(t, path.items[0].count, 5);
        CHECK(t, path.items[0].closed);
        CHECK_INT(t, path.items[1].count, 29);
        CHECK(t, !path.items[1].closed);
        const SrPathPoint *p = path.items[1].points;
        CHECK_NEAR(t, p[6].x, 6, 1e-14);
        CHECK_NEAR(t, p[6].y, 1, 1e-14);
        CHECK_NEAR(t, p[12].x, 8, 0);
        CHECK_NEAR(t, p[12].y, 0, 0);
        CHECK_NEAR(t, p[20].x, 9.5, 1e-14);
        CHECK_NEAR(t, p[20].y, .75, 1e-14);
        CHECK_NEAR(t, p[28].x, 11, 0);
        CHECK_NEAR(t, p[28].y, 0, 0);
    }
    sr_prepared_path_free(&path);
    CHECK(t, path.items == NULL && path.count == 0 && path.capacity == 0);
    sr_prepared_path_free(&path);
    sr_prepared_path_free(NULL);

    CHECK_INT(t, sr_prepared_path_parse("M .5 .5 h 2 v 2 h -2 z", &path), SR_OK);
    float output[16], outline[16];
    CHECK_INT(t, sr_prepared_path_coverage(&path, SR_FILL_NONZERO, 0,
                                           4, 4, output, outline), SR_OK);
    const float axis[] = {.5f, 1, .5f, 0};
    for (size_t y = 0; y < 4; ++y) for (size_t x = 0; x < 4; ++x) {
        CHECK_NEAR(t, output[y*4+x], axis[x]*axis[y], 0);
        CHECK_NEAR(t, outline[y*4+x], 0, 0);
    }
    sr_prepared_path_free(&path);
    const char *bad[] = {NULL, "", "M 0 0", "M 0 0 L 1 1 M 5 5 Q 2",
                         "M 0 0 L 1 1 M 4 4 L 5 5 A 1 1"};
    for (size_t i = 0; i < sizeof(bad)/sizeof(bad[0]); ++i) {
        CHECK_INT(t, sr_prepared_path_parse(bad[i], &path), SR_ERR_ASSET);
        CHECK(t, path.items == NULL && path.count == 0 && path.capacity == 0);
        sr_prepared_path_free(&path);
    }
    CHECK_INT(t, sr_prepared_path_parse(text, NULL), SR_ERR_ARGUMENT);
}

typedef struct {
    const SrPreparedPath *path;
    float fill[8][16 * 16], stroke[8][16 * 16];
    SrStatus status[8];
} PreparedJobs;

static void prepared_raster_jobs(void *opaque, size_t begin, size_t end) {
    PreparedJobs *jobs = opaque;
    for (size_t i = begin; i < end; ++i)
        jobs->status[i] = sr_prepared_path_coverage(jobs->path,
            i % 2 ? SR_FILL_EVENODD : SR_FILL_NONZERO, i % 3 ? 1.5 : 0,
            16, 16, jobs->fill[i], jobs->stroke[i]);
}

static void prepared_path_reuse_and_threads(sr_test_ctx *t) {
    const char *text = "M 1 2 C 3 14 11 -2 14 12 L 2 12 Z "
                       "M 3 3 Q 10 1 12 10 L 4 12 Z M -2 8 H 18";
    SrPreparedPath path = {0};
    CHECK_INT(t, sr_prepared_path_parse(text, &path), SR_OK);
    if (!path.count) return;
    SrPreparedPath saved = path;
    uint64_t before = SR_FNV_OFFSET;
    for (size_t i = 0; i < path.count; ++i) {
        before = sr_fnv1a64(before, &path.items[i], sizeof(path.items[i]));
        before = sr_fnv1a64(before, path.items[i].points,
                            path.items[i].count * sizeof(*path.items[i].points));
    }
    PreparedJobs *jobs = sr_alloc(sizeof(*jobs));
    CHECK(t, jobs != NULL);
    if (jobs) {
        jobs->path = &path;
        CHECK_INT(t, sr_parallel_for(8, 4, prepared_raster_jobs, jobs), SR_OK);
        for (size_t i = 0; i < 8; ++i) {
            float expected[256], outline[256];
            CHECK_INT(t, jobs->status[i], SR_OK);
            CHECK_INT(t, sr_vector_path_coverage(text,
                i % 2 ? SR_FILL_EVENODD : SR_FILL_NONZERO, i % 3 ? 1.5 : 0,
                16, 16, expected, outline), SR_OK);
            CHECK(t, !memcmp(expected, jobs->fill[i], sizeof(expected)));
            CHECK(t, !memcmp(outline, jobs->stroke[i], sizeof(outline)));
        }
        /* Reuse with different dimensions/rules, then repeat the first grid. */
        float other[7 * 19];
        CHECK_INT(t, sr_prepared_path_coverage(&path, SR_FILL_EVENODD,
                                               0, 7, 19, other, NULL), SR_OK);
        float repeated[256];
        CHECK_INT(t, sr_prepared_path_coverage(&path, SR_FILL_NONZERO,
                                               0, 16, 16, repeated, NULL), SR_OK);
        CHECK(t, !memcmp(repeated, jobs->fill[0], sizeof(repeated)));
        free(jobs);
    }
    uint64_t after = SR_FNV_OFFSET;
    for (size_t i = 0; i < path.count; ++i) {
        after = sr_fnv1a64(after, &path.items[i], sizeof(path.items[i]));
        after = sr_fnv1a64(after, path.items[i].points,
                           path.items[i].count * sizeof(*path.items[i].points));
    }
    CHECK_INT(t, path.count, saved.count);
    CHECK(t, !memcmp(&path, &saved, sizeof(path)));
    CHECK(t, before == after);
    sr_prepared_path_free(&path);
}

static void prepared_path_invalid_outputs(sr_test_ctx *t) {
    SrPreparedPath path = {0}, empty = {0};
    CHECK_INT(t, sr_prepared_path_parse("M 0 0 H 2 V 2 Z", &path), SR_OK);
    float out[16], sentinel[16];
    for (size_t i = 0; i < 16; ++i) out[i] = sentinel[i] = .375f;
    CHECK_INT(t, sr_prepared_path_coverage(NULL, SR_FILL_NONZERO,
                                           0, 4, 4, out, NULL), SR_ERR_ARGUMENT);
    CHECK_INT(t, sr_prepared_path_coverage(&empty, SR_FILL_NONZERO,
                                           0, 4, 4, out, NULL), SR_ERR_ARGUMENT);
    SrPreparedPath missing = {.count=1};
    CHECK_INT(t, sr_prepared_path_coverage(&missing, SR_FILL_NONZERO,
                                           0, 4, 4, out, NULL), SR_ERR_ARGUMENT);
    CHECK_INT(t, sr_prepared_path_coverage(&path, SR_FILL_NONZERO,
                                           0, 4, 4, NULL, NULL), SR_ERR_ARGUMENT);
    const uint32_t sizes[][2] = {{0,4}, {4,0}, {UINT32_MAX,4}, {4,UINT32_MAX}};
    for (size_t i = 0; i < sizeof(sizes)/sizeof(sizes[0]); ++i)
        CHECK_INT(t, sr_prepared_path_coverage(&path, SR_FILL_NONZERO,
                   0, sizes[i][0], sizes[i][1], out, NULL), SR_ERR_ARGUMENT);
    CHECK(t, !memcmp(out, sentinel, sizeof(out)));
    sr_prepared_path_free(&path);
}

const sr_test_case sr_tests_path[] = {
    {"unit_square_exact", test_unit_square_exact},
    {"prepared_geometry_and_lifetime", prepared_path_geometry_and_lifetime},
    {"prepared_reuse_and_threads", prepared_path_reuse_and_threads},
    {"prepared_invalid_outputs", prepared_path_invalid_outputs},
    {"star_fill_rules", test_star_fill_rules},
    {"stroke_width_coverage", test_stroke_width_coverage},
    {"render_premultiplied", test_render_premultiplied},
    {"stroke_joins_not_double_counted", test_stroke_joins_not_double_counted},
    {"left_edge_rounding_stays_in_bounds", test_left_edge_rounding_stays_in_bounds},
    {NULL, NULL},
};
