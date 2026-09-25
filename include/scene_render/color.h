#ifndef SCENE_RENDER_COLOR_H
#define SCENE_RENDER_COLOR_H

#include "scene_render/compositor.h"
#include "scene_render/scene.h"

#define SR_COLOR_LUT_SIZE 65536

bool sr_color_space_parse(const char *text, SrColorSpace *space);
const char *sr_color_space_name(SrColorSpace space);
/* Exact transfer functions of a color space on [0,1] (inputs are clamped). */
double sr_color_decode(double value, SrColorSpace space);
double sr_color_encode(double value, SrColorSpace space);

/* Blend space: the project working gamut, linear-light when
 * project.linear_light, otherwise the working space's transfer-encoded
 * values. Pixels are float premultiplied RGBA. */

/* Converts a straight working-space color (as written in the XML, i.e.
 * transfer-encoded working-space values) to premultiplied blend space. */
void sr_color_to_blend(const SrProject *project, SrColor color, float out[4]);

/* Straight transfer-encoded color of `color` at `time`. With keyframes the
 * r/g/b channel tracks interpolate linear-light values which are then
 * re-encoded with color->space; alpha interpolates directly. Without keys
 * the result is exactly color->base. */
SrColor sr_anim_color_eval(const SrAnimColor *color, double time);

/* Straight transfer-encoded color `t` of the way from `a` to `b`, mixed in
 * linear light (alpha mixed directly). Returns `a` exactly when t == 0 and
 * keeps equal channels bit-exact. */
SrColor sr_color_mix_linear(SrColor a, SrColor b, double t, SrColorSpace space);

/* Converts 8-bit straight-alpha RGBA in `source` (rows `stride` bytes apart)
 * to a freshly allocated premultiplied blend-space image: transfer decode via
 * a 256-entry table, source->working gamut matrix, re-encode when the
 * project is not linear-light, premultiply. `out->px` is owned by the
 * caller; on failure `out` is left empty. */
SrStatus sr_color_image_from_rgba8(const SrProject *project,
                                   SrColorSpace source,
                                   const uint8_t *rgba8, size_t stride,
                                   uint32_t width, uint32_t height,
                                   SrImage *out);

/* Blend space -> output conversion tables (see sr_color_convert_frame). */
typedef struct {
    bool linear_light;
    bool passthrough;        /* !linear_light and working == output */
    bool identity_gamut;
    double matrix[3][3];     /* linear working -> linear output */
    float *decode;           /* working-encoded [0,1] -> linear (65536) */
    uint8_t *encode;         /* linear output [0,1] -> 8-bit code (65536) */
    uint16_t *encode16;      /* linear output [0,1] -> 16-bit code (65536) */
    unsigned bits;           /* 8 or 16: which convert entry point applies */
} SrColorOutput;

/* 8-bit output tables (sr_color_convert_frame). */
SrStatus sr_color_output_init(SrColorOutput *output, const SrProject *project,
                              SrColorSpace target);
/* `bits` 8 or 16; 16 builds the tables for sr_color_convert_frame16. */
SrStatus sr_color_output_init_bits(SrColorOutput *output,
                                   const SrProject *project,
                                   SrColorSpace target, unsigned bits);
void sr_color_output_free(SrColorOutput *output);

/* Blend space frame -> unpremultiply -> linear working -> output gamut ->
 * output transfer -> clamp -> round half up, written as 8-bit straight RGBA
 * into `rgba8` (width*height*4 bytes). Row-parallel and deterministic. */
SrStatus sr_color_convert_frame(const SrColorOutput *output,
                                const SrFrame *frame, uint8_t *rgba8,
                                unsigned threads);
/* The same conversion producing 16-bit straight RGBA: each component is
 * clamp(v) * 65535 rounded half up (output must be initialised with 16
 * bits). */
SrStatus sr_color_convert_frame16(const SrColorOutput *output,
                                  const SrFrame *frame, uint16_t *rgba16,
                                  unsigned threads);

#endif
