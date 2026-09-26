/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SCENE_RENDER_OUTPUT_PLAN_INTERNAL_H
#define SCENE_RENDER_OUTPUT_PLAN_INTERNAL_H

#include "scene_render/renderer.h"

/* Render passes of one sr_render call (docs/design/b1-6-outputs.md §4).
 * A pass renders the composition once per frame at one size and rate;
 * each sink converts and encodes those frames; stills render single frames
 * of the pass. Everything is owned by the plan. */

/* Aggregate per-frame sink buffers of one shared pass (x3 writer slots). */
#define SR_MAX_PASS_FRAME_BYTES (UINT64_C(1) << 30)

typedef struct {
    const SrOutput *output;     /* borrowed from the scene */
    char *path;                 /* effective path (override or joined) */
    uint64_t first, end;        /* frames at the pass rate; empty: inactive */
} SrPlanSink;

typedef struct {
    const SrOutput *output;
    const SrStill *still;
    char *path;
    uint64_t frame;             /* at the pass rate */
} SrPlanStill;

typedef struct {
    uint32_t width, height, fps_num, fps_den;
    uint64_t total_frames;      /* composition frames at this rate */
    SrPlanSink *sinks;
    size_t sink_count;
    SrPlanStill *stills;
    size_t still_count;
} SrPlanPass;

typedef struct {
    SrPlanPass *passes;
    size_t pass_count;
    bool active;                /* some sink has a frame or some still exists */
    bool single_output;         /* the scene declares one output */
} SrOutputPlan;

/* True when the pass has a frame to encode or a still to write. */
bool sr_plan_pass_active(const SrPlanPass *pass);

/* Selection, ranges, grouping and effective paths for `options`, checked
 * before anything renders: SR_ERR_ARGUMENT (diagnostic) for CLI conflicts,
 * SR_ERR_MEMORY. The plan must be freed even on failure. */
SrStatus sr_output_plan_build(SrOutputPlan *plan, const SrScene *scene,
                              const SrRenderOptions *options,
                              SrDiagnostics *diag);
void sr_output_plan_free(SrOutputPlan *plan);

/* Frame count of the composition at a rate (the project formula). */
uint64_t sr_output_total_frames(double duration, uint32_t fps_num,
                                uint32_t fps_den);

#endif
