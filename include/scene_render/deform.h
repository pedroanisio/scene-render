#ifndef SCENE_RENDER_DEFORM_H
#define SCENE_RENDER_DEFORM_H

#include "scene_render/common.h"

/* A rows x cols control grid spans the local box [0,width] x [0,height];
 * node (r, c) rests at (c/(cols-1)*width, r/(rows-1)*height) and moves by
 * offsets[2*(r*cols+c)], offsets[2*(r*cols+c)+1]. The forward warp is
 * p -> p + D(p) with D the bilinear interpolation of the offsets (constant
 * beyond the box). sr_grid_warp_inverse returns the source point p whose
 * warped position is q, by Newton iteration from q - D(q); an all-zero grid
 * returns q exactly. */
SrVec2 sr_grid_warp_inverse(const double *offsets, uint32_t rows, uint32_t cols,
                            double width, double height, SrVec2 q);

/* Largest |dx| or |dy| among `count` grid nodes (bounds padding). */
double sr_grid_warp_extent(const double *offsets, size_t count);

#endif
