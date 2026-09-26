/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SR_COMPOSITOR_SHAPE_INTERNAL_H
#define SR_COMPOSITOR_SHAPE_INTERNAL_H

#include "compositor_internal.h"
#include "compositor_resources_internal.h"
#include "scene_render/paint.h"

/* B1-4 extended shapes (docs/design/b1-4-shapes-paints.md, "Rendering
 * path"). A raster is built on the calling thread and is immutable while
 * row workers sample it; it is released after the immediate draw. */

typedef struct {
    const SrScene *scene;
    const SrNode *node;
    double time;
    double width, height;       /* evaluated local box */
    SrMat3 world;               /* local -> target pixels */
    SrClip clip;                /* target clip */
    bool local_grid;            /* deformation: rasterize in local units */
    SrCompositeResources *resources;  /* borrowed, may be NULL */
    SrDiagnostics *diag;
} SrShapeInput;

typedef struct SrShapeRaster {
    SrCompositeResources *resources;
    SrClip bounds;              /* target pixels to visit (screen grid) */
    bool local_grid;
    int grid_x0, grid_y0;       /* grid origin: target pixel or local unit */
    uint32_t grid_w, grid_h;
    float *fill, *stroke;       /* coverage per grid sample, or NULL */
    bool draw_fill;             /* fill contributes colour */
    SrPaintOrder order;
    float fill_px[4], stroke_px[4];     /* solid premultiplied blend colours */
    const SrPaintEval *fill_paint, *stroke_paint;
    SrPaintEval *evals;         /* owned storage for the two evaluations */
    double extent[4];           /* local geometry bounds (x0 y0 x1 y1) */
    uint64_t pixel_cost;        /* logical work per sampled pixel */
    bool empty;
} SrShapeRaster;

/* Builds the raster; *out is always freeable. SR_ERR_RENDER reports the
 * owning shape attribute through diag or the ledger. */
SrStatus sr_shape_raster_build(const SrShapeInput *input, SrShapeRaster *out);
void sr_shape_raster_free(SrShapeRaster *raster);

/* Composed premultiplied colour before opacity, masks and blending; false
 * when the sample is empty. (px, py) are target pixel coordinates for the
 * grid and dithering, (x, y) the local point. */
bool sr_shape_raster_sample(const SrShapeRaster *raster, int px, int py,
                            double x, double y, float out[4]);

/* Preparation check of an extended shape's authored storage for direct-C
 * scenes: counts, even dash storage, stop storage, paint and path presence.
 * *bytes receives the prepared geometry's heap capacity. */
bool sr_shape_style_prepare(const SrNode *node, uint64_t *bytes,
                            const char **attribute, const char **message);

/* Conservative local bounds of an extended shape at `time` (geometry plus
 * stroke, joins, caps and one pixel), for content bounds. */
bool sr_shape_local_bounds(SrCompositeResources *resources, const SrNode *node,
                           double time, double width, double height,
                           double out[4]);

#endif
