/* SPDX-License-Identifier: Apache-2.0 */
/* Loader parsing and reference resolution for B1-3 compositing: advanced
 * masks, track mattes and adjustment layers. Semantic validation of the
 * resulting scene (mask geometry, path grammar, matte cycles) is done by
 * sr_scene_prepare_compositing so XML and direct-C scenes share it. */
#include "xml_internal.h"
#include "compositing_internal.h"
#include "scene_render/markers.h"

#include <stdlib.h>
#include <string.h>

static bool parse_unit_attr(ParseContext *ctx, const XML_Char **attrs,
                            const char *name, SrAnimValue *value, double low,
                            double high, const char *message, bool *set) {
    if (!sr_xml_attr(attrs, name)) return true;
    if (!sr_xml_parse_double_attr(ctx, "mask", attrs, name, &value->base))
        return false;
    if (!(value->base >= low && value->base <= high)) {
        sr_xml_fail(ctx, "mask", name, message);
        return false;
    }
    if (set) *set = true;
    return true;
}

bool sr_xml_parse_mask(ParseContext *ctx, const XML_Char **attrs, SrMask *mask) {
    const char *type = sr_xml_required(ctx, "mask", attrs, "type");
    if (!type) return false;
    if (!strcmp(type, "rect")) mask->type = SR_MASK_RECT;
    else if (!strcmp(type, "ellipse")) mask->type = SR_MASK_ELLIPSE;
    else if (!strcmp(type, "rounded-rect")) mask->type = SR_MASK_ROUNDED_RECT;
    else if (!strcmp(type, "path")) mask->type = SR_MASK_PATH;
    else if (!strcmp(type, "polygon")) mask->type = SR_MASK_POLYGON;
    else if (!strcmp(type, "star")) mask->type = SR_MASK_STAR;
    else {
        sr_xml_fail(ctx, "mask", "type",
                    "expected rect, ellipse, rounded-rect, path, polygon or star");
        return false;
    }
    bool simple = mask->type <= SR_MASK_ROUNDED_RECT;
    mask->fill_rule = SR_FILL_NONZERO;
    mask->points = 5;
    mask->opacity.base = 1.0;
    if (!sr_xml_anim_length_attr(ctx, "mask", attrs, "x", &mask->x, false) ||
        !sr_xml_anim_length_attr(ctx, "mask", attrs, "y", &mask->y, false) ||
        !sr_xml_anim_length_attr(ctx, "mask", attrs, "width", &mask->width, true) ||
        !sr_xml_anim_length_attr(ctx, "mask", attrs, "height", &mask->height, true) ||
        !sr_xml_parse_double_attr(ctx, "mask", attrs, "radius", &mask->radius.base))
        return false;
    bool width = sr_xml_attr(attrs, "width") != NULL;
    bool height = sr_xml_attr(attrs, "height") != NULL;
    if (simple && (mask->width.base <= 0.0 || mask->height.base <= 0.0)) {
        sr_xml_fail(ctx, "mask", "width/height", "expected positive dimensions");
        return false;
    }
    if (!simple && width != height) {
        sr_xml_fail(ctx, "mask", "width/height",
                    "a path/polygon/star clipping box needs both width and height");
        return false;
    }
    mask->box_set = !simple && width && height;
    if (mask->radius.base < 0.0) {
        sr_xml_fail(ctx, "mask", "radius", "expected a non-negative radius");
        return false;
    }
    const char *value = sr_xml_attr(attrs, "invert");
    if (value && !sr_parse_bool(value, &mask->invert)) {
        sr_xml_fail(ctx, "mask", "invert", "expected true or false");
        return false;
    }
    if ((value = sr_xml_attr(attrs, "mode"))) {
        static const char *const modes[] = {"intersect", "add", "subtract",
            "lighten", "darken", "difference", "none"};
        size_t i = 0;
        while (i < sizeof(modes) / sizeof(modes[0]) && strcmp(value, modes[i])) ++i;
        if (i == sizeof(modes) / sizeof(modes[0])) {
            sr_xml_fail(ctx, "mask", "mode", "unknown mask mode");
            return false;
        }
        mask->mode = (SrMaskMode)i;
    }
    if ((value = sr_xml_attr(attrs, "fillRule"))) {
        if (!strcmp(value, "nonzero")) mask->fill_rule = SR_FILL_NONZERO;
        else if (!strcmp(value, "evenodd")) mask->fill_rule = SR_FILL_EVENODD;
        else {
            sr_xml_fail(ctx, "mask", "fillRule", "expected nonzero or evenodd");
            return false;
        }
    }
    if ((value = sr_xml_attr(attrs, "points"))) {
        uint32_t points;
        if (!sr_parse_u32(value, &points) || points < 3 || points > 4096) {
            sr_xml_fail(ctx, "mask", "points", "expected an integer in [3,4096]");
            return false;
        }
        if (mask->type != SR_MASK_POLYGON && mask->type != SR_MASK_STAR &&
            points != 5) {
            sr_xml_fail(ctx, "mask", "points",
                        "points is valid only for polygon and star masks");
            return false;
        }
        mask->points = points;
    }
    if (!parse_unit_attr(ctx, attrs, "opacity", &mask->opacity, 0.0, 1.0,
                         "expected a number in [0,1]", &mask->opacity_set) ||
        !parse_unit_attr(ctx, attrs, "feather", &mask->feather, 0.0,
                         SR_MAX_MASK_FILTER_RADIUS,
                         "expected a feather in [0,4096] local px", NULL) ||
        !parse_unit_attr(ctx, attrs, "expansion", &mask->expansion,
                         -(double)SR_MAX_MASK_FILTER_RADIUS, SR_MAX_MASK_FILTER_RADIUS,
                         "expected an expansion within +-4096 local px", NULL) ||
        !parse_unit_attr(ctx, attrs, "innerRadius", &mask->inner_radius, 0.0,
                         SR_MAX_MASK_COORDINATE,
                         "expected a non-negative innerRadius", &mask->inner_radius_set))
        return false;
    mask->opacity_set = true;
    /* An omitted innerRadius follows half the evaluated outer radius; a
     * later innerRadius track (possibly additive) uses half the static outer
     * radius as its base. */
    if (!mask->inner_radius_set) mask->inner_radius.base = 0.5 * mask->radius.base;
    if (mask->inner_radius_set && mask->type != SR_MASK_STAR) {
        sr_xml_fail(ctx, "mask", "innerRadius", "innerRadius is valid only for star masks");
        return false;
    }
    if ((value = sr_xml_attr(attrs, "path"))) {
        if (mask->type != SR_MASK_PATH) {
            sr_xml_fail(ctx, "mask", "path", "path is valid only for type=\"path\"");
            return false;
        }
        if (strlen(value) > SR_MAX_MASK_PATH_BYTES) {
            sr_xml_fail(ctx, "mask", "path", "path text exceeds 1048576 bytes");
            return false;
        }
        if (!(mask->path = sr_strdup(value))) {
            sr_xml_fail(ctx, "mask", "path", "out of memory");
            return false;
        }
    } else if (mask->type == SR_MASK_PATH) {
        sr_xml_fail(ctx, "mask", "path", "a path mask requires a nonempty path");
        return false;
    }
    return true;
}

bool sr_xml_parse_matte(ParseContext *ctx, const char *element,
                        const XML_Char **attrs, SrNode *node) {
    const char *value = sr_xml_attr(attrs, "matte");
    if (value) {
        if (!(node->matte_id = sr_strdup(value))) {
            sr_xml_fail(ctx, element, "matte", "out of memory");
            return false;
        }
        ctx->scene->compositing_required = true;
    }
    if ((value = sr_xml_attr(attrs, "matteMode"))) {
        static const char *const modes[] = {"alpha", "alpha-inverted", "luma",
                                            "luma-inverted"};
        size_t i = 0;
        while (i < 4 && strcmp(value, modes[i])) ++i;
        if (i == 4) {
            sr_xml_fail(ctx, element, "matteMode",
                        "expected alpha, alpha-inverted, luma or luma-inverted");
            return false;
        }
        node->matte_mode = (SrMatteMode)i;
    }
    if ((value = sr_xml_attr(attrs, "matteVisible")) &&
        !sr_parse_bool(value, &node->matte_visible)) {
        sr_xml_fail(ctx, element, "matteVisible", "expected true or false");
        return false;
    }
    return true;
}

void sr_xml_start_adjustment(ParseContext *ctx, const XML_Char **attrs) {
    static const char *const allowed[] = {"id", "z", "visible", "opacity", "x",
        "y", "rotation", "scaleX", "scaleY", "anchorX", "anchorY", "start",
        "end", "skewX", "skewY", "matte", "matteMode", "matteVisible",
        "effects", "blend", "name", "tags", "startMarker", "endMarker"};
    if (!sr_xml_attrs_allowed(ctx, "adjustment", attrs, allowed,
                              sizeof(allowed) / sizeof(allowed[0]))) return;
    SrNode *node = sr_node_create(ctx->scene, SR_NODE_ADJUSTMENT);
    if (!node) SR_XML_FAIL_RETURN(ctx, "adjustment", NULL, "out of memory");
    const char *effects = sr_xml_required(ctx, "adjustment", attrs, "effects");
    if (!effects || !sr_xml_parse_node_common(ctx, "adjustment", attrs, node) ||
        !sr_xml_parse_node_timeline(ctx, "adjustment", attrs, node, false) ||
        !sr_xml_parse_matte(ctx, "adjustment", attrs, node)) {
        sr_node_free(node);
        return;
    }
    if (sr_scene_id_exists(ctx->scene, node->id)) {
        sr_node_free(node);
        SR_XML_FAIL_RETURN(ctx, "adjustment", "id", "node id must be unique");
    }
    const char *blend = sr_xml_attr(attrs, "blend");
    if (blend && !sr_blend_parse(blend, &node->blend)) {
        sr_node_free(node);
        SR_XML_FAIL_RETURN(ctx, "adjustment", "blend", "unsupported blend mode");
    }
    if (!sr_xml_parse_effect_ids(node, effects)) {
        sr_node_free(node);
        SR_XML_FAIL_RETURN(ctx, "adjustment", "effects", "out of memory");
    }
    if (node->effect_ref_count > SR_MAX_ADJUSTMENT_EFFECTS) {
        sr_node_free(node);
        SR_XML_FAIL_RETURN(ctx, "adjustment", "effects",
                           "adjustment effect list limit is 256");
    }
    if (!node->effect_ref_count) {
        sr_node_free(node);
        SR_XML_FAIL_RETURN(ctx, "adjustment", "effects",
                           "an adjustment layer requires at least one effect");
    }
    ctx->scene->compositing_required = true;
    ParseFrame *p = sr_xml_parent(ctx);
    SrStatus attached = p && p->node ? sr_node_add_child(p->node, node)
                                     : SR_ERR_ARGUMENT;
    if (attached != SR_OK) {
        sr_node_free(node);
        SR_XML_FAIL_RETURN(ctx, "adjustment", NULL,
                           attached == SR_ERR_MEMORY
                               ? "out of memory while attaching node to parent"
                               : "cannot attach node to parent");
    }
    sr_xml_push(ctx, (ParseFrame){.kind = E_ADJUSTMENT, .node = node,
                                  .curve = SR_CURVE_LINEAR}, "adjustment");
}

typedef struct {
    const char *id;
    SrNode *node;
} IdEntry;

static int id_order(const void *left, const void *right) {
    return strcmp(((const IdEntry *)left)->id, ((const IdEntry *)right)->id);
}

static const char *node_element(const SrNode *node) {
    switch (node->type) {
    case SR_NODE_GROUP:
        return node->timeline && node->timeline->sequence ? "sequence" : "group";
    case SR_NODE_MEDIA: return "layer";
    case SR_NODE_SHAPE: return "shape";
    case SR_NODE_PARTICLES: return "particleEmitter";
    case SR_NODE_ADJUSTMENT: return "adjustment";
    }
    return "node";
}

/* Resolves matte ids through a sorted id index, before child sorting. The
 * structural preflight already bounded the tree (65536 nodes, depth 256),
 * so the index walk and its explicit stack are bounded as well. */
bool sr_xml_resolve_compositing(ParseContext *ctx) {
    SrScene *scene = ctx->scene;
    if (!scene->compositing_required) return true;
    size_t count = 0, capacity = 64;
    bool any = false;
    IdEntry *entries = sr_alloc(capacity * sizeof(*entries));
    SrNode **stack = sr_alloc(SR_MAX_COMPOSITE_NODES * sizeof(*stack));
    size_t depth = 0;
    bool ok = entries && stack;
    if (ok) stack[depth++] = scene->root;
    while (ok && depth) {
        SrNode *node = stack[--depth];
        if (node->matte_id) any = true;
        if (node->id && node != scene->root) {
            if (count == capacity) {
                IdEntry *grown = sr_realloc(entries, capacity * 2 * sizeof(*grown));
                if (!grown) { ok = false; break; }
                entries = grown;
                capacity *= 2;
            }
            entries[count++] = (IdEntry){node->id, node};
        }
        for (size_t i = node->child_count; i-- > 0;) {
            if (depth == SR_MAX_COMPOSITE_NODES) { ok = false; break; }
            stack[depth++] = node->children[i];
        }
    }
    if (!ok) {
        free(entries);
        free(stack);
        sr_diag_error(ctx->diag, 0, "composition", NULL,
                      "out of memory resolving track mattes");
        ctx->out_of_memory = true;
        return false;
    }
    if (!any) {
        free(entries);
        free(stack);
        return true;
    }
    if (count > 1) qsort(entries, count, sizeof(*entries), id_order);
    depth = 0;
    stack[depth++] = scene->root;
    while (ok && depth) {
        SrNode *node = stack[--depth];
        for (size_t i = node->child_count; i-- > 0;) stack[depth++] = node->children[i];
        if (!node->matte_id) continue;
        IdEntry key = {node->matte_id, NULL};
        const IdEntry *found = count
            ? bsearch(&key, entries, count, sizeof(*entries), id_order) : NULL;
        if (found) {
            node->matte = found->node;
            continue;
        }
        ok = false;
        sr_diag_error(ctx->diag, node->source_line, node_element(node), "matte",
                      sr_scene_id_exists(scene, node->matte_id)
                          ? "matte '%.128s' is not a group, layer, shape, "
                            "particleEmitter or adjustment"
                          : "unknown matte id '%.128s'",
                      node->matte_id);
    }
    free(entries);
    free(stack);
    return ok;
}
