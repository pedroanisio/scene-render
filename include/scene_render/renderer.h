#ifndef SCENE_RENDER_RENDERER_H
#define SCENE_RENDER_RENDERER_H

#include "scene_render/diagnostics.h"
#include "scene_render/scene.h"

/* Per-frame pipeline stages timed by the renderer, in execution order. */
typedef enum {
    SR_STAGE_CLEAR,
    SR_STAGE_LIGHTING,
    SR_STAGE_COMPOSITE,
    SR_STAGE_VIEWPORT,
    SR_STAGE_EFFECTS,
    SR_STAGE_CONVERT,
    SR_STAGE_RESUME,
    SR_STAGE_ENCODE,
    SR_STAGE_COUNT
} SrStage;

/* Wall seconds and engine-process CPU seconds (all engine threads). */
typedef struct {
    double wall[SR_STAGE_COUNT];
    double cpu[SR_STAGE_COUNT];
} SrStageTimes;

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
    bool request_gpu;
    bool report_metrics;
    bool resume;
    const char *trace_path; /* JSONL: one row per frame plus a summary row */
} SrRenderOptions;

typedef struct {
    uint64_t frames;
    double render_seconds;
    double encode_seconds;
    double wall_seconds;
    long peak_self_rss_kib;
    long peak_rss_kib;           /* == peak_self_rss_kib: no child processes */
    double setup_wall_seconds;   /* asset load + physics preparation */
    double setup_cpu_seconds;
    SrStageTimes stages;         /* totals over rendered frames */
    SrStageTimes stage_max;      /* slowest single frame per stage */
    double user_seconds, system_seconds;             /* engine process */
    uint64_t audio_samples;      /* per channel, handed to the encoder */
    size_t video_sources;        /* open video decoders */
    uint64_t video_requests, video_cache_hits, video_decoded, video_seeks;
} SrRenderMetrics;

const char *sr_stage_name(SrStage stage);

SrStatus sr_render(SrScene *scene, const SrRenderOptions *options,
                   SrRenderMetrics *metrics, SrDiagnostics *diag);
SrStatus sr_write_ppm(const char *path, uint32_t width, uint32_t height,
                      const uint8_t *rgba, SrDiagnostics *diag);
/* 8-bit straight RGBA PNG through libavcodec's png encoder (bit-exact). */
SrStatus sr_write_png(const char *path, uint32_t width, uint32_t height,
                      const uint8_t *rgba, SrDiagnostics *diag);

#endif
