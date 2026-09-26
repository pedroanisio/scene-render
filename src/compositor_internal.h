/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SR_COMPOSITOR_INTERNAL_H
#define SR_COMPOSITOR_INTERNAL_H

#include "scene_render/compositor.h"
#include "scene_render/card.h"
#include "scene_render/lighting.h"
#include "length_frame.h"

/* Shared frame-local values for coverage, captures and adjustments. These
 * descriptors borrow their pointer fields; their owning render or queue
 * must keep the referenced storage alive until the last draw completes. */

typedef struct {
    int x0, y0, x1, y1;
} SrClip;

/* Mask geometry evaluated at the current time. */
typedef struct {
    SrMaskType type;
    bool invert;
    double x, y, width, height, radius;
} SrMaskEval;

/* The masks of one node, evaluated in that node's inverse world transform.
 * Links chain outward so coverage is the product over the whole chain. */
typedef struct SrMaskLink {
    const SrMaskEval *masks;
    size_t count;
    SrMat3 inverse;
    double aa;
    const struct SrMaskLink *parent;
} SrMaskLink;

typedef struct {
    float *px;
    uint32_t width, height;
    SrGroupBuffer *buffer;  /* non-NULL when drawing into an isolated group */
} SrTarget;

/* Per-sample depth test of a card composite (SR_OP_BUFFER): a sample is
 * visible where the card plane lies inside [near, far] and not behind the
 * shared depth buffer; `write` stores the card depth where the composited
 * alpha reaches 0.5 (only for composites straight into the frame). The
 * pose is held by value so a queued composite does not point into its
 * caller's stack frame (the view outlives every queued op: the queue is
 * flushed before a render returns). */
typedef struct {
    SrDepthBuffer *depth;       /* NULL: near/far clipping only */
    const SrCardView *view;
    SrCardPose pose;
    bool write;
} SrCardTest;

/* The scene and evaluated geometry are borrowed and read-only during drawing.
 * The compositor, diagnostics and lighting pass are render-owned mutable
 * state. A copied context must propagate the root lighting handoff when it
 * consumes that pass; copies do not transfer ownership of any pointer. */
typedef struct {
    SrCompositor *compositor;
    SrScene *scene;
    SrDiagnostics *diag;
    double time;
    const SrCardView *view;     /* camera state for depth cards */
    const SrNode *card_node;    /* card being drawn as content: normal blend */
    double particle_scale;      /* particle radius factor inside cards */
    SrLightingPass *lighting;   /* 3D objects still to interleave, or NULL */
    const SrLengthFrame *lengths; /* borrowed, including inside card buffers */
    bool skewed;                /* this transform chain uses nonzero skew */
} SrDrawContext;

#endif
