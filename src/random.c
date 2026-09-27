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

uint64_t sr_random_property_seed(uint64_t project_seed, const char *id,
                                 const char *domain) {
    uint64_t hash = UINT64_C(14695981039346656037);
    for (const unsigned char *p = (const unsigned char *)(id ? id : "");
         *p; ++p) {
        hash ^= *p;
        hash *= UINT64_C(1099511628211);
    }
    hash ^= 0;  /* the zero separator byte */
    hash *= UINT64_C(1099511628211);
    for (const unsigned char *p = (const unsigned char *)(domain ? domain : "");
         *p; ++p) {
        hash ^= *p;
        hash *= UINT64_C(1099511628211);
    }
    return project_seed ^ hash;
}

double sr_random_pixel_value(uint64_t seed, int64_t x, int64_t y) {
    uint64_t ux = (uint64_t)x, uy = (uint64_t)y;
    uint64_t bits = sr_random_mix64_inline(seed ^ sr_random_mix64_inline(ux));
    bits = sr_random_mix64_inline(bits ^
        sr_random_mix64_inline(uy ^ UINT64_C(0xD1B54A32D192ED03)));
    return (double)(bits >> 11) * (1.0 / 9007199254740992.0);
}
