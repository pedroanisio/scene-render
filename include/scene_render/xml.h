#ifndef SCENE_RENDER_XML_H
#define SCENE_RENDER_XML_H

#include "scene_render/diagnostics.h"
#include "scene_render/scene.h"

SrStatus sr_scene_load_xml(const char *path, SrScene *scene,
                           SrDiagnostics *diag);

#endif
