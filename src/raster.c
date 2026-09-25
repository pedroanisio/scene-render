#include "scene_render/raster.h"

/* Out-of-line entry points; the arithmetic lives in the inline forms in
 * raster.h so per-pixel loops can inline it unchanged. */

void sr_blend_px(SrBlendMode mode, float dst[4], const float src[4]) {
    sr_blend_px_inline(mode, dst, src);
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
