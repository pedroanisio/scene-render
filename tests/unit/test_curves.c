/* SPDX-License-Identifier: Apache-2.0 */
#include "harness.h"
#include "scene_render/timeline.h"
#include "scene_render/color.h"
#include "scene_render/particles.h"
#include "scene_render/physics.h"
#include "scene_text.h"

#include <math.h>

static double evaluate(sr_test_ctx *t, SrKeyframe first, double time) {
    SrTrack track = {0};
    first.time = 0;
    first.value = 0;
    CHECK_INT(t, sr_track_add(&track, first), SR_OK);
    CHECK_INT(t, sr_track_add(&track, (SrKeyframe){.time = 1, .value = 1}), SR_OK);
    CHECK_INT(t, sr_track_finalize(&track), SR_OK);
    double value = sr_track_eval(&track, 0, time);
    sr_track_free(&track);
    return value;
}

static void penner_reference_values(sr_test_ctx *t) {
    /* Penner closed forms at t=1/4: in, out, in-out for each family. */
    static const double expected[][3] = {
        {0.0761204674887133, 0.3826834323650898, 0.1464466094067262},
        {0.0625, 0.4375, 0.125},
        {0.015625, 0.578125, 0.0625},
        {0.00390625, 0.68359375, 0.03125},
        {0.0009765625, 0.7626953125, 0.015625},
        {0.005524271728019903, 0.8232233047033631, 0.015625},
        {0.03175416344814574, 0.6614378277661477, 0.0669872981077807},
        {-0.0641365625, 0.8174096875, -0.09968184375},
        {-0.005524271728019903, 0.9116116523516816, 0.011969444423734044},
        {0.02734375, 0.47265625, 0.1171875},
    };
    for (size_t family = 0; family < 10; ++family) {
        for (size_t direction = 0; direction < 3; ++direction) {
            SrCurve curve = (SrCurve)(SR_CURVE_SINE_IN + family * 3 + direction);
            SrKeyframe key = {.curve = curve};
            CHECK_NEAR(t, evaluate(t, key, 0.25), expected[family][direction], 1e-12);
            CHECK_NEAR(t, evaluate(t, key, 0), 0, 0);
            CHECK_NEAR(t, evaluate(t, key, 1), 1, 0);
            SrCurve parsed;
            CHECK(t, sr_curve_parse(sr_curve_name(curve), &parsed));
            CHECK_INT(t, parsed, curve);
        }
    }
}

static void step_boundaries(sr_test_ctx *t) {
    SrCurve hold;
    CHECK(t, sr_curve_parse("hold", &hold));
    CHECK_INT(t, hold, SR_CURVE_STEP);
    SrKeyframe key = {.curve = SR_CURVE_STEPS, .steps = 4};
    CHECK_NEAR(t, evaluate(t, key, 0.24999), 0, 0);
    CHECK_NEAR(t, evaluate(t, key, 0.25), 0.25, 0);
    CHECK_NEAR(t, evaluate(t, key, 0.99999), 0.75, 0);
    key.step_start = true;
    CHECK_NEAR(t, evaluate(t, key, -1e-9), 0, 0);
    CHECK_NEAR(t, evaluate(t, key, 0), 0.25, 0);
    CHECK_NEAR(t, evaluate(t, key, 0.25), 0.5, 0);
    CHECK_NEAR(t, evaluate(t, key, 0.99999), 1, 0);
    SrTrack track = {0};
    for (int i = 0; i < 3; ++i) {
        key.time = i;
        key.value = i;
        CHECK_INT(t, sr_track_add(&track, key), SR_OK);
    }
    CHECK_INT(t, sr_track_finalize(&track), SR_OK);
    CHECK_NEAR(t, sr_track_eval(&track, 0, 1), 1.25, 0);
    CHECK_NEAR(t, sr_track_eval(&track, 0, 2), 2, 0);
    track.extrapolate_before = track.extrapolate_after = SR_EXTRAPOLATE_LOOP;
    CHECK_NEAR(t, sr_track_eval(&track, 0, 4), 0.25, 0);
    CHECK_NEAR(t, sr_track_eval(&track, 0, -2), 0.25, 0);
    track.extrapolate_before = track.extrapolate_after = SR_EXTRAPOLATE_PING_PONG;
    CHECK_NEAR(t, sr_track_eval(&track, 0, -2), 2, 0);
    CHECK_NEAR(t, sr_track_eval(&track, 0, 4), 0.25, 0);
    sr_track_free(&track);
}

static void hermite_neighbors(sr_test_ctx *t) {
    SrTrack track = {0};
    double times[] = {0, 1, 3, 4}, values[] = {0, 2, 4, 0};
    for (size_t i = 0; i < 4; ++i)
        CHECK_INT(t, sr_track_add(&track, (SrKeyframe){.time = times[i],
            .value = values[i], .curve = SR_CURVE_CATMULL_ROM}), SR_OK);
    CHECK_INT(t, sr_track_finalize(&track), SR_OK);
    CHECK_NEAR(t, sr_track_eval(&track, 0, 2), 3.75, 1e-12);
    CHECK_NEAR(t, sr_track_eval(&track, 0, 0.5), 1.0625, 1e-12);
    track.keys[1].curve = SR_CURVE_TCB;
    CHECK_NEAR(t, sr_track_eval(&track, 0, 2), 3.75, 1e-12);
    track.keys[1].tension = 0.5;
    track.keys[1].continuity = 0.5;
    track.keys[1].bias = -0.5;
    CHECK_NEAR(t, sr_track_eval(&track, 0, 2), 3.515625, 1e-12);
    sr_track_free(&track);
}

static void spring_regimes(sr_test_ctx *t) {
    const double damping[] = {0, 10, 20, 30};
    const double expected[] = {0.45969769413186023, 0.34029984660829826,
                               0.26424111765711533, 0.2133544006966314};
    SrKeyframe key = {.curve = SR_CURVE_SPRING, .stiffness = 100, .mass = 1};
    for (size_t i = 0; i < 4; ++i) {
        key.damping = damping[i];
        CHECK_NEAR(t, evaluate(t, key, 0.1), expected[i], 1e-12);
    }
    key.damping = 20.0 + 1e-11;
    CHECK_NEAR(t, evaluate(t, key, 0.1), expected[2], 1e-12);
    key.damping = 1e6;
    key.stiffness = key.mass = 1e-6;
    CHECK_NEAR(t, evaluate(t, key, 0.5), 5e-13, 1e-15);
}

static void extrapolation_and_clocks(sr_test_ctx *t) {
    SrTrack track = {0};
    CHECK_INT(t, sr_track_add(&track, (SrKeyframe){.time = 1, .value = 10,
                                                  .curve = SR_CURVE_LINEAR}), SR_OK);
    CHECK_INT(t, sr_track_add(&track, (SrKeyframe){.time = 3, .value = 20}), SR_OK);
    const double before[] = {10, 7.5, 17.5, 12.5, 7.5};
    const double after[] = {20, 22.5, 12.5, 17.5, 22.5};
    const char *names[] = {"hold", "linear", "loop", "ping-pong", "offset"};
    for (int i = 0; i <= SR_EXTRAPOLATE_OFFSET; ++i) {
        SrExtrapolation parsed;
        CHECK(t, sr_extrapolation_parse(names[i], &parsed));
        CHECK_INT(t, parsed, i);
        track.extrapolate_before = track.extrapolate_after = parsed;
        CHECK_NEAR(t, sr_track_eval(&track, 100, 0.5), before[i], 1e-12);
        CHECK_NEAR(t, sr_track_eval(&track, 100, 3.5), after[i], 1e-12);
    }
    track.additive = true;
    CHECK_NEAR(t, sr_track_eval(&track, 100, 2), 115, 0);
    track.clock_set = true;
    track.clock_scale = 0.5;
    track.clock_offset = -1;
    CHECK_NEAR(t, sr_track_eval(&track, 100, 6), 115, 0);
    track.count = 1;
    CHECK_NEAR(t, sr_track_eval(&track, 100, -100), 110, 0);
    sr_track_free(&track);
    CHECK(t, !sr_extrapolation_parse("bad", &track.extrapolate_before));
    CHECK(t, !sr_extrapolation_parse(NULL, &track.extrapolate_before));
    CHECK(t, !sr_extrapolation_parse("hold", NULL));
}

static void handles_and_bounds(sr_test_ctx *t) {
    SrAnimValue value = {0};
    SrKeyframe a = {.curve = SR_CURVE_BEZIER, .ease_out_set = true,
                    .ease_out = {0.4, 0.25}};
    SrKeyframe b = {.time = 1, .value = 1, .ease_in_set = true,
                    .ease_in = {0.3, 0.5}};
    CHECK_INT(t, sr_track_add(&value.track, a), SR_OK);
    CHECK_INT(t, sr_track_add(&value.track, b), SR_OK);
    CHECK_INT(t, sr_track_finalize(&value.track), SR_OK);
    CHECK_NEAR(t, value.track.keys[0].x1, 0.4, 0);
    CHECK_NEAR(t, value.track.keys[0].y1, 0.1, 0);
    CHECK_NEAR(t, value.track.keys[0].x2, 0.7, 1e-12);
    CHECK_NEAR(t, value.track.keys[0].y2, 0.85, 1e-12);
    value.track.keys[0].curve = SR_CURVE_ELASTIC_OUT;
    value.track.keys[0].ease_out_set = false;
    value.track.keys[1].ease_in_set = false;
    value.track.extrapolate_after = SR_EXTRAPOLATE_OFFSET;
    value.track.additive = true;
    value.base = 4;
    double high = sr_anim_upper_bound(&value, -2, 3);
    for (int i = -200; i <= 300; ++i)
        CHECK(t, sr_anim_eval(&value, i / 100.0) <= high + 1e-12);
    value.track.keys[0].curve = SR_CURVE_STEPS;
    CHECK_INT(t, sr_track_finalize(&value.track), SR_ERR_XML);
    value.track.keys[0].steps = SR_MAX_CURVE_STEPS + 1;
    CHECK_INT(t, sr_track_finalize(&value.track), SR_ERR_XML);
    value.track.keys[0].steps = 4;
    value.track.keys[1].value = SR_MAX_ANIMATION_VALUE * 2;
    CHECK_INT(t, sr_track_finalize(&value.track), SR_ERR_XML);
    sr_track_free(&value.track);
}

static void finalized_configuration_limits(sr_test_ctx *t) {
    SrTrack track = {.additive = true};
    CHECK_INT(t, sr_track_add(&track, (SrKeyframe){.value = 1}), SR_OK);
    CHECK_INT(t, sr_track_add(&track, (SrKeyframe){.time = 1, .value = 2}), SR_OK);
    CHECK_INT(t, sr_track_finalize(&track), SR_OK);
    SrTrack valid = track;
    for (int invalid = 0; invalid < 11; ++invalid) {
        track = valid;
        switch (invalid) {
        case 0: track.extrapolate_before = (SrExtrapolation)-1; break;
        case 1: track.extrapolate_after = (SrExtrapolation)99; break;
        case 2: track.time_base = (SrTimeBase)99; break;
        case 3: track.clock_set = true; track.clock_scale = NAN; break;
        case 4: track.clock_set = true; track.clock_offset = INFINITY; break;
        case 5:
            track.clock_set = true;
            track.clock_scale = SR_MAX_ANIMATION_VALUE * 2;
            break;
        case 6: track.domain_start = NAN; break;
        case 7: track.domain_end = INFINITY; break;
        case 8: track.time_base = SR_TIME_NORMALIZED; break;
        case 9:
            track.time_base = SR_TIME_NORMALIZED;
            track.seconds_per_unit = INFINITY;
            break;
        case 10: track.keys = NULL; break;
        }
        CHECK_INT(t, sr_track_finalize(&track), SR_ERR_XML);
    }
    track = valid;
    track.count = SR_MAX_TRACK_KEYS;
    CHECK_INT(t, sr_track_add(&track, (SrKeyframe){0}), SR_ERR_XML);
    track = valid;
    sr_track_free(&track);
}

static void normalized_spring_and_additive_color(sr_test_ctx *t) {
    SrTrack track = {.time_base = SR_TIME_NORMALIZED, .clock_set = true,
        .clock_scale = 0.5, .clock_offset = -0.5, .seconds_per_unit = 2,
        .domain_start = 1, .domain_end = 3};
    CHECK_INT(t, sr_track_add(&track, (SrKeyframe){.curve = SR_CURVE_SPRING,
        .stiffness = 100, .damping = 10, .mass = 1}), SR_OK);
    CHECK_INT(t, sr_track_add(&track, (SrKeyframe){.time = 1, .value = 1}), SR_OK);
    CHECK_INT(t, sr_track_finalize(&track), SR_OK);
    CHECK_NEAR(t, sr_track_eval(&track, 0, 1.1), 0.34029984660829826, 1e-12);
    track.seconds_per_unit = 1e308;
    CHECK_INT(t, sr_track_finalize(&track), SR_ERR_XML);
    sr_track_free(&track);

    SrAnimColor color = sr_anim_color_static((SrColor){0.25, 0.5, 0.75, 0.2});
    color.r.additive = color.g.additive = color.b.additive = color.a.additive = true;
    CHECK_INT(t, sr_anim_color_add_key(&color, (SrKeyframe){.curve = SR_CURVE_LINEAR},
                                     (SrColor){0.25, 0.25, 0.25, 0.3}), SR_OK);
    CHECK_INT(t, sr_anim_color_finalize(&color), SR_OK);
    SrColor result = sr_anim_color_eval(&color, 0);
    /* IEC 61966-2-1: decode before adding, then encode the channel sum. */
    CHECK_NEAR(t, result.r, 0.35212615552186044, 1e-12);
    CHECK_NEAR(t, result.g, 0.5515710949565031, 1e-12);
    CHECK_NEAR(t, result.b, 0.7817757501067063, 1e-12);
    CHECK_NEAR(t, result.a, 0.5, 1e-12);
    sr_anim_color_free(&color);
}

static void distant_short_cycles(sr_test_ctx *t) {
    SrTrack track = {.extrapolate_before = SR_EXTRAPOLATE_LOOP,
                     .extrapolate_after = SR_EXTRAPOLATE_LOOP};
    CHECK_INT(t, sr_track_add(&track, (SrKeyframe){.curve = SR_CURVE_LINEAR}), SR_OK);
    CHECK_INT(t, sr_track_add(&track, (SrKeyframe){
        .time = 3 * ldexp(1.0, -40), .value = 1}), SR_OK);
    CHECK_INT(t, sr_track_finalize(&track), SR_OK);
    CHECK_NEAR(t, sr_track_eval(&track, 0, 262144), 1.0 / 3.0, 1e-12);
    CHECK_NEAR(t, sr_track_eval(&track, 0, -262144), 2.0 / 3.0, 1e-12);
    CHECK_NEAR(t, sr_track_eval(&track, 0, 8192), 2.0 / 3.0, 1e-12);
    CHECK_NEAR(t, sr_track_eval(&track, 0, -8192), 1.0 / 3.0, 1e-12);
    track.extrapolate_before = track.extrapolate_after = SR_EXTRAPOLATE_PING_PONG;
    CHECK_NEAR(t, sr_track_eval(&track, 0, 262144), 2.0 / 3.0, 1e-12);
    CHECK_NEAR(t, sr_track_eval(&track, 0, -262144), 2.0 / 3.0, 1e-12);
    CHECK_NEAR(t, sr_track_eval(&track, 0, 8192), 2.0 / 3.0, 1e-12);
    CHECK_NEAR(t, sr_track_eval(&track, 0, -8192), 2.0 / 3.0, 1e-12);
    sr_track_free(&track);
}

static void decimal_cycle_boundaries(sr_test_ctx *t) {
    SrTrack track = {0};
    CHECK_INT(t, sr_track_add(&track, (SrKeyframe){.curve = SR_CURVE_LINEAR}), SR_OK);
    CHECK_INT(t, sr_track_add(&track, (SrKeyframe){.time = 0.1, .value = 1}), SR_OK);
    for (int i = SR_EXTRAPOLATE_LOOP; i <= SR_EXTRAPOLATE_OFFSET; ++i) {
        track.extrapolate_before = track.extrapolate_after = (SrExtrapolation)i;
        CHECK_INT(t, sr_track_finalize(&track), SR_OK);
        double positive = i == SR_EXTRAPOLATE_LOOP ? 0 : i == SR_EXTRAPOLATE_OFFSET ? 5 : 1;
        double negative = i == SR_EXTRAPOLATE_LOOP ? 0 : i == SR_EXTRAPOLATE_OFFSET ? -5 : 1;
        CHECK_NEAR(t, sr_track_eval(&track, 0, 0.5), positive, 1e-12);
        CHECK_NEAR(t, sr_track_eval(&track, 0, -0.5), negative, 1e-12);
        for (int magnitude = 10; magnitude <= 1000; magnitude *= 100) {
            double value = i == SR_EXTRAPOLATE_OFFSET ? magnitude * 10.0 : 0.0;
            CHECK_NEAR(t, sr_track_eval(&track, 0, magnitude), value, 1e-12);
            CHECK_NEAR(t, sr_track_eval(&track, 0, -magnitude), -value, 1e-12);
        }
        if (i == SR_EXTRAPOLATE_OFFSET) {
            CHECK_NEAR(t, sr_track_eval(&track, 0, 0.5 - 1e-9), 5 - 1e-8, 1e-12);
            CHECK_NEAR(t, sr_track_eval(&track, 0, 0.5 + 1e-9), 5 + 1e-8, 1e-12);
        }
    }
    sr_track_free(&track);
}

static SrStatus load_track(sr_test_ctx *t, const char *version, const char *options,
                            const char *keys, SrScene *scene, char **message) {
    char xml[8192];
    snprintf(xml, sizeof(xml), "<scene version=\"%s\">"
        "<project width=\"32\" height=\"32\" duration=\"2\" fps=\"12\"/>"
        "<composition><shape id=\"s\" shape=\"rect\" width=\"8\" height=\"8\" "
        "x=\"3\" start=\"0.5\" end=\"1.75\"><animate property=\"position.x\" %s>%s"
        "</animate></shape></composition></scene>", version, options, keys);
    return st_load(t, "curves-xml.xml", xml, scene, message);
}

static void xml_track_semantics(sr_test_ctx *t) {
    const char *keys = "<key time=\"0\" value=\"0\"/><key time=\"1\" value=\"10\"/>";
    SrScene scene;
    SrStatus status = load_track(t, "1.0", "additive=\"true\" timeBase=\"local\"",
                                  keys, &scene, NULL);
    CHECK_INT(t, status, SR_OK);
    if (status == SR_OK) {
        SrNode *node = sr_scene_find_node(&scene, "s");
        CHECK_NEAR(t, sr_anim_eval(&node->transform.x, 1), 8, 1e-12);
        sr_scene_free(&scene);
    }
    status = load_track(t, "1.1", "timeBase=\"normalized\" extrapolateAfter=\"offset\"",
                          keys, &scene, NULL);
    CHECK_INT(t, status, SR_OK);
    if (status == SR_OK) {
        SrNode *node = sr_scene_find_node(&scene, "s");
        CHECK_NEAR(t, sr_anim_eval(&node->transform.x, 0.5), 0, 1e-12);
        CHECK_NEAR(t, sr_anim_eval(&node->transform.x, 1), 4, 1e-12);
        CHECK_NEAR(t, sr_anim_eval(&node->transform.x, 2), 12, 1e-12);
        sr_scene_free(&scene);
    }
    keys = "<key time=\"0\" value=\"0\" easeOut=\"0.4,0.25\"/>"
           "<key time=\"1\" value=\"10\" easeIn=\"0.3,0.5\"/>";
    status = load_track(t, "1.1", "defaultInterpolation=\"cubic-bezier\"",
                          keys, &scene, NULL);
    CHECK_INT(t, status, SR_OK);
    if (status == SR_OK) {
        SrTrack *track = &sr_scene_find_node(&scene, "s")->transform.x.track;
        CHECK_NEAR(t, track->keys[0].x1, 0.4, 0);
        CHECK_NEAR(t, track->keys[0].y2, 0.85, 1e-12);
        sr_scene_free(&scene);
    }
    const char *one_sided[] = {
        "<key time=\"0\" value=\"0\" easeOut=\"0.4,0.25\"/>"
        "<key time=\"1\" value=\"10\"/>",
        "<key time=\"0\" value=\"0\"/>"
        "<key time=\"1\" value=\"10\" easeIn=\"0.3,0.5\"/>"
    };
    for (size_t i = 0; i < 2; ++i) {
        status = load_track(t, "1.1", "defaultInterpolation=\"cubic-bezier\"",
                            one_sided[i], &scene, NULL);
        CHECK_INT(t, status, SR_OK);
        if (status == SR_OK) {
            SrTrack *track = &sr_scene_find_node(&scene, "s")->transform.x.track;
            CHECK_NEAR(t, track->keys[0].x1, i ? 0.25 : 0.4, 1e-12);
            CHECK_NEAR(t, track->keys[0].y2, i ? 0.85 : 1, 1e-12);
            sr_scene_free(&scene);
        }
    }
}

static void xml_rejections(sr_test_ctx *t) {
    const struct { const char *key, *diagnostic; } cases[] = {
        {"interpolation=\"steps\"", "requires a step count"},
        {"interpolation=\"steps\" steps=\"1000001\"", "[1,1000000]"},
        {"steps=\"4\"", "valid only with steps"},
        {"stepPosition=\"start\"", "requires steps"},
        {"tension=\"0.2\"", "adjacent tcb segment"},
        {"stiffness=\"100\"", "require spring"},
        {"interpolation=\"spring\" mass=\"0.00000001\"", "[1e-6,1e6]"},
        {"interpolation=\"spring\" damping=\"1000001\"", "[0,1e6]"},
        {"interpolation=\"cubic-bezier\" easeOut=\"1.1,0.2\"", "[0,1]"},
        {"interpolation=\"cubic-bezier\" easeOut=\"0.2,0.2\" bezier=\"0,0,1,1\"",
         "without explicit bezier"},
        {"easeOut=\"0.3,0.2\"", "require cubic-bezier"},
        {"easeIn=\"0.3,0.2\"", "preceding segment"},
        {"interpolation=\"cubic-bezier\"", "bezier controls"},
    };
    for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); ++i) {
        char keys[1024], *message = NULL;
        snprintf(keys, sizeof(keys), "<key time=\"0\" value=\"0\" %s/>"
                 "<key time=\"1\" value=\"10\"/>", cases[i].key);
        SrScene scene;
        SrStatus status = load_track(t, "1.1", "", keys, &scene, &message);
        CHECK_INT(t, status, SR_ERR_XML);
        CHECK_CONTAINS(t, message, cases[i].diagnostic);
        CHECK_CONTAINS(t, message, "<key>");
        free(message);
        if (status == SR_OK) sr_scene_free(&scene);
    }
    char *message = NULL;
    SrScene scene;
    SrStatus status = load_track(t, "1.0", "defaultInterpolation=\"sine-in\"",
        "<key time=\"0\" value=\"0\"/>", &scene, &message);
    CHECK_INT(t, status, SR_ERR_XML);
    CHECK_CONTAINS(t, message, "requires version=\"1.1\"");
    free(message);
    if (status == SR_OK) sr_scene_free(&scene);
    message = NULL;
    status = load_track(t, "1.1", "", "<key time=\"0\" value=\"0\"/>"
        "<key time=\"1\" value=\"10\" easeOut=\"0.3,0.2\"/>", &scene, &message);
    CHECK_INT(t, status, SR_ERR_XML);
    CHECK_CONTAINS(t, message, "following segment");
    free(message);
    if (status == SR_OK) sr_scene_free(&scene);
    message = NULL;
    status = load_track(t, "1.1", "additive=\"true\"",
        "<key time=\"0\" value=\"0\" interpolation=\"cubic-bezier\" "
        "bezier=\"0.2,1000001,0.8,1\"/><key time=\"1\" value=\"10\"/>",
        &scene, &message);
    CHECK_INT(t, status, SR_ERR_XML);
    CHECK_CONTAINS(t, message, "<key>");
    CHECK_CONTAINS(t, message, "@bezier");
    free(message);
    if (status == SR_OK) sr_scene_free(&scene);
}

static void seeded_parser_mutations(sr_test_ctx *t) {
    const uint32_t seeds[] = {7, 4711, 0x51a8a234};
    for (size_t seed = 0; seed < 3; ++seed) {
        uint32_t state = seeds[seed];
        for (int trial = 0; trial < 48; ++trial) {
            char keys[1024];
            state = state * UINT32_C(1664525) + UINT32_C(1013904223);
            SrCurve curve = (SrCurve)(SR_CURVE_SINE_IN + state % 30);
            snprintf(keys, sizeof(keys), "<key time=\"0\" value=\"0\" interpolation=\"%s\"/>"
                "<key time=\"1\" value=\"%u\"/>", sr_curve_name(curve), state % 100);
            bool valid = trial % 3 == 0;
            if (!valid) {
                size_t length = strlen(keys);
                if (trial % 3 == 1) keys[length - 1] = '\0';
                else keys[state % length] = '<';
            }
            SrScene scene;
            SrStatus status = load_track(t, "1.1", "", keys, &scene, NULL);
            CHECK(t, status == SR_OK || status == SR_ERR_XML);
            if (valid) CHECK_INT(t, status, SR_OK);
            if (status == SR_OK) sr_scene_free(&scene);
        }
    }
}

static void particle_extended_tracks(sr_test_ctx *t) {
    const char *xml = "<scene version=\"1.1\"><project width=\"32\" height=\"32\" "
        "fps=\"24\" duration=\"3\"/><composition>"
        "<particleEmitter id=\"p\" rate=\"2\" lifetime=\"100\" speed=\"0\">"
        "<animate property=\"rate\" additive=\"true\" extrapolateAfter=\"loop\">"
        "<key time=\"0\" value=\"0\"/><key time=\"0.5\" value=\"4\"/>"
        "</animate></particleEmitter>"
        "<particleEmitter id=\"q\" rate=\"10\" speed=\"0\">"
        "<animate property=\"lifetime\"><key time=\"0\" value=\"0.4\" "
        "interpolation=\"spring\" stiffness=\"100\" damping=\"0\" mass=\"1\"/>"
        "<key time=\"1\" value=\"1.4\"/></animate>"
        "</particleEmitter></composition></scene>";
    SrScene scene;
    if (st_load(t, "curves-particles.xml", xml, &scene, NULL) != SR_OK) {
        SR_FAIL(t, "particle fixture failed to load");
        return;
    }
    SrNode *p = sr_scene_find_node(&scene, "p");
    SrParticle *first = NULL, *again = NULL, *warm = NULL;
    size_t count = 0, repeated = 0, warm_count = 0;
    CHECK_INT(t, sr_particles_eval(&scene, p, 1.4, &first, &count), SR_OK);
    CHECK_INT(t, count, 6); /* Integral of 2 + repeating 0..4 ramp is 5.44. */
    CHECK_INT(t, sr_particles_eval(&scene, p, 2.4, &warm, &warm_count), SR_OK);
    CHECK_INT(t, sr_particles_eval(&scene, p, 1.4, &again, &repeated), SR_OK);
    CHECK_INT(t, count, repeated);
    for (size_t i = 0; i < count && i < repeated; ++i) {
        CHECK_INT(t, first[i].index, again[i].index);
        CHECK_NEAR(t, first[i].x, again[i].x, 0);
        CHECK_NEAR(t, first[i].y, again[i].y, 0);
    }
    free(first);
    free(again);
    free(warm);
    SrNode *q = sr_scene_find_node(&scene, "q");
    first = NULL;
    CHECK_INT(t, sr_particles_eval(&scene, q, 2.2, &first, &count), SR_OK);
    bool found = false;
    for (size_t i = 0; i < count; ++i) if (first[i].index == 3) found = true;
    /* At birth .3, lifetime .4 + (1-cos(3)) is >2.38; age is only 1.9. */
    CHECK(t, found);
    free(first);
    sr_scene_free(&scene);
}

static void mutate_cache_track(SrTrack *track, int change) {
    switch (change) {
    case 0: track->keys[0].stiffness = 200; break;
    case 1: track->keys[0].damping = 20; break;
    case 2: track->keys[0].mass = 2; break;
    case 3: track->keys[0].steps = 5; break;
    case 4: track->keys[0].step_start = true; break;
    case 5: track->keys[0].tension = 0.5; break;
    case 6: track->keys[0].continuity = 0.5; break;
    case 7: track->keys[0].bias = 0.5; break;
    case 8: track->additive = true; break;
    case 9: track->extrapolate_before = SR_EXTRAPOLATE_LINEAR; break;
    case 10: track->extrapolate_after = SR_EXTRAPOLATE_OFFSET; break;
    case 11: track->clock_scale = 2; break;
    case 12: track->clock_offset = 0.05; break;
    case 13: track->seconds_per_unit = 0.5; break;
    }
}

static void particle_index_limit(sr_test_ctx *t) {
    const char *xml = "<scene version=\"1.1\"><project width=\"32\" height=\"32\" "
        "fps=\"24\" duration=\"2\"/><composition>"
        "<particleEmitter id=\"p\" lifetime=\"100\" speed=\"0\">"
        "<animate property=\"rate\" defaultInterpolation=\"catmull-rom\">"
        "<key time=\"0\" value=\"0\"/><key time=\"1e-12\" value=\"1e12\"/>"
        "<key time=\"1\" value=\"1e12\"/></animate>"
        "</particleEmitter></composition></scene>";
    SrScene scene;
    if (st_load(t, "curves-particle-limit.xml", xml, &scene, NULL) != SR_OK) {
        SR_FAIL(t, "particle limit fixture failed to load");
        return;
    }
    SrNode *node = sr_scene_find_node(&scene, "p");
    node->particle_max = 1;
    SrParticle *items = NULL;
    size_t count = 0;
    CHECK_INT(t, sr_particles_eval(&scene, node, 0.5, &items, &count), SR_ERR_RENDER);
    CHECK_INT(t, count, 0);
    free(items);
    sr_scene_free(&scene);
}

static void physics_animation_fingerprint(sr_test_ctx *t) {
    for (int change = 0; change < 14; ++change) {
        const char *curve = change < 3 || change == 13 ? "spring"
            : change < 5 ? "steps" : change < 8 ? "tcb" : "linear";
        const char *extra = change == 3 || change == 4 ? " steps=\"4\"" : "";
        char xml[2048], cache[1024];
        snprintf(cache, sizeof(cache), "%s", sr_test_tmp_path("animation.physics"));
        unlink(cache);
        snprintf(xml, sizeof(xml), "<scene version=\"1.1\">"
            "<project width=\"32\" height=\"32\" fps=\"10\" duration=\"0.4\"/>"
            "<composition><shape id=\"b\" shape=\"rect\" width=\"4\" height=\"4\">"
            "<rigidBody type=\"dynamic\"/></shape></composition>"
            "<physics fixedStep=\"0.01\" gravityY=\"0\" cache=\"animation.physics\">"
            "<forceField id=\"f\" type=\"directional\" forceX=\"2\">"
            "<animate property=\"forceX\" timeBase=\"%s\">"
            "<key time=\"0.1\" value=\"2\" interpolation=\"%s\"%s/>"
            "<key time=\"0.3\" value=\"10\"/></animate></forceField>"
            "</physics></scene>", change == 13 ? "normalized" : "local", curve, extra);
        SrScene initial, changed, uncached;
        if (st_load(t, "animation-cache.xml", xml, &initial, NULL) != SR_OK) {
            SR_FAIL(t, "physics animation fixture failed to load");
            return;
        }
        FILE *sink;
        SrDiagnostics diag;
        st_diag(&diag, &sink);
        CHECK_INT(t, sr_physics_prepare(&initial, &diag), SR_OK);
        sr_scene_free(&initial);
        SrStatus changed_status = st_load(t, "animation-cache.xml", xml, &changed, NULL);
        SrStatus uncached_status = st_load(t, "animation-cache.xml", xml, &uncached, NULL);
        if (changed_status != SR_OK || uncached_status != SR_OK) {
            SR_FAIL(t, "physics reload failed");
            if (changed_status == SR_OK) sr_scene_free(&changed);
            if (uncached_status == SR_OK) sr_scene_free(&uncached);
            if (sink) fclose(sink);
            return;
        }
        mutate_cache_track(&changed.physics.fields[0].force_x.track, change);
        mutate_cache_track(&uncached.physics.fields[0].force_x.track, change);
        free(uncached.physics.cache_path);
        uncached.physics.cache_path = NULL;
        CHECK_INT(t, sr_physics_prepare(&changed, &diag), SR_OK);
        CHECK(t, !changed.physics.cache_hit);
        CHECK_INT(t, sr_physics_prepare(&uncached, &diag), SR_OK);
        SrNode *a = sr_scene_find_node(&changed, "b");
        SrNode *b = sr_scene_find_node(&uncached, "b");
        CHECK_INT(t, a->physics_sample_count, b->physics_sample_count);
        for (size_t i = 0; i < a->physics_sample_count && i < b->physics_sample_count; ++i)
            CHECK_NEAR(t, a->physics_samples[i].x.base, b->physics_samples[i].x.base, 0);
        sr_scene_free(&changed);
        sr_scene_free(&uncached);
        if (sink) fclose(sink);
        unlink(cache);
    }
}

const sr_test_case sr_tests_curves[] = {
    {"penner_reference_values", penner_reference_values},
    {"step_boundaries", step_boundaries},
    {"hermite_neighbors", hermite_neighbors},
    {"spring_regimes", spring_regimes},
    {"extrapolation_and_clocks", extrapolation_and_clocks},
    {"handles_and_bounds", handles_and_bounds},
    {"finalized_configuration_limits", finalized_configuration_limits},
    {"normalized_spring_and_additive_color", normalized_spring_and_additive_color},
    {"distant_short_cycles", distant_short_cycles},
    {"decimal_cycle_boundaries", decimal_cycle_boundaries},
    {"xml_track_semantics", xml_track_semantics},
    {"xml_rejections", xml_rejections},
    {"seeded_parser_mutations", seeded_parser_mutations},
    {"particle_extended_tracks", particle_extended_tracks},
    {"particle_index_limit", particle_index_limit},
    {"physics_animation_fingerprint", physics_animation_fingerprint},
    {NULL, NULL},
};
