#ifndef SCENE_RENDER_EFFECTS_H
#define SCENE_RENDER_EFFECTS_H

#include "scene_render/compositor.h"

/* Pixel rectangle [x0,x1) x [y0,y1). */
typedef struct {
    int x0, y0, x1, y1;
} SrEffectRect;

/* Applies every enabled effect that no group references, in XML order, to
 * the whole frame (effect positions are canvas pixels). */
SrStatus sr_effects_apply(const SrScene *scene, double time, SrFrame *frame,
                          unsigned threads, SrDiagnostics *diag);

/* Applies `effects` in order to an isolated group buffer whose non-zero
 * pixels lie inside *rect. Each effect first grows *rect by its reach
 * (sr_effect_reach) and runs over the grown rectangle only, so the result
 * equals running it over the whole buffer. `to_canvas` maps the group's
 * local space to buffer pixels (light positions and ranges). */
SrStatus sr_effects_apply_group(const SrScene *scene, SrEffect *const *effects,
                                size_t count, double time, SrFrame *frame,
                                SrMat3 to_canvas, SrEffectRect *rect,
                                unsigned threads);

/* Depth-of-field blur of the pixels in *rect (grown by the kernel reach,
 * clipped to the frame, and returned in *rect): three box passes giving a
 * near-Gaussian of standard deviation radius / 2, the fractional part
 * blended between the neighbouring integer kernels so animated radii vary
 * smoothly. The deviation saturates at 64 px. */
SrStatus sr_effects_blur_rect(SrFrame *frame, SrEffectRect *rect,
                              double radius, unsigned threads);

/* How far, in pixels, `effect` at `time` can move content outward: the
 * blur radius for glow/bloom/blur, offset plus radius plus one for
 * drop-shadow, zero for per-pixel effects. */
int sr_effect_reach(const SrEffect *effect, double time);

/* 2D light falloff curve at normalized distance q = d / range (0 at q >= 1):
 * smooth (1 - q^2)^2, linear 1 - q, quadratic (1 - q)^2, none 1. */
double sr_light_falloff(SrFalloff falloff, double q);

/* Exact slope bounds x/z of the tangent cone from the origin to a sphere
 * of radius r centred at (a, z) in one light-space axis plane (a along the
 * map axis, z along the light direction): the tangent directions at
 * atan2(a, z) +- asin(r / hypot(a, z)). False when the sphere reaches
 * z <= r (the cone is unbounded). Used for spot shadow-map caster bounds
 * (src/lighting.c). */
bool sr_light_cone_slopes(double a, double z, double r, double *low,
                          double *high);

#endif
