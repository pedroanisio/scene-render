/* SPDX-License-Identifier: Apache-2.0 */
#include "scene_render/length.h"
#include "length_internal.h"
#include "harness.h"

#include <float.h>

static void parser_values(sr_test_ctx *t) {
    static const struct {
        const char *text;
        double value;
        SrLengthUnit unit;
    } valid[] = {
        {"0", 0, SR_LENGTH_PIXELS}, {"-0", -0.0, SR_LENGTH_PIXELS},
        {"  +1e2  ", 100, SR_LENGTH_PIXELS},
        {"1e200", 1e200, SR_LENGTH_PIXELS},
        {"50%", 50, SR_LENGTH_PERCENT}, {"-.5vw", -0.5, SR_LENGTH_VW},
        {".5vh", 0.5, SR_LENGTH_VH}, {"1.vmin", 1, SR_LENGTH_VMIN},
        {"00025vmax", 25, SR_LENGTH_VMAX}, {"-0%", -0.0, SR_LENGTH_PERCENT},
        {"1000000%", SR_MAX_RELATIVE_LENGTH, SR_LENGTH_PERCENT},
        {"-1000000.0vh", -SR_MAX_RELATIVE_LENGTH, SR_LENGTH_VH},
    };
    for (size_t i = 0; i < sizeof(valid) / sizeof(valid[0]); ++i) {
        SrLength length = {17, SR_LENGTH_VMAX};
        CHECK(t, sr_parse_length(valid[i].text, &length));
        CHECK_NEAR(t, length.value, valid[i].value, 0);
        CHECK_INT(t, signbit(length.value) != 0, signbit(valid[i].value) != 0);
        CHECK_INT(t, length.unit, valid[i].unit);
    }
    static const char *const invalid[] = {
        "", " ", "%", "vw", "-%", ".%", "-.%", "+50%", "--1%", "1..0vh",
        "1e2%", "1E-2vw", " 50%", "50% ", "1 %", "1px", "1VW", "1vmi",
        "1vmaxx", "1%%", "nan%", "infvh", "nan", "inf", "1e400", "1e-400",
        "1000000.000001vmin", "-1000001vmax", "\xd9\xa1%",
    };
    for (size_t i = 0; i < sizeof(invalid) / sizeof(invalid[0]); ++i) {
        SrLength length = {17, SR_LENGTH_VMAX};
        CHECK(t, !sr_parse_length(invalid[i], &length));
        CHECK_NEAR(t, length.value, 17, 0);
        CHECK_INT(t, length.unit, SR_LENGTH_VMAX);
    }
    SrLength length = {0};
    CHECK(t, !sr_parse_length(NULL, &length));
    CHECK(t, !sr_parse_length("1%", NULL));
}

static void parser_byte_limit(sr_test_ctx *t) {
    char text[SR_MAX_LENGTH_BYTES + 2];
    memset(text, '0', sizeof(text));
    text[SR_MAX_LENGTH_BYTES - 2] = '1';
    text[SR_MAX_LENGTH_BYTES - 1] = '%';
    text[SR_MAX_LENGTH_BYTES] = '\0';
    SrLength length = {0};
    CHECK(t, sr_parse_length(text, &length));
    CHECK_NEAR(t, length.value, 1, 0);
    CHECK_INT(t, length.unit, SR_LENGTH_PERCENT);
    text[SR_MAX_LENGTH_BYTES - 1] = '0';
    text[SR_MAX_LENGTH_BYTES] = '%';
    text[SR_MAX_LENGTH_BYTES + 1] = '\0';
    CHECK(t, !sr_parse_length(text, &length));
    CHECK_NEAR(t, length.value, 1, 0);
    /* The new spelling limit must not restrict old unitless parsing. */
    char plain[SR_MAX_LENGTH_BYTES * 2 + 1];
    memset(plain, '0', sizeof(plain));
    plain[sizeof(plain) - 2] = '1';
    plain[sizeof(plain) - 1] = '\0';
    CHECK(t, sr_parse_length(plain, &length));
    CHECK_NEAR(t, length.value, 1, 0);
    CHECK_INT(t, length.unit, SR_LENGTH_PIXELS);
}

static void resolved_reference_values(sr_test_ctx *t) {
    const SrLengthBox frame = {320, 180};
    const double expected[] = {25, 20, 80, 45, 45, 80};
    for (SrLengthUnit unit = SR_LENGTH_PIXELS; unit <= SR_LENGTH_VMAX; ++unit) {
        double pixels = -1;
        CHECK_INT(t, sr_length_resolve((SrLength){25, unit}, 80,
                                       frame, true, &pixels), SR_OK);
        CHECK_NEAR(t, pixels, expected[unit], 0);
        CHECK_INT(t, sr_length_resolve((SrLength){-25, unit}, 80,
                                       frame, false, &pixels), SR_OK);
        CHECK_NEAR(t, pixels, -expected[unit], 0);
        CHECK_INT(t, sr_length_resolve((SrLength){-25, unit}, 80,
                                       frame, true, &pixels), SR_ERR_RENDER);
        CHECK_NEAR(t, pixels, -expected[unit], 0);
        CHECK_INT(t, sr_length_resolve((SrLength){0, unit}, 80,
                                       frame, true, &pixels), SR_ERR_RENDER);
    }
    double pixels;
    CHECK_INT(t, sr_length_resolve((SrLength){25, SR_LENGTH_VMIN}, NAN,
                                   (SrLengthBox){180, 320}, false, &pixels), SR_OK);
    CHECK_NEAR(t, pixels, 45, 0);
    CHECK_INT(t, sr_length_resolve((SrLength){25, SR_LENGTH_VMAX}, NAN,
                                   (SrLengthBox){180, 320}, false, &pixels), SR_OK);
    CHECK_NEAR(t, pixels, 80, 0);
    CHECK_INT(t, sr_length_resolve((SrLength){DBL_MAX, SR_LENGTH_PIXELS}, NAN,
                                   (SrLengthBox){0}, true, &pixels), SR_OK);
    CHECK_NEAR(t, pixels, DBL_MAX, 0);
}

static void resolution_limits(sr_test_ctx *t) {
    const SrLengthBox frame = {320, 180};
    double pixels = 7;
    const SrLength valid = {100, SR_LENGTH_PERCENT};
    CHECK_INT(t, sr_length_resolve(valid, SR_MAX_RESOLVED_LENGTH,
                                   frame, true, &pixels), SR_OK);
    CHECK_NEAR(t, pixels, SR_MAX_RESOLVED_LENGTH, 0);
    const double invalid[] = {NAN, INFINITY, -INFINITY};
    for (size_t i = 0; i < sizeof(invalid) / sizeof(invalid[0]); ++i) {
        CHECK_INT(t, sr_length_resolve((SrLength){invalid[i], SR_LENGTH_PIXELS},
                                       80, frame, false, &pixels), SR_ERR_RENDER);
        CHECK_INT(t, sr_length_resolve(valid, invalid[i], frame,
                                       false, &pixels), SR_ERR_RENDER);
        CHECK_INT(t, sr_length_resolve(valid, 80,
                                       (SrLengthBox){invalid[i], 180},
                                       false, &pixels), SR_ERR_RENDER);
        CHECK_INT(t, sr_length_resolve(valid, 80,
                                       (SrLengthBox){320, invalid[i]},
                                       false, &pixels), SR_ERR_RENDER);
    }
    CHECK_INT(t, sr_length_resolve(valid, 0, frame, false, &pixels), SR_ERR_RENDER);
    CHECK_INT(t, sr_length_resolve(valid, -1, frame, false, &pixels), SR_ERR_RENDER);
    CHECK_INT(t, sr_length_resolve(valid, DBL_MAX, frame,
                                   false, &pixels), SR_ERR_RENDER);
    CHECK_INT(t, sr_length_resolve(valid, SR_MAX_RESOLVED_LENGTH * 2,
                                   frame, false, &pixels), SR_ERR_RENDER);
    CHECK_INT(t, sr_length_resolve((SrLength){1e6 + 1, SR_LENGTH_VW}, 80,
                                   frame, false, &pixels), SR_ERR_RENDER);
    CHECK_INT(t, sr_length_resolve((SrLength){1, (SrLengthUnit)-1}, 80,
                                   frame, false, &pixels), SR_ERR_RENDER);
    CHECK_INT(t, sr_length_resolve((SrLength){1, (SrLengthUnit)99}, 80,
                                   frame, false, &pixels), SR_ERR_RENDER);
    CHECK_INT(t, sr_length_resolve(valid, 80, (SrLengthBox){0, 180},
                                   false, &pixels), SR_ERR_RENDER);
    CHECK_INT(t, sr_length_resolve(valid, 80, (SrLengthBox){320, 0},
                                   false, &pixels), SR_ERR_RENDER);
    CHECK_INT(t, sr_length_resolve(valid, 80, frame, false, NULL), SR_ERR_ARGUMENT);
    CHECK_NEAR(t, pixels, SR_MAX_RESOLVED_LENGTH, 0);
    CHECK_INT(t, sr_length_resolve((SrLength){DBL_MIN, SR_LENGTH_PERCENT}, DBL_MIN,
                                   frame, true, &pixels), SR_ERR_RENDER);
    CHECK_NEAR(t, pixels, SR_MAX_RESOLVED_LENGTH, 0);
}

static void parser_mutations(sr_test_ctx *t) {
    static const char *const seeds[] = {"50%", "-.5vw", "0vh", "1.vmin", "2.3vmax"};
    static const char mutations[] = "09.-+ %vwhaeE\t";
    uint32_t state = 0x517de03u;
    for (size_t s = 0; s < sizeof(seeds) / sizeof(seeds[0]); ++s) {
        size_t size = strlen(seeds[s]);
        for (size_t i = 0; i <= size; ++i) {
            char text[32];
            memcpy(text, seeds[s], i);
            text[i] = '\0';
            SrLength length = {0};
            if (sr_parse_length(text, &length)) CHECK(t, isfinite(length.value));
        }
        for (size_t i = 0; i < 64; ++i) {
            char text[32];
            strcpy(text, seeds[s]);
            state = state * UINT32_C(1664525) + UINT32_C(1013904223);
            size_t at = state % size;
            state = state * UINT32_C(1664525) + UINT32_C(1013904223);
            text[at] = mutations[state % (sizeof(mutations) - 1)];
            SrLength length = {0};
            if (sr_parse_length(text, &length)) {
                CHECK(t, isfinite(length.value));
                CHECK(t, length.unit >= SR_LENGTH_PIXELS && length.unit <= SR_LENGTH_VMAX);
                CHECK(t, length.unit == SR_LENGTH_PIXELS ||
                         fabs(length.value) <= SR_MAX_RELATIVE_LENGTH);
            }
        }
    }
}

static void mixed_track_reference(sr_test_ctx *t) {
    const double times[] = {0, 0.1, 0.3, 0.5, 0.7, 1.1, 1.6, 1.9};
    const double values[] = {20, -10, 30, 40, 15, 60, 75, 25};
    const SrLengthUnit units[] = {SR_LENGTH_PERCENT, SR_LENGTH_VW, SR_LENGTH_VH,
        SR_LENGTH_VMIN, SR_LENGTH_VMAX, SR_LENGTH_PIXELS, SR_LENGTH_VW, SR_LENGTH_PERCENT};
    const double converted[][8] = {
        {16, -32, 54, 72, 48, 60, 240, 20},
        {40, -64, 108, 144, 96, 60, 480, 50},
    };
    const SrLengthBox frames[] = {{320, 180}, {640, 360}};
    const double parents[] = {80, 200}, bases[] = {80, 160};
    for (SrCurve curve = SR_CURVE_STEP; curve < SR_CURVE_COUNT; ++curve) {
        SrKeyframe keys[8], literal_keys[8];
        for (size_t i = 0; i < 8; ++i) {
            keys[i] = (SrKeyframe){.time = times[i], .value = values[i], .unit = units[i],
                .curve = curve, .x1 = 0.25, .y1 = 0.1, .x2 = 0.25, .y2 = 1,
                .steps = 4, .step_start = true, .tension = 0.2,
                .continuity = -0.3, .bias = 0.4,
                .stiffness = 100, .damping = 6, .mass = 1};
        }
        SrAnimValue value = {.base = 25, .unit = SR_LENGTH_VW,
            .track = {.keys = keys, .count = 8, .capacity = 8}};
        CHECK_INT(t, sr_track_finalize(&value.track), SR_OK);
        CHECK(t, value.track.has_relative);
        SrKeyframe original[8];
        memcpy(original, keys, sizeof(keys));
        for (SrExtrapolation mode = SR_EXTRAPOLATE_HOLD;
             mode <= SR_EXTRAPOLATE_OFFSET; ++mode) {
            value.track.extrapolate_before = value.track.extrapolate_after = mode;
            for (unsigned additive = 0; additive < 2; ++additive) {
                value.track.additive = additive != 0;
                for (unsigned clock = 0; clock < 3; ++clock) {
                    value.track.clock_set = clock != 0;
                    value.track.time_base = (SrTimeBase)clock;
                    value.track.clock_scale = clock == 2 ? 0.5 : 2;
                    value.track.clock_offset = -0.2;
                    value.track.seconds_per_unit = 2;
                    SrTrack literal = value.track;
                    literal.keys = literal_keys;
                    literal.has_relative = false;
                    unsigned char snapshot[sizeof(value)];
                    memcpy(snapshot, &value, sizeof(value));
                    /* Alternating boxes and descending times exercise reuse. */
                    for (unsigned box = 0; box < 4; ++box) {
                        size_t f = box % 2;
                        for (size_t i = 0; i < 8; ++i) {
                            literal_keys[i] = keys[i];
                            literal_keys[i].value = converted[f][i];
                            literal_keys[i].unit = SR_LENGTH_PIXELS;
                        }
                        for (int sample = 120; sample >= -60; --sample) {
                            double time = (double)sample * 0.03;
                            double expected = sr_track_eval(&literal, bases[f], time);
                            double actual = -1;
                            CHECK_INT(t, sr_anim_length_eval(&value, time, parents[f],
                                                            frames[f], &actual), SR_OK);
                            if (memcmp(&expected, &actual, sizeof(actual))) {
                                SR_FAIL(t, "curve %d mode %d time %.17g: %.17g != %.17g",
                                        curve, mode, time, actual, expected);
                                return;
                            }
                        }
                    }
                    CHECK(t, !memcmp(snapshot, &value, sizeof(value)));
                }
            }
        }
        CHECK(t, !memcmp(keys, original, sizeof(keys)));
    }
}

static void animation_limits_and_bases(sr_test_ctx *t) {
    const SrLengthBox frame = {320, 180};
    SrAnimValue value = {.base = 50, .unit = SR_LENGTH_PERCENT};
    double pixels = -1;
    CHECK_INT(t, sr_anim_length_eval(&value, 0, 80, frame, &pixels), SR_OK);
    CHECK_NEAR(t, pixels, 40, 0);
    CHECK_INT(t, sr_track_add(&value.track, (SrKeyframe){.value = 50,
        .unit = SR_LENGTH_VW, .curve = SR_CURVE_LINEAR}), SR_OK);
    CHECK(t, value.track.has_relative);
    CHECK_INT(t, sr_anim_length_eval(&value, 0, 80, frame, &pixels), SR_OK);
    CHECK_NEAR(t, pixels, 160, 0);
    value.track.additive = true;
    CHECK_INT(t, sr_anim_length_eval(&value, 0, 80, frame, &pixels), SR_OK);
    CHECK_NEAR(t, pixels, 200, 0);
    CHECK_INT(t, sr_track_add(&value.track, (SrKeyframe){.time = 1,
        .value = 40, .unit = SR_LENGTH_PIXELS}), SR_OK);
    CHECK_INT(t, sr_track_finalize(&value.track), SR_OK);
    CHECK_INT(t, sr_anim_length_eval(&value, 0.5, 80, frame, &pixels), SR_OK);
    CHECK_NEAR(t, pixels, 140, 0);
    value.track.extrapolate_after = SR_EXTRAPOLATE_LINEAR;
    CHECK_INT(t, sr_anim_length_eval(&value, 1e100, 80, frame, &pixels), SR_ERR_RENDER);
    CHECK_NEAR(t, pixels, 140, 0);
    CHECK_INT(t, sr_anim_length_eval(&value, NAN, 80, frame, &pixels), SR_ERR_RENDER);
    CHECK_INT(t, sr_anim_length_eval(&value, 0, 80,
                                    (SrLengthBox){DBL_MAX, 180}, &pixels), SR_ERR_RENDER);
    value.unit = (SrLengthUnit)99;
    CHECK_INT(t, sr_anim_length_eval(&value, 0, 80, frame, &pixels), SR_ERR_RENDER);
    value.track.additive = false;
    CHECK_INT(t, sr_anim_length_eval(&value, 0, 80, frame, &pixels), SR_OK);
    CHECK_NEAR(t, pixels, 160, 0);
    value.track.clock_set = true;
    value.track.clock_scale = DBL_MAX;
    CHECK_INT(t, sr_anim_length_eval(&value, 2, 80, frame, &pixels), SR_ERR_RENDER);
    CHECK_NEAR(t, pixels, 160, 0);
    CHECK_INT(t, sr_anim_length_eval(NULL, 0, 80, frame, &pixels), SR_ERR_ARGUMENT);
    CHECK_INT(t, sr_anim_length_eval(&value, 0, 80, frame, NULL), SR_ERR_ARGUMENT);
    sr_track_free(&value.track);
    value = (SrAnimValue){.base = 1e200};
    CHECK_INT(t, sr_anim_length_eval(&value, 0, 80, frame, &pixels), SR_OK);
    CHECK_NEAR(t, pixels, 1e200, 0);
    CHECK_INT(t, sr_track_add(&value.track, (SrKeyframe){.value = 1e200}), SR_OK);
    CHECK(t, !value.track.has_relative);
    CHECK_INT(t, sr_anim_length_eval(&value, 0, 80, frame, &pixels), SR_OK);
    CHECK_NEAR(t, pixels, 1e200, 0);
    value.track.keys[0].value = INFINITY;
    CHECK_INT(t, sr_anim_length_eval(&value, 0, 80, frame, &pixels), SR_ERR_RENDER);
    sr_track_free(&value.track);
}

static void invalid_track_units(sr_test_ctx *t) {
    SrKeyframe key = {.value = 1};
    SrTrack track = {.keys = &key, .count = 1};
    key.unit = (SrLengthUnit)-1;
    CHECK_INT(t, sr_track_finalize(&track), SR_ERR_XML);
    key.unit = (SrLengthUnit)99;
    CHECK_INT(t, sr_track_finalize(&track), SR_ERR_XML);
    key.unit = SR_LENGTH_PERCENT;
    key.value = SR_MAX_RELATIVE_LENGTH + 1;
    CHECK_INT(t, sr_track_finalize(&track), SR_ERR_XML);
    key.value = SR_MAX_RELATIVE_LENGTH;
    CHECK_INT(t, sr_track_finalize(&track), SR_OK);
    CHECK(t, track.has_relative);
}

static void refinalize_pixel_track(sr_test_ctx *t) {
    SrKeyframe key = {.value = 50, .unit = SR_LENGTH_VW};
    SrAnimValue value = {.track = {.keys = &key, .count = 1}};
    CHECK_INT(t, sr_track_finalize(&value.track), SR_OK);
    CHECK(t, value.track.has_relative);
    key.unit = SR_LENGTH_PIXELS;
    key.value = 1e200;
    CHECK_INT(t, sr_track_finalize(&value.track), SR_OK);
    CHECK(t, !value.track.has_relative);
    double pixels = -1;
    CHECK_INT(t, sr_anim_length_eval(&value, 0, 80,
                                    (SrLengthBox){320, 180}, &pixels), SR_OK);
    CHECK_NEAR(t, pixels, 1e200, 0);
}

static void relative_track_time_limits(sr_test_ctx *t) {
    SrKeyframe keys[] = {
        {.time = -1e308, .value = 50, .unit = SR_LENGTH_PERCENT,
         .curve = SR_CURVE_LINEAR},
        {.time = 1e308, .value = 100, .unit = SR_LENGTH_PERCENT},
    };
    SrAnimValue value = {.base = 25, .unit = SR_LENGTH_PERCENT,
        .track = {.keys = keys, .count = 2}};
    CHECK_INT(t, sr_track_finalize(&value.track), SR_ERR_XML);
    keys[0].time = -SR_MAX_ANIMATION_TIME;
    keys[1].time = SR_MAX_ANIMATION_TIME;
    CHECK_INT(t, sr_track_finalize(&value.track), SR_OK);
    double pixels = -1;
    CHECK_INT(t, sr_anim_length_eval(&value, 0, 80,
                                    (SrLengthBox){320, 180}, &pixels), SR_OK);
    CHECK_NEAR(t, pixels, 60, 0);
    keys[1].time = SR_MAX_ANIMATION_TIME + 1;
    CHECK_INT(t, sr_track_finalize(&value.track), SR_ERR_XML);
    /* A relative additive base also takes the already-bounded additive path. */
    keys[0].unit = keys[1].unit = SR_LENGTH_PIXELS;
    value.track.additive = true;
    CHECK_INT(t, sr_track_finalize(&value.track), SR_ERR_XML);
    keys[1].time = SR_MAX_ANIMATION_TIME;
    CHECK_INT(t, sr_track_finalize(&value.track), SR_OK);
    CHECK_INT(t, sr_anim_length_eval(&value, 0, 80,
                                    (SrLengthBox){320, 180}, &pixels), SR_OK);
    CHECK_NEAR(t, pixels, 95, 0);
    /* Ordinary numeric-only legacy tracks keep their existing time range. */
    keys[0].time = -1e308;
    keys[1].time = 1e308;
    value.track.additive = false;
    CHECK_INT(t, sr_track_finalize(&value.track), SR_OK);
}

const sr_test_case sr_tests_length[] = {
    {"parser_values", parser_values},
    {"parser_byte_limit", parser_byte_limit},
    {"resolved_reference_values", resolved_reference_values},
    {"resolution_limits", resolution_limits},
    {"parser_mutations", parser_mutations},
    {"mixed_track_reference", mixed_track_reference},
    {"animation_limits_and_bases", animation_limits_and_bases},
    {"invalid_track_units", invalid_track_units},
    {"refinalize_pixel_track", refinalize_pixel_track},
    {"relative_track_time_limits", relative_track_time_limits},
    {NULL, NULL},
};
