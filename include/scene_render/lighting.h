#ifndef SCENE_RENDER_LIGHTING_H
#define SCENE_RENDER_LIGHTING_H

#include "scene_render/compositor.h"

SrStatus sr_lighting_render(SrScene *scene, double time, SrFrame *frame,
                            SrDiagnostics *diag);

#endif
