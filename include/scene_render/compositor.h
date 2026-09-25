#ifndef SCENE_RENDER_COMPOSITOR_H
#define SCENE_RENDER_COMPOSITOR_H

#include "scene_render/diagnostics.h"
#include "scene_render/raster.h"
#include "scene_render/scene.h"

/* A float premultiplied RGBA frame in the project blend space (see
 * SrImage). A 3840x2160 frame is 126.6 MiB. */
typedef struct {
    uint32_t width;
    uint32_t height;
    float *px;
} SrFrame;

/* An isolated-group render target, pooled per nesting depth. Only the dirty
 * rectangle [x0,x1) x [y0,y1) can hold non-zero pixels. */
typedef struct {
    SrFrame frame;
    int x0, y0, x1, y1;
} SrGroupBuffer;

/* Per-render state kept across frames: the isolated-group buffer pool
 * (allocated lazily once per depth and reused, never per frame) and the
 * worker count used for row-parallel draws. */
typedef struct {
    SrGroupBuffer **pool;
    size_t pool_count;
    unsigned threads;
} SrCompositor;

SrStatus sr_frame_init(SrFrame *frame, uint32_t width, uint32_t height);
void sr_frame_free(SrFrame *frame);
/* Fills every pixel with the premultiplied blend-space color. */
void sr_frame_clear(SrFrame *frame, const float color[4], unsigned threads);

void sr_compositor_init(SrCompositor *compositor, unsigned threads);
void sr_compositor_free(SrCompositor *compositor);
/* Draws the scene graph at `time` over the current contents of `frame`. */
SrStatus sr_compositor_render(SrCompositor *compositor, SrScene *scene,
                              double time, SrFrame *frame,
                              SrDiagnostics *diag);
/* One-shot single-threaded convenience wrapper around a temporary
 * compositor. */
SrStatus sr_composite_scene(SrScene *scene, double time, SrFrame *frame,
                            SrDiagnostics *diag);

#endif
