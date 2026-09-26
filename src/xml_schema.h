#ifndef SCENE_RENDER_XML_SCHEMA_H
#define SCENE_RENDER_XML_SCHEMA_H

#include "scene_render/diagnostics.h"
#include "scene_render/common.h"

/* Set when the document was not schema-checked because the Expat pass owns
 * the diagnostic: the file is not well-formed, nests too deeply or has a
 * DOCTYPE. `message`
 * (at `line`) is reported only if Expat unexpectedly accepts the file. */
typedef struct {
    bool deferred;
    size_t line;
    char message[512];
} SrSchemaDeferral;

/* Deepest element nesting either parser accepts (the root is level 1). */
#define SR_XML_MAX_DEPTH 256
/* Largest scene file the loader reads. */
#define SR_XML_MAX_BYTES ((size_t)64 << 20)

/* Parses the `size` bytes at `data` (the whole scene file, read once by the
 * loader; `name` labels libxml2's own messages) with libxml2 as UTF-8 (no
 * network, no external entities or DTD loading, no entity substitution;
 * an external entity loader that refuses every load is installed for the
 * whole call) and validates it against the embedded XSD. Every schema
 * error becomes a diagnostic "file:line: error: <element> @attribute:
 * message"; returns SR_ERR_XML when there was any, SR_ERR_MEMORY when
 * libxml2 ran out of memory. */
SrStatus sr_xml_schema_check(const char *data, size_t size, const char *name,
                             SrDiagnostics *diag, SrSchemaDeferral *deferral);

/* Same checks, optionally listing every unsupported construct. */
SrStatus sr_xml_schema_check_profile(const char *data, size_t size,
                                    const char *name, SrDiagnostics *diag,
                                    SrSchemaDeferral *deferral, bool report_all);

/* Loads the refusing loader has turned away since the process started
 * (tests: proof that nothing was fetched is that nothing was attempted, or
 * that every attempt ended here). */
size_t sr_xml_schema_refused_loads(void);

#endif
