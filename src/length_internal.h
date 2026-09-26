/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SCENE_RENDER_LENGTH_INTERNAL_H
#define SCENE_RENDER_LENGTH_INTERNAL_H

#include "scene_render/timeline.h"

/* Evaluate an immutable, finalized length track after converting the required
 * keys and additive base to pixels. Uses constant stack storage, no allocation.
 * Failure leaves the caller-owned result intact. */
SrStatus sr_anim_length_eval(const SrAnimValue *value, double time,
                             double parent_axis, SrLengthBox frame,
                             double *pixels);

#endif
