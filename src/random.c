/* SPDX-License-Identifier: Apache-2.0 */
#include "scene_render/random.h"
#include "random_internal.h"

uint64_t sr_random_mix64(uint64_t value) {
    return sr_random_mix64_inline(value);
}

double sr_random_particle_value(uint64_t seed, uint64_t index,
                                unsigned stream) {
    return sr_random_particle_value_inline(seed, index, stream);
}

uint64_t sr_random_particle_seed(uint64_t project_seed, const char *id) {
    uint64_t hash = UINT64_C(1469598103934665603);
    for (const unsigned char *p = (const unsigned char *)(id ? id : "");
         *p; ++p) {
        hash ^= *p;
        hash *= UINT64_C(1099511628211);
    }
    return project_seed ^ hash;
}
