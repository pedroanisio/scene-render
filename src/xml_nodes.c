#include "xml_internal.h"

#include <ctype.h>
#include <limits.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

/* Splits a whitespace-separated id list into node->effect_ids. */
static bool parse_effect_ids(SrNode *node, const char *text) {
    size_t capacity = 0;
    const char *cursor = text;
    while (*cursor) {
        while (*cursor && isspace((unsigned char)*cursor)) ++cursor;
        if (!*cursor) break;
        const char *begin = cursor;
        while (*cursor && !isspace((unsigned char)*cursor)) ++cursor;
        if (node->effect_ref_count == capacity) {
            capacity = capacity ? capacity * 2 : 4;
            char **ids = sr_realloc(node->effect_ids, capacity * sizeof(*ids));
            if (!ids) return false;
            node->effect_ids = ids;
        }
        size_t length = (size_t)(cursor - begin);
        char *id = sr_alloc(length + 1);
        if (!id) return false;
        memcpy(id, begin, length);
        id[length] = '\0';
        node->effect_ids[node->effect_ref_count++] = id;
    }
    return true;
}

typedef struct {
    const char *name;
    double speed_factor, speed_var_factor;
    double direction, gravity_y, emitter_width, wobble;
    bool grow;
} ParticlePreset;

/* Defaults that reproduce the pre-parametric preset motion (see
 * docs/xml-reference.md); explicit attributes override every field. */
static const ParticlePreset presets[] = {
    {"sparks", 0.75, 0.25, -90.0, 400.0, 0.0, 0.0, false},
    {"smoke", 0.75, 0.25, -90.0, 0.0, 0.0, 15.0, true},
    {"dust", 0.1875, 0.0625, -90.0, 0.0, 0.0, 0.0, false},
    {"rain", 0.75, 0.25, 90.0, 0.0, 80.0, 0.0, false},
};

static bool nonnegative_attr(ParseContext *ctx, const char *element,
                             const XML_Char **attrs, const char *name,
                             double *target) {
    if (!sr_xml_parse_double_attr(ctx, element, attrs, name, target)) return false;
    if (sr_xml_attr(attrs, name) && *target < 0.0) {
        sr_xml_fail(ctx, element, name, "expected a non-negative number");
        return false;
    }
    return true;
}

static bool parse_particles(ParseContext *ctx, const char *name,
                            const XML_Char **attrs, SrNode *node) {
    const char *preset = sr_xml_attr(attrs, "preset");
    if (preset) {
        const ParticlePreset *found = NULL;
        for (size_t i = 0; i < sizeof(presets) / sizeof(presets[0]); ++i)
            if (strcmp(preset, presets[i].name) == 0) found = &presets[i];
        if (!found) {
            sr_xml_fail(ctx, name, "preset", "expected smoke, sparks, dust, or rain");
            return false;
        }
        if (!(node->particle_preset = sr_strdup(preset))) {
            sr_xml_fail(ctx, name, "preset", "out of memory");
            return false;
        }
        node->particle_speed_factor = found->speed_factor;
        node->particle_speed_var_factor = found->speed_var_factor;
        node->particle_direction.base = found->direction;
        node->particle_gravity_y = found->gravity_y;
        node->particle_emitter_width = found->emitter_width;
        node->particle_wobble = found->wobble;
        node->particle_grow = found->grow;
    }
    if (!sr_xml_parse_double_attr(ctx, name, attrs, "rate", &node->particle_rate.base) ||
        !sr_xml_parse_double_attr(ctx, name, attrs, "lifetime", &node->particle_lifetime.base) ||
        !sr_xml_parse_double_attr(ctx, name, attrs, "speed", &node->particle_speed.base) ||
        !sr_xml_parse_double_attr(ctx, name, attrs, "spread", &node->particle_spread.base) ||
        !sr_xml_parse_double_attr(ctx, name, attrs, "size", &node->particle_size.base) ||
        !sr_xml_parse_double_attr(ctx, name, attrs, "direction", &node->particle_direction.base) ||
        !sr_xml_parse_double_attr(ctx, name, attrs, "gravityX", &node->particle_gravity_x) ||
        !sr_xml_parse_double_attr(ctx, name, attrs, "gravityY", &node->particle_gravity_y) ||
        !nonnegative_attr(ctx, name, attrs, "sizeEnd", &node->particle_size_end) ||
        !nonnegative_attr(ctx, name, attrs, "speedVariance", &node->particle_speed_variance) ||
        !nonnegative_attr(ctx, name, attrs, "lifetimeVariance", &node->particle_lifetime_variance) ||
        !nonnegative_attr(ctx, name, attrs, "emitterWidth", &node->particle_emitter_width) ||
        !nonnegative_attr(ctx, name, attrs, "emitterHeight", &node->particle_emitter_height))
        return false;
    node->particle_size_end_set = sr_xml_attr(attrs, "sizeEnd") != NULL;
    node->particle_speed_variance_set = sr_xml_attr(attrs, "speedVariance") != NULL;
    if (node->particle_size_end_set) node->particle_grow = false;
    const char *value = sr_xml_attr(attrs, "color");
    if (value && !sr_parse_color(value, &node->particle_color.base)) {
        sr_xml_fail(ctx, name, "color", "invalid color");
        return false;
    }
    /* Particles fade out by default: colorEnd is the birth color at zero
     * alpha unless given. */
    node->particle_color_end.base = node->particle_color.base;
    node->particle_color_end.base.a = 0.0;
    value = sr_xml_attr(attrs, "colorEnd");
    if (value) {
        if (!sr_parse_color(value, &node->particle_color_end.base)) {
            sr_xml_fail(ctx, name, "colorEnd", "invalid color");
            return false;
        }
        node->particle_color_end_set = true;
    }
    node->particle_color.space = node->particle_color_end.space =
        ctx->scene->project.working_color_space;
    if ((value = sr_xml_attr(attrs, "maxParticles"))) {
        uint32_t max;
        if (!sr_parse_u32(value, &max) || max == 0 || max > 10000000) {
            sr_xml_fail(ctx, name, "maxParticles", "expected an integer in [1, 10000000]");
            return false;
        }
        node->particle_max = max;
    }
    if ((value = sr_xml_attr(attrs, "seed"))) {
        if (!sr_parse_u64(value, &node->particle_seed)) {
            sr_xml_fail(ctx, name, "seed", "expected an unsigned 64-bit integer");
            return false;
        }
        node->particle_seed_set = true;
    }
    if ((value = sr_xml_attr(attrs, "shape"))) {
        if (strcmp(value, "disc") == 0) node->particle_shape = SR_PARTICLE_DISC;
        else if (strcmp(value, "square") == 0) node->particle_shape = SR_PARTICLE_SQUARE;
        else {
            sr_xml_fail(ctx, name, "shape", "expected disc or square");
            return false;
        }
    }
    value = sr_xml_attr(attrs, "blend");
    if (value && !sr_blend_parse(value, &node->blend)) {
        sr_xml_fail(ctx, name, "blend", "unsupported blend mode");
        return false;
    }
    if (node->particle_rate.base < 0.0 || node->particle_lifetime.base <= 0.0 ||
        node->particle_size.base <= 0.0) {
        sr_xml_fail(ctx, name, "rate/lifetime/size", "invalid particle parameters");
        return false;
    }
    return true;
}

void sr_xml_start_node(ParseContext *ctx, const char *name,
                       const XML_Char **attrs, SrNodeType type) {
    static const char *const common[] = {"id", "z", "visible", "opacity", "x",
        "y", "rotation", "scaleX", "scaleY", "anchorX", "anchorY", "start",
        "end"};
    const char *allowed[40];
    memcpy(allowed, common, sizeof(common));
    size_t count = sizeof(common) / sizeof(common[0]);
    if (type == SR_NODE_GROUP) {
        allowed[count++] = "blend";
        allowed[count++] = "effects";
    } else if (type == SR_NODE_MEDIA) {
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
        allowed[count++] = "direction";
        allowed[count++] = "speedVariance";
        allowed[count++] = "gravityX";
        allowed[count++] = "gravityY";
        allowed[count++] = "sizeEnd";
        allowed[count++] = "colorEnd";
        allowed[count++] = "lifetimeVariance";
        allowed[count++] = "maxParticles";
        allowed[count++] = "emitterWidth";
        allowed[count++] = "emitterHeight";
        allowed[count++] = "seed";
        allowed[count++] = "shape";
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
    if (type == SR_NODE_GROUP) {
        const char *blend = sr_xml_attr(attrs, "blend");
        if (blend && !sr_blend_parse(blend, &node->blend)) {
            sr_node_free(node);
            SR_XML_FAIL_RETURN(ctx, name, "blend", "unsupported blend mode");
        }
        const char *effects = sr_xml_attr(attrs, "effects");
        if (effects && !parse_effect_ids(node, effects)) {
            sr_node_free(node);
            SR_XML_FAIL_RETURN(ctx, name, "effects", "out of memory");
        }
    } else if (type == SR_NODE_MEDIA) {
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
        if (value && !sr_parse_color(value, &node->fill.base)) {
            sr_node_free(node); SR_XML_FAIL_RETURN(ctx, name, "fill", "invalid color");
        }
        value = sr_xml_attr(attrs, "stroke");
        if (value && !sr_parse_color(value, &node->stroke.base)) {
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
        if (!parse_particles(ctx, name, attrs, node)) {
            sr_node_free(node);
            return;
        }
    }
    ParseFrame *p = sr_xml_parent(ctx);
    SrStatus attached = p && p->node ? sr_node_add_child(p->node, node)
                                     : SR_ERR_ARGUMENT;
    if (attached != SR_OK) {
        sr_node_free(node);
        SR_XML_FAIL_RETURN(ctx, name, NULL,
                           attached == SR_ERR_MEMORY
                               ? "out of memory while attaching node to parent"
                               : "cannot attach node to parent");
    }
    sr_xml_push(ctx, (ParseFrame){.kind = type == SR_NODE_GROUP ? E_GROUP : E_LAYER,
                                  .node = node, .curve = SR_CURVE_LINEAR}, name);
}
