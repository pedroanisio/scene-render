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
    size_t matte_source;      /* capture index when a matte source, else SIZE_MAX */
    bool suppressed;          /* matte source not drawn normally */
} SrCompositeNode;

/* Owns its array; entries follow finalized tree preorder. An AVL tree keyed
 * by pointer representation provides bounded lookup/duplicate detection. Its
 * address-dependent layout never determines rendering or resource policy. */
typedef struct SrCompositePlan {
    SrCompositeNode *nodes;
    size_t count, capacity, mask_count;
    size_t lookup_root;
    uint64_t owned_bytes;
    /* Track-matte sources in dependency order (a source's own matte
     * requirements precede it); consumers of source k are
     * consumers[consumer_offsets[k] .. consumer_offsets[k + 1]). */
    const SrNode **matte_sources;
    size_t matte_source_count;
    size_t *consumer_offsets;
    const SrNode **consumers;
} SrCompositePlan;

/* out must not already own storage. Builds unpublished structure before XML
 * resolution, or the final index after ordering. Successful output is
 * immutable once published; every failure leaves *out NULL. */
SrStatus sr_composite_plan_build(const SrScene *scene, SrCompositePlan **out,
                                  SrDiagnostics *diag);
void sr_composite_plan_free(SrCompositePlan *plan);
const SrCompositeNode *sr_composite_plan_node(const SrCompositePlan *plan,
                                               const SrNode *node);

struct SrMaskPath;
void sr_mask_path_free(struct SrMaskPath *path);  /* compositor_coverage.c */

bool sr_node_uses_compositing(const SrNode *node);
/* The attribute named by diagnostics about the node's new features. */
const char *sr_node_compositing_attribute(const SrNode *node);
SrStatus sr_composite_scene_ready(const SrScene *scene, SrDiagnostics *diag);
SrStatus sr_composite_node_ready(const SrScene *scene, const SrNode *node,
                                  SrDiagnostics *diag);

#endif
