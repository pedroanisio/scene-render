/* SPDX-License-Identifier: Apache-2.0 */
/* Load-time checks and per-codec defaults of every <output> (B1-6). Each
 * check names the output's line and attribute. 1.0 codecs keep their 1.0
 * acceptance: nothing here rejects a document the 1.0 loader accepted. */
#include "scene_render/outputs.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static bool set_default(char **field, const char *value) {
    char *copy = sr_strdup(value);
    if (!copy) return false;
    free(*field);
    *field = copy;
    return true;
}

#define FAIL(line, element, attribute, ...)                               \
    do {                                                                  \
        sr_diag_error(diag, (line), (element), (attribute), __VA_ARGS__); \
        return SR_ERR_XML;                                                \
    } while (0)

static bool is_prores_444(SrProresProfile profile) {
    return profile == SR_PRORES_4444 || profile == SR_PRORES_4444XQ;
}

static SrStatus resolve_prores(SrOutput *o, SrDiagnostics *diag) {
    if (o->prores_profile == SR_PRORES_AUTO) o->prores_profile = SR_PRORES_422;
    bool wide = is_prores_444(o->prores_profile);
    if (!o->pixel_format_authored) {
        return set_default(&o->pixel_format, wide ? "yuv444p10le" : "yuv422p10le")
            ? SR_OK : SR_ERR_MEMORY;
    }
    bool is444 = !strcmp(o->pixel_format, "yuv444p10le") ||
                 !strcmp(o->pixel_format, "yuva444p10le");
    if (wide != is444)
        FAIL(o->source_line, "output", "pixelFormat",
             "proresProfile %s needs pixelFormat %s", wide ? "4444/4444xq" :
             "proxy/lt/422/hq", wide ? "yuv444p10le or yuva444p10le" :
             "yuv422p10le");
    return SR_OK;
}

/* Attributes that mean nothing for the codec are errors, never ignored. */
static SrStatus check_applicability(const SrOutput *o, const char *muxer,
                                    SrDiagnostics *diag) {
    const SrCodecInfo *info = sr_codec_info(o->codec);
    size_t line = o->source_line;
    if (!info->legacy && !info->rate_control) {
        if (o->crf_authored) FAIL(line, "output", "crf",
                                  "crf does not apply to codec %s", info->name);
        if (o->preset_authored) FAIL(line, "output", "preset",
                                     "preset does not apply to codec %s", info->name);
        if (o->bitrate_authored) FAIL(line, "output", "bitrate",
                                      "bitrate does not apply to codec %s", info->name);
    }
    if (o->keyframe_interval > 0.0 && !info->gop)
        FAIL(line, "output", "keyframeInterval",
             "keyframeInterval does not apply to codec %s", info->name);
    if (o->b_frames >= 0 && !info->b_frames)
        FAIL(line, "output", "bFrames", "bFrames applies only to h264 and h265");
    if (o->loop_count_authored && !info->loops)
        FAIL(line, "output", "loopCount", "loopCount applies only to gif and apng");
    if (o->prores_profile != SR_PRORES_AUTO && o->codec != SR_CODEC_PRORES)
        FAIL(line, "output", "proresProfile", "proresProfile applies only to prores");
    if (o->container != SR_CONTAINER_AUTO && (info->sequence || info->muxer))
        FAIL(line, "output", "container", "container does not apply to codec %s",
             info->name);
    if (o->faststart_authored && muxer && strcmp(muxer, "mp4") && strcmp(muxer, "mov"))
        FAIL(line, "output", "faststart", "faststart applies only to mp4 and mov");
    if (o->faststart_authored && !muxer)
        FAIL(line, "output", "faststart", "faststart applies only to mp4 and mov");
    return SR_OK;
}

static SrStatus resolve_codec(SrOutput *o, SrDiagnostics *diag) {
    const SrCodecInfo *info = sr_codec_info(o->codec);
    const char *muxer = NULL;
    /* 1.0 derives the container when the encoder opens (an --output
     * override may supply the extension); keep that for 1.0 codecs. */
    bool derive = !info->legacy || o->container != SR_CONTAINER_AUTO ||
                  o->faststart_authored;
    if (derive && !info->sequence) {
        const char *why = NULL;
        muxer = sr_output_muxer(o, o->path, &why);
        if (!muxer)
            FAIL(o->source_line, "output", o->container ? "container" : "path",
                 "output '%s' %s", o->path, why ? why : "has no container");
    }
    SrStatus status = check_applicability(o, muxer, diag);
    if (status != SR_OK) return status;
    if (info->legacy) return SR_OK;
    if (o->codec == SR_CODEC_PRORES) {
        status = resolve_prores(o, diag);
        if (status != SR_OK) return status;
    } else if (!o->pixel_format_authored &&
               !set_default(&o->pixel_format, info->default_pixel_format)) {
        return SR_ERR_MEMORY;
    }
    if ((o->codec == SR_CODEC_VP9 || o->codec == SR_CODEC_AV1) &&
        sr_output_preset_level(o->codec, o->preset) < 0)
        FAIL(o->source_line, "output", "preset",
             "expected ultrafast, superfast, veryfast, faster, fast, medium, "
             "slow, slower, veryslow or placebo");
    if (o->codec == SR_CODEC_AV1 && o->crf_authored && o->crf == 0)
        FAIL(o->source_line, "output", "crf",
             "av1 crf must be in [1,63] (FFmpeg 7.1 ignores 0)");
    if (o->codec == SR_CODEC_GIF && strcmp(o->pixel_format, "pal8"))
        FAIL(o->source_line, "output", "pixelFormat",
             "gif output uses pixelFormat pal8 (a per-frame palette)");
    if (o->codec == SR_CODEC_EXR_SEQUENCE && strcmp(o->pixel_format, "gbrpf32le") &&
        strcmp(o->pixel_format, "gbrapf32le"))
        FAIL(o->source_line, "output", "pixelFormat",
             "exr-sequence expects pixelFormat gbrpf32le or gbrapf32le");
    if (info->sequence && !sr_sequence_pattern_valid(o->path))
        FAIL(o->source_line, "output", "path",
             "sequence path '%s' needs exactly one %%d or %%0Nd (N 1-9) "
             "conversion; write %%%% for a literal percent sign", o->path);
    if (muxer && !strcmp(muxer, "webm") && !o->audio_codec_authored &&
        !set_default(&o->audio_codec, "libopus"))
        return SR_ERR_MEMORY;
    if (o->keyframe_interval == 0.0 && info->gop) o->keyframe_interval = 2.0;
    return SR_OK;
}

static SrStatus resolve_range(const SrScene *scene, const SrOutput *o,
                              SrDiagnostics *diag) {
    double duration = scene->project.duration;
    double end = o->has_end ? o->end : duration;
    if (!(o->start < duration))
        FAIL(o->source_line, "output", "start",
             "start %.17g must be before the project duration %.17g", o->start,
             duration);
    if (!(end > o->start) || end > duration)
        FAIL(o->source_line, "output", "end",
             "expected start < end <= project duration (%.17g)", duration);
    if ((o->width || o->height) && scene->project.mode == SR_MODE_EQUIRECTANGULAR &&
        ((o->width && o->width != scene->project.width) ||
         (o->height && o->height != scene->project.height)))
        FAIL(o->source_line, "output", "width/height",
             "an equirectangular project renders at its panorama size; "
             "per-output sizes need mode standard or viewport");
    if (o->fps_num && ((uint64_t)o->fps_num < o->fps_den ||
                       (uint64_t)o->fps_num > 1000u * (uint64_t)o->fps_den))
        FAIL(o->source_line, "output", "fps",
             "expected between 1 and 1000 frames per second");
    for (size_t i = 0; i < o->still_count; ++i) {
        const SrStill *still = &o->stills[i];
        const char *element = still->kind == SR_STILL_POSTER ? "poster" : "thumbnail";
        if (!(still->time >= o->start && still->time < end))
            FAIL(still->source_line, element, "time",
                 "time %.17g must lie inside its output's range [%.17g, %.17g)",
                 still->time, o->start, end);
    }
    return SR_OK;
}

/* Every file an output writes is distinct: two writers of one path would
 * silently overwrite each other. Paths are compared as authored (they all
 * resolve against the scene directory); the plan repeats the check on the
 * effective paths. */
static SrStatus check_paths(const SrScene *scene, SrDiagnostics *diag) {
    size_t count = sr_scene_output_count(scene);
    for (size_t i = 0; i < count; ++i) {
        const SrOutput *a = sr_scene_output_const(scene, i);
        for (size_t s = 0; s <= a->still_count; ++s) {
            const char *path = s == 0 ? a->path : a->stills[s - 1].path;
            bool sequence = s == 0 && sr_codec_info(a->codec)->sequence;
            size_t line = s == 0 ? a->source_line : a->stills[s - 1].source_line;
            for (size_t j = i; j < count; ++j) {
                const SrOutput *b = sr_scene_output_const(scene, j);
                for (size_t t = j == i ? s + 1 : 0; t <= b->still_count; ++t) {
                    const char *other = t == 0 ? b->path : b->stills[t - 1].path;
                    bool other_sequence = t == 0 && sr_codec_info(b->codec)->sequence;
                    size_t other_line = t == 0 ? b->source_line
                                               : b->stills[t - 1].source_line;
                    if (sr_output_paths_collide(path, sequence, other, other_sequence))
                        FAIL(other_line, t == 0 ? "output" : "poster/thumbnail",
                             "path", "path '%s' can overwrite a file written by "
                             "line %zu", other, line);
                }
            }
        }
    }
    return SR_OK;
}

SrStatus sr_outputs_resolve(SrScene *scene, SrDiagnostics *diag) {
    size_t count = sr_scene_output_count(scene);
    for (size_t i = 0; i < count; ++i) {
        SrOutput *o = sr_scene_output_at(scene, i);
        if (count > 1 && !o->id)
            FAIL(o->source_line, "output", "id",
                 "every output needs an id when the scene has several outputs");
        for (size_t j = 0; o->id && j < i; ++j) {
            const SrOutput *other = sr_scene_output_const(scene, j);
            if (other->id && !strcmp(other->id, o->id))
                FAIL(o->source_line, "output", "id", "duplicate output id '%s'",
                     o->id);
        }
        SrStatus status = resolve_range(scene, o, diag);
        if (status == SR_OK) status = resolve_codec(o, diag);
        if (status != SR_OK) return status;
    }
    return check_paths(scene, diag);
}

#undef FAIL
