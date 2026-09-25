#ifndef SCENE_RENDER_XML_H
#define SCENE_RENDER_XML_H

#include "scene_render/diagnostics.h"
#include "scene_render/scene.h"

/* Loads a scene: the document is first validated against the embedded XSD
 * (libxml2), then built and semantically checked (Expat). Diagnostics name
 * file:line, element and attribute; failures return SR_ERR_XML (SR_ERR_IO
 * when the file cannot be read). */
SrStatus sr_scene_load_xml(const char *path, SrScene *scene,
                           SrDiagnostics *diag);

/* The XSD embedded at build time (schema/scene-v1.xsd), `*length` bytes. */
const char *sr_scene_schema_text(size_t *length);

#endif
