#ifndef SCENE_RENDER_OUTPUTS_H
#define SCENE_RENDER_OUTPUTS_H

#include "scene_render/diagnostics.h"
#include "scene_render/scene.h"

/* Output descriptions (schema 1.1, B1-6): codec properties, containers,
 * sequence patterns and access to every <output> of a scene. The scene
 * keeps its first output in scene->output and later ones in
 * scene->extra_outputs; use these accessors instead of either field. */

typedef struct {
    const char *name;           /* XML codec value */
    const char *encoder;        /* libavcodec encoder name */
    bool legacy;                /* a 1.0 codec (h264, h265, ffv1) */
    bool sequence;              /* one image file per frame */
    bool rate_control;          /* crf, preset, bitrate apply */
    bool gop;                   /* keyframeInterval applies */
    bool b_frames;              /* bFrames applies */
    bool loops;                 /* loopCount applies */
    bool audio;                 /* the container can carry the scene audio */
    bool resumable;             /* --resume can packet-copy segments */
    int max_crf;                /* inclusive */
    unsigned containers;        /* bit (1 << SrContainer) of allowed ones */
    SrContainer default_container;
    const char *default_pixel_format;
    const char *muxer;          /* fixed muxer (gif, apng); NULL otherwise */
} SrCodecInfo;

const SrCodecInfo *sr_codec_info(SrCodec codec);
bool sr_codec_parse(const char *text, SrCodec *codec);
bool sr_container_parse(const char *text, SrContainer *container);
const char *sr_container_name(SrContainer container);

/* Default values for an output that has no attribute yet (the defaults of
 * sr_scene_init's scene->output). Allocates the default strings. */
SrStatus sr_output_init(SrOutput *output);
void sr_output_free(SrOutput *output);

size_t sr_scene_output_count(const SrScene *scene);
SrOutput *sr_scene_output_at(SrScene *scene, size_t index);
const SrOutput *sr_scene_output_const(const SrScene *scene, size_t index);
/* Appends an initialised output (count bounded by SR_MAX_OUTPUTS by the
 * caller); NULL when out of memory. */
SrOutput *sr_scene_add_extra_output(SrScene *scene);

/* True when the output uses only 1.0 attributes and codecs, so every 1.0
 * code path (and fingerprint text) applies unchanged. */
bool sr_output_is_legacy(const SrOutput *output);

/* libavformat muxer for `output` written at `path`: "mp4", "mov",
 * "matroska", "webm", "gif" or "apng"; NULL for sequences or when the
 * container cannot be derived (*why then holds a reason). */
const char *sr_output_muxer(const SrOutput *output, const char *path,
                            const char **why);
/* Encoder input bits per component: 8, 16, or 32 (linear float, EXR). */
unsigned sr_output_input_bits(const SrOutput *output);

/* Sequence patterns: exactly one %d or %0Nd (N 1..9) conversion, %%
 * escapes. sr_sequence_format writes the file name of `number` into
 * `out`; false when the pattern is invalid or `out` is too small. */
bool sr_sequence_pattern_valid(const char *pattern);
bool sr_sequence_format(const char *pattern, uint64_t number, char *out,
                        size_t size);

/* Frame index of time t at rate num/den: ceil(t * num / den - 1e-12). */
uint64_t sr_output_frame_at(double seconds, uint32_t fps_num, uint32_t fps_den);

/* Cross-field checks and per-codec defaults, run once after loading. */
SrStatus sr_outputs_resolve(SrScene *scene, SrDiagnostics *diag);

#endif
