/* SPDX-License-Identifier: Apache-2.0 */
#include "length_frame.h"
#include "length_internal.h"
#include "compositor_resources_internal.h"
#include "compositor_geometry_internal.h"
#include "scene_render/physics.h"

#include <math.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    SrLengthFrame *frame;
    const SrScene *scene;
    SrLengthBox output;
    SrDiagnostics *diag;
    double time;
    size_t masks_seen;
    bool base_pose;
} LengthWalk;

static const char *element(const SrNode *node) {
    switch (node->type) {
    case SR_NODE_GROUP: return "group";
    case SR_NODE_MEDIA: return "layer";
    case SR_NODE_SHAPE: return "shape";
    case SR_NODE_PARTICLES: return "particleEmitter";
    }
    return "node";
}

static SrStatus fail(LengthWalk *walk, const SrNode *node, SrStatus status,
                      const char *message) {
    if (walk->diag)
        sr_diag_error(walk->diag, node->source_line, element(node), NULL,
                      "%s (node '%s')", message, node->id ? node->id : "");
    return status;
}

static void *grow(SrCompositeResources *resources, void *buffer,
                   size_t *capacity, size_t needed,
                   size_t size, size_t maximum) {
    size_t next = *capacity ? *capacity : 32;
    while (next < needed) next = next > maximum / 2 ? maximum : next * 2;
    if (next > SIZE_MAX / size) return NULL;
    size_t zero_bytes = (next - *capacity) * size;
    if (!sr_composite_work(resources, zero_bytes / 4 + (zero_bytes % 4 != 0), 1))
        return NULL;
    void *grown = sr_composite_realloc(resources, buffer, next, size, 0);
    if (!grown) return NULL;
    memset((char *)grown + *capacity * size, 0, (next - *capacity) * size);
    *capacity = next;
    return grown;
}

static SrStatus scalar(LengthWalk *walk, const SrAnimValue *value, double axis,
                        size_t line, const char *host, const char *attribute,
                        double *pixels) {
    SrCompositeResources *resources = walk->frame->resources;
    if (resources) {
        SrCompositeOwner previous = sr_composite_owner(resources,
            (SrCompositeOwner){line, host, attribute});
        bool admitted = walk->base_pose ? sr_composite_work(resources, 1, 1)
            : sr_composite_anim_work(resources, value, true);
        sr_composite_owner(resources, previous);
        if (!admitted) return SR_ERR_RENDER;
    }
    SrStatus status = walk->base_pose
        ? sr_length_resolve((SrLength){value->base, value->unit}, axis,
                             walk->output, false, pixels)
        : sr_anim_length_eval(value, walk->time, axis, walk->output, pixels);
    if (status != SR_OK && walk->diag)
        sr_diag_error(walk->diag, line, host, attribute,
                      "length evaluation is non-finite or exceeds its relative limit");
    return status;
}

static SrStatus dimension(LengthWalk *walk, const SrNode *node, SrLength value,
                           double axis, const char *attribute, double *pixels) {
    SrStatus status = sr_length_resolve(value, axis, walk->output, true, pixels);
    if (status != SR_OK && walk->diag)
        sr_diag_error(walk->diag, node->source_line, element(node), attribute,
                      "length must resolve to a finite positive dimension "
                      "within its relative limit");
    return status;
}

static SrStatus local_box(LengthWalk *walk, const SrNode *node, SrLengthBox parent,
                           SrLengthBox *box) {
    *box = walk->output;
    SrStatus status = SR_OK;
    if (node->type == SR_NODE_GROUP) {
        if (node->group_width_set)
            status = dimension(walk, node, node->group_width, parent.width,
                                 "width", &box->width);
        if (status == SR_OK && node->group_height_set)
            status = dimension(walk, node, node->group_height, parent.height,
                                 "height", &box->height);
    } else if (node->type == SR_NODE_SHAPE) {
        status = dimension(walk, node,
                             (SrLength){node->shape_width, node->shape_width_unit},
                             parent.width, "width", &box->width);
        if (status == SR_OK)
            status = dimension(walk, node,
                                 (SrLength){node->shape_height, node->shape_height_unit},
                                 parent.height, "height", &box->height);
    } else if (node->type == SR_NODE_MEDIA && node->asset) {
        *box = (SrLengthBox){node->asset->width, node->asset->height};
    }
    return status;
}

static SrStatus masks(LengthWalk *walk, const SrNode *node, SrNodeGeometry *geometry) {
    SrLengthFrame *frame = walk->frame;
    size_t needed = frame->mask_count + node->mask_count;
    if (needed > frame->mask_capacity) {
        SrMaskGeometry *grown = grow(frame->resources, frame->masks,
                                      &frame->mask_capacity, needed,
                                      sizeof(*grown), SR_MAX_LENGTH_MASKS);
        if (!grown) {
            if (frame->resources) return sr_composite_resource_status(frame->resources);
            return fail(walk, node, SR_ERR_MEMORY,
                          "out of memory evaluating mask lengths");
        }
        frame->masks = grown;
    }
    geometry->mask_offset = frame->mask_count;
    frame->mask_count = needed;
    for (size_t i = 0; i < node->mask_count; ++i) {
        const SrMask *mask = &node->masks[i];
        SrMaskGeometry *out = &frame->masks[geometry->mask_offset + i];
        const SrAnimValue *values[] = {&mask->x, &mask->y, &mask->width, &mask->height};
        double *targets[] = {&out->x, &out->y, &out->width, &out->height};
        const char *names[] = {"x", "y", "width", "height"};
        for (size_t k = 0; k < 4; ++k) {
            double axis = k % 2 ? geometry->box.height : geometry->box.width;
            SrStatus status = scalar(walk, values[k], axis, mask->source_line,
                                       "mask", names[k], targets[k]);
            if (status != SR_OK) return status;
        }
        out->width = fmax(0.0, out->width);
        out->height = fmax(0.0, out->height);
    }
    return SR_OK;
}

static SrStatus index_node(LengthWalk *walk, const SrNode *node, size_t depth) {
    SrLengthFrame *frame = walk->frame;
    if (!node) return SR_ERR_ARGUMENT;
    sr_composite_owner(frame->resources, (SrCompositeOwner){
        node->source_line, node == walk->scene->root ? "composition" : element(node),
        NULL});
    if (!sr_composite_physics_ready(frame->resources, walk->scene, node, walk->time))
        return SR_ERR_RENDER;
    if (depth > SR_MAX_LENGTH_DEPTH)
        return fail(walk, node, SR_ERR_RENDER, "relative length depth limit is 256");
    if (node->order >= SR_MAX_LENGTH_NODES || node->child_count > SR_MAX_LENGTH_NODES)
        return fail(walk, node, SR_ERR_RENDER, "relative length node limit is 65536");
    if (node->mask_count > SR_MAX_LENGTH_MASKS - walk->masks_seen)
        return fail(walk, node, SR_ERR_RENDER, "relative length mask limit is 262144");
    walk->masks_seen += node->mask_count;
    /* Index/evaluation, child traversal, local box and each mask record;
     * scalar animation/key work is reserved separately at its consumer. */
    if (!sr_composite_work(frame->resources,
                            8 + 2 * node->child_count + node->mask_count, 1))
        return SR_ERR_RENDER;
    size_t needed = node->order + 1;
    if (needed > frame->node_capacity) {
        SrNodeGeometry *grown = grow(frame->resources, frame->nodes,
                                      &frame->node_capacity, needed,
                                      sizeof(*grown), SR_MAX_LENGTH_NODES);
        if (!grown) {
            if (frame->resources) return sr_composite_resource_status(frame->resources);
            return fail(walk, node, SR_ERR_MEMORY,
                          "out of memory evaluating node lengths");
        }
        frame->nodes = grown;
    }
    if (needed > frame->node_count) frame->node_count = needed;
    SrNodeGeometry *geometry = &frame->nodes[node->order];
    if (geometry->node)
        return fail(walk, node, SR_ERR_RENDER, "duplicate relative length node order");
    geometry->node = node;
    return SR_OK;
}

/* Index first so limits apply before physics collection. Mark only actual
 * consumers and ancestor boxes; unrelated or ignored bases stay unused. */
static SrStatus index_physics(LengthWalk *walk, const SrNode *node, size_t depth) {
    SrStatus status = index_node(walk, node, depth);
    if (status != SR_OK) return status;
    bool local = node->type == SR_NODE_SHAPE
        ? node->shape_width > 0 && node->shape_height > 0
        : node->type == SR_NODE_MEDIA && node->asset &&
          node->asset->width > 0 && node->asset->height > 0;
    bool rigid = walk->scene->physics.enabled && node->body.type != SR_BODY_NONE;
    bool soft = node->soft_body.enabled && local;
    bool children = false;
    for (size_t i = 0; i < node->child_count; ++i) {
        status = index_physics(walk, node->children[i], depth + 1);
        if (status != SR_OK) return status;
        children = children || walk->frame->nodes[node->children[i]->order].base_scope;
    }
    SrNodeGeometry *g = &walk->frame->nodes[node->order];
    g->base_node = rigid || soft;
    g->base_anchor = soft;
    g->base_box = children || soft || (rigid && local);
    g->base_scope = children || g->base_node;
    return SR_OK;
}

static SrStatus visit(LengthWalk *walk, const SrNode *node, SrLengthBox parent,
                       size_t depth, bool parent_live, bool parent_drawn,
                       bool bounds) {
    if (!walk->base_pose) {
        SrStatus status = index_node(walk, node, depth);
        if (status != SR_OK) return status;
    }
    SrNodeGeometry *geometry = &walk->frame->nodes[node->order];
    if (walk->base_pose && !geometry->base_scope) return SR_OK;
    bool live = parent_live && node->visible && walk->time >= node->start_time &&
        walk->time < node->end_time;
    if (!walk->base_pose && parent_drawn && live &&
        !sr_composite_anim_work(walk->frame->resources, &node->opacity, false))
        return SR_ERR_RENDER;
    bool drawn = !walk->base_pose && parent_drawn && live &&
        !(sr_anim_eval(&node->opacity, walk->time) <= 0.0);
    bool content = walk->base_pose ? geometry->base_box : drawn || (bounds && live);
    bool transform = walk->base_pose ? geometry->base_node
                                     : content || (parent_drawn && node->card);
    if (transform) {
        const SrAnimValue *values[] = {&node->transform.x, &node->transform.y,
            &node->transform.anchor_x, &node->transform.anchor_y};
        double *targets[] = {&geometry->x, &geometry->y,
            &geometry->anchor_x, &geometry->anchor_y};
        const char *names[] = {"x", "y", "anchorX", "anchorY"};
        double angle;
        bool physics = !walk->base_pose && sr_physics_pose(walk->scene, node, walk->time,
                                                           &geometry->x, &geometry->y, &angle);
        size_t count = walk->base_pose && !geometry->base_anchor ? 2 : 4;
        for (size_t i = physics ? 2 : 0; i < count; ++i) {
            SrStatus status = scalar(walk, values[i], i % 2 ? parent.height : parent.width,
                                       node->source_line, element(node), names[i], targets[i]);
            if (status != SR_OK) return status;
        }
    }
    SrLengthBox box = walk->output;
    if (content) {
        SrStatus status = local_box(walk, node, parent, &box);
        if (status != SR_OK) return status;
        geometry->box = box;
    }
    if (drawn && !walk->base_pose) {
        SrStatus status = masks(walk, node, geometry);
        if (status != SR_OK) return status;
    }
    /* Recursion may grow the arrays; retain only the box, not an entry pointer. */
    for (size_t i = 0; i < node->child_count; ++i) {
        SrStatus status = visit(walk, node->children[i], box, depth + 1, live, drawn,
                                 bounds || (drawn && node->card));
        if (status != SR_OK) return status;
    }
    return SR_OK;
}

static SrStatus prepare(SrLengthFrame *frame, const SrScene *scene,
                                 double time, bool base_pose, SrDiagnostics *diag) {
    if (!frame || !scene || !scene->root || !isfinite(time)) return SR_ERR_ARGUMENT;
    if (!sr_composite_work(frame->resources, frame->node_count,
                            sizeof(*frame->nodes) / 4))
        return SR_ERR_RENDER;
    if (frame->node_count)
        memset(frame->nodes, 0, frame->node_count * sizeof(*frame->nodes));
    frame->node_count = frame->mask_count = 0;
    LengthWalk walk = {.frame = frame, .scene = scene,
        .output = {scene->project.width, scene->project.height},
        .diag = diag, .time = time, .base_pose = base_pose};
    if (base_pose) {
        SrStatus status = index_physics(&walk, scene->root, 1);
        if (status != SR_OK) return status;
    }
    return visit(&walk, scene->root, walk.output, 1, true, true, false);
}

SrStatus sr_length_frame_prepare(SrLengthFrame *frame, const SrScene *scene,
                                 double time, bool base_pose, SrDiagnostics *diag) {
    SrCompositeResources *resources = frame ? frame->resources : NULL;
    SrCompositeOwner previous = resources ? resources->owner : (SrCompositeOwner){0};
    SrStatus status = prepare(frame, scene, time, base_pose, diag);
    sr_composite_owner(resources, previous);
    return status;
}

void sr_length_frame_free(SrLengthFrame *frame) {
    if (!frame) return;
    sr_composite_free(frame->resources, frame->nodes);
    sr_composite_free(frame->resources, frame->masks);
    *frame = (SrLengthFrame){0};
}
