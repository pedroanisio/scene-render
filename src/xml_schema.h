#ifndef SCENE_RENDER_XML_SCHEMA_H
#define SCENE_RENDER_XML_SCHEMA_H

#include "scene_render/diagnostics.h"
#include "scene_render/common.h"

/* Set when the document was not schema-checked because the Expat pass owns
 * the diagnostic: the file is not well-formed or has a DOCTYPE. `message`
 * (at `line`) is reported only if Expat unexpectedly accepts the file. */
typedef struct {
    bool deferred;
    size_t line;
    char message[512];
} SrSchemaDeferral;

/* Parses `path` with libxml2 (no network, no external entities or DTD
 * loading, no entity substitution) and validates it against the embedded
 * XSD. Every schema error becomes a diagnostic "file:line: error: <element>
 * @attribute: message"; returns SR_ERR_XML when there was any. */
SrStatus sr_xml_schema_check(const char *path, SrDiagnostics *diag,
                             SrSchemaDeferral *deferral);

#endif
