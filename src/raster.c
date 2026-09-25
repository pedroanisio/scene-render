#include "scene_render/raster.h"

#include <math.h>

static float clamp01f(float value) {
    return value < 0.0f ? 0.0f : (value > 1.0f ? 1.0f : value);
}

static float screen(float cb, float cs) {
    return cb + cs - cb * cs;
}

static float mix(SrBlendMode mode, float cb, float cs) {
    switch (mode) {
    case SR_BLEND_ADD:
        return cb + cs;
    case SR_BLEND_MULTIPLY:
        return cb * cs;
    case SR_BLEND_SCREEN:
        return screen(clamp01f(cb), clamp01f(cs));
    case SR_BLEND_OVERLAY: {
        /* overlay(cb, cs) = hardlight(cs, cb) */
        float b = clamp01f(cb), s = clamp01f(cs);
        return b <= 0.5f ? 2.0f * b * s : screen(s, 2.0f * b - 1.0f);
    }
    case SR_BLEND_DIFFERENCE:
        return fabsf(cb - cs);
    case SR_BLEND_NORMAL:
    default:
        return cs;
    }
}

void sr_blend_px(SrBlendMode mode, float dst[4], const float src[4]) {
    float as = src[3];
    if (!(as > 0.0f)) return;
    float ab = dst[3];
    if (mode == SR_BLEND_NORMAL || !(ab > 0.0f)) {
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
                 as * ab * mix(mode, cb, cs);
    }
    dst[3] = as + ab * (1.0f - as);
}

double sr_shape_distance(SrMaskType type, double x, double y, double w,
                         double h, double radius, double lx, double ly) {
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

float sr_distance_coverage(double distance, double aa) {
    if (!(aa > 0.0)) aa = 1e-9;
    double c = 0.5 - distance / aa;
    return (float)(c < 0.0 ? 0.0 : (c > 1.0 ? 1.0 : c));
}

float sr_shape_coverage(SrMaskType type, double x, double y, double w,
                        double h, double radius, double lx, double ly,
                        double aa) {
    return sr_distance_coverage(
        sr_shape_distance(type, x, y, w, h, radius, lx, ly), aa);
}
