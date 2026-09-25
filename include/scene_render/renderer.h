#ifndef SCENE_RENDER_RENDERER_H
#define SCENE_RENDER_RENDERER_H

#include "scene_render/diagnostics.h"
#include "scene_render/scene.h"

#include <stdio.h>

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

#define SR_RESUME_DEFAULT_SEGMENT_FRAMES 150u

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
    /* Segmented resume: the range is rendered in independently encoded,
     * video-only segments of segment_frames frames under OUTPUT.parts/,
     * reused by a rerun with an identical manifest, then muxed into the
     * output by packet copy with one audio encode over the whole range. */
    bool resume;
    uint32_t segment_frames;    /* 0 means SR_RESUME_DEFAULT_SEGMENT_FRAMES */
    bool keep_parts;            /* keep OUTPUT.parts/ after success */
    /* Hash mode: render the range without encoding and print
     * "<frame> <FNV-1a 64 hex>" per frame over the exact bytes the encoder
     * would receive, then "audio <hex>" over the range's mixed float PCM
     * when the scene has audio. NULL hash_stream means stdout. */
    bool hash;
    FILE *hash_stream;
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
    uint64_t preview_hash;       /* FNV-1a 64 of the 8-bit preview RGBA */
    uint64_t segments_rendered;  /* --resume: encoded by this run */
    uint64_t segments_reused;    /* --resume: kept from an earlier run */
    uint64_t physics_steps;      /* fixed steps simulated by this run */
    bool physics_cache_hit;      /* samples restored from the physics cache */
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
