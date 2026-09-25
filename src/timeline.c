#include "scene_render/timeline.h"

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
    if (track->count == track->capacity) {
        size_t capacity = track->capacity ? track->capacity * 2 : 4;
        SrKeyframe *keys = sr_realloc(track->keys, capacity * sizeof(*keys));
        if (!keys) {
            return SR_ERR_MEMORY;
        }
        track->keys = keys;
        track->capacity = capacity;
    }
    track->keys[track->count++] = key;
    return SR_OK;
}

static int sr_key_compare(const void *left, const void *right) {
    const SrKeyframe *a = left;
    const SrKeyframe *b = right;
    return (a->time > b->time) - (a->time < b->time);
}

SrStatus sr_track_finalize(SrTrack *track) {
    if (!track || track->count < 2) {
        return SR_OK;
    }
    qsort(track->keys, track->count, sizeof(*track->keys), sr_key_compare);
    for (size_t i = 1; i < track->count; ++i) {
        if (fabs(track->keys[i].time - track->keys[i - 1].time) < 1e-12) {
            return SR_ERR_XML;
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

double sr_track_eval(const SrTrack *track, double base, double time) {
    if (!track || track->count == 0) {
        return base;
    }
    if (time <= track->keys[0].time) {
        return track->keys[0].value;
    }
    if (time >= track->keys[track->count - 1].time) {
        return track->keys[track->count - 1].value;
    }
    size_t low = 0;
    size_t high = track->count - 1;
    while (high - low > 1) {
        size_t mid = low + (high - low) / 2;
        if (track->keys[mid].time <= time) {
            low = mid;
        } else {
            high = mid;
        }
    }
    const SrKeyframe *a = &track->keys[low];
    const SrKeyframe *b = &track->keys[high];
    double t = (time - a->time) / (b->time - a->time);
    double curved = sr_curve_apply(a, t);
    return a->value + (b->value - a->value) * curved;
}

double sr_anim_eval(const SrAnimValue *value, double time) {
    return value ? sr_track_eval(&value->track, value->base, time) : 0.0;
}

bool sr_curve_parse(const char *text, SrCurve *curve) {
    if (!text || !curve) {
        return false;
    }
    static const char *names[] = {"step", "linear", "ease-in", "ease-out",
                                  "ease-in-out", "cubic-bezier"};
    for (size_t i = 0; i < sizeof(names) / sizeof(names[0]); ++i) {
        if (strcmp(text, names[i]) == 0) {
            *curve = (SrCurve)i;
            return true;
        }
    }
    return false;
}

const char *sr_curve_name(SrCurve curve) {
    static const char *names[] = {"step", "linear", "ease-in", "ease-out",
                                  "ease-in-out", "cubic-bezier"};
    return curve >= SR_CURVE_STEP && curve <= SR_CURVE_BEZIER ? names[curve]
                                                               : "unknown";
}
