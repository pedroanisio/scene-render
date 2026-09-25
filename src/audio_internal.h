/* Internal to the core (and its unit tests); not installed. */
#ifndef SR_AUDIO_INTERNAL_H
#define SR_AUDIO_INTERNAL_H

#include <stdint.h>

/* round(seconds * rate) as a sample count: 0 for non-positive or NaN
 * seconds, UINT64_MAX when the product reaches 2^63 (saturated in double
 * before llround, which cannot represent it). */
uint64_t sr_audio_seconds_to_samples(double seconds, uint32_t rate);

#endif
