/* SPDX-License-Identifier: Apache-2.0 */
/* Pass planning for sr_render: which outputs render, at which size and
 * rate, over which frames, into which files (B1-6). A scene with one 1.0
 * output always yields one pass with one sink and no still, whose range is
 * exactly the 1.0 range. */
#include "output_plan_internal.h"

#include "scene_render/encoder.h"
#include "scene_render/outputs.h"

#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

uint64_t sr_output_total_frames(double duration, uint32_t fps_num,
                                uint32_t fps_den) {
    double exact_frames = duration * fps_num / fps_den;
    return (uint64_t)ceil(exact_frames - 1e-12);
}

void sr_output_plan_free(SrOutputPlan *plan) {
    if (!plan) return;
    for (size_t p = 0; p < plan->pass_count; ++p) {
        SrPlanPass *pass = &plan->passes[p];
        for (size_t i = 0; i < pass->sink_count; ++i) free(pass->sinks[i].path);
        for (size_t i = 0; i < pass->still_count; ++i) free(pass->stills[i].path);
        free(pass->sinks);
        free(pass->stills);
    }
    free(plan->passes);
    *plan = (SrOutputPlan){0};
}

/* Every array below holds at most SR_MAX_OUTPUTS * (1 + SR_MAX_OUTPUT_STILLS)
 * entries, so the plan is allocated at its bound once. */
enum { SR_PLAN_CAPACITY = SR_MAX_OUTPUTS * (1 + SR_MAX_OUTPUT_STILLS) };

static SrPlanPass *find_pass(SrOutputPlan *plan, uint32_t width, uint32_t height,
                             uint32_t num, uint32_t den, bool reuse) {
    for (size_t p = 0; reuse && p < plan->pass_count; ++p) {
        SrPlanPass *pass = &plan->passes[p];
        if (pass->width == width && pass->height == height &&
            pass->fps_num == num && pass->fps_den == den)
            return pass;
    }
    SrPlanPass *pass = &plan->passes[plan->pass_count++];
    *pass = (SrPlanPass){.width = width, .height = height, .fps_num = num,
                         .fps_den = den};
    pass->sinks = calloc(SR_MAX_OUTPUTS, sizeof(*pass->sinks));
    pass->stills = calloc(SR_MAX_OUTPUTS * SR_MAX_OUTPUT_STILLS,
                          sizeof(*pass->stills));
    return pass->sinks && pass->stills ? pass : NULL;
}

/* Parses "--output-id a,b" into `selected` (document indices). */
static SrStatus select_outputs(const SrScene *scene, const char *list,
                               bool selected[SR_MAX_OUTPUTS], size_t *count,
                               SrDiagnostics *diag) {
    size_t outputs = sr_scene_output_count(scene);
    *count = 0;
    if (!list) {
        for (size_t i = 0; i < outputs; ++i) selected[i] = true;
        *count = outputs;
        return SR_OK;
    }
    for (const char *at = list;;) {
        const char *comma = strchr(at, ',');
        size_t length = comma ? (size_t)(comma - at) : strlen(at);
        size_t match = outputs;
        for (size_t i = 0; length && i < outputs; ++i) {
            const char *id = sr_scene_output_const(scene, i)->id;
            if (id && strlen(id) == length && !strncmp(id, at, length)) match = i;
        }
        if (match == outputs) {
            sr_diag_error(diag, 0, NULL, NULL,
                          "--output-id: no output has id '%.*s'", (int)length, at);
            return SR_ERR_ARGUMENT;
        }
        if (selected[match]) {
            sr_diag_error(diag, 0, NULL, NULL,
                          "--output-id: '%.*s' is listed twice", (int)length, at);
            return SR_ERR_ARGUMENT;
        }
        selected[match] = true;
        ++*count;
        if (!comma) break;
        at = comma + 1;
    }
    return SR_OK;
}

static const char *label(const SrOutput *output) {
    return output->id ? output->id : output->path;
}

/* Effective written files of the plan are pairwise distinct. */
static SrStatus check_collisions(const SrOutputPlan *plan, SrDiagnostics *diag) {
    typedef struct { const char *path; bool sequence; } Written;
    Written written[SR_PLAN_CAPACITY];
    size_t count = 0;
    for (size_t p = 0; p < plan->pass_count; ++p) {
        const SrPlanPass *pass = &plan->passes[p];
        for (size_t i = 0; i < pass->sink_count; ++i)
            written[count++] = (Written){pass->sinks[i].path,
                sr_codec_info(pass->sinks[i].output->codec)->sequence};
        for (size_t i = 0; i < pass->still_count; ++i)
            written[count++] = (Written){pass->stills[i].path, false};
    }
    for (size_t i = 0; i < count; ++i)
        for (size_t j = i + 1; j < count; ++j)
            if (sr_output_paths_collide(written[i].path, written[i].sequence,
                                        written[j].path, written[j].sequence)) {
                sr_diag_error(diag, 0, NULL, NULL,
                              "outputs would write the same file: '%s' and '%s'",
                              written[i].path, written[j].path);
                return SR_ERR_ARGUMENT;
            }
    return SR_OK;
}

static SrStatus check_pass_memory(const SrOutputPlan *plan, SrDiagnostics *diag) {
    for (size_t p = 0; p < plan->pass_count; ++p) {
        const SrPlanPass *pass = &plan->passes[p];
        if (pass->sink_count < 2) continue;
        uint64_t bytes = 0;
        for (size_t i = 0; i < pass->sink_count; ++i)
            bytes += (uint64_t)pass->width * pass->height * 4 *
                     (sr_output_input_bits(pass->sinks[i].output) / 8);
        if (bytes > SR_MAX_PASS_FRAME_BYTES) {
            sr_diag_error(diag, pass->sinks[1].output->source_line, "output", NULL,
                          "outputs sharing a %ux%u pass need %llu bytes of frame "
                          "buffers per frame (SR_MAX_PASS_FRAME_BYTES %llu)",
                          pass->width, pass->height, (unsigned long long)bytes,
                          (unsigned long long)SR_MAX_PASS_FRAME_BYTES);
            return SR_ERR_ARGUMENT;
        }
    }
    return SR_OK;
}

static SrStatus plan_stills(SrOutputPlan *plan, const SrScene *scene,
                            const SrOutput *o, const SrRenderOptions *options,
                            uint32_t width, uint32_t height, uint32_t num,
                            uint32_t den, uint64_t total, SrDiagnostics *diag) {
    for (size_t k = 0; k < o->still_count; ++k) {
        const SrStill *still = &o->stills[k];
        uint64_t frame = (uint64_t)floor(still->time * num / den + 1e-9);
        if (frame >= total) frame = total ? total - 1 : 0;
        if (options->has_range &&
            (frame < options->first_frame || frame >= options->end_frame))
            continue;
        uint32_t still_width = still->width ? still->width : width;
        double scaled = floor((double)still_width * height / width + 0.5);
        if (scaled < 1.0 || scaled > SR_MAX_OUTPUT_DIMENSION) {
            sr_diag_error(diag, still->source_line,
                          still->kind == SR_STILL_POSTER ? "poster" : "thumbnail",
                          "width", "derived still height %.0f is outside [1, %u]",
                          scaled, SR_MAX_OUTPUT_DIMENSION);
            return SR_ERR_ARGUMENT;
        }
        SrPlanPass *pass = find_pass(plan, still_width, (uint32_t)scaled, num, den,
                                     true);
        if (!pass) return SR_ERR_MEMORY;
        pass->total_frames = total;
        SrPlanStill *entry = &pass->stills[pass->still_count++];
        *entry = (SrPlanStill){o, still, sr_path_join(scene->base_dir, still->path),
                               frame};
        if (!entry->path) return SR_ERR_MEMORY;
    }
    return SR_OK;
}

SrStatus sr_output_plan_build(SrOutputPlan *plan, const SrScene *scene,
                              const SrRenderOptions *options,
                              SrDiagnostics *diag) {
    *plan = (SrOutputPlan){0};
    bool selected[SR_MAX_OUTPUTS] = {false};
    size_t chosen = 0;
    SrStatus status = select_outputs(scene, options->output_ids, selected, &chosen,
                                     diag);
    if (status != SR_OK) return status;
    bool single = options->preview || options->hash;
    if (single && options->output_ids && chosen != 1) {
        sr_diag_error(diag, 0, NULL, NULL, "%s renders one output; --output-id "
                      "names %zu", options->preview ? "--preview-frame" : "--hash",
                      chosen);
        return SR_ERR_ARGUMENT;
    }
    if (!single && options->output_override && chosen > 1) {
        sr_diag_error(diag, 0, NULL, NULL,
                      "--output needs exactly one selected output (%zu are "
                      "selected; use --output-id)", chosen);
        return SR_ERR_ARGUMENT;
    }
    plan->passes = calloc(SR_PLAN_CAPACITY, sizeof(*plan->passes));
    if (!plan->passes) return SR_ERR_MEMORY;
    const SrProject *project = &scene->project;
    size_t outputs = sr_scene_output_count(scene);
    bool active = false;
    for (size_t i = 0; i < outputs && status == SR_OK; ++i) {
        if (!selected[i]) continue;
        const SrOutput *o = sr_scene_output_const(scene, i);
        const SrCodecInfo *info = sr_codec_info(o->codec);
        if (options->resume && !single && !info->resumable) {
            sr_diag_error(diag, o->source_line, "output", "codec",
                          "--resume cannot resume the %s output '%s'", info->name,
                          label(o));
            return SR_ERR_ARGUMENT;
        }
        uint32_t width = o->width ? o->width : project->width;
        uint32_t height = o->height ? o->height : project->height;
        uint32_t num = o->fps_num ? o->fps_num : project->fps_num;
        uint32_t den = o->fps_num ? o->fps_den : project->fps_den;
        uint64_t total = sr_output_total_frames(project->duration, num, den);
        uint64_t first = sr_output_frame_at(o->start, num, den);
        uint64_t end = o->has_end ? sr_output_frame_at(o->end, num, den) : total;
        if (options->has_range) {
            if (options->first_frame > first) first = options->first_frame;
            if (options->end_frame < end) end = options->end_frame;
        }
        if (end > total) end = total;
        SrPlanPass *pass = find_pass(plan, width, height, num, den,
                                     !single && !options->resume);
        if (!pass) return SR_ERR_MEMORY;
        pass->total_frames = total;
        SrPlanSink *sink = &pass->sinks[pass->sink_count++];
        *sink = (SrPlanSink){o, options->output_override
                                 ? sr_strdup(options->output_override)
                                 : sr_path_join(scene->base_dir, o->path),
                             first, end};
        if (!sink->path) return SR_ERR_MEMORY;
        if (first < end) {
            active = true;
        } else if (outputs > 1 && !single) {
            sr_diag_info(diag, "output '%s' has no frames in the selected range; "
                         "skipped", label(o));
        }
        if (!single)
            status = plan_stills(plan, scene, o, options, width, height, num, den,
                                 total, diag);
        if (single) break;
    }
    (void)active;
    if (status == SR_OK && !single) status = check_collisions(plan, diag);
    if (status == SR_OK && !single) status = check_pass_memory(plan, diag);
    return status;
}
