/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SCENE_RENDER_TIMELINE_INTERNAL_H
#define SCENE_RENDER_TIMELINE_INTERNAL_H

#include "scene_render/timeline.h"

#define SR_TRACK_NEIGHBORHOOD 6u

/* Select sorted, unique indices from a finalized track for evaluation at
 * scene time: global endpoints, the sampled pair and its tangent neighbors.
 * Copy those keys and the track options, then evaluate with the same scene
 * time. The track is borrowed and never modified; indices is caller-owned. */
size_t sr_track_neighborhood(const SrTrack *track, double time,
                             size_t indices[SR_TRACK_NEIGHBORHOOD]);

#endif
