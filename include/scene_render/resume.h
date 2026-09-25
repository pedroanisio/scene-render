#ifndef SCENE_RENDER_RESUME_H
#define SCENE_RENDER_RESUME_H

#include "scene_render/diagnostics.h"
#include "scene_render/scene.h"

typedef struct {
    bool enabled;
    char *directory;
    size_t frame_bytes;
} SrResumeCache;

/* Frames are cached exactly as the encoder consumes them: straight RGBA of
 * `bits` (8 or 16) per component; the bit depth is part of the signature. */
SrStatus sr_resume_open(SrResumeCache *cache, const SrScene *scene,
                        const char *output_path, bool enabled, unsigned bits,
                        SrDiagnostics *diag);
bool sr_resume_load(const SrResumeCache *cache, uint64_t frame,
                    void *rgba, SrDiagnostics *diag);
SrStatus sr_resume_store(const SrResumeCache *cache, uint64_t frame,
                         const void *rgba, SrDiagnostics *diag);
void sr_resume_close(SrResumeCache *cache);

#endif
