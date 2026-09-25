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

#include <limits.h>
#include <stdio.h>
#include <string.h>

/* 2.9.14 is the oldest release with the upstream fixes for external
 * parameter entities being loaded while substitution is off; the build
 * files require it too. */
#if LIBXML_VERSION < 20914
#error "libxml2 >= 2.9.14 is required"
#endif

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
    bool out_of_memory;
} SchemaSink;

static size_t refused_loads;

/* Installed for the whole libxml2 pass: nothing is ever fetched, whatever
 * the document or the parser options ask for. */
static xmlParserInputPtr refuse_external_entity(const char *url, const char *id,
                                                xmlParserCtxtPtr context) {
    (void)url;
    (void)id;
    (void)context;
    ++refused_loads;
    return NULL;
}

size_t sr_xml_schema_refused_loads(void) {
    return refused_loads;
}

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
    if (error->code == XML_ERR_NO_MEMORY) {
        sink->out_of_memory = true;
        return;
    }
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

/* Deepest element nesting of the tree (root = 1), walked iteratively. */
static size_t document_depth(xmlDocPtr doc) {
    size_t depth = 0, deepest = 0;
    xmlNodePtr node = xmlDocGetRootElement(doc);
    if (node) depth = deepest = 1;
    while (node) {
        xmlNodePtr child = node->children;
        while (child && child->type != XML_ELEMENT_NODE) child = child->next;
        if (child) {
            node = child;
            if (++depth > deepest) deepest = depth;
            continue;
        }
        while (node) {
            xmlNodePtr next = node->next;
            while (next && next->type != XML_ELEMENT_NODE) next = next->next;
            if (next) {
                node = next;
                break;
            }
            node = node->parent;
            if (!node || node->type != XML_ELEMENT_NODE) {
                node = NULL;
                break;
            }
            --depth;
        }
    }
    return deepest;
}

static SrStatus validate_document(xmlDocPtr doc, SrDiagnostics *diag) {
    SchemaSink sink = {diag, 0, false};
    xmlSchemaParserCtxtPtr parser =
        xmlSchemaNewMemParserCtxt((const char *)sr_schema_xsd, (int)sr_schema_xsd_len);
    if (!parser) return SR_ERR_MEMORY;
    xmlSchemaSetParserStructuredErrors(parser, on_schema_error, &sink);
    xmlSchemaPtr schema = xmlSchemaParse(parser);
    xmlSchemaFreeParserCtxt(parser);
    if (sink.out_of_memory) {
        xmlSchemaFree(schema);
        return SR_ERR_MEMORY;
    }
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
    if (sink.out_of_memory) return SR_ERR_MEMORY;
    if (rc == 0 && !sink.errors) return SR_OK;
    if (!sink.errors)
        sr_diag_error(diag, 0, NULL, NULL, "XSD validation failed (libxml2 code %d)", rc);
    return SR_ERR_XML;
}

SrStatus sr_xml_schema_check(const char *data, size_t size, const char *name,
                             SrDiagnostics *diag, SrSchemaDeferral *deferral) {
    *deferral = (SrSchemaDeferral){0};
    if (!data || size > (size_t)INT_MAX) return SR_ERR_ARGUMENT;
    xmlExternalEntityLoader previous = xmlGetExternalEntityLoader();
    xmlSetExternalEntityLoader(refuse_external_entity);
    xmlParserCtxtPtr context = xmlNewParserCtxt();
    if (!context) {
        xmlSetExternalEntityLoader(previous);
        return SR_ERR_MEMORY;
    }
    /* The caller's bytes, decoded as UTF-8 whatever the declaration says:
     * exactly what the Expat pass reads. */
    xmlDocPtr doc = xmlCtxtReadMemory(context, data, (int)size, name, "UTF-8",
                                      parse_options());
    SrStatus status = SR_OK;
    if (!doc) {
        const xmlError *error = xmlCtxtGetLastError(context);
        if (error && error->code == XML_ERR_NO_MEMORY) {
            status = SR_ERR_MEMORY;
        } else {
            /* Not well-formed (or over libxml2's depth limit): the Expat
             * pass reports it in its usual words; this message is the
             * fallback should Expat accept. */
            deferral->deferred = true;
            deferral->line = error && error->line > 0 ? (size_t)error->line : 0;
            snprintf(deferral->message, sizeof(deferral->message), "XML syntax: %s",
                     error && error->message ? error->message : "not well-formed");
            size_t length = strlen(deferral->message);
            while (length && deferral->message[length - 1] == '\n')
                deferral->message[--length] = '\0';
        }
    } else if (doc->intSubset || doc->extSubset) {
        /* DOCTYPE: rejected by the Expat pass with its own diagnostic. */
        deferral->deferred = true;
        snprintf(deferral->message, sizeof(deferral->message),
                 "document type declarations are not allowed");
    } else if (document_depth(doc) > SR_XML_MAX_DEPTH) {
        deferral->deferred = true;
        snprintf(deferral->message, sizeof(deferral->message),
                 "element nesting exceeds %d levels", SR_XML_MAX_DEPTH);
    } else {
        status = validate_document(doc, diag);
    }
    xmlFreeDoc(doc);
    xmlFreeParserCtxt(context);
    xmlSetExternalEntityLoader(previous);
    return status;
}
