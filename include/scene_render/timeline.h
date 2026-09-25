#ifndef SCENE_RENDER_TIMELINE_H
#define SCENE_RENDER_TIMELINE_H

#include "scene_render/common.h"

typedef enum {
    SR_CURVE_STEP,
    SR_CURVE_LINEAR,
    SR_CURVE_EASE_IN,
    SR_CURVE_EASE_OUT,
    SR_CURVE_EASE_IN_OUT,
    SR_CURVE_BEZIER
} SrCurve;

typedef struct {
    double time;
    double value;
    SrCurve curve;
    double x1, y1, x2, y2;
} SrKeyframe;

typedef struct {
    SrKeyframe *keys;
    size_t count;
    size_t capacity;
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

#endif
