/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SCENE_RENDER_LENGTH_FRAME_H
#define SCENE_RENDER_LENGTH_FRAME_H

#include "scene_render/diagnostics.h"
#include "scene_render/scene.h"

#define SR_MAX_LENGTH_NODES 65536u
#define SR_MAX_LENGTH_MASKS 262144u
#define SR_MAX_LENGTH_DEPTH 256u
#define SR_MAX_LENGTH_CONSTRAINTS 65536u

typedef struct {
    double x, y, width, height;
} SrMaskGeometry;

typedef struct {
    const SrNode *node;        /* borrowed; scene must outlive use of this frame */
    double x, y, anchor_x, anchor_y;
    SrLengthBox box;
    size_t mask_offset;
    bool base_node, base_anchor, base_box, base_scope;
} SrNodeGeometry;

/* Zero-initialize; owns reusable arrays, freed by sr_length_frame_free.
 * Preparation replaces all used entries, including after a failed prepare.
 * Once prepared, readers may share the frame until its next preparation. */
typedef struct SrLengthFrame {
    SrNodeGeometry *nodes;
    size_t node_count, node_capacity;
    SrMaskGeometry *masks;
    size_t mask_count, mask_capacity;
} SrLengthFrame;

SrStatus sr_length_frame_prepare(SrLengthFrame *frame, const SrScene *scene,
                                 double time, bool base_pose, SrDiagnostics *diag);
void sr_length_frame_free(SrLengthFrame *frame);

/* Requires a successful preparation for this scene; NULL denotes legacy
 * geometry and lets callers preserve the old scalar evaluation path. */
static inline const SrNodeGeometry *sr_length_node(const SrLengthFrame *frame,
                                                   const SrNode *node) {
    return frame ? &frame->nodes[node->order] : NULL;
}

#endif
