/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SR_COMPOSITOR_GEOMETRY_INTERNAL_H
#define SR_COMPOSITOR_GEOMETRY_INTERNAL_H

#include "compositor_resources_internal.h"

#define SR_MAX_COMPOSITE_MODIFIERS 65536u
#define SR_MAX_COMPOSITE_GRID_SIDE 16u

/* Work units cover loop iterations, including the slow analytic inverse.
 * Geometry arrays remain caller-owned: points must hold rows*cols*2 values,
 * offsets sample_count*rows*cols*2 doubles, physics_samples sample_count
 * records. The API cannot discover an undersized allocation from its pointer. */
bool sr_composite_physics_ready(SrCompositeResources *resources,
                                 const SrScene *scene, const SrNode *node,
                                 double time);
bool sr_composite_deform_admit(SrCompositeResources *resources,
                                const SrScene *scene, const SrNode *node,
                                double time, uint64_t *pixel_work);

#endif
