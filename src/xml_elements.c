#include "xml_internal.h"
#include "scene_render/color.h"

#include <ctype.h>
#include <errno.h>
#include <limits.h>
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

static bool parse_int(const char *text, int *value) {
    if (!text || !value) return false;
    errno = 0;
    char *tail = NULL;
    long result = strtol(text, &tail, 10);
    if (errno || tail == text || *tail || result < INT_MIN || result > INT_MAX)
        return false;
    *value = (int)result;
    return true;
}

/* Parsed from a bounded stack copy: no allocation, so a failure always
 * means malformed text (the XSD limits fps to digits and one slash). */
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
    return ok && *num > 0 && *den > 0;
}

static bool parse_bezier(const char *text, SrKeyframe *key) {
    char tail;
    int count = sscanf(text, " %lf , %lf , %lf , %lf %c", &key->x1, &key->y1,
                       &key->x2, &key->y2, &tail);
    return count == 4 && key->x1 >= 0.0 && key->x1 <= 1.0 &&
           key->x2 >= 0.0 && key->x2 <= 1.0;
}

bool sr_xml_parse_double_attr(ParseContext *ctx, const char *element,
                              const XML_Char **attrs, const char *name,
                              double *target) {
    const char *value = sr_xml_attr(attrs, name);
    if (!value) return true;
    if (!sr_parse_double(value, target)) {
        sr_xml_fail(ctx, element, name, "expected a finite decimal number");
        return false;
    }
    return true;
}

bool sr_xml_parse_node_common(ParseContext *ctx, const char *element,
                              const XML_Char **attrs, SrNode *node) {
    const char *id = sr_xml_required(ctx, element, attrs, "id");
    if (!id || !set_string(&node->id, id)) {
        if (!ctx->failed) sr_xml_fail(ctx, element, "id", "out of memory");
        return false;
    }
    if (!sr_id_valid(id)) {
        sr_xml_fail(ctx, element, "id", "expected an XML-compatible identifier");
        return false;
    }
    node->source_line = sr_xml_line(ctx);
    const char *value;
    if ((value = sr_xml_attr(attrs, "z")) && !parse_int(value, &node->z)) {
        sr_xml_fail(ctx, element, "z", "expected a signed integer");
        return false;
    }
    if ((value = sr_xml_attr(attrs, "visible")) &&
        !sr_parse_bool(value, &node->visible)) {
        sr_xml_fail(ctx, element, "visible", "expected true or false");
        return false;
    }
    if (!sr_xml_parse_double_attr(ctx, element, attrs, "opacity", &node->opacity.base) ||
        !sr_xml_anim_length_attr(ctx, element, attrs, "x", &node->transform.x, false) ||
        !sr_xml_anim_length_attr(ctx, element, attrs, "y", &node->transform.y, false) ||
        !sr_xml_parse_double_attr(ctx, element, attrs, "rotation",
                           &node->transform.rotation.base) ||
        !sr_xml_parse_double_attr(ctx, element, attrs, "scaleX",
                           &node->transform.scale_x.base) ||
        !sr_xml_parse_double_attr(ctx, element, attrs, "scaleY",
                           &node->transform.scale_y.base) ||
        !sr_xml_anim_length_attr(ctx, element, attrs, "anchorX",
                                  &node->transform.anchor_x, false) ||
        !sr_xml_anim_length_attr(ctx, element, attrs, "anchorY",
                                  &node->transform.anchor_y, false) ||
        !sr_xml_parse_double_attr(ctx, element, attrs, "start", &node->start_time) ||
        !sr_xml_parse_double_attr(ctx, element, attrs, "end", &node->end_time) ||
        !sr_xml_parse_double_attr(ctx, element, attrs, "depth",
                           &node->transform.z.base) ||
        !sr_xml_parse_double_attr(ctx, element, attrs, "rotationX",
                           &node->transform.rotation_x.base) ||
        !sr_xml_parse_double_attr(ctx, element, attrs, "rotationY",
                           &node->transform.rotation_y.base)) return false;
    const char *three_d = sr_xml_attr(attrs, "threeD");
    if (three_d && !sr_parse_bool(three_d, &node->card)) {
        sr_xml_fail(ctx, element, "threeD", "expected true or false");
        return false;
    }
    if (sr_xml_attr(attrs, "depth") && sr_xml_attr(attrs, "zDepth")) {
        sr_xml_fail(ctx, element, "zDepth", "depth and zDepth are mutually exclusive");
        return false;
    }
    if (!sr_xml_parse_double_attr(ctx, element, attrs, "zDepth",
                                  &node->transform.z.base)) return false;
    bool depth_attributes = sr_xml_attr(attrs, "depth") ||
        sr_xml_attr(attrs, "zDepth") || sr_xml_attr(attrs, "rotationX") ||
        sr_xml_attr(attrs, "rotationY");
    if (depth_attributes && ctx->scene->format_version < 11) node->card = true;
    if (depth_attributes && !node->card)
        sr_diag_warning(ctx->diag, node->source_line, element, "threeD",
                        "depth rotations and zDepth require threeD=\"true\" in 1.1");
    if (node->opacity.base < 0.0 || node->opacity.base > 1.0) {
        sr_xml_fail(ctx, element, "opacity", "expected a number in [0,1]");
        return false;
    }
    if (node->start_time < 0.0 || node->end_time <= node->start_time) {
        sr_xml_fail(ctx, element, "start/end", "expected 0 <= start < end");
        return false;
    }
    return true;
}

void sr_xml_start_project(ParseContext *ctx, const XML_Char **attrs) {
    const char *const allowed[] = {"width", "height", "fps", "duration", "seed",
                                    "linearLight", "background", "mode",
                                    "workingColorSpace", "antialias3d"};
    if (!sr_xml_attrs_allowed(ctx, "project", attrs, allowed, 10)) return;
    const char *w = sr_xml_required(ctx, "project", attrs, "width");
    const char *h = sr_xml_required(ctx, "project", attrs, "height");
    const char *fps = sr_xml_required(ctx, "project", attrs, "fps");
    const char *duration = sr_xml_required(ctx, "project", attrs, "duration");
    if (ctx->failed) return;
    if (!sr_parse_u32(w, &ctx->scene->project.width) || !ctx->scene->project.width)
        SR_XML_FAIL_RETURN(ctx, "project", "width", "expected a positive integer");
    if (!sr_parse_u32(h, &ctx->scene->project.height) || !ctx->scene->project.height)
        SR_XML_FAIL_RETURN(ctx, "project", "height", "expected a positive integer");
    if (!parse_fps(fps, &ctx->scene->project.fps_num,
                   &ctx->scene->project.fps_den))
        SR_XML_FAIL_RETURN(ctx, "project", "fps",
                           "expected N or N/D with positive integers");
    if (!sr_parse_double(duration, &ctx->scene->project.duration) ||
        ctx->scene->project.duration <= 0.0 ||
        ctx->scene->project.duration > SR_MAX_DURATION)
        SR_XML_FAIL_RETURN(ctx, "project", "duration",
                           "expected a positive duration of at most 1e6 seconds");
    const char *value;
    if ((value = sr_xml_attr(attrs, "seed")) &&
        !sr_parse_u64(value, &ctx->scene->project.seed))
        SR_XML_FAIL_RETURN(ctx, "project", "seed",
                           "expected an unsigned 64-bit integer");
    if ((value = sr_xml_attr(attrs, "linearLight")) &&
        !sr_parse_bool(value, &ctx->scene->project.linear_light))
        SR_XML_FAIL_RETURN(ctx, "project", "linearLight", "expected true or false");
    if ((value = sr_xml_attr(attrs, "workingColorSpace")) &&
        !sr_color_space_parse(value, &ctx->scene->project.working_color_space))
        SR_XML_FAIL_RETURN(ctx, "project", "workingColorSpace",
                           "expected srgb, rec709, display-p3, or rec2020");
    if ((value = sr_xml_attr(attrs, "background")) &&
        !sr_xml_parse_color(ctx, "project", "background", value,
                            &ctx->scene->project.background))
        SR_XML_FAIL_RETURN(ctx, "project", "background",
                           "expected #RRGGBB, #RRGGBBAA, or r,g,b,a");
    if ((value = sr_xml_attr(attrs, "mode"))) {
        if (strcmp(value, "standard") == 0)
            ctx->scene->project.mode = SR_MODE_STANDARD;
        else if (strcmp(value, "equirectangular") == 0)
            ctx->scene->project.mode = SR_MODE_EQUIRECTANGULAR;
        else if (strcmp(value, "viewport") == 0)
            ctx->scene->project.mode = SR_MODE_VIEWPORT;
        else
            SR_XML_FAIL_RETURN(ctx, "project", "mode",
                               "expected standard, equirectangular, or viewport");
    }
    if ((value = sr_xml_attr(attrs, "antialias3d")) &&
        (!sr_parse_u32(value, &ctx->scene->project.antialias3d) ||
         ctx->scene->project.antialias3d < 1 || ctx->scene->project.antialias3d > 4))
        SR_XML_FAIL_RETURN(ctx, "project", "antialias3d", "expected 1, 2, 3, or 4");
    ctx->seen_project = true;
}

void sr_xml_start_output(ParseContext *ctx, const XML_Char **attrs) {
    const char *const allowed[] = {"path", "codec", "pixelFormat", "preset",
                                    "crf", "bitrate", "audioCodec",
                                    "audioBitrate", "colorSpace", "colorRange",
                                    "sphericalMetadata", "embedMetadata"};
    if (!sr_xml_attrs_allowed(ctx, "output", attrs, allowed, 12)) return;
    ctx->scene->output.source_line = sr_xml_line(ctx);
    const char *path = sr_xml_required(ctx, "output", attrs, "path");
    const char *codec = sr_xml_required(ctx, "output", attrs, "codec");
    if (ctx->failed) return;
    if (!set_string(&ctx->scene->output.path, path))
        SR_XML_FAIL_RETURN(ctx, "output", "path", "out of memory");
    if (strcmp(codec, "h264") == 0) ctx->scene->output.codec = SR_CODEC_H264;
    else if (strcmp(codec, "h265") == 0) ctx->scene->output.codec = SR_CODEC_H265;
    else if (strcmp(codec, "ffv1") == 0) ctx->scene->output.codec = SR_CODEC_FFV1;
    else SR_XML_FAIL_RETURN(ctx, "output", "codec", "expected h264, h265, or ffv1");
    const char *value;
    if ((value = sr_xml_attr(attrs, "pixelFormat")) &&
        !set_string(&ctx->scene->output.pixel_format, value))
        SR_XML_FAIL_RETURN(ctx, "output", "pixelFormat", "out of memory");
    if ((value = sr_xml_attr(attrs, "preset")) &&
        !set_string(&ctx->scene->output.preset, value))
        SR_XML_FAIL_RETURN(ctx, "output", "preset", "out of memory");
    if ((value = sr_xml_attr(attrs, "crf"))) {
        int crf;
        if (!parse_int(value, &crf) || crf < 0 || crf > 51)
            SR_XML_FAIL_RETURN(ctx, "output", "crf",
                               "expected an integer in [0,51]");
        ctx->scene->output.crf = crf;
    }
    if ((value = sr_xml_attr(attrs, "bitrate")) &&
        (!sr_parse_u64(value, &ctx->scene->output.bitrate) ||
         ctx->scene->output.bitrate == 0))
        SR_XML_FAIL_RETURN(ctx, "output", "bitrate",
                           "expected a positive integer in bits/second");
    if ((value = sr_xml_attr(attrs, "audioCodec")) &&
        !set_string(&ctx->scene->output.audio_codec, value))
        SR_XML_FAIL_RETURN(ctx, "output", "audioCodec", "out of memory");
    if ((value = sr_xml_attr(attrs, "audioBitrate")) &&
        (!sr_parse_u64(value, &ctx->scene->output.audio_bitrate) ||
         !ctx->scene->output.audio_bitrate))
        SR_XML_FAIL_RETURN(ctx, "output", "audioBitrate",
                           "expected a positive integer in bits/second");
    if ((value = sr_xml_attr(attrs, "colorSpace")) &&
        !sr_color_space_parse(value, &ctx->scene->output.color_space))
        SR_XML_FAIL_RETURN(ctx, "output", "colorSpace",
                           "expected srgb, rec709, display-p3, or rec2020");
    if ((value = sr_xml_attr(attrs, "colorRange"))) {
        if (!strcmp(value, "full")) ctx->scene->output.full_range = true;
        else if (!strcmp(value, "limited")) ctx->scene->output.full_range = false;
        else SR_XML_FAIL_RETURN(ctx, "output", "colorRange",
                                "expected limited or full");
    }
    if ((value = sr_xml_attr(attrs, "sphericalMetadata")) &&
        !sr_parse_bool(value, &ctx->scene->output.spherical_metadata))
        SR_XML_FAIL_RETURN(ctx, "output", "sphericalMetadata",
                           "expected true or false");
    if ((value = sr_xml_attr(attrs, "embedMetadata")) &&
        !sr_parse_bool(value, &ctx->scene->output.embed_metadata))
        SR_XML_FAIL_RETURN(ctx, "output", "embedMetadata",
                           "expected true or false");
    ctx->seen_output = true;
}

void sr_xml_start_image(ParseContext *ctx, const XML_Char **attrs) {
    const char *const allowed[] = {"id", "src", "width", "height",
                                    "colorSpace"};
    if (!sr_xml_attrs_allowed(ctx, "image", attrs, allowed, 5)) return;
    const char *id = sr_xml_required(ctx, "image", attrs, "id");
    const char *src = sr_xml_required(ctx, "image", attrs, "src");
    const char *w = sr_xml_required(ctx, "image", attrs, "width");
    const char *h = sr_xml_required(ctx, "image", attrs, "height");
    if (ctx->failed) return;
    if (!sr_id_valid(id))
        SR_XML_FAIL_RETURN(ctx, "image", "id",
                           "expected an XML-compatible identifier");
    if (sr_scene_id_exists(ctx->scene, id))
        SR_XML_FAIL_RETURN(ctx, "image", "id", "asset id must be unique");
    SrAsset *asset = sr_scene_add_asset(ctx->scene);
    if (!asset) SR_XML_FAIL_RETURN(ctx, "image", NULL, "out of memory");
    asset->source_line = sr_xml_line(ctx);
    asset->type = SR_ASSET_IMAGE;
    asset->id = sr_strdup(id);
    asset->source = sr_strdup(src);
    if (!asset->id || !asset->source)
        SR_XML_FAIL_RETURN(ctx, "image", NULL, "out of memory");
    if (!sr_parse_u32(w, &asset->width) || !asset->width)
        SR_XML_FAIL_RETURN(ctx, "image", "width", "expected a positive integer");
    if (!sr_parse_u32(h, &asset->height) || !asset->height)
        SR_XML_FAIL_RETURN(ctx, "image", "height", "expected a positive integer");
    const char *color_space = sr_xml_attr(attrs, "colorSpace");
    if (color_space && !sr_color_space_parse(color_space,
                                              &asset->source_color_space))
        SR_XML_FAIL_RETURN(ctx, "image", "colorSpace",
                           "expected srgb, rec709, display-p3, or rec2020");
}

void sr_xml_start_video(ParseContext *ctx, const XML_Char **attrs) {
    const char *const allowed[] = {"id", "src", "width", "height", "fps",
                                    "duration", "colorSpace"};
    if (!sr_xml_attrs_allowed(ctx, "video", attrs, allowed, 7)) return;
    const char *id = sr_xml_required(ctx, "video", attrs, "id");
    const char *src = sr_xml_required(ctx, "video", attrs, "src");
    const char *w = sr_xml_required(ctx, "video", attrs, "width");
    const char *h = sr_xml_required(ctx, "video", attrs, "height");
    const char *fps = sr_xml_required(ctx, "video", attrs, "fps");
    const char *duration = sr_xml_required(ctx, "video", attrs, "duration");
    if (ctx->failed) return;
    if (!sr_id_valid(id) || sr_scene_id_exists(ctx->scene, id))
        SR_XML_FAIL_RETURN(ctx, "video", "id", "expected a unique XML identifier");
    SrAsset *asset = sr_scene_add_asset(ctx->scene);
    if (!asset) SR_XML_FAIL_RETURN(ctx, "video", NULL, "out of memory");
    asset->type = SR_ASSET_VIDEO;
    asset->source_line = sr_xml_line(ctx);
    asset->id = sr_strdup(id);
    asset->source = sr_strdup(src);
    if (!asset->id || !asset->source)
        SR_XML_FAIL_RETURN(ctx, "video", NULL, "out of memory");
    if (!sr_parse_u32(w, &asset->width) || !asset->width)
        SR_XML_FAIL_RETURN(ctx, "video", "width", "expected a positive integer");
    if (!sr_parse_u32(h, &asset->height) || !asset->height)
        SR_XML_FAIL_RETURN(ctx, "video", "height", "expected a positive integer");
    if (!parse_fps(fps, &asset->fps_num, &asset->fps_den))
        SR_XML_FAIL_RETURN(ctx, "video", "fps", "expected positive N or N/D");
    if (!sr_parse_double(duration, &asset->duration) || asset->duration <= 0.0)
        SR_XML_FAIL_RETURN(ctx, "video", "duration", "expected a positive duration");
    const char *color_space = sr_xml_attr(attrs, "colorSpace");
    if (color_space && !sr_color_space_parse(color_space,
                                              &asset->source_color_space))
        SR_XML_FAIL_RETURN(ctx, "video", "colorSpace",
                           "expected srgb, rec709, display-p3, or rec2020");
}

void sr_xml_start_audio(ParseContext *ctx, const XML_Char **attrs) {
    const char *const allowed[] = {"id", "src"};
    if (!sr_xml_attrs_allowed(ctx, "audio", attrs, allowed, 2)) return;
    const char *id = sr_xml_required(ctx, "audio", attrs, "id");
    const char *src = sr_xml_required(ctx, "audio", attrs, "src");
    if (ctx->failed) return;
    if (!sr_id_valid(id) || sr_scene_id_exists(ctx->scene, id))
        SR_XML_FAIL_RETURN(ctx, "audio", "id", "expected a unique XML identifier");
    SrAsset *asset = sr_scene_add_asset(ctx->scene);
    if (!asset) SR_XML_FAIL_RETURN(ctx, "audio", NULL, "out of memory");
    asset->type = SR_ASSET_AUDIO;
    asset->source_line = sr_xml_line(ctx);
    asset->id = sr_strdup(id);
    asset->source = sr_strdup(src);
    if (!asset->id || !asset->source)
        SR_XML_FAIL_RETURN(ctx, "audio", NULL, "out of memory");
}

void sr_xml_start_mask(ParseContext *ctx, const XML_Char **attrs) {
    const char *const allowed[] = {"type", "x", "y", "width", "height",
                                    "radius", "invert"};
    if (!sr_xml_attrs_allowed(ctx, "mask", attrs, allowed, 7)) return;
    ParseFrame *p = sr_xml_parent(ctx);
    const char *type = sr_xml_required(ctx, "mask", attrs, "type");
    if (!p || !p->node || !type) return;
    SrMask mask = {.source_line = sr_xml_line(ctx)};
    if (!strcmp(type,"rect")) mask.type=SR_MASK_RECT;
    else if (!strcmp(type,"ellipse")) mask.type=SR_MASK_ELLIPSE;
    else if (!strcmp(type,"rounded-rect")) mask.type=SR_MASK_ROUNDED_RECT;
    else SR_XML_FAIL_RETURN(ctx,"mask","type",
                            "expected rect, ellipse, or rounded-rect");
    if (!sr_xml_anim_length_attr(ctx, "mask", attrs, "x", &mask.x, false) ||
        !sr_xml_anim_length_attr(ctx, "mask", attrs, "y", &mask.y, false) ||
        !sr_xml_anim_length_attr(ctx, "mask", attrs, "width", &mask.width, true) ||
        !sr_xml_anim_length_attr(ctx, "mask", attrs, "height", &mask.height, true) ||
        !sr_xml_parse_double_attr(ctx, "mask", attrs, "radius", &mask.radius.base)) return;
    if (mask.width.base <= 0.0 || mask.height.base <= 0.0)
        SR_XML_FAIL_RETURN(ctx, "mask", "width/height", "expected positive dimensions");
    if (mask.radius.base < 0.0)
        SR_XML_FAIL_RETURN(ctx,"mask","radius","expected a non-negative radius");
    const char *invert = sr_xml_attr(attrs, "invert");
    if (invert && !sr_parse_bool(invert, &mask.invert))
        SR_XML_FAIL_RETURN(ctx, "mask", "invert", "expected true or false");
    /* Every mask is kept in document order; coverage multiplies. */
    if (sr_node_add_mask(p->node, mask) != SR_OK)
        SR_XML_FAIL_RETURN(ctx, "mask", NULL, "out of memory");
    /* Valid until the next sibling mask is added, i.e. for this element's
     * nested <animate> children. */
    SrMask *stored = &p->node->masks[p->node->mask_count - 1];
    sr_xml_push(ctx, (ParseFrame){.kind = E_MASK, .node = p->node,
                                  .mask = stored,
                                  .curve = SR_CURVE_LINEAR}, "mask");
}

static const SrProperty *animate_target(ParseFrame *frame, const char *name,
                                         void **object) {
    SrPropertyHost host;
    switch (frame->kind) {
    case E_GROUP:
    case E_LAYER:
    case E_PARTICLES:
        host = sr_property_node_host(frame->node);
        *object = frame->node;
        break;
    case E_MASK: host = SR_PROPERTY_MASK; *object = frame->mask; break;
    case E_POINT: host = SR_PROPERTY_POINT; *object = frame->point; break;
    case E_CAMERA: host = SR_PROPERTY_CAMERA; *object = frame->camera; break;
    case E_LIGHT: host = SR_PROPERTY_LIGHT; *object = frame->light; break;
    case E_EFFECT: host = SR_PROPERTY_EFFECT; *object = frame->effect; break;
    case E_FORCE_FIELD: host = SR_PROPERTY_FIELD; *object = frame->field; break;
    case E_MODIFIER: host = SR_PROPERTY_MODIFIER; *object = frame->modifier; break;
    case E_OBJECT3D: host = SR_PROPERTY_OBJECT3D; *object = frame->object3d; break;
    case E_MATERIAL: host = SR_PROPERTY_MATERIAL; *object = frame->material; break;
    case E_AUDIO_TRACK: host = SR_PROPERTY_AUDIO_TRACK; *object = frame->audio_track; break;
    default: return NULL;
    }
    const SrProperty *property = sr_property_find(host, name);
    /* Legacy modifier animations may address their owning node's scalar
     * properties. Keep that accepted vocabulary in this structural change. */
    if (!property && frame->kind == E_MODIFIER && frame->node) {
        property = sr_property_find(sr_property_node_host(frame->node), name);
        if (property && property->type != SR_PROPERTY_NUMBER) return NULL;
        *object = frame->node;
    }
    return property;
}

void sr_xml_start_animate(ParseContext *ctx, const XML_Char **attrs) {
    const char *const allowed[] = {"property", "defaultInterpolation",
        "extrapolateBefore", "extrapolateAfter", "additive", "timeBase"};
    if (!sr_xml_attrs_allowed(ctx, "animate", attrs, allowed,
                              sizeof(allowed) / sizeof(allowed[0]))) return;
    ParseFrame *p = sr_xml_parent(ctx);
    const char *property = sr_xml_required(ctx, "animate", attrs, "property");
    if (!p || !property) return;
    void *object = NULL;
    const SrProperty *entry = animate_target(p, property, &object);
    void *target = sr_property_target(entry, object);
    SrAnimValue *anim = entry && entry->type == SR_PROPERTY_NUMBER ? target : NULL;
    SrAnimColor *color = entry && entry->type == SR_PROPERTY_COLOR ? target : NULL;
    sr_property_activate(entry, object);
    if ((p->kind == E_GROUP || p->kind == E_LAYER || p->kind == E_PARTICLES) &&
        p->node && anim && (entry->flags & SR_PROPERTY_DEPTH_CARD)) {
        if (ctx->scene->format_version < 11) p->node->card = true;
        else if (!p->node->card)
            sr_diag_warning(ctx->diag, sr_xml_line(ctx), "animate", "property",
                            "depth animation requires threeD=\"true\" in 1.1");
    }
    if (!anim && !color)
        SR_XML_FAIL_RETURN(ctx, "animate", "property",
                           "property is not animatable for this element");
    if ((anim && anim->track.count) || (color && color->r.count))
        SR_XML_FAIL_RETURN(ctx, "animate", "property",
                           "property already has an animation track");
    if (!sr_xml_animation_options(ctx, attrs, p, anim, color)) return;
    if (color) color->space = ctx->scene->project.working_color_space;
    SrCurve curve = SR_CURVE_LINEAR;
    const char *interpolation = sr_xml_attr(attrs, "defaultInterpolation");
    if (interpolation && !sr_curve_parse(interpolation, &curve))
        SR_XML_FAIL_RETURN(ctx, "animate", "defaultInterpolation",
                           "unsupported interpolation curve");
    sr_xml_push(ctx, (ParseFrame){.kind = E_ANIMATE, .node = p->node,
        .anim = anim, .color_anim = color, .property = entry, .camera = p->camera,
        .light = p->light, .effect = p->effect, .modifier = p->modifier,
        .object3d = p->object3d, .mask = p->mask, .field = p->field,
        .curve = curve}, "animate");
}

void sr_xml_start_key(ParseContext *ctx, const XML_Char **attrs) {
    const char *const allowed[] = {"time", "value", "interpolation", "bezier",
        "easeIn", "easeOut", "steps", "stepPosition", "tension", "continuity",
        "bias", "stiffness", "damping", "mass"};
    if (!sr_xml_attrs_allowed(ctx, "key", attrs, allowed,
                              sizeof(allowed) / sizeof(allowed[0]))) return;
    ParseFrame *p = sr_xml_parent(ctx);
    const char *time_text = sr_xml_required(ctx, "key", attrs, "time");
    const char *value_text = sr_xml_required(ctx, "key", attrs, "value");
    if (!p || p->kind != E_ANIMATE || !time_text || !value_text) return;
    SrKeyframe key = {.curve = p->curve, .x1 = 0.25, .y1 = 0.1,
                      .x2 = 0.25, .y2 = 1.0};
    if (!sr_parse_double(time_text, &key.time) || key.time < 0.0)
        SR_XML_FAIL_RETURN(ctx, "key", "time",
                           "expected a non-negative time in seconds");
    SrColor color_value = {0, 0, 0, 0};
    if (p->color_anim) {
        if (!sr_xml_parse_color(ctx, "key", "value", value_text, &color_value))
            SR_XML_FAIL_RETURN(ctx, "key", "value",
                               "expected a color (#RRGGBB, #RRGGBBAA, or r,g,b[,a])");
    } else if (p->property->flags & (SR_PROPERTY_LENGTH_X | SR_PROPERTY_LENGTH_Y)) {
        SrLength length;
        if (!sr_parse_length(value_text, &length))
            SR_XML_FAIL_RETURN(ctx, "key", "value",
                               "expected a finite length; relative spelling limit "
                               "128 bytes, coefficient limit 1e6");
        key.value = length.value;
        key.unit = length.unit;
        if (key.unit != SR_LENGTH_PIXELS) ctx->scene->has_relative_lengths = true;
    } else if (!sr_parse_double(value_text, &key.value)) {
        SR_XML_FAIL_RETURN(ctx, "key", "value", "expected a finite decimal number");
    }
    if (p->anim && !sr_property_key_valid(p->property, key.value))
        SR_XML_FAIL_RETURN(ctx, "key", "value", p->property->bounds_error);
    const char *curve = sr_xml_attr(attrs, "interpolation");
    if (curve && !sr_curve_parse(curve, &key.curve))
        SR_XML_FAIL_RETURN(ctx, "key", "interpolation",
                           "unsupported interpolation curve");
    if (!sr_xml_key_options(ctx, attrs, &key)) return;
    const char *bezier = sr_xml_attr(attrs, "bezier");
    key.bezier_set = bezier != NULL;
    if (key.curve == SR_CURVE_BEZIER) {
        if (bezier && !parse_bezier(bezier, &key))
            SR_XML_FAIL_RETURN(ctx, "key", "bezier",
                               "expected x1,y1,x2,y2 with x values in [0,1]");
    } else if (bezier) {
        SR_XML_FAIL_RETURN(ctx, "key", "bezier",
                           "bezier is valid only with cubic-bezier");
    }
    SrTrack *track = p->color_anim ? &p->color_anim->r : &p->anim->track;
    if (track->count >= SR_MAX_TRACK_KEYS)
        SR_XML_FAIL_RETURN(ctx, "key", NULL, "track key limit is 65536");
    if (ctx->key_count >= SR_MAX_SCENE_KEYS)
        SR_XML_FAIL_RETURN(ctx, "key", NULL, "scene key limit is 1048576");
    ++ctx->key_count;
    SrStatus added = p->color_anim
        ? sr_anim_color_add_key(p->color_anim, key, color_value)
        : sr_track_add(&p->anim->track, key);
    if (added != SR_OK)
        SR_XML_FAIL_RETURN(ctx, "key", NULL, "out of memory");
    sr_xml_push(ctx, (ParseFrame){.kind = E_KEY, .node = p->node,
                                  .anim = p->anim, .curve = key.curve}, "key");
}
