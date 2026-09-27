/* SPDX-License-Identifier: Apache-2.0 */
/* B1-4 gradient paints. Definitions, references and limits are in
 * docs/design/b1-4-shapes-paints.md ("Paints"). Evaluation is a pure
 * function of the paint, the project and the time; the evaluated form is
 * immutable and read by row workers. */
#include "scene_render/paint.h"
#include "scene_render/color.h"
#include "scene_render/parallel.h"
#include "scene_render/random.h"
#include "vector_path_internal.h"

#include <float.h>
#include <math.h>
#include <stdlib.h>
#include <string.h>

#define SR_OKLCH_POWERLESS 1e-6

void sr_paint_init(SrPaint *paint, SrPaintType type) {
    *paint = (SrPaint){.type = type, .dither = true};
    paint->x2.base = 1.0;
    paint->cx.base = paint->cy.base = paint->r.base = 0.5;
    paint->aspect.base = 1.0;
}

static bool animated(const SrAnimValue *value) {
    return value->track.count != 0;
}

bool sr_paint_has_animation(const SrPaint *paint) {
    if (!paint) return false;
    const SrAnimValue *values[] = {&paint->rotation, &paint->x1, &paint->y1,
        &paint->x2, &paint->y2, &paint->cx, &paint->cy, &paint->r, &paint->fx,
        &paint->fy, &paint->fr, &paint->aspect, &paint->angle};
    for (size_t i = 0; i < sizeof(values) / sizeof(values[0]); ++i)
        if (animated(values[i])) return true;
    for (size_t i = 0; i < paint->stop_count; ++i) {
        const SrGradientStop *stop = &paint->stops[i];
        if (animated(&stop->offset) || animated(&stop->opacity) ||
            animated(&stop->midpoint) || stop->color.r.count)
            return true;
    }
    return false;
}

void sr_paint_free(SrPaint *paint) {
    if (!paint) return;
    free(paint->id);
    SrAnimValue *values[] = {&paint->rotation, &paint->x1, &paint->y1,
        &paint->x2, &paint->y2, &paint->cx, &paint->cy, &paint->r, &paint->fx,
        &paint->fy, &paint->fr, &paint->aspect, &paint->angle};
    for (size_t i = 0; i < sizeof(values) / sizeof(values[0]); ++i)
        sr_track_free(&values[i]->track);
    for (size_t i = 0; i < paint->stop_count; ++i) {
        SrGradientStop *stop = &paint->stops[i];
        sr_track_free(&stop->offset.track);
        sr_track_free(&stop->opacity.track);
        sr_track_free(&stop->midpoint.track);
        sr_anim_color_free(&stop->color);
    }
    free(paint->stops);
    *paint = (SrPaint){0};
}

uint64_t sr_paint_dither_key(const char *id, uint64_t project_seed) {
    static const char domain[] = "paint.dither";
    const char *text = id ? id : "";
    uint64_t hash = sr_fnv1a64(SR_FNV_OFFSET, text, strlen(text));
    const unsigned char zero = 0;
    hash = sr_fnv1a64(hash, &zero, 1);
    hash = sr_fnv1a64(hash, domain, sizeof(domain) - 1);
    return sr_random_mix64(hash ^ project_seed);
}

/* ---- shape and vector styles ------------------------------------------ */

void sr_stroke_style_init(SrStrokeStyle *style) {
    *style = (SrStrokeStyle){.cap = SR_LINE_CAP_BUTT, .join = SR_LINE_JOIN_MITER,
                             .miter_limit = 4.0, .position = SR_STROKE_CENTER,
                             .order = SR_PAINT_FILL_STROKE};
}

void sr_shape_style_init(SrShapeStyle *style) {
    *style = (SrShapeStyle){.fill_rule = SR_FILL_NONZERO, .points = 5,
                            .trim_mode = SR_TRIM_SIMULTANEOUS};
    style->trim_end.base = 1.0;
    sr_stroke_style_init(&style->stroke);
}

void sr_stroke_style_free(SrStrokeStyle *style) {
    if (!style) return;
    free(style->dash);
    sr_track_free(&style->dash_offset.track);
    style->dash = NULL;
    style->dash_count = 0;
}

void sr_paint_ref_free(SrPaintRef *ref) {
    if (!ref) return;
    free(ref->id);
    *ref = (SrPaintRef){0};
}

void sr_shape_style_free(SrShapeStyle *style) {
    if (!style) return;
    SrAnimValue *values[] = {&style->radius, &style->inner_radius,
        &style->outer_radius, &style->inner_roundness, &style->outer_roundness,
        &style->trim_start, &style->trim_end, &style->trim_offset};
    for (size_t i = 0; i < sizeof(values) / sizeof(values[0]); ++i)
        sr_track_free(&values[i]->track);
    if (style->path) {
        sr_prepared_path_free(style->path);
        free(style->path);
        style->path = NULL;
    }
    sr_stroke_style_free(&style->stroke);
    sr_paint_ref_free(&style->fill_paint);
    sr_paint_ref_free(&style->stroke_paint);
}

void sr_vector_ext_free(SrVectorExtension *ext) {
    if (!ext) return;
    sr_stroke_style_free(&ext->stroke);
    sr_paint_ref_free(&ext->fill_paint);
    sr_paint_ref_free(&ext->stroke_paint);
    free(ext);
}

/* ---- Oklab (Ottosson 2020, https://bottosson.github.io/posts/oklab/) --- */

void sr_oklab_from_linear_srgb(const double rgb[3], double lab[3]) {
    double l = 0.4122214708 * rgb[0] + 0.5363325363 * rgb[1] + 0.0514459929 * rgb[2];
    double m = 0.2119034982 * rgb[0] + 0.6806995451 * rgb[1] + 0.1073969566 * rgb[2];
    double s = 0.0883024619 * rgb[0] + 0.2817188376 * rgb[1] + 0.6299787005 * rgb[2];
    l = cbrt(l);
    m = cbrt(m);
    s = cbrt(s);
    lab[0] = 0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s;
    lab[1] = 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s;
    lab[2] = 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s;
}

void sr_linear_srgb_from_oklab(const double lab[3], double rgb[3]) {
    double l = lab[0] + 0.3963377774 * lab[1] + 0.2158037573 * lab[2];
    double m = lab[0] - 0.1055613458 * lab[1] - 0.0638541728 * lab[2];
    double s = lab[0] - 0.0894841775 * lab[1] - 1.2914855480 * lab[2];
    l = l * l * l;
    m = m * m * m;
    s = s * s * s;
    rgb[0] = 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s;
    rgb[1] = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s;
    rgb[2] = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s;
}

static void apply3(const double m[3][3], const double in[3], double out[3]) {
    for (int r = 0; r < 3; ++r)
        out[r] = m[r][0] * in[0] + m[r][1] * in[1] + m[r][2] * in[2];
}

static double unit(double value) {
    return value > 0.0 ? (value < 1.0 ? value : 1.0) : 0.0;
}

/* ---- evaluation --------------------------------------------------------- */

static bool finite_bounded(double value) {
    return isfinite(value) && fabs(value) <= SR_MAX_PAINT_COORDINATE;
}

static const char *paint_element(const SrPaint *paint) {
    return paint->type == SR_PAINT_LINEAR ? "linearGradient"
         : paint->type == SR_PAINT_RADIAL ? "radialGradient" : "conicGradient";
}

static SrStatus eval_fail(const SrPaint *paint, SrDiagnostics *diag,
                          const char *attribute, const char *message) {
    if (diag)
        sr_diag_error(diag, paint->source_line, paint_element(paint), attribute,
                      "paint '%s': %s", paint->id ? paint->id : "", message);
    return SR_ERR_RENDER;
}

/* Converts one straight encoded working colour into the interpolation
 * space, premultiplied (hue is never premultiplied). */
static void stop_value(const SrPaintEval *eval, SrColor color, double alpha,
                       double out[4], bool *powerless) {
    double encoded[3] = {unit(color.r), unit(color.g), unit(color.b)};
    *powerless = false;
    if (eval->space == SR_INTERP_SRGB) {
        for (int c = 0; c < 3; ++c) out[c] = encoded[c] * alpha;
        out[3] = 0.0;
        return;
    }
    double linear[3];
    for (int c = 0; c < 3; ++c)
        linear[c] = sr_color_decode(encoded[c], eval->working);
    if (eval->space == SR_INTERP_LINEAR) {
        for (int c = 0; c < 3; ++c) out[c] = linear[c] * alpha;
        out[3] = 0.0;
        return;
    }
    double srgb[3], lab[3];
    if (eval->srgb_gamut) memcpy(srgb, linear, sizeof(srgb));
    else apply3(eval->to_srgb, linear, srgb);
    sr_oklab_from_linear_srgb(srgb, lab);
    if (eval->space == SR_INTERP_OKLAB) {
        for (int c = 0; c < 3; ++c) out[c] = lab[c] * alpha;
        out[3] = 0.0;
        return;
    }
    double chroma = hypot(lab[1], lab[2]);
    double hue = atan2(lab[2], lab[1]) * (180.0 / SR_PI);
    if (hue < 0.0) hue += 360.0;
    *powerless = chroma < SR_OKLCH_POWERLESS;
    out[0] = lab[0] * alpha;
    out[1] = chroma * alpha;
    out[2] = *powerless ? 0.0 : hue;
    out[3] = 0.0;
}

static double value_at(const SrAnimValue *value, double time) {
    return sr_anim_eval(value, time);
}

SrStatus sr_paint_eval(const SrPaint *paint, const SrProject *project,
                       double time, double width, double height,
                       SrPaintEval *out, SrDiagnostics *diag) {
    if (!paint || !project || !out) return SR_ERR_ARGUMENT;
    memset(out, 0, sizeof(*out));
    if (!paint->stop_count || paint->stop_count > SR_MAX_GRADIENT_STOPS ||
        !paint->stops)
        return eval_fail(paint, diag, "stop", "gradient needs 1 to 256 stops");
    if (!(width > 0.0) || !(height > 0.0) || !finite_bounded(width) ||
        !finite_bounded(height))
        return eval_fail(paint, diag, NULL, "painted box must be finite and positive");
    out->paint = paint;
    out->type = paint->type;
    out->spread = paint->spread;
    out->space = paint->space;
    out->dither = paint->dither;
    out->dither_x = (unsigned)(paint->dither_key & 7u);
    out->dither_y = (unsigned)((paint->dither_key >> 3) & 7u);
    out->linear_light = project->linear_light;
    out->working = project->working_color_space;
    out->srgb_gamut = out->working == SR_COLOR_SRGB || out->working == SR_COLOR_REC709;
    if (!out->srgb_gamut) {
        sr_color_gamut_matrix(out->working, SR_COLOR_SRGB, out->to_srgb);
        sr_color_gamut_matrix(SR_COLOR_SRGB, out->working, out->from_srgb);
    }
    /* Local point -> rotated local point -> gradient units. */
    double rotation = value_at(&paint->rotation, time);
    if (!finite_bounded(rotation))
        return eval_fail(paint, diag, "rotation", "rotation must be finite");
    double radians = rotation * (SR_PI / 180.0);
    double c = cos(radians), s = sin(radians);
    if (rotation == 0.0) {
        c = 1.0;
        s = 0.0;
    }
    double hx = width * 0.5, hy = height * 0.5;
    /* p' = centre + R(-rotation)(p - centre), R(-a) = [c s; -s c]. */
    double r00 = c, r01 = s, r10 = -s, r11 = c;
    double t0 = hx - (r00 * hx + r01 * hy), t1 = hy - (r10 * hx + r11 * hy);
    double sx = paint->units == SR_PAINT_UNITS_OBJECT ? 1.0 / width : 1.0;
    double sy = paint->units == SR_PAINT_UNITS_OBJECT ? 1.0 / height : 1.0;
    out->a00 = r00 * sx;
    out->a01 = r01 * sx;
    out->a10 = r10 * sy;
    out->a11 = r11 * sy;
    out->b0 = t0 * sx;
    out->b1 = t1 * sy;
    if (paint->type == SR_PAINT_LINEAR) {
        double x1 = value_at(&paint->x1, time), y1 = value_at(&paint->y1, time);
        double x2 = value_at(&paint->x2, time), y2 = value_at(&paint->y2, time);
        if (!finite_bounded(x1) || !finite_bounded(y1) || !finite_bounded(x2) ||
            !finite_bounded(y2))
            return eval_fail(paint, diag, "x1/y1/x2/y2",
                             "gradient coordinates must be finite and within 1e9");
        out->x1 = x1;
        out->y1 = y1;
        /* t = (q - P1).D / |D|^2 when the squared length is a normal
         * number; tinier vectors use the unit direction and 1/|D| (hypot
         * avoids the underflow). A vector whose reciprocal overflows counts
         * as coincident points. */
        double dx = x2 - x1, dy = y2 - y1, length2 = dx * dx + dy * dy;
        out->scaled = !(length2 >= DBL_MIN);
        if (!out->scaled) {
            out->dx = dx;
            out->dy = dy;
            out->inv_length = 1.0 / length2;
        } else {
            double length = hypot(dx, dy);
            double inverse = length > 0.0 ? 1.0 / length : 0.0;
            out->degenerate = !(length > 0.0) || !isfinite(inverse);
            out->dx = out->degenerate ? 0.0 : dx / length;
            out->dy = out->degenerate ? 0.0 : dy / length;
            out->inv_length = out->degenerate ? 0.0 : inverse;
        }
    } else {
        double cx = value_at(&paint->cx, time), cy = value_at(&paint->cy, time);
        if (!finite_bounded(cx) || !finite_bounded(cy))
            return eval_fail(paint, diag, "cx/cy",
                             "gradient centre must be finite and within 1e9");
        out->cx = cx;
        out->cy = cy;
        if (paint->type == SR_PAINT_CONIC) {
            double angle = value_at(&paint->angle, time);
            if (!finite_bounded(angle))
                return eval_fail(paint, diag, "angle", "angle must be finite");
            out->angle = angle;
        } else {
            double r = value_at(&paint->r, time), fr = value_at(&paint->fr, time);
            double fx = paint->fx_set ? value_at(&paint->fx, time) : cx;
            double fy = paint->fy_set ? value_at(&paint->fy, time) : cy;
            double aspect = value_at(&paint->aspect, time);
            if (!finite_bounded(r) || !finite_bounded(fr) || !finite_bounded(fx) ||
                !finite_bounded(fy) || r < 0.0 || fr < 0.0)
                return eval_fail(paint, diag, "r/fr/fx/fy",
                                 "radii must be non-negative; coordinates finite within 1e9");
            if (!(aspect > 0.0) || !finite_bounded(aspect) || aspect < 1e-9)
                return eval_fail(paint, diag, "aspect", "aspect must be positive");
            out->r = r;
            out->fr = fr;
            out->fx = fx;
            out->fy = fy;
            out->aspect = aspect;
            out->concentric = fx == cx && fy == cy;
            out->unpainted = out->concentric && r == fr;
        }
    }
    /* Stops: clamped offsets, forced non-decreasing (SVG/CSS). */
    double previous = 0.0;
    out->stop_count = paint->stop_count;
    for (size_t i = 0; i < paint->stop_count; ++i) {
        const SrGradientStop *stop = &paint->stops[i];
        double raw = value_at(&stop->offset, time);
        if (!isfinite(raw))
            return eval_fail(paint, diag, "offset", "stop offset must be finite");
        double offset = unit(raw);
        if (i && offset < previous) offset = previous;
        previous = offset;
        double opacity = value_at(&stop->opacity, time);
        double midpoint = value_at(&stop->midpoint, time);
        if (!isfinite(opacity) || !isfinite(midpoint))
            return eval_fail(paint, diag, "stop", "stop opacity and midpoint must be finite");
        opacity = unit(opacity);
        midpoint = unit(midpoint);
        SrColor color = sr_anim_color_eval(&stop->color, time);
        double alpha = unit(color.a) * opacity;
        out->offsets[i] = offset;
        out->midpoints[i] = midpoint;
        out->hints[i] = midpoint > 0.0 && midpoint < 1.0
            ? (midpoint == 0.5 ? 1.0 : log(0.5) / log(midpoint)) : -1.0;
        out->alpha[i] = alpha;
        stop_value(out, color, alpha, out->values[i], &out->powerless[i]);
    }
    return SR_OK;
}

/* ---- per-pixel ---------------------------------------------------------- */

bool sr_paint_parameter(const SrPaintEval *eval, double x, double y, double *t) {
    double gx = eval->a00 * x + eval->a01 * y + eval->b0;
    double gy = eval->a10 * x + eval->a11 * y + eval->b1;
    if (eval->type == SR_PAINT_LINEAR) {
        if (eval->degenerate) {
            *t = 1.0;
            return true;
        }
        /* dx, dy and inv_length: D and 1/|D|^2, or unit D and 1/|D|. */
        *t = ((gx - eval->x1) * eval->dx + (gy - eval->y1) * eval->dy) *
             eval->inv_length;
        if (isinf(*t)) *t = copysign(1e18, *t);
        return true;
    }
    if (eval->type == SR_PAINT_CONIC) {
        double dx = gx - eval->cx, dy = gy - eval->cy;
        double theta = atan2(dx, -dy) * (180.0 / SR_PI);
        double turns = (theta - eval->angle) / 360.0;
        *t = turns - floor(turns);
        return true;
    }
    if (eval->unpainted) return false;
    /* Two-point conical gradient (HTML canvas createRadialGradient) in the
     * aspect-scaled space: the largest w with r(w) >= 0 such that
     * |p - f - w (c - f)| = fr + w (r - fr). */
    double k = eval->aspect;
    double pdx = gx - eval->fx, pdy = (gy - eval->fy) * k;
    double dr = eval->r - eval->fr;
    if (eval->concentric) {
        *t = (hypot(pdx, pdy) - eval->fr) / dr;
        return true;
    }
    double cdx = eval->cx - eval->fx, cdy = (eval->cy - eval->fy) * k;
    double a = cdx * cdx + cdy * cdy - dr * dr;
    double b = pdx * cdx + pdy * cdy + eval->fr * dr;
    double c = pdx * pdx + pdy * pdy - eval->fr * eval->fr;
    double scale = cdx * cdx + cdy * cdy + dr * dr;
    double w;
    if (fabs(a) <= 1e-12 * scale) {
        if (b == 0.0) return false;
        w = c / (2.0 * b);
        if (!(eval->fr + w * dr >= 0.0)) return false;
    } else {
        double disc = b * b - a * c;
        if (disc < 0.0) return false;
        double root = sqrt(disc);
        double w1 = (b + root) / a, w2 = (b - root) / a;
        double high = w1 > w2 ? w1 : w2, low = w1 > w2 ? w2 : w1;
        if (eval->fr + high * dr >= 0.0) w = high;
        else if (eval->fr + low * dr >= 0.0) w = low;
        else return false;
    }
    if (!isfinite(w)) return false;
    *t = w;
    return true;
}

static double spread(SrSpread mode, double t) {
    if (mode == SR_SPREAD_REPEAT) return t - floor(t);
    if (mode == SR_SPREAD_REFLECT) {
        double m = t - 2.0 * floor(t * 0.5);
        return 1.0 - fabs(m - 1.0);
    }
    return t < 0.0 ? 0.0 : (t > 1.0 ? 1.0 : t);
}

static double hint_weight(double p, double midpoint, double exponent) {
    if (exponent > 0.0) return exponent == 1.0 ? p : pow(p, exponent);
    if (midpoint <= 0.0) return p > 0.0 ? 1.0 : 0.0;
    return p >= 1.0 ? 1.0 : 0.0;
}

static const unsigned char bayer8[8][8] = {
    { 0, 32,  8, 40,  2, 34, 10, 42}, {48, 16, 56, 24, 50, 18, 58, 26},
    {12, 44,  4, 36, 14, 46,  6, 38}, {60, 28, 52, 20, 62, 30, 54, 22},
    { 3, 35, 11, 43,  1, 33,  9, 41}, {51, 19, 59, 27, 49, 17, 57, 25},
    {15, 47,  7, 39, 13, 45,  5, 37}, {63, 31, 55, 23, 61, 29, 53, 21}};

void sr_paint_sample(const SrPaintEval *eval, double x, double y,
                     int px, int py, float out[4]) {
    double t;
    if (!sr_paint_parameter(eval, x, y, &t) || !isfinite(t)) {
        out[0] = out[1] = out[2] = out[3] = 0.0f;
        return;
    }
    size_t n = eval->stop_count;
    double v[3], alpha;
    bool degenerate = eval->type == SR_PAINT_LINEAR && eval->degenerate;
    if (!degenerate) t = spread(eval->spread, t);
    if (degenerate || t >= eval->offsets[n - 1]) {
        memcpy(v, eval->values[n - 1], sizeof(v));
        alpha = eval->alpha[n - 1];
    } else if (t < eval->offsets[0]) {
        memcpy(v, eval->values[0], sizeof(v));
        alpha = eval->alpha[0];
    } else {
        size_t lo = 0, hi = n - 1;     /* offsets[lo] <= t < offsets[hi] */
        while (hi - lo > 1) {
            size_t mid = lo + (hi - lo) / 2;
            if (eval->offsets[mid] <= t) lo = mid;
            else hi = mid;
        }
        double p = (t - eval->offsets[lo]) / (eval->offsets[hi] - eval->offsets[lo]);
        double w = hint_weight(p, eval->midpoints[lo], eval->hints[lo]);
        const double *a = eval->values[lo], *b = eval->values[hi];
        alpha = eval->alpha[lo] + w * (eval->alpha[hi] - eval->alpha[lo]);
        v[0] = a[0] + w * (b[0] - a[0]);
        v[1] = a[1] + w * (b[1] - a[1]);
        if (eval->space == SR_INTERP_OKLCH) {
            bool pa = eval->powerless[lo], pb = eval->powerless[hi];
            double ha = pa ? (pb ? 0.0 : b[2]) : a[2];
            double hb = pb ? ha : b[2];
            double delta = hb - ha;
            if (delta > 180.0) ha += 360.0;
            else if (delta < -180.0) hb += 360.0;
            v[2] = ha + w * (hb - ha);
        } else {
            v[2] = a[2] + w * (b[2] - a[2]);
        }
    }
    if (!(alpha > 0.0)) {
        out[0] = out[1] = out[2] = out[3] = 0.0f;
        return;
    }
    alpha = alpha < 1.0 ? alpha : 1.0;
    /* Straight colour in the interpolation space. */
    double rgb[3];
    bool encoded = eval->space == SR_INTERP_SRGB;
    if (eval->space == SR_INTERP_SRGB || eval->space == SR_INTERP_LINEAR) {
        for (int c = 0; c < 3; ++c) rgb[c] = v[c] / alpha;
    } else {
        /* Oklch hue is never premultiplied; Oklab b is. */
        double lab[3] = {v[0] / alpha, v[1] / alpha,
                         eval->space == SR_INTERP_OKLCH ? v[2] : v[2] / alpha};
        if (eval->space == SR_INTERP_OKLCH) {
            double chroma = lab[1], hue = lab[2] * (SR_PI / 180.0);
            lab[1] = chroma * cos(hue);
            lab[2] = chroma * sin(hue);
        }
        double srgb[3];
        sr_linear_srgb_from_oklab(lab, srgb);
        if (eval->srgb_gamut) memcpy(rgb, srgb, sizeof(rgb));
        else apply3(eval->from_srgb, srgb, rgb);
    }
    for (int c = 0; c < 3; ++c) rgb[c] = unit(rgb[c]);
    if (eval->dither) {
        double d = ((bayer8[((unsigned)py + eval->dither_y) & 7u]
                          [((unsigned)px + eval->dither_x) & 7u] + 0.5) / 64.0 -
                    0.5) / 255.0;
        for (int c = 0; c < 3; ++c) {
            double e = encoded ? rgb[c] : sr_color_encode(rgb[c], eval->working);
            e = unit(e + d);
            rgb[c] = eval->linear_light ? sr_color_decode(e, eval->working) : e;
        }
    } else if (eval->linear_light && encoded) {
        for (int c = 0; c < 3; ++c) rgb[c] = sr_color_decode(rgb[c], eval->working);
    } else if (!eval->linear_light && !encoded) {
        for (int c = 0; c < 3; ++c) rgb[c] = sr_color_encode(rgb[c], eval->working);
    }
    out[0] = (float)(rgb[0] * alpha);
    out[1] = (float)(rgb[1] * alpha);
    out[2] = (float)(rgb[2] * alpha);
    out[3] = (float)alpha;
}

/* ---- background --------------------------------------------------------- */

typedef struct {
    const SrPaintEval *eval;
    SrFrame *frame;
} FillJob;

static void fill_rows(void *opaque, size_t begin, size_t end) {
    const FillJob *job = opaque;
    size_t width = job->frame->width;
    for (size_t y = begin; y < end; ++y) {
        float *row = job->frame->px + y * width * 4;
        for (size_t x = 0; x < width; ++x)
            sr_paint_sample(job->eval, (double)x + 0.5, (double)y + 0.5,
                            (int)x, (int)y, row + x * 4);
    }
}

SrStatus sr_paint_fill_frame(const SrPaint *paint, const SrProject *project,
                             double time, SrFrame *frame, unsigned threads,
                             SrDiagnostics *diag) {
    if (!frame || !frame->px) return SR_ERR_ARGUMENT;
    /* On the calling thread's stack: the fill allocates nothing. */
    SrPaintEval eval;
    SrStatus status = sr_paint_eval(paint, project, time, frame->width,
                                    frame->height, &eval, diag);
    if (status == SR_OK) {
        FillJob job = {&eval, frame};
        status = sr_parallel_for(frame->height, threads, fill_rows, &job);
    }
    return status;
}
