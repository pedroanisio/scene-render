/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SR_TEST_RESOURCE_FIXTURE_H
#define SR_TEST_RESOURCE_FIXTURE_H

#include "fixture.h"

/* A perspective card containing another isolated group and a masked emitter
 * exercises nested pool ownership and a shared queued mask copy. */
static inline bool resource_card_scene(SrScene *scene) {
    fx_scene(scene, 64, 64);
    scene->cameras = sr_alloc(sizeof(*scene->cameras));
    if (!scene->cameras) return false;
    scene->camera_count = scene->camera_capacity = 1;
    scene->cameras[0] = (SrCamera){.active = true, .z = {.base = -128},
        .zoom = {.base = 128}, .zoom_set = true,
        .near_plane = .1, .far_plane = 10000};
    scene->has_cards = true;
    SrNode *card = fx_add(scene, NULL, SR_NODE_GROUP);
    if (!card) return false;
    card->card = true;
    card->transform.rotation_y.base = 25;
    card->transform.anchor_x.base = card->transform.anchor_y.base = 32;
    card->transform.x.base = card->transform.y.base = 32;
    SrNode *group = fx_add(scene, card, SR_NODE_GROUP);
    if (!group) return false;
    group->opacity.base = .5;
    if (!fx_rect(scene, group, 8, 8, 48, 48, (SrColor){.8, .4, .2, 1}, 1))
        return false;
    SrNode *emitter = fx_add(scene, group, SR_NODE_PARTICLES);
    if (!emitter) return false;
    emitter->particle_rate.base = 30;
    emitter->particle_lifetime.base = 1;
    emitter->transform.x.base = emitter->transform.y.base = 32;
    emitter->masks = sr_alloc(sizeof(*emitter->masks));
    if (!emitter->masks) return false;
    emitter->mask_count = 1;
    emitter->masks[0] = fx_mask(SR_MASK_RECT, -32, -32, 64, 64, false);
    return true;
}

/* Enough relative nodes/masks to grow both evaluated arrays, with one mesh,
 * analytic modifier and already prepared soft grid. */
static inline bool resource_geometry_scene(SrScene *scene) {
    fx_scene(scene, 64, 64);
    scene->has_relative_lengths = true;
    for (size_t i = 0; i < 40; ++i) {
        SrNode *node = fx_rect(scene, NULL, i % 8, i % 5, 50, 50,
                                (SrColor){.3, .6, .8, .1}, 1);
        if (!node) return false;
        node->source_line = 20 + i;
        node->transform.x.unit = node->transform.y.unit = SR_LENGTH_PERCENT;
        node->shape_width_unit = node->shape_height_unit = SR_LENGTH_PERCENT;
        SrMask mask = fx_mask(SR_MASK_RECT, 0, 0, 100, 100, false);
        mask.width.unit = mask.height.unit = SR_LENGTH_PERCENT;
        for (size_t j = 0; j < 2; ++j)
            if (sr_node_add_mask(node, mask) != SR_OK) return false;
    }
    SrNode *node = scene->root->children[0];
    if (sr_track_add(&node->transform.x.track,
        (SrKeyframe){.time = 0, .value = 0, .unit = SR_LENGTH_PERCENT,
                     .curve = SR_CURVE_LINEAR}) != SR_OK ||
        sr_track_add(&node->transform.x.track,
        (SrKeyframe){.time = 1, .value = 50, .unit = SR_LENGTH_PERCENT,
                     .curve = SR_CURVE_LINEAR}) != SR_OK ||
        sr_track_finalize(&node->transform.x.track) != SR_OK)
        return false;
    node->modifiers = sr_alloc(2 * sizeof(*node->modifiers));
    if (!node->modifiers) return false;
    node->modifier_count = 2;
    node->modifiers[0] = (SrModifier){.type = SR_MOD_MESH_WARP, .rows = 2, .cols = 2};
    node->modifiers[0].points = sr_alloc(8 * sizeof(SrAnimValue));
    if (!node->modifiers[0].points) return false;
    node->modifiers[0].points[6].base = 2;
    node->modifiers[1] = (SrModifier){.type = SR_MOD_WAVE,
        .amount = {.base = 1}, .frequency = {.base = 1}, .axis = 'x'};
    node->soft_body = (SrSoftBody){.enabled = true, .rows = 2, .cols = 2,
        .sample_count = 1, .offsets = sr_alloc(8 * sizeof(double))};
    return node->soft_body.offsets != NULL;
}


/* A keyed emitter crosses checkpoint blocks and grows both output and queue;
 * the shared mask makes particle copies outlive the evaluated output array. */
static inline bool resource_particle_scene(SrScene *scene) {
    fx_scene(scene, 96, 96);
    scene->project.duration = 24;
    SrNode *node = fx_add(scene, NULL, SR_NODE_PARTICLES);
    if (!node) return false;
    node->source_line = 91;
    node->transform.x.base = node->transform.y.base = 48;
    node->particle_rate.base = 90;
    node->particle_lifetime.base = 5;
    node->particle_speed.base = 4;
    node->particle_size.base = 6;
    node->particle_spread.base = 180;
    node->particle_max = 2000;
    node->particle_color.base = (SrColor){.8, .4, .2, .2};
    const double times[] = {2, 8, 20}, rates[] = {90, 130, 70};
    for (size_t i = 0; i < 3; ++i)
        if (sr_track_add(&node->particle_rate.track, (SrKeyframe){
            .time = times[i], .value = rates[i], .curve = SR_CURVE_LINEAR}) != SR_OK)
            return false;
    if (sr_track_finalize(&node->particle_rate.track) != SR_OK) return false;
    SrMask mask = fx_mask(SR_MASK_RECT, -48, -48, 96, 96, false);
    return sr_node_add_mask(node, mask) == SR_OK;
}

#endif
