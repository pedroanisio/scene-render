/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SR_PARTICLES_INTERNAL_H
#define SR_PARTICLES_INTERNAL_H

#include "scene_render/particles.h"
#include "compositor_resources_internal.h"

#define SR_MAX_PARTICLE_OUTPUT 10000000u
#define SR_MAX_PARTICLE_ID_BYTES 1048576u

/* Finalized authored tracks; capacity is reserved by the prepared plan even
 * before the mutex-protected cache exists. No mutation may overlap rendering. */
bool sr_particles_cache_bound(const SrNode *node, uint64_t *bytes);
void sr_particles_invalidate(SrNode *node);

/* The caller already reserves scene-owned cache capacity through its plan.
 * Returned particles require sr_composite_free with this same ledger. */
SrStatus sr_particles_eval_composite(const SrScene *scene, const SrNode *node,
                                      double time, SrCompositeResources *resources,
                                      SrParticle **particles, size_t *count);

#endif
