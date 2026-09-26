/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SCENE_RENDER_XML_CAPABILITIES_H
#define SCENE_RENDER_XML_CAPABILITIES_H

#include "scene_render/diagnostics.h"
#include "scene_render/common.h"
#include <libxml/tree.h>

/* This pass runs only on a validated, bounded DOM; it never mutates it.
 * The caller owns the document. report_all lists every unsupported use. */
SrStatus sr_xml_check_capabilities(xmlDocPtr doc, SrDiagnostics *diag,
                                    bool report_all);

#endif
