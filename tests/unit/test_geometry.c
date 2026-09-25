/* SPDX-License-Identifier: Apache-2.0 */
#include "scene_render/common.h"

#include "harness.h"

#define NEAR_EPS 1e-6

static void test_matrix_inverse_roundtrip(sr_test_ctx *t)
{
    SrMat3 matrix = sr_mat_multiply(sr_mat_translate(10, 20), sr_mat_scale(2, 3));
    SrVec2 point = sr_mat_point(matrix, (SrVec2){4, 5});
    CHECK_NEAR(t, point.x, 18, NEAR_EPS);
    CHECK_NEAR(t, point.y, 35, NEAR_EPS);
    SrMat3 inverse;
    CHECK(t, sr_mat_inverse(matrix, &inverse));
    point = sr_mat_point(inverse, point);
    CHECK_NEAR(t, point.x, 4, NEAR_EPS);
    CHECK_NEAR(t, point.y, 5, NEAR_EPS);
}

const sr_test_case sr_tests_geometry[] = {
    {"matrix_inverse_roundtrip", test_matrix_inverse_roundtrip},
    {NULL, NULL},
};
