/* SPDX-License-Identifier: Apache-2.0 */
#include "scene_render/timeline.h"

#include "harness.h"

#define NEAR_EPS 1e-6

static void test_curves(sr_test_ctx *t)
{
    SrTrack track = {0};
    CHECK(t, sr_track_add(&track, (SrKeyframe){.time = 2, .value = 20,
                                               .curve = SR_CURVE_LINEAR}) == SR_OK);
    CHECK(t, sr_track_add(&track, (SrKeyframe){.time = 0, .value = 0,
                                               .curve = SR_CURVE_LINEAR}) == SR_OK);
    CHECK(t, sr_track_finalize(&track) == SR_OK);
    CHECK_NEAR(t, sr_track_eval(&track, -1, -1), 0, NEAR_EPS);
    CHECK_NEAR(t, sr_track_eval(&track, -1, 1), 10, NEAR_EPS);
    CHECK_NEAR(t, sr_track_eval(&track, -1, 3), 20, NEAR_EPS);
    track.keys[0].curve = SR_CURVE_STEP;
    CHECK_NEAR(t, sr_track_eval(&track, -1, 1.999), 0, NEAR_EPS);
    track.keys[0].curve = SR_CURVE_EASE_IN;
    CHECK_NEAR(t, sr_track_eval(&track, -1, 1), 2.5, NEAR_EPS);
    track.keys[0].curve = SR_CURVE_EASE_OUT;
    CHECK_NEAR(t, sr_track_eval(&track, -1, 1), 17.5, NEAR_EPS);
    track.keys[0].curve = SR_CURVE_EASE_IN_OUT;
    CHECK_NEAR(t, sr_track_eval(&track, -1, 1), 10, NEAR_EPS);
    track.keys[0].curve = SR_CURVE_BEZIER;
    track.keys[0].x1 = 0;
    track.keys[0].y1 = 0;
    track.keys[0].x2 = 1;
    track.keys[0].y2 = 1;
    CHECK_NEAR(t, sr_track_eval(&track, -1, 1), 10, 1e-5);
    sr_track_free(&track);
}

static void test_duplicate_key_rejected(sr_test_ctx *t)
{
    SrTrack duplicate = {0};
    sr_track_add(&duplicate, (SrKeyframe){.time = 1});
    sr_track_add(&duplicate, (SrKeyframe){.time = 1});
    CHECK(t, sr_track_finalize(&duplicate) == SR_ERR_XML);
    sr_track_free(&duplicate);
}

const sr_test_case sr_tests_timeline[] = {
    {"curves", test_curves},
    {"duplicate_key_rejected", test_duplicate_key_rejected},
    {NULL, NULL},
};
