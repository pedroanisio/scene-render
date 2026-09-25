/* SPDX-License-Identifier: Apache-2.0 */
#include "fixture.h"

static const SrColor white = {1, 1, 1, 1};
static const float black[4] = {0, 0, 0, 1};

/* Two overlapping 50% children inside a 50% group: the group is isolated,
 * so the overlap is 0.75 inside the group and 0.375 after compositing,
 * not the double-blended 0.4375 of multiplying 0.25 into each child. */
static void test_isolated_opacity(sr_test_ctx *t)
{
    SrScene scene;
    fx_scene(&scene, 30, 10);
    SrNode *group = fx_add(&scene, NULL, SR_NODE_GROUP);
    CHECK(t, group != NULL);
    if (!group) { sr_scene_free(&scene); return; }
    group->opacity.base = 0.5;
    fx_rect(&scene, group, 0, 0, 20, 10, white, 0.5);
    fx_rect(&scene, group, 10, 0, 20, 10, white, 0.5);
    SrFrame frame = {0};
    if (fx_render(t, &scene, 0.0, black, &frame)) {
        CHECK_NEAR(t, fx_px(&frame, 5, 5)[0], 0.25, 1e-6);
        CHECK_NEAR(t, fx_px(&frame, 15, 5)[0], 0.375, 1e-6);
        CHECK_NEAR(t, fx_px(&frame, 25, 5)[0], 0.25, 1e-6);
        CHECK_NEAR(t, fx_px(&frame, 15, 5)[3], 1.0, 1e-6);
    }
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

/* An isolated group with a non-normal blend composites once with that
 * blend; its children blend against the group's transparent buffer. */
static void test_isolated_blend(sr_test_ctx *t)
{
    SrScene scene;
    fx_scene(&scene, 10, 10);
    SrNode *group = fx_add(&scene, NULL, SR_NODE_GROUP);
    CHECK(t, group != NULL);
    if (!group) { sr_scene_free(&scene); return; }
    group->blend = SR_BLEND_MULTIPLY;
    fx_rect(&scene, group, 0, 0, 10, 10, (SrColor){0.5, 0.5, 0.5, 1}, 1.0);
    const float grey[4] = {0.8f, 0.8f, 0.8f, 1.0f};
    SrFrame frame = {0};
    if (fx_render(t, &scene, 0.0, grey, &frame))
        CHECK_NEAR(t, fx_px(&frame, 5, 5)[0], 0.4, 1e-6);
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

static void build_children(SrScene *scene, SrNode *parent)
{
    fx_rect(scene, parent, 1.25, 2.5, 11, 7, (SrColor){1, 0.2, 0.1, 1}, 0.6);
    SrNode *rotated = fx_rect(scene, parent, 8, 3, 9, 5,
                              (SrColor){0.1, 0.4, 1, 0.8}, 1.0);
    if (rotated) {
        rotated->transform.rotation.base = 23.0;
        rotated->blend = SR_BLEND_SCREEN;
    }
}

/* A group with normal blend, full opacity and no mask is a pass-through:
 * the result equals drawing its children directly. */
static void test_pass_through_equals_direct(sr_test_ctx *t)
{
    const float backdrop[4] = {0.05f, 0.1f, 0.2f, 1.0f};
    SrScene direct, grouped;
    fx_scene(&direct, 24, 16);
    fx_scene(&grouped, 24, 16);
    build_children(&direct, NULL);
    SrNode *group = fx_add(&grouped, NULL, SR_NODE_GROUP);
    CHECK(t, group != NULL);
    if (group) build_children(&grouped, group);
    SrFrame a = {0}, b = {0};
    if (fx_render(t, &direct, 0.0, backdrop, &a) &&
        fx_render(t, &grouped, 0.0, backdrop, &b))
        CHECK(t, memcmp(a.px, b.px, (size_t)24 * 16 * 4 * sizeof(float)) == 0);
    sr_frame_free(&a);
    sr_frame_free(&b);
    sr_scene_free(&direct);
    sr_scene_free(&grouped);
}

/* Pooled buffers are reused across frames and only their dirty rectangle
 * is cleared: rendering twice with one compositor gives identical frames. */
static void test_pool_reuse(sr_test_ctx *t)
{
    SrScene scene;
    fx_scene(&scene, 32, 32);
    SrNode *outer = fx_add(&scene, NULL, SR_NODE_GROUP);
    SrNode *inner = outer ? fx_add(&scene, outer, SR_NODE_GROUP) : NULL;
    CHECK(t, inner != NULL);
    if (!inner) { sr_scene_free(&scene); return; }
    outer->opacity.base = 0.7;
    inner->blend = SR_BLEND_ADD;
    SrNode *moving = fx_rect(&scene, inner, 0, 4, 8, 8, (SrColor){1, 1, 0, 1}, 1.0);
    SrKeyframe k0 = {.time = 0.0, .value = 0.0, .curve = SR_CURVE_LINEAR};
    SrKeyframe k1 = {.time = 1.0, .value = 20.0, .curve = SR_CURVE_LINEAR};
    if (moving) {
        sr_track_add(&moving->transform.x.track, k0);
        sr_track_add(&moving->transform.x.track, k1);
        sr_track_finalize(&moving->transform.x.track);
    }
    SrCompositor compositor;
    sr_compositor_init(&compositor, 2);
    SrFrame first = {0}, again = {0}, fresh = {0};
    CHECK(t, sr_frame_init(&first, 32, 32) == SR_OK);
    CHECK(t, sr_frame_init(&again, 32, 32) == SR_OK);
    CHECK(t, sr_frame_init(&fresh, 32, 32) == SR_OK);
    if (first.px && again.px && fresh.px) {
        CHECK(t, sr_compositor_render(&compositor, &scene, 0.0, &first, NULL) == SR_OK);
        CHECK_INT(t, compositor.pool_count, 2);
        SrGroupBuffer *pooled = compositor.pool[0];
        CHECK(t, sr_compositor_render(&compositor, &scene, 1.0, &again, NULL) == SR_OK);
        CHECK(t, compositor.pool[0] == pooled);
        CHECK(t, sr_composite_scene(&scene, 1.0, &fresh, NULL) == SR_OK);
        CHECK(t, memcmp(again.px, fresh.px, (size_t)32 * 32 * 4 * sizeof(float)) == 0);
        /* Nothing from the t=0 position leaks into the t=1 frame. */
        CHECK_NEAR(t, fx_px(&again, 4, 8)[3], 0.0, 1e-9);
    }
    sr_frame_free(&first);
    sr_frame_free(&again);
    sr_frame_free(&fresh);
    sr_compositor_free(&compositor);
    sr_scene_free(&scene);
}

const sr_test_case sr_tests_group[] = {
    {"isolated_opacity", test_isolated_opacity},
    {"isolated_blend", test_isolated_blend},
    {"pass_through_equals_direct", test_pass_through_equals_direct},
    {"pool_reuse", test_pool_reuse},
    {NULL, NULL},
};
