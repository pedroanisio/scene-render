/* Internal to the core (and its unit tests); not installed. */
#ifndef SR_SPATIAL_INTERNAL_H
#define SR_SPATIAL_INTERNAL_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

/* *moov (*size bytes, heap-allocated, may be reallocated) is a complete moov
 * box that grew by `extra` bytes in front of the media data it describes.
 * Adds the growth to every stco/co64 chunk offset at or past `threshold`
 * (the file offset where the original moov ended). A 32-bit stco table that
 * would overflow is promoted to co64 first; that growth (4 bytes per entry)
 * is added to the shift too, repeating until no table overflows. False
 * when the boxes are malformed, a box size would overflow, or on
 * allocation failure. */
bool sr_spatial_shift_offsets(uint8_t **moov, size_t *size, uint64_t threshold,
                              uint64_t extra);

#endif
