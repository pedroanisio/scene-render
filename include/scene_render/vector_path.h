#ifndef SCENE_RENDER_VECTOR_PATH_H
#define SCENE_RENDER_VECTOR_PATH_H

#include "scene_render/diagnostics.h"
#include "scene_render/scene.h"

SrStatus sr_vector_path_render(const char *path, SrImage *image, SrColor color,
                               size_t source_line, SrDiagnostics *diag);
bool sr_vector_path_valid(const char *path);

#endif
