/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SCENE_RENDER_LENGTH_H
#define SCENE_RENDER_LENGTH_H

#include "scene_render/common.h"

#define SR_MAX_LENGTH_BYTES 128u
#define SR_MAX_RELATIVE_LENGTH 1e6
#define SR_MAX_RESOLVED_LENGTH 1e12

typedef enum {
    SR_LENGTH_PIXELS, SR_LENGTH_PERCENT, SR_LENGTH_VW, SR_LENGTH_VH,
    SR_LENGTH_VMIN, SR_LENGTH_VMAX
} SrLengthUnit;

typedef struct {
    double value;
    SrLengthUnit unit;
} SrLength;

typedef struct {
    double width, height;
} SrLengthBox;

/* Relative spellings follow the XSD decimal grammar; unitless values retain
 * sr_parse_double semantics. Failure leaves the caller-owned result intact. */
bool sr_parse_length(const char *text, SrLength *length);
/* parent_axis is the relevant untransformed local width or height. frame
 * always describes the output frame, including viewport and offscreen draws.
 * Unitless values retain their existing range; relative values are bounded.
 * Failure leaves the caller-owned pixel value intact. */
SrStatus sr_length_resolve(SrLength length, double parent_axis,
                           SrLengthBox frame, bool positive, double *pixels);

#endif
