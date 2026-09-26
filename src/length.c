/* SPDX-License-Identifier: Apache-2.0 */
#include "scene_render/length.h"
#include "length_internal.h"
#include "timeline_internal.h"

#include <math.h>
#include <string.h>

static bool digit(char value) {
    return value >= '0' && value <= '9';
}

bool sr_parse_length(const char *text, SrLength *length) {
    if (!text || !length) return false;
    double value;
    if (sr_parse_double(text, &value)) {
        *length = (SrLength){value, SR_LENGTH_PIXELS};
        return true;
    }
    size_t size = 0;
    while (size <= SR_MAX_LENGTH_BYTES && text[size]) ++size;
    if (!size || size > SR_MAX_LENGTH_BYTES) return false;
    size_t end = text[0] == '-' ? 1 : 0;
    bool digits = false;
    while (digit(text[end])) {
        digits = true;
        ++end;
    }
    if (text[end] == '.') {
        ++end;
        while (digit(text[end])) {
            digits = true;
            ++end;
        }
    }
    if (!digits) return false;
    static const char *const units[] = {"%", "vw", "vh", "vmin", "vmax"};
    SrLengthUnit unit = SR_LENGTH_PIXELS;
    for (size_t i = 0; i < sizeof(units) / sizeof(units[0]); ++i) {
        if (!strcmp(text + end, units[i])) {
            unit = (SrLengthUnit)(i + 1);
            break;
        }
    }
    if (unit == SR_LENGTH_PIXELS) return false;
    char number[SR_MAX_LENGTH_BYTES + 1];
    memcpy(number, text, end);
    number[end] = '\0';
    if (!sr_parse_double(number, &value) || fabs(value) > SR_MAX_RELATIVE_LENGTH)
        return false;
    *length = (SrLength){value, unit};
    return true;
}

SrStatus sr_length_resolve(SrLength length, double parent_axis,
                           SrLengthBox frame, bool positive, double *pixels) {
    if (!pixels) return SR_ERR_ARGUMENT;
    if (!isfinite(length.value) || length.unit < SR_LENGTH_PIXELS ||
        length.unit > SR_LENGTH_VMAX) return SR_ERR_RENDER;
    double value = length.value;
    if (length.unit != SR_LENGTH_PIXELS) {
        if (fabs(value) > SR_MAX_RELATIVE_LENGTH ||
            !isfinite(frame.width) || frame.width <= 0.0 ||
            !isfinite(frame.height) || frame.height <= 0.0)
            return SR_ERR_RENDER;
        double reference;
        switch (length.unit) {
        case SR_LENGTH_PERCENT: reference = parent_axis; break;
        case SR_LENGTH_VW: reference = frame.width; break;
        case SR_LENGTH_VH: reference = frame.height; break;
        case SR_LENGTH_VMIN: reference = fmin(frame.width, frame.height); break;
        case SR_LENGTH_VMAX: reference = fmax(frame.width, frame.height); break;
        default: return SR_ERR_RENDER;
        }
        if (!isfinite(reference) || reference <= 0.0) return SR_ERR_RENDER;
        value = value * reference / 100.0;
        if (!isfinite(value) || fabs(value) > SR_MAX_RESOLVED_LENGTH)
            return SR_ERR_RENDER;
    }
    if (positive && value <= 0.0) return SR_ERR_RENDER;
    *pixels = value;
    return SR_OK;
}

SrStatus sr_anim_length_eval(const SrAnimValue *value, double time,
                             double parent_axis, SrLengthBox frame,
                             double *pixels) {
    if (!value || !pixels) return SR_ERR_ARGUMENT;
    const SrTrack *track = &value->track;
    if (!isfinite(time) || (track->clock_set &&
        !isfinite(time * track->clock_scale + track->clock_offset)))
        return SR_ERR_RENDER;
    double base = value->base;
    if (!track->count || track->additive) {
        SrStatus status = sr_length_resolve((SrLength){base, value->unit},
                                            parent_axis, frame, false, &base);
        if (status != SR_OK) return status;
    }
    double result;
    if (track->has_relative) {
        size_t indices[SR_TRACK_NEIGHBORHOOD];
        SrKeyframe keys[SR_TRACK_NEIGHBORHOOD];
        size_t count = sr_track_neighborhood(track, time, indices);
        for (size_t i = 0; i < count; ++i) {
            keys[i] = track->keys[indices[i]];
            SrStatus status = sr_length_resolve((SrLength){keys[i].value, keys[i].unit},
                                                parent_axis, frame, false, &keys[i].value);
            if (status != SR_OK) return status;
            keys[i].unit = SR_LENGTH_PIXELS;
        }
        SrTrack converted = *track;
        converted.keys = keys;
        converted.count = converted.capacity = count;
        converted.has_relative = false;
        result = sr_track_eval(&converted, base, time);
    } else {
        result = sr_track_eval(track, base, time);
    }
    bool relative = track->has_relative ||
        (value->unit != SR_LENGTH_PIXELS && (!track->count || track->additive));
    if (!isfinite(result) || (relative && fabs(result) > SR_MAX_RESOLVED_LENGTH))
        return SR_ERR_RENDER;
    *pixels = result;
    return SR_OK;
}
