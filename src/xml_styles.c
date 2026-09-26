/* SPDX-License-Identifier: Apache-2.0 */
#include "xml_styles.h"
#include "xml_internal.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static bool token_name_valid(const char *name, size_t size) {
    if (!size || size > SR_MAX_TOKEN_NAME) return false;
    for (size_t i = 0; i < size; ++i) {
        unsigned char c = (unsigned char)name[i];
        if (!((c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') ||
              (c >= '0' && c <= '9') || c == '_' || c == '-'))
            return false;
    }
    return true;
}

/* 0: literal, 1: reference, -1: malformed reference. */
static int token_reference(const char *value, char name[SR_MAX_TOKEN_NAME + 1]) {
    if (strncmp(value, "var(", 4)) return 0;
    size_t size = strlen(value);
    if (size < 8 || strncmp(value, "var(--", 6) || value[size - 1] != ')' ||
        !token_name_valid(value + 6, size - 7))
        return -1;
    memcpy(name, value + 6, size - 7);
    name[size - 7] = '\0';
    return 1;
}

static size_t token_index(const SrScene *scene, const char *name) {
    for (size_t i = 0; i < scene->token_count; ++i)
        if (!strcmp(scene->tokens[i].name, name)) return i;
    return SIZE_MAX;
}

static SrStatus collect_token(const xmlNode *node, SrScene *scene,
                               SrDiagnostics *diag) {
    long number = xmlGetLineNo(node);
    size_t line = number > 0 ? (size_t)number : 0;
    if (scene->token_count == SR_MAX_STYLE_TOKENS) {
        sr_diag_error(diag, line, "token", "name",
                      "style token count exceeds %u", SR_MAX_STYLE_TOKENS);
        return SR_ERR_XML;
    }
    xmlChar *name = xmlGetProp(node, (const xmlChar *)"name");
    xmlChar *value = xmlGetProp(node, (const xmlChar *)"value");
    SrStatus status = SR_OK;
    if (!name || !value) {
        status = SR_ERR_MEMORY;
    } else if (!token_name_valid((const char *)name, strlen((const char *)name))) {
        sr_diag_error(diag, line, "token", "name",
                      "expected 1..%u ASCII letters, digits, '_' or '-'",
                      SR_MAX_TOKEN_NAME);
        status = SR_ERR_XML;
    } else if (strlen((const char *)value) > SR_MAX_TOKEN_VALUE) {
        sr_diag_error(diag, line, "token", "value",
                      "token value exceeds %u bytes", SR_MAX_TOKEN_VALUE);
        status = SR_ERR_XML;
    } else if (token_index(scene, (const char *)name) != SIZE_MAX) {
        sr_diag_error(diag, line, "token", "name", "duplicate token '%s'", name);
        status = SR_ERR_XML;
    }
    if (status == SR_OK && scene->token_count == scene->token_capacity) {
        size_t capacity = scene->token_capacity ? scene->token_capacity * 2 : 16;
        if (capacity > SR_MAX_STYLE_TOKENS) capacity = SR_MAX_STYLE_TOKENS;
        SrStyleToken *grown = sr_realloc(scene->tokens, capacity * sizeof(*grown));
        if (!grown) status = SR_ERR_MEMORY;
        else {
            scene->tokens = grown;
            scene->token_capacity = capacity;
        }
    }
    if (status == SR_OK) {
        SrStyleToken *token = &scene->tokens[scene->token_count++];
        *token = (SrStyleToken){.source_line = line};
        token->name = sr_strdup((const char *)name);
        token->value = sr_strdup((const char *)value);
        if (!token->name || !token->value) status = SR_ERR_MEMORY;
    }
    xmlFree(name);
    xmlFree(value);
    if (status == SR_ERR_MEMORY)
        sr_diag_error(diag, line, "token", NULL, "out of memory loading token");
    return status;
}

static SrStatus resolve_token(SrScene *scene, size_t index, unsigned depth,
                               unsigned char *states, SrDiagnostics *diag) {
    SrStyleToken *token = &scene->tokens[index];
    if (states[index] == 2) return SR_OK;
    if (states[index] == 1) {
        sr_diag_error(diag, token->source_line, "token", "value",
                      "token alias cycle at '%s'", token->name);
        return SR_ERR_XML;
    }
    if (depth > SR_MAX_TOKEN_DEPTH) {
        sr_diag_error(diag, token->source_line, "token", "value",
                      "token alias chain exceeds %u hops", SR_MAX_TOKEN_DEPTH);
        return SR_ERR_XML;
    }
    states[index] = 1;
    char name[SR_MAX_TOKEN_NAME + 1];
    int reference = token_reference(token->value, name);
    if (reference < 0) {
        sr_diag_error(diag, token->source_line, "token", "value",
                      "malformed token reference; expected var(--name)");
        return SR_ERR_XML;
    }
    token->resolved_index = index;
    if (reference) {
        size_t target = token_index(scene, name);
        if (target == SIZE_MAX) {
            sr_diag_error(diag, token->source_line, "token", "value",
                          "unknown token '%s'", name);
            return SR_ERR_XML;
        }
        SrStatus status = resolve_token(scene, target, depth + 1, states, diag);
        if (status != SR_OK) return status;
        token->resolved_hops = scene->tokens[target].resolved_hops + 1;
        if (token->resolved_hops > SR_MAX_TOKEN_DEPTH) {
            sr_diag_error(diag, token->source_line, "token", "value",
                          "token alias chain exceeds %u hops", SR_MAX_TOKEN_DEPTH);
            return SR_ERR_XML;
        }
        token->resolved_index = scene->tokens[target].resolved_index;
    }
    states[index] = 2;
    return SR_OK;
}

SrStatus sr_xml_prepare_styles(const xmlDoc *doc, SrScene *scene,
                                SrDiagnostics *diag) {
    const xmlNode *root = xmlDocGetRootElement(doc);
    for (const xmlNode *section = root ? root->children : NULL;
         section; section = section->next) {
        if (section->type != XML_ELEMENT_NODE ||
            strcmp((const char *)section->name, "styles")) continue;
        for (const xmlNode *node = section->children; node; node = node->next) {
            if (node->type != XML_ELEMENT_NODE ||
                strcmp((const char *)node->name, "token")) continue;
            SrStatus status = collect_token(node, scene, diag);
            if (status != SR_OK) return status;
        }
    }
    unsigned char states[SR_MAX_STYLE_TOKENS] = {0};
    for (size_t i = 0; i < scene->token_count; ++i) {
        SrStatus status = resolve_token(scene, i, 0, states, diag);
        if (status != SR_OK) return status;
    }
    return SR_OK;
}

void sr_xml_start_token(ParseContext *ctx, const XML_Char **attrs) {
    const char *const allowed[] = {"name", "value"};
    (void)sr_xml_attrs_allowed(ctx, "token", attrs, allowed, 2);
}

bool sr_xml_parse_color(ParseContext *ctx, const char *element,
                         const char *attribute, const char *text, SrColor *color) {
    char name[SR_MAX_TOKEN_NAME + 1];
    int reference = token_reference(text, name);
    if (reference < 0) {
        sr_xml_fail(ctx, element, attribute,
                    "malformed token reference; expected var(--name)");
        return false;
    }
    if (reference) {
        size_t index = token_index(ctx->scene, name);
        if (index == SIZE_MAX) {
            char message[SR_MAX_TOKEN_NAME + 32];
            snprintf(message, sizeof(message), "unknown token '%s'", name);
            sr_xml_fail(ctx, element, attribute, message);
            return false;
        }
        text = ctx->scene->tokens[ctx->scene->tokens[index].resolved_index].value;
    }
    SrStatus status = sr_parse_color_status(text, color);
    if (status == SR_ERR_MEMORY) {
        ctx->out_of_memory = true;
        sr_xml_fail(ctx, element, attribute, "out of memory parsing color");
    } else if (status != SR_OK && reference) {
        char message[SR_MAX_TOKEN_NAME + 48];
        snprintf(message, sizeof(message), "token '%s' does not resolve to a color", name);
        sr_xml_fail(ctx, element, attribute, message);
    }
    return status == SR_OK;
}
