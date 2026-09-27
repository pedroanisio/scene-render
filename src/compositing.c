/* SPDX-License-Identifier: Apache-2.0 */
#include "compositing_internal.h"
#include "compositor_coverage_internal.h"
#include "particles_internal.h"
#include "scene_render/markers.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static const char *element(const SrScene *scene, const SrNode *node) {
    if (!node || node == scene->root) return "composition";
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
        node->transform.skew_x.track.count || node->transform.skew_y.track.count ||
        node->type == SR_NODE_ADJUSTMENT || node->matte || node->matte_id ||
        sr_node_masks_advanced(node));
}

const char *sr_node_compositing_attribute(const SrNode *node) {
    if (node->blend > SR_BLEND_DIFFERENCE) return "blend";
    if (node->type == SR_NODE_ADJUSTMENT) return "effects";
    if (node->matte || node->matte_id) return "matte";
    if (sr_node_masks_advanced(node)) return "mask";
    return "skewX/skewY";
}

void sr_composite_plan_free(SrCompositePlan *plan) {
    if (!plan) return;
    free(plan->nodes);
    free(plan->matte_sources);
    free(plan->consumer_offsets);
    free(plan->consumers);
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
                                         SIZE_MAX, SIZE_MAX, 1, SIZE_MAX, false};
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

static SrStatus mask_fail(SrDiagnostics *diag, const SrMask *mask,
                           SrStatus status, const char *attribute,
                           const char *message) {
    if (diag) sr_diag_error(diag, mask->source_line, "mask", attribute, "%s", message);
    return status;
}

static const char *path_error(SrPathParseError error) {
    switch (error) {
    case SR_PATH_PARSE_SYNTAX: return "malformed path data";
    case SR_PATH_PARSE_BYTES: return "path text exceeds 1048576 bytes";
    case SR_PATH_PARSE_COMMANDS: return "path exceeds 65536 commands";
    case SR_PATH_PARSE_CONTOURS: return "path exceeds 4096 contours";
    case SR_PATH_PARSE_POINTS: return "path exceeds 262144 flattened points";
    case SR_PATH_PARSE_COORDINATE: return "path coordinate outside +-1e9";
    case SR_PATH_PARSE_STORAGE: return "path storage exceeds the compositing byte limit";
    case SR_PATH_PARSE_MEMORY: return "out of memory preparing mask path";
    default: return "invalid path";
    }
}

/* Masks, adjustment nodes and aggregate authored B1-3 ownership. Path masks
 * are (re)parsed here, so the immutable geometry always matches the text. */
static SrStatus prepare_nodes(const SrScene *scene, SrCompositePlan *plan,
                              SrDiagnostics *diag) {
    for (size_t i = 0; i < plan->count; ++i) {
        SrNode *node = (SrNode *)plan->nodes[i].node;
        if (node->type == SR_NODE_ADJUSTMENT) {
            if (node->child_count)
                return fail(scene, node, diag, SR_ERR_RENDER, NULL,
                            "an adjustment layer has no children");
            if (node->card)
                return fail(scene, node, diag, SR_ERR_RENDER, "threeD",
                            "adjustment depth cards are unsupported in this build");
            if (!node->effect_ref_count || !node->effect_refs)
                return fail(scene, node, diag, SR_ERR_RENDER, "effects",
                            "an adjustment layer requires resolved effects");
            for (size_t k = 0; k < node->effect_ref_count; ++k)
                if (!node->effect_refs[k])
                    return fail(scene, node, diag, SR_ERR_RENDER, "effects",
                                "an adjustment layer requires resolved effects");
        }
        if (node->matte_id) {
            size_t bytes = strlen(node->matte_id) + 1;
            if (bytes > SR_MAX_COMPOSITE_BYTES - plan->owned_bytes)
                return fail(scene, node, diag, SR_ERR_RENDER, "matte",
                            "compositing preparation storage limit exceeded");
            plan->owned_bytes += bytes;
        }
        for (size_t k = 0; k < node->mask_count; ++k) {
            SrMask *mask = &node->masks[k];
            const char *message = NULL;
            const char *attribute = sr_mask_validate(mask, &message);
            if (attribute) return mask_fail(diag, mask, SR_ERR_RENDER, attribute, message);
            if (mask->type != SR_MASK_PATH) {
                sr_mask_path_free(mask->prepared);
                mask->prepared = NULL;
                continue;
            }
            size_t text = strlen(mask->path) + 1;
            if (text > SR_MAX_MASK_PATH_BYTES + 1 ||
                text > SR_MAX_COMPOSITE_BYTES - plan->owned_bytes)
                return mask_fail(diag, mask, SR_ERR_RENDER, "path",
                                 "path text exceeds 1048576 bytes");
            plan->owned_bytes += text;
            SrPathParseInfo info;
            SrStatus status = sr_mask_path_prepare(mask,
                SR_MAX_COMPOSITE_BYTES - plan->owned_bytes, &info);
            if (status != SR_OK) {
                char message_text[160];
                snprintf(message_text, sizeof(message_text), "%s at byte %zu",
                         path_error(info.error), info.byte_offset);
                return mask_fail(diag, mask, status == SR_ERR_MEMORY
                                 ? SR_ERR_MEMORY : SR_ERR_RENDER, "path",
                                 message_text);
            }
            plan->owned_bytes += mask->prepared->owned_bytes;
        }
    }
    return SR_OK;
}

typedef struct {
    size_t from, to;            /* source indices */
    const SrNode *consumer;     /* owner of the closing matte edge */
} MatteEdge;

static int edge_order(const void *left, const void *right) {
    const MatteEdge *a = left, *b = right;
    if (a->from != b->from) return (a->from > b->from) - (a->from < b->from);
    return (a->to > b->to) - (a->to < b->to);
}

static bool node_kind_ok(const SrNode *node) {
    return node->type == SR_NODE_GROUP || node->type == SR_NODE_MEDIA ||
           node->type == SR_NODE_SHAPE || node->type == SR_NODE_PARTICLES ||
           node->type == SR_NODE_ADJUSTMENT;
}

static SrStatus cycle_fail(const SrScene *scene, SrDiagnostics *diag,
                           const SrNode *consumer, const SrNode *const *chain,
                           size_t count) {
    if (!diag) return SR_ERR_RENDER;
    char text[512];
    size_t used = 0;
    text[0] = '\0';
    for (size_t i = 0; i < count && used + 80 < sizeof(text); ++i) {
        int written = snprintf(text + used, sizeof(text) - used, "%s%.64s",
                               i ? " -> " : "", chain[i]->id ? chain[i]->id : "");
        if (written > 0) used += (size_t)written;
    }
    if (count > 0 && used + 80 < sizeof(text))
        snprintf(text + used, sizeof(text) - used, " -> %.64s",
                 chain[0]->id ? chain[0]->id : "");
    sr_diag_error(diag, consumer->source_line, element(scene, consumer), "matte",
                  "track matte dependency cycle: %s", text);
    return SR_ERR_RENDER;
}

/* Sources, suppression, consumers and a dependency order. Capturing source
 * A draws A's whole subtree, so A requires the matte of every consumer in
 * that subtree (edge A -> matte). Containment references therefore close a
 * cycle, while a group may use a descendant as its matte: capture excludes
 * ancestor mattes. Iterative colored DFS; the edge stack is bounded. */
static SrStatus prepare_mattes(const SrScene *scene, SrCompositePlan *plan,
                               SrDiagnostics *diag) {
    size_t sources = 0, consumers = 0;
    for (size_t i = 0; i < plan->count; ++i) {
        const SrNode *node = plan->nodes[i].node;
        if (!node->matte) {
            if (node->matte_id)
                return fail(scene, node, diag, SR_ERR_RENDER, "matte",
                            "track matte reference is unresolved");
            continue;
        }
        if ((unsigned)node->matte_mode > SR_MATTE_LUMA_INVERTED)
            return fail(scene, node, diag, SR_ERR_RENDER, "matteMode",
                        "unknown matte mode");
        SrCompositeNode *source = (SrCompositeNode *)sr_composite_plan_node(plan, node->matte);
        if (!source || node->matte == scene->root || !node_kind_ok(node->matte))
            return fail(scene, node, diag, SR_ERR_RENDER, "matte",
                        "track matte must reference a render node of this composition");
        if (source->matte_source == SIZE_MAX) {
            source->matte_source = sources++;
            source->suppressed = true;
        }
        if (node->matte_visible) source->suppressed = false;
        ++consumers;
    }
    if (!sources) return SR_OK;
    if (sources > SR_MAX_COMPOSITE_CAPTURES)
        return fail(scene, scene->root, diag, SR_ERR_RENDER, "matte",
                    "track matte source limit is 65536");
    SrStatus status = SR_ERR_MEMORY;
    MatteEdge *edges = NULL;
    size_t edge_count = 0, *offsets = NULL, *order = NULL, *position = NULL;
    size_t *consumer_index = NULL;
    const SrNode **by_index = sr_alloc(sources * sizeof(*by_index));
    unsigned char *color = sr_alloc(sources);
    size_t *stack = sr_alloc(sources * sizeof(*stack));
    size_t *cursor = sr_alloc(sources * sizeof(*cursor));
    order = sr_alloc(sources * sizeof(*order));
    position = sr_alloc(sources * sizeof(*position));
    offsets = sr_alloc((sources + 1) * sizeof(*offsets));
    consumer_index = sr_alloc(consumers * sizeof(*consumer_index));
    if (!by_index || !color || !stack || !cursor || !order || !position || !offsets ||
        !consumer_index)
        goto done;
    size_t capacity = 0, listed = 0;
    for (size_t i = 0; i < plan->count; ++i)
        if (plan->nodes[i].node->matte) consumer_index[listed++] = i;
    for (size_t i = 0; i < plan->count; ++i) {
        const SrCompositeNode *entry = &plan->nodes[i];
        if (entry->matte_source != SIZE_MAX) by_index[entry->matte_source] = entry->node;
        if (!entry->node->matte) continue;
        size_t to = sr_composite_plan_node(plan, entry->node->matte)->matte_source;
        for (size_t a = i; a != SIZE_MAX; a = plan->nodes[a].parent) {
            if (plan->nodes[a].matte_source == SIZE_MAX) continue;
            if (edge_count == SR_MAX_COMPOSITE_EDGES) {
                status = fail(scene, entry->node, diag, SR_ERR_RENDER, "matte",
                              "track matte dependency edge limit is 1048576");
                goto done;
            }
            if (edge_count == capacity) {
                size_t next = capacity ? capacity * 2 : 64;
                MatteEdge *grown = sr_realloc(edges, next * sizeof(*grown));
                if (!grown) goto done;
                edges = grown;
                capacity = next;
            }
            edges[edge_count++] = (MatteEdge){plan->nodes[a].matte_source, to,
                                              entry->node};
        }
    }
    /* An adjustment source's image replays its preceding siblings, whose
     * subtrees are the preorder range between its parent and itself. */
    for (size_t a = 0; a < plan->count; ++a) {
        const SrCompositeNode *adjustment = &plan->nodes[a];
        if (adjustment->matte_source == SIZE_MAX ||
            adjustment->node->type != SR_NODE_ADJUSTMENT ||
            adjustment->parent == SIZE_MAX) continue;
        /* Binary search keeps the scan proportional to the edges produced,
         * which the edge limit bounds. */
        size_t lo = 0, hi = consumers;
        while (lo < hi) {
            size_t mid = lo + (hi - lo) / 2;
            if (consumer_index[mid] <= adjustment->parent) lo = mid + 1;
            else hi = mid;
        }
        for (size_t c = lo; c < consumers && consumer_index[c] < a; ++c) {
            const SrNode *node = plan->nodes[consumer_index[c]].node;
            if (edge_count == SR_MAX_COMPOSITE_EDGES) {
                status = fail(scene, node, diag, SR_ERR_RENDER, "matte",
                              "track matte dependency edge limit is 1048576");
                goto done;
            }
            if (edge_count == capacity) {
                size_t next = capacity ? capacity * 2 : 64;
                MatteEdge *grown = sr_realloc(edges, next * sizeof(*grown));
                if (!grown) goto done;
                edges = grown;
                capacity = next;
            }
            edges[edge_count++] = (MatteEdge){adjustment->matte_source,
                sr_composite_plan_node(plan, node->matte)->matte_source, node};
        }
    }
    if (edge_count > 1) qsort(edges, edge_count, sizeof(*edges), edge_order);
    for (size_t k = 0; k <= sources; ++k) offsets[k] = 0;
    for (size_t e = 0; e < edge_count; ++e) ++offsets[edges[e].from + 1];
    for (size_t k = 0; k < sources; ++k) offsets[k + 1] += offsets[k];
    size_t emitted = 0;
    for (size_t k = 0; k < sources; ++k) color[k] = 0;
    for (size_t root = 0; root < sources; ++root) {
        if (color[root]) continue;
        size_t depth = 0;
        stack[depth++] = root;
        cursor[root] = offsets[root];
        color[root] = 1;
        while (depth) {
            size_t vertex = stack[depth - 1];
            if (cursor[vertex] == offsets[vertex + 1]) {
                color[vertex] = 2;
                order[emitted++] = vertex;
                --depth;
                continue;
            }
            const MatteEdge *edge = &edges[cursor[vertex]++];
            if (color[edge->to] == 1) {
                size_t start = depth;
                while (start > 0 && stack[start - 1] != edge->to) --start;
                const SrNode *chain[16];
                size_t count = 0;
                for (size_t k = start ? start - 1 : 0; k < depth && count < 16; ++k)
                    chain[count++] = by_index[stack[k]];
                status = cycle_fail(scene, diag, edge->consumer, chain, count);
                goto done;
            }
            if (color[edge->to] == 0) {
                color[edge->to] = 1;
                cursor[edge->to] = offsets[edge->to];
                stack[depth++] = edge->to;
            }
        }
    }
    /* Publish dependency order: capture index = position in `order`. */
    plan->matte_sources = sr_alloc(sources * sizeof(*plan->matte_sources));
    plan->consumer_offsets = sr_alloc((sources + 1) * sizeof(*plan->consumer_offsets));
    plan->consumers = sr_alloc(consumers * sizeof(*plan->consumers));
    if (!plan->matte_sources || !plan->consumer_offsets || !plan->consumers) goto done;
    for (size_t k = 0; k < sources; ++k) position[order[k]] = k;
    for (size_t k = 0; k < sources; ++k) plan->matte_sources[k] = by_index[order[k]];
    for (size_t k = 0; k <= sources; ++k) plan->consumer_offsets[k] = 0;
    for (size_t i = 0; i < plan->count; ++i) {
        SrCompositeNode *entry = &plan->nodes[i];
        if (entry->matte_source != SIZE_MAX)
            entry->matte_source = position[entry->matte_source];
    }
    for (size_t i = 0; i < plan->count; ++i) {
        const SrNode *node = plan->nodes[i].node;
        if (!node->matte) continue;
        ++plan->consumer_offsets[sr_composite_plan_node(plan, node->matte)->matte_source + 1];
    }
    for (size_t k = 0; k < sources; ++k)
        plan->consumer_offsets[k + 1] += plan->consumer_offsets[k];
    for (size_t k = 0; k < sources; ++k) cursor[k] = plan->consumer_offsets[k];
    for (size_t i = 0; i < plan->count; ++i) {
        const SrNode *node = plan->nodes[i].node;
        if (!node->matte) continue;
        size_t k = sr_composite_plan_node(plan, node->matte)->matte_source;
        plan->consumers[cursor[k]++] = node;
    }
    plan->matte_source_count = sources;
    uint64_t bytes = sources * (sizeof(*plan->matte_sources) + sizeof(size_t)) +
                     sizeof(size_t) + consumers * sizeof(*plan->consumers);
    if (bytes > SR_MAX_COMPOSITE_BYTES - plan->owned_bytes) {
        status = fail(scene, scene->root, diag, SR_ERR_RENDER, "matte",
                      "compositing preparation storage limit exceeded");
        goto done;
    }
    plan->owned_bytes += bytes;
    status = SR_OK;
done:
    if (status == SR_ERR_MEMORY)
        fail(scene, scene->root, diag, SR_ERR_MEMORY, NULL,
             "out of memory preparing track mattes");
    free(edges);
    free(consumer_index);
    free(offsets);
    free(order);
    free(position);
    free(by_index);
    free(color);
    free(stack);
    free(cursor);
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
    status = prepare_nodes(scene, plan, diag);
    if (status == SR_OK) status = prepare_mattes(scene, plan, diag);
    if (status != SR_OK) {
        sr_composite_plan_free(plan);
        return status;
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
                    sr_node_compositing_attribute(node),
                    "new feature requires compositing preparation");
    return SR_OK;
}
