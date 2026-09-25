#ifndef SCENE_RENDER_PHYSICS_H
#define SCENE_RENDER_PHYSICS_H

#include "scene_render/diagnostics.h"
#include "scene_render/scene.h"

SrStatus sr_physics_prepare(SrScene *scene, SrDiagnostics *diag);
bool sr_physics_pose(const SrScene *scene, const SrNode *node, double time,
                     double *x, double *y, double *rotation);

#endif
