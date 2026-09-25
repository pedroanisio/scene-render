#ifndef SCENE_RENDER_AUDIO_H
#define SCENE_RENDER_AUDIO_H

#include "scene_render/diagnostics.h"
#include "scene_render/scene.h"

SrStatus sr_audio_mix_to_file(SrScene *scene, uint64_t first_frame,
                              uint64_t end_frame, char **mixed_path,
                              SrDiagnostics *diag);
void sr_audio_remove_temporary(SrScene *scene, char **mixed_path);

#endif
