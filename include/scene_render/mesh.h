#ifndef SCENE_RENDER_MESH_H
#define SCENE_RENDER_MESH_H

#include "scene_render/diagnostics.h"
#include "scene_render/scene.h"

SrStatus sr_mesh_load_obj(const char *path, SrMesh **mesh,
                          size_t source_line, SrDiagnostics *diag);

#endif
