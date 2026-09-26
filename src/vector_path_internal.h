/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SR_VECTOR_PATH_INTERNAL_H
#define SR_VECTOR_PATH_INTERNAL_H

#include "scene_render/vector_path.h"

typedef struct { double x, y; } SrPathPoint;

typedef struct {
    SrPathPoint *points;
    size_t count, capacity;
    bool closed;
} SrPathContour;

/* Owns each contour and its flattened points. After preparation, all fields
 * and reachable storage are immutable until sr_prepared_path_free(). The
 * caller owns this descriptor and must keep it alive through every raster
 * call. Distinct calls may share a prepared path and use separate outputs. */
typedef struct {
    SrPathContour *items;
    size_t count, capacity;
} SrPreparedPath;

/* out must not already own storage. On failure it is empty and freeable.
 * Preserves the legacy path grammar and fixed curve subdivision policy;
 * new mask limits are added separately before enabling those consumers. */
SrStatus sr_prepared_path_parse(const char *text, SrPreparedPath *out);
void sr_prepared_path_free(SrPreparedPath *path);

/* Rasterizes an immutable successfully prepared path. Scratch allocations
 * belong to this call; outputs have the public coverage API's contract. */
SrStatus sr_prepared_path_coverage(const SrPreparedPath *path, SrFillRule rule,
                                   double stroke_width, uint32_t width,
                                   uint32_t height, float *fill, float *stroke);

#endif
