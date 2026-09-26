#ifndef SCENE_RENDER_TIMELINE_H
#define SCENE_RENDER_TIMELINE_H

#include "scene_render/common.h"

#define SR_MAX_TRACK_KEYS 65536u
#define SR_MAX_SCENE_KEYS 1048576u
#define SR_MAX_CURVE_STEPS 1000000u
#define SR_MAX_ANIMATION_VALUE 1e12
#define SR_MAX_ANIMATION_TIME 1e6
#define SR_MAX_BEZIER_HANDLE 1e6
#define SR_MIN_KEY_SPACING 1e-12
#define SR_MIN_SPRING_PARAMETER 1e-6
#define SR_MAX_SPRING_PARAMETER 1e6

typedef enum {
    SR_CURVE_STEP,
    SR_CURVE_LINEAR,
    SR_CURVE_EASE_IN,
    SR_CURVE_EASE_OUT,
    SR_CURVE_EASE_IN_OUT,
    SR_CURVE_BEZIER,
    SR_CURVE_STEPS,
    SR_CURVE_CATMULL_ROM,
    SR_CURVE_TCB,
    SR_CURVE_SPRING,
    SR_CURVE_SINE_IN, SR_CURVE_SINE_OUT, SR_CURVE_SINE_IN_OUT,
    SR_CURVE_QUAD_IN, SR_CURVE_QUAD_OUT, SR_CURVE_QUAD_IN_OUT,
    SR_CURVE_CUBIC_IN, SR_CURVE_CUBIC_OUT, SR_CURVE_CUBIC_IN_OUT,
    SR_CURVE_QUART_IN, SR_CURVE_QUART_OUT, SR_CURVE_QUART_IN_OUT,
    SR_CURVE_QUINT_IN, SR_CURVE_QUINT_OUT, SR_CURVE_QUINT_IN_OUT,
    SR_CURVE_EXPO_IN, SR_CURVE_EXPO_OUT, SR_CURVE_EXPO_IN_OUT,
    SR_CURVE_CIRC_IN, SR_CURVE_CIRC_OUT, SR_CURVE_CIRC_IN_OUT,
    SR_CURVE_BACK_IN, SR_CURVE_BACK_OUT, SR_CURVE_BACK_IN_OUT,
    SR_CURVE_ELASTIC_IN, SR_CURVE_ELASTIC_OUT, SR_CURVE_ELASTIC_IN_OUT,
    SR_CURVE_BOUNCE_IN, SR_CURVE_BOUNCE_OUT, SR_CURVE_BOUNCE_IN_OUT,
    SR_CURVE_COUNT
} SrCurve;

typedef enum {
    SR_EXTRAPOLATE_HOLD, SR_EXTRAPOLATE_LINEAR, SR_EXTRAPOLATE_LOOP,
    SR_EXTRAPOLATE_PING_PONG, SR_EXTRAPOLATE_OFFSET
} SrExtrapolation;

typedef enum {
    SR_TIME_COMPOSITION, SR_TIME_LOCAL, SR_TIME_NORMALIZED
} SrTimeBase;

typedef struct {
    double time;
    double value;
    SrCurve curve;
    double x1, y1, x2, y2;
    uint32_t steps;
    bool step_start;
    double tension, continuity, bias;
    double stiffness, damping, mass;
    SrVec2 ease_in, ease_out;   /* normalized influence, speed */
    bool ease_in_set, ease_out_set, bezier_set;
    bool tcb_set, spring_set;
    size_t source_line;
} SrKeyframe;

typedef struct {
    SrKeyframe *keys;
    size_t count;
    size_t capacity;
    SrExtrapolation extrapolate_before, extrapolate_after;
    SrTimeBase time_base;
    bool additive;
    bool extended;             /* curve/handle extension, set by add/finalize */
    bool clock_set;
    double clock_scale, clock_offset; /* scene seconds -> track coordinate */
    double seconds_per_unit;   /* normalized tracks: local duration */
    double domain_start, domain_end; /* finite host interval, scene seconds */
} SrTrack;

typedef struct {
    double base;
    SrTrack track;
} SrAnimValue;

void sr_track_free(SrTrack *track);
SrStatus sr_track_add(SrTrack *track, SrKeyframe key);
SrStatus sr_track_finalize(SrTrack *track);
double sr_track_eval(const SrTrack *track, double base, double time);
double sr_anim_eval(const SrAnimValue *value, double time);
bool sr_curve_parse(const char *text, SrCurve *curve);
const char *sr_curve_name(SrCurve curve);
bool sr_extrapolation_parse(const char *text, SrExtrapolation *mode);
bool sr_track_extended(const SrTrack *track);
/* Conservative upper bound over a finite interval of scene time. */
double sr_anim_upper_bound(const SrAnimValue *value, double begin, double end);

#endif
