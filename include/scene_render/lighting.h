#ifndef SCENE_RENDER_LIGHTING_H
#define SCENE_RENDER_LIGHTING_H

#include "scene_render/compositor.h"

/* The 3D pass supersampling factor (project antialias3d, 1..4). */
int sr_lighting_samples(const SrScene *scene);

SrStatus sr_lighting_render(SrScene *scene, double time, SrFrame *frame,
                            SrDiagnostics *diag);
/* As sr_lighting_render, depth testing against and writing into `depth`
 * (sized frame x sr_lighting_samples) instead of a private buffer. The
 * caller clears it; NULL behaves exactly like sr_lighting_render. */
SrStatus sr_lighting_render_depth(SrScene *scene, double time, SrFrame *frame,
                                  SrDepthBuffer *depth, SrDiagnostics *diag);

/* The 3D pass drawn one object at a time (depth cards interleave objects
 * with 2D cards, see src/compositor.c). begin evaluates lights and shadow
 * maps; draw_object renders object `index` (with its point-light blob when
 * `blob`) into the pass target, deferring translucent samples; flush sorts
 * and blends these samples before resolving what was drawn since the
 * last flush over `frame` or, with `whole_frame`, the entire supersampled
 * buffer. Consecutive objects within each batch have per-sample visibility
 * independent of submission order. begin sets *pass to
 * NULL, with SR_OK, when the scene has no 3D objects. */
typedef struct SrLightingPass SrLightingPass;
SrStatus sr_lighting_begin(SrScene *scene, double time, SrFrame *frame,
                           SrDepthBuffer *depth, SrDiagnostics *diag,
                           SrLightingPass **pass);
void sr_lighting_draw_blobs(SrLightingPass *pass);
SrStatus sr_lighting_draw_object(SrLightingPass *pass, size_t index, bool blob);
void sr_lighting_flush(SrLightingPass *pass, bool whole_frame);
void sr_lighting_end(SrLightingPass *pass);
/* View depth of object `index`'s center (the sort key among cards). */
double sr_lighting_object_depth(const SrScene *scene, size_t index, double time);

#endif
