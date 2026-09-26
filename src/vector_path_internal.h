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

#endif
