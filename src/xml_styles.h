/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SCENE_RENDER_XML_STYLES_H
#define SCENE_RENDER_XML_STYLES_H

#include "scene_render/diagnostics.h"
#include "scene_render/scene.h"

#include <libxml/tree.h>

/* Copies tokens from an already validated DOM into scene-owned storage. */
SrStatus sr_xml_prepare_styles(const xmlDoc *doc, SrScene *scene,
                                SrDiagnostics *diag);

#endif
