/* SPDX-License-Identifier: Apache-2.0 */
#include "xml_capabilities.h"

#include <ctype.h>
#include <string.h>
#include <stdlib.h>

typedef enum {
    SR_CAP_ATTRIBUTE, SR_CAP_ELEMENT, SR_CAP_FORM, SR_CAP_OCCURRENCE, SR_CAP_VALUE
} CapabilityKind;

typedef struct {
    CapabilityKind kind;
    const char *host;
    const char *name;
    const char *value;
    unsigned minimum_version;
    bool implemented;
} Capability;

static const Capability capabilities[] = {
#include "xml_capabilities_data.inc"
};

/* Rows are sorted by host, kind, name and value. A NULL value searches for
 * any row of that name (element and attribute rows have unique names). */
static const Capability *find_capability(CapabilityKind kind, const char *host,
                                         const char *name, const char *value) {
    size_t lo = 0, hi = sizeof(capabilities) / sizeof(capabilities[0]);
    while (lo < hi) {
        size_t mid = lo + (hi - lo) / 2;
        const Capability *row = &capabilities[mid];
        int order = strcmp(host, row->host);
        if (!order) order = (int)kind - (int)row->kind;
        if (!order) order = strcmp(name, row->name);
        if (!order && value) order = strcmp(value, row->value);
        if (!order) return row;
        if (order < 0) hi = mid;
        else lo = mid + 1;
    }
    return NULL;
}

static const char *attribute_value(const xmlAttr *attribute) {
    return attribute->children && attribute->children->content
        ? (const char *)attribute->children->content : "";
}

static const char *property(const xmlNode *node, const char *name) {
    for (const xmlAttr *attr = node->properties; attr; attr = attr->next)
        if (!strcmp((const char *)attr->name, name)) return attribute_value(attr);
    return NULL;
}

static void diagnostic(SrDiagnostics *diag, const xmlNode *node,
                        const char *attribute, const char *value,
                        bool version_error) {
    const char *prefix = version_error ? "requires version=\"1.1\""
                                       : "unsupported in this build";
    size_t line = (size_t)xmlGetLineNo(node);
    if (attribute) {
        /* Reports are often pasted into logs. URI query tokens, userinfo
         * and credential profiles need no value echo to identify a use. */
        if (strstr(value, "://") || !strcmp(attribute, "credentials"))
            value = "[redacted]";
        sr_diag_error(diag, line, (const char *)node->name, attribute,
                      "%s: <%s %s=\"%.160s\">", prefix, node->name,
                      attribute, value);
    } else {
        sr_diag_error(diag, line, (const char *)node->name, NULL,
                      "%s: <%s>", prefix, node->name);
    }
}

static bool check_row(const Capability *row, unsigned version,
                       const xmlNode *node, const char *attribute,
                       const char *value, SrDiagnostics *diag) {
    if (row && row->minimum_version <= version && row->implemented) return true;
    if (row && row->kind == SR_CAP_ATTRIBUTE && !strcmp(row->value, "xs:anyURI"))
        value = "[redacted]";
    diagnostic(diag, node, attribute, value,
               row && row->minimum_version > version);
    return false;
}

static const char *value_form(const Capability *attribute, const char *value) {
    if (!attribute) return NULL;
    const char *type = attribute->value;
    bool key = !strcmp(attribute->host, "keyType") && !strcmp(attribute->name, "value");
    if (!strcmp(type, "lengthType") || !strcmp(type, "positiveLengthType") || key) {
        size_t size = strlen(value);
        static const char *const suffixes[] = {"%", "vw", "vh", "vmin", "vmax"};
        for (size_t i = 0; i < sizeof(suffixes) / sizeof(suffixes[0]); ++i) {
            size_t suffix_size = strlen(suffixes[i]);
            if (size >= suffix_size && !strcmp(value + size - suffix_size, suffixes[i]))
                return "relative-length";
        }
    }
    if (!strcmp(type, "colorType") || !strcmp(type, "paintType") || key) {
        if (!strncmp(value, "var(", 4)) return "token";
        if (!strncmp(value, "url(", 4)) return "paint-reference";
    }
    return NULL;
}

/* The schema pass already bounds depth at SR_XML_MAX_DEPTH. Unsupported
 * parents still recurse during a report so nested uses are not hidden. */
static bool check_node(const xmlNode *node, const char *host, unsigned version,
                        bool report_all, SrDiagnostics *diag) {
    bool ok = true;
    for (const xmlAttr *attr = node->properties; attr; attr = attr->next) {
        const char *name = (const char *)attr->name;
        const char *value = attribute_value(attr);
        const Capability *row = find_capability(SR_CAP_ATTRIBUTE, host, name, NULL);
        bool accepted = check_row(row, version, node, name, value, diag);
        if (accepted && find_capability(SR_CAP_VALUE, host, name, NULL)) {
            const Capability *choice = find_capability(SR_CAP_VALUE, host, name, value);
            if (!choice) {
                sr_diag_error(diag, (size_t)xmlGetLineNo(node),
                              (const char *)node->name, name,
                              "unknown value for this attribute");
                accepted = false;
            } else {
                accepted = check_row(choice, version, node, name, value, diag);
            }
        }
        const char *form = accepted ? value_form(row, value) : NULL;
        if (form) {
            const Capability *choice = find_capability(SR_CAP_FORM, host, name, form);
            accepted = check_row(choice, version, node, name, value, diag);
        }
        ok = accepted && ok;
        if (!ok && !report_all) return false;
    }
    /* There are at most 40 distinct child kinds in the contract. Bound
     * counters independently of document size when reporting every use. */
    enum { SR_XML_MAX_OCCURRENCE_RULES = 64 };
    const Capability *limits[SR_XML_MAX_OCCURRENCE_RULES] = {0};
    size_t counts[SR_XML_MAX_OCCURRENCE_RULES] = {0};
    size_t limit_count = 0;
    for (const xmlNode *child = node->children; child; child = child->next) {
        if (child->type != XML_ELEMENT_NODE) continue;
        const char *name = (const char *)child->name;
        const Capability *row = find_capability(SR_CAP_ELEMENT, host, name, NULL);
        bool accepted = check_row(row, version, child, NULL, NULL, diag);
        const Capability *limit = find_capability(SR_CAP_OCCURRENCE, host, name, NULL);
        if (limit) {
            size_t slot = 0;
            while (slot < limit_count && limits[slot] != limit) ++slot;
            if (slot == limit_count) {
                if (limit_count == SR_XML_MAX_OCCURRENCE_RULES) {
                    diagnostic(diag, child, NULL, NULL, false);
                    return false;
                }
                limits[limit_count++] = limit;
            }
            if (++counts[slot] > (size_t)strtoul(limit->value, NULL, 10)) {
                diagnostic(diag, child, NULL, NULL, false);
                accepted = false;
            }
        }
        ok = accepted && ok;
        if (!ok && !report_all) return false;
        if (row) ok = check_node(child, row->value, version, report_all, diag) && ok;
        if (!ok && !report_all) return false;
    }
    return ok;
}

SrStatus sr_xml_check_capabilities(xmlDocPtr doc, SrDiagnostics *diag,
                                    bool report_all) {
    const xmlNode *root = xmlDocGetRootElement(doc);
    const char *text = root ? property(root, "version") : NULL;
    if (!text) return SR_ERR_XML;
    unsigned version = !strcmp(text, "1.1") ? 11 : 10;
    return check_node(root, "scene", version, report_all, diag)
        ? SR_OK : SR_ERR_XML;
}
