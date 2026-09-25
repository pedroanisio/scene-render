#ifndef SCENE_RENDER_RENDERER_H
#define SCENE_RENDER_RENDERER_H

#include "scene_render/diagnostics.h"
#include "scene_render/scene.h"

typedef struct {
    bool validate_only;
    bool preview;
    uint64_t preview_frame;
    const char *preview_path;
    bool has_range;
    uint64_t first_frame;
    uint64_t end_frame;
    const char *output_override;
    unsigned encoder_threads;
    bool report_metrics;
    bool resume;
} SrRenderOptions;

typedef struct {
    uint64_t frames;
    double render_seconds;
    double encode_seconds;
    double wall_seconds;
    long peak_rss_kib;
} SrRenderMetrics;

SrStatus sr_render(SrScene *scene, const SrRenderOptions *options,
                   SrRenderMetrics *metrics, SrDiagnostics *diag);
SrStatus sr_write_ppm(const char *path, uint32_t width, uint32_t height,
                      const uint8_t *rgba, SrDiagnostics *diag);

#endif
