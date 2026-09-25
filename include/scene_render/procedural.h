#ifndef SCENE_RENDER_PROCEDURAL_H
#define SCENE_RENDER_PROCEDURAL_H

#include "scene_render/diagnostics.h"
#include "scene_render/scene.h"

/* Rasterizes a vector asset into asset->decoded (blend space). */
SrStatus sr_procedural_asset(const SrProject *project, SrAsset *asset,
                             SrDiagnostics *diag);

#endif
