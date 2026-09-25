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
 * rule. `fill` receives the fill coverage; `stroke`, when non-NULL and
 * stroke_width > 0, the coverage of a round-joined, round-capped stroke of
 * that width centred on the outline. Both arrays hold width*height floats
 * in [0,1]. */
SrStatus sr_vector_path_coverage(const char *path, SrFillRule rule,
                                 double stroke_width, uint32_t width,
                                 uint32_t height, float *fill, float *stroke);

/* Composes stroke-over-fill coverage with the style colors into 8-bit
 * straight RGBA (working-space values). `stroke` may be NULL. */
void sr_vector_compose(const float *fill, const float *stroke,
                       const SrVectorStyle *style, size_t pixels,
                       uint8_t *rgba8);

/* Rasterizes `path` with `style` into 8-bit straight RGBA. */
SrStatus sr_vector_path_render(const char *path, const SrVectorStyle *style,
                               uint32_t width, uint32_t height, uint8_t *rgba8,
                               size_t source_line, SrDiagnostics *diag);

#endif
