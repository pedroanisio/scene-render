#ifndef SCENE_RENDER_AUDIO_H
#define SCENE_RENDER_AUDIO_H

#include "scene_render/diagnostics.h"
#include "scene_render/scene.h"

/* Longest decoded clip accepted, in seconds (memory guard). */
#define SR_AUDIO_MAX_SECONDS (4.0 * 3600.0)

/* First output sample of video frame `frame`:
 * floor(frame * rate * fps_den / fps_num), exact in 128-bit integers. */
uint64_t sr_frame_to_sample(uint64_t frame, uint32_t rate, uint32_t fps_num,
                            uint32_t fps_den);

/* Decodes the best audio stream of `path` (an audio file or a video's
 * soundtrack) in memory, resampled with swresample to `rate` Hz and
 * `channels` interleaved float channels. Samples are placed on the file's
 * timeline (late starts are padded with silence, codec priming before t=0 is
 * dropped). Clips longer than max_seconds are rejected. On failure *pcm is
 * NULL, *samples 0 and `err` receives a one-line reason; returns
 * SR_ERR_MEMORY or SR_ERR_ASSET. */
SrStatus sr_audio_decode_file(const char *path, uint32_t rate,
                              uint32_t channels, double max_seconds,
                              float **pcm, uint64_t *samples, char *err,
                              size_t errlen);

/* Decodes every asset referenced by an audioTrack once (shared by all the
 * tracks that use it) into asset->audio_pcm. */
SrStatus sr_audio_load(SrScene *scene, SrDiagnostics *diag);

/* Sample-exact mixer over the scene's audioTracks. Output sample s (at the
 * audioMix rate, counted from scene time 0) is the sum over tracks of the
 * track's source sample for s, times volume, equal-power pan and fades, then
 * clipped to [-1,1]. The result does not depend on how [first, first+count)
 * ranges are split across calls. Assets must be decoded (sr_audio_load).
 * The mixer borrows scene-owned tracks and PCM; the scene must remain
 * unchanged and outlive the mixer. */
typedef struct SrMixer SrMixer;

SrStatus sr_mixer_create(const SrScene *scene, SrMixer **out);
void sr_mixer_mix(const SrMixer *mixer, uint64_t first, size_t count,
                  float *out);
void sr_mixer_destroy(SrMixer *mixer);

#endif
