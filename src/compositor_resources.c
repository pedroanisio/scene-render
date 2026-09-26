/* SPDX-License-Identifier: Apache-2.0 */
#include "compositor_resources_internal.h"
#include "compositing_internal.h"
#include "timeline_internal.h"

#include <assert.h>
#include <stddef.h>
#include <stdlib.h>

typedef union {
    max_align_t alignment;
    struct {
        SrCompositeResources *owner;
        uint64_t bytes, pixels;
    } charge;
} SrResourceHeader;

static uint64_t minimum(uint64_t a, uint64_t b) {
    return a < b ? a : b;
}

void sr_composite_resources_init(SrCompositeResources *resources,
                                  const SrCompositeLimits *limits) {
    SrCompositeLimits maximum = {SR_MAX_COMPOSITE_BYTES,
        SR_MAX_COMPOSITE_PIXELS, SR_MAX_COMPOSITE_WORK};
    *resources = (SrCompositeResources){.limits = maximum};
    if (limits) {
        resources->limits.bytes = minimum(limits->bytes, maximum.bytes);
        resources->limits.pixels = minimum(limits->pixels, maximum.pixels);
        resources->limits.work = minimum(limits->work, maximum.work);
    }
}

bool sr_composite_resource_fail(SrCompositeResources *resources,
                                 SrStatus status, const char *reason) {
    if (resources && resources->status == SR_OK) {
        resources->status = status;
        resources->failure = reason;
        resources->failure_owner = resources->owner;
    }
    return false;
}

bool sr_composite_reserve(SrCompositeResources *resources, uint64_t bytes,
                           uint64_t pixels, uint64_t work) {
    if (!resources) return true;
    if (resources->status != SR_OK) return false;
    const char *failure = NULL;
    if (bytes > resources->limits.bytes - resources->bytes)
        failure = "compositing byte budget exceeded";
    else if (pixels > resources->limits.pixels - resources->pixels)
        failure = "compositing pixel budget exceeded";
    else if (work > resources->limits.work - resources->work)
        failure = "compositing work budget exceeded";
    if (failure)
        return sr_composite_resource_fail(resources, SR_ERR_RENDER, failure);
    resources->bytes += bytes;
    resources->pixels += pixels;
    resources->work += work;
    if (resources->bytes > resources->peak_bytes)
        resources->peak_bytes = resources->bytes;
    if (resources->pixels > resources->peak_pixels)
        resources->peak_pixels = resources->pixels;
    return true;
}

void sr_composite_release(SrCompositeResources *resources, uint64_t bytes,
                           uint64_t pixels) {
    if (!resources) return;
    assert(bytes <= resources->bytes && pixels <= resources->pixels);
    if (bytes > resources->bytes || pixels > resources->pixels) {
        sr_composite_resource_fail(resources, SR_ERR_RENDER,
                                    "invalid compositing resource release");
        return;
    }
    resources->bytes -= bytes;
    resources->pixels -= pixels;
}

bool sr_composite_work(SrCompositeResources *resources, uint64_t count,
                        uint64_t cost) {
    if (!resources) return true;
    if (count && cost > UINT64_MAX / count)
        return sr_composite_resource_fail(resources, SR_ERR_RENDER,
                                            "compositing work overflow");
    return sr_composite_reserve(resources, 0, 0, count * cost);
}

bool sr_composite_anim_work(SrCompositeResources *resources,
                             const SrAnimValue *value, bool length) {
    if (!resources) return true;
    const SrTrack *track = &value->track;
    if (track->count > SR_MAX_TRACK_KEYS || (track->count && !track->keys))
        return sr_composite_resource_fail(resources, SR_ERR_RENDER,
                                            "invalid compositing animation keys");
    /* Scalar evaluation has at most 16 binary-search steps and 24 Bezier
     * iterations; the remaining curve/clock/tangent branches are constant.
     * Relative keys add a second search, six-key neighborhood conversion and
     * copying the selected keys plus the temporary track descriptor. */
    uint64_t work = track->count ? 64 : 1;
    if (length && track->has_relative) {
        work += 32 + sizeof(SrTrack) / 4 + (sizeof(SrTrack) % 4 != 0);
        work += SR_TRACK_NEIGHBORHOOD *
            (2 + sizeof(SrKeyframe) / 4 + (sizeof(SrKeyframe) % 4 != 0));
    }
    return sr_composite_work(resources, work, 1);
}

SrStatus sr_composite_resource_status(const SrCompositeResources *resources) {
    return resources && resources->status != SR_OK ? resources->status
                                                   : SR_ERR_MEMORY;
}

SrCompositeOwner sr_composite_owner(SrCompositeResources *resources,
                                     SrCompositeOwner owner) {
    if (!resources) return (SrCompositeOwner){0};
    SrCompositeOwner previous = resources->owner;
    resources->owner = owner;
    return previous;
}

static bool allocation_size(SrCompositeResources *resources, size_t count,
                              size_t size, size_t *bytes) {
    if (count && size > SIZE_MAX / count)
        return sr_composite_resource_fail(resources, SR_ERR_RENDER,
                                            "compositing allocation overflow");
    *bytes = count * size;
    if (!*bytes) *bytes = 1;
    if (!resources) return true;
    if (*bytes > SIZE_MAX - sizeof(SrResourceHeader))
        return sr_composite_resource_fail(resources, SR_ERR_RENDER,
                                            "compositing allocation overflow");
    *bytes += sizeof(SrResourceHeader);
    return true;
}

void *sr_composite_alloc(SrCompositeResources *resources, size_t count,
                          size_t size, uint64_t pixels) {
    size_t bytes;
    if (!allocation_size(resources, count, size, &bytes)) return NULL;
    if (!resources) return sr_alloc(bytes);
    uint64_t work = bytes / 4 + (bytes % 4 != 0);
    if (!sr_composite_reserve(resources, bytes, pixels, work)) return NULL;
    SrResourceHeader *header = sr_alloc(bytes);
    if (!header) {
        sr_composite_release(resources, bytes, pixels);
        sr_composite_resource_fail(resources, SR_ERR_MEMORY, "out of memory");
        return NULL;
    }
    header->charge.owner = resources;
    header->charge.bytes = bytes;
    header->charge.pixels = pixels;
    return header + 1;
}

void *sr_composite_realloc(SrCompositeResources *resources, void *ptr,
                            size_t count, size_t size, uint64_t pixels) {
    size_t bytes;
    if (!allocation_size(resources, count, size, &bytes)) return NULL;
    if (!resources) return sr_realloc(ptr, bytes);
    SrResourceHeader *old = ptr ? (SrResourceHeader *)ptr - 1 : NULL;
    assert(!old || old->charge.owner == resources);
    uint64_t old_bytes = old ? old->charge.bytes : 0;
    uint64_t old_pixels = old ? old->charge.pixels : 0;
    uint64_t work = bytes / 4 + (bytes % 4 != 0);
    if (!sr_composite_reserve(resources, bytes, pixels, work)) return NULL;
    SrResourceHeader *header = sr_realloc(old, bytes);
    if (!header) {
        sr_composite_release(resources, bytes, pixels);
        sr_composite_resource_fail(resources, SR_ERR_MEMORY, "out of memory");
        return NULL;
    }
    sr_composite_release(resources, old_bytes, old_pixels);
    header->charge.owner = resources;
    header->charge.bytes = bytes;
    header->charge.pixels = pixels;
    return header + 1;
}

void sr_composite_free(SrCompositeResources *resources, void *ptr) {
    if (!ptr) return;
    if (!resources) {
        free(ptr);
        return;
    }
    SrResourceHeader *header = (SrResourceHeader *)ptr - 1;
    assert(header->charge.owner == resources);
    sr_composite_release(resources, header->charge.bytes, header->charge.pixels);
    free(header);
}

SrStatus sr_composite_scope_begin(SrCompositeScope *scope,
                                   SrCompositor *compositor,
                                   const SrScene *scene, const SrFrame *frame,
                                   const SrCompositeLimits *limits) {
    *scope = (SrCompositeScope){0};
    if (!scene->compositing_required || compositor->resources) return SR_OK;
    unsigned threads = compositor->threads;
    SrDepthBuffer *borrowed = compositor->depth;
    if (borrowed == &compositor->depth_store ||
        (borrowed && compositor->depth_store.z &&
         borrowed->z == compositor->depth_store.z))
        borrowed = NULL;
    sr_compositor_free(compositor);
    sr_compositor_init(compositor, threads);
    scope->borrowed_depth = compositor->depth = borrowed;
    scope->active = true;
    sr_composite_resources_init(&scope->resources, limits);
    compositor->resources = &scope->resources;
    sr_composite_owner(compositor->resources, (SrCompositeOwner){
        scene->root->source_line, "composition", "width/height"});
    if (!frame->width || !frame->height ||
        frame->width > SR_MAX_COVERAGE_DIMENSION ||
        frame->height > SR_MAX_COVERAGE_DIMENSION) {
        sr_composite_resource_fail(compositor->resources, SR_ERR_RENDER,
                                    "compositing target dimension exceeded");
        return SR_ERR_RENDER;
    }
    uint64_t pixels = (uint64_t)frame->width * frame->height * 4;
    uint64_t bytes = pixels * sizeof(float) + scene->compositing->owned_bytes;
    if (borrowed) {
        sr_composite_owner(compositor->resources, (SrCompositeOwner){
            scene->root->source_line, "composition", "depth"});
        if (!borrowed->z || borrowed->samples < 1 || borrowed->samples > 4 ||
            borrowed->width != frame->width * (uint32_t)borrowed->samples ||
            borrowed->height != frame->height * (uint32_t)borrowed->samples) {
            sr_composite_resource_fail(compositor->resources, SR_ERR_RENDER,
                                        "invalid borrowed compositing depth buffer");
            return SR_ERR_RENDER;
        }
        uint64_t samples = (uint64_t)borrowed->width * borrowed->height;
        bytes += samples * sizeof(double);
        pixels += samples * 2;
    }
    if (!sr_composite_reserve(compositor->resources, bytes, pixels, 0))
        return SR_ERR_RENDER;
    scope->initial_bytes = bytes;
    scope->initial_pixels = pixels;
    return SR_OK;
}

SrStatus sr_composite_scope_end(SrCompositeScope *scope,
                                 SrCompositor *compositor, SrStatus status,
                                 SrDiagnostics *diag) {
    if (!scope->active) return status;
    unsigned threads = compositor->threads;
    sr_compositor_free(compositor);
    sr_compositor_init(compositor, threads);
    compositor->depth = scope->borrowed_depth;
    SrCompositeResources *resources = &scope->resources;
    sr_composite_release(resources, scope->initial_bytes, scope->initial_pixels);
    assert(resources->bytes == 0 && resources->pixels == 0);
    if (resources->status != SR_OK) {
        SrCompositeOwner owner = resources->failure_owner;
        if (diag)
            sr_diag_error(diag, owner.line, owner.element, owner.attribute,
                          "%s", resources->failure);
        status = resources->status;
    }
    scope->active = false;
    return status;
}
