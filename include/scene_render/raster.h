#ifndef SCENE_RENDER_RASTER_H
#define SCENE_RENDER_RASTER_H

#include "scene_render/scene.h"

#include <math.h>

/* Composites premultiplied `src` over premultiplied `dst` in place with the
 * separable W3C Compositing Level 1 formula
 *   co = (1-ab)*cs' + (1-as)*cb' + as*ab*B(cb, cs),  ao = as + ab*(1-as)
 * where cs', cb' are premultiplied and cs, cb straight (unpremultiplied only
 * to evaluate B). ADD (B = cb + cs) is never clamped; screen and overlay
 * clamp their straight inputs to [0,1]. A transparent src is a no-op and a
 * transparent backdrop yields src for every mode. */
void sr_blend_px(SrBlendMode mode, float dst[4], const float src[4]);

/* Signed distance (negative inside) from local point (lx, ly) to the
 * rect/ellipse/rounded-rect occupying [x, x+w] x [y, y+h]. The ellipse uses
 * the first-order estimate g/|grad g|, exact on the outline. */
double sr_shape_distance(SrMaskType type, double x, double y, double w,
                         double h, double radius, double lx, double ly);

/* Anti-aliased coverage in [0,1] for a signed distance, where `aa` is the
 * local size of one output pixel. */
float sr_distance_coverage(double distance, double aa);

/* Shorthand for sr_distance_coverage(sr_shape_distance(...), aa). */
float sr_shape_coverage(SrMaskType type, double x, double y, double w,
                        double h, double radius, double lx, double ly,
                        double aa);

/* ---- inline forms for per-pixel loops -----------------------------------
 * The functions above are thin wrappers around these. Both evaluate exactly
 * the same float/double operations in the same order, so a caller gets
 * bit-identical results from either; inlining only lets hot loops hoist the
 * per-draw invariants (mode, shape type, sizes) and drop the call. */

#define SR_BLEND_MIN_ALPHA 1e-6f

static inline float sr_clamp01f(float value) {
    return value < 0.0f ? 0.0f : (value > 1.0f ? 1.0f : value);
}

static inline float sr_screen_f(float cb, float cs) {
    return cb + cs - cb * cs;
}

static inline float sr_blend_mix(SrBlendMode mode, float cb, float cs) {
    switch (mode) {
    case SR_BLEND_ADD:
        return cb + cs;
    case SR_BLEND_MULTIPLY:
        return cb * cs;
    case SR_BLEND_SCREEN:
        return sr_screen_f(sr_clamp01f(cb), sr_clamp01f(cs));
    case SR_BLEND_OVERLAY: {
        /* overlay(cb, cs) = hardlight(cs, cb) */
        float b = sr_clamp01f(cb), s = sr_clamp01f(cs);
        return b <= 0.5f ? 2.0f * b * s : sr_screen_f(s, 2.0f * b - 1.0f);
    }
    case SR_BLEND_DIFFERENCE:
        return fabsf(cb - cs);
    case SR_BLEND_NORMAL:
    default:
        return cs;
    }
}

static inline void sr_blend_px_inline(SrBlendMode mode, float *dst,
                                      const float *src) {
    float as = src[3];
    if (!(as > 0.0f)) return;
    float ab = dst[3];
    /* Un-premultiplying divides by alpha; below SR_BLEND_MIN_ALPHA the
     * quotient can overflow while the mode term's weight (as * ab) is
     * negligible, so such pixels use source-over instead. */
    if (mode == SR_BLEND_NORMAL || as < SR_BLEND_MIN_ALPHA ||
        ab < SR_BLEND_MIN_ALPHA) {
        float keep = 1.0f - as;
        dst[0] = src[0] + dst[0] * keep;
        dst[1] = src[1] + dst[1] * keep;
        dst[2] = src[2] + dst[2] * keep;
        dst[3] = as + ab * keep;
        return;
    }
    float inv_as = 1.0f / as, inv_ab = 1.0f / ab;
    for (int c = 0; c < 3; ++c) {
        float cs = src[c] * inv_as;
        float cb = dst[c] * inv_ab;
        dst[c] = (1.0f - ab) * src[c] + (1.0f - as) * dst[c] +
                 as * ab * sr_blend_mix(mode, cb, cs);
    }
    dst[3] = as + ab * (1.0f - as);
}

static inline double sr_shape_distance_inline(SrMaskType type, double x,
                                              double y, double w, double h,
                                              double radius, double lx,
                                              double ly) {
    if (!(w > 0.0) || !(h > 0.0)) return HUGE_VAL;
    double hw = 0.5 * w, hh = 0.5 * h;
    double px = lx - (x + hw), py = ly - (y + hh);
    if (type == SR_MASK_ELLIPSE) {
        double ux = px / hw, uy = py / hh;
        double k0 = sqrt(ux * ux + uy * uy);
        double gx = ux / hw, gy = uy / hh;
        double k1 = sqrt(gx * gx + gy * gy);
        return k1 > 1e-12 ? k0 * (k0 - 1.0) / k1 : -(hw < hh ? hw : hh);
    }
    double r = 0.0;
    if (type == SR_MASK_ROUNDED_RECT) {
        r = radius > 0.0 ? radius : 0.0;
        if (r > hw) r = hw;
        if (r > hh) r = hh;
    }
    double qx = fabs(px) - (hw - r), qy = fabs(py) - (hh - r);
    double ox = qx > 0.0 ? qx : 0.0, oy = qy > 0.0 ? qy : 0.0;
    double inside = qx > qy ? qx : qy;
    return sqrt(ox * ox + oy * oy) + (inside < 0.0 ? inside : 0.0) - r;
}

static inline float sr_distance_coverage_inline(double distance, double aa) {
    if (!(aa > 0.0)) aa = 1e-9;
    double c = 0.5 - distance / aa;
    return (float)(c < 0.0 ? 0.0 : (c > 1.0 ? 1.0 : c));
}

#endif
