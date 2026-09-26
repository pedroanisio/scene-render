#define _POSIX_C_SOURCE 200809L
#include "scene_render/renderer.h"
#include "compositing_internal.h"
#include "output_media_internal.h"
#include "output_plan_internal.h"

#include "scene_render/assets.h"
#include "scene_render/audio.h"
#include "scene_render/camera.h"
#include "scene_render/color.h"
#include "scene_render/compositor.h"
#include "scene_render/encoder.h"
#include "scene_render/effects.h"
#include "scene_render/gpu.h"
#include "scene_render/lighting.h"
#include "scene_render/outputs.h"
#include "scene_render/parallel.h"
#include "scene_render/physics.h"
#include "scene_render/resume.h"
#include "scene_render/spatial.h"

#include <libavcodec/avcodec.h>

#include <errno.h>
#include <math.h>
#include <pthread.h>
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
    if (!path || !*path) {
        sr_diag_error(diag, 0, NULL, NULL, "empty output path");
        return SR_ERR_ARGUMENT;
    }
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
    unsigned bits;      /* 8 or 16 per component in `pixels`; 32: linear float */
    void *pixels;       /* straight RGBA handed to encoder/cache/preview */
    SrColorSpace space; /* colour space of the first sink */
    bool allow_gpu;     /* OpenCL conversion: one 1.0 sink only */
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
        status = sr_lighting_render_threads(scene, time, composition, threads,
                                            diag);
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
    if (status == SR_OK && state->bits == 32) {
        status = sr_output_convert_linear(&state->color, &scene->project, output,
                                          state->pixels, threads);
    } else if (status == SR_OK && state->bits == 16) {
        status = sr_color_convert_frame16(&state->color, output, state->pixels,
                                          threads);
    } else if (status == SR_OK) {
        if (gpu && gpu->implementation && state->allow_gpu) {
            status = sr_gpu_convert_frame(gpu, output, state->pixels,
                                          &scene->project, state->space, diag);
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
    const SrResumeInputs *inputs;   /* --resume: fingerprinted input files */
    size_t audio_values;    /* floats in one frame's block (run->audio) */
    const SrPlanPass *pass;
    struct SrRunSink *sinks;        /* pass->sink_count entries */
} SrRun;

/* One output of a pass while it renders. The first sink converts inside
 * sr_render_frame (state->pixels); later sinks convert the same float frame
 * into their own buffers. */
typedef struct SrRunSink {
    const SrPlanSink *plan;
    SrEncoder *encoder;
    SrColorOutput color;    /* sinks after the first */
    unsigned bits;
    size_t frame_bytes;
    void *pixels;           /* sinks after the first */
} SrRunSink;

static bool sr_sink_has(const SrRunSink *sink, uint64_t index) {
    return sink->encoder && index >= sink->plan->first && index < sink->plan->end;
}

/* Converts the rendered float frame for every later sink that encodes
 * frame `index`, into buffers[k] (buffers[0] is unused). */
static SrStatus sr_run_convert_sinks(SrRun *run, uint64_t index, void **buffers,
                                     SrStageTimes *times) {
    SrStatus status = SR_OK;
    size_t count = run->pass ? run->pass->sink_count : 1;
    for (size_t k = 1; k < count && status == SR_OK; ++k) {
        SrRunSink *sink = &run->sinks[k];
        if (!sr_sink_has(sink, index)) continue;
        SrStageMark mark = sr_stage_begin();
        unsigned threads = run->options->encoder_threads;
        if (sink->bits == 32)
            status = sr_output_convert_linear(&sink->color, &run->scene->project,
                                              run->frame, buffers[k], threads);
        else if (sink->bits == 16)
            status = sr_color_convert_frame16(&sink->color, run->frame, buffers[k],
                                              threads);
        else
            status = sr_color_convert_frame(&sink->color, run->frame, buffers[k],
                                            threads);
        sr_stage_end(times, SR_STAGE_CONVERT, mark);
    }
    return status;
}

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
        run->audio_values = block * scene->audio.channels;
        run->audio = sr_alloc(run->audio_values * sizeof(float));
        if (!run->audio) status = SR_ERR_MEMORY;
    }
    run->channels = scene->audio.channels;
    return status;
}

/* Mixes frame `index`'s samples [S(index), S(index+1)) into `out` (room for
 * run->audio_values floats): the blocks tile the range exactly, so the audio
 * of [first, end) lasts exactly S(end) - S(first) samples. Returns the
 * sample count. */
static size_t sr_run_mix(SrRun *run, uint64_t index, float *out) {
    const SrScene *scene = run->scene;
    uint32_t rate = scene->audio.sample_rate;
    uint64_t from = sr_frame_to_sample(index, rate, scene->project.fps_num,
                                       scene->project.fps_den);
    uint64_t to = sr_frame_to_sample(index + 1, rate, scene->project.fps_num,
                                     scene->project.fps_den);
    sr_mixer_mix(run->mixer, from, (size_t)(to - from), out);
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
            size_t samples = sr_run_mix(run, index, run->audio);
            audio_hash = sr_fnv1a64(audio_hash, run->audio,
                                    samples * run->channels * sizeof(float));
        }
        sr_run_account(run, index, end - first, &times);
    }
    if (status == SR_OK && run->mixer)
        fprintf(out, "audio %016llx\n", (unsigned long long)audio_hash);
    if ((fflush(out) != 0 || ferror(out)) && status == SR_OK) {
        sr_diag_error(run->diag, 0, NULL, NULL, "cannot write hashes: %s",
                      strerror(errno));
        status = SR_ERR_IO;
    }
    return status;
}

/* ---- frame writer ------------------------------------------------------
 * A writer thread encodes frame N (video, then its audio block) while the
 * render thread renders frame N+1. Frames reach the encoder in the same
 * order, with the same bytes and the same encoder settings as a serial
 * render, so the output file is identical. The render thread renders
 * straight into a slot's buffer (state->pixels points at it) and mixes the
 * frame's audio into the slot; the writer takes slots in queue order. */

/* One slot being encoded, one queued, one being rendered into. */
enum { SR_WRITE_SLOTS = 3 };

typedef struct {
    void *pixels;           /* frame_bytes of encoder input (first sink) */
    void *extra[SR_MAX_OUTPUTS];    /* later sinks' input, index = sink */
    float *audio;           /* run->audio_values floats; NULL: no audio */
    size_t samples;
    uint64_t index;         /* frame index */
} SrWriteSlot;

typedef struct {
    SrRunSink *sinks;
    size_t sink_count;
    SrWriteSlot slots[SR_WRITE_SLOTS];
    bool owned[SR_WRITE_SLOTS];     /* buffers allocated by the writer */
    unsigned head;          /* oldest queued slot: the one being encoded */
    unsigned count;         /* queued slots, the one being encoded included */
    bool closing;           /* no more frames will be queued */
    SrStatus status;        /* first writer failure; SR_OK until then */
    pthread_mutex_t lock;
    pthread_cond_t cond;
    pthread_t thread;
    /* The writer reports into its own diagnostics (SrDiagnostics is not
     * thread-safe); the text and counts are replayed after the join. */
    SrDiagnostics diag;
    FILE *log;
    char *log_text;
    size_t log_size;
} SrWriter;

/* Encodes one frame into every sink that covers it, in sink order: video,
 * then that sink's audio block. With one sink this is exactly the 1.0
 * sequence of calls. */
static SrStatus sr_write_frame(SrRunSink *sinks, size_t count,
                               const SrWriteSlot *slot, SrDiagnostics *diag) {
    SrStatus status = SR_OK;
    for (size_t k = 0; k < count && status == SR_OK; ++k) {
        SrRunSink *sink = &sinks[k];
        if (!sr_sink_has(sink, slot->index)) continue;
        status = sr_encoder_write_video(sink->encoder,
                                        k == 0 ? slot->pixels : slot->extra[k], diag);
        if (status == SR_OK && slot->audio && sr_encoder_has_audio(sink->encoder))
            status = sr_encoder_write_audio(sink->encoder, slot->audio,
                                            slot->samples, diag);
    }
    return status;
}

static void *sr_writer_main(void *arg) {
    SrWriter *w = arg;
    pthread_mutex_lock(&w->lock);
    for (;;) {
        while (w->count == 0 && !w->closing) pthread_cond_wait(&w->cond, &w->lock);
        if (w->count == 0) break;   /* closing and drained */
        SrWriteSlot *slot = &w->slots[w->head];
        pthread_mutex_unlock(&w->lock);
        SrStatus status = sr_write_frame(w->sinks, w->sink_count, slot, &w->diag);
        pthread_mutex_lock(&w->lock);
        w->head = (w->head + 1) % SR_WRITE_SLOTS;
        --w->count;
        if (status != SR_OK) w->status = status;
        pthread_cond_broadcast(&w->cond);
        if (status != SR_OK) break;   /* later frames are not encoded */
    }
    pthread_mutex_unlock(&w->lock);
    return NULL;
}

static void sr_writer_free_buffers(SrWriter *w) {
    for (unsigned i = 0; i < SR_WRITE_SLOTS; ++i) {
        if (w->owned[i]) {
            free(w->slots[i].pixels);
            free(w->slots[i].audio);
            for (size_t k = 1; k < w->sink_count; ++k) free(w->slots[i].extra[k]);
        }
    }
}

/* Starts the writer; false (nothing to release) when it cannot run, and
 * the caller then encodes serially. Slot 0 borrows the run's own buffers. */
static bool sr_writer_start(SrWriter *w, SrRun *run, bool with_audio) {
    *w = (SrWriter){0};
    w->sinks = run->sinks;
    w->sink_count = run->pass->sink_count;
    w->status = SR_OK;
    bool ok = true;
    for (unsigned i = 0; i < SR_WRITE_SLOTS && ok; ++i) {
        SrWriteSlot *slot = &w->slots[i];
        if (i == 0) {
            slot->pixels = run->state->pixels;
            slot->audio = with_audio ? run->audio : NULL;
            for (size_t k = 1; k < w->sink_count; ++k)
                slot->extra[k] = run->sinks[k].pixels;
            continue;
        }
        w->owned[i] = true;
        slot->pixels = malloc(run->frame_bytes);
        slot->audio = with_audio ? malloc(run->audio_values * sizeof(float)) : NULL;
        ok = slot->pixels && (!with_audio || slot->audio);
        for (size_t k = 1; ok && k < w->sink_count; ++k) {
            slot->extra[k] = malloc(run->sinks[k].frame_bytes);
            ok = slot->extra[k] != NULL;
        }
    }
    if (ok) {
        w->log = open_memstream(&w->log_text, &w->log_size);
        ok = w->log != NULL;
    }
    if (!ok) {
        sr_writer_free_buffers(w);
        return false;
    }
    sr_diag_init(&w->diag, run->diag->source, w->log);
    w->diag.verbose = run->diag->verbose;
    bool mutex = pthread_mutex_init(&w->lock, NULL) == 0;
    bool cond = mutex && pthread_cond_init(&w->cond, NULL) == 0;
    if (cond && pthread_create(&w->thread, NULL, sr_writer_main, w) == 0)
        return true;
    if (cond) pthread_cond_destroy(&w->cond);
    if (mutex) pthread_mutex_destroy(&w->lock);
    fclose(w->log);
    free(w->log_text);
    sr_writer_free_buffers(w);
    return false;
}

/* The slot to render the next frame into (waits while every slot is
 * queued); NULL once the writer has failed, with *status its failure. */
static SrWriteSlot *sr_writer_acquire(SrWriter *w, SrStatus *status) {
    pthread_mutex_lock(&w->lock);
    while (w->count == SR_WRITE_SLOTS && w->status == SR_OK)
        pthread_cond_wait(&w->cond, &w->lock);
    *status = w->status;
    SrWriteSlot *slot = w->status == SR_OK
        ? &w->slots[(w->head + w->count) % SR_WRITE_SLOTS] : NULL;
    pthread_mutex_unlock(&w->lock);
    return slot;
}

/* Queues the slot sr_writer_acquire returned last. */
static void sr_writer_submit(SrWriter *w) {
    pthread_mutex_lock(&w->lock);
    ++w->count;
    pthread_cond_broadcast(&w->cond);
    pthread_mutex_unlock(&w->lock);
}

/* Lets the writer encode every queued frame, joins it, replays its
 * diagnostics into `diag` and releases it; returns its first failure. */
static SrStatus sr_writer_finish(SrWriter *w, SrDiagnostics *diag) {
    pthread_mutex_lock(&w->lock);
    w->closing = true;
    pthread_cond_broadcast(&w->cond);
    pthread_mutex_unlock(&w->lock);
    pthread_join(w->thread, NULL);
    pthread_cond_destroy(&w->cond);
    pthread_mutex_destroy(&w->lock);
    if (fclose(w->log) == 0 && w->log_size)
        fwrite(w->log_text, 1, w->log_size, diag->stream);
    free(w->log_text);
    diag->errors += w->diag.errors;
    diag->warnings += w->diag.warnings;
    sr_writer_free_buffers(w);
    return w->status;
}

/* Renders [first, end) into the open sink encoders on this thread. */
static SrStatus sr_run_frames_serial(SrRun *run, uint64_t first, uint64_t end,
                                     uint64_t total, bool with_audio) {
    SrStatus status = SR_OK;
    SrWriteSlot slot = {.pixels = run->state->pixels,
                        .audio = with_audio ? run->audio : NULL};
    for (size_t k = 1; k < run->pass->sink_count; ++k)
        slot.extra[k] = run->sinks[k].pixels;
    for (uint64_t index = first; index < end && status == SR_OK; ++index) {
        SrStageTimes times = {0};
        status = sr_run_render(run, index, &times);
        if (status == SR_OK)
            status = sr_run_convert_sinks(run, index, slot.extra, &times);
        if (status != SR_OK) break;
        SrStageMark mark = sr_stage_begin();
        slot.index = index;
        if (with_audio) slot.samples = sr_run_mix(run, index, run->audio);
        status = sr_write_frame(run->sinks, run->pass->sink_count, &slot, run->diag);
        sr_stage_end(&times, SR_STAGE_ENCODE, mark);
        if (status == SR_OK) sr_run_account(run, index, total, &times);
    }
    return status;
}

/* Renders [first, end) into the open sink encoders; audio too when
 * `with_audio`. Encoding runs on a writer thread unless --threads 1 asked
 * for a single worker (or the writer cannot start). The encode stage is the
 * time the render thread spends waiting for a free slot, mixing and
 * queueing, plus the final drain; the encoders' own busy time is
 * sr_encoder_seconds. */
static SrStatus sr_run_frames(SrRun *run, uint64_t first, uint64_t end,
                              uint64_t total, bool with_audio) {
    with_audio = with_audio && run->mixer;
    SrWriter writer;
    if (run->options->encoder_threads == 1 ||
        !sr_writer_start(&writer, run, with_audio))
        return sr_run_frames_serial(run, first, end, total, with_audio);
    void *own_pixels = run->state->pixels;
    SrStatus status = SR_OK;
    for (uint64_t index = first; index < end; ++index) {
        SrStageTimes times = {0};
        SrStageMark mark = sr_stage_begin();
        SrWriteSlot *slot = sr_writer_acquire(&writer, &status);
        sr_stage_end(&times, SR_STAGE_ENCODE, mark);
        if (!slot) break;
        run->state->pixels = slot->pixels;
        slot->index = index;
        status = sr_run_render(run, index, &times);
        if (status == SR_OK)
            status = sr_run_convert_sinks(run, index, slot->extra, &times);
        if (status != SR_OK) break;
        mark = sr_stage_begin();
        if (with_audio) slot->samples = sr_run_mix(run, index, slot->audio);
        sr_writer_submit(&writer);
        sr_stage_end(&times, SR_STAGE_ENCODE, mark);
        sr_run_account(run, index, total, &times);
    }
    run->state->pixels = own_pixels;
    SrStageMark mark = sr_stage_begin();
    SrStatus written = sr_writer_finish(&writer, run->diag);
    sr_stage_end(&run->metrics->stages, SR_STAGE_ENCODE, mark);
    return status != SR_OK ? status : written;
}

static SrEncoderAudio sr_run_audio_format(const SrRun *run) {
    return (SrEncoderAudio){run->scene->audio.sample_rate,
                            run->mixer ? run->channels : 0};
}

static bool sr_wants_spherical(const SrScene *scene, const SrOutput *output,
                               const char *path) {
    if (scene->project.mode != SR_MODE_EQUIRECTANGULAR || !output->spherical_metadata)
        return false;
    if (sr_codec_info(output->codec)->legacy && output->container == SR_CONTAINER_AUTO)
        return sr_spatial_is_mp4(path);
    const char *muxer = sr_output_muxer(output, path, NULL);
    return muxer && (!strcmp(muxer, "mp4") || !strcmp(muxer, "mov"));
}

/* Opens every active sink, renders the pass's frames into them and
 * finishes them all (a failure still finishes the others' files). */
static SrStatus sr_run_encode(SrRun *run, uint64_t first, uint64_t end) {
    SrEncoderAudio audio = sr_run_audio_format(run);
    size_t count = run->pass->sink_count;
    SrStatus status = SR_OK;
    bool any_audio = false;
    for (size_t k = 0; k < count && status == SR_OK; ++k) {
        SrRunSink *sink = &run->sinks[k];
        if (sink->plan->first >= sink->plan->end) continue;
        status = sr_make_parent_dirs(sink->plan->path, run->diag);
        if (status == SR_OK)
            status = sr_encoder_open_output(&sink->encoder, run->scene,
                                            sink->plan->output, sink->plan->path,
                                            run->options->encoder_threads, &audio,
                                            run->diag);
        if (status == SR_OK) {
            sr_encoder_set_first_number(sink->encoder, sink->plan->first);
            any_audio |= sr_encoder_has_audio(sink->encoder);
            if (run->mixer && !sr_encoder_has_audio(sink->encoder))
                sr_diag_info(run->diag, "output '%s' carries no audio",
                             sink->plan->path);
        }
    }
    if (status == SR_OK)
        status = sr_run_frames(run, first, end, end - first, any_audio);
    for (size_t k = 0; k < count; ++k) {
        SrRunSink *sink = &run->sinks[k];
        if (!sink->encoder) continue;
        SrStatus finished = sr_encoder_finish(sink->encoder, run->diag);
        if (status == SR_OK) status = finished;
        run->metrics->encode_seconds += sr_encoder_seconds(sink->encoder);
        sr_encoder_destroy(sink->encoder);
        sink->encoder = NULL;
        /* --resume injects into its temporary file before the rename. */
        if (status == SR_OK &&
            sr_wants_spherical(run->scene, sink->plan->output, sink->plan->path))
            status = sr_spatial_inject_mp4(sink->plan->path, run->scene->project.width,
                                           run->scene->project.height, run->diag);
    }
    return status;
}

#ifdef SR_TEST_HOOKS
/* Test hook, compiled only into scene-render-testhooks:
 * SR_TEST_ABORT_AFTER_SEGMENTS=N kills the process (SIGKILL, like an
 * interruption) once this run has committed N segments. */
static void sr_run_maybe_abort(uint64_t committed) {
    const char *value = getenv("SR_TEST_ABORT_AFTER_SEGMENTS");
    uint64_t limit;
    if (value && sr_parse_u64(value, &limit) && limit && committed >= limit) {
        fflush(NULL);
        raise(SIGKILL);
    }
}
#else
static void sr_run_maybe_abort(uint64_t committed) {
    (void)committed;
}
#endif

/* The colour-conversion backend frames are actually converted with. */
static void sr_run_backend(const SrRun *run, char *out, size_t size) {
    if (run->state->bits == 8 && run->state->allow_gpu && run->gpu &&
        run->gpu->implementation)
        snprintf(out, size, "opencl:%s", sr_gpu_device_name(run->gpu));
    else
        snprintf(out, size, "cpu");
}

/* Renders segment k into a fresh temporary file and commits it, unless the
 * inputs changed (SR_ERR_ASSET) or the backend no longer matches
 * `backend` (*backend_changed; nothing committed). */
static SrStatus sr_run_segment(SrRun *run, SrResume *resume, uint64_t k,
                               const char *backend, bool *backend_changed) {
    *backend_changed = false;
    uint64_t from, to;
    sr_resume_segment_range(resume, k, &from, &to);
    char *partial = NULL;
    SrStatus status = sr_resume_segment_begin(resume, k, &partial, run->diag);
    if (status != SR_OK) return status;
    SrRunSink *sink = &run->sinks[0];
    status = sr_encoder_open_segment_output(&sink->encoder, run->scene,
                                            sink->plan->output, partial,
                                            run->options->encoder_threads,
                                            run->diag);
    if (status == SR_OK) {
        status = sr_run_frames(run, from, to, resume->end - resume->first, false);
        SrStatus finished = sr_encoder_finish(sink->encoder, run->diag);
        if (status == SR_OK) status = finished;
    }
    run->metrics->encode_seconds += sr_encoder_seconds(sink->encoder);
    sr_encoder_destroy(sink->encoder);
    sink->encoder = NULL;
    if (status == SR_OK)
        status = sr_resume_inputs_verify(run->inputs, "rendering", run->diag);
    if (status == SR_OK) {
        char now[160];
        sr_run_backend(run, now, sizeof(now));
        *backend_changed = strcmp(now, backend) != 0;
    }
    if (status == SR_OK && !*backend_changed)
        status = sr_resume_segment_commit(resume, k, partial, run->diag);
    else
        sr_resume_segment_abandon(resume, partial);
    free(partial);
    return status;
}

/* Packet-copies every committed segment into `path` and encodes the audio
 * of the whole range once. */
static SrStatus sr_run_assemble(SrRun *run, const SrResume *resume,
                                const char *path) {
    char *template_path = sr_resume_segment_path(resume, 0);
    if (!template_path) return SR_ERR_MEMORY;
    SrEncoderAudio audio = sr_run_audio_format(run);
    SrEncoder *encoder = NULL;
    SrStatus status = sr_encoder_open_copy_output(&encoder, run->scene,
                                                  run->sinks[0].plan->output, path,
                                                  template_path, &audio, run->diag);
    free(template_path);
    for (uint64_t k = 0; status == SR_OK && k < resume->segment_count; ++k) {
        uint64_t from, to;
        sr_resume_segment_range(resume, k, &from, &to);
        char *segment = sr_resume_segment_path(resume, k);
        status = segment ? sr_encoder_copy_video(encoder, segment,
                                                 from - resume->first, to - from,
                                                 run->diag)
                         : SR_ERR_MEMORY;
        free(segment);
        for (uint64_t index = from; status == SR_OK && run->mixer && index < to;
             ++index) {
            size_t samples = sr_run_mix(run, index, run->audio);
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

/* A committed segment is reused only when it demuxes as expected;
 * otherwise it is deleted and rendered again. */
static SrStatus sr_run_check_segment(SrRun *run, SrResume *resume, uint64_t k,
                                     bool *valid) {
    *valid = false;
    if (!sr_resume_segment_done(resume, k)) return SR_OK;
    uint64_t from, to;
    sr_resume_segment_range(resume, k, &from, &to);
    char *path = sr_resume_segment_path(resume, k);
    if (!path) return SR_ERR_MEMORY;
    char why[256];
    SrStatus status = sr_encoder_check_segment_output(run->scene,
                                                      run->sinks[0].plan->output,
                                                      path, to - from, why,
                                                      sizeof(why));
    if (status == SR_OK) {
        *valid = true;
    } else if (status == SR_ERR_ENCODER) {
        sr_diag_warning(run->diag, 0, NULL, NULL,
                        "segment '%s' %s; rendering it again", path, why);
        status = sr_resume_segment_discard(resume, k, run->diag);
    }
    free(path);
    return status;
}

static SrStatus sr_run_resume(SrRun *run, const char *path, uint64_t first,
                              uint64_t end) {
    const SrRenderOptions *options = run->options;
    /* Everything loaded (audio included) must still be what was hashed. */
    SrStatus status = sr_resume_inputs_verify(run->inputs, "loading", run->diag);
    if (status != SR_OK) return status;
    char backend[160];
    sr_run_backend(run, backend, sizeof(backend));
    SrResumeSettings settings = {
        options->encoder_threads ? options->encoder_threads
                                 : sr_parallel_thread_count(0, SIZE_MAX),
        run->state->bits, backend, run->sinks[0].plan->output};
    uint32_t segment_frames = options->segment_frames
                                  ? options->segment_frames
                                  : SR_RESUME_DEFAULT_SEGMENT_FRAMES;
    SrResume resume;
    status = sr_resume_prepare(&resume, run->scene, path, first, end,
                               segment_frames, run->inputs, &settings, run->diag);
    if (status != SR_OK) return status;
    uint64_t committed = 0;
    for (uint64_t k = 0; status == SR_OK && k < resume.segment_count; ++k) {
        bool valid = false;
        status = sr_run_check_segment(run, &resume, k, &valid);
        if (status != SR_OK) break;
        if (valid) {
            ++run->metrics->segments_reused;
            continue;
        }
        bool changed = false;
        status = sr_run_segment(run, &resume, k, backend, &changed);
        if (status == SR_OK && changed) {
            /* The GPU fell back to the CPU: the manifest must name the
             * backend every kept segment was converted with, so start
             * over under a new manifest (this happens at most once). */
            sr_run_backend(run, backend, sizeof(backend));
            sr_diag_warning(run->diag, 0, NULL, NULL,
                            "colour conversion backend changed to %s; "
                            "re-rendering every segment", backend);
            status = sr_resume_sync(&resume, run->scene, run->inputs, &settings,
                                    run->diag);
            run->metrics->segments_reused = 0;
            k = (uint64_t)-1;   /* ++k: segment 0 */
            continue;
        }
        if (status == SR_OK) {
            ++run->metrics->segments_rendered;
            sr_run_maybe_abort(++committed);
        }
    }
    /* The previous OUTPUT stays until the new one is complete. */
    char *temporary = NULL;
    if (status == SR_OK) status = sr_output_temp_create(path, &temporary, run->diag);
    if (status == SR_OK) {
        SrStageMark mark = sr_stage_begin();
        status = sr_run_assemble(run, &resume, temporary);
        if (status == SR_OK &&
            sr_wants_spherical(run->scene, run->sinks[0].plan->output, path))
            status = sr_spatial_inject_mp4(temporary, run->scene->project.width,
                                           run->scene->project.height, run->diag);
        if (status == SR_OK)
            status = sr_output_temp_commit(temporary, path, run->diag);
        else
            unlink(temporary);
        sr_stage_end(&run->metrics->stages, SR_STAGE_RESUME, mark);
    }
    free(temporary);
    if (status == SR_OK && !options->keep_parts)
        status = sr_resume_remove(&resume, run->diag);
    sr_diag_info(run->diag, "resume: %llu segment(s) rendered, %llu reused",
                 (unsigned long long)run->metrics->segments_rendered,
                 (unsigned long long)run->metrics->segments_reused);
    sr_resume_close(&resume);
    return status;
}

/* Poster and thumbnail frames of the pass, after its video: each is
 * rendered again (frames are a pure function of the index) and converted
 * to 8 bits in its own output's colour space. */
static SrStatus sr_run_stills(SrRun *run) {
    const SrPlanPass *pass = run->pass;
    SrStatus status = SR_OK;
    uint8_t *rgba = NULL;
    if (pass->still_count) {
        rgba = sr_alloc((size_t)run->frame->width * run->frame->height * 4);
        if (!rgba) return SR_ERR_MEMORY;
    }
    for (size_t i = 0; i < pass->still_count && status == SR_OK; ++i) {
        const SrPlanStill *still = &pass->stills[i];
        SrStageTimes times = {0};
        status = sr_run_render(run, still->frame, &times);
        SrColorOutput color;
        if (status == SR_OK)
            status = sr_color_output_init_bits(&color, &run->scene->project,
                                               still->output->color_space, 8);
        if (status == SR_OK) {
            status = sr_color_convert_frame(&color, run->frame, rgba,
                                            run->options->encoder_threads);
            sr_color_output_free(&color);
        }
        if (status == SR_OK) status = sr_make_parent_dirs(still->path, run->diag);
        if (status == SR_OK)
            status = sr_output_write_still(still->still, still->path,
                                           run->frame->width, run->frame->height,
                                           rgba, run->diag);
        if (status == SR_OK) sr_stage_accumulate(run->metrics, &times);
    }
    free(rgba);
    return status;
}

static void sr_run_sinks_free(SrRunSink *sinks, size_t count) {
    for (size_t k = 0; sinks && k < count; ++k) {
        sr_encoder_destroy(sinks[k].encoder);
        sr_color_output_free(&sinks[k].color);
        free(sinks[k].pixels);
    }
    free(sinks);
}

/* One pass of the plan: the pre-B1-6 body of sr_render, at the pass's
 * size and rate (already applied to scene->project). */
static SrStatus render_pass(SrScene *scene, const SrRenderOptions *options,
                            const SrPlanPass *pass, SrRenderMetrics *metrics,
                            FILE *trace, SrDiagnostics *diag) {
    uint64_t video_totals[4] = {0};
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
    /* --resume fingerprints every input file before it is loaded, so the
     * manifest describes the bytes the frames were rendered from. */
    SrResumeInputs inputs = {0};
    SrStatus status = options->resume && !options->hash && !options->preview
                          ? sr_resume_inputs_capture(&inputs, scene, diag)
                          : SR_OK;
    if (status == SR_OK) status = sr_assets_load(scene, diag);
    if (status == SR_OK && options->resume && !options->hash && !options->preview)
        status = sr_resume_inputs_add_fonts(&inputs, scene, diag);
    if (status == SR_OK) status = sr_physics_prepare(scene, diag);
    metrics->physics_steps += scene->physics.steps_simulated;
    metrics->physics_cache_hit = scene->physics.cache_hit;
    metrics->setup_wall_seconds += sr_monotonic_seconds() - setup.wall;
    metrics->setup_cpu_seconds += sr_process_cpu_seconds() - setup.cpu;
    if (status != SR_OK) {
        sr_resume_inputs_free(&inputs);
        sr_gpu_close(&gpu);
        return status;
    }
    uint32_t canvas_width = scene->project.mode == SR_MODE_VIEWPORT
        ? scene->scene360.width : scene->project.width;
    uint32_t canvas_height = scene->project.mode == SR_MODE_VIEWPORT
        ? scene->scene360.height : scene->project.height;
    SrFrame composition = {0}, viewport = {0};
    SrFrameState state = {0};
    SrRunSink *sinks = NULL;
    size_t sink_count = pass->sink_count;
    const SrOutput *first_output = sink_count ? pass->sinks[0].output
                                 : pass->stills[0].output;
    sr_compositor_init(&state.compositor, options->encoder_threads);
    status = sr_frame_init(&composition, canvas_width, canvas_height);
    if (status != SR_OK) {
        sr_diag_error(diag, 0, NULL, NULL, "cannot allocate %ux%u float RGBA frame",
                      canvas_width, canvas_height);
        sr_resume_inputs_free(&inputs);
        sr_gpu_close(&gpu);
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
    state.bits = options->preview || !sink_count ? 8
                                                 : sr_output_input_bits(first_output);
    state.space = first_output->color_space;
    state.allow_gpu = sink_count <= 1 && sr_output_is_legacy(first_output);
    status = sr_color_output_init_bits(&state.color, &scene->project,
                                       first_output->color_space,
                                       state.bits == 32 ? 16 : state.bits);
    if (status != SR_OK) goto cleanup;
    size_t frame_bytes = (size_t)frame->width * frame->height * 4 * (state.bits / 8);
    state.pixels = sr_alloc(frame_bytes);
    if (!state.pixels) {
        status = SR_ERR_MEMORY;
        goto cleanup;
    }
    sinks = sr_alloc((sink_count ? sink_count : 1) * sizeof(*sinks));
    if (!sinks) {
        status = SR_ERR_MEMORY;
        goto cleanup;
    }
    memset(sinks, 0, (sink_count ? sink_count : 1) * sizeof(*sinks));
    for (size_t k = 0; k < sink_count; ++k) {
        sinks[k].plan = &pass->sinks[k];
        sinks[k].bits = k == 0 ? state.bits : sr_output_input_bits(sinks[k].plan->output);
        sinks[k].frame_bytes = (size_t)frame->width * frame->height * 4 *
                               (sinks[k].bits / 8);
    }
    for (size_t k = 1; k < sink_count && !options->preview && !options->hash; ++k) {
        status = sr_color_output_init_bits(&sinks[k].color, &scene->project,
                                           sinks[k].plan->output->color_space,
                                           sinks[k].bits == 32 ? 16 : sinks[k].bits);
        if (status == SR_OK) {
            sinks[k].pixels = sr_alloc(sinks[k].frame_bytes);
            if (!sinks[k].pixels) status = SR_ERR_MEMORY;
        }
        if (status != SR_OK) goto cleanup;
    }
    uint64_t total_frames = pass->total_frames;
    /* The pass renders the union of its sinks' ranges. */
    uint64_t first = UINT64_MAX, end = 0;
    for (size_t k = 0; k < sink_count; ++k) {
        if (pass->sinks[k].first >= pass->sinks[k].end) continue;
        if (pass->sinks[k].first < first) first = pass->sinks[k].first;
        if (pass->sinks[k].end > end) end = pass->sinks[k].end;
    }
    if (first >= end && !pass->still_count) {
        uint64_t from = options->has_range ? options->first_frame : 0;
        uint64_t to = options->has_range ? options->end_frame : total_frames;
        if (to > total_frames) to = total_frames;
        sr_diag_error(diag, 0, NULL, NULL,
                      "empty frame range [%llu, %llu) for %llu-frame scene",
                      (unsigned long long)from, (unsigned long long)to,
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
                 &gpu, trace, frame_bytes, NULL, NULL, 0, &inputs, 0, pass, sinks};
    status = sr_run_audio_open(&run);
    if (status == SR_OK) {
        if (options->hash) {
            status = sr_run_hash(&run, first, end);
        } else {
            if (first < end && options->resume) {
                status = sr_make_parent_dirs(sinks[0].plan->path, diag);
                if (status == SR_OK)
                    status = sr_run_resume(&run, sinks[0].plan->path, first, end);
            } else if (first < end) {
                status = sr_run_encode(&run, first, end);
            }
            if (status == SR_OK) status = sr_run_stills(&run);
        }
    }
    sr_mixer_destroy(run.mixer);
    free(run.audio);

cleanup:
    sr_run_sinks_free(sinks, sink_count);
    sr_resume_inputs_free(&inputs);
    sr_assets_video_stats(scene, &metrics->video_sources, video_totals);
    metrics->video_requests += video_totals[0];
    metrics->video_cache_hits += video_totals[1];
    metrics->video_decoded += video_totals[2];
    metrics->video_seeks += video_totals[3];
    sr_gpu_close(&gpu);
    sr_compositor_free(&state.compositor);
    /* Effect scratch and transfer tables are cached per thread across
     * frames; release them so a finished render leaves nothing live. */
    sr_effects_release();
    sr_lighting_release();
    sr_color_output_free(&state.color);
    free(state.pixels);
    sr_frame_free(&viewport);
    sr_frame_free(&composition);
    return status;
}

SrStatus sr_render(SrScene *scene, const SrRenderOptions *options,
                   SrRenderMetrics *metrics, SrDiagnostics *diag) {
    if (!scene || !options || !metrics) {
        return SR_ERR_ARGUMENT;
    }
    *metrics = (SrRenderMetrics){0};
    SrStatus ready = sr_composite_scene_ready(scene, diag);
    if (ready != SR_OK) return ready;
    if (scene->has_cards && scene->project.mode != SR_MODE_STANDARD) {
        sr_diag_error(diag, 0, "project", "mode",
                      "depth cards require mode standard; equirectangular and "
                      "viewport canvases are not projected by a camera");
        return SR_ERR_ARGUMENT;
    }
    SrOutputPlan plan;
    SrStatus status = sr_output_plan_build(&plan, scene, options, diag);
    if (status != SR_OK || options->validate_only) {
        sr_output_plan_free(&plan);
        return status;
    }
    for (size_t p = 0; !options->preview && !options->hash && p < plan.pass_count;
         ++p) {
        const SrPlanPass *pass = &plan.passes[p];
        for (size_t k = 0; k < pass->sink_count && status == SR_OK; ++k) {
            const SrPlanSink *sink = &pass->sinks[k];
            /* The 1.0 check saw the unjoined path; the extension is the same. */
            const char *path = options->output_override
                ? options->output_override : sink->path;
            status = sr_encoder_validate_output_metadata(scene, sink->output, path,
                                                         diag);
        }
    }
    if (status != SR_OK) {
        sr_output_plan_free(&plan);
        return status;
    }
    double wall_start = sr_monotonic_seconds();
    FILE *trace = NULL;
    if (options->trace_path) {
        if (sr_make_parent_dirs(options->trace_path, diag) != SR_OK) {
            sr_output_plan_free(&plan);
            return SR_ERR_IO;
        }
        trace = fopen(options->trace_path, "w");
        if (!trace) {
            sr_diag_error(diag, 0, NULL, NULL, "cannot write trace '%s': %s",
                          options->trace_path, strerror(errno));
            sr_output_plan_free(&plan);
            return SR_ERR_IO;
        }
    }
    SrProject saved = scene->project;
    for (size_t p = 0; p < plan.pass_count && status == SR_OK; ++p) {
        const SrPlanPass *pass = &plan.passes[p];
        /* Between passes only: frames never see a project change. */
        scene->project.width = pass->width;
        scene->project.height = pass->height;
        scene->project.fps_num = pass->fps_num;
        scene->project.fps_den = pass->fps_den;
        status = render_pass(scene, options, pass, metrics, trace, diag);
        scene->project = saved;
    }
    sr_output_plan_free(&plan);
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
