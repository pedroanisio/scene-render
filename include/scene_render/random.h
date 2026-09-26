/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SCENE_RENDER_RANDOM_H
#define SCENE_RENDER_RANDOM_H

#include <stdint.h>

/* Stateless SplitMix64 transform; unsigned arithmetic wraps modulo 2^64. */
uint64_t sr_random_mix64(uint64_t value);

/* Legacy particle stream mapping, including index*8+stream wraparound.
 * Returns an exactly representable multiple of 2^-53 in [0,1). */
double sr_random_particle_value(uint64_t seed, uint64_t index,
                                unsigned stream);

/* Particle compatibility seed: project_seed XOR the id's historical hash.
 * A NULL id is the empty string. This is deliberately not canonical FNV-1a;
 * existing scenes use the offset 1469598103934665603. No storage is retained. */
uint64_t sr_random_particle_seed(uint64_t project_seed, const char *id);

/* Canonical per-property seed: 64-bit FNV-1a over the id bytes, one zero
 * separator byte and the domain string, XOR the project seed. A NULL id or
 * domain is the empty string. New 1.1 features use this rule. */
uint64_t sr_random_property_seed(uint64_t project_seed, const char *id,
                                 const char *domain);

/* Stateless value in [0,1) (a multiple of 2^-53) for integer pixel (x, y):
 * x and y are converted to uint64 (two's complement for negatives) and
 * mixed separately, so no width-dependent flattened index is involved. */
double sr_random_pixel_value(uint64_t seed, int64_t x, int64_t y);

#endif
