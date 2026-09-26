#include "scene_render/raster.h"

/* Legacy arithmetic stays in raster.h's inline forms. New color kernels
 * live here so they do not enlarge the old per-pixel paths. */

void sr_blend_px(SrBlendMode mode, float dst[4], const float src[4]) {
    sr_blend_px_inline(mode, dst, src);
}

static double clamp_unit(double value) {
    return value < 0.0 ? 0.0 : (value > 1.0 ? 1.0 : value);
}

static double dodge(double backdrop, double source) {
    if (backdrop == 0.0) return 0.0;
    if (source == 1.0) return 1.0;
    return fmin(1.0, backdrop / (1.0 - source));
}

static double burn(double backdrop, double source) {
    if (backdrop == 1.0) return 1.0;
    if (source == 0.0) return 0.0;
    return 1.0 - fmin(1.0, (1.0 - backdrop) / source);
}

static double vivid(double backdrop, double source) {
    return source <= 0.5 ? burn(backdrop, 2.0 * source)
                         : dodge(backdrop, 2.0 * source - 1.0);
}

static double separable(SrBlendMode mode, double b, double s) {
    switch (mode) {
    case SR_BLEND_EXCLUSION: return b + s - 2.0 * b * s;
    case SR_BLEND_SUBTRACT: return fmax(0.0, b - s);
    case SR_BLEND_DIVIDE:
        if (b == 0.0) return 0.0;
        return s == 0.0 ? 1.0 : fmin(1.0, b / s);
    case SR_BLEND_DARKEN: return fmin(b, s);
    case SR_BLEND_LIGHTEN: return fmax(b, s);
    case SR_BLEND_COLOR_DODGE: return dodge(b, s);
    case SR_BLEND_COLOR_BURN: return burn(b, s);
    case SR_BLEND_LINEAR_DODGE: return fmin(1.0, b + s);
    case SR_BLEND_LINEAR_BURN: return fmax(0.0, b + s - 1.0);
    case SR_BLEND_SOFT_LIGHT: {
        if (s <= 0.5) return b - (1.0 - 2.0 * s) * b * (1.0 - b);
        double d = b <= 0.25 ? ((16.0 * b - 12.0) * b + 4.0) * b
                             : sqrt(b);
        return b + (2.0 * s - 1.0) * (d - b);
    }
    case SR_BLEND_HARD_LIGHT:
        return s <= 0.5 ? 2.0 * b * s
                        : 1.0 - (1.0 - b) * (2.0 - 2.0 * s);
    case SR_BLEND_LINEAR_LIGHT: return clamp_unit(b + 2.0 * s - 1.0);
    case SR_BLEND_VIVID_LIGHT: return vivid(b, s);
    case SR_BLEND_PIN_LIGHT:
        return s <= 0.5 ? fmin(b, 2.0 * s) : fmax(b, 2.0 * s - 1.0);
    case SR_BLEND_HARD_MIX: return vivid(b, s) < 0.5 ? 0.0 : 1.0;
    default: return s;
    }
}

static double lum(const double rgb[3]) {
    return 0.30 * rgb[0] + 0.59 * rgb[1] + 0.11 * rgb[2];
}

static double saturation(const double rgb[3]) {
    return fmax(rgb[0], fmax(rgb[1], rgb[2])) -
           fmin(rgb[0], fmin(rgb[1], rgb[2]));
}

static void set_saturation(double rgb[3], double value) {
    double low = fmin(rgb[0], fmin(rgb[1], rgb[2]));
    double high = fmax(rgb[0], fmax(rgb[1], rgb[2]));
    for (size_t c = 0; c < 3; ++c) {
        if (rgb[c] == low) rgb[c] = 0.0;
        else if (rgb[c] == high) rgb[c] = value;
        else rgb[c] = (rgb[c] - low) * value / (high - low);
    }
}

/* W3C SetLum/ClipColor, using double intermediates for new modes only.
 * Endpoint guards prevent roundoff in a shifted black/white from dividing
 * by a vanishing range. The final clamp removes sub-ulp gamut excursions. */
static void set_lum(double rgb[3], double value) {
    double shift = value - lum(rgb);
    for (size_t c = 0; c < 3; ++c) rgb[c] += shift;
    double light = lum(rgb);
    double low = fmin(rgb[0], fmin(rgb[1], rgb[2]));
    double high = fmax(rgb[0], fmax(rgb[1], rgb[2]));
    for (size_t c = 0; c < 3; ++c) {
        if (light <= 0.0) rgb[c] = 0.0;
        else if (light >= 1.0) rgb[c] = 1.0;
        else {
            if (low < 0.0)
                rgb[c] = light + (rgb[c] - light) * light / (light - low);
            if (high > 1.0)
                rgb[c] = light + (rgb[c] - light) * (1.0 - light) /
                         (high - light);
            rgb[c] = clamp_unit(rgb[c]);
        }
    }
}

static void mix_color(SrBlendMode mode, const double b[3],
                       const double s[3], double out[3]) {
    const double *chosen = s;
    if (mode == SR_BLEND_DARKER_COLOR)
        chosen = lum(s) < lum(b) ? s : b;
    else if (mode == SR_BLEND_LIGHTER_COLOR)
        chosen = lum(s) > lum(b) ? s : b;
    else if (mode == SR_BLEND_SATURATION || mode == SR_BLEND_LUMINOSITY)
        chosen = b;
    for (size_t c = 0; c < 3; ++c) out[c] = chosen[c];
    if (mode == SR_BLEND_HUE) set_saturation(out, saturation(b));
    else if (mode == SR_BLEND_SATURATION) set_saturation(out, saturation(s));
    if (mode == SR_BLEND_HUE || mode == SR_BLEND_SATURATION ||
        mode == SR_BLEND_COLOR)
        set_lum(out, lum(b));
    else if (mode == SR_BLEND_LUMINOSITY)
        set_lum(out, lum(s));
    else if (mode != SR_BLEND_DARKER_COLOR && mode != SR_BLEND_LIGHTER_COLOR)
        for (size_t c = 0; c < 3; ++c) out[c] = separable(mode, b[c], s[c]);
}

void sr_blend_px_color(SrBlendMode mode, float dst[4], const float src[4]) {
    float as = src[3], ab = dst[3];
    if (!(as > 0.0f)) return;
    if (mode == SR_BLEND_PLUS_LIGHTER) {
        for (size_t c = 0; c < 4; ++c)
            dst[c] = (float)clamp_unit((double)src[c] + dst[c]);
        return;
    }
    if (as < SR_BLEND_MIN_ALPHA || ab < SR_BLEND_MIN_ALPHA) {
        sr_blend_px_inline(SR_BLEND_NORMAL, dst, src);
        return;
    }
    double b[3], s[3], mixed[3];
    for (size_t c = 0; c < 3; ++c) {
        b[c] = clamp_unit((double)dst[c] / ab);
        s[c] = clamp_unit((double)src[c] / as);
    }
    mix_color(mode, b, s, mixed);
    for (size_t c = 0; c < 3; ++c)
        dst[c] = (float)((1.0 - ab) * src[c] + (1.0 - as) * dst[c] +
                        (double)as * ab * mixed[c]);
    dst[3] = as + ab * (1.0f - as);
}

double sr_shape_distance(SrMaskType type, double x, double y, double w,
                         double h, double radius, double lx, double ly) {
    return sr_shape_distance_inline(type, x, y, w, h, radius, lx, ly);
}

float sr_distance_coverage(double distance, double aa) {
    return sr_distance_coverage_inline(distance, aa);
}

float sr_shape_coverage(SrMaskType type, double x, double y, double w,
                        double h, double radius, double lx, double ly,
                        double aa) {
    return sr_distance_coverage_inline(
        sr_shape_distance_inline(type, x, y, w, h, radius, lx, ly), aa);
}
