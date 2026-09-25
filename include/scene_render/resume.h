#ifndef SCENE_RENDER_RESUME_H
#define SCENE_RENDER_RESUME_H

#include "scene_render/diagnostics.h"
#include "scene_render/scene.h"

typedef struct {
    bool enabled;
    char *directory;
    size_t frame_bytes;
} SrResumeCache;

SrStatus sr_resume_open(SrResumeCache *cache, const SrScene *scene,
                        const char *output_path, bool enabled,
                        SrDiagnostics *diag);
bool sr_resume_load(const SrResumeCache *cache, uint64_t frame,
                    uint8_t *rgba, SrDiagnostics *diag);
SrStatus sr_resume_store(const SrResumeCache *cache, uint64_t frame,
                         const uint8_t *rgba, SrDiagnostics *diag);
void sr_resume_close(SrResumeCache *cache);

#endif
