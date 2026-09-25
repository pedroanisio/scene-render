#define _POSIX_C_SOURCE 200809L
#include "scene_render/renderer.h"

#include "scene_render/assets.h"
#include "scene_render/audio.h"
#include "scene_render/camera.h"
#include "scene_render/compositor.h"
#include "scene_render/encoder.h"
#include "scene_render/effects.h"
#include "scene_render/lighting.h"
#include "scene_render/physics.h"
#include "scene_render/resume.h"

#include <errno.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <sys/stat.h>

static SrStatus sr_make_parent_dirs(const char *path, SrDiagnostics *diag) {
    char *copy = sr_strdup(path);
    if (!copy) {
        return SR_ERR_MEMORY;
    }
    for (char *cursor = copy + 1; *cursor; ++cursor) {
        if (*cursor != '/') {
            continue;
        }
        *cursor = '\0';
        if (mkdir(copy, 0775) != 0 && errno != EEXIST) {
            sr_diag_error(diag, 0, NULL, NULL, "cannot create directory '%s': %s",
                          copy, strerror(errno));
            free(copy);
            return SR_ERR_IO;
        }
        *cursor = '/';
    }
    free(copy);
    return SR_OK;
}

SrStatus sr_write_ppm(const char *path, uint32_t width, uint32_t height,
                      const uint8_t *rgba, SrDiagnostics *diag) {
    SrStatus status = sr_make_parent_dirs(path, diag);
    if (status != SR_OK) {
        return status;
    }
    FILE *file = fopen(path, "wb");
    if (!file) {
        sr_diag_error(diag, 0, NULL, NULL, "cannot write '%s': %s", path,
                      strerror(errno));
        return SR_ERR_IO;
    }
    fprintf(file, "P6\n%u %u\n255\n", width, height);
    bool ok = true;
    size_t pixels = (size_t)width * height;
    for (size_t i = 0; i < pixels; ++i) {
        uint8_t rgb[3] = {rgba[i * 4], rgba[i * 4 + 1], rgba[i * 4 + 2]};
        if (fwrite(rgb, 1, sizeof(rgb), file) != sizeof(rgb)) {
            ok = false;
            break;
        }
    }
    if (fclose(file) != 0) {
        ok = false;
    }
    if (!ok) {
        sr_diag_error(diag, 0, NULL, NULL, "failed while writing '%s'", path);
        return SR_ERR_IO;
    }
    return SR_OK;
}

static SrStatus sr_render_frame(SrScene *scene, uint64_t index,
                                SrFrame *composition, SrFrame *output,
                                unsigned threads, double *seconds,
                                SrDiagnostics *diag) {
    double start = sr_monotonic_seconds();
    sr_frame_clear(composition, scene->project.background);
    double time = (double)index * scene->project.fps_den / scene->project.fps_num;
    SrStatus status = sr_lighting_render(scene, time, composition, diag);
    if (status == SR_OK)
        status = sr_composite_scene(scene, time, composition, diag);
    if (status == SR_OK && scene->project.mode == SR_MODE_VIEWPORT)
        status = sr_camera_extract_viewport(scene, time, composition, output,
                                            threads, diag);
    if (status == SR_OK)
        status = sr_effects_apply(scene, time, output, diag);
    *seconds += sr_monotonic_seconds() - start;
    return status;
}

SrStatus sr_render(SrScene *scene, const SrRenderOptions *options,
                   SrRenderMetrics *metrics, SrDiagnostics *diag) {
    if (!scene || !options || !metrics) {
        return SR_ERR_ARGUMENT;
    }
    *metrics = (SrRenderMetrics){0};
    if (options->validate_only) {
        return SR_OK;
    }
    double wall_start = sr_monotonic_seconds();
    SrStatus status = sr_assets_load(scene, diag);
    if (status != SR_OK) {
        return status;
    }
    status = sr_physics_prepare(scene, diag);
    if (status != SR_OK) return status;
    uint32_t canvas_width = scene->project.mode == SR_MODE_VIEWPORT
        ? scene->scene360.width : scene->project.width;
    uint32_t canvas_height = scene->project.mode == SR_MODE_VIEWPORT
        ? scene->scene360.height : scene->project.height;
    SrFrame composition = {0}, viewport = {0};
    status = sr_frame_init(&composition, canvas_width, canvas_height);
    if (status != SR_OK) {
        sr_diag_error(diag, 0, NULL, NULL, "cannot allocate %ux%u RGBA frame",
                      canvas_width, canvas_height);
        return status;
    }
    SrFrame *frame = &composition;
    if (scene->project.mode == SR_MODE_VIEWPORT) {
        status = sr_frame_init(&viewport, scene->project.width,
                               scene->project.height);
        if (status != SR_OK) goto cleanup;
        frame = &viewport;
    }
    double exact_frames = scene->project.duration * scene->project.fps_num /
                          scene->project.fps_den;
    uint64_t total_frames = (uint64_t)ceil(exact_frames - 1e-12);
    uint64_t first = options->has_range ? options->first_frame : 0;
    uint64_t end = options->has_range ? options->end_frame : total_frames;
    if (end > total_frames) end = total_frames;
    if (first >= end) {
        sr_diag_error(diag, 0, NULL, NULL,
                      "empty frame range [%llu, %llu) for %llu-frame scene",
                      (unsigned long long)first, (unsigned long long)end,
                      (unsigned long long)total_frames);
        status = SR_ERR_ARGUMENT;
        goto cleanup;
    }
    if (options->preview) {
        if (options->preview_frame >= total_frames) {
            sr_diag_error(diag, 0, NULL, NULL,
                          "preview frame %llu is outside [0, %llu)",
                          (unsigned long long)options->preview_frame,
                          (unsigned long long)total_frames);
            status = SR_ERR_ARGUMENT;
            goto cleanup;
        }
        status = sr_render_frame(scene, options->preview_frame, &composition,
                                 frame, options->encoder_threads,
                                 &metrics->render_seconds, diag);
        if (status == SR_OK) {
            status = sr_write_ppm(options->preview_path, frame->width,
                                  frame->height, frame->rgba, diag);
        }
        metrics->frames = status == SR_OK ? 1 : 0;
        goto cleanup;
    }
    char *path = options->output_override
                     ? sr_strdup(options->output_override)
                     : sr_path_join(scene->base_dir, scene->output.path);
    if (!path) {
        status = SR_ERR_MEMORY;
        goto cleanup;
    }
    status = sr_make_parent_dirs(path, diag);
    char *audio_path = NULL;
    if (status == SR_OK)
        status = sr_audio_mix_to_file(scene, first, end, &audio_path, diag);
    SrEncoder encoder = {0};
    SrResumeCache resume = {0};
    if (status == SR_OK)
        status = sr_resume_open(&resume, scene, path, options->resume, diag);
    if (status == SR_OK) {
        status = sr_encoder_open(&encoder, scene, path, options->encoder_threads,
                                 audio_path, diag);
    }
    if (status == SR_OK) {
        size_t bytes = (size_t)frame->width * frame->height * 4;
        for (uint64_t index = first; index < end; ++index) {
            bool reused = sr_resume_load(&resume, index, frame->rgba, diag);
            if (!reused)
                status = sr_render_frame(scene, index, &composition, frame,
                                         options->encoder_threads,
                                         &metrics->render_seconds, diag);
            if (status == SR_OK && !reused)
                status = sr_resume_store(&resume, index, frame->rgba, diag);
            if (status != SR_OK) break;
            status = sr_encoder_write(&encoder, frame->rgba, bytes, diag);
            if (status != SR_OK) break;
            ++metrics->frames;
            if (diag->verbose && (metrics->frames % 30 == 0)) {
                sr_diag_info(diag, "rendered %llu/%llu selected frames",
                             (unsigned long long)metrics->frames,
                             (unsigned long long)(end - first));
            }
        }
    }
    if (sr_encoder_close(&encoder, diag) != SR_OK && status == SR_OK) {
        status = SR_ERR_ENCODER;
    }
    metrics->encode_seconds = encoder.write_seconds;
    sr_audio_remove_temporary(scene, &audio_path);
    sr_resume_close(&resume);
    free(path);

cleanup:
    sr_frame_free(&viewport);
    sr_frame_free(&composition);
    metrics->wall_seconds = sr_monotonic_seconds() - wall_start;
    struct rusage usage;
    if (getrusage(RUSAGE_SELF, &usage) == 0) {
        metrics->peak_rss_kib = usage.ru_maxrss;
    }
    if (getrusage(RUSAGE_CHILDREN, &usage) == 0 &&
        usage.ru_maxrss > metrics->peak_rss_kib)
        metrics->peak_rss_kib = usage.ru_maxrss;
    return status;
}
