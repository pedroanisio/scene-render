/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SR_COMPOSITOR_RESOURCES_INTERNAL_H
#define SR_COMPOSITOR_RESOURCES_INTERNAL_H

#include "scene_render/compositor.h"
#include "compositing_limits_internal.h"

typedef struct {
    uint64_t bytes, pixels, work;
} SrCompositeLimits;

typedef struct {
    size_t line;
    const char *element, *attribute;  /* borrowed through the render */
} SrCompositeOwner;

/* Calling-thread state. Workers use reservations established before dispatch.
 * A NULL ledger selects the unchanged legacy allocator/ownership contract. */
typedef struct SrCompositeResources {
    SrCompositeLimits limits;
    uint64_t bytes, pixels, work, peak_bytes, peak_pixels;
    SrStatus status;
    const char *failure;
    SrCompositeOwner owner, failure_owner;
} SrCompositeResources;

void sr_composite_resources_init(SrCompositeResources *resources,
                                  const SrCompositeLimits *limits);
bool sr_composite_reserve(SrCompositeResources *resources, uint64_t bytes,
                           uint64_t pixels, uint64_t work);
void sr_composite_release(SrCompositeResources *resources, uint64_t bytes,
                           uint64_t pixels);
bool sr_composite_work(SrCompositeResources *resources, uint64_t count,
                        uint64_t cost);
/* Before scalar/relative-key evaluation; accepts only finalized track storage.
 * A NULL ledger leaves legacy validation and arithmetic unchanged. */
bool sr_composite_track_work(SrCompositeResources *resources,
                              const SrTrack *track, bool length);
bool sr_composite_anim_work(SrCompositeResources *resources,
                             const SrAnimValue *value, bool length);
bool sr_composite_resource_fail(SrCompositeResources *resources,
                                 SrStatus status, const char *reason);
SrStatus sr_composite_resource_status(const SrCompositeResources *resources);
SrCompositeOwner sr_composite_owner(SrCompositeResources *resources,
                                     SrCompositeOwner owner);

/* Paired private allocation API. Full capacity, alignment prefix and a
 * conservative zero/copy cost count. Growth reserves old + replacement even
 * if realloc can extend in place. A failed growth leaves ptr valid. */
void *sr_composite_alloc(SrCompositeResources *resources, size_t count,
                          size_t size, uint64_t pixels);
void *sr_composite_realloc(SrCompositeResources *resources, void *ptr,
                            size_t count, size_t size, uint64_t pixels);
void sr_composite_free(SrCompositeResources *resources, void *ptr);

/* Owns an outer compositor evaluation, reclaiming legacy caches on entry and
 * releasing frame allocations before its stack ledger expires. Nested calls
 * borrow the already attached ledger. Targets are borrowed, declared RGBA
 * capacity; an outer renderer will reserve its additional coexisting targets.
 * Lower limits are private remaining quotas/test inputs, never user options. */
typedef struct {
    SrCompositeResources resources;
    uint64_t initial_bytes, initial_pixels;
    SrDepthBuffer *borrowed_depth;
    bool active;
} SrCompositeScope;

SrStatus sr_composite_scope_begin(SrCompositeScope *scope,
                                   SrCompositor *compositor,
                                   const SrScene *scene, const SrFrame *frame,
                                   const SrCompositeLimits *limits);
SrStatus sr_composite_scope_end(SrCompositeScope *scope,
                                 SrCompositor *compositor, SrStatus status,
                                 SrDiagnostics *diag);

#endif
