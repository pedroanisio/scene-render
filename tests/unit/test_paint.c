/* SPDX-License-Identifier: Apache-2.0 */
/* B1-4 gradient paints against independent formulas: linear projection,
 * two-point conical radial (solved independently by bisection), conic
 * angles, spread, stop clamping, CSS colour hints, Oklab reference values,
 * oklch hue rules and the seeded dither. */
#include "scene_render/paint.h"
#include "scene_render/color.h"

#include <stdlib.h>

#include "harness.h"

static SrProject project(void) {
    return (SrProject){.width = 64, .height = 64, .fps_num = 30, .fps_den = 1,
                       .duration = 1.0, .working_color_space = SR_COLOR_SRGB};
}

static void add_stop(SrPaint *paint, double offset, SrColor color) {
    if (paint->stop_count == paint->stop_capacity) {
        paint->stop_capacity = paint->stop_capacity ? paint->stop_capacity * 2 : 4;
        paint->stops = realloc(paint->stops, paint->stop_capacity * sizeof(*paint->stops));
    }
    SrGradientStop *stop = &paint->stops[paint->stop_count++];
    *stop = (SrGradientStop){0};
    stop->offset.base = offset;
    stop->color = sr_anim_color_static(color);
    stop->opacity.base = 1.0;
    stop->midpoint.base = 0.5;
}

static SrPaint gray_ramp(SrPaintType type) {
    SrPaint paint;
    sr_paint_init(&paint, type);
    paint.dither = false;
    paint.space = SR_INTERP_SRGB;
    add_stop(&paint, 0.0, (SrColor){0, 0, 0, 1});
    add_stop(&paint, 1.0, (SrColor){1, 1, 1, 1});
    return paint;
}

static double gray(const SrPaintEval *eval, double x, double y) {
    float out[4];
    sr_paint_sample(eval, x, y, 0, 0, out);
    return out[0];
}

static void linear_projection_units_rotation(sr_test_ctx *t) {
    SrProject p = project();
    SrPaint paint = gray_ramp(SR_PAINT_LINEAR);
    SrPaintEval eval;
    CHECK_INT(t, sr_paint_eval(&paint, &p, 0.0, 200, 100, &eval, NULL), SR_OK);
    double v;
    CHECK(t, sr_paint_parameter(&eval, 50, 30, &v));
    CHECK_NEAR(t, v, 0.25, 1e-15);
    CHECK_NEAR(t, gray(&eval, 50, 30), 0.25, 1e-6);
    /* User units: pixels in the painted node's space. */
    paint.units = SR_PAINT_UNITS_USER;
    paint.x1.base = 10;
    paint.x2.base = 30;
    CHECK_INT(t, sr_paint_eval(&paint, &p, 0.0, 200, 100, &eval, NULL), SR_OK);
    CHECK(t, sr_paint_parameter(&eval, 15, 99, &v));
    CHECK_NEAR(t, v, 0.25, 1e-15);
    /* Rotation 90 degrees clockwise about the box centre (100, 50): the
     * ramp now increases downward. */
    paint.units = SR_PAINT_UNITS_OBJECT;
    paint.x1.base = 0;
    paint.x2.base = 1;
    paint.rotation.base = 90;
    CHECK_INT(t, sr_paint_eval(&paint, &p, 0.0, 100, 100, &eval, NULL), SR_OK);
    double top, bottom;
    CHECK(t, sr_paint_parameter(&eval, 50, 10, &top));
    CHECK(t, sr_paint_parameter(&eval, 50, 90, &bottom));
    CHECK_NEAR(t, top, 0.1, 1e-12);
    CHECK_NEAR(t, bottom, 0.9, 1e-12);
    /* Coincident points paint the last stop, whatever the spread. */
    paint.rotation.base = 0;
    paint.x2.base = 0;
    paint.spread = SR_SPREAD_REPEAT;
    CHECK_INT(t, sr_paint_eval(&paint, &p, 0.0, 100, 100, &eval, NULL), SR_OK);
    CHECK_NEAR(t, gray(&eval, 3, 3), 1.0, 0.0);
    sr_paint_free(&paint);
}

/* Largest w with fr + w dr >= 0 and |q - f - w (c - f)| = fr + w dr, by
 * scanning then bisection (independent of the closed form). */
static bool conical_reference(double qx, double qy, double fx, double fy,
                              double fr, double cx, double cy, double r,
                              double *out) {
    double best = NAN;
    double previous = NAN, pw = 0.0;
    for (int i = 0; i <= 200000; ++i) {
        double w = 20.0 - i * 0.0002;
        double radius = fr + w * (r - fr);
        if (radius < 0.0) { previous = NAN; continue; }
        double g = hypot(qx - fx - w * (cx - fx), qy - fy - w * (cy - fy)) - radius;
        if (!isnan(previous) && ((g <= 0.0) != (previous <= 0.0))) {
            double lo = w, hi = pw;
            for (int k = 0; k < 80; ++k) {
                double mid = 0.5 * (lo + hi);
                double gm = hypot(qx - fx - mid * (cx - fx), qy - fy - mid * (cy - fy)) -
                            (fr + mid * (r - fr));
                if ((gm <= 0.0) == (g <= 0.0)) lo = mid; else hi = mid;
            }
            best = 0.5 * (lo + hi);
            break;
        }
        previous = g;
        pw = w;
    }
    if (isnan(best)) return false;
    *out = best;
    return true;
}

static void radial_focal_and_aspect(sr_test_ctx *t) {
    SrProject p = project();
    SrPaint paint = gray_ramp(SR_PAINT_RADIAL);
    paint.units = SR_PAINT_UNITS_USER;
    paint.cx.base = 50;
    paint.cy.base = 50;
    paint.r.base = 40;
    SrPaintEval eval;
    CHECK_INT(t, sr_paint_eval(&paint, &p, 0.0, 100, 100, &eval, NULL), SR_OK);
    double v;
    CHECK(t, sr_paint_parameter(&eval, 70, 50, &v));
    CHECK_NEAR(t, v, 0.5, 1e-12);
    /* Focal point and focal radius against the independent solver. */
    paint.fx.base = 35;
    paint.fy.base = 45;
    paint.fr.base = 5;
    paint.fx_set = paint.fy_set = true;
    CHECK_INT(t, sr_paint_eval(&paint, &p, 0.0, 100, 100, &eval, NULL), SR_OK);
    const double points[][2] = {{70, 50}, {20, 20}, {50, 88}, {36, 45}, {80, 80}};
    for (size_t i = 0; i < sizeof(points) / sizeof(points[0]); ++i) {
        double expected = NAN;
        CHECK(t, conical_reference(points[i][0], points[i][1], 35, 45, 5, 50, 50, 40,
                                   &expected));
        CHECK(t, sr_paint_parameter(&eval, points[i][0], points[i][1], &v));
        CHECK_NEAR(t, v, expected, 1e-6);
    }
    /* A focal circle outside the end circle makes a cone: points behind it
     * are not painted and sample as transparent. */
    paint.fx.base = 5;
    paint.fy.base = 50;
    paint.fr.base = 0;
    paint.r.base = 10;
    CHECK_INT(t, sr_paint_eval(&paint, &p, 0.0, 100, 100, &eval, NULL), SR_OK);
    CHECK(t, !sr_paint_parameter(&eval, 5, 90, &v));
    float out[4];
    sr_paint_sample(&eval, 5, 90, 0, 0, out);
    CHECK(t, out[3] == 0.0f);
    /* Aspect 2 halves the vertical radius. */
    paint.fx_set = paint.fy_set = false;
    paint.fr.base = 0;
    paint.r.base = 40;
    paint.aspect.base = 2.0;
    CHECK_INT(t, sr_paint_eval(&paint, &p, 0.0, 100, 100, &eval, NULL), SR_OK);
    CHECK(t, sr_paint_parameter(&eval, 50, 70, &v));
    CHECK_NEAR(t, v, 1.0, 1e-12);
    /* Identical circles paint nothing (HTML canvas). */
    paint.aspect.base = 1.0;
    paint.fr.base = 40;
    CHECK_INT(t, sr_paint_eval(&paint, &p, 0.0, 100, 100, &eval, NULL), SR_OK);
    CHECK(t, !sr_paint_parameter(&eval, 50, 50, &v));
    sr_paint_free(&paint);
}

static void conic_and_spread(sr_test_ctx *t) {
    SrProject p = project();
    SrPaint paint = gray_ramp(SR_PAINT_CONIC);
    SrPaintEval eval;
    CHECK_INT(t, sr_paint_eval(&paint, &p, 0.0, 100, 100, &eval, NULL), SR_OK);
    double v;
    CHECK(t, sr_paint_parameter(&eval, 50, 10, &v));
    CHECK_NEAR(t, v, 0.0, 1e-12);
    CHECK(t, sr_paint_parameter(&eval, 90, 50, &v));
    CHECK_NEAR(t, v, 0.25, 1e-12);
    CHECK(t, sr_paint_parameter(&eval, 50, 90, &v));
    CHECK_NEAR(t, v, 0.5, 1e-12);
    CHECK(t, sr_paint_parameter(&eval, 10, 50, &v));
    CHECK_NEAR(t, v, 0.75, 1e-12);
    paint.angle.base = 90;
    CHECK_INT(t, sr_paint_eval(&paint, &p, 0.0, 100, 100, &eval, NULL), SR_OK);
    CHECK(t, sr_paint_parameter(&eval, 90, 50, &v));
    CHECK_NEAR(t, v, 0.0, 1e-12);
    sr_paint_free(&paint);
    /* Spread on a linear ramp at t = 1.25 and -0.25. */
    SrPaint ramp = gray_ramp(SR_PAINT_LINEAR);
    ramp.units = SR_PAINT_UNITS_USER;
    ramp.x2.base = 100;
    const SrSpread modes[3] = {SR_SPREAD_PAD, SR_SPREAD_REPEAT, SR_SPREAD_REFLECT};
    const double after[3] = {1.0, 0.25, 0.75}, before[3] = {0.0, 0.75, 0.25};
    for (int i = 0; i < 3; ++i) {
        ramp.spread = modes[i];
        CHECK_INT(t, sr_paint_eval(&ramp, &p, 0.0, 100, 100, &eval, NULL), SR_OK);
        CHECK_NEAR(t, gray(&eval, 125, 0), after[i], 1e-6);
        CHECK_NEAR(t, gray(&eval, -25, 0), before[i], 1e-6);
    }
    sr_paint_free(&ramp);
}

static void stops_hints_and_animation(sr_test_ctx *t) {
    SrProject p = project();
    SrPaint paint = gray_ramp(SR_PAINT_LINEAR);
    paint.units = SR_PAINT_UNITS_USER;
    paint.x2.base = 100;
    SrPaintEval eval;
    /* Colour hint H = 0.25: w = P^(ln .5 / ln H), so P = 0.25 gives 0.5. */
    paint.stops[0].midpoint.base = 0.25;
    CHECK_INT(t, sr_paint_eval(&paint, &p, 0.0, 100, 100, &eval, NULL), SR_OK);
    CHECK_NEAR(t, gray(&eval, 25, 0), 0.5, 1e-6);
    CHECK_NEAR(t, gray(&eval, 50, 0), pow(0.5, log(0.5) / log(0.25)), 1e-6);
    paint.stops[0].midpoint.base = 0.0;
    CHECK_INT(t, sr_paint_eval(&paint, &p, 0.0, 100, 100, &eval, NULL), SR_OK);
    CHECK_NEAR(t, gray(&eval, 1, 0), 1.0, 0.0);
    paint.stops[0].midpoint.base = 1.0;
    CHECK_INT(t, sr_paint_eval(&paint, &p, 0.0, 100, 100, &eval, NULL), SR_OK);
    CHECK_NEAR(t, gray(&eval, 99, 0), 0.0, 0.0);
    paint.stops[0].midpoint.base = 0.5;
    /* Out-of-order offsets clamp to the previous stop: a hard edge at 0.6. */
    paint.stops[0].offset.base = 0.6;
    paint.stops[1].offset.base = 0.2;
    add_stop(&paint, 1.0, (SrColor){0, 0, 0, 1});
    CHECK_INT(t, sr_paint_eval(&paint, &p, 0.0, 100, 100, &eval, NULL), SR_OK);
    CHECK_NEAR(t, eval.offsets[1], 0.6, 0.0);
    CHECK_NEAR(t, gray(&eval, 59, 0), 0.0, 0.0);
    CHECK_NEAR(t, gray(&eval, 60, 0), 1.0, 1e-6);   /* the later stop wins */
    CHECK_NEAR(t, gray(&eval, 80, 0), 0.5, 1e-6);
    /* Animated offset: a linear key track moves the stop. */
    SrKeyframe k0 = {.time = 0.0, .value = 0.0, .curve = SR_CURVE_LINEAR};
    SrKeyframe k1 = {.time = 1.0, .value = 1.0, .curve = SR_CURVE_LINEAR};
    CHECK_INT(t, sr_track_add(&paint.stops[0].opacity.track, k0), SR_OK);
    CHECK_INT(t, sr_track_add(&paint.stops[0].opacity.track, k1), SR_OK);
    CHECK_INT(t, sr_track_finalize(&paint.stops[0].opacity.track), SR_OK);
    CHECK(t, sr_paint_has_animation(&paint));
    CHECK_INT(t, sr_paint_eval(&paint, &p, 0.5, 100, 100, &eval, NULL), SR_OK);
    CHECK_NEAR(t, eval.alpha[0], 0.5, 1e-12);
    /* Opacity multiplies alpha; premultiplied interpolation. */
    float out[4];
    sr_paint_sample(&eval, 10, 0, 0, 0, out);
    CHECK_NEAR(t, out[3], 0.5, 1e-6);
    sr_paint_free(&paint);
    /* Bounds: a nonfinite coordinate fails evaluation with a diagnostic. */
    SrPaint bad = gray_ramp(SR_PAINT_LINEAR);
    bad.x1.base = 2e9;
    SrDiagnostics diag;
    FILE *sink = tmpfile();
    sr_diag_init(&diag, "paint", sink);
    CHECK_INT(t, sr_paint_eval(&bad, &p, 0.0, 100, 100, &eval, &diag), SR_ERR_RENDER);
    CHECK_INT(t, diag.errors, 1);
    if (sink) fclose(sink);
    sr_paint_free(&bad);
}

static void oklab_references(sr_test_ctx *t) {
    /* Values from Ottosson's post and the CSS Color 4 sample code. */
    const double white[3] = {1, 1, 1}, red[3] = {1, 0, 0}, blue[3] = {0, 0, 1};
    double lab[3], rgb[3];
    sr_oklab_from_linear_srgb(white, lab);
    CHECK_NEAR(t, lab[0], 1.0, 1e-4);
    CHECK_NEAR(t, lab[1], 0.0, 1e-4);
    CHECK_NEAR(t, lab[2], 0.0, 1e-4);
    sr_oklab_from_linear_srgb(red, lab);
    CHECK_NEAR(t, lab[0], 0.627955, 1e-5);
    CHECK_NEAR(t, lab[1], 0.224863, 1e-5);
    CHECK_NEAR(t, lab[2], 0.125846, 1e-5);
    sr_oklab_from_linear_srgb(blue, lab);
    CHECK_NEAR(t, lab[0], 0.452014, 1e-5);
    CHECK_NEAR(t, lab[1], -0.032457, 1e-5);
    CHECK_NEAR(t, lab[2], -0.311528, 1e-5);
    sr_linear_srgb_from_oklab(lab, rgb);
    CHECK_NEAR(t, rgb[0], 0.0, 1e-6);
    CHECK_NEAR(t, rgb[2], 1.0, 1e-6);
    /* Interpolation spaces at the midpoint of black -> white. */
    SrProject p = project();
    SrPaint paint = gray_ramp(SR_PAINT_LINEAR);
    paint.units = SR_PAINT_UNITS_USER;
    paint.x2.base = 100;
    SrPaintEval eval;
    paint.space = SR_INTERP_LINEAR;
    CHECK_INT(t, sr_paint_eval(&paint, &p, 0.0, 100, 100, &eval, NULL), SR_OK);
    CHECK_NEAR(t, gray(&eval, 50, 0), sr_color_encode(0.5, SR_COLOR_SRGB), 1e-6);
    paint.space = SR_INTERP_OKLAB;
    CHECK_INT(t, sr_paint_eval(&paint, &p, 0.0, 100, 100, &eval, NULL), SR_OK);
    /* L = 0.5 on the gray axis: linear 0.125 = 0.5^3. */
    CHECK_NEAR(t, gray(&eval, 50, 0), sr_color_encode(0.125, SR_COLOR_SRGB), 1e-4);
    sr_paint_free(&paint);
}

static double hue_of(const float px[4]) {
    double linear[3], lab[3];
    for (int c = 0; c < 3; ++c) linear[c] = sr_color_decode(px[c] / px[3], SR_COLOR_SRGB);
    sr_oklab_from_linear_srgb(linear, lab);
    double h = atan2(lab[2], lab[1]) * 180.0 / SR_PI;
    return h < 0 ? h + 360.0 : h;
}

static void oklch_hue_rules(sr_test_ctx *t) {
    SrProject p = project();
    SrPaint paint;
    sr_paint_init(&paint, SR_PAINT_LINEAR);
    paint.units = SR_PAINT_UNITS_USER;
    paint.x2.base = 100;
    paint.dither = false;
    paint.space = SR_INTERP_OKLCH;
    /* Red (h ~ 29) to blue (h ~ 264): the shorter arc passes through 327
     * (magenta), not through green. */
    add_stop(&paint, 0.0, (SrColor){1, 0, 0, 1});
    add_stop(&paint, 1.0, (SrColor){0, 0, 1, 1});
    SrPaintEval eval;
    CHECK_INT(t, sr_paint_eval(&paint, &p, 0.0, 100, 100, &eval, NULL), SR_OK);
    double h1 = eval.values[0][2], h2 = eval.values[1][2];
    double expected = h1 + 360.0 + 0.5 * (h2 - (h1 + 360.0));
    float out[4];
    sr_paint_sample(&eval, 50, 0, 0, 0, out);
    CHECK_NEAR(t, hue_of(out), expected, 1.5);   /* after gamut clipping */
    /* White is achromatic: its hue is powerless and takes red's. */
    paint.stops[1].color = sr_anim_color_static((SrColor){1, 1, 1, 1});
    CHECK_INT(t, sr_paint_eval(&paint, &p, 0.0, 100, 100, &eval, NULL), SR_OK);
    CHECK(t, eval.powerless[1] && !eval.powerless[0]);
    sr_paint_sample(&eval, 50, 0, 0, 0, out);
    CHECK_NEAR(t, hue_of(out), eval.values[0][2], 1.0);
    sr_paint_free(&paint);
}

static void seeded_ordered_dither(sr_test_ctx *t) {
    SrProject p = project();
    SrPaint paint = gray_ramp(SR_PAINT_LINEAR);
    paint.id = strdup("ramp");
    paint.dither = true;
    paint.dither_key = sr_paint_dither_key(paint.id, 7);
    CHECK(t, paint.dither_key == sr_paint_dither_key("ramp", 7));
    CHECK(t, paint.dither_key != sr_paint_dither_key("ramp", 8));
    CHECK(t, paint.dither_key != sr_paint_dither_key("other", 7));
    SrPaintEval eval;
    CHECK_INT(t, sr_paint_eval(&paint, &p, 0.0, 100, 100, &eval, NULL), SR_OK);
    double total = 0.0;
    for (int y = 0; y < 8; ++y)
        for (int x = 0; x < 8; ++x) {
            float out[4];
            sr_paint_sample(&eval, 50, 0, x, y, out);
            CHECK(t, fabs(out[0] - 0.5) <= 0.5 / 255.0 + 1e-7);
            total += out[0] - 0.5;
            float again[4];
            sr_paint_sample(&eval, 50, 0, x + 8, y + 16, again);
            CHECK(t, again[0] == out[0]);          /* 8-periodic */
        }
    CHECK_NEAR(t, total / 64.0, 0.0, 1e-7);         /* zero-mean thresholds */
    sr_paint_free(&paint);
}

/* Review regressions: translucent Oklab stops keep their colour; a tiny
 * but nonzero linear vector still paints. */
static void translucent_oklab_and_tiny_vectors(sr_test_ctx *t) {
    SrProject p = project();
    SrPaint paint;
    sr_paint_init(&paint, SR_PAINT_LINEAR);
    paint.dither = false;
    paint.space = SR_INTERP_OKLAB;
    add_stop(&paint, 0.0, (SrColor){0, 0, 1, 0.5});
    SrPaintEval eval;
    CHECK_INT(t, sr_paint_eval(&paint, &p, 0.0, 10, 10, &eval, NULL), SR_OK);
    float out[4];
    sr_paint_sample(&eval, 5, 5, 0, 0, out);
    CHECK_NEAR(t, out[0], 0.0, 1e-4);
    CHECK_NEAR(t, out[1], 0.0, 1e-4);
    CHECK_NEAR(t, out[2], 0.5, 1e-4);
    CHECK_NEAR(t, out[3], 0.5, 1e-6);
    sr_paint_free(&paint);
    sr_paint_init(&paint, SR_PAINT_LINEAR);
    paint.dither = false;
    paint.x2.base = 1e-160;
    add_stop(&paint, 0.0, (SrColor){1, 1, 1, 1});
    CHECK_INT(t, sr_paint_eval(&paint, &p, 0.0, 10, 10, &eval, NULL), SR_OK);
    sr_paint_sample(&eval, 5, 5, 0, 0, out);
    CHECK_NEAR(t, out[3], 1.0, 0.0);
    CHECK_NEAR(t, out[0], 1.0, 0.0);
    sr_paint_free(&paint);
    /* Repeat boundary: t is exactly 1 at (0.5, 1.5) on (0,0) -> (1,1). */
    SrPaint ramp = gray_ramp(SR_PAINT_LINEAR);
    ramp.units = SR_PAINT_UNITS_USER;
    ramp.x2.base = 1.0;
    ramp.y2.base = 1.0;
    ramp.spread = SR_SPREAD_REPEAT;
    CHECK_INT(t, sr_paint_eval(&ramp, &p, 0.0, 10, 10, &eval, NULL), SR_OK);
    double v;
    CHECK(t, sr_paint_parameter(&eval, 0.5, 1.5, &v));
    CHECK(t, v == 1.0);
    CHECK_NEAR(t, gray(&eval, 0.5, 1.5), 0.0, 0.0);
    sr_paint_free(&ramp);
}

static void background_fill(sr_test_ctx *t) {
    SrProject p = project();
    SrPaint paint = gray_ramp(SR_PAINT_LINEAR);
    SrFrame frame;
    CHECK_INT(t, sr_frame_init(&frame, 16, 4), SR_OK);
    CHECK_INT(t, sr_paint_fill_frame(&paint, &p, 0.0, &frame, 3, NULL), SR_OK);
    CHECK_NEAR(t, frame.px[0], 0.5 / 16.0, 1e-6);
    CHECK_NEAR(t, frame.px[(15) * 4], 15.5 / 16.0, 1e-6);
    CHECK_NEAR(t, frame.px[3], 1.0, 0.0);
    SrFrame single;
    CHECK_INT(t, sr_frame_init(&single, 16, 4), SR_OK);
    CHECK_INT(t, sr_paint_fill_frame(&paint, &p, 0.0, &single, 1, NULL), SR_OK);
    CHECK(t, !memcmp(frame.px, single.px, 16 * 4 * 4 * sizeof(float)));
    sr_frame_free(&frame);
    sr_frame_free(&single);
    sr_paint_free(&paint);
}

const sr_test_case sr_tests_paint[] = {
    {"linear_projection_units_rotation", linear_projection_units_rotation},
    {"radial_focal_and_aspect", radial_focal_and_aspect},
    {"conic_and_spread", conic_and_spread},
    {"stops_hints_and_animation", stops_hints_and_animation},
    {"oklab_references", oklab_references},
    {"oklch_hue_rules", oklch_hue_rules},
    {"seeded_ordered_dither", seeded_ordered_dither},
    {"translucent_oklab_and_tiny_vectors", translucent_oklab_and_tiny_vectors},
    {"background_fill", background_fill},
    {NULL, NULL},
};
