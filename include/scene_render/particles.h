#ifndef SCENE_RENDER_PARTICLES_H
#define SCENE_RENDER_PARTICLES_H

#include "scene_render/scene.h"

/* One live particle, in the emitter's local space. */
typedef struct {
    uint64_t index;             /* emission order */
    double x, y;
    double radius;              /* canvas pixels (not scaled by transforms) */
    SrColor color;              /* straight, transfer-encoded working space */
} SrParticle;

/* The emitter's random seed: its `seed` attribute when given, otherwise
 * the project seed XOR a hash of the node id. */
uint64_t sr_particles_seed(const SrScene *scene, const SrNode *node);

/* Uniform double in [0, 1) for (seed, particle index, stream), stateless. */
double sr_particles_random(uint64_t seed, uint64_t index, unsigned stream);

/* Evaluates the particles of emitter `node` alive at scene time `time`.
 * Closed form per particle: nothing depends on other frames or on thread
 * count. Emission-time parameters (rate, speed, spread, size, lifetime,
 * direction, colors) are sampled at each particle's birth. At most
 * node->particle_max particles are returned (the newest when the cap
 * binds), oldest first. *particles is malloc'd (NULL when *count is 0). */
SrStatus sr_particles_eval(const SrScene *scene, const SrNode *node,
                           double time, SrParticle **particles, size_t *count);

#endif
