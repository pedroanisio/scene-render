/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SCENE_RENDER_CURVES_INTERNAL_H
#define SCENE_RENDER_CURVES_INTERNAL_H

#include "scene_render/timeline.h"

double sr_curve_extended_segment(const SrTrack *track, size_t left,
                                  double progress);
bool sr_curve_parameters_valid(const SrKeyframe *key);
double sr_curve_segment_upper_bound(const SrTrack *track, size_t left);

#endif
