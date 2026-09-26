/* SPDX-License-Identifier: Apache-2.0 */
#include "xml_internal.h"
#include "metadata.h"

#include <stdio.h>
#include <string.h>

static void add_entry(ParseContext *ctx, const char *element,
                       const char *attribute, const char *name, const char *value) {
    SrScene *scene = ctx->scene;
    char message[256];
    const char *error = NULL;
    if (!name[0] || strlen(name) > SR_MAX_METADATA_NAME) {
        snprintf(message, sizeof(message), "metadata name must have 1..%u bytes",
                 SR_MAX_METADATA_NAME);
        error = message;
    } else if (strlen(value) > SR_MAX_METADATA_VALUE) {
        snprintf(message, sizeof(message), "metadata value exceeds %u bytes",
                 SR_MAX_METADATA_VALUE);
        error = message;
        if (!strcmp(element, "meta")) attribute = "value";
    } else if (scene->metadata_count == SR_MAX_METADATA_ENTRIES) {
        snprintf(message, sizeof(message), "metadata entry count exceeds %u",
                 SR_MAX_METADATA_ENTRIES);
        error = message;
    }
    for (size_t i = 0; !error && i < scene->metadata_count; ++i) {
        if (sr_metadata_name_equal(scene->metadata[i].name, name)) {
            snprintf(message, sizeof(message), "duplicate metadata name '%s'", name);
            error = message;
        }
    }
    if (error) {
        sr_xml_fail(ctx, element, attribute, error);
        return;
    }
    if (scene->metadata_count == scene->metadata_capacity) {
        size_t capacity = scene->metadata_capacity ? scene->metadata_capacity * 2 : 16;
        if (capacity > SR_MAX_METADATA_ENTRIES) capacity = SR_MAX_METADATA_ENTRIES;
        SrMetadataEntry *grown = sr_realloc(scene->metadata, capacity * sizeof(*grown));
        if (!grown) goto oom;
        scene->metadata = grown;
        scene->metadata_capacity = capacity;
    }
    SrMetadataEntry *entry = &scene->metadata[scene->metadata_count++];
    *entry = (SrMetadataEntry){.source_line = sr_xml_line(ctx)};
    entry->name = sr_strdup(name);
    entry->value = sr_strdup(value);
    if (entry->name && entry->value) return;
oom:
    ctx->out_of_memory = true;
    sr_xml_fail(ctx, element, attribute, "out of memory loading metadata");
}

void sr_xml_start_metadata(ParseContext *ctx, const XML_Char **attrs) {
    const char *const allowed[] = {
        "title", "author", "description", "keywords", "copyright", "revision",
        "created", "modified", "generator", "language"
    };
    if (!sr_xml_attrs_allowed(ctx, "metadata", attrs, allowed,
                              sizeof(allowed) / sizeof(allowed[0]))) return;
    for (size_t i = 0; attrs[i] && !ctx->failed; i += 2)
        add_entry(ctx, "metadata", attrs[i], attrs[i], attrs[i + 1]);
}

void sr_xml_start_meta(ParseContext *ctx, const XML_Char **attrs) {
    const char *const allowed[] = {"name", "value"};
    if (!sr_xml_attrs_allowed(ctx, "meta", attrs, allowed, 2)) return;
    const char *name = sr_xml_attr(attrs, "name");
    const char *value = sr_xml_attr(attrs, "value");
    if (!name) SR_XML_FAIL_RETURN(ctx, "meta", "name", "missing required name");
    if (!value) SR_XML_FAIL_RETURN(ctx, "meta", "value", "missing required value");
    add_entry(ctx, "meta", "name", name, value);
}
