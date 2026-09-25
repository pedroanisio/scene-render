#ifndef SCENE_RENDER_PHYSICS_H
#define SCENE_RENDER_PHYSICS_H

#include "scene_render/diagnostics.h"
#include "scene_render/scene.h"

/* Simulates every rigid body (when the scene has <physics>) and every soft
 * body at the fixed step from t = 0 to the scene duration, storing one
 * sample per step on the nodes, or restores them from the physics cache.
 * Deterministic: the result depends only on the scene. */
SrStatus sr_physics_prepare(SrScene *scene, SrDiagnostics *diag);
bool sr_physics_pose(const SrScene *scene, const SrNode *node, double time,
                     double *x, double *y, double *rotation);
/* Soft-body grid offsets of `node` at `time` (rows*cols (dx, dy) pairs in
 * the node's local space), linearly interpolated between fixed samples;
 * false when the node has no simulated soft body. */
bool sr_physics_soft_offsets(const SrScene *scene, const SrNode *node,
                             double time, double *out);

#endif
