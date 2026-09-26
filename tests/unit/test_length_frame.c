/* SPDX-License-Identifier: Apache-2.0 */
#include "fixture.h"
#include "length_frame.h"

#include <stdlib.h>

static void set_length(SrAnimValue *value, double coefficient, SrLengthUnit unit) {
    value->base = coefficient;
    value->unit = unit;
}

static SrNode *rectangle(SrScene *scene, SrNode *parent) {
    return fx_rect(scene, parent, 0, 0, 20, 10, (SrColor){1, .25, .5, 1}, 1);
}

static SrMask percentage_mask(void) {
    SrMask mask = fx_mask(SR_MASK_RECT, 25, 25, 50, 50, false);
    mask.x.unit = mask.y.unit = mask.width.unit = mask.height.unit = SR_LENGTH_PERCENT;
    return mask;
}

static void scoped_boxes_and_hosts(sr_test_ctx *t) {
    SrScene scene;
    fx_scene(&scene, 320, 180);
    SrNode *sized = fx_add(&scene, NULL, SR_NODE_GROUP);
    SrNode *shape = rectangle(&scene, sized);
    SrNode *unsized = fx_add(&scene, sized, SR_NODE_GROUP);
    SrNode *inside = rectangle(&scene, unsized);
    SrNode *partial = fx_add(&scene, sized, SR_NODE_GROUP);
    SrNode *nested = rectangle(&scene, partial);
    SrNode *media = fx_add(&scene, sized, SR_NODE_MEDIA);
    SrNode *emitter = fx_add(&scene, sized, SR_NODE_PARTICLES);
    SrAsset *asset = sr_scene_add_asset(&scene);
    CHECK(t, sized && shape && unsized && inside && partial && nested &&
              media && emitter && asset);
    if (!sized || !shape || !unsized || !inside || !partial || !nested ||
        !media || !emitter || !asset) { sr_scene_free(&scene); return; }
    asset->width = 80;
    asset->height = 60;
    media->asset = asset;
    sized->group_width = (SrLength){200, SR_LENGTH_PIXELS};
    sized->group_height = (SrLength){100, SR_LENGTH_PIXELS};
    sized->group_width_set = sized->group_height_set = true;
    partial->group_width = (SrLength){50, SR_LENGTH_VW};
    partial->group_width_set = true;
    SrNode *nodes[] = {sized, shape, unsized, inside, partial, nested, media, emitter};
    for (size_t i = 0; i < sizeof(nodes) / sizeof(nodes[0]); ++i) {
        set_length(&nodes[i]->transform.x, 50, SR_LENGTH_PERCENT);
        set_length(&nodes[i]->transform.y, 25, SR_LENGTH_PERCENT);
        set_length(&nodes[i]->transform.anchor_x, 5, SR_LENGTH_VW);
        set_length(&nodes[i]->transform.anchor_y, 10, SR_LENGTH_VH);
        CHECK_INT(t, sr_node_add_mask(nodes[i], percentage_mask()), SR_OK);
    }
    shape->shape_width = 25;
    shape->shape_height = 50;
    shape->shape_width_unit = shape->shape_height_unit = SR_LENGTH_PERCENT;
    SrNode saved[8];
    for (size_t i = 0; i < 8; ++i) saved[i] = *nodes[i];
    SrLengthFrame frame = {0};
    for (int resized = 0; resized < 2; ++resized) {
        scene.project.width = resized ? 640 : 320;
        scene.project.height = resized ? 360 : 180;
        double w = scene.project.width, h = scene.project.height;
        CHECK_INT(t, sr_length_frame_prepare(&frame, &scene, 0, false, NULL), SR_OK);
        const double boxes[][2] = {
            {200, 100}, {50, 50}, {w, h}, {20, 10},
            {w / 2, h}, {20, 10}, {80, 60}, {w, h}
        };
        const double parents[][2] = {
            {w, h}, {200, 100}, {200, 100}, {w, h},
            {200, 100}, {w / 2, h}, {200, 100}, {200, 100}
        };
        for (size_t i = 0; i < 8; ++i) {
            const SrNodeGeometry *g = sr_length_node(&frame, nodes[i]);
            CHECK_NEAR(t, g->x, parents[i][0] / 2, 0);
            CHECK_NEAR(t, g->y, parents[i][1] / 4, 0);
            CHECK_NEAR(t, g->anchor_x, w * 5 / 100, 0);
            CHECK_NEAR(t, g->anchor_y, h * 10 / 100, 0);
            CHECK_NEAR(t, g->box.width, boxes[i][0], 0);
            CHECK_NEAR(t, g->box.height, boxes[i][1], 0);
            const SrMaskGeometry *m = &frame.masks[g->mask_offset];
            CHECK_NEAR(t, m->x, boxes[i][0] / 4, 0);
            CHECK_NEAR(t, m->y, boxes[i][1] / 4, 0);
            CHECK_NEAR(t, m->width, boxes[i][0] / 2, 0);
            CHECK_NEAR(t, m->height, boxes[i][1] / 2, 0);
            CHECK(t, !memcmp(&saved[i], nodes[i], sizeof(saved[i])));
        }
    }
    sr_length_frame_free(&frame);
    sr_scene_free(&scene);
}

static void animation_and_base_pose(sr_test_ctx *t) {
    SrScene scene;
    fx_scene(&scene, 320, 180);
    SrNode *shape = rectangle(&scene, NULL);
    CHECK(t, shape != NULL);
    if (!shape) { sr_scene_free(&scene); return; }
    set_length(&shape->transform.x, 25, SR_LENGTH_PERCENT);
    SrTrack *track = &shape->transform.x.track;
    CHECK_INT(t, sr_track_add(track, (SrKeyframe){.time = 0, .value = 10,
                  .unit = SR_LENGTH_VH, .curve = SR_CURVE_LINEAR}), SR_OK);
    CHECK_INT(t, sr_track_add(track, (SrKeyframe){.time = 1, .value = 50,
                  .unit = SR_LENGTH_PERCENT}), SR_OK);
    CHECK_INT(t, sr_track_finalize(track), SR_OK);
    SrMask mask = percentage_mask();
    CHECK_INT(t, sr_track_add(&mask.width.track,
                  (SrKeyframe){.time = 0, .value = -20, .curve = SR_CURVE_LINEAR}), SR_OK);
    CHECK_INT(t, sr_track_add(&mask.width.track,
                  (SrKeyframe){.time = 1, .value = 100, .unit = SR_LENGTH_PERCENT}), SR_OK);
    CHECK_INT(t, sr_track_finalize(&mask.width.track), SR_OK);
    CHECK_INT(t, sr_node_add_mask(shape, mask), SR_OK);
    SrNode saved = *shape;
    SrKeyframe keys[2];
    memcpy(keys, track->keys, sizeof(keys));
    const double times[] = {1, .25, .75, 0, .5, .25};
    SrLengthFrame frame = {0};
    for (size_t i = 0; i < sizeof(times) / sizeof(times[0]); ++i) {
        CHECK_INT(t, sr_length_frame_prepare(&frame, &scene, times[i], false, NULL), SR_OK);
        const SrNodeGeometry *g = sr_length_node(&frame, shape);
        CHECK_NEAR(t, g->x, 18 + (160 - 18) * times[i], 0);
        CHECK_NEAR(t, frame.masks[g->mask_offset].width, fmax(0, -20 + 40 * times[i]), 0);
        CHECK(t, !memcmp(&saved, shape, sizeof(saved)));
        CHECK(t, !memcmp(keys, track->keys, sizeof(keys)));
    }
    scene.physics.enabled = true;
    shape->body.type = SR_BODY_DYNAMIC;
    CHECK_INT(t, sr_length_frame_prepare(&frame, &scene, .75, true, NULL), SR_OK);
    CHECK_NEAR(t, sr_length_node(&frame, shape)->x, 80, 0);
    CHECK_INT(t, frame.mask_count, 0);
    sr_length_frame_free(&frame);
    sr_scene_free(&scene);
}

static void inactive_consumers(sr_test_ctx *t) {
    SrScene scene;
    fx_scene(&scene, 320, 180);
    SrNode *card = fx_add(&scene, NULL, SR_NODE_GROUP);
    SrNode *zero = fx_add(&scene, card, SR_NODE_GROUP);
    SrNode *bounds = rectangle(&scene, zero);
    SrNode *hidden = rectangle(&scene, NULL);
    SrNode *expired = rectangle(&scene, NULL);
    CHECK(t, card && zero && bounds && hidden && expired);
    if (!card || !zero || !bounds || !hidden || !expired) {
        sr_scene_free(&scene); return;
    }
    card->card = true;
    zero->opacity.base = 0;
    zero->group_width_set = true;
    zero->group_width = (SrLength){50, SR_LENGTH_VW};
    set_length(&zero->transform.x, 50, SR_LENGTH_PERCENT);
    bounds->shape_width = 200;
    bounds->shape_width_unit = SR_LENGTH_PERCENT;
    set_length(&bounds->transform.x, 50, SR_LENGTH_PERCENT);
    hidden->card = expired->card = true;
    hidden->visible = false;
    expired->end_time = .5;
    set_length(&hidden->transform.x, 25, SR_LENGTH_PERCENT);
    set_length(&expired->transform.y, 50, SR_LENGTH_PERCENT);
    SrMask unused = percentage_mask();
    unused.width.base = SR_MAX_RELATIVE_LENGTH + 1;
    CHECK_INT(t, sr_node_add_mask(hidden, unused), SR_OK);
    CHECK_INT(t, sr_node_add_mask(bounds, unused), SR_OK);
    SrLengthFrame frame = {0};
    CHECK_INT(t, sr_length_frame_prepare(&frame, &scene, 1, false, NULL), SR_OK);
    CHECK_NEAR(t, sr_length_node(&frame, hidden)->x, 80, 0);
    CHECK_NEAR(t, sr_length_node(&frame, expired)->y, 90, 0);
    CHECK_NEAR(t, sr_length_node(&frame, zero)->x, 160, 0);
    CHECK_NEAR(t, sr_length_node(&frame, bounds)->x, 80, 0);
    CHECK_NEAR(t, sr_length_node(&frame, bounds)->box.width, 320, 0);
    CHECK_INT(t, frame.mask_count, 0);
    hidden->card = false;
    hidden->transform.x.base = SR_MAX_RELATIVE_LENGTH + 1;
    CHECK_INT(t, sr_length_frame_prepare(&frame, &scene, 1, false, NULL), SR_OK);
    hidden->card = true;
    CHECK_INT(t, sr_length_frame_prepare(&frame, &scene, 1, false, NULL), SR_ERR_RENDER);
    hidden->transform.x.base = 25;
    CHECK_INT(t, sr_length_frame_prepare(&frame, &scene, 1, false, NULL), SR_OK);
    sr_length_frame_free(&frame);
    sr_scene_free(&scene);
}

static void table_limits_and_reuse(sr_test_ctx *t) {
    SrScene scene;
    fx_scene(&scene, 320, 180);
    SrNode *shape = rectangle(&scene, NULL);
    CHECK(t, shape != NULL);
    if (!shape) { sr_scene_free(&scene); return; }
    SrLengthFrame frame = {0};
    size_t order = shape->order;
    shape->order = SR_MAX_LENGTH_NODES - 1;
    CHECK_INT(t, sr_length_frame_prepare(&frame, &scene, 0, false, NULL), SR_OK);
    CHECK_INT(t, frame.node_count, SR_MAX_LENGTH_NODES);
    shape->order = SR_MAX_LENGTH_NODES;
    CHECK_INT(t, sr_length_frame_prepare(&frame, &scene, 0, false, NULL), SR_ERR_RENDER);
    shape->order = scene.root->order;
    CHECK_INT(t, sr_length_frame_prepare(&frame, &scene, 0, false, NULL), SR_ERR_RENDER);
    shape->order = order;
    shape->mask_count = SR_MAX_LENGTH_MASKS + 1;
    CHECK_INT(t, sr_length_frame_prepare(&frame, &scene, 0, false, NULL), SR_ERR_RENDER);
    shape->mask_count = 0;
    shape->child_count = SR_MAX_LENGTH_NODES + 1;
    CHECK_INT(t, sr_length_frame_prepare(&frame, &scene, 0, false, NULL), SR_ERR_RENDER);
    shape->child_count = 0;
    CHECK_INT(t, sr_length_frame_prepare(&frame, &scene, 0, false, NULL), SR_OK);
    CHECK_INT(t, frame.node_count, 2);
    CHECK_INT(t, sr_length_frame_prepare(NULL, &scene, 0, false, NULL), SR_ERR_ARGUMENT);
    CHECK_INT(t, sr_length_frame_prepare(&frame, NULL, 0, false, NULL), SR_ERR_ARGUMENT);
    CHECK_INT(t, sr_length_frame_prepare(&frame, &scene, NAN, false, NULL), SR_ERR_ARGUMENT);
    SrNode *parent = scene.root;
    for (size_t i = 1; i < SR_MAX_LENGTH_DEPTH; ++i) {
        parent = fx_add(&scene, parent, SR_NODE_GROUP);
        CHECK(t, parent != NULL);
        if (!parent) break;
    }
    CHECK_INT(t, sr_length_frame_prepare(&frame, &scene, 0, false, NULL), SR_OK);
    if (parent) {
        CHECK(t, fx_add(&scene, parent, SR_NODE_GROUP) != NULL);
        CHECK_INT(t, sr_length_frame_prepare(&frame, &scene, 0, false, NULL), SR_ERR_RENDER);
    }
    sr_length_frame_free(&frame);
    sr_length_frame_free(&frame);
    sr_length_frame_free(NULL);
    sr_scene_free(&scene);
}

/* The reference is authored directly in pixels, without calling the length
 * resolver. Rasters deliberately differ from project dimensions. */
static bool reference_scene(SrScene *scene, bool relative, uint32_t w, uint32_t h,
                             bool card) {
    fx_scene(scene, w, h);
    scene->has_relative_lengths = relative;
    scene->has_cards = card;
    SrNode *group = fx_add(scene, NULL, SR_NODE_GROUP);
    SrNode *shape = rectangle(scene, group);
    SrNode *unsized = fx_add(scene, group, SR_NODE_GROUP);
    SrNode *second = rectangle(scene, unsized);
    SrNode *zero = fx_add(scene, group, SR_NODE_GROUP);
    SrNode *bounds = rectangle(scene, zero);
    if (!group || !shape || !unsized || !second || !zero || !bounds) return false;
    group->group_width_set = group->group_height_set = true;
    group->group_width = (SrLength){w / 2.0, SR_LENGTH_PIXELS};
    group->group_height = (SrLength){h / 2.0, SR_LENGTH_PIXELS};
    group->opacity.base = .8;
    group->card = card;
    group->transform.rotation_y.base = card ? 32 : 0;
    group->transform.z.base = 25;
    set_length(&group->transform.x, relative ? 10 : w * 10 / 100.0,
               relative ? SR_LENGTH_VW : SR_LENGTH_PIXELS);
    set_length(&group->transform.y, relative ? 10 : h * 10 / 100.0,
               relative ? SR_LENGTH_VH : SR_LENGTH_PIXELS);
    set_length(&shape->transform.x, relative ? 20 : w * 10 / 100.0,
               relative ? SR_LENGTH_PERCENT : SR_LENGTH_PIXELS);
    set_length(&shape->transform.y, relative ? 15 : h * 15 / 200.0,
               relative ? SR_LENGTH_PERCENT : SR_LENGTH_PIXELS);
    set_length(&shape->transform.anchor_x, relative ? 5 : w * 5 / 200.0,
               relative ? SR_LENGTH_PERCENT : SR_LENGTH_PIXELS);
    set_length(&shape->transform.anchor_y, relative ? 10 : h * 10 / 200.0,
               relative ? SR_LENGTH_PERCENT : SR_LENGTH_PIXELS);
    shape->shape_width = relative ? 50 : w / 4.0;
    shape->shape_height = relative ? 40 : h / 5.0;
    shape->shape_width_unit = shape->shape_height_unit =
        relative ? SR_LENGTH_PERCENT : SR_LENGTH_PIXELS;
    SrMask mask = percentage_mask();
    if (!relative) mask = fx_mask(SR_MASK_RECT, w / 16.0, h / 20.0,
                                  w / 8.0, h / 10.0, false);
    if (sr_node_add_mask(shape, mask) != SR_OK) return false;
    set_length(&second->transform.x, relative ? 50 : w / 2.0,
               relative ? SR_LENGTH_PERCENT : SR_LENGTH_PIXELS);
    set_length(&second->transform.y, relative ? 50 : h / 2.0,
               relative ? SR_LENGTH_PERCENT : SR_LENGTH_PIXELS);
    second->shape_width = relative ? 10 : fmin(w, h) * 10 / 100;
    second->shape_height = relative ? 10 : fmax(w, h) * 10 / 100;
    second->shape_width_unit = relative ? SR_LENGTH_VMIN : SR_LENGTH_PIXELS;
    second->shape_height_unit = relative ? SR_LENGTH_VMAX : SR_LENGTH_PIXELS;
    second->fill.base = (SrColor){.2, .7, .3, 1};
    zero->opacity.base = 0;
    set_length(&bounds->transform.x, relative ? -20 : -(double)w * 20 / 100.0,
               relative ? SR_LENGTH_VW : SR_LENGTH_PIXELS);
    bounds->shape_width = relative ? 100 : w;
    bounds->shape_width_unit = relative ? SR_LENGTH_VW : SR_LENGTH_PIXELS;
    bounds->shape_height = relative ? 100 : h;
    bounds->shape_height_unit = relative ? SR_LENGTH_VH : SR_LENGTH_PIXELS;
    if (card) {
        SrCamera *camera = scene->cameras = sr_alloc(sizeof(*camera));
        if (!camera) return false;
        scene->camera_count = scene->camera_capacity = 1;
        camera->active = camera->zoom_set = true;
        camera->z.base = -300;
        camera->zoom.base = 300;
        camera->near_plane = 1;
        camera->far_plane = 2000;
    }
    return true;
}

static void literal_render_and_reuse(sr_test_ctx *t) {
    SrCompositor compositors[3];
    for (size_t i = 0; i < 3; ++i) sr_compositor_init(&compositors[i], i == 2 ? 4 : 1);
    const uint32_t sizes[][2] = {{320, 180}, {160, 90}, {320, 180}, {180, 320}};
    const float background[4] = {.03f, .02f, .01f, 1};
    for (int card = 0; card < 2; ++card) {
        for (size_t k = 0; k < sizeof(sizes) / sizeof(sizes[0]); ++k) {
            SrScene scenes[2];
            bool a = reference_scene(&scenes[0], false, sizes[k][0], sizes[k][1], card);
            bool b = reference_scene(&scenes[1], true, sizes[k][0], sizes[k][1], card);
            CHECK(t, a && b);
            if (!a || !b) {
                sr_scene_free(&scenes[0]); sr_scene_free(&scenes[1]); continue;
            }
            SrNode *group = scenes[1].root->children[0];
            SrNode saved = *group;
            SrFrame frames[3] = {{0}};
            for (size_t i = 0; i < 3; ++i) {
                CHECK_INT(t, sr_frame_init(&frames[i], 360, 360), SR_OK);
                if (!frames[i].px) continue;
                sr_frame_clear(&frames[i], background, 1);
                CHECK_INT(t, sr_compositor_render_scene(&compositors[i],
                              &scenes[i ? 1 : 0], .25 * (3 - k), &frames[i], NULL), SR_OK);
            }
            if (frames[0].px && frames[1].px && frames[2].px) {
                size_t bytes = (size_t)360 * 360 * 4 * sizeof(float);
                if (memcmp(frames[0].px, frames[1].px, bytes)) {
                    size_t first = 0;
                    while (first < bytes / sizeof(float) &&
                           frames[0].px[first] == frames[1].px[first]) ++first;
                    SR_FAIL(t, "card=%d size=%ux%u first=%zu literal=%.9g relative=%.9g",
                            card, sizes[k][0], sizes[k][1], first,
                            frames[0].px[first], frames[1].px[first]);
                }
                CHECK(t, !memcmp(frames[1].px, frames[2].px, bytes));
                size_t changed = 0;
                for (size_t p = 0; p < (size_t)360 * 360; ++p)
                    changed += memcmp(frames[1].px + p * 4, background, sizeof(background)) != 0;
                CHECK(t, changed > 20);
            }
            CHECK(t, !memcmp(&saved, group, sizeof(saved)));
            for (size_t i = 0; i < 3; ++i) sr_frame_free(&frames[i]);
            sr_scene_free(&scenes[0]); sr_scene_free(&scenes[1]);
        }
    }
    for (size_t i = 0; i < 3; ++i) sr_compositor_free(&compositors[i]);
}

const sr_test_case sr_tests_length_frame[] = {
    {"scoped_boxes_and_hosts", scoped_boxes_and_hosts},
    {"animation_and_base_pose", animation_and_base_pose},
    {"inactive_consumers", inactive_consumers},
    {"table_limits_and_reuse", table_limits_and_reuse},
    {"literal_render_and_reuse", literal_render_and_reuse},
    {NULL, NULL}
};
