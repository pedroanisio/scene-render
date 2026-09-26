/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SCENE_RENDER_METADATA_H
#define SCENE_RENDER_METADATA_H

#include "scene_render/scene.h"
#include "scene_render/diagnostics.h"
#include <libavformat/avformat.h>

bool sr_metadata_name_equal(const char *left, const char *right);
SrStatus sr_metadata_validate(const SrScene *scene, const char *container,
                               SrDiagnostics *diag);
SrStatus sr_metadata_apply(const SrScene *scene, AVFormatContext *format,
                            const char *container, SrDiagnostics *diag);
SrStatus sr_metadata_check(const SrScene *scene, const AVFormatContext *format,
                            const char *container, SrDiagnostics *diag);

#endif
