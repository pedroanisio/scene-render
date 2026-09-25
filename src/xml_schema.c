/* Runtime XSD validation of scene documents with libxml2 against the schema
 * embedded at build time (schema/scene-v1.xsd), before the Expat loader
 * applies its semantic checks. */
#include "scene_render/xml.h"

#include "schema_data.h"
#include "xml_schema.h"

#include <libxml/parser.h>
#include <libxml/tree.h>
#include <libxml/xmlerror.h>
#include <libxml/xmlschemas.h>
#include <libxml/xmlversion.h>

#include <stdio.h>
#include <string.h>

/* libxml2 2.12 made the structured error callback take a const error. */
#if LIBXML_VERSION >= 21200
typedef const xmlError *SrXmlErrorArg;
#else
typedef xmlErrorPtr SrXmlErrorArg;
#endif

const char *sr_scene_schema_text(size_t *length) {
    if (length) *length = sr_schema_xsd_len;
    return (const char *)sr_schema_xsd;
}

typedef struct {
    SrDiagnostics *diag;
    size_t errors;
} SchemaSink;

/* Schema attribute errors are raised against the owning element and name
 * the attribute only in the message ("Element 'shape', attribute 'foo':
 * ..."). Copies the quoted name after "attribute '" into out. */
static bool attribute_from_message(const char *message, char *out, size_t size) {
    static const char key[] = "attribute '";
    const char *at = message ? strstr(message, key) : NULL;
    if (!at) return false;
    at += sizeof(key) - 1;
    const char *end = strchr(at, '\'');
    size_t length = end ? (size_t)(end - at) : 0;
    if (!length || length >= size) return false;
    memcpy(out, at, length);
    out[length] = '\0';
    return true;
}

static void on_schema_error(void *user, SrXmlErrorArg error) {
    SchemaSink *sink = user;
    if (!error) return;
    char message[512];
    snprintf(message, sizeof(message), "%s",
             error->message ? error->message : "schema violation");
    size_t length = strlen(message);
    while (length && (message[length - 1] == '\n' || message[length - 1] == ' '))
        message[--length] = '\0';
    const xmlNode *node = error->node;
    const char *element = NULL;
    char attribute[128];
    bool has_attribute = attribute_from_message(message, attribute,
                                                sizeof(attribute));
    size_t line = error->line > 0 ? (size_t)error->line : 0;
    if (node && node->type == XML_ATTRIBUTE_NODE) {
        if (!has_attribute && node->name &&
            strlen((const char *)node->name) < sizeof(attribute)) {
            snprintf(attribute, sizeof(attribute), "%s", (const char *)node->name);
            has_attribute = true;
        }
        node = node->parent;
    }
    if (node && node->type == XML_ELEMENT_NODE) {
        element = (const char *)node->name;
        if (!line) {
            long number = xmlGetLineNo(node);
            if (number > 0) line = (size_t)number;
        }
    }
    if (error->level == XML_ERR_WARNING) {
        sr_diag_warning(sink->diag, line, element,
                        has_attribute ? attribute : NULL, "%s", message);
        return;
    }
    sr_diag_error(sink->diag, line, element, has_attribute ? attribute : NULL,
                  "%s", message);
    ++sink->errors;
}

/* Network access, external entities/DTDs and entity substitution stay off;
 * big line numbers keep diagnostics exact past line 65535. */
static int parse_options(void) {
    int options = XML_PARSE_NONET | XML_PARSE_NOERROR | XML_PARSE_NOWARNING |
                  XML_PARSE_BIG_LINES;
#if LIBXML_VERSION >= 21300
    options |= XML_PARSE_NO_XXE;
#endif
    return options;
}

static SrStatus validate_document(xmlDocPtr doc, SrDiagnostics *diag) {
    SchemaSink sink = {diag, 0};
    xmlSchemaParserCtxtPtr parser =
        xmlSchemaNewMemParserCtxt((const char *)sr_schema_xsd, (int)sr_schema_xsd_len);
    if (!parser) return SR_ERR_MEMORY;
    xmlSchemaSetParserStructuredErrors(parser, on_schema_error, &sink);
    xmlSchemaPtr schema = xmlSchemaParse(parser);
    xmlSchemaFreeParserCtxt(parser);
    if (!schema) {
        if (!sink.errors)
            sr_diag_error(diag, 0, NULL, NULL, "embedded XSD could not be compiled");
        return SR_ERR_XML;
    }
    xmlSchemaValidCtxtPtr valid = xmlSchemaNewValidCtxt(schema);
    if (!valid) {
        xmlSchemaFree(schema);
        return SR_ERR_MEMORY;
    }
    xmlSchemaSetValidStructuredErrors(valid, on_schema_error, &sink);
    int rc = xmlSchemaValidateDoc(valid, doc);
    xmlSchemaFreeValidCtxt(valid);
    xmlSchemaFree(schema);
    if (rc == 0 && !sink.errors) return SR_OK;
    if (!sink.errors)
        sr_diag_error(diag, 0, NULL, NULL, "XSD validation failed (libxml2 code %d)", rc);
    return SR_ERR_XML;
}

SrStatus sr_xml_schema_check(const char *path, SrDiagnostics *diag,
                             SrSchemaDeferral *deferral) {
    *deferral = (SrSchemaDeferral){0};
    xmlParserCtxtPtr context = xmlNewParserCtxt();
    if (!context) return SR_ERR_MEMORY;
    xmlDocPtr doc = xmlCtxtReadFile(context, path, NULL, parse_options());
    SrStatus status = SR_OK;
    if (!doc) {
        /* Not well-formed: the Expat pass reports the syntax error in its
         * usual words; this message is the fallback should Expat accept. */
        const xmlError *error = xmlCtxtGetLastError(context);
        deferral->deferred = true;
        deferral->line = error && error->line > 0 ? (size_t)error->line : 0;
        snprintf(deferral->message, sizeof(deferral->message), "XML syntax: %s",
                 error && error->message ? error->message : "not well-formed");
        size_t length = strlen(deferral->message);
        while (length && deferral->message[length - 1] == '\n')
            deferral->message[--length] = '\0';
    } else if (doc->intSubset || doc->extSubset) {
        /* DOCTYPE: rejected by the Expat pass with its own diagnostic. */
        deferral->deferred = true;
        snprintf(deferral->message, sizeof(deferral->message),
                 "document type declarations are not allowed");
    } else {
        status = validate_document(doc, diag);
    }
    xmlFreeDoc(doc);
    xmlFreeParserCtxt(context);
    return status;
}
