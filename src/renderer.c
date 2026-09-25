#define _POSIX_C_SOURCE 200809L
#include "scene_render/renderer.h"

#include "scene_render/assets.h"
#include "scene_render/audio.h"
#include "scene_render/camera.h"
#include "scene_render/color.h"
#include "scene_render/compositor.h"
#include "scene_render/encoder.h"
#include "scene_render/effects.h"
#include "scene_render/gpu.h"
#include "scene_render/lighting.h"
#include "scene_render/physics.h"
#include "scene_render/resume.h"
#include "scene_render/spatial.h"

#include <libavcodec/avcodec.h>

#include <errno.h>
#include <math.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <sys/resource.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

const char *sr_stage_name(SrStage stage) {
    static const char *const names[SR_STAGE_COUNT] = {
        "clear", "lighting", "composite", "viewport",
        "effects", "convert", "resume", "encode"};
    return stage < SR_STAGE_COUNT ? names[stage] : "unknown";
}

/* CPU consumed by every thread of this process. */
static double sr_process_cpu_seconds(void) {
    struct timespec now;
    if (clock_gettime(CLOCK_PROCESS_CPUTIME_ID, &now) != 0) return 0.0;
    return (double)now.tv_sec + (double)now.tv_nsec * 1e-9;
}

typedef struct {
    double wall;
    double cpu;
} SrStageMark;

static SrStageMark sr_stage_begin(void) {
    return (SrStageMark){sr_monotonic_seconds(), sr_process_cpu_seconds()};
}

static void sr_stage_end(SrStageTimes *times, SrStage stage, SrStageMark mark) {
    times->wall[stage] += sr_monotonic_seconds() - mark.wall;
    times->cpu[stage] += sr_process_cpu_seconds() - mark.cpu;
}

static void sr_stage_accumulate(SrRenderMetrics *metrics,
                                const SrStageTimes *frame) {
    for (int stage = 0; stage < SR_STAGE_COUNT; ++stage) {
        metrics->stages.wall[stage] += frame->wall[stage];
        metrics->stages.cpu[stage] += frame->cpu[stage];
        if (frame->wall[stage] > metrics->stage_max.wall[stage])
            metrics->stage_max.wall[stage] = frame->wall[stage];
        if (frame->cpu[stage] > metrics->stage_max.cpu[stage])
            metrics->stage_max.cpu[stage] = frame->cpu[stage];
    }
}

static void sr_trace_stages(FILE *trace, const char *key, const double *values) {
    fprintf(trace, "\"%s\":{", key);
    for (int stage = 0; stage < SR_STAGE_COUNT; ++stage)
        fprintf(trace, "%s\"%s\":%.6f", stage ? "," : "",
                sr_stage_name((SrStage)stage), values[stage]);
    fputc('}', trace);
}

static void sr_trace_frame(FILE *trace, uint64_t index, double time,
                           bool reused, const SrStageTimes *times) {
    if (!trace) return;
    fprintf(trace, "{\"frame\":%llu,\"time\":%.6f,\"reused\":%s,",
            (unsigned long long)index, time, reused ? "true" : "false");
    sr_trace_stages(trace, "wall", times->wall);
    fputc(',', trace);
    sr_trace_stages(trace, "cpu", times->cpu);
    fputs("}\n", trace);
}

static void sr_trace_summary(FILE *trace, const SrRenderMetrics *metrics,
                             SrStatus status) {
    if (!trace) return;
    fprintf(trace, "{\"summary\":true,\"status\":%d,\"frames\":%llu,"
            "\"wall_s\":%.6f,\"render_s\":%.6f,\"encode_s\":%.6f,"
            "\"setup_wall_s\":%.6f,\"setup_cpu_s\":%.6f,"
            "\"user_s\":%.6f,\"sys_s\":%.6f,"
            "\"peak_self_rss_kib\":%ld,\"audio_samples\":%llu,"
            "\"video_requests\":%llu,\"video_cache_hits\":%llu,"
            "\"video_decoded\":%llu,\"video_seeks\":%llu,",
            (int)status, (unsigned long long)metrics->frames,
            metrics->wall_seconds, metrics->render_seconds,
            metrics->encode_seconds, metrics->setup_wall_seconds,
            metrics->setup_cpu_seconds, metrics->user_seconds,
            metrics->system_seconds, metrics->peak_self_rss_kib,
            (unsigned long long)metrics->audio_samples,
            (unsigned long long)metrics->video_requests,
            (unsigned long long)metrics->video_cache_hits,
            (unsigned long long)metrics->video_decoded,
            (unsigned long long)metrics->video_seeks);
    sr_trace_stages(trace, "wall", metrics->stages.wall);
    fputc(',', trace);
    sr_trace_stages(trace, "cpu", metrics->stages.cpu);
    fputc(',', trace);
    sr_trace_stages(trace, "max_wall", metrics->stage_max.wall);
    fputs("}\n", trace);
}

static double sr_timeval_seconds(struct timeval value) {
    return (double)value.tv_sec + (double)value.tv_usec * 1e-6;
}

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

SrStatus sr_write_png(const char *path, uint32_t width, uint32_t height,
                      const uint8_t *rgba, SrDiagnostics *diag) {
    SrStatus status = sr_make_parent_dirs(path, diag);
    if (status != SR_OK) return status;
    if (!width || !height || width > INT32_MAX / 4 || height > INT32_MAX)
        return SR_ERR_ARGUMENT;
    const AVCodec *codec = avcodec_find_encoder(AV_CODEC_ID_PNG);
    AVCodecContext *context = codec ? avcodec_alloc_context3(codec) : NULL;
    AVFrame *frame = av_frame_alloc();
    AVPacket *packet = av_packet_alloc();
    int rc = context && frame && packet ? 0 : AVERROR(ENOMEM);
    if (rc == 0) {
        context->width = (int)width;
        context->height = (int)height;
        context->pix_fmt = AV_PIX_FMT_RGBA;
        context->time_base = (AVRational){1, 1};
        context->flags |= AV_CODEC_FLAG_BITEXACT;
        rc = avcodec_open2(context, codec, NULL);
    }
    if (rc == 0) {
        frame->format = AV_PIX_FMT_RGBA;
        frame->width = (int)width;
        frame->height = (int)height;
        frame->data[0] = (uint8_t *)rgba;
        frame->linesize[0] = (int)width * 4;
        frame->pts = 0;
        rc = avcodec_send_frame(context, frame);
    }
    if (rc == 0) rc = avcodec_send_frame(context, NULL);
    if (rc == 0) rc = avcodec_receive_packet(context, packet);
    if (rc == 0) {
        FILE *file = fopen(path, "wb");
        bool ok = file && fwrite(packet->data, 1, (size_t)packet->size, file) ==
                              (size_t)packet->size;
        if (file && fclose(file) != 0) ok = false;
        if (!ok) {
            sr_diag_error(diag, 0, NULL, NULL, "cannot write '%s': %s", path,
                          strerror(errno));
            status = SR_ERR_IO;
        }
    } else {
        char message[AV_ERROR_MAX_STRING_SIZE];
        av_strerror(rc, message, sizeof(message));
        sr_diag_error(diag, 0, NULL, NULL, "PNG encoding of '%s' failed: %s",
                      path, message);
        status = rc == AVERROR(ENOMEM) ? SR_ERR_MEMORY : SR_ERR_ENCODER;
    }
    av_packet_free(&packet);
    av_frame_free(&frame);   /* data[0] is borrowed: the frame owns no buffer */
    avcodec_free_context(&context);
    return status;
}

static bool sr_has_suffix(const char *path, const char *suffix) {
    size_t length = strlen(path), size = strlen(suffix);
    return length >= size && strcasecmp(path + length - size, suffix) == 0;
}

typedef struct {
    SrCompositor compositor;
    SrColorOutput color;
    unsigned bits;      /* 8 or 16 per component in `pixels` */
    void *pixels;       /* straight RGBA handed to encoder/cache/preview */
} SrFrameState;

static SrStatus sr_render_frame(SrScene *scene, uint64_t index,
                                SrFrameState *state, SrFrame *composition,
                                SrFrame *output, unsigned threads, SrGpu *gpu,
                                double *seconds, SrStageTimes *times,
                                SrDiagnostics *diag) {
    double start = sr_monotonic_seconds();
    SrStageMark mark = sr_stage_begin();
    float background[4];
    sr_color_to_blend(&scene->project, scene->project.background, background);
    sr_frame_clear(composition, background, threads);
    sr_stage_end(times, SR_STAGE_CLEAR, mark);
    double time = (double)index * scene->project.fps_den / scene->project.fps_num;
    SrStatus status;
    if (scene->has_cards) {
        /* Cards interleave with the 3D objects: one timed stage. */
        mark = sr_stage_begin();
        status = sr_compositor_render_scene(&state->compositor, scene, time,
                                            composition, diag);
        sr_stage_end(times, SR_STAGE_COMPOSITE, mark);
    } else {
        mark = sr_stage_begin();
        status = sr_lighting_render(scene, time, composition, diag);
        sr_stage_end(times, SR_STAGE_LIGHTING, mark);
        if (status == SR_OK) {
            mark = sr_stage_begin();
            status = sr_compositor_render(&state->compositor, scene, time,
                                          composition, diag);
            sr_stage_end(times, SR_STAGE_COMPOSITE, mark);
        }
    }
    if (status == SR_OK && scene->project.mode == SR_MODE_VIEWPORT) {
        mark = sr_stage_begin();
        status = sr_camera_extract_viewport(scene, time, composition, output,
                                            threads, diag);
        sr_stage_end(times, SR_STAGE_VIEWPORT, mark);
    }
    if (status == SR_OK) {
        mark = sr_stage_begin();
        status = sr_effects_apply(scene, time, output, threads, diag);
        sr_stage_end(times, SR_STAGE_EFFECTS, mark);
    }
    mark = sr_stage_begin();
    if (status == SR_OK && state->bits == 16) {
        status = sr_color_convert_frame16(&state->color, output, state->pixels,
                                          threads);
    } else if (status == SR_OK) {
        if (gpu && gpu->implementation) {
            status = sr_gpu_convert_frame(gpu, output, state->pixels,
                                          &scene->project,
                                          scene->output.color_space, diag);
            if (status != SR_OK) {
                sr_diag_warning(diag, 0, NULL, NULL,
                                "OpenCL conversion failed; using CPU fallback");
                sr_gpu_close(gpu);
                status = sr_color_convert_frame(&state->color, output,
                                                state->pixels, threads);
            }
        } else
            status = sr_color_convert_frame(&state->color, output,
                                            state->pixels, threads);
    }
    sr_stage_end(times, SR_STAGE_CONVERT, mark);
    *seconds += sr_monotonic_seconds() - start;
    return status;
}

/* Everything one range render (encode, hash or segmented resume) shares. */
typedef struct {
    SrScene *scene;
    const SrRenderOptions *options;
    SrRenderMetrics *metrics;
    SrDiagnostics *diag;
    SrFrameState *state;
    SrFrame *composition;
    SrFrame *frame;         /* output-sized frame (viewport or composition) */
    SrGpu *gpu;
    FILE *trace;
    size_t frame_bytes;     /* bytes of state->pixels */
    SrMixer *mixer;         /* NULL: the scene has no audio */
    float *audio;           /* one frame's mixed block */
    uint32_t channels;
} SrRun;

static SrStatus sr_run_audio_open(SrRun *run) {
    SrScene *scene = run->scene;
    if (!scene->audio.track_count) return SR_OK;
    SrStatus status = sr_audio_load(scene, run->diag);
    if (status == SR_OK) status = sr_mixer_create(scene, &run->mixer);
    /* Largest per-frame block: ceil(rate * fps_den / fps_num). */
    size_t block = (size_t)(sr_frame_to_sample(1, scene->audio.sample_rate,
                                               scene->project.fps_num,
                                               scene->project.fps_den) + 1);
    if (status == SR_OK) {
        run->audio = sr_alloc(block * scene->audio.channels * sizeof(float));
        if (!run->audio) status = SR_ERR_MEMORY;
    }
    run->channels = scene->audio.channels;
    return status;
}

/* Mixes frame `index`'s samples [S(index), S(index+1)) into run->audio: the
 * blocks tile the range exactly, so the audio of [first, end) lasts exactly
 * S(end) - S(first) samples. Returns the sample count. */
static size_t sr_run_mix(SrRun *run, uint64_t index) {
    const SrScene *scene = run->scene;
    uint32_t rate = scene->audio.sample_rate;
    uint64_t from = sr_frame_to_sample(index, rate, scene->project.fps_num,
                                       scene->project.fps_den);
    uint64_t to = sr_frame_to_sample(index + 1, rate, scene->project.fps_num,
                                     scene->project.fps_den);
    sr_mixer_mix(run->mixer, from, (size_t)(to - from), run->audio);
    run->metrics->audio_samples += to - from;
    return (size_t)(to - from);
}

static SrStatus sr_run_render(SrRun *run, uint64_t index, SrStageTimes *times) {
    return sr_render_frame(run->scene, index, run->state, run->composition,
                           run->frame, run->options->encoder_threads, run->gpu,
                           &run->metrics->render_seconds, times, run->diag);
}

static void sr_run_account(SrRun *run, uint64_t index, uint64_t total,
                           const SrStageTimes *times) {
    sr_stage_accumulate(run->metrics, times);
    sr_trace_frame(run->trace, index,
                   (double)index * run->scene->project.fps_den /
                       run->scene->project.fps_num, false, times);
    ++run->metrics->frames;
    if (run->diag->verbose && (run->metrics->frames % 30 == 0))
        sr_diag_info(run->diag, "rendered %llu/%llu selected frames",
                     (unsigned long long)run->metrics->frames,
                     (unsigned long long)total);
}

/* --hash: frames are hashed exactly as the encoder would receive them. */
static SrStatus sr_run_hash(SrRun *run, uint64_t first, uint64_t end) {
    FILE *out = run->options->hash_stream ? run->options->hash_stream : stdout;
    uint64_t audio_hash = SR_FNV_OFFSET;
    SrStatus status = SR_OK;
    for (uint64_t index = first; index < end && status == SR_OK; ++index) {
        SrStageTimes times = {0};
        status = sr_run_render(run, index, &times);
        if (status != SR_OK) break;
        fprintf(out, "%llu %016llx\n", (unsigned long long)index,
                (unsigned long long)sr_fnv1a64(SR_FNV_OFFSET, run->state->pixels,
                                               run->frame_bytes));
        if (run->mixer) {
            size_t samples = sr_run_mix(run, index);
            audio_hash = sr_fnv1a64(audio_hash, run->audio,
                                    samples * run->channels * sizeof(float));
        }
        sr_run_account(run, index, end - first, &times);
    }
    if (status == SR_OK && run->mixer)
        fprintf(out, "audio %016llx\n", (unsigned long long)audio_hash);
    if (fflush(out) != 0 && status == SR_OK) {
        sr_diag_error(run->diag, 0, NULL, NULL, "cannot write hashes: %s",
                      strerror(errno));
        status = SR_ERR_IO;
    }
    return status;
}

/* Renders [first, end) into an open encoder; audio too when `with_audio`. */
static SrStatus sr_run_frames(SrRun *run, SrEncoder *encoder, uint64_t first,
                              uint64_t end, uint64_t total, bool with_audio) {
    SrStatus status = SR_OK;
    for (uint64_t index = first; index < end && status == SR_OK; ++index) {
        SrStageTimes times = {0};
        status = sr_run_render(run, index, &times);
        if (status != SR_OK) break;
        SrStageMark mark = sr_stage_begin();
        status = sr_encoder_write_video(encoder, run->state->pixels, run->diag);
        if (status == SR_OK && with_audio && run->mixer) {
            size_t samples = sr_run_mix(run, index);
            status = sr_encoder_write_audio(encoder, run->audio, samples, run->diag);
        }
        sr_stage_end(&times, SR_STAGE_ENCODE, mark);
        if (status == SR_OK) sr_run_account(run, index, total, &times);
    }
    return status;
}

static SrEncoderAudio sr_run_audio_format(const SrRun *run) {
    return (SrEncoderAudio){run->scene->audio.sample_rate,
                            run->mixer ? run->channels : 0};
}

static SrStatus sr_run_encode(SrRun *run, const char *path, uint64_t first,
                              uint64_t end) {
    SrEncoderAudio audio = sr_run_audio_format(run);
    SrEncoder *encoder = NULL;
    SrStatus status = sr_encoder_open(&encoder, run->scene, path,
                                      run->options->encoder_threads, &audio,
                                      run->diag);
    if (status == SR_OK) {
        status = sr_run_frames(run, encoder, first, end, end - first, true);
        SrStatus finished = sr_encoder_finish(encoder, run->diag);
        if (status == SR_OK) status = finished;
    }
    run->metrics->encode_seconds += sr_encoder_seconds(encoder);
    sr_encoder_destroy(encoder);
    return status;
}

/* Test hook: SR_TEST_ABORT_AFTER_SEGMENTS=N kills the process (SIGKILL,
 * like an interruption) once this run has committed N segments. */
static void sr_run_maybe_abort(uint64_t committed) {
    const char *value = getenv("SR_TEST_ABORT_AFTER_SEGMENTS");
    uint64_t limit;
    if (value && sr_parse_u64(value, &limit) && limit && committed >= limit) {
        fflush(NULL);
        raise(SIGKILL);
    }
}

static SrStatus sr_run_segment(SrRun *run, const SrResume *resume, uint64_t k) {
    uint64_t from, to;
    sr_resume_segment_range(resume, k, &from, &to);
    char *partial = sr_resume_segment_path(resume, k, true);
    if (!partial) return SR_ERR_MEMORY;
    SrEncoder *encoder = NULL;
    SrStatus status = sr_encoder_open_segment(&encoder, run->scene, partial,
                                              run->options->encoder_threads,
                                              run->diag);
    if (status == SR_OK) {
        status = sr_run_frames(run, encoder, from, to, resume->end - resume->first,
                               false);
        SrStatus finished = sr_encoder_finish(encoder, run->diag);
        if (status == SR_OK) status = finished;
    }
    run->metrics->encode_seconds += sr_encoder_seconds(encoder);
    sr_encoder_destroy(encoder);
    if (status == SR_OK) status = sr_resume_commit(resume, k, run->diag);
    if (status != SR_OK) unlink(partial);
    free(partial);
    return status;
}

/* Packet-copies every committed segment into `path` and encodes the audio
 * of the whole range once. */
static SrStatus sr_run_assemble(SrRun *run, const SrResume *resume,
                                const char *path) {
    char *template_path = sr_resume_segment_path(resume, 0, false);
    if (!template_path) return SR_ERR_MEMORY;
    SrEncoderAudio audio = sr_run_audio_format(run);
    SrEncoder *encoder = NULL;
    SrStatus status = sr_encoder_open_copy(&encoder, run->scene, path,
                                           template_path, &audio, run->diag);
    free(template_path);
    for (uint64_t k = 0; status == SR_OK && k < resume->segment_count; ++k) {
        uint64_t from, to;
        sr_resume_segment_range(resume, k, &from, &to);
        char *segment = sr_resume_segment_path(resume, k, false);
        status = segment ? sr_encoder_copy_video(encoder, segment,
                                                 from - resume->first, to - from,
                                                 run->diag)
                         : SR_ERR_MEMORY;
        free(segment);
        for (uint64_t index = from; status == SR_OK && run->mixer && index < to;
             ++index) {
            size_t samples = sr_run_mix(run, index);
            status = sr_encoder_write_audio(encoder, run->audio, samples, run->diag);
        }
    }
    if (encoder) {
        SrStatus finished = sr_encoder_finish(encoder, run->diag);
        if (status == SR_OK) status = finished;
    }
    run->metrics->encode_seconds += sr_encoder_seconds(encoder);
    sr_encoder_destroy(encoder);
    return status;
}

static SrStatus sr_run_resume(SrRun *run, const char *path, uint64_t first,
                              uint64_t end) {
    const SrRenderOptions *options = run->options;
    SrResumeSettings settings = {options->encoder_threads, options->request_gpu,
                                 run->state->bits};
    uint32_t segment_frames = options->segment_frames
                                  ? options->segment_frames
                                  : SR_RESUME_DEFAULT_SEGMENT_FRAMES;
    SrResume resume;
    SrStatus status = sr_resume_prepare(&resume, run->scene, path, first, end,
                                        segment_frames, &settings, run->diag);
    if (status != SR_OK) return status;
    uint64_t committed = 0;
    for (uint64_t k = 0; status == SR_OK && k < resume.segment_count; ++k) {
        if (sr_resume_segment_done(&resume, k)) {
            ++run->metrics->segments_reused;
            continue;
        }
        status = sr_run_segment(run, &resume, k);
        if (status == SR_OK) {
            ++run->metrics->segments_rendered;
            sr_run_maybe_abort(++committed);
        }
    }
    if (status == SR_OK) {
        SrStageMark mark = sr_stage_begin();
        status = sr_run_assemble(run, &resume, path);
        sr_stage_end(&run->metrics->stages, SR_STAGE_RESUME, mark);
    }
    if (status == SR_OK && !options->keep_parts)
        status = sr_resume_remove(&resume, run->diag);
    sr_diag_info(run->diag, "resume: %llu segment(s) rendered, %llu reused",
                 (unsigned long long)run->metrics->segments_rendered,
                 (unsigned long long)run->metrics->segments_reused);
    sr_resume_close(&resume);
    return status;
}

SrStatus sr_render(SrScene *scene, const SrRenderOptions *options,
                   SrRenderMetrics *metrics, SrDiagnostics *diag) {
    if (!scene || !options || !metrics) {
        return SR_ERR_ARGUMENT;
    }
    *metrics = (SrRenderMetrics){0};
    if (scene->has_cards && scene->project.mode != SR_MODE_STANDARD) {
        sr_diag_error(diag, 0, "project", "mode",
                      "depth cards require mode standard; equirectangular and "
                      "viewport canvases are not projected by a camera");
        return SR_ERR_ARGUMENT;
    }
    if (options->validate_only) {
        return SR_OK;
    }
    double wall_start = sr_monotonic_seconds();
    uint64_t video_totals[4] = {0};
    FILE *trace = NULL;
    if (options->trace_path) {
        if (sr_make_parent_dirs(options->trace_path, diag) != SR_OK)
            return SR_ERR_IO;
        trace = fopen(options->trace_path, "w");
        if (!trace) {
            sr_diag_error(diag, 0, NULL, NULL, "cannot write trace '%s': %s",
                          options->trace_path, strerror(errno));
            return SR_ERR_IO;
        }
    }
    SrStageMark setup = sr_stage_begin();
    SrGpu gpu = {0};
    if (options->request_gpu) {
        if (sr_gpu_open(&gpu, diag))
            sr_diag_info(diag, "using OpenCL GPU device: %s",
                         sr_gpu_device_name(&gpu));
        else
            sr_diag_warning(diag, 0, NULL, NULL,
                            "no usable OpenCL GPU; using deterministic CPU fallback");
    }
    SrStatus status = sr_assets_load(scene, diag);
    if (status == SR_OK) status = sr_physics_prepare(scene, diag);
    metrics->physics_steps = scene->physics.steps_simulated;
    metrics->physics_cache_hit = scene->physics.cache_hit;
    metrics->setup_wall_seconds = sr_monotonic_seconds() - setup.wall;
    metrics->setup_cpu_seconds = sr_process_cpu_seconds() - setup.cpu;
    if (status != SR_OK) {
        sr_gpu_close(&gpu);
        sr_trace_summary(trace, metrics, status);
        if (trace) fclose(trace);
        return status;
    }
    uint32_t canvas_width = scene->project.mode == SR_MODE_VIEWPORT
        ? scene->scene360.width : scene->project.width;
    uint32_t canvas_height = scene->project.mode == SR_MODE_VIEWPORT
        ? scene->scene360.height : scene->project.height;
    SrFrame composition = {0}, viewport = {0};
    SrFrameState state = {0};
    sr_compositor_init(&state.compositor, options->encoder_threads);
    status = sr_frame_init(&composition, canvas_width, canvas_height);
    if (status != SR_OK) {
        sr_diag_error(diag, 0, NULL, NULL, "cannot allocate %ux%u float RGBA frame",
                      canvas_width, canvas_height);
        sr_gpu_close(&gpu);
        if (trace) fclose(trace);
        return status;
    }
    SrFrame *frame = &composition;
    if (scene->project.mode == SR_MODE_VIEWPORT) {
        status = sr_frame_init(&viewport, scene->project.width,
                               scene->project.height);
        if (status != SR_OK) goto cleanup;
        frame = &viewport;
    }
    /* Previews are 8-bit; video output feeds the encoder whatever depth its
     * pixel format needs. */
    state.bits = options->preview ? 8
                                  : sr_encoder_input_bits(scene->output.pixel_format);
    status = sr_color_output_init_bits(&state.color, &scene->project,
                                       scene->output.color_space, state.bits);
    if (status != SR_OK) goto cleanup;
    size_t frame_bytes = (size_t)frame->width * frame->height * 4 * (state.bits / 8);
    state.pixels = sr_alloc(frame_bytes);
    if (!state.pixels) {
        status = SR_ERR_MEMORY;
        goto cleanup;
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
        SrStageTimes times = {0};
        status = sr_render_frame(scene, options->preview_frame, &state,
                                 &composition, frame, options->encoder_threads,
                                 &gpu, &metrics->render_seconds, &times, diag);
        sr_stage_accumulate(metrics, &times);
        sr_trace_frame(trace, options->preview_frame,
                       (double)options->preview_frame * scene->project.fps_den /
                           scene->project.fps_num, false, &times);
        char fallback[40];
        const char *preview_path = options->preview_path;
        if (!preview_path) {
            snprintf(fallback, sizeof(fallback), "frame-%06llu.png",
                     (unsigned long long)options->preview_frame);
            preview_path = fallback;
        }
        if (status == SR_OK) {
            metrics->preview_hash = sr_fnv1a64(SR_FNV_OFFSET, state.pixels,
                                               frame_bytes);
            status = sr_has_suffix(preview_path, ".png")
                ? sr_write_png(preview_path, frame->width,
                               frame->height, state.pixels, diag)
                : sr_write_ppm(preview_path, frame->width,
                               frame->height, state.pixels, diag);
        }
        metrics->frames = status == SR_OK ? 1 : 0;
        goto cleanup;
    }
    SrRun run = {scene, options, metrics, diag, &state, &composition, frame,
                 &gpu, trace, frame_bytes, NULL, NULL, 0};
    status = sr_run_audio_open(&run);
    if (status == SR_OK) {
        if (options->hash) {
            status = sr_run_hash(&run, first, end);
        } else {
            char *path = options->output_override
                             ? sr_strdup(options->output_override)
                             : sr_path_join(scene->base_dir, scene->output.path);
            status = path ? sr_make_parent_dirs(path, diag) : SR_ERR_MEMORY;
            if (status == SR_OK)
                status = options->resume ? sr_run_resume(&run, path, first, end)
                                         : sr_run_encode(&run, path, first, end);
            if (status == SR_OK && scene->project.mode == SR_MODE_EQUIRECTANGULAR &&
                scene->output.spherical_metadata && sr_spatial_is_mp4(path))
                status = sr_spatial_inject_mp4(path, scene->project.width,
                                               scene->project.height, diag);
            free(path);
        }
    }
    sr_mixer_destroy(run.mixer);
    free(run.audio);

cleanup:
    sr_assets_video_stats(scene, &metrics->video_sources, video_totals);
    metrics->video_requests = video_totals[0];
    metrics->video_cache_hits = video_totals[1];
    metrics->video_decoded = video_totals[2];
    metrics->video_seeks = video_totals[3];
    sr_gpu_close(&gpu);
    sr_compositor_free(&state.compositor);
    sr_color_output_free(&state.color);
    free(state.pixels);
    sr_frame_free(&viewport);
    sr_frame_free(&composition);
    metrics->wall_seconds = sr_monotonic_seconds() - wall_start;
    struct rusage usage;
    if (getrusage(RUSAGE_SELF, &usage) == 0) {
        metrics->peak_self_rss_kib = usage.ru_maxrss;
        metrics->user_seconds = sr_timeval_seconds(usage.ru_utime);
        metrics->system_seconds = sr_timeval_seconds(usage.ru_stime);
    }
    metrics->peak_rss_kib = metrics->peak_self_rss_kib;
    sr_trace_summary(trace, metrics, status);
    if (trace && fclose(trace) != 0 && status == SR_OK) {
        sr_diag_error(diag, 0, NULL, NULL, "failed while writing trace '%s'",
                      options->trace_path);
        status = SR_ERR_IO;
    }
    return status;
}
