#include "scene_render/timeline.h"
#include "curves_internal.h"
#include "timeline_internal.h"

#include <float.h>
#include <math.h>
#include <stdlib.h>
#include <string.h>

void sr_track_free(SrTrack *track) {
    if (!track) {
        return;
    }
    free(track->keys);
    *track = (SrTrack){0};
}

SrStatus sr_track_add(SrTrack *track, SrKeyframe key) {
    if (!track) {
        return SR_ERR_ARGUMENT;
    }
    if (track->count >= SR_MAX_TRACK_KEYS) return SR_ERR_XML;
    if (track->count == track->capacity) {
        size_t capacity = track->capacity ? track->capacity * 2 : 4;
        if (capacity > SR_MAX_TRACK_KEYS) capacity = SR_MAX_TRACK_KEYS;
        SrKeyframe *keys = sr_realloc(track->keys, capacity * sizeof(*keys));
        if (!keys) {
            return SR_ERR_MEMORY;
        }
        track->keys = keys;
        track->capacity = capacity;
    }
    track->keys[track->count++] = key;
    if (key.unit != SR_LENGTH_PIXELS) track->has_relative = true;
    if (key.curve > SR_CURVE_BEZIER || key.ease_in_set || key.ease_out_set)
        track->extended = true;
    return SR_OK;
}

static int sr_key_compare(const void *left, const void *right) {
    const SrKeyframe *a = left;
    const SrKeyframe *b = right;
    return (a->time > b->time) - (a->time < b->time);
}

bool sr_track_extended(const SrTrack *track) {
    return track && (track->extended || track->has_relative || track->additive ||
        track->time_base != SR_TIME_COMPOSITION || track->clock_set ||
        track->extrapolate_before != SR_EXTRAPOLATE_HOLD ||
        track->extrapolate_after != SR_EXTRAPOLATE_HOLD);
}

static bool handle_valid(SrVec2 handle) {
    return isfinite(handle.x) && isfinite(handle.y) &&
        handle.x >= 0.0 && handle.x <= 1.0 &&
        handle.y >= 0.0 && handle.y <= 1.0;
}

SrStatus sr_track_finalize(SrTrack *track) {
    if (!track) return SR_OK;
    if (track->count > SR_MAX_TRACK_KEYS || (track->count && !track->keys) ||
        track->extrapolate_before < SR_EXTRAPOLATE_HOLD ||
        track->extrapolate_before > SR_EXTRAPOLATE_OFFSET ||
        track->extrapolate_after < SR_EXTRAPOLATE_HOLD ||
        track->extrapolate_after > SR_EXTRAPOLATE_OFFSET ||
        track->time_base < SR_TIME_COMPOSITION || track->time_base > SR_TIME_NORMALIZED)
        return SR_ERR_XML;
    track->has_relative = false;
    if (track->count > 1)
        qsort(track->keys, track->count, sizeof(*track->keys), sr_key_compare);
    for (size_t i = 0; i < track->count; ++i) {
        SrKeyframe *key = &track->keys[i];
        if (!isfinite(key->time) || !isfinite(key->value) ||
            !sr_curve_parameters_valid(key)) return SR_ERR_XML;
        if (key->unit < SR_LENGTH_PIXELS || key->unit > SR_LENGTH_VMAX ||
            (key->unit != SR_LENGTH_PIXELS &&
             fabs(key->value) > SR_MAX_RELATIVE_LENGTH)) return SR_ERR_XML;
        if (key->unit != SR_LENGTH_PIXELS) track->has_relative = true;
        if (i && fabs(key->time - track->keys[i - 1].time) < SR_MIN_KEY_SPACING)
            return SR_ERR_XML;
        if (key->curve > SR_CURVE_BEZIER || key->ease_in_set || key->ease_out_set)
            track->extended = true;
        if ((key->ease_in_set && !handle_valid(key->ease_in)) ||
            (key->ease_out_set && !handle_valid(key->ease_out))) return SR_ERR_XML;
        if ((!i && key->ease_in_set) ||
            (i + 1 == track->count && key->ease_out_set)) return SR_ERR_XML;
        if (i + 1 < track->count) {
            const SrKeyframe *next = &track->keys[i + 1];
            if (key->ease_out_set || next->ease_in_set) {
                if (key->curve != SR_CURVE_BEZIER || key->bezier_set)
                    return SR_ERR_XML;
                if (key->ease_out_set) {
                    key->x1 = key->ease_out.x;
                    key->y1 = key->ease_out.x * key->ease_out.y;
                }
                if (next->ease_in_set) {
                    key->x2 = 1.0 - next->ease_in.x;
                    key->y2 = 1.0 - next->ease_in.x * next->ease_in.y;
                }
            }
        }
    }
    if (sr_track_extended(track)) {
        if (!isfinite(track->domain_start) || !isfinite(track->domain_end) ||
            (track->clock_set &&
             (!isfinite(track->clock_scale) || !isfinite(track->clock_offset) ||
              fabs(track->clock_scale) > SR_MAX_ANIMATION_VALUE ||
              fabs(track->clock_offset) > SR_MAX_ANIMATION_VALUE ||
              !isfinite(track->domain_start * track->clock_scale + track->clock_offset) ||
              !isfinite(track->domain_end * track->clock_scale + track->clock_offset))) ||
            (track->time_base == SR_TIME_NORMALIZED &&
             (!isfinite(track->seconds_per_unit) ||
              track->seconds_per_unit < SR_MIN_KEY_SPACING)))
            return SR_ERR_XML;
        for (size_t i = 0; i < track->count; ++i) {
            const SrKeyframe *key = &track->keys[i];
            if (fabs(key->time) > SR_MAX_ANIMATION_TIME ||
                fabs(key->value) > SR_MAX_ANIMATION_VALUE ||
                !isfinite(key->x1) || !isfinite(key->y1) ||
                !isfinite(key->x2) || !isfinite(key->y2) ||
                fabs(key->y1) > SR_MAX_BEZIER_HANDLE ||
                fabs(key->y2) > SR_MAX_BEZIER_HANDLE)
                return SR_ERR_XML;
            if (i + 1 < track->count) {
                if (!isfinite(sr_curve_segment_upper_bound(track, i))) return SR_ERR_XML;
                if (key->curve == SR_CURVE_SPRING && track->time_base == SR_TIME_NORMALIZED) {
                    double seconds = (track->keys[i + 1].time - key->time) *
                        track->seconds_per_unit;
                    double fastest = key->damping / key->mass +
                        sqrt(key->stiffness / key->mass);
                    if (!isfinite(seconds * fastest)) return SR_ERR_XML;
                }
            }
        }
    }
    return SR_OK;
}

static double sr_cubic(double a, double b, double c, double d, double t) {
    double u = 1.0 - t;
    return u * u * u * a + 3.0 * u * u * t * b +
           3.0 * u * t * t * c + t * t * t * d;
}

static double sr_bezier_progress(const SrKeyframe *key, double x) {
    double low = 0.0;
    double high = 1.0;
    double t = x;
    for (int i = 0; i < 24; ++i) {
        double bx = sr_cubic(0.0, key->x1, key->x2, 1.0, t);
        if (bx < x) {
            low = t;
        } else {
            high = t;
        }
        t = (low + high) * 0.5;
    }
    return sr_cubic(0.0, key->y1, key->y2, 1.0, t);
}

static double sr_curve_apply(const SrKeyframe *key, double t) {
    switch (key->curve) {
    case SR_CURVE_STEP:
        return 0.0;
    case SR_CURVE_EASE_IN:
        return t * t * t;
    case SR_CURVE_EASE_OUT: {
        double u = 1.0 - t;
        return 1.0 - u * u * u;
    }
    case SR_CURVE_EASE_IN_OUT:
        return t < 0.5 ? 4.0 * t * t * t
                       : 1.0 - pow(-2.0 * t + 2.0, 3.0) * 0.5;
    case SR_CURVE_BEZIER:
        return sr_bezier_progress(key, t);
    case SR_CURVE_LINEAR:
    default:
        return t;
    }
}

static double sample(const SrTrack *track, double time) {
    if (track->count == 1) return track->keys[0].value;
    if (time < track->keys[0].time ||
        (time == track->keys[0].time &&
         !(track->keys[0].curve == SR_CURVE_STEPS && track->keys[0].step_start)))
        return track->keys[0].value;
    if (time >= track->keys[track->count - 1].time)
        return track->keys[track->count - 1].value;
    size_t low = 0;
    size_t high = track->count - 1;
    while (high - low > 1) {
        size_t mid = low + (high - low) / 2;
        if (track->keys[mid].time <= time) low = mid;
        else high = mid;
    }
    const SrKeyframe *a = &track->keys[low];
    const SrKeyframe *b = &track->keys[high];
    double t = (time - a->time) / (b->time - a->time);
    if (a->curve > SR_CURVE_BEZIER)
        return sr_curve_extended_segment(track, low, t);
    double curved = sr_curve_apply(a, t);
    return a->value + (b->value - a->value) * curved;
}

typedef struct {
    double time;
    double cycles;
} CycleSample;

static CycleSample cycle_sample(double first, double last, double time,
                                 SrExtrapolation mode) {
    double span = last - first;
    double elapsed = time - first;
    double period = mode == SR_EXTRAPOLATE_PING_PONG ? 2.0 * span : span;
    double local = fmod(elapsed, period);
    if (local < 0.0) local += period;
    if (mode == SR_EXTRAPOLATE_PING_PONG && local > span)
        local = period - local;
    /* Decimal spans can divide to an exact integer while fmod is
     * just below the span. Snap representable cycle products, but
     * never discard the remainder/parity of enormous quotients. */
    double nearest = round(elapsed / span);
    double edge_distance = fmin(local, fabs(span - local));
    double edge_tolerance = fmin(4.0 * DBL_EPSILON * fmax(span, fabs(elapsed)),
                                 1e-7 * span);
    bool boundary = fabs(nearest) < 0x1p52 && elapsed == nearest * span &&
        edge_distance <= edge_tolerance;
    if (boundary)
        local = mode == SR_EXTRAPOLATE_PING_PONG
            ? fmod(fabs(nearest), 2.0) * span : 0.0;
    CycleSample mapped = {.time = first + local};
    if (mode == SR_EXTRAPOLATE_OFFSET) {
        double remainder = fmod(elapsed, span);
        mapped.cycles = boundary ? nearest
            : round((elapsed - remainder) / span) - (remainder < 0.0 ? 1.0 : 0.0);
    }
    return mapped;
}

size_t sr_track_neighborhood(const SrTrack *track, double time,
                             size_t indices[SR_TRACK_NEIGHBORHOOD]) {
    if (!track || !track->count || !indices) return 0;
    indices[0] = 0;
    if (track->count == 1) return 1;
    if (track->clock_set) time = time * track->clock_scale + track->clock_offset;
    double first = track->keys[0].time;
    double last = track->keys[track->count - 1].time;
    if (time < first || time > last) {
        SrExtrapolation mode = time < first ? track->extrapolate_before
                                            : track->extrapolate_after;
        if (mode >= SR_EXTRAPOLATE_LOOP)
            time = cycle_sample(first, last, time, mode).time;
    }
    size_t low = 0, high = track->count - 1;
    while (high - low > 1) {
        size_t mid = low + (high - low) / 2;
        if (track->keys[mid].time <= time) low = mid;
        else high = mid;
    }
    size_t begin = low ? low - 1 : low;
    size_t end = high + 1 < track->count ? high + 1 : high;
    size_t count = 1;
    for (size_t i = begin; i <= end; ++i)
        if (i != indices[count - 1]) indices[count++] = i;
    if (indices[count - 1] != track->count - 1)
        indices[count++] = track->count - 1;
    return count;
}

double sr_track_eval(const SrTrack *track, double base, double time) {
    if (!track || track->count == 0) return base;
    if (track->clock_set) time = time * track->clock_scale + track->clock_offset;
    const SrKeyframe *first = &track->keys[0];
    const SrKeyframe *last = &track->keys[track->count - 1];
    double value;
    if (track->count < 2 || (time >= first->time && time <= last->time)) {
        value = sample(track, time);
    } else {
        bool before = time < first->time;
        SrExtrapolation mode = before ? track->extrapolate_before
                                      : track->extrapolate_after;
        if (mode == SR_EXTRAPOLATE_HOLD) {
            value = before ? first->value : last->value;
        } else if (mode == SR_EXTRAPOLATE_LINEAR) {
            const SrKeyframe *a = before ? first : last - 1;
            const SrKeyframe *b = before ? first + 1 : last;
            value = a->value + (b->value - a->value) *
                ((time - a->time) / (b->time - a->time));
        } else {
            CycleSample mapped = cycle_sample(first->time, last->time, time, mode);
            value = sample(track, mapped.time);
            if (mode == SR_EXTRAPOLATE_OFFSET)
                value += mapped.cycles * (last->value - first->value);
        }
    }
    return track->additive ? base + value : value;
}

double sr_anim_eval(const SrAnimValue *value, double time) {
    return value ? sr_track_eval(&value->track, value->base, time) : 0.0;
}

double sr_anim_upper_bound(const SrAnimValue *value, double begin, double end) {
    if (!value) return 0.0;
    const SrTrack *track = &value->track;
    if (!track->count) return value->base;
    double high = track->keys[0].value;
    for (size_t i = 0; i + 1 < track->count; ++i)
        high = fmax(high, sr_curve_segment_upper_bound(track, i));
    if (track->count > 1) {
        const SrKeyframe *first = track->keys, *last = first + track->count - 1;
        double a = begin, b = end;
        if (track->clock_set) {
            a = a * track->clock_scale + track->clock_offset;
            b = b * track->clock_scale + track->clock_offset;
        }
        double low_time = fmin(a, b), high_time = fmax(a, b);
        double extra = 0.0, delta = last->value - first->value;
        if (low_time < first->time && track->extrapolate_before == SR_EXTRAPOLATE_OFFSET) {
            double cycles = floor((low_time - first->time) / (last->time - first->time));
            extra = fmax(extra, delta * cycles);
        }
        if (high_time > last->time && track->extrapolate_after == SR_EXTRAPOLATE_OFFSET) {
            double cycles = floor((high_time - first->time) / (last->time - first->time));
            extra = fmax(extra, delta * cycles);
        }
        high += extra;
    }
    if (track->additive) high += value->base;
    /* Linear extrapolation is affine outside the keys, so its maximum
     * over either exterior interval occurs at an interval endpoint. */
    return fmax(high, fmax(sr_anim_eval(value, begin), sr_anim_eval(value, end)));
}

static const char *const curve_names[] = {
    "step", "linear", "ease-in",
    "ease-out", "ease-in-out", "cubic-bezier",
    "steps", "catmull-rom", "tcb",
    "spring", "sine-in", "sine-out",
    "sine-in-out", "quad-in", "quad-out",
    "quad-in-out", "cubic-in", "cubic-out",
    "cubic-in-out", "quart-in", "quart-out",
    "quart-in-out", "quint-in", "quint-out",
    "quint-in-out", "expo-in", "expo-out",
    "expo-in-out", "circ-in", "circ-out",
    "circ-in-out", "back-in", "back-out",
    "back-in-out", "elastic-in", "elastic-out",
    "elastic-in-out", "bounce-in", "bounce-out",
    "bounce-in-out",
};

bool sr_curve_parse(const char *text, SrCurve *curve) {
    if (!text || !curve) return false;
    if (!strcmp(text, "hold")) {
        *curve = SR_CURVE_STEP;
        return true;
    }
    for (size_t i = 0; i < SR_CURVE_COUNT; ++i) {
        if (!strcmp(text, curve_names[i])) {
            *curve = (SrCurve)i;
            return true;
        }
    }
    return false;
}

const char *sr_curve_name(SrCurve curve) {
    return curve >= SR_CURVE_STEP && curve < SR_CURVE_COUNT
        ? curve_names[curve] : "unknown";
}

bool sr_extrapolation_parse(const char *text, SrExtrapolation *mode) {
    if (!text || !mode) return false;
    static const char *const names[] = {"hold", "linear", "loop", "ping-pong", "offset"};
    for (size_t i = 0; i < sizeof(names) / sizeof(names[0]); ++i) {
        if (!strcmp(text, names[i])) {
            *mode = (SrExtrapolation)i;
            return true;
        }
    }
    return false;
}
