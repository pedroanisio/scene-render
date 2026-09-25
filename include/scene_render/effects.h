#ifndef SCENE_RENDER_EFFECTS_H
#define SCENE_RENDER_EFFECTS_H

#include "scene_render/compositor.h"

SrStatus sr_effects_apply(const SrScene *scene, double time, SrFrame *frame,
                          SrDiagnostics *diag);

#endif
