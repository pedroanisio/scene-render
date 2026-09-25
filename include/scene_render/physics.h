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

/* Most integration substeps a soft body may take per fixed physics step;
 * XML validation rejects bodies that would need more. */
#define SR_SOFT_MAX_SUBSTEPS 4096

/* Substeps per fixed step the soft-body integrator needs to stay stable
 * for `body` (explicit spring stability omega*h < 0.25 and damping
 * c*h/m < 0.5 over the 8 springs per node, 9 with a rigid body's anchor
 * spring); +inf when not finite. */
double sr_soft_body_substeps(const SrSoftBody *body, bool rigid,
                             double fixed_step);

#endif
