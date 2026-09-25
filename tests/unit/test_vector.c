/* SPDX-License-Identifier: Apache-2.0 */
#include "scene_render/vector_path.h"

#include "harness.h"

static void test_triangle_coverage(sr_test_ctx *t)
{
    static float pixels[32 * 32 * 4];
    FILE *sink = tmpfile();
    CHECK(t, sink != NULL);
    if (!sink) return;
    SrDiagnostics diag;
    sr_diag_init(&diag, "unit-vector", sink);
    SrVectorStyle style = {SR_FILL_EVENODD, {1, .5, .25, 1}, {0, 0, 0, 0}, 0};
    const float fill_px[4] = {1, .5f, .25f, 1}, none[4] = {0, 0, 0, 0};
    CHECK(t, sr_vector_path_render("M 2 30 L 16 2 L 30 30 Z", &style, fill_px,
                                   none, 32, 32, pixels, 4, &diag) == SR_OK);
    size_t opaque = 0;
    for (size_t i = 0; i < 32U * 32U; ++i) opaque += pixels[i * 4 + 3] > 0.0f;
    CHECK(t, opaque > 300 && opaque < 500);
    CHECK_INT(t, diag.errors, 0);
    fclose(sink);
}

static void test_invalid_path_reports_error(sr_test_ctx *t)
{
    static float pixels[32 * 32 * 4];
    FILE *sink = tmpfile();
    CHECK(t, sink != NULL);
    if (!sink) return;
    SrDiagnostics diag;
    sr_diag_init(&diag, "unit-vector", sink);
    SrVectorStyle style = {SR_FILL_EVENODD, {1, 1, 1, 1}, {0, 0, 0, 0}, 0};
    const float white[4] = {1, 1, 1, 1}, none[4] = {0, 0, 0, 0};
    CHECK(t, sr_vector_path_render("M nope", &style, white, none, 32, 32,
                                   pixels, 8, &diag) == SR_ERR_ASSET);
    CHECK_INT(t, diag.errors, 1);
    fclose(sink);
}

const sr_test_case sr_tests_vector[] = {
    {"triangle_coverage", test_triangle_coverage},
    {"invalid_path_reports_error", test_invalid_path_reports_error},
    {NULL, NULL},
};
