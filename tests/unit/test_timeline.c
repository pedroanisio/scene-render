/* SPDX-License-Identifier: Apache-2.0 */
#include "scene_render/timeline.h"
#include "timeline_internal.h"

#include "harness.h"

#include <stdlib.h>

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

static bool same_neighborhood(sr_test_ctx *t, const SrTrack *track, double time) {
    size_t indices[SR_TRACK_NEIGHBORHOOD];
    SrKeyframe keys[SR_TRACK_NEIGHBORHOOD];
    unsigned char original[sizeof(*track)];
    memcpy(original, track, sizeof(original));
    size_t count = sr_track_neighborhood(track, time, indices);
    if (count > SR_TRACK_NEIGHBORHOOD || count > track->count) {
        SR_FAIL(t, "invalid neighborhood count %zu", count);
        return false;
    }
    if (track->count && (!count || indices[0] != 0 ||
                        indices[count - 1] != track->count - 1)) {
        SR_FAIL(t, "missing global endpoints");
        return false;
    }
    for (size_t i = 0; i < count; ++i) {
        if (indices[i] >= track->count || (i && indices[i - 1] >= indices[i])) {
            SR_FAIL(t, "invalid or unsorted key index");
            return false;
        }
        keys[i] = track->keys[indices[i]];
    }
    SrTrack subset = *track;
    subset.keys = keys;
    subset.count = subset.capacity = count;
    double full = sr_track_eval(track, -3.25, time);
    double selected = sr_track_eval(&subset, -3.25, time);
    if (memcmp(&full, &selected, sizeof(full))) {
        SR_FAIL(t, "curve %d, modes %d/%d, clock %.17g/%.17g, time %.17g: "
                "full %.17g, selected %.17g",
                track->count ? (int)track->keys[0].curve : -1,
                track->extrapolate_before, track->extrapolate_after,
                track->clock_scale, track->clock_offset, time, full, selected);
        return false;
    }
    CHECK(t, !memcmp(track, original, sizeof(original)));
    return true;
}

static void test_neighborhood_boundaries(sr_test_ctx *t) {
    size_t indices[SR_TRACK_NEIGHBORHOOD];
    CHECK_INT(t, sr_track_neighborhood(NULL, 0, indices), 0);
    SrKeyframe keys[] = {
        {.time = 0, .value = -0.0, .curve = SR_CURVE_LINEAR},
        {.time = 0.1, .value = 7, .curve = SR_CURVE_LINEAR},
        {.time = 0.3, .value = -2, .curve = SR_CURVE_LINEAR},
    };
    SrKeyframe original[3];
    memcpy(original, keys, sizeof(keys));
    SrTrack track = {.keys = keys, .count = 3, .capacity = 3};
    CHECK_INT(t, sr_track_neighborhood(&track, 0, NULL), 0);
    for (size_t count = 0; count <= 3; ++count) {
        track.count = count;
        for (SrExtrapolation mode = SR_EXTRAPOLATE_HOLD;
             mode <= SR_EXTRAPOLATE_OFFSET; ++mode) {
            track.extrapolate_before = track.extrapolate_after = mode;
            const double times[] = {-1e16, -0.9, -0.3, -0.0, 0.05, 0.1,
                                     0.2, 0.3, 0.6, 0.9, 1e16};
            for (size_t i = 0; i < sizeof(times) / sizeof(times[0]); ++i)
                if (!same_neighborhood(t, &track, times[i])) return;
        }
    }
    CHECK(t, !memcmp(keys, original, sizeof(keys)));
}

static void test_neighborhood_curves_and_clocks(sr_test_ctx *t) {
    static const double times[] = {0, 0.03, 0.07, 0.1, 0.14, 0.17, 0.2, 0.26, 0.3};
    static const double values[] = {5, -10, 4, 9, -2, 6, 1, -8, 3};
    static const struct {
        SrTimeBase base;
        bool set;
        double scale, offset, seconds;
    } clocks[] = {
        {SR_TIME_COMPOSITION, false, 1, 0, 1},
        {SR_TIME_LOCAL, true, 0.5, -0.25, 1},
        {SR_TIME_NORMALIZED, true, 1.0 / 3.0, -0.1, 3},
        {SR_TIME_LOCAL, true, -2, 0.5, 1},
        {SR_TIME_LOCAL, true, 0, 0.15, 1},
    };
    for (SrCurve curve = SR_CURVE_STEP; curve < SR_CURVE_COUNT; ++curve) {
        SrKeyframe keys[9];
        for (size_t i = 0; i < 9; ++i) {
            keys[i] = (SrKeyframe){.time = times[i], .value = values[i],
                .curve = curve, .x1 = 0.2, .y1 = -0.1, .x2 = 0.7, .y2 = 1.1,
                .steps = 5, .step_start = i % 2 == 0,
                .tension = (double)i / 16, .continuity = -0.5, .bias = 0.3,
                .stiffness = 120, .damping = 9, .mass = 2,
                .ease_in = {0.2, 0.4}, .ease_out = {0.6, 0.7},
                .ease_in_set = curve == SR_CURVE_BEZIER && i > 0,
                .ease_out_set = curve == SR_CURVE_BEZIER && i < 8};
        }
        SrTrack track = {.keys = keys, .count = 9, .capacity = 9};
        CHECK_INT(t, sr_track_finalize(&track), SR_OK);
        SrKeyframe original[9];
        memcpy(original, keys, sizeof(keys));
        for (SrExtrapolation before = SR_EXTRAPOLATE_HOLD;
             before <= SR_EXTRAPOLATE_OFFSET; ++before) {
            for (SrExtrapolation after = SR_EXTRAPOLATE_HOLD;
                 after <= SR_EXTRAPOLATE_OFFSET; ++after) {
                track.extrapolate_before = before;
                track.extrapolate_after = after;
                for (size_t c = 0; c < sizeof(clocks) / sizeof(clocks[0]); ++c) {
                    track.time_base = clocks[c].base;
                    track.clock_set = clocks[c].set;
                    track.clock_scale = clocks[c].scale;
                    track.clock_offset = clocks[c].offset;
                    track.seconds_per_unit = clocks[c].seconds;
                    for (unsigned additive = 0; additive < 2; ++additive) {
                        track.additive = additive != 0;
                        for (int sample = -80; sample <= 100; ++sample) {
                            double time = (double)sample * 0.0075;
                            if (!same_neighborhood(t, &track, time)) return;
                        }
                        const double edges[] = {-1e100, -1e16, -0.9, -0.6, -0.3,
                                                 0.3, 0.6, 0.9, 1e16, 1e100};
                        for (size_t i = 0; i < sizeof(edges) / sizeof(edges[0]); ++i)
                            if (!same_neighborhood(t, &track, edges[i])) return;
                        for (size_t i = 0; i < 9; ++i) {
                            double time = clocks[c].scale
                                ? (times[i] - clocks[c].offset) / clocks[c].scale
                                : times[i];
                            if (!same_neighborhood(t, &track, time) ||
                                !same_neighborhood(t, &track, nextafter(time, -INFINITY)) ||
                                !same_neighborhood(t, &track, nextafter(time, INFINITY)))
                                return;
                        }
                    }
                }
            }
        }
        CHECK(t, !memcmp(keys, original, sizeof(keys)));
    }
}

static void test_neighborhood_maximum_track(sr_test_ctx *t) {
    SrKeyframe *keys = sr_alloc(SR_MAX_TRACK_KEYS * sizeof(*keys));
    CHECK(t, keys != NULL);
    if (!keys) return;
    for (size_t i = 0; i < SR_MAX_TRACK_KEYS; ++i) {
        keys[i] = (SrKeyframe){.time = (double)i * 0.01,
            .value = (double)(i % 13), .curve = SR_CURVE_CATMULL_ROM};
    }
    SrTrack track = {.keys = keys, .count = SR_MAX_TRACK_KEYS,
        .capacity = SR_MAX_TRACK_KEYS, .extrapolate_before = SR_EXTRAPOLATE_OFFSET,
        .extrapolate_after = SR_EXTRAPOLATE_PING_PONG};
    for (size_t i = 0; i < SR_MAX_TRACK_KEYS; i += 257) {
        double time = keys[i].time + 0.003;
        if (!same_neighborhood(t, &track, time) ||
            !same_neighborhood(t, &track, time - 1000) ||
            !same_neighborhood(t, &track, time + 1000)) break;
    }
    free(keys);
}

const sr_test_case sr_tests_timeline[] = {
    {"curves", test_curves},
    {"duplicate_key_rejected", test_duplicate_key_rejected},
    {"neighborhood_boundaries", test_neighborhood_boundaries},
    {"neighborhood_curves_and_clocks", test_neighborhood_curves_and_clocks},
    {"neighborhood_maximum_track", test_neighborhood_maximum_track},
    {NULL, NULL},
};
