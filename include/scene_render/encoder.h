#ifndef SCENE_RENDER_ENCODER_H
#define SCENE_RENDER_ENCODER_H

#include "scene_render/diagnostics.h"
#include "scene_render/scene.h"

/* In-process libavformat/libavcodec encoder. Video frames are packed
 * straight-alpha RGBA in the output color space: 8 bits per component, or
 * 16 bits (native endian, 0..65535) when the output pixel format carries
 * more than 8 bits per component (see sr_encoder_input_bits). Frames get
 * constant-rate timestamps 0..N-1 in units of 1/fps; audio is interleaved
 * float PCM with timestamps 0.. in units of 1/sample_rate. Identical inputs
 * produce identical bytes (bit-exact container and codec flags). */
typedef struct SrEncoder SrEncoder;

typedef struct {
    uint32_t sample_rate;
    uint32_t channels;      /* 0: no audio stream */
} SrEncoderAudio;

/* Bits per RGBA component the encoder consumes for this output pixel format
 * name: 16 when any component of the format has more than 8 bits, 8
 * otherwise (also for unknown names, which sr_encoder_open rejects). */
unsigned sr_encoder_input_bits(const char *pixel_format);

/* Opens `path` (.mp4/.mov -> MP4/QuickTime, .mkv -> Matroska) for the scene's
 * project size/rate and SrOutput settings. `threads` 0 means one encoder
 * thread per online CPU. On failure *out is NULL and a diagnostic names the
 * cause; SR_ERR_MEMORY, SR_ERR_IO (output file) or SR_ERR_ENCODER. */
SrStatus sr_encoder_open(SrEncoder **out, const SrScene *scene,
                         const char *path, unsigned threads,
                         const SrEncoderAudio *audio, SrDiagnostics *diag);
/* Bits per component this encoder expects in sr_encoder_write_video. */
unsigned sr_encoder_bits(const SrEncoder *encoder);
/* One frame: width*height*4 components of 8 or 16 bits, tightly packed. */
SrStatus sr_encoder_write_video(SrEncoder *encoder, const void *rgba,
                                SrDiagnostics *diag);
/* `samples` frames of interleaved float PCM (channels values each); values
 * outside [-1,1] are clipped. */
SrStatus sr_encoder_write_audio(SrEncoder *encoder, const float *pcm,
                                size_t samples, SrDiagnostics *diag);
/* Flushes both encoders, writes the trailer and closes the file. When a
 * flush step fails the remaining steps still run (the trailer is written,
 * so an MP4 keeps its moov box) and the first failure is returned. After
 * the first call, writes are refused; a second call returns
 * SR_ERR_ARGUMENT after success and SR_ERR_ENCODER after a failure. The
 * audio padding of a fixed-frame codec's last frame is marked for discard
 * (Matroska DiscardPadding; MP4 uses the last packet's duration). */
SrStatus sr_encoder_finish(SrEncoder *encoder, SrDiagnostics *diag);
/* Releases everything; safe on NULL and on unfinished encoders. When the
 * header was written but not the trailer, the trailer is written first so
 * the partial file stays readable. */
void sr_encoder_destroy(SrEncoder *encoder);
/* Seconds spent inside write/finish calls (colour conversion + encode). */
double sr_encoder_seconds(const SrEncoder *encoder);
/* Name of the libavcodec video encoder in use ("none" for NULL). */
const char *sr_encoder_name(const SrEncoder *encoder);
/* True when libavcodec has an encoder of this name. */
bool sr_encoder_available(const char *name);

#endif
