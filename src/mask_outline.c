/* SPDX-License-Identifier: Apache-2.0 */
#include "mask_outline_internal.h"
#include "scene_render/common.h"
#include "vector_shape_internal.h"


size_t sr_mask_outline_count(bool star, uint32_t points) {
    return star ? 2 * (size_t)points : (size_t)points;
}

void sr_mask_outline(bool star, uint32_t points, double cx, double cy,
                     double outer, double inner, SrPathPoint *out) {
    size_t count = sr_mask_outline_count(star, points);
    /* The shared B1-4 vertex helper gives shape and mask outlines the same
     * vertex order and arithmetic. */
    for (size_t k = 0; k < count; ++k)
        out[k] = sr_shape_vertex(cx, cy, star && (k & 1) ? inner : outer, k, count);
}
