#ifndef SCENE_RENDER_GPU_H
#define SCENE_RENDER_GPU_H

#include "scene_render/compositor.h"
#include "scene_render/diagnostics.h"
#include "scene_render/scene.h"

typedef struct { void *implementation; } SrGpu;

bool sr_gpu_open(SrGpu *gpu, SrDiagnostics *diag);
void sr_gpu_close(SrGpu *gpu);
const char *sr_gpu_device_name(const SrGpu *gpu);
SrStatus sr_gpu_convert_frame(SrGpu *gpu, SrFrame *frame,
                              SrColorSpace source, SrColorSpace target,
                              SrDiagnostics *diag);

#endif
