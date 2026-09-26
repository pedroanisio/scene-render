/* SPDX-License-Identifier: Apache-2.0 */
#include "compositor_geometry_internal.h"

#include <math.h>

static bool grid_valid(uint32_t rows, uint32_t cols) {
    return rows >= 2 && rows <= SR_MAX_COMPOSITE_GRID_SIDE &&
           cols >= 2 && cols <= SR_MAX_COMPOSITE_GRID_SIDE;
}

static uint64_t grid_work(uint32_t rows, uint32_t cols) {
    /* Initial/final field tests and 12 Newton iterations; fallback visits
     * (rows+1)*(cols+1) patches including exterior strips/corners. Per patch:
     * row/column dispatch, four hull corners, up to two analytic roots. */
    return 16 + 8 * ((uint64_t)rows + 1) * ((uint64_t)cols + 1);
}

static bool invalid(SrCompositeResources *resources, const char *reason) {
    return sr_composite_resource_fail(resources, SR_ERR_RENDER, reason);
}

bool sr_composite_physics_ready(SrCompositeResources *resources,
                                 const SrScene *scene, const SrNode *node,
                                 double time) {
    if (!resources) return true;
    bool rigid = scene->physics.enabled && node->physics_sample_count;
    const SrSoftBody *soft = &node->soft_body;
    bool simulated = soft->enabled && soft->sample_count;
    if (!rigid && !simulated) return true;
    SrCompositeOwner previous = sr_composite_owner(resources,
        (SrCompositeOwner){node->source_line, simulated ? "softBody" : "rigidBody", NULL});
    bool valid = true;
    if (!isfinite(time) || !isfinite(scene->physics.fixed_step) ||
        scene->physics.fixed_step <= 0)
        valid = invalid(resources, "invalid compositing physics sampling clock");
    else if (rigid && (!node->physics_samples ||
        node->physics_sample_count > SIZE_MAX / sizeof(*node->physics_samples)))
        valid = invalid(resources, "invalid compositing rigid-body sample storage");
    else if (simulated && (!grid_valid(soft->rows, soft->cols) || !soft->offsets))
        valid = invalid(resources, "invalid compositing soft-body grid storage");
    else if (simulated && soft->sample_count >
        SIZE_MAX / ((size_t)soft->rows * soft->cols * 2 * sizeof(double)))
        valid = invalid(resources, "compositing soft-body sample storage overflow");
    sr_composite_owner(resources, previous);
    return valid;
}

bool sr_composite_deform_admit(SrCompositeResources *resources,
                                const SrScene *scene, const SrNode *node,
                                double time, uint64_t *pixel_work) {
    *pixel_work = 0;
    if (!resources) return true;
    if (!sr_composite_physics_ready(resources, scene, node, time)) return false;
    SrCompositeOwner previous = sr_composite_owner(resources,
        (SrCompositeOwner){node->source_line, "deform", "modifier"});
    bool valid = true;
    if (node->modifier_count > SR_MAX_COMPOSITE_MODIFIERS ||
        (node->modifier_count && !node->modifiers))
        valid = invalid(resources, "invalid compositing modifier count/storage");
    /* Preflight, parameter zero/evaluation loops, grid pointer collection and
     * bounds traversal. Key sampling and each grid scalar are separate. */
    if (valid) valid = sr_composite_work(resources, node->modifier_count, 17);
    for (size_t i = 0; valid && i < node->modifier_count; ++i) {
        const SrModifier *modifier = &node->modifiers[i];
        ++*pixel_work; /* reverse modifier dispatch, also for a mesh */
        /* Bounds evaluate amount even for mesh-warp. Keep that arithmetic. */
        valid = sr_composite_anim_work(resources, &modifier->amount, false);
        if (!valid) break;
        if (modifier->type != SR_MOD_MESH_WARP) {
            valid = sr_composite_anim_work(resources, &modifier->amount, false) &&
                sr_composite_anim_work(resources, &modifier->frequency, false) &&
                sr_composite_anim_work(resources, &modifier->phase, false);
            continue;
        }
        if (!grid_valid(modifier->rows, modifier->cols) || !modifier->points) {
            valid = invalid(resources, "invalid compositing mesh-warp grid storage");
            break;
        }
        size_t values = (size_t)modifier->rows * modifier->cols * 2;
        *pixel_work += grid_work(modifier->rows, modifier->cols);
        /* Metadata admission, evaluated output loop and extent scan. */
        valid = sr_composite_work(resources, values, 3);
        for (size_t j = 0; valid && j < values; ++j)
            valid = sr_composite_anim_work(resources, &modifier->points[j], false);
    }
    const SrSoftBody *soft = &node->soft_body;
    if (valid && soft->enabled && soft->sample_count) {
        *pixel_work += grid_work(soft->rows, soft->cols);
        /* Interpolation/output plus the full extent scan. */
        valid = sr_composite_work(resources, (uint64_t)soft->rows * soft->cols * 2, 3);
    }
    sr_composite_owner(resources, previous);
    return valid;
}
