#ifndef SCENE_RENDER_DEFORM_H
#define SCENE_RENDER_DEFORM_H

#include "scene_render/common.h"

/* A rows x cols control grid spans the local box [0,width] x [0,height];
 * node (r, c) rests at (c/(cols-1)*width, r/(rows-1)*height) and moves by
 * offsets[2*(r*cols+c)], offsets[2*(r*cols+c)+1]. The forward warp is
 * p -> p + D(p) with D the bilinear interpolation of the offsets (constant
 * beyond the box). sr_grid_warp_inverse returns the source point p whose
 * warped position is q, by Newton iteration from q - D(q); an all-zero grid
 * returns q exactly. A Newton result whose forward residual is not below
 * 1e-4 px is replaced by an analytic per-cell inverse (the in-range
 * solution nearest the Newton start); when no cell maps onto q the call
 * returns false ("no source": the pixel stays transparent). Non-finite
 * queries and unrepresentable inverse results also return false. */
bool sr_grid_warp_inverse(const double *offsets, uint32_t rows, uint32_t cols,
                          double width, double height, SrVec2 q, SrVec2 *p);

/* Largest |dx| or |dy| among `count` grid nodes. Expand the current bounds
 * by this displacement when applying each grid in modifier order. */
double sr_grid_warp_extent(const double *offsets, size_t count);

#endif
