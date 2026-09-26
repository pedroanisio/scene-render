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

#endif
