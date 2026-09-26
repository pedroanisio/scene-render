/* SPDX-License-Identifier: Apache-2.0 */
/* <output>, <poster> and <thumbnail> (B1-6). The 1.0 attributes keep their
 * 1.0 parsing and messages; cross-field checks run in sr_outputs_resolve. */
#include "xml_internal.h"
#include "scene_render/color.h"
#include "scene_render/outputs.h"

#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static bool set_string(char **target, const char *value) {
    char *copy = sr_strdup(value);
    if (!copy) return false;
    free(*target);
    *target = copy;
    return true;
}

/* Bounded stack copy, as the project's fps parser. */
static bool parse_fps(const char *text, uint32_t *num, uint32_t *den) {
    char copy[64];
    size_t length = strlen(text);
    if (length >= sizeof(copy)) return false;
    memcpy(copy, text, length + 1);
    char *slash = strchr(copy, '/');
    bool ok;
    if (slash) {
        *slash++ = '\0';
        ok = sr_parse_u32(copy, num) && sr_parse_u32(slash, den);
    } else {
        ok = sr_parse_u32(copy, num);
        *den = 1;
    }
    return ok && *num > 0 && *den > 0 && *num <= INT32_MAX && *den <= INT32_MAX;
}

static bool parse_dimension(const char *text, uint32_t *value) {
    return sr_parse_u32(text, value) && *value > 0 &&
           *value <= SR_MAX_OUTPUT_DIMENSION;
}

static bool parse_seconds(const char *text, double *value) {
    return sr_parse_double(text, value) && isfinite(*value) && *value >= 0.0 &&
           *value <= SR_MAX_DURATION;
}

/* The 1.0 attributes, parsed exactly as in 1.0. */
static void parse_legacy(ParseContext *ctx, SrOutput *output,
                         const XML_Char **attrs) {
    const char *path = sr_xml_required(ctx, "output", attrs, "path");
    const char *codec = sr_xml_required(ctx, "output", attrs, "codec");
    if (ctx->failed) return;
    if (!set_string(&output->path, path))
        SR_XML_FAIL_RETURN(ctx, "output", "path", "out of memory");
    if (!sr_codec_parse(codec, &output->codec))
        SR_XML_FAIL_RETURN(ctx, "output", "codec",
                           "expected h264, h265, ffv1, prores, vp9, av1, gif, "
                           "apng, png-sequence, tiff-sequence or exr-sequence");
    const SrCodecInfo *info = sr_codec_info(output->codec);
    const char *value;
    if ((value = sr_xml_attr(attrs, "pixelFormat"))) {
        if (!set_string(&output->pixel_format, value))
            SR_XML_FAIL_RETURN(ctx, "output", "pixelFormat", "out of memory");
        output->pixel_format_authored = true;
    }
    if ((value = sr_xml_attr(attrs, "preset"))) {
        if (!set_string(&output->preset, value))
            SR_XML_FAIL_RETURN(ctx, "output", "preset", "out of memory");
        output->preset_authored = true;
    }
    if ((value = sr_xml_attr(attrs, "crf"))) {
        uint32_t crf;
        int limit = info->max_crf > 51 ? info->max_crf : 51;
        if (!sr_parse_u32(value, &crf) || crf > (uint32_t)limit) {
            char message[64];
            snprintf(message, sizeof(message), "expected an integer in [0,%d]",
                     limit);
            SR_XML_FAIL_RETURN(ctx, "output", "crf", message);
        }
        output->crf = (int)crf;
        output->crf_authored = true;
    }
    if ((value = sr_xml_attr(attrs, "bitrate"))) {
        if (!sr_parse_u64(value, &output->bitrate) || output->bitrate == 0 ||
            output->bitrate > INT64_MAX)
            SR_XML_FAIL_RETURN(ctx, "output", "bitrate",
                               "expected a positive integer in bits/second");
        output->bitrate_authored = true;
    }
    if ((value = sr_xml_attr(attrs, "audioCodec"))) {
        if (!set_string(&output->audio_codec, value))
            SR_XML_FAIL_RETURN(ctx, "output", "audioCodec", "out of memory");
        output->audio_codec_authored = true;
    }
    if ((value = sr_xml_attr(attrs, "audioBitrate")) &&
        (!sr_parse_u64(value, &output->audio_bitrate) || !output->audio_bitrate))
        SR_XML_FAIL_RETURN(ctx, "output", "audioBitrate",
                           "expected a positive integer in bits/second");
    if ((value = sr_xml_attr(attrs, "colorSpace")) &&
        !sr_color_space_parse(value, &output->color_space))
        SR_XML_FAIL_RETURN(ctx, "output", "colorSpace",
                           "expected srgb, rec709, display-p3, or rec2020");
    if ((value = sr_xml_attr(attrs, "colorRange"))) {
        if (!strcmp(value, "full")) output->full_range = true;
        else if (!strcmp(value, "limited")) output->full_range = false;
        else SR_XML_FAIL_RETURN(ctx, "output", "colorRange",
                                "expected limited or full");
    }
    if ((value = sr_xml_attr(attrs, "sphericalMetadata")) &&
        !sr_parse_bool(value, &output->spherical_metadata))
        SR_XML_FAIL_RETURN(ctx, "output", "sphericalMetadata",
                           "expected true or false");
    if ((value = sr_xml_attr(attrs, "embedMetadata")) &&
        !sr_parse_bool(value, &output->embed_metadata))
        SR_XML_FAIL_RETURN(ctx, "output", "embedMetadata",
                           "expected true or false");
}

static bool parse_prores(const char *text, SrProresProfile *profile) {
    static const char *const names[] = {NULL, "proxy", "lt", "422", "hq",
                                        "4444", "4444xq"};
    for (size_t i = 1; i < sizeof(names) / sizeof(names[0]); ++i) {
        if (!strcmp(text, names[i])) {
            *profile = (SrProresProfile)i;
            return true;
        }
    }
    return false;
}

static void parse_extended(ParseContext *ctx, SrOutput *output,
                           const XML_Char **attrs) {
    const char *value;
    if ((value = sr_xml_attr(attrs, "id"))) {
        if (!sr_id_valid(value))
            SR_XML_FAIL_RETURN(ctx, "output", "id",
                               "expected an XML-compatible identifier");
        if (!set_string(&output->id, value))
            SR_XML_FAIL_RETURN(ctx, "output", "id", "out of memory");
    }
    if ((value = sr_xml_attr(attrs, "container")) &&
        !sr_container_parse(value, &output->container))
        SR_XML_FAIL_RETURN(ctx, "output", "container",
                           "unsupported in this build: expected mp4, mov, mkv "
                           "or webm");
    if ((value = sr_xml_attr(attrs, "width")) &&
        !parse_dimension(value, &output->width))
        SR_XML_FAIL_RETURN(ctx, "output", "width",
                           "expected a positive integer of at most 16384");
    if ((value = sr_xml_attr(attrs, "height")) &&
        !parse_dimension(value, &output->height))
        SR_XML_FAIL_RETURN(ctx, "output", "height",
                           "expected a positive integer of at most 16384");
    if ((value = sr_xml_attr(attrs, "fps")) &&
        !parse_fps(value, &output->fps_num, &output->fps_den))
        SR_XML_FAIL_RETURN(ctx, "output", "fps",
                           "expected N or N/D with positive integers");
    if ((value = sr_xml_attr(attrs, "start")) &&
        !parse_seconds(value, &output->start))
        SR_XML_FAIL_RETURN(ctx, "output", "start",
                           "expected a non-negative number of seconds");
    if ((value = sr_xml_attr(attrs, "end"))) {
        if (!parse_seconds(value, &output->end))
            SR_XML_FAIL_RETURN(ctx, "output", "end",
                               "expected a non-negative number of seconds");
        output->has_end = true;
    }
    if ((value = sr_xml_attr(attrs, "keyframeInterval")) &&
        (!parse_seconds(value, &output->keyframe_interval) ||
         !(output->keyframe_interval > 0.0)))
        SR_XML_FAIL_RETURN(ctx, "output", "keyframeInterval",
                           "expected a positive number of seconds");
    if ((value = sr_xml_attr(attrs, "bFrames"))) {
        uint32_t frames;
        if (!sr_parse_u32(value, &frames) || frames > 16)
            SR_XML_FAIL_RETURN(ctx, "output", "bFrames",
                               "expected an integer in [0,16]");
        output->b_frames = (int)frames;
    }
    if ((value = sr_xml_attr(attrs, "faststart"))) {
        if (!sr_parse_bool(value, &output->faststart))
            SR_XML_FAIL_RETURN(ctx, "output", "faststart", "expected true or false");
        output->faststart_authored = true;
    }
    if ((value = sr_xml_attr(attrs, "loopCount"))) {
        if (!sr_parse_u32(value, &output->loop_count) || output->loop_count > 65535)
            SR_XML_FAIL_RETURN(ctx, "output", "loopCount",
                               "expected an integer in [0,65535]");
        output->loop_count_authored = true;
    }
    if ((value = sr_xml_attr(attrs, "proresProfile")) &&
        !parse_prores(value, &output->prores_profile))
        SR_XML_FAIL_RETURN(ctx, "output", "proresProfile",
                           "expected proxy, lt, 422, hq, 4444 or 4444xq");
}

void sr_xml_start_output(ParseContext *ctx, const XML_Char **attrs) {
    const char *const allowed[] = {
        "path", "codec", "pixelFormat", "preset", "crf", "bitrate",
        "audioCodec", "audioBitrate", "colorSpace", "colorRange",
        "sphericalMetadata", "embedMetadata", "id", "container", "width",
        "height", "fps", "start", "end", "keyframeInterval", "bFrames",
        "faststart", "loopCount", "proresProfile"};
    if (!sr_xml_attrs_allowed(ctx, "output", attrs, allowed,
                              sizeof(allowed) / sizeof(allowed[0])))
        return;
    SrScene *scene = ctx->scene;
    SrOutput *output = &scene->output;
    if (ctx->seen_output) {
        if (scene->format_version < 11)
            SR_XML_FAIL_RETURN(ctx, "output", NULL,
                               "more than one output requires version=\"1.1\"");
        if (sr_scene_output_count(scene) >= SR_MAX_OUTPUTS) {
            char message[96];
            snprintf(message, sizeof(message),
                     "more than %u outputs (SR_MAX_OUTPUTS)", SR_MAX_OUTPUTS);
            SR_XML_FAIL_RETURN(ctx, "output", NULL, message);
        }
        output = sr_scene_add_extra_output(scene);
        if (!output) {
            ctx->out_of_memory = true;
            SR_XML_FAIL_RETURN(ctx, "output", NULL, "out of memory");
        }
    }
    output->source_line = sr_xml_line(ctx);
    ctx->seen_output = true;
    parse_legacy(ctx, output, attrs);
    if (!ctx->failed) parse_extended(ctx, output, attrs);
    if (ctx->failed) return;
    sr_xml_push(ctx, (ParseFrame){.kind = E_OUTPUT, .output = output}, "output");
}

void sr_xml_start_still(ParseContext *ctx, const XML_Char **attrs,
                        SrStillKind kind) {
    const char *element = kind == SR_STILL_POSTER ? "poster" : "thumbnail";
    const char *const allowed[] = {"time", "path", "format", "width", "quality"};
    if (!sr_xml_attrs_allowed(ctx, element, attrs, allowed, 5)) return;
    ParseFrame *parent = sr_xml_parent(ctx);
    SrOutput *output = parent ? parent->output : NULL;
    if (!output) SR_XML_FAIL_RETURN(ctx, element, NULL, "must be inside <output>");
    if (output->still_count >= SR_MAX_OUTPUT_STILLS) {
        char message[96];
        snprintf(message, sizeof(message),
                 "more than %u stills in one output (SR_MAX_OUTPUT_STILLS)",
                 SR_MAX_OUTPUT_STILLS);
        SR_XML_FAIL_RETURN(ctx, element, NULL, message);
    }
    const char *path = sr_xml_required(ctx, element, attrs, "path");
    if (ctx->failed) return;
    SrStill still = {.kind = kind, .format = SR_STILL_JPEG, .quality = 0.9,
                     .source_line = sr_xml_line(ctx)};
    const char *value;
    if ((value = sr_xml_attr(attrs, "time")) && !parse_seconds(value, &still.time))
        SR_XML_FAIL_RETURN(ctx, element, "time",
                           "expected a non-negative number of seconds");
    if ((value = sr_xml_attr(attrs, "format"))) {
        if (!strcmp(value, "png")) still.format = SR_STILL_PNG;
        else if (!strcmp(value, "jpeg")) still.format = SR_STILL_JPEG;
        else SR_XML_FAIL_RETURN(ctx, element, "format",
                                "unsupported in this build: expected jpeg or png");
    }
    if ((value = sr_xml_attr(attrs, "width")) && !parse_dimension(value, &still.width))
        SR_XML_FAIL_RETURN(ctx, element, "width",
                           "expected a positive integer of at most 16384");
    if ((value = sr_xml_attr(attrs, "quality")) &&
        (!sr_parse_double(value, &still.quality) || !(still.quality >= 0.0) ||
         still.quality > 1.0))
        SR_XML_FAIL_RETURN(ctx, element, "quality", "expected a number in [0,1]");
    /* Grow by one: at most SR_MAX_OUTPUT_STILLS entries. */
    SrStill *grown = sr_realloc(output->stills,
                                (output->still_count + 1) * sizeof(*grown));
    if (!grown) {
        ctx->out_of_memory = true;
        SR_XML_FAIL_RETURN(ctx, element, NULL, "out of memory");
    }
    output->stills = grown;
    still.path = sr_strdup(path);
    if (!still.path) {
        ctx->out_of_memory = true;
        SR_XML_FAIL_RETURN(ctx, element, "path", "out of memory");
    }
    output->stills[output->still_count++] = still;
}
