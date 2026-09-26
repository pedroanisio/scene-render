/* SPDX-License-Identifier: Apache-2.0 */
#include "mask_outline_internal.h"
#include "scene_render/common.h"

#include <math.h>

size_t sr_mask_outline_count(bool star, uint32_t points) {
    return star ? 2 * (size_t)points : (size_t)points;
}

void sr_mask_outline(bool star, uint32_t points, double cx, double cy,
                     double outer, double inner, SrPathPoint *out) {
    size_t count = sr_mask_outline_count(star, points);
    for (size_t k = 0; k < count; ++k) {
        double angle = -0.5 * SR_PI + 2.0 * SR_PI * (double)k / (double)count;
        double radius = star && (k & 1) ? inner : outer;
        out[k] = (SrPathPoint){cx + radius * cos(angle), cy + radius * sin(angle)};
    }
}
