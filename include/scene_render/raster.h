#ifndef SCENE_RENDER_RASTER_H
#define SCENE_RENDER_RASTER_H

#include "scene_render/scene.h"

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

#endif
