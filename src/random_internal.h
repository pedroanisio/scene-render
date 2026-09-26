/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SCENE_RENDER_RANDOM_INTERNAL_H
#define SCENE_RENDER_RANDOM_INTERNAL_H

#include <stdint.h>

/* Shared by random.c's public wrappers and the particle hot path so moving
 * the implementation does not prevent the old per-particle inlining. */
static inline uint64_t sr_random_mix64_inline(uint64_t value) {
    value += UINT64_C(0x9E3779B97F4A7C15);
    value = (value ^ (value >> 30)) * UINT64_C(0xBF58476D1CE4E5B9);
    value = (value ^ (value >> 27)) * UINT64_C(0x94D049BB133111EB);
    return value ^ (value >> 31);
}

static inline double sr_random_particle_value_inline(uint64_t seed,
                                                      uint64_t index,
                                                      unsigned stream) {
    uint64_t bits = sr_random_mix64_inline(seed ^
        sr_random_mix64_inline(index * 8 + stream));
    return (double)(bits >> 11) * (1.0 / 9007199254740992.0);
}

#endif
