#ifndef SCENE_RENDER_ASSETS_H
#define SCENE_RENDER_ASSETS_H

#include "scene_render/diagnostics.h"
#include "scene_render/scene.h"

SrStatus sr_assets_load(SrScene *scene, SrDiagnostics *diag);
void sr_assets_unload(SrScene *scene);
SrImage *sr_asset_get_frame(SrScene *scene, SrAsset *asset, double source_time,
                            SrDiagnostics *diag);

#endif
