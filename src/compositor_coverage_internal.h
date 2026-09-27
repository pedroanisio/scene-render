/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SR_COMPOSITOR_COVERAGE_INTERNAL_H
#define SR_COMPOSITOR_COVERAGE_INTERNAL_H

#include "compositor_internal.h"
#include "compositor_resources_internal.h"
#include "vector_path_internal.h"

/* Scene-owned immutable geometry of a path mask (mask->prepared). The
 * flattened contours are parsed once, at load or explicit preparation, and
 * shared by every render. owned_bytes is the retained heap capacity,
 * including this descriptor, counted in aggregate prepared ownership. */
typedef struct SrMaskPath {
    SrPreparedPath path;
    uint64_t owned_bytes;
} SrMaskPath;


/* Parses mask->path into a new mask->prepared (replacing any previous one)
 * with the bounded mask-path grammar. available_bytes is the remaining
 * compositing byte quota. info is optional (error category and offset). */
SrStatus sr_mask_path_prepare(SrMask *mask, uint64_t available_bytes,
                               SrPathParseInfo *info);

/* True when the mask needs the node-local coverage raster: a path, polygon
 * or star type, a non-intersect mode or a non-default opacity, feather or
 * expansion (static values or tracks). Explicit defaults stay analytic. */
bool sr_mask_advanced(const SrMask *mask);
bool sr_node_masks_advanced(const SrNode *node);

/* Static load/preparation validation of one mask's new attributes;
 * NULL when valid, otherwise the offending attribute (and *message). */
const char *sr_mask_validate(const SrMask *mask, const char **message);

/* A node-local scalar coverage raster: sample (i, j) is local point
 * (x0 + i + 0.5, y0 + j + 0.5). Outside the stored grid the coverage is the
 * constant `exterior`. Values are in [0,1]. Immutable after building. */
typedef struct SrCoverageGrid {
    float *values;
    int x0, y0;
    uint32_t width, height;
    float exterior;
} SrCoverageGrid;

/* Evaluates `node`'s masks at `time` (evaluated relative lengths supplied
 * by `lengths` when present) and builds their combined coverage for the
 * receiving `clip`, whose pixel centres map to local space by `inverse`.
 * All storage and work are charged to `resources` (never NULL here).
 * Evaluated-range and resource failures return SR_ERR_RENDER with the
 * mask's source owner; allocation failure SR_ERR_MEMORY. */
SrStatus sr_coverage_build(SrCompositeResources *resources, const SrScene *scene,
                           const SrNode *node, const SrLengthFrame *lengths,
                           double time, SrMat3 inverse, SrClip clip,
                           SrDiagnostics *diag, SrCoverageGrid *grid);
void sr_coverage_free(SrCompositeResources *resources, SrCoverageGrid *grid);

/* Bilinear sample at local point (lx, ly), with the exterior value for
 * taps outside the stored grid. */
float sr_coverage_sample(const SrCoverageGrid *grid, double lx, double ly);

/* Local-space bounds [x0,x1) x [y0,y1) outside which coverage equals the
 * exterior value. */
void sr_coverage_extent(const SrCoverageGrid *grid, double box[4]);

/* Coverage contributed by a special link (grid or matte) at receiving
 * pixel centre (cx, cy). */
float sr_link_special_coverage(const SrMaskLink *link, double cx, double cy);

#endif
