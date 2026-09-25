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

/* Per-sample view depth shared by the 3D pass and depth cards: `samples`
 * x `samples` entries per frame pixel (the antialias3d factor), INFINITY
 * where nothing has been drawn. */
typedef struct {
    double *z;
    uint32_t width, height;     /* frame size x samples */
    int samples;
} SrDepthBuffer;

/* Per-render state kept across frames: the isolated-group buffer pool
 * (allocated lazily once per depth and reused, never per frame), the
 * worker count used for row-parallel draws and, for scenes with depth
 * cards, the shared depth buffer and a second pool for the plane buffers
 * of perspective-warped cards (see sr_compositor_render_scene). Draw ops
 * are recorded in `queue` and executed band by band (every pixel still
 * receives the ops in submission order); it is empty between renders. */
typedef struct SrCompositor {
    SrGroupBuffer **pool;
    size_t pool_count;
    unsigned threads;
    struct SrOpQueue *queue;
    SrDepthBuffer *depth;           /* NULL: cards are not depth tested */
    SrDepthBuffer depth_store;      /* owned by sr_compositor_render_scene */
    struct SrCompositor *plane;     /* lazily allocated plane-buffer pool */
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
/* Draws the 3D pass and the scene graph over `frame`. Without depth cards
 * this is exactly sr_lighting_render followed by sr_compositor_render.
 * With cards, both share a per-sample depth buffer; if the composition has
 * cards as direct children, the 3D objects are drawn one by one among the
 * first run of them in far-to-near order, else before the scene graph. */
SrStatus sr_compositor_render_scene(SrCompositor *compositor, SrScene *scene,
                                    double time, SrFrame *frame,
                                    SrDiagnostics *diag);
/* One-shot single-threaded convenience wrapper around a temporary
 * compositor. */
SrStatus sr_composite_scene(SrScene *scene, double time, SrFrame *frame,
                            SrDiagnostics *diag);

#endif
