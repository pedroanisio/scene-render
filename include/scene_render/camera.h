#ifndef SCENE_RENDER_CAMERA_H
#define SCENE_RENDER_CAMERA_H

#include "scene_render/compositor.h"

SrStatus sr_camera_extract_viewport(const SrScene *scene, double time,
                                    const SrFrame *panorama, SrFrame *viewport,
                                    unsigned threads, SrDiagnostics *diag);

#endif
