/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SR_COMPOSITING_INTERNAL_H
#define SR_COMPOSITING_INTERNAL_H

#include "scene_render/compositing.h"
#include "compositing_limits_internal.h"

typedef struct {
    const SrNode *node;       /* borrowed from the scene */
    size_t parent;           /* SIZE_MAX for root */
    size_t depth;            /* root is 1 */
    size_t mask_offset;
    size_t left, right;       /* AVL lookup links, SIZE_MAX for absent */
    unsigned height;
} SrCompositeNode;

/* Owns its array; entries follow finalized tree preorder. An AVL tree keyed
 * by pointer representation provides bounded lookup/duplicate detection. Its
 * address-dependent layout never determines rendering or resource policy. */
typedef struct SrCompositePlan {
    SrCompositeNode *nodes;
    size_t count, capacity, mask_count;
    size_t lookup_root;
    uint64_t owned_bytes;
} SrCompositePlan;

/* out must not already own storage. Builds unpublished structure before XML
 * resolution, or the final index after ordering. Successful output is
 * immutable once published; every failure leaves *out NULL. */
SrStatus sr_composite_plan_build(const SrScene *scene, SrCompositePlan **out,
                                  SrDiagnostics *diag);
void sr_composite_plan_free(SrCompositePlan *plan);
const SrCompositeNode *sr_composite_plan_node(const SrCompositePlan *plan,
                                               const SrNode *node);

bool sr_node_uses_compositing(const SrNode *node);
SrStatus sr_composite_scene_ready(const SrScene *scene, SrDiagnostics *diag);
SrStatus sr_composite_node_ready(const SrScene *scene, const SrNode *node,
                                  SrDiagnostics *diag);

#endif
