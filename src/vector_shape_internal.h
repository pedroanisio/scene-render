/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SR_VECTOR_SHAPE_INTERNAL_H
#define SR_VECTOR_SHAPE_INTERNAL_H

#include "vector_path_internal.h"
#include "compositor_resources_internal.h"

/* B1-4 shape geometry (docs/design/b1-4-shapes-paints.md). All storage uses
 * the optional ledger: NULL selects plain allocation (vector assets and
 * tests). Growth charges 16 work units per new point slot before use. */

#define SR_SHAPE_TOLERANCE 0.01     /* raster pixels */

typedef struct {
    size_t offset, count;
    bool closed;
} SrPolyRange;

/* Contours share one point arena; `contours` is built by sr_polyset_seal and
 * points into it. Not thread-safe while building; immutable once sealed. */
typedef struct {
    SrCompositeResources *resources;   /* borrowed, may be NULL */
    SrPathPoint *points;
    size_t point_count, point_capacity;
    SrPolyRange *ranges;
    size_t range_count, range_capacity;
    SrPathContour *contours;           /* range_count entries after sealing */
    size_t limit;                      /* maximum points */
    size_t open;                       /* index of the open range + 1, or 0 */
    SrStatus status;
    const char *error;                 /* limit/argument diagnostic */
} SrPolySet;

void sr_polyset_init(SrPolySet *set, SrCompositeResources *resources,
                     size_t limit);
void sr_polyset_free(SrPolySet *set);
bool sr_polyset_begin(SrPolySet *set);
bool sr_polyset_add(SrPolySet *set, SrPathPoint point);
/* Ends the open contour; contours with fewer than one point are dropped. */
bool sr_polyset_end(SrPolySet *set, bool closed);
bool sr_polyset_seal(SrPolySet *set);
bool sr_polyset_fail(SrPolySet *set, SrStatus status, const char *error);

/* ---- constructors ------------------------------------------------------ */

/* Flattening counts for tolerance tau at local->raster scale s. */
size_t sr_shape_arc_pieces(double radius, double sweep, double scale);
size_t sr_shape_cubic_pieces(const SrPathPoint p[4], double scale);

/* Appends arc points of a circle (centre, radius) from angle a0 sweeping by
 * `sweep` radians, excluding the start point and including the end. */
bool sr_shape_arc(SrPolySet *set, SrPathPoint centre, double radius, double a0,
                  double sweep, double scale);
bool sr_shape_cubic(SrPolySet *set, const SrPathPoint p[4], double scale);

typedef struct {
    SrShapeType type;
    double width, height;
    double radii[4];             /* TL TR BR BL, after evaluation */
    uint32_t points;
    double outer_radius, inner_radius;
    double outer_roundness, inner_roundness;
    const SrPreparedPath *path;  /* borrowed */
    double scale;
} SrShapeParams;

/* CSS Backgrounds 3 corner-radius scaling of radii (clamped at zero). */
void sr_shape_corner_radii(double width, double height, double radii[4]);
/* Builds the local fill contours of a shape. */
bool sr_shape_build(SrPolySet *set, const SrShapeParams *params);
/* Vertex k of a polygon/star (first vertex up, clockwise in y-down). */
SrPathPoint sr_shape_vertex(double cx, double cy, double radius, size_t index,
                            size_t count);

/* ---- strokes ----------------------------------------------------------- */

typedef struct {
    double width;                /* full width, local units, > 0 */
    SrLineCap cap;
    SrLineJoin join;
    double miter_limit;
    const double *dash;          /* even count, or NULL */
    size_t dash_count;
    double dash_offset;
    bool trimmed;
    double trim_start, trim_end, trim_offset;
    SrTrimMode trim_mode;
    double scale;                /* local -> raster, for round pieces */
} SrStrokeParams;

/* A visible stretch of a contour in arc length; wraps continue from the end
 * of a closed contour through its start to `wrap_end`. */
typedef struct {
    size_t contour;
    double start, end;
    bool wrap;
    double wrap_end;
    bool whole;                  /* the untrimmed closed contour */
} SrStrokePiece;

/* Visible pieces after trimming (tests). pieces holds at least
 * 2 * contour_count + 2 entries. lengths[i] is contour i's arc length. */
size_t sr_stroke_trim(const double *lengths, const bool *closed, size_t count,
                      const SrStrokeParams *params, SrStrokePiece *pieces);

/* Nonzero-fill polygons of the stroke of the sealed source contours. */
bool sr_stroke_build(const SrPolySet *source, const SrStrokeParams *params,
                     SrPolySet *out);

/* Stroker of one polyline (open, or closed without repeating the first
 * point). tangent orients a degenerate square cap. */
bool sr_stroke_polyline(SrPolySet *out, const SrPathPoint *points, size_t count,
                        bool closed, SrPathPoint tangent,
                        const SrStrokeParams *params);

#endif
