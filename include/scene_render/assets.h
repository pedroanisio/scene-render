#ifndef SCENE_RENDER_ASSETS_H
#define SCENE_RENDER_ASSETS_H

#include "scene_render/diagnostics.h"
#include "scene_render/scene.h"

SrStatus sr_assets_load(SrScene *scene, SrDiagnostics *diag);
void sr_assets_unload(SrScene *scene);
/* Sums the decoder statistics of every open video asset: totals are
 * requests, cache hits, frames decoded and seeks. */
void sr_assets_video_stats(const SrScene *scene, size_t *sources,
                           uint64_t totals[4]);
SrImage *sr_asset_get_frame(SrScene *scene, SrAsset *asset, double source_time,
                            SrDiagnostics *diag);

#endif
