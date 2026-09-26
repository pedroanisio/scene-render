/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SR_COMPOSITOR_EVALUATION_INTERNAL_H
#define SR_COMPOSITOR_EVALUATION_INTERNAL_H

#include "compositor_resources_internal.h"

#define SR_MAX_COMPOSITE_CAMERAS 65536u
#define SR_MAX_COMPOSITE_OBJECTS 65536u
#define SR_MAX_COMPOSITE_DRAW_ITEMS (SR_MAX_COMPOSITE_NODES + SR_MAX_COMPOSITE_OBJECTS)
#define SR_MAX_COMPOSITE_CLIP_VERTICES 12u
#define SR_COMPOSITE_PROJECTIVE_WORK 1024u
#define SR_COMPOSITE_WARP_PIXEL_WORK 512u

/* Sort keys are normalized (no NaN); order is unique within a run. */
typedef struct {
    const SrNode *node;
    size_t object;
    double key;
    size_t order;
} SrDrawItem;

SrCompositeOwner sr_composite_node_owner(const SrScene *scene, const SrNode *node,
                                          const char *attribute);
bool sr_composite_view_work(SrCompositeResources *resources, const SrScene *scene,
                             double time);
bool sr_composite_world_work(SrCompositeResources *resources, const SrScene *scene,
                              const SrNode *node, bool resolved);
bool sr_composite_pivot_work(SrCompositeResources *resources, const SrScene *scene,
                              const SrNode *node, bool resolved, bool pose);
bool sr_composite_masks_work(SrCompositeResources *resources, const SrNode *node,
                              bool resolved);
bool sr_composite_color_work(SrCompositeResources *resources, const SrAnimColor *color);
bool sr_composite_object_key_work(SrCompositeResources *resources,
                                   const SrScene *scene, size_t index);
bool sr_composite_sort_items(SrCompositeResources *resources, SrDrawItem *items,
                              size_t count);

#endif
