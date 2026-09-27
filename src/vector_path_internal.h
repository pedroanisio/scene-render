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
typedef struct SrPreparedPath {
    SrPathContour *items;
    size_t count, capacity;
} SrPreparedPath;

/* out must not already own storage. On failure it is empty and freeable.
 * Preserves the legacy path grammar and fixed curve subdivision policy. */
SrStatus sr_prepared_path_parse(const char *text, SrPreparedPath *out);
void sr_prepared_path_free(SrPreparedPath *path);

typedef enum {
    SR_PATH_PARSE_OK,
    SR_PATH_PARSE_ARGUMENT,
    SR_PATH_PARSE_SYNTAX,
    SR_PATH_PARSE_BYTES,
    SR_PATH_PARSE_COMMANDS,
    SR_PATH_PARSE_CONTOURS,
    SR_PATH_PARSE_POINTS,
    SR_PATH_PARSE_COORDINATE,
    SR_PATH_PARSE_STORAGE,
    SR_PATH_PARSE_MEMORY
} SrPathParseError;

typedef struct {
    SrPathParseError error;
    size_t byte_offset;
    uint64_t owned_bytes; /* retained heap capacity; zero on failure */
} SrPathParseInfo;

/* Uses SR_MAX_MASK_PATH_* / SR_MAX_MASK_COORDINATE, including implicit
 * commands, closing points and relative-derived controls. available_bytes
 * is the caller's remaining compositing quota, excluding this caller-owned
 * descriptor and text. It is capped at SR_MAX_COMPOSITE_BYTES. Growth checks
 * include both old and new buffers regardless of realloc's implementation.
 * info is optional. Offsets identify the bad number, the owning command for
 * derived/count/allocation failures, or EOF for incomplete geometry.
 * Syntax/range/resource failures return SR_ERR_ASSET; actual allocation
 * failure returns SR_ERR_MEMORY. out has the ordinary parse ownership. */
SrStatus sr_prepared_mask_path_parse(const char *text, uint64_t available_bytes,
                                      SrPreparedPath *out, SrPathParseInfo *info);

/* Rasterizes an immutable successfully prepared path. Scratch allocations
 * belong to this call; outputs have the public coverage API's contract. */
SrStatus sr_prepared_path_coverage(const SrPreparedPath *path, SrFillRule rule,
                                   double stroke_width, uint32_t width,
                                   uint32_t height, float *fill, float *stroke);

/* Fill coverage of `path` translated by (dx, dy) over a width x height
 * grid of unit pixels, into `fill` (width*height floats). `cells` is
 * caller-owned scratch of (width + 2) * height floats. No allocation. */
SrStatus sr_prepared_path_fill_offset(const SrPreparedPath *path,
                                      SrFillRule rule, double dx, double dy,
                                      uint32_t width, uint32_t height,
                                      float *cells, float *fill);
/* ---- B1-4 geometry prerequisites --------------------------------------- */

/* Segments of a contour: count - 1, plus the closing segment when closed. */
size_t sr_path_contour_segments(const SrPathContour *contour);

/* Arc-length parametrisation of a flattened contour. cumulative receives
 * segments + 1 values: cumulative[0] = 0 and cumulative[i + 1] the length
 * of the first i + 1 segments, summed in order in double. Returns the total
 * length (the last value). */
double sr_path_contour_measure(const SrPathContour *contour, double *cumulative);

/* Sub-polyline between arc lengths s0 <= s1 (clamped to [0, total]), with
 * linearly interpolated end points. out must hold segments + 2 points;
 * *count receives the number written (at least 1; 2 unless the range is a
 * point on a degenerate contour). *tangent receives the unit direction of
 * the segment containing s0 (0,0 for a zero-length contour). */
void sr_path_contour_extract(const SrPathContour *contour,
                             const double *cumulative, double s0, double s1,
                             SrPathPoint *out, size_t *count,
                             SrPathPoint *tangent);

/* Coverage clipping for inside/outside strokes: stroke *= fill (inside) or
 * stroke *= 1 - fill (outside), per sample, clamped to [0,1]. */
void sr_coverage_clip(float *stroke, const float *fill, size_t samples,
                      bool inside);

/* Affine local -> raster map: x' = m[0] x + m[1] y + m[2],
 * y' = m[3] x + m[4] y + m[5] (raster pixel coordinates of the grid). */
typedef struct { double m[6]; } SrPathMap;

/* Conservative logical work of rasterizing `contours` through `map` on a
 * width x height grid: per edge 8 + 4 * (rows + 1) + (columns + 2) of its
 * clamped raster extent. Saturates at UINT64_MAX. */
uint64_t sr_path_raster_work(const SrPathContour *contours, size_t count,
                             const SrPathMap *map, uint32_t width,
                             uint32_t height);

/* Exact-area coverage of the contours mapped by `map` onto a width x height
 * grid, resolved with `rule` (every contour is closed implicitly). `cells`
 * is caller-owned scratch of (width + 2) * height floats; coverage receives
 * width * height values in [0,1]. Mapped points must be finite. */
SrStatus sr_path_rasterize(const SrPathContour *contours, size_t count,
                           const SrPathMap *map, SrFillRule rule,
                           uint32_t width, uint32_t height, float *cells,
                           float *coverage);

#endif
