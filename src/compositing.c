/* SPDX-License-Identifier: Apache-2.0 */
#include "compositing_internal.h"
#include "particles_internal.h"

#include <stdlib.h>
#include <string.h>

static const char *element(const SrScene *scene, const SrNode *node) {
    if (!node || node == scene->root) return "composition";
    switch (node->type) {
    case SR_NODE_GROUP: return "group";
    case SR_NODE_MEDIA: return "layer";
    case SR_NODE_SHAPE: return "shape";
    case SR_NODE_PARTICLES: return "particleEmitter";
    }
    return "node";
}

static SrStatus fail(const SrScene *scene, const SrNode *node,
                       SrDiagnostics *diag, SrStatus status,
                       const char *attribute, const char *message) {
    if (diag)
        sr_diag_error(diag, node ? node->source_line : 0, element(scene, node),
                      attribute, "%s (node '%s')", message,
                      node && node->id ? node->id : "");
    return status;
}

bool sr_node_uses_compositing(const SrNode *node) {
    return node && (node->blend > SR_BLEND_DIFFERENCE ||
        node->transform.skew_x.base != 0 || node->transform.skew_y.base != 0 ||
        node->transform.skew_x.track.count || node->transform.skew_y.track.count);
}

void sr_composite_plan_free(SrCompositePlan *plan) {
    if (!plan) return;
    free(plan->nodes);
    free(plan);
}

void sr_scene_invalidate_compositing(SrScene *scene) {
    if (!scene) return;
    if (scene->compositing) {
        for (size_t i = 0; i < scene->compositing->count; ++i) {
            SrNode *node = (SrNode *)scene->compositing->nodes[i].node;
            if (node->type == SR_NODE_PARTICLES) sr_particles_invalidate(node);
        }
    }
    sr_composite_plan_free(scene->compositing);
    scene->compositing = NULL;
    scene->compositing_required = true;
}

/* AVL height is below 24 for at most 65536 entries. The fixed bound makes
 * pointer lookups safe independently of allocator layout and hash collisions. */
#define SR_MAX_COMPOSITE_LOOKUP_DEPTH 32u

const SrCompositeNode *sr_composite_plan_node(const SrCompositePlan *plan,
                                               const SrNode *node) {
    if (!plan || !node) return NULL;
    size_t index = plan->lookup_root;
    for (size_t i = 0; index != SIZE_MAX && i < SR_MAX_COMPOSITE_LOOKUP_DEPTH; ++i) {
        const SrCompositeNode *entry = &plan->nodes[index];
        if (entry->node == node) return entry;
        index = (uintptr_t)node < (uintptr_t)entry->node ? entry->left : entry->right;
    }
    return NULL;
}

static unsigned height(const SrCompositePlan *plan, size_t index) {
    return index == SIZE_MAX ? 0 : plan->nodes[index].height;
}

static void update_height(SrCompositePlan *plan, size_t index) {
    SrCompositeNode *node = &plan->nodes[index];
    unsigned left = height(plan, node->left), right = height(plan, node->right);
    node->height = 1 + (left > right ? left : right);
}

static size_t rotate_left(SrCompositePlan *plan, size_t index) {
    SrCompositeNode *node = &plan->nodes[index];
    size_t right = node->right;
    node->right = plan->nodes[right].left;
    plan->nodes[right].left = index;
    update_height(plan, index);
    update_height(plan, right);
    return right;
}

static size_t rotate_right(SrCompositePlan *plan, size_t index) {
    SrCompositeNode *node = &plan->nodes[index];
    size_t left = node->left;
    node->left = plan->nodes[left].right;
    plan->nodes[left].right = index;
    update_height(plan, index);
    update_height(plan, left);
    return left;
}

static size_t balance(SrCompositePlan *plan, size_t index) {
    SrCompositeNode *node = &plan->nodes[index];
    int delta = (int)height(plan, node->left) - (int)height(plan, node->right);
    if (delta > 1) {
        const SrCompositeNode *left = &plan->nodes[node->left];
        if (height(plan, left->right) > height(plan, left->left))
            node->left = rotate_left(plan, node->left);
        return rotate_right(plan, index);
    }
    if (delta < -1) {
        const SrCompositeNode *right = &plan->nodes[node->right];
        if (height(plan, right->left) > height(plan, right->right))
            node->right = rotate_right(plan, node->right);
        return rotate_left(plan, index);
    }
    update_height(plan, index);
    return index;
}

static SrStatus grow_nodes(SrCompositePlan *plan) {
    if (plan->count < plan->capacity) return SR_OK;
    size_t next = plan->capacity ? plan->capacity * 2 : 32;
    if (next > SR_MAX_COMPOSITE_NODES) next = SR_MAX_COMPOSITE_NODES;
    if (next > SIZE_MAX / sizeof(*plan->nodes)) return SR_ERR_RENDER;
    size_t bytes = next * sizeof(*plan->nodes);
    if (bytes > SR_MAX_COMPOSITE_BYTES - plan->owned_bytes) return SR_ERR_RENDER;
    SrCompositeNode *nodes = sr_alloc(bytes);
    if (!nodes) return SR_ERR_MEMORY;
    if (plan->count) memcpy(nodes, plan->nodes, plan->count * sizeof(*nodes));
    free(plan->nodes);
    plan->owned_bytes += bytes - plan->capacity * sizeof(*nodes);
    plan->nodes = nodes;
    plan->capacity = next;
    return SR_OK;
}

typedef struct {
    size_t entry, next_child;
} WalkFrame;

static SrStatus append_node(const SrScene *scene, SrCompositePlan *plan,
                              const SrNode *node, size_t parent, size_t depth,
                              SrDiagnostics *diag) {
    const SrNode *owner = node ? node
        : parent != SIZE_MAX ? plan->nodes[parent].node : NULL;
    if (!node)
        return fail(scene, owner, diag, SR_ERR_RENDER, "children",
                    "null node in compositing ownership tree");
    if (depth > SR_MAX_COMPOSITE_DEPTH)
        return fail(scene, node, diag, SR_ERR_RENDER, "children",
                    "compositing ancestry depth limit is 256");
    if (plan->count == SR_MAX_COMPOSITE_NODES ||
        node->child_count >= SR_MAX_COMPOSITE_NODES)
        return fail(scene, node, diag, SR_ERR_RENDER, "children",
                    "compositing node limit is 65536 including root");
    if (node->mask_count > SR_MAX_COMPOSITE_MASKS - plan->mask_count)
        return fail(scene, node, diag, SR_ERR_RENDER, "mask",
                    "compositing mask limit is 262144");
    if ((node->child_count && !node->children) ||
        (node->mask_count && !node->masks))
        return fail(scene, node, diag, SR_ERR_RENDER,
                    node->child_count && !node->children ? "children" : "mask",
                    "compositing count has no backing array");
    size_t ancestors[SR_MAX_COMPOSITE_LOOKUP_DEPTH];
    bool directions[SR_MAX_COMPOSITE_LOOKUP_DEPTH];
    size_t used = 0, index = plan->lookup_root;
    while (index != SIZE_MAX) {
        const SrCompositeNode *entry = &plan->nodes[index];
        if (entry->node == node)
            return fail(scene, node, diag, SR_ERR_RENDER, "children",
                        "compositing ownership cycle or shared child");
        if (used == SR_MAX_COMPOSITE_LOOKUP_DEPTH)
            return fail(scene, node, diag, SR_ERR_RENDER, "children",
                        "compositing lookup depth limit exceeded");
        ancestors[used] = index;
        directions[used] = (uintptr_t)node > (uintptr_t)entry->node;
        index = directions[used++] ? entry->right : entry->left;
    }
    SrStatus status = grow_nodes(plan);
    if (status != SR_OK)
        return fail(scene, node, diag, status, NULL, status == SR_ERR_MEMORY
                    ? "out of memory preparing compositing"
                    : "compositing preparation storage limit exceeded");
    index = plan->count++;
    plan->nodes[index] = (SrCompositeNode){node, parent, depth, plan->mask_count,
                                         SIZE_MAX, SIZE_MAX, 1};
    plan->mask_count += node->mask_count;
    if (!used) plan->lookup_root = index;
    else if (directions[used - 1]) plan->nodes[ancestors[used - 1]].right = index;
    else plan->nodes[ancestors[used - 1]].left = index;
    while (used) {
        size_t root = balance(plan, ancestors[--used]);
        if (!used) plan->lookup_root = root;
        else if (directions[used - 1]) plan->nodes[ancestors[used - 1]].right = root;
        else plan->nodes[ancestors[used - 1]].left = root;
    }
    return SR_OK;
}

SrStatus sr_composite_plan_build(const SrScene *scene, SrCompositePlan **out,
                                  SrDiagnostics *diag) {
    if (!out) return SR_ERR_ARGUMENT;
    *out = NULL;
    if (!scene || !scene->root) return SR_ERR_ARGUMENT;
    SrCompositePlan *plan = sr_alloc(sizeof(*plan));
    if (!plan)
        return fail(scene, scene->root, diag, SR_ERR_MEMORY, NULL,
                    "out of memory preparing compositing");
    plan->owned_bytes = sizeof(*plan);
    plan->lookup_root = SIZE_MAX;
    SrStatus status = append_node(scene, plan, scene->root, SIZE_MAX, 1, diag);
    WalkFrame stack[SR_MAX_COMPOSITE_DEPTH] = {{0}};
    size_t depth = 1;
    while (status == SR_OK && depth) {
        WalkFrame *frame = &stack[depth - 1];
        const SrNode *node = plan->nodes[frame->entry].node;
        if (frame->next_child == node->child_count) {
            --depth;
            continue;
        }
        const SrNode *child = node->children[frame->next_child++];
        size_t entry = plan->count;
        status = append_node(scene, plan, child, frame->entry, depth + 1, diag);
        if (status == SR_OK) stack[depth++] = (WalkFrame){entry, 0};
    }
    if (status != SR_OK) sr_composite_plan_free(plan);
    else *out = plan;
    return status;
}

SrStatus sr_scene_prepare_compositing(SrScene *scene, SrDiagnostics *diag) {
    if (!scene) return SR_ERR_ARGUMENT;
    sr_scene_invalidate_compositing(scene);
    SrCompositePlan *plan = NULL;
    SrStatus status = sr_composite_plan_build(scene, &plan, diag);
    if (status != SR_OK) return status;
    for (size_t i = 0; i < plan->count; ++i) {
        const SrNode *node = plan->nodes[i].node;
        if (node->type != SR_NODE_PARTICLES) continue;
        uint64_t bytes;
        if (!sr_particles_cache_bound(node, &bytes) ||
            bytes > SR_MAX_COMPOSITE_BYTES - plan->owned_bytes) {
            /* Do not rescan an overlong id while reporting bounded rejection. */
            if (diag) sr_diag_error(diag, node->source_line, "particleEmitter",
                "rate/maxParticles/id", "invalid particle metadata or cache capacity limit");
            sr_composite_plan_free(plan);
            return SR_ERR_RENDER;
        }
        plan->owned_bytes += bytes;
    }
    /* Initial preparation may admit nodes whose legacy caches were warmed.
     * Clear them only after every bound passes, before publishing ownership. */
    for (size_t i = 0; i < plan->count; ++i) {
        SrNode *node = (SrNode *)plan->nodes[i].node;
        if (node->type == SR_NODE_PARTICLES) sr_particles_invalidate(node);
    }
    scene->compositing = plan;
    return SR_OK;
}

SrStatus sr_composite_scene_ready(const SrScene *scene, SrDiagnostics *diag) {
    if (scene->compositing_required && !scene->compositing)
        return fail(scene, scene->root, diag, SR_ERR_RENDER, NULL,
                    "compositing preparation is required before evaluation");
    return SR_OK;
}

SrStatus sr_composite_node_ready(const SrScene *scene, const SrNode *node,
                                  SrDiagnostics *diag) {
    if (!scene->compositing && sr_node_uses_compositing(node))
        return fail(scene, node, diag, SR_ERR_RENDER,
                    node->blend > SR_BLEND_DIFFERENCE ? "blend" : "skewX/skewY",
                    "new feature requires compositing preparation");
    return SR_OK;
}
