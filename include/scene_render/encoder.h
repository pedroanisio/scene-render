#ifndef SCENE_RENDER_ENCODER_H
#define SCENE_RENDER_ENCODER_H

#include <sys/types.h>

#include "scene_render/diagnostics.h"
#include "scene_render/scene.h"

typedef struct {
    pid_t pid;
    int input_fd;
    bool active;
    double write_seconds;
} SrEncoder;

SrStatus sr_encoder_open(SrEncoder *encoder, const SrScene *scene,
                         const char *path, unsigned threads,
                         const char *audio_path,
                         SrDiagnostics *diag);
SrStatus sr_encoder_write(SrEncoder *encoder, const uint8_t *rgba,
                          size_t byte_count, SrDiagnostics *diag);
SrStatus sr_encoder_close(SrEncoder *encoder, SrDiagnostics *diag);
bool sr_encoder_available(const char *name);

#endif
