#include "scene_render/renderer.h"
#include "scene_render/xml.h"

#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    const char *scene_path;
    SrRenderOptions render;
    bool verbose;
    bool metrics;
    bool override_resolution;
    uint32_t width, height;
    bool override_fps;
    uint32_t fps_num, fps_den;
    const char *quality;
    bool requested_gpu;
} CliOptions;

static void usage(FILE *stream) {
    fprintf(stream,
            "scene-render %s\n"
            "Usage: scene-render --scene FILE [options]\n\n"
            "  --output FILE            Override XML output path\n"
            "  --validate               Validate without decoding or rendering\n"
            "  --frame-range A:B        Render half-open frame range [A,B)\n"
            "  --preview-frame N        Write one deterministic PPM frame\n"
            "  --preview-out FILE       Preview path (default build/preview.ppm)\n"
            "  --resolution WIDTHxHEIGHT\n"
            "  --fps N or N/D           Override frame rate\n"
            "  --quality low|medium|high\n"
            "  --threads auto|N         Render/encoder worker count\n"
            "  --renderer cpu|gpu       Select backend (gpu falls back to CPU)\n"
            "  --resume                  Reuse deterministic cached RGBA frames\n"
            "  --metrics                 Print timing and peak memory\n"
            "  --verbose                 Verbose diagnostics\n"
            "  --version                 Print version\n",
            SR_VERSION);
}

static bool parse_pair(const char *text, char separator, uint64_t *a,
                       uint64_t *b) {
    char *copy = sr_strdup(text);
    if (!copy) return false;
    char *middle = strchr(copy, separator);
    bool ok = false;
    if (middle) {
        *middle++ = '\0';
        ok = sr_parse_u64(copy, a) && sr_parse_u64(middle, b);
    }
    free(copy);
    return ok;
}

static bool parse_resolution(const char *text, uint32_t *width,
                             uint32_t *height) {
    uint64_t a, b;
    if (!parse_pair(text, 'x', &a, &b) || a == 0 || b == 0 ||
        a > UINT32_MAX || b > UINT32_MAX) {
        return false;
    }
    *width = (uint32_t)a;
    *height = (uint32_t)b;
    return true;
}

static bool parse_fps(const char *text, uint32_t *num, uint32_t *den) {
    uint64_t a, b = 1;
    const char *slash = strchr(text, '/');
    if (slash) {
        if (!parse_pair(text, '/', &a, &b)) return false;
    } else if (!sr_parse_u64(text, &a)) {
        return false;
    }
    if (a == 0 || b == 0 || a > UINT32_MAX || b > UINT32_MAX) return false;
    *num = (uint32_t)a;
    *den = (uint32_t)b;
    return true;
}

static bool require_value(int argc, char **argv, int *index, const char **value) {
    if (*index + 1 >= argc) {
        fprintf(stderr, "error: %s requires a value\n", argv[*index]);
        return false;
    }
    *value = argv[++*index];
    return true;
}

static int parse_cli(int argc, char **argv, CliOptions *options) {
    *options = (CliOptions){0};
    options->render.preview_path = "build/preview.ppm";
    for (int i = 1; i < argc; ++i) {
        const char *value = NULL;
        if (strcmp(argv[i], "--scene") == 0) {
            if (!require_value(argc, argv, &i, &options->scene_path)) return 2;
        } else if (strcmp(argv[i], "--output") == 0) {
            if (!require_value(argc, argv, &i, &options->render.output_override)) return 2;
        } else if (strcmp(argv[i], "--validate") == 0) {
            options->render.validate_only = true;
        } else if (strcmp(argv[i], "--frame-range") == 0) {
            if (!require_value(argc, argv, &i, &value) ||
                !parse_pair(value, ':', &options->render.first_frame,
                            &options->render.end_frame)) {
                fprintf(stderr, "error: --frame-range expects A:B\n");
                return 2;
            }
            options->render.has_range = true;
        } else if (strcmp(argv[i], "--preview-frame") == 0) {
            if (!require_value(argc, argv, &i, &value) ||
                !sr_parse_u64(value, &options->render.preview_frame)) {
                fprintf(stderr, "error: --preview-frame expects an integer\n");
                return 2;
            }
            options->render.preview = true;
        } else if (strcmp(argv[i], "--preview-out") == 0) {
            if (!require_value(argc, argv, &i, &options->render.preview_path)) return 2;
        } else if (strcmp(argv[i], "--resolution") == 0) {
            if (!require_value(argc, argv, &i, &value) ||
                !parse_resolution(value, &options->width, &options->height)) {
                fprintf(stderr, "error: --resolution expects WIDTHxHEIGHT\n");
                return 2;
            }
            options->override_resolution = true;
        } else if (strcmp(argv[i], "--fps") == 0) {
            if (!require_value(argc, argv, &i, &value) ||
                !parse_fps(value, &options->fps_num, &options->fps_den)) {
                fprintf(stderr, "error: --fps expects N or N/D\n");
                return 2;
            }
            options->override_fps = true;
        } else if (strcmp(argv[i], "--quality") == 0) {
            if (!require_value(argc, argv, &i, &options->quality)) return 2;
        } else if (strcmp(argv[i], "--threads") == 0) {
            if (!require_value(argc, argv, &i, &value)) return 2;
            if (strcmp(value, "auto") == 0) {
                options->render.encoder_threads = 0;
            } else {
                uint32_t count;
                if (!sr_parse_u32(value, &count) || count == 0) {
                    fprintf(stderr, "error: --threads expects auto or a positive integer\n");
                    return 2;
                }
                options->render.encoder_threads = count;
            }
        } else if (strcmp(argv[i], "--renderer") == 0) {
            if (!require_value(argc, argv, &i, &value)) return 2;
            if (strcmp(value, "gpu") == 0) {
                options->requested_gpu = true;
            } else if (strcmp(value, "cpu") != 0) {
                fprintf(stderr, "error: --renderer expects cpu or gpu\n");
                return 2;
            }
        } else if (strcmp(argv[i], "--resume") == 0) {
            options->render.resume = true;
        } else if (strcmp(argv[i], "--metrics") == 0) {
            options->metrics = true;
            options->render.report_metrics = true;
        } else if (strcmp(argv[i], "--verbose") == 0) {
            options->verbose = true;
        } else if (strcmp(argv[i], "--version") == 0) {
            printf("scene-render %s\n", SR_VERSION);
            return 1;
        } else if (strcmp(argv[i], "--help") == 0 || strcmp(argv[i], "-h") == 0) {
            usage(stdout);
            return 1;
        } else {
            fprintf(stderr, "error: unknown option '%s'\n", argv[i]);
            return 2;
        }
    }
    if (!options->scene_path) {
        fprintf(stderr, "error: --scene is required\n");
        return 2;
    }
    return 0;
}

static bool apply_quality(SrScene *scene, const char *quality) {
    if (!quality) return true;
    const char *preset;
    if (strcmp(quality, "low") == 0) {
        scene->output.crf = 28;
        preset = "veryfast";
    } else if (strcmp(quality, "medium") == 0) {
        scene->output.crf = 23;
        preset = "medium";
    } else if (strcmp(quality, "high") == 0) {
        scene->output.crf = 18;
        preset = "slow";
    } else {
        return false;
    }
    char *copy = sr_strdup(preset);
    if (!copy) return false;
    free(scene->output.preset);
    scene->output.preset = copy;
    scene->output.bitrate = 0;
    return true;
}

int main(int argc, char **argv) {
    CliOptions options;
    int parsed = parse_cli(argc, argv, &options);
    if (parsed == 1) return 0;
    if (parsed != 0) {
        usage(stderr);
        return parsed;
    }
    SrDiagnostics diag;
    sr_diag_init(&diag, options.scene_path, stderr);
    diag.verbose = options.verbose;
    if (options.requested_gpu)
        sr_diag_warning(&diag, 0, NULL, NULL,
                        "GPU backend unavailable; using deterministic CPU renderer");
    SrScene scene;
    SrStatus status = sr_scene_load_xml(options.scene_path, &scene, &diag);
    if (status != SR_OK) {
        return status;
    }
    if (options.override_resolution) {
        scene.project.width = options.width;
        scene.project.height = options.height;
    }
    if (options.override_fps) {
        scene.project.fps_num = options.fps_num;
        scene.project.fps_den = options.fps_den;
    }
    if (!apply_quality(&scene, options.quality)) {
        fprintf(stderr, "error: --quality expects low, medium, or high\n");
        sr_scene_free(&scene);
        return SR_ERR_ARGUMENT;
    }
    SrRenderMetrics metrics;
    status = sr_render(&scene, &options.render, &metrics, &diag);
    if (status == SR_OK && options.render.validate_only) {
        printf("valid: %s (%zu assets, %zu top-level layers)\n",
               options.scene_path, scene.asset_count, scene.root->child_count);
    }
    if (options.metrics) {
        double fps = metrics.wall_seconds > 0.0
                         ? metrics.frames / metrics.wall_seconds
                         : 0.0;
        fprintf(stderr,
                "metrics: frames=%llu render_s=%.6f encode_s=%.6f "
                "wall_s=%.6f fps=%.3f peak_rss_kib=%ld\n",
                (unsigned long long)metrics.frames, metrics.render_seconds,
                metrics.encode_seconds, metrics.wall_seconds, fps,
                metrics.peak_rss_kib);
    }
    sr_scene_free(&scene);
    return status;
}
