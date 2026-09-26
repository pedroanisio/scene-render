/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SCENE_RENDER_COMPOSITING_H
#define SCENE_RENDER_COMPOSITING_H

#include "scene_render/diagnostics.h"
#include "scene_render/scene.h"

/* Required before evaluating a programmatic scene using new color blends,
 * skew or later 1.1 compositing features. XML loading prepares automatically.
 * Owns an immutable plan through scene; sr_scene_free releases it. Preparation
 * checks all authored nodes, including inactive content. Failure discards the
 * old plan and leaves evaluation disabled until preparation succeeds.
 *
 * Invalidate BEFORE changing authored fields or tracks, then prepare again.
 * Direct assignments are not automatically detected. Neither mutation nor
 * preparation may overlap rendering. Invalidation also clears prepared
 * emitters' rate caches; successful preparation clears newly admitted caches
 * before publishing their capacity reservation. Asset/physics caches remain governed by
 * their own preparation APIs. Invalid ownership graphs must be repaired by
 * the caller before ordinary recursive scene destruction. */
SrStatus sr_scene_prepare_compositing(SrScene *scene, SrDiagnostics *diag);
void sr_scene_invalidate_compositing(SrScene *scene);

#endif
