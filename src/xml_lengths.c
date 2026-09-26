/* SPDX-License-Identifier: Apache-2.0 */
#include "xml_internal.h"
#include "length_frame.h"

static bool length_limits(ParseContext *ctx, const SrNode *node, size_t depth,
                           size_t *nodes, size_t *masks) {
    const char *element = node == ctx->scene->root ? "composition" :
        node->type == SR_NODE_GROUP ? "group" :
        node->type == SR_NODE_MEDIA ? "layer" :
        node->type == SR_NODE_SHAPE ? "shape" : "particleEmitter";
    const char *message = NULL;
    if (depth > SR_MAX_LENGTH_DEPTH)
        message = "relative length depth limit is 256";
    else if (*nodes == SR_MAX_LENGTH_NODES || node->order >= SR_MAX_LENGTH_NODES ||
             node->child_count > SR_MAX_LENGTH_NODES)
        message = "relative length node limit is 65536 (including composition)";
    else if (node->mask_count > SR_MAX_LENGTH_MASKS - *masks)
        message = "relative length mask limit is 262144";
    if (message) {
        sr_diag_error(ctx->diag, node->source_line, element, NULL, "%s", message);
        return false;
    }
    ++*nodes;
    *masks += node->mask_count;
    for (size_t i = 0; i < node->child_count; ++i)
        if (!length_limits(ctx, node->children[i], depth + 1, nodes, masks))
            return false;
    return true;
}

/* Check document-wide limits after parsing: a relative value may occur only
 * in the last node. Geometry itself depends on loaded assets and render time. */
bool sr_xml_resolve_lengths(ParseContext *ctx) {
    if (!ctx->scene->has_relative_lengths) return true;
    size_t nodes = 0, masks = 0;
    if (!length_limits(ctx, ctx->scene->root, 1, &nodes, &masks)) return false;
    if (ctx->scene->physics.constraint_count > SR_MAX_LENGTH_CONSTRAINTS) {
        const SrConstraint *first =
            &ctx->scene->physics.constraints[SR_MAX_LENGTH_CONSTRAINTS];
        sr_diag_error(ctx->diag, first->source_line, "constraint", NULL,
                      "relative length constraint limit is 65536");
        return false;
    }
    return true;
}

bool sr_xml_length_attr(ParseContext *ctx, const char *element,
                         const XML_Char **attrs, const char *attribute,
                         double *value, SrLengthUnit *unit, bool positive) {
    const char *text = sr_xml_attr(attrs, attribute);
    if (!text) return true;
    SrLength length;
    if (!sr_parse_length(text, &length) || (positive && length.value <= 0.0)) {
        sr_xml_fail(ctx, element, attribute, positive
                    ? "expected a positive length (pixels, %, vw, vh, vmin or vmax); "
                      "relative spelling limit 128 bytes, coefficient limit 1e6"
                    : "expected a finite length (pixels, %, vw, vh, vmin or vmax); "
                      "relative spelling limit 128 bytes, coefficient limit 1e6");
        return false;
    }
    *value = length.value;
    *unit = length.unit;
    if (length.unit != SR_LENGTH_PIXELS) ctx->scene->has_relative_lengths = true;
    return true;
}

bool sr_xml_anim_length_attr(ParseContext *ctx, const char *element,
                              const XML_Char **attrs, const char *attribute,
                              SrAnimValue *value, bool positive) {
    return sr_xml_length_attr(ctx, element, attrs, attribute,
                              &value->base, &value->unit, positive);
}
