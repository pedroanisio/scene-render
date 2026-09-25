#include "xml_internal.h"

#include <limits.h>
#include <stdint.h>
#include <string.h>

void sr_xml_start_node(ParseContext *ctx, const char *name,
                       const XML_Char **attrs, SrNodeType type) {
    static const char *const common[] = {"id", "z", "visible", "opacity", "x",
        "y", "zPosition", "rotation", "rotationX", "rotationY", "scaleX",
        "scaleY", "scaleZ", "anchorX", "anchorY", "start", "end"};
    const char *allowed[26];
    memcpy(allowed, common, sizeof(common));
    size_t count = sizeof(common) / sizeof(common[0]);
    if (type == SR_NODE_MEDIA) {
        allowed[count++] = "asset";
        allowed[count++] = "blend";
        allowed[count++] = "clipIn";
        allowed[count++] = "clipOut";
        allowed[count++] = "loop";
        allowed[count++] = "reverse";
        allowed[count++] = "speed";
        allowed[count++] = "timeStretch";
    } else if (type == SR_NODE_SHAPE) {
        allowed[count++] = "shape";
        allowed[count++] = "width";
        allowed[count++] = "height";
        allowed[count++] = "fill";
        allowed[count++] = "stroke";
        allowed[count++] = "strokeWidth";
        allowed[count++] = "blend";
    } else if (type == SR_NODE_PARTICLES) {
        allowed[count++] = "preset";
        allowed[count++] = "rate";
        allowed[count++] = "lifetime";
        allowed[count++] = "speed";
        allowed[count++] = "spread";
        allowed[count++] = "size";
        allowed[count++] = "color";
        allowed[count++] = "blend";
    }
    if (!sr_xml_attrs_allowed(ctx, name, attrs, allowed, count)) return;
    SrNode *node = sr_node_create(ctx->scene, type);
    if (!node) SR_XML_FAIL_RETURN(ctx, name, NULL, "out of memory");
    if (!sr_xml_parse_node_common(ctx, name, attrs, node)) {
        sr_node_free(node);
        return;
    }
    if (sr_scene_id_exists(ctx->scene, node->id)) {
        sr_node_free(node);
        SR_XML_FAIL_RETURN(ctx, name, "id", "node id must be unique");
    }
    if (type == SR_NODE_MEDIA) {
        const char *asset = sr_xml_required(ctx, name, attrs, "asset");
        if (!asset || !(node->asset_id = sr_strdup(asset))) {
            sr_node_free(node);
            if (!ctx->failed) sr_xml_fail(ctx, name, "asset", "out of memory");
            return;
        }
        const char *blend = sr_xml_attr(attrs, "blend");
        if (blend && !sr_blend_parse(blend, &node->blend)) {
            sr_node_free(node);
            SR_XML_FAIL_RETURN(ctx, name, "blend", "unsupported blend mode");
        }
        const char *value;
        if (!sr_xml_parse_double_attr(ctx, name, attrs, "clipIn", &node->clip_in) ||
            !sr_xml_parse_double_attr(ctx, name, attrs, "clipOut", &node->clip_out) ||
            !sr_xml_parse_double_attr(ctx, name, attrs, "speed", &node->speed) ||
            !sr_xml_parse_double_attr(ctx, name, attrs, "timeStretch",
                               &node->time_stretch)) {
            sr_node_free(node);
            return;
        }
        if (node->clip_in < 0.0 || (node->clip_out >= 0.0 &&
            node->clip_out <= node->clip_in) || node->speed <= 0.0 ||
            node->time_stretch <= 0.0) {
            sr_node_free(node);
            SR_XML_FAIL_RETURN(ctx, name, "clip/time",
                               "expected a valid positive media time range");
        }
        if ((value = sr_xml_attr(attrs, "loop"))) {
            uint64_t loops;
            if (!sr_parse_u64(value, &loops) || loops > INT64_MAX) {
                sr_node_free(node);
                SR_XML_FAIL_RETURN(ctx, name, "loop",
                                   "expected a non-negative integer");
            }
            node->loop_count = (int64_t)loops;
        }
        if ((value = sr_xml_attr(attrs, "reverse")) &&
            !sr_parse_bool(value, &node->reverse)) {
            sr_node_free(node);
            SR_XML_FAIL_RETURN(ctx, name, "reverse", "expected true or false");
        }
    } else if (type == SR_NODE_SHAPE) {
        const char *shape = sr_xml_required(ctx, name, attrs, "shape");
        if (!shape) { sr_node_free(node); return; }
        if (strcmp(shape, "rect") == 0) node->shape = SR_SHAPE_RECT;
        else if (strcmp(shape, "ellipse") == 0) node->shape = SR_SHAPE_ELLIPSE;
        else { sr_node_free(node); SR_XML_FAIL_RETURN(ctx, name, "shape",
            "expected rect or ellipse"); }
        if (!sr_xml_parse_double_attr(ctx, name, attrs, "width", &node->shape_width) ||
            !sr_xml_parse_double_attr(ctx, name, attrs, "height", &node->shape_height) ||
            !sr_xml_parse_double_attr(ctx, name, attrs, "strokeWidth", &node->stroke_width)) {
            sr_node_free(node); return;
        }
        const char *value = sr_xml_attr(attrs, "fill");
        if (value && !sr_parse_color(value, &node->fill)) {
            sr_node_free(node); SR_XML_FAIL_RETURN(ctx, name, "fill", "invalid color");
        }
        value = sr_xml_attr(attrs, "stroke");
        if (value && !sr_parse_color(value, &node->stroke)) {
            sr_node_free(node); SR_XML_FAIL_RETURN(ctx, name, "stroke", "invalid color");
        }
        value = sr_xml_attr(attrs, "blend");
        if (value && !sr_blend_parse(value, &node->blend)) {
            sr_node_free(node); SR_XML_FAIL_RETURN(ctx, name, "blend", "unsupported blend mode");
        }
        if (node->shape_width <= 0.0 || node->shape_height <= 0.0 ||
            node->stroke_width < 0.0) {
            sr_node_free(node); SR_XML_FAIL_RETURN(ctx, name, "width/height",
                "shape dimensions must be positive");
        }
    } else if (type == SR_NODE_PARTICLES) {
        const char *preset = sr_xml_required(ctx, name, attrs, "preset");
        if (!preset || !(node->particle_preset = sr_strdup(preset))) {
            sr_node_free(node); if (!ctx->failed) sr_xml_fail(ctx, name, "preset", "out of memory"); return;
        }
        if (strcmp(preset, "smoke") && strcmp(preset, "sparks") &&
            strcmp(preset, "dust") && strcmp(preset, "rain")) {
            sr_node_free(node); SR_XML_FAIL_RETURN(ctx, name, "preset",
                "expected smoke, sparks, dust, or rain");
        }
        if (!sr_xml_parse_double_attr(ctx, name, attrs, "rate", &node->particle_rate) ||
            !sr_xml_parse_double_attr(ctx, name, attrs, "lifetime", &node->particle_lifetime) ||
            !sr_xml_parse_double_attr(ctx, name, attrs, "speed", &node->particle_speed) ||
            !sr_xml_parse_double_attr(ctx, name, attrs, "spread", &node->particle_spread) ||
            !sr_xml_parse_double_attr(ctx, name, attrs, "size", &node->particle_size)) {
            sr_node_free(node); return;
        }
        const char *value = sr_xml_attr(attrs, "color");
        if (value && !sr_parse_color(value, &node->particle_color)) {
            sr_node_free(node); SR_XML_FAIL_RETURN(ctx, name, "color", "invalid color");
        }
        value = sr_xml_attr(attrs, "blend");
        if (value && !sr_blend_parse(value, &node->blend)) {
            sr_node_free(node); SR_XML_FAIL_RETURN(ctx, name, "blend", "unsupported blend mode");
        }
        if (node->particle_rate < 0.0 || node->particle_lifetime <= 0.0 ||
            node->particle_size <= 0.0) {
            sr_node_free(node); SR_XML_FAIL_RETURN(ctx, name, "rate/lifetime/size",
                "invalid particle parameters");
        }
    }
    ParseFrame *p = sr_xml_parent(ctx);
    if (!p || !p->node || sr_node_add_child(p->node, node) != SR_OK) {
        sr_node_free(node);
        SR_XML_FAIL_RETURN(ctx, name, NULL, "cannot attach node to parent");
    }
    sr_xml_push(ctx, (ParseFrame){.kind = type == SR_NODE_GROUP ? E_GROUP : E_LAYER,
                                  .node = node, .curve = SR_CURVE_LINEAR}, name);
}
