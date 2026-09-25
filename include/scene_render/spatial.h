#ifndef SCENE_RENDER_SPATIAL_H
#define SCENE_RENDER_SPATIAL_H

#include "scene_render/common.h"
#include "scene_render/diagnostics.h"

SrStatus sr_spatial_inject_mp4(const char *path, uint32_t width,
                               uint32_t height, SrDiagnostics *diag);
bool sr_spatial_is_mp4(const char *path);

#endif
