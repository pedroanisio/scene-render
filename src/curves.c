/* SPDX-License-Identifier: Apache-2.0 */
#include "curves_internal.h"

#include <math.h>

static double bounce_out(double t) {
    const double n = 7.5625, d = 2.75;
    if (t < 1.0 / d) return n * t * t;
    if (t < 2.0 / d) {
        t -= 1.5 / d;
        return n * t * t + 0.75;
    }
    if (t < 2.5 / d) {
        t -= 2.25 / d;
        return n * t * t + 0.9375;
    }
    t -= 2.625 / d;
    return n * t * t + 0.984375;
}

static double penner_in(unsigned family, double t, bool in_out) {
    switch (family) {
    case 0: return 1.0 - cos(t * SR_PI / 2.0);
    case 1: return t * t;
    case 2: return t * t * t;
    case 3: return t * t * t * t;
    case 4: return t * t * t * t * t;
    case 5: return t == 0.0 ? 0.0 : pow(2.0, 10.0 * t - 10.0);
    case 6: return 1.0 - sqrt(fmax(0.0, 1.0 - t * t));
    case 7: {
        double overshoot = 1.70158 * (in_out ? 1.525 : 1.0);
        return (overshoot + 1.0) * t * t * t - overshoot * t * t;
    }
    case 8: {
        if (t == 0.0 || t == 1.0) return t;
        double period = in_out ? 0.45 : 0.3;
        return -pow(2.0, 10.0 * (t - 1.0)) *
            sin((t - 1.0 - period / 4.0) * (2.0 * SR_PI / period));
    }
    case 9: return 1.0 - bounce_out(1.0 - t);
    default: return t;
    }
}

static double penner(SrCurve curve, double t) {
    unsigned index = (unsigned)(curve - SR_CURVE_SINE_IN);
    unsigned family = index / 3, direction = index % 3;
    if (direction == 0) return penner_in(family, t, false);
    if (direction == 1) return 1.0 - penner_in(family, 1.0 - t, false);
    return t < 0.5 ? penner_in(family, 2.0 * t, true) * 0.5
        : 1.0 - penner_in(family, 2.0 * (1.0 - t), true) * 0.5;
}

static double spring(const SrKeyframe *key, double seconds) {
    double w2 = key->stiffness / key->mass;
    double a = key->damping / (2.0 * key->mass);
    double disc = a * a - w2;
    if (fabs(disc) <= 1e-12 * fmax(a * a, w2)) {
        double x = a * seconds;
        return -expm1(-x) - x * exp(-x);
    }
    if (disc < 0.0) {
        double w = sqrt(-disc), x = w * seconds;
        double sinc = fabs(x) < 1e-6 ? 1.0 - x * x / 6.0 : sin(x) / x;
        double half_sine = sin(x * 0.5), at = a * seconds;
        return -expm1(-at) + exp(-at) *
            (2.0 * half_sine * half_sine - at * sinc);
    }
    double b = sqrt(disc);
    double x = b * seconds;
    if (fabs(x) < 1e-3) {
        double x2 = x * x;
        double coshm1 = x2 * (0.5 + x2 * (1.0 / 24.0 + x2 / 720.0));
        double sinhc = 1.0 + x2 * (1.0 / 6.0 + x2 * (1.0 / 120.0 + x2 / 5040.0));
        double at = a * seconds;
        return -expm1(-at) - exp(-at) * (coshm1 + at * sinhc);
    }
    /* The slow root avoids subtracting two nearly equal large values. */
    double slow = -w2 / (a + b), fast = -a - b;
    return -expm1(slow * seconds) + slow / (fast - slow) *
        (exp(fast * seconds) - exp(slow * seconds));
}

static double tangent(const SrTrack *track, size_t index, bool outgoing,
                       bool tcb) {
    const SrKeyframe *key = &track->keys[index];
    size_t before = index ? index - 1 : index;
    size_t after = index + 1 < track->count ? index + 1 : index;
    double left = index ? (key->value - track->keys[before].value) /
                           (key->time - track->keys[before].time) : 0.0;
    double right = after != index ? (track->keys[after].value - key->value) /
                                    (track->keys[after].time - key->time) : left;
    if (!index) left = right;
    double tension = tcb ? key->tension : 0.0;
    double continuity = tcb ? key->continuity : 0.0;
    double bias = tcb ? key->bias : 0.0;
    if (!outgoing) continuity = -continuity;
    return (1.0 - tension) * 0.5 *
        ((1.0 + continuity) * (1.0 + bias) * left +
         (1.0 - continuity) * (1.0 - bias) * right);
}

double sr_curve_extended_segment(const SrTrack *track, size_t left, double t) {
    const SrKeyframe *a = &track->keys[left], *b = &track->keys[left + 1];
    double dt = b->time - a->time;
    if (a->curve == SR_CURVE_CATMULL_ROM || a->curve == SR_CURVE_TCB) {
        bool tcb = a->curve == SR_CURVE_TCB;
        double out = tangent(track, left, true, tcb) * dt;
        double in = tangent(track, left + 1, false, tcb) * dt;
        double t2 = t * t, t3 = t2 * t;
        return (2.0 * t3 - 3.0 * t2 + 1.0) * a->value +
            (-2.0 * t3 + 3.0 * t2) * b->value +
            (t3 - 2.0 * t2 + t) * out + (t3 - t2) * in;
    }
    double progress = t;
    if (a->curve == SR_CURVE_STEPS) {
        double jumps = floor(t * a->steps) + (a->step_start ? 1.0 : 0.0);
        progress = fmin(1.0, jumps / a->steps);
    } else if (a->curve == SR_CURVE_SPRING) {
        double seconds = t * dt;
        if (track->time_base == SR_TIME_NORMALIZED) seconds *= track->seconds_per_unit;
        progress = spring(a, seconds);
    } else {
        progress = penner(a->curve, t);
    }
    return a->value + (b->value - a->value) * progress;
}

double sr_curve_segment_upper_bound(const SrTrack *track, size_t left) {
    const SrKeyframe *a = &track->keys[left], *b = a + 1;
    double high = fmax(a->value, b->value), delta = fabs(b->value - a->value);
    if (a->curve == SR_CURVE_CATMULL_ROM || a->curve == SR_CURVE_TCB) {
        bool tcb = a->curve == SR_CURVE_TCB;
        /* The two tangent Hermite bases have absolute value <= 4/27. */
        return high + (b->time - a->time) * 0.25 *
            (fabs(tangent(track, left, true, tcb)) +
             fabs(tangent(track, left + 1, false, tcb)));
    }
    if (a->curve == SR_CURVE_BEZIER) {
        double over = fmax(0.0, fmax(fmax(a->y1 - 1.0, a->y2 - 1.0),
                                     fmax(-a->y1, -a->y2)));
        return high + delta * over;
    }
    /* Nonnegative damping bounds the spring response to [0,2] by its
     * initial mechanical energy. Penner back/elastic also fit [-1,2]. */
    if (a->curve == SR_CURVE_SPRING ||
        (a->curve >= SR_CURVE_BACK_IN && a->curve <= SR_CURVE_ELASTIC_IN_OUT))
        return high + delta;
    return high;
}

bool sr_curve_parameters_valid(const SrKeyframe *key) {
    if (key->curve < 0 || key->curve >= SR_CURVE_COUNT) return false;
    if (key->curve == SR_CURVE_STEPS &&
        (!key->steps || key->steps > SR_MAX_CURVE_STEPS)) return false;
    if (!isfinite(key->tension) || fabs(key->tension) > 1.0 ||
         !isfinite(key->continuity) || fabs(key->continuity) > 1.0 ||
         !isfinite(key->bias) || fabs(key->bias) > 1.0) return false;
    if (key->curve == SR_CURVE_SPRING &&
        (!isfinite(key->stiffness) || !isfinite(key->damping) || !isfinite(key->mass) ||
         key->stiffness < SR_MIN_SPRING_PARAMETER ||
         key->stiffness > SR_MAX_SPRING_PARAMETER ||
         key->mass < SR_MIN_SPRING_PARAMETER || key->mass > SR_MAX_SPRING_PARAMETER ||
         key->damping < 0.0 || key->damping > SR_MAX_SPRING_PARAMETER)) return false;
    return true;
}
