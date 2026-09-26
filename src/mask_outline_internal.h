/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SR_MASK_OUTLINE_INTERNAL_H
#define SR_MASK_OUTLINE_INTERNAL_H

#include "vector_path_internal.h"

/* Regular polygon and star outlines shared by masks (B1-3). Kept separate
 * so shape constructors (B1-4) can use the same vertex order.
 *
 * Vertex k of a polygon with n points lies at angle -90 + k*360/n degrees
 * (the first points up, subsequent vertices advance clockwise in the
 * renderer's downward-y coordinates) on radius `outer` about (cx, cy). A
 * star has 2n vertices at -90 + k*180/n degrees alternating outer and
 * inner radius, starting with an outer vertex. */
size_t sr_mask_outline_count(bool star, uint32_t points);
void sr_mask_outline(bool star, uint32_t points, double cx, double cy,
                     double outer, double inner, SrPathPoint *out);

#endif
