/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SCENE_RENDER_OUTPUT_MEDIA_INTERNAL_H
#define SCENE_RENDER_OUTPUT_MEDIA_INTERNAL_H

#include "scene_render/color.h"
#include "scene_render/diagnostics.h"
#include "scene_render/raster.h"
#include "scene_render/scene.h"

/* sr_color_decode's curve, sign-symmetric and not clamped. */
double sr_output_decode_extended(double value, SrColorSpace space);

/* Blend-space frame -> premultiplied linear light in the output gamut
 * (`color` from sr_color_output_init_bits for the output colour space),
 * 32-bit float RGBA, not clamped (EXR). Row-parallel, deterministic. */
SrStatus sr_output_convert_linear(const SrColorOutput *color,
                                  const SrProject *project, const SrFrame *frame,
                                  float *rgba, unsigned threads);

/* Writes a poster/thumbnail: PNG (8-bit RGBA, as previews) or baseline
 * JPEG (4:2:0, qscale from still->quality). The parent directory must
 * exist. */
SrStatus sr_output_write_still(const SrStill *still, const char *path,
                               uint32_t width, uint32_t height,
                               const uint8_t *rgba, SrDiagnostics *diag);

#endif
