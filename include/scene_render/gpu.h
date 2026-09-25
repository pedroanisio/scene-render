#ifndef SCENE_RENDER_GPU_H
#define SCENE_RENDER_GPU_H

#include "scene_render/compositor.h"
#include "scene_render/diagnostics.h"
#include "scene_render/scene.h"

typedef struct { void *implementation; } SrGpu;

bool sr_gpu_open(SrGpu *gpu, SrDiagnostics *diag);
void sr_gpu_close(SrGpu *gpu);
const char *sr_gpu_device_name(const SrGpu *gpu);
/* GPU twin of sr_color_convert_frame: float premultiplied blend-space
 * frame in, 8-bit straight RGBA out. The CPU path remains the reference. */
SrStatus sr_gpu_convert_frame(SrGpu *gpu, const SrFrame *frame, uint8_t *rgba8,
                              const SrProject *project, SrColorSpace target,
                              SrDiagnostics *diag);

#endif
