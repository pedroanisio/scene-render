/* SPDX-License-Identifier: Apache-2.0 */
#include "fixture.h"

static void test_clear_fills_premultiplied(sr_test_ctx *t)
{
    SrFrame frame = {0};
    CHECK(t, sr_frame_init(&frame, 5, 3) == SR_OK);
    if (!frame.px) return;
    const float color[4] = {0.1f, 0.2f, 0.3f, 0.5f};
    sr_frame_clear(&frame, color, 2);
    for (size_t i = 0; i < 15; ++i)
        CHECK(t, memcmp(&frame.px[i * 4], color, sizeof(color)) == 0);
    sr_frame_free(&frame);
}

/* Row-parallel drawing is thread invariant: every pixel is written by one
 * worker with the same arithmetic. */
static void test_thread_invariant(sr_test_ctx *t)
{
    SrScene scene;
    fx_scene(&scene, 320, 240);
    SrNode *group = fx_add(&scene, NULL, SR_NODE_GROUP);
    if (!group) { sr_scene_free(&scene); return; }
    group->opacity.base = 0.8;
    CHECK(t, sr_node_add_mask(group, fx_mask(SR_MASK_ELLIPSE, 10, 10, 300, 220, false)) == SR_OK);
    for (int i = 0; i < 6; ++i) {
        /* Large enough (> SR_PARALLEL_MIN_PIXELS) to run row-parallel. */
        SrNode *rect = fx_rect(&scene, group, 20.0 + i * 34.6, 16.0 + i * 22.2,
                               160, 110, (SrColor){0.2 * i, 0.9, 1.0 - 0.1 * i, 0.7},
                               0.9);
        if (rect) {
            rect->transform.rotation.base = 11.0 * i;
            rect->blend = (SrBlendMode)(i % 6);
            rect->shape = i % 2 ? SR_SHAPE_ELLIPSE : SR_SHAPE_RECT;
        }
    }
    const float background[4] = {0.02f, 0.03f, 0.05f, 1.0f};
    SrFrame frames[2] = {{0}, {0}};
    const unsigned threads[2] = {1, 7};
    for (int i = 0; i < 2; ++i) {
        CHECK(t, sr_frame_init(&frames[i], 320, 240) == SR_OK);
        if (!frames[i].px) continue;
        sr_frame_clear(&frames[i], background, threads[i]);
        SrCompositor compositor;
        sr_compositor_init(&compositor, threads[i]);
        CHECK(t, sr_compositor_render(&compositor, &scene, 0.0, &frames[i], NULL) == SR_OK);
        sr_compositor_free(&compositor);
    }
    if (frames[0].px && frames[1].px)
        CHECK(t, memcmp(frames[0].px, frames[1].px,
                        (size_t)320 * 240 * 4 * sizeof(float)) == 0);
    sr_frame_free(&frames[0]);
    sr_frame_free(&frames[1]);
    sr_scene_free(&scene);
}

const sr_test_case sr_tests_compositor[] = {
    {"clear_fills_premultiplied", test_clear_fills_premultiplied},
    {"thread_invariant", test_thread_invariant},
    {NULL, NULL},
};
