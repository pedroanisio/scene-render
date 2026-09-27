/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SCENE_RENDER_GIF_PALETTE_INTERNAL_H
#define SCENE_RENDER_GIF_PALETTE_INTERNAL_H

#include "scene_render/common.h"

/* Per-frame palette for GIF output (docs/design/b1-6-outputs.md §5). Each
 * frame is quantized alone: at most 256 distinct colours are kept exactly;
 * more are reduced by median cut over the sorted colour histogram. The
 * result depends only on the frame's pixels. Alpha is ignored (GIF output
 * is opaque, like the other codecs without an alpha plane). */
typedef struct SrGifPalette SrGifPalette;

/* Scratch for width x height frames; SR_ERR_MEMORY or SR_ERR_ARGUMENT. */
SrStatus sr_gif_palette_create(SrGifPalette **out, uint32_t width,
                               uint32_t height);
void sr_gif_palette_free(SrGifPalette *palette);
/* rgba: 8-bit straight RGBA, tightly packed. Writes one index per pixel
 * (rows `stride` bytes apart) and 256 palette entries 0xAARRGGBB (unused
 * entries opaque black). Returns the number of palette entries used. */
unsigned sr_gif_palette_quantize(SrGifPalette *palette, const uint8_t *rgba,
                                 uint8_t *indices, int stride,
                                 uint32_t entries[256]);

#endif
