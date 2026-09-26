/* SPDX-License-Identifier: Apache-2.0 */
#include "xml_internal.h"
#include "curves_internal.h"
#include "scene_render/color.h"

#include <math.h>
#include <stdlib.h>
#include <string.h>

static bool fail(ParseContext *ctx, const char *element, const char *attribute,
                  const char *message) {
    sr_xml_fail(ctx, element, attribute, message);
    return false;
}

bool sr_xml_animation_options(ParseContext *ctx, const XML_Char **attrs,
                               ParseFrame *host, SrAnimValue *value,
                               SrAnimColor *color) {
    SrTrack config = {0};
    const char *text = sr_xml_attr(attrs, "extrapolateBefore");
    if (text && !sr_extrapolation_parse(text, &config.extrapolate_before))
        return fail(ctx, "animate", "extrapolateBefore", "unknown extrapolation mode");
    text = sr_xml_attr(attrs, "extrapolateAfter");
    if (text && !sr_extrapolation_parse(text, &config.extrapolate_after))
        return fail(ctx, "animate", "extrapolateAfter", "unknown extrapolation mode");
    text = sr_xml_attr(attrs, "additive");
    if (text && !sr_parse_bool(text, &config.additive))
        return fail(ctx, "animate", "additive", "expected true or false");
    text = sr_xml_attr(attrs, "timeBase");
    if (text && strcmp(text, "composition")) {
        if (!strcmp(text, "local")) config.time_base = SR_TIME_LOCAL;
        else if (!strcmp(text, "normalized")) config.time_base = SR_TIME_NORMALIZED;
        else return fail(ctx, "animate", "timeBase", "unknown time base");
    }
    double parent_offset = 0.0;
    for (size_t i = 0; host->node && i < ctx->depth; ++i) {
        const ParseFrame *ancestor = &ctx->stack[i];
        if (ancestor->kind == E_GROUP && ancestor->node != host->node)
            parent_offset += ancestor->node->start_time;
    }
    config.domain_start = host->node ? parent_offset + host->node->start_time : 0.0;
    if (host->audio_track) config.domain_start = host->audio_track->start;
    config.domain_end = host->node && isfinite(host->node->end_time)
        ? parent_offset + host->node->end_time : ctx->scene->project.duration;
    config.seconds_per_unit = 1.0;
    if (config.time_base != SR_TIME_COMPOSITION) {
        config.clock_set = true;
        config.clock_scale = 1.0;
        config.clock_offset = -config.domain_start;
        if (config.time_base == SR_TIME_NORMALIZED) {
            double span = config.domain_end - config.domain_start;
            if (!(span >= SR_MIN_KEY_SPACING) || !isfinite(span))
                return fail(ctx, "animate", "timeBase",
                            "normalized animation requires a finite positive host span");
            config.clock_scale = 1.0 / span;
            config.clock_offset = -config.domain_start / span;
            config.seconds_per_unit = span;
        }
        if (fabs(config.clock_scale) > SR_MAX_ANIMATION_VALUE ||
            fabs(config.clock_offset) > SR_MAX_ANIMATION_VALUE)
            return fail(ctx, "animate", "timeBase", "resolved clock exceeds 1e12 limit");
    }
    if (value) value->track = config;
    else color->r = color->g = color->b = color->a = config;
    return true;
}

static bool handle(ParseContext *ctx, const char *text, const char *name, SrVec2 *value) {
    /* No allocation: two bounded decimal tokens separated by one comma. */
    char first[128], second[128];
    const char *comma = strchr(text, ',');
    if (!comma || comma == text || (size_t)(comma - text) >= sizeof(first) ||
        strlen(comma + 1) >= sizeof(second) || strchr(comma + 1, ','))
        return fail(ctx, "key", name, "expected normalized influence,speed in [0,1]");
    memcpy(first, text, (size_t)(comma - text));
    first[comma - text] = '\0';
    memcpy(second, comma + 1, strlen(comma + 1) + 1);
    if (!sr_parse_double(first, &value->x) || !sr_parse_double(second, &value->y) ||
        value->x < 0.0 || value->x > 1.0 || value->y < 0.0 || value->y > 1.0)
        return fail(ctx, "key", name, "expected normalized influence,speed in [0,1]");
    return true;
}

static bool parameter(ParseContext *ctx, const XML_Char **attrs, const char *name,
                       double *target, double low, double high, const char *message) {
    if (!sr_xml_parse_double_attr(ctx, "key", attrs, name, target)) return false;
    if (*target < low || *target > high) return fail(ctx, "key", name, message);
    return true;
}

bool sr_xml_key_options(ParseContext *ctx, const XML_Char **attrs, SrKeyframe *key) {
    key->source_line = sr_xml_line(ctx);
    key->stiffness = 100.0;
    key->damping = 10.0;
    key->mass = 1.0;
    const char *text = sr_xml_attr(attrs, "steps");
    if (text && (!sr_parse_u32(text, &key->steps) || !key->steps ||
                 key->steps > SR_MAX_CURVE_STEPS))
        return fail(ctx, "key", "steps", "steps must be in [1,1000000]");
    if (key->curve == SR_CURVE_STEPS && !text)
        return fail(ctx, "key", "steps", "steps interpolation requires a step count");
    if (text && key->curve != SR_CURVE_STEPS)
        return fail(ctx, "key", "steps", "steps is valid only with steps interpolation");
    text = sr_xml_attr(attrs, "stepPosition");
    if (text) {
        if (key->curve != SR_CURVE_STEPS)
            return fail(ctx, "key", "stepPosition", "stepPosition requires steps interpolation");
        if (!strcmp(text, "start")) key->step_start = true;
        else if (strcmp(text, "end"))
            return fail(ctx, "key", "stepPosition", "expected start or end");
    }
    if (!parameter(ctx, attrs, "tension", &key->tension, -1, 1, "expected [-1,1]") ||
        !parameter(ctx, attrs, "continuity", &key->continuity, -1, 1, "expected [-1,1]") ||
        !parameter(ctx, attrs, "bias", &key->bias, -1, 1, "expected [-1,1]") ||
        !parameter(ctx, attrs, "stiffness", &key->stiffness, SR_MIN_SPRING_PARAMETER,
                   SR_MAX_SPRING_PARAMETER, "spring stiffness must be in [1e-6,1e6]") ||
        !parameter(ctx, attrs, "damping", &key->damping, 0, SR_MAX_SPRING_PARAMETER,
                   "spring damping must be in [0,1e6]") ||
        !parameter(ctx, attrs, "mass", &key->mass, SR_MIN_SPRING_PARAMETER,
                   SR_MAX_SPRING_PARAMETER, "spring mass must be in [1e-6,1e6]"))
        return false;
    key->tcb_set = sr_xml_attr(attrs, "tension") || sr_xml_attr(attrs, "continuity") ||
        sr_xml_attr(attrs, "bias");
    key->spring_set = sr_xml_attr(attrs, "stiffness") || sr_xml_attr(attrs, "damping") ||
        sr_xml_attr(attrs, "mass");
    if (key->spring_set && key->curve != SR_CURVE_SPRING)
        return fail(ctx, "key", "stiffness/damping/mass",
                    "spring parameters require spring interpolation");
    text = sr_xml_attr(attrs, "easeIn");
    if (text) {
        if (!handle(ctx, text, "easeIn", &key->ease_in)) return false;
        key->ease_in_set = true;
    }
    text = sr_xml_attr(attrs, "easeOut");
    if (text) {
        if (!handle(ctx, text, "easeOut", &key->ease_out)) return false;
        key->ease_out_set = true;
    }
    return true;
}

bool sr_xml_finish_animation(ParseContext *ctx, ParseFrame *frame) {
    SrTrack *track = frame->anim ? &frame->anim->track : &frame->color_anim->r;
    if (!track->count)
        return fail(ctx, "animate", NULL, "animation track requires at least one key");
    SrStatus status = frame->anim ? sr_track_finalize(track)
                                  : sr_anim_color_finalize(frame->color_anim);
    for (size_t i = 0; i < track->count; ++i) {
        const SrKeyframe *key = &track->keys[i];
        const SrKeyframe *next = i + 1 < track->count ? key + 1 : NULL;
        const char *attribute = NULL, *message = NULL;
        if (!i && key->ease_in_set) {
            attribute = "easeIn";
            message = "easeIn requires a preceding segment";
        } else if (!next && key->ease_out_set) {
            attribute = "easeOut";
            message = "easeOut requires a following segment";
        } else if (i && key->time - track->keys[i - 1].time < SR_MIN_KEY_SPACING) {
            attribute = "time";
            message = "keyframe times must be unique (minimum spacing 1e-12)";
        } else if (sr_track_extended(track) && fabs(key->value) > SR_MAX_ANIMATION_VALUE) {
            attribute = "value";
            message = "extended animation value exceeds 1e12 limit";
        } else if (sr_track_extended(track) && fabs(key->time) > SR_MAX_ANIMATION_TIME) {
            attribute = "time";
            message = "extended animation time exceeds 1e6 limit";
        } else if (sr_track_extended(track) && key->curve == SR_CURVE_BEZIER &&
                   (!isfinite(key->y1) || !isfinite(key->y2) ||
                    fabs(key->y1) > SR_MAX_BEZIER_HANDLE ||
                    fabs(key->y2) > SR_MAX_BEZIER_HANDLE)) {
            attribute = "bezier";
            message = "extended Bezier ordinates must be finite and at most 1e6";
        } else if (next && (key->ease_out_set || next->ease_in_set) &&
                   (key->curve != SR_CURVE_BEZIER || key->bezier_set)) {
            attribute = "easeOut/easeIn";
            message = "temporal handles require cubic-bezier without explicit bezier";
        } else if (key->tcb_set && key->curve != SR_CURVE_TCB &&
                   !(i && track->keys[i - 1].curve == SR_CURVE_TCB)) {
            attribute = "tension/continuity/bias";
            message = "TCB parameters require an adjacent tcb segment";
        } else if (key->curve == SR_CURVE_BEZIER && !key->bezier_set &&
                   !key->ease_out_set && !(next && next->ease_in_set) &&
                   !(!next && sr_track_extended(track))) {
            attribute = "bezier";
            message = "expected bezier controls or temporal handles";
        }
        if (message) {
            sr_xml_fail_at(ctx, key->source_line, "key", attribute, message);
            return false;
        }
    }
    if (status != SR_OK)
        return fail(ctx, "animate", "interpolation", "invalid or unbounded curve parameters");
    if (frame->anim && sr_track_extended(track) &&
        fabs(frame->anim->base) > SR_MAX_ANIMATION_VALUE)
        return fail(ctx, "animate", "property", "extended animation base exceeds 1e12 limit");
    return true;
}
