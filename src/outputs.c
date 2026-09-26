/* SPDX-License-Identifier: Apache-2.0 */
/* Output descriptions: codec table, containers, sequence patterns and the
 * load-time cross-field checks of every <output> (B1-6). */
#include "scene_render/outputs.h"
#include "scene_render/encoder.h"

#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>

#define C(x) (1u << (x))

static const SrCodecInfo codecs[] = {
    [SR_CODEC_H264] = {"h264", "libx264", true, false, true, true, true, false,
                       true, true, 51,
                       C(SR_CONTAINER_MP4) | C(SR_CONTAINER_MOV) | C(SR_CONTAINER_MKV),
                       SR_CONTAINER_MP4, "yuv420p", NULL},
    [SR_CODEC_H265] = {"h265", "libx265", true, false, true, true, true, false,
                       true, true, 51,
                       C(SR_CONTAINER_MP4) | C(SR_CONTAINER_MOV) | C(SR_CONTAINER_MKV),
                       SR_CONTAINER_MP4, "yuv420p", NULL},
    [SR_CODEC_FFV1] = {"ffv1", "ffv1", true, false, false, false, false, false,
                       true, true, 51, C(SR_CONTAINER_MKV), SR_CONTAINER_MKV,
                       "yuv420p", NULL},
    [SR_CODEC_PRORES] = {"prores", "prores_ks", false, false, false, false, false,
                         false, true, true, 0,
                         C(SR_CONTAINER_MOV) | C(SR_CONTAINER_MKV),
                         SR_CONTAINER_MOV, "yuv422p10le", NULL},
    [SR_CODEC_VP9] = {"vp9", "libvpx-vp9", false, false, true, true, false, false,
                      true, true, 63,
                      C(SR_CONTAINER_WEBM) | C(SR_CONTAINER_MKV) | C(SR_CONTAINER_MP4),
                      SR_CONTAINER_WEBM, "yuv420p", NULL},
    [SR_CODEC_AV1] = {"av1", "libsvtav1", false, false, true, true, false, false,
                      true, true, 63,
                      C(SR_CONTAINER_MP4) | C(SR_CONTAINER_MKV) | C(SR_CONTAINER_WEBM),
                      SR_CONTAINER_MP4, "yuv420p", NULL},
    [SR_CODEC_GIF] = {"gif", "gif", false, false, false, false, false, true,
                      false, false, 0, 0, SR_CONTAINER_AUTO, "pal8", "gif"},
    [SR_CODEC_APNG] = {"apng", "apng", false, false, false, false, false, true,
                       false, false, 0, 0, SR_CONTAINER_AUTO, "rgb24", "apng"},
    [SR_CODEC_PNG_SEQUENCE] = {"png-sequence", "png", false, true, false, false,
                               false, false, false, false, 0, 0,
                               SR_CONTAINER_AUTO, "rgb24", NULL},
    [SR_CODEC_TIFF_SEQUENCE] = {"tiff-sequence", "tiff", false, true, false, false,
                                false, false, false, false, 0, 0,
                                SR_CONTAINER_AUTO, "rgb24", NULL},
    [SR_CODEC_EXR_SEQUENCE] = {"exr-sequence", "exr", false, true, false, false,
                               false, false, false, false, 0, 0,
                               SR_CONTAINER_AUTO, "gbrpf32le", NULL},
};

#undef C

const SrCodecInfo *sr_codec_info(SrCodec codec) {
    size_t count = sizeof(codecs) / sizeof(codecs[0]);
    return (size_t)codec < count ? &codecs[codec] : &codecs[SR_CODEC_H264];
}

bool sr_codec_parse(const char *text, SrCodec *codec) {
    for (size_t i = 0; text && i < sizeof(codecs) / sizeof(codecs[0]); ++i) {
        if (!strcmp(text, codecs[i].name)) {
            *codec = (SrCodec)i;
            return true;
        }
    }
    return false;
}

static const char *const container_names[] = {"auto", "mp4", "mov", "mkv", "webm"};

bool sr_container_parse(const char *text, SrContainer *container) {
    for (size_t i = 1; text && i < sizeof(container_names) / sizeof(*container_names);
         ++i) {
        if (!strcmp(text, container_names[i])) {
            *container = (SrContainer)i;
            return true;
        }
    }
    return false;
}

const char *sr_container_name(SrContainer container) {
    return (size_t)container < sizeof(container_names) / sizeof(*container_names)
        ? container_names[container] : "auto";
}

static const char *const muxer_names[] = {NULL, "mp4", "mov", "matroska", "webm"};

SrStatus sr_output_init(SrOutput *output) {
    *output = (SrOutput){
        .codec = SR_CODEC_H264, .crf = 18, .audio_bitrate = 192000,
        .color_space = SR_COLOR_SRGB, .spherical_metadata = true,
        .embed_metadata = true, .b_frames = -1, .faststart = true};
    output->path = sr_strdup("build/output.mp4");
    output->pixel_format = sr_strdup("yuv420p");
    output->preset = sr_strdup("medium");
    output->audio_codec = sr_strdup("aac");
    if (!output->path || !output->pixel_format || !output->preset ||
        !output->audio_codec) {
        sr_output_free(output);
        return SR_ERR_MEMORY;
    }
    return SR_OK;
}

void sr_output_free(SrOutput *output) {
    if (!output) return;
    free(output->path);
    free(output->pixel_format);
    free(output->preset);
    free(output->audio_codec);
    free(output->id);
    for (size_t i = 0; i < output->still_count; ++i) free(output->stills[i].path);
    free(output->stills);
    output->path = output->pixel_format = output->preset = NULL;
    output->audio_codec = output->id = NULL;
    output->stills = NULL;
    output->still_count = 0;
}

size_t sr_scene_output_count(const SrScene *scene) {
    return scene ? 1 + scene->extra_output_count : 0;
}

SrOutput *sr_scene_output_at(SrScene *scene, size_t index) {
    if (!scene || index > scene->extra_output_count) return NULL;
    return index == 0 ? &scene->output : &scene->extra_outputs[index - 1];
}

const SrOutput *sr_scene_output_const(const SrScene *scene, size_t index) {
    return sr_scene_output_at((SrScene *)scene, index);
}

SrOutput *sr_scene_add_extra_output(SrScene *scene) {
    if (scene->extra_output_count == scene->extra_output_capacity) {
        size_t capacity = scene->extra_output_capacity
            ? scene->extra_output_capacity * 2 : 4;
        if (capacity > SR_MAX_OUTPUTS) capacity = SR_MAX_OUTPUTS;
        if (capacity <= scene->extra_output_count) return NULL;
        SrOutput *grown = sr_realloc(scene->extra_outputs,
                                     capacity * sizeof(*grown));
        if (!grown) return NULL;
        scene->extra_outputs = grown;
        scene->extra_output_capacity = capacity;
    }
    SrOutput *output = &scene->extra_outputs[scene->extra_output_count];
    if (sr_output_init(output) != SR_OK) return NULL;
    ++scene->extra_output_count;
    return output;
}

bool sr_output_is_legacy(const SrOutput *o) {
    return sr_codec_info(o->codec)->legacy && o->container == SR_CONTAINER_AUTO &&
           !o->width && !o->height && !o->fps_num && o->start == 0.0 &&
           !o->has_end && o->keyframe_interval == 0.0 && o->b_frames < 0 &&
           !o->faststart_authored && !o->loop_count_authored &&
           o->prores_profile == SR_PRORES_AUTO && !o->still_count;
}

static SrContainer container_from_path(const char *path) {
    const char *dot = path ? strrchr(path, '.') : NULL;
    const char *slash = path ? strrchr(path, '/') : NULL;
    if (!dot || (slash && dot < slash)) return SR_CONTAINER_AUTO;
    if (!strcasecmp(dot, ".mp4")) return SR_CONTAINER_MP4;
    if (!strcasecmp(dot, ".mov")) return SR_CONTAINER_MOV;
    if (!strcasecmp(dot, ".mkv")) return SR_CONTAINER_MKV;
    if (!strcasecmp(dot, ".webm")) return SR_CONTAINER_WEBM;
    return SR_CONTAINER_AUTO;
}

const char *sr_output_muxer(const SrOutput *output, const char *path,
                            const char **why) {
    static const char *const legacy_why = "needs a .mp4, .mov or .mkv extension";
    const SrCodecInfo *info = sr_codec_info(output->codec);
    if (why) *why = NULL;
    if (info->sequence) {
        if (why) *why = "is an image sequence";
        return NULL;
    }
    if (info->muxer) return info->muxer;
    SrContainer container = output->container;
    if (container == SR_CONTAINER_AUTO) {
        container = container_from_path(path);
        if (info->legacy) {
            /* The 1.0 rule: the extension decides; .webm was not known. */
            if (container == SR_CONTAINER_WEBM) container = SR_CONTAINER_AUTO;
            if (container == SR_CONTAINER_AUTO) {
                if (why) *why = legacy_why;
                return NULL;
            }
        } else if (container == SR_CONTAINER_AUTO) {
            container = info->default_container;
        }
    }
    if (!(info->containers & (1u << container))) {
        if (why) *why = "names a container this codec cannot be stored in";
        return NULL;
    }
    return muxer_names[container];
}

unsigned sr_output_input_bits(const SrOutput *output) {
    if (output->codec == SR_CODEC_EXR_SEQUENCE) return 32;
    if (output->codec == SR_CODEC_GIF) return 8;
    return sr_encoder_input_bits(output->pixel_format);
}

/* Walks the pattern; with validate_only it writes nothing. */
static bool sequence_walk(const char *pattern, uint64_t number, char *out,
                          size_t size, bool validate_only) {
    if (!pattern || !*pattern) return false;
    size_t used = 0;
    unsigned conversions = 0;
    for (const char *at = pattern; *at; ++at) {
        char piece[32];
        size_t length;
        if (*at != '%') {
            piece[0] = *at;
            length = 1;
        } else if (at[1] == '%') {
            piece[0] = '%';
            length = 1;
            ++at;
        } else {
            const char *cursor = at + 1;
            unsigned width = 0;
            bool zero = false;
            if (*cursor == '0') {
                zero = true;
                ++cursor;
                if (*cursor < '1' || *cursor > '9') return false;
                width = (unsigned)(*cursor++ - '0');
            }
            if (*cursor != 'd' || ++conversions > 1) return false;
            if (number >= UINT64_C(1000000000)) return false;
            int written = zero
                ? snprintf(piece, sizeof(piece), "%0*llu", (int)width,
                           (unsigned long long)number)
                : snprintf(piece, sizeof(piece), "%llu",
                           (unsigned long long)number);
            if (written < 0) return false;
            length = (size_t)written;
            at = cursor;
        }
        if (!validate_only) {
            if (used + length >= size) return false;
            memcpy(out + used, piece, length);
        }
        used += length;
    }
    if (conversions != 1) return false;
    if (!validate_only) out[used] = '\0';
    return true;
}

bool sr_sequence_pattern_valid(const char *pattern) {
    return sequence_walk(pattern, 0, NULL, 0, true);
}

bool sr_sequence_format(const char *pattern, uint64_t number, char *out,
                        size_t size) {
    if (!out || !size) return false;
    return sequence_walk(pattern, number, out, size, false);
}

uint64_t sr_output_frame_at(double seconds, uint32_t fps_num, uint32_t fps_den) {
    if (!(seconds > 0.0) || !fps_num || !fps_den) return 0;
    return (uint64_t)ceil(seconds * fps_num / fps_den - 1e-12);
}
