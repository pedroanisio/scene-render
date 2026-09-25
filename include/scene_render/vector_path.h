#ifndef SCENE_RENDER_VECTOR_PATH_H
#define SCENE_RENDER_VECTOR_PATH_H

#include "scene_render/diagnostics.h"
#include "scene_render/scene.h"

typedef struct {
    SrFillRule fill_rule;
    SrColor fill;
    SrColor stroke;
    double stroke_width;   /* 0 disables the stroke */
} SrVectorStyle;

/* True when `path` parses: SVG subset M/L/H/V/C/Q/Z (absolute and
 * relative), every subpath with at least two points. */
bool sr_vector_path_valid(const char *path);

/* Exact-area coverage of `path` (coordinates in pixels) over a width x
 * height grid: signed-area scanline accumulation, resolved with the fill
 * rule. Exact for non-overlapping contours; where contours overlap within
 * one pixel the areas are summed before the fill rule is applied. `fill` receives the fill coverage; `stroke`, when non-NULL and
 * stroke_width > 0, the coverage of a round-joined, round-capped stroke of
 * that width centred on the outline. Both arrays hold width*height floats
 * in [0,1]. */
SrStatus sr_vector_path_coverage(const char *path, SrFillRule rule,
                                 double stroke_width, uint32_t width,
                                 uint32_t height, float *fill, float *stroke);

/* Composes stroke-over-fill coverage with premultiplied blend-space colors
 * into premultiplied float RGBA `px` (4 floats per pixel), so the stroke is
 * blended over the fill in the project's blend space. `stroke` may be NULL. */
void sr_vector_compose(const float *fill, const float *stroke,
                       const float fill_px[4], const float stroke_px[4],
                       size_t pixels, float *px);

/* Rasterizes `path` with `style` (fill rule and stroke width) and the given
 * premultiplied blend-space colors into premultiplied float RGBA. */
SrStatus sr_vector_path_render(const char *path, const SrVectorStyle *style,
                               const float fill_px[4], const float stroke_px[4],
                               uint32_t width, uint32_t height, float *px,
                               size_t source_line, SrDiagnostics *diag);

#endif
