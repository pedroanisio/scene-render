#include "scene_render/cli_args.h"
#include "scene_render/renderer.h"
#include "scene_render/xml.h"

#include <libavutil/log.h>

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static SrStatus apply_quality(SrScene *scene, SrQuality quality) {
    const char *preset;
    switch (quality) {
    case SR_QUALITY_LOW: scene->output.crf = 28; preset = "veryfast"; break;
    case SR_QUALITY_MEDIUM: scene->output.crf = 23; preset = "medium"; break;
    case SR_QUALITY_HIGH: scene->output.crf = 18; preset = "slow"; break;
    case SR_QUALITY_KEEP:
    default: return SR_OK;
    }
    char *copy = sr_strdup(preset);
    if (!copy) return SR_ERR_MEMORY;
    free(scene->output.preset);
    scene->output.preset = copy;
    scene->output.bitrate = 0;
    return SR_OK;
}

/* Command-line overrides of the loaded scene. */
static SrStatus apply_overrides(SrScene *scene, const SrCliOptions *options) {
    if (options->override_mode) {
        if (options->mode != SR_MODE_STANDARD && !scene->scene360.enabled) {
            fprintf(stderr, "error: --mode %s requires a scene360 element\n",
                    options->mode == SR_MODE_VIEWPORT ? "viewport" : "equirectangular");
            return SR_ERR_ARGUMENT;
        }
        scene->project.mode = options->mode;
        if (options->mode == SR_MODE_EQUIRECTANGULAR) {
            scene->project.width = scene->scene360.width;
            scene->project.height = scene->scene360.height;
        }
    }
    if (options->override_resolution) {
        scene->project.width = options->width;
        scene->project.height = options->height;
        if (scene->project.mode == SR_MODE_EQUIRECTANGULAR) {
            if ((uint64_t)options->width != (uint64_t)options->height * 2U) {
                fprintf(stderr, "error: equirectangular --resolution must be 2:1\n");
                return SR_ERR_ARGUMENT;
            }
            scene->scene360.width = options->width;
            scene->scene360.height = options->height;
        }
    }
    if (options->override_fps) {
        scene->project.fps_num = options->fps_num;
        scene->project.fps_den = options->fps_den;
    }
    if (options->physics_cache_dir) {
        char *dir = sr_strdup(options->physics_cache_dir);
        if (!dir) return SR_ERR_MEMORY;
        free(scene->physics.cache_dir);
        scene->physics.cache_dir = dir;
    }
    return apply_quality(scene, options->quality);
}

static void print_metrics(const SrRenderMetrics *metrics) {
    double fps = metrics->wall_seconds > 0.0
                     ? metrics->frames / metrics->wall_seconds
                     : 0.0;
    /* Encoding runs in-process, so peak_rss_kib is the whole cost. */
    fprintf(stderr,
            "metrics: frames=%llu render_s=%.6f encode_s=%.6f "
            "wall_s=%.6f fps=%.3f peak_rss_kib=%ld "
            "self_peak_rss_kib=%ld "
            "user_s=%.3f sys_s=%.3f setup_s=%.3f audio_samples=%llu\n",
            (unsigned long long)metrics->frames, metrics->render_seconds,
            metrics->encode_seconds, metrics->wall_seconds, fps,
            metrics->peak_rss_kib, metrics->peak_self_rss_kib,
            metrics->user_seconds, metrics->system_seconds,
            metrics->setup_wall_seconds,
            (unsigned long long)metrics->audio_samples);
    fprintf(stderr,
            "video: sources=%zu requests=%llu cache_hits=%llu "
            "decoded=%llu seeks=%llu\n",
            metrics->video_sources,
            (unsigned long long)metrics->video_requests,
            (unsigned long long)metrics->video_cache_hits,
            (unsigned long long)metrics->video_decoded,
            (unsigned long long)metrics->video_seeks);
    fprintf(stderr, "physics: steps=%llu cache_hit=%d\n",
            (unsigned long long)metrics->physics_steps,
            (int)metrics->physics_cache_hit);
    fprintf(stderr, "resume: segments_rendered=%llu segments_reused=%llu\n",
            (unsigned long long)metrics->segments_rendered,
            (unsigned long long)metrics->segments_reused);
    /* Per stage: total wall seconds / total engine CPU seconds. */
    fputs("stages:", stderr);
    for (int stage = 0; stage < SR_STAGE_COUNT; ++stage)
        fprintf(stderr, " %s=%.3f/%.3f", sr_stage_name((SrStage)stage),
                metrics->stages.wall[stage], metrics->stages.cpu[stage]);
    fputc('\n', stderr);
}

/* Flushes stdout; a failed write anywhere before is an I/O error (7). */
static SrStatus finish_stdout(SrStatus status) {
    if (fflush(stdout) != 0 || ferror(stdout)) {
        if (status == SR_OK) {
            fprintf(stderr, "error: cannot write to standard output\n");
            return SR_ERR_IO;
        }
    }
    return status;
}

int main(int argc, char **argv) {
    SrCliOptions options;
    char error[256];
    if (sr_cli_parse(argc, argv, &options, error, sizeof(error)) != SR_OK) {
        fprintf(stderr, "%s\n\n%s", error, sr_cli_usage());
        return SR_ERR_ARGUMENT;
    }
    if (options.help) {
        fputs(sr_cli_usage(), stdout);
        return finish_stdout(SR_OK);
    }
    if (options.version) {
        printf("scene-render %s\n", SR_VERSION);
        return finish_stdout(SR_OK);
    }
    if (options.print_schema) {
        size_t length = 0;
        const char *xsd = sr_scene_schema_text(&length);
        return finish_stdout(fwrite(xsd, 1, length, stdout) == length
                                 ? SR_OK : SR_ERR_IO);
    }
    SrDiagnostics diag;
    sr_diag_init(&diag, options.scene_path, stderr);
    diag.verbose = options.verbose;
    /* libav reports through diagnostics; its own log only when verbose. */
    av_log_set_level(options.verbose ? AV_LOG_INFO : AV_LOG_ERROR);
    SrScene scene;
    SrStatus status = sr_scene_load_xml_report(options.scene_path, &scene, &diag,
                                                options.report_unsupported);
    if (status != SR_OK) return status;
    status = apply_overrides(&scene, &options);
    if (status != SR_OK) {
        sr_scene_free(&scene);
        return status;
    }
    char preview_name[40];
    if (options.render.preview && !options.render.preview_path) {
        snprintf(preview_name, sizeof(preview_name), "frame-%06llu.png",
                 (unsigned long long)options.render.preview_frame);
        options.render.preview_path = preview_name;
    }
    SrRenderMetrics metrics;
    status = sr_render(&scene, &options.render, &metrics, &diag);
    if (status == SR_OK && options.render.validate_only) {
        printf("valid: %s (%zu assets, %zu top-level layers)\n",
               options.scene_path, scene.asset_count, scene.root->child_count);
    }
    if (status == SR_OK && options.render.preview) {
        printf("%s %016llx\n", options.render.preview_path,
               (unsigned long long)metrics.preview_hash);
    }
    status = finish_stdout(status);
    if (options.metrics) print_metrics(&metrics);
    sr_scene_free(&scene);
    return status;
}
