#ifndef SCENE_RENDER_COMPOSITOR_H
#define SCENE_RENDER_COMPOSITOR_H

#include "scene_render/diagnostics.h"
#include "scene_render/scene.h"

typedef struct {
    uint32_t width;
    uint32_t height;
    uint8_t *rgba;
} SrFrame;

SrStatus sr_frame_init(SrFrame *frame, uint32_t width, uint32_t height);
void sr_frame_free(SrFrame *frame);
void sr_frame_clear(SrFrame *frame, SrColor color);
SrStatus sr_composite_scene(SrScene *scene, double time, SrFrame *frame,
                            SrDiagnostics *diag);
SrColor sr_blend_pixel(SrColor backdrop, SrColor source, double opacity,
                       SrBlendMode mode, bool linear_light);
SrColor sr_blend_pixel_space(SrColor backdrop, SrColor source, double opacity,
                             SrBlendMode mode, bool linear_light,
                             SrColorSpace color_space);

#endif
