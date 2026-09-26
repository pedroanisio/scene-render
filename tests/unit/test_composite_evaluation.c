/* SPDX-License-Identifier: Apache-2.0 */
#include "resource_fixture.h"
#include "compositor_evaluation_internal.h"
#include "scene_render/compositing.h"
#include "scene_render/card.h"
#include "scene_render/lighting.h"

#include <float.h>

static SrStatus render(SrCompositor *compositor, SrScene *scene, double time,
                        SrFrame *frame, const SrCompositeLimits *limits,
                        SrCompositeResources *result) {
    sr_frame_clear(frame, (float[4]){0}, 1);
    SrCompositeScope scope;
    SrStatus status = sr_composite_scope_begin(&scope, compositor, scene, frame, limits);
    if (status == SR_OK)
        status = sr_compositor_render_scene(compositor, scene, time, frame, NULL);
    status = sr_composite_scope_end(&scope, compositor, status, NULL);
    *result = scope.resources;
    return status;
}

static void sort_order_and_admission(sr_test_ctx *t) {
    const size_t counts[] = {0, 1, 2, 3, 7, 16, 33, 129};
    SrDrawItem source[129], legacy[129], bounded[129];
    for (size_t pattern = 0; pattern < 4; ++pattern) {
        for (size_t trial = 0; trial < sizeof(counts) / sizeof(counts[0]); ++trial) {
            size_t count = counts[trial];
            for (size_t i = 0; i < count; ++i) {
                double key = pattern == 0 ? (double)i : pattern == 1 ? -(double)i
                    : pattern == 2 ? 0 : (double)((i * 37) % 13);
                if (pattern == 3 && i % 17 == 0) key = INFINITY;
                source[i] = (SrDrawItem){.object = i, .key = key, .order = i};
            }
            memcpy(legacy, source, count * sizeof(*source));
            memcpy(bounded, source, count * sizeof(*source));
            CHECK(t, sr_composite_sort_items(NULL, legacy, count));
            SrCompositeResources resources;
            sr_composite_resources_init(&resources, NULL);
            CHECK(t, sr_composite_sort_items(&resources, bounded, count));
            CHECK(t, memcmp(legacy, bounded, count * sizeof(*source)) == 0);
            for (size_t i = 1; i < count; ++i) {
                CHECK(t, bounded[i - 1].key >= bounded[i].key);
                if (bounded[i - 1].key == bounded[i].key)
                    CHECK(t, bounded[i - 1].order < bounded[i].order);
            }
            size_t levels = 0, capacity = 1;
            while (capacity < count) { capacity *= 2; ++levels; }
            uint64_t expected = count < 2 ? 0 : 2 * count * (levels + 1) *
                (4 + 3 * ((sizeof(SrDrawItem) + 3) / 4));
            CHECK_INT(t, resources.work, expected);
            CHECK_INT(t, resources.peak_bytes, 0);
            if (!expected) continue;
            SrCompositeLimits quota = {SR_MAX_COMPOSITE_BYTES, SR_MAX_COMPOSITE_PIXELS,
                expected - 1};
            memcpy(bounded, source, count * sizeof(*source));
            sr_composite_resources_init(&resources, &quota);
            CHECK(t, !sr_composite_sort_items(&resources, bounded, count));
            CHECK(t, memcmp(source, bounded, count * sizeof(*source)) == 0);
            CHECK_INT(t, resources.status, SR_ERR_RENDER);
            ++quota.work;
            sr_composite_resources_init(&resources, &quota);
            CHECK(t, sr_composite_sort_items(&resources, bounded, count));
            CHECK(t, memcmp(legacy, bounded, count * sizeof(*source)) == 0);
        }
    }
    SrCompositeResources resources;
    sr_composite_resources_init(&resources, NULL);
    CHECK(t, !sr_composite_sort_items(&resources, NULL, SR_MAX_COMPOSITE_DRAW_ITEMS + 1));
    CHECK_INT(t, resources.work, 0);
    sr_composite_resources_init(&resources, NULL);
    CHECK(t, !sr_composite_sort_items(&resources, NULL, 1));
}

static void scalar_costs_at_render_consumers(sr_test_ctx *t) {
    SrScene scene;
    fx_scene(&scene, 16, 16);
    SrNode *node = fx_rect(&scene, NULL, 0, 0, 16, 16, (SrColor){.3, .5, .8, 1}, 1);
    CHECK(t, node != NULL);
    if (!node) { sr_scene_free(&scene); return; }
    CHECK_INT(t, sr_node_add_mask(node, fx_mask(SR_MASK_RECT, 0, 0, 16, 16, false)), SR_OK);
    SrCompositor compositor;
    sr_compositor_init(&compositor, 1);
    SrFrame frame = {0}, reference = {0};
    CHECK_INT(t, sr_frame_init(&frame, 16, 16), SR_OK);
    CHECK_INT(t, sr_frame_init(&reference, 16, 16), SR_OK);
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
    SrCompositeResources base, keyed;
    CHECK_INT(t, render(&compositor, &scene, .5, &reference, NULL, &base), SR_OK);
    SrTrack *tracks[] = {&node->transform.x.track, &node->transform.rotation.track,
        &node->transform.scale_x.track, &node->opacity.track, &node->masks[0].x.track,
        &node->masks[0].radius.track, &node->fill.r};
    const double values[] = {0, 0, 1, 1, 0, 0, 0};
    for (size_t i = 0; i < sizeof(tracks) / sizeof(tracks[0]); ++i) {
        sr_scene_invalidate_compositing(&scene);
        SrKeyframe key = {.time = 0, .value = values[i], .curve = SR_CURVE_LINEAR};
        *tracks[i] = (SrTrack){.keys = &key, .count = 1, .additive = i == 6};
        CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
        CHECK_INT(t, render(&compositor, &scene, .5, &frame, NULL, &keyed), SR_OK);
        /* Each selected track is sampled once: 64 keyed versus one static. */
        CHECK_INT(t, keyed.work - base.work, 63);
        CHECK(t, memcmp(frame.px, reference.px, 16 * 16 * 4 * sizeof(float)) == 0);
        sr_scene_invalidate_compositing(&scene);
        *tracks[i] = (SrTrack){0};
    }
    /* A color's r channel enables evaluation of every RGBA track. Keep that
     * keyed in both renders, so the alpha delta measures a consumed track. */
    node->stroke_width = 2;
    node->stroke.base = (SrColor){0, 1, 0, .25};
    SrKeyframe red = {.time = 0, .value = 0, .curve = SR_CURVE_LINEAR};
    SrKeyframe alpha = {.time = 0, .value = .25, .curve = SR_CURVE_LINEAR};
    node->stroke.r = (SrTrack){.keys = &red, .count = 1, .additive = true};
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
    CHECK_INT(t, render(&compositor, &scene, .5, &reference, NULL, &base), SR_OK);
    sr_scene_invalidate_compositing(&scene);
    node->stroke.a = (SrTrack){.keys = &alpha, .count = 1};
    CHECK(t, node->stroke.r.count > 0);
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
    CHECK_INT(t, render(&compositor, &scene, .5, &frame, NULL, &keyed), SR_OK);
    CHECK_INT(t, keyed.work - base.work, 63);
    CHECK(t, memcmp(frame.px, reference.px, 16 * 16 * 4 * sizeof(float)) == 0);
    sr_scene_invalidate_compositing(&scene);
    node->stroke.r = node->stroke.a = (SrTrack){0};
    sr_compositor_free(&compositor);
    sr_frame_free(&frame);
    sr_frame_free(&reference);
    sr_scene_free(&scene);
}

static void malformed_consumed_tracks(sr_test_ctx *t) {
    SrScene scene;
    fx_scene(&scene, 16, 16);
    SrNode *node = fx_rect(&scene, NULL, 0, 0, 16, 16, (SrColor){1, 0, 0, 1}, 1);
    CHECK(t, node != NULL);
    if (!node) { sr_scene_free(&scene); return; }
    node->source_line = 41;
    for (size_t i = 0; i < 9; ++i) {
        SrMask mask = fx_mask(SR_MASK_RECT, 0, 0, 16, 16, false);
        mask.source_line = 52;
        CHECK_INT(t, sr_node_add_mask(node, mask), SR_OK);
    }
    SrTrack *tracks[] = {&node->transform.scale_x.track, &node->opacity.track,
        &node->masks[8].radius.track, &node->fill.r, &node->source_time.track};
    SrCompositor compositor;
    sr_compositor_init(&compositor, 4);
    SrFrame frame = {0};
    CHECK_INT(t, sr_frame_init(&frame, 16, 16), SR_OK);
    for (size_t i = 0; i < sizeof(tracks) / sizeof(tracks[0]); ++i) {
        for (size_t variant = 0; variant < 2; ++variant) {
            sr_scene_invalidate_compositing(&scene);
            node->type = i == 4 ? SR_NODE_MEDIA : SR_NODE_SHAPE;
            SrKeyframe key = {0};
            *tracks[i] = variant ? (SrTrack){.count = SR_MAX_TRACK_KEYS + 1, .keys = &key}
                : (SrTrack){.count = 1};
            CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
            SrCompositeResources result;
            CHECK_INT(t, render(&compositor, &scene, 0, &frame, NULL, &result), SR_ERR_RENDER);
            CHECK_INT(t, result.failure_owner.line, i == 2 ? 52 : 41);
            CHECK_INT(t, result.bytes, 0);
            CHECK_INT(t, result.pixels, 0);
            sr_scene_invalidate_compositing(&scene);
            *tracks[i] = (SrTrack){0};
        }
    }
    node->type = SR_NODE_SHAPE;
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
    SrCompositeResources result;
    CHECK_INT(t, render(&compositor, &scene, 0, &frame, NULL, &result), SR_OK);
    sr_compositor_free(&compositor);
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

static void entry_clock_counts_and_camera(sr_test_ctx *t) {
    SrScene scene;
    fx_scene(&scene, 16, 16);
    const SrCamera valid_camera = {.active = true, .source_line = 73,
        .z = {.base = -16}, .fov = {.base = 60}, .zoom = {.base = 16},
        .near_plane = .1, .far_plane = 1000};
    SrCamera camera = valid_camera;
    scene.root->source_line = 19;
    scene.cameras = &camera;
    scene.camera_count = 1;
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
    SrCompositor compositor;
    sr_compositor_init(&compositor, 1);
    SrFrame frame = {0};
    CHECK_INT(t, sr_frame_init(&frame, 16, 16), SR_OK);
    for (size_t i = 0; i < 9; ++i) {
        SrCompositeResources baseline;
        CHECK_INT(t, render(&compositor, &scene, 0, &frame, NULL, &baseline), SR_OK);
        CHECK_INT(t, baseline.status, SR_OK);
        CHECK_INT(t, baseline.bytes, 0);
        double time = 0;
        if (i == 0) time = NAN;
        if (i == 1) time = SR_MAX_DURATION + 1;
        if (i == 2) time = -SR_MAX_DURATION - 1;
        if (i == 3) scene.camera_count = SR_MAX_COMPOSITE_CAMERAS + 1;
        if (i == 4) scene.cameras = NULL;
        if (i == 5) scene.object3d_count = SR_MAX_COMPOSITE_OBJECTS + 1;
        if (i == 6) scene.object3d_count = 1;
        if (i == 7) camera.zoom_set = true;
        if (i == 7) camera.zoom.track.count = 1;
        if (i == 8) camera.x.track.count = 1;
        SrCompositeResources result;
        CHECK_INT(t, render(&compositor, &scene, time, &frame, NULL, &result), SR_ERR_RENDER);
        CHECK_INT(t, result.status, SR_ERR_RENDER);
        CHECK_INT(t, result.bytes, 0);
        CHECK_INT(t, result.pixels, 0);
        if (i >= 7) {
            CHECK_INT(t, result.failure_owner.line, 73);
            CHECK_STR(t, result.failure_owner.element, "camera");
        } else {
            CHECK_INT(t, result.failure_owner.line, 19);
            CHECK_STR(t, result.failure_owner.element, "composition");
            CHECK_STR(t, result.failure_owner.attribute, "camera/object3D/time");
        }
        scene.cameras = &camera;
        scene.camera_count = 1;
        scene.object3d_count = 0;
        camera = valid_camera;
    }
    scene.cameras = NULL;
    scene.camera_count = 0;
    for (int i = -1; i <= 1; ++i) {
        SrCompositeResources result;
        CHECK_INT(t, render(&compositor, &scene, (double)i * SR_MAX_DURATION,
                             &frame, NULL, &result), SR_OK);
    }
    sr_compositor_free(&compositor);
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

static void card_history_and_quotas(sr_test_ctx *t) {
    for (size_t relative = 0; relative < 2; ++relative) {
        SrScene scene;
        bool built = resource_evaluation_scene(&scene);
        CHECK(t, built);
        if (!built) { sr_scene_free(&scene); return; }
        if (relative) {
            scene.has_relative_lengths = true;
            SrNode *card = scene.root->children[0];
            card->transform.x = card->transform.anchor_x =
                (SrAnimValue){.base = 50, .unit = SR_LENGTH_PERCENT};
        }
        if (relative) {
            scene.objects3d = sr_alloc(sizeof(*scene.objects3d));
            CHECK(t, scene.objects3d != NULL);
            if (!scene.objects3d) { sr_scene_free(&scene); return; }
            scene.object3d_count = scene.object3d_capacity = 1;
            scene.objects3d[0] = (SrObject3D){.radius = 24, .source_line = 121,
                .transform = {.z = {.base = 5}, .scale_x = {.base = 1},
                    .scale_y = {.base = 1}, .scale_z = {.base = 1}}};
        }
        SrFrame frame = {0}, reference = {0};
        CHECK_INT(t, sr_frame_init(&frame, 256, 192), SR_OK);
        CHECK_INT(t, sr_frame_init(&reference, 256, 192), SR_OK);
        SrCompositor compositor;
        sr_compositor_init(&compositor, 1);
        if (!relative) {
            size_t count = scene.root->child_count;
            scene.root->child_count = 1;
            SrCardView view = sr_card_view(&scene, .5);
            SrCardPose pose = sr_card_pose(&view, 128, 96, 0, 0, 30);
            CHECK(t, !pose.is_affine && pose.invertible);
            CHECK_INT(t, sr_compositor_render_scene(&compositor, &scene, .5,
                                                    &reference, NULL), SR_OK);
            size_t warped_pixels = 0;
            for (size_t pixel = 0; pixel < 256 * 192; ++pixel)
                if (reference.px[pixel * 4 + 3] > 0) ++warped_pixels;
            /* With only this projective card, each nonzero output pixel was
             * warped. Its dispatch area therefore exceeds the threshold. */
            CHECK(t, warped_pixels > 16384);
            compositor.threads = 4;
            CHECK_INT(t, sr_compositor_render_scene(&compositor, &scene, .5,
                                                    &frame, NULL), SR_OK);
            CHECK(t, memcmp(frame.px, reference.px, 256 * 192 * 4 * sizeof(float)) == 0);
            scene.root->child_count = count;
            sr_frame_clear(&frame, (float[4]){0}, 1);
            sr_frame_clear(&reference, (float[4]){0}, 1);
            compositor.threads = 1;
        }
        CHECK_INT(t, sr_compositor_render_scene(&compositor, &scene, .5, &reference, NULL), SR_OK);
        compositor.threads = 4;
        CHECK_INT(t, sr_compositor_render_scene(&compositor, &scene, .5, &frame, NULL), SR_OK);
        size_t bytes = 256 * 192 * 4 * sizeof(float);
        CHECK(t, memcmp(frame.px, reference.px, bytes) == 0);
        CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
        SrCompositeResources first, next;
        CHECK_INT(t, render(&compositor, &scene, .5, &frame, NULL, &first), SR_OK);
        CHECK(t, memcmp(frame.px, reference.px, bytes) == 0);
        CHECK(t, first.work > 16384 * SR_COMPOSITE_WARP_PIXEL_WORK);
        size_t visible = 0;
        for (size_t pixel = 0; pixel < 256 * 192; ++pixel)
            if (frame.px[pixel * 4 + 3] > 0) ++visible;
        CHECK(t, visible > 16384);
        SrCompositeLimits exact = {first.peak_bytes, first.peak_pixels, first.work};
        for (size_t run = 0; run < 4; ++run) {
            compositor.threads = run % 2 ? 1 : 4;
            CHECK_INT(t, render(&compositor, &scene, run % 2 ? 0 : 1,
                                 &frame, NULL, &next), SR_OK);
            CHECK_INT(t, render(&compositor, &scene, .5, &frame, &exact, &next), SR_OK);
            CHECK_INT(t, next.work, first.work);
            CHECK_INT(t, next.peak_bytes, first.peak_bytes);
            CHECK_INT(t, next.peak_pixels, first.peak_pixels);
            CHECK_INT(t, next.bytes, 0);
            CHECK(t, memcmp(frame.px, reference.px, bytes) == 0);
        }
        SrCompositeLimits low = exact;
        --low.work;
        CHECK_INT(t, render(&compositor, &scene, .5, &frame, &low, &next), SR_ERR_RENDER);
        CHECK_INT(t, next.bytes, 0);
        low = exact;
        --low.bytes;
        CHECK_INT(t, render(&compositor, &scene, .5, &frame, &low, &next), SR_ERR_RENDER);
        CHECK_INT(t, next.bytes, 0);
        CHECK_INT(t, render(&compositor, &scene, .5, &frame, &exact, &next), SR_OK);
        sr_compositor_free(&compositor);
        sr_frame_free(&frame);
        sr_frame_free(&reference);
        sr_scene_free(&scene);
        sr_lighting_release();
    }
}

static void checked_projective_coordinates(sr_test_ctx *t) {
    SrScene scene;
    fx_scene(&scene, UINT32_MAX, 64);
    scene.cameras = sr_alloc(sizeof(*scene.cameras));
    CHECK(t, scene.cameras != NULL);
    if (!scene.cameras) { sr_scene_free(&scene); return; }
    scene.camera_count = scene.camera_capacity = 1;
    scene.cameras[0] = (SrCamera){.active = true, .z = {.base = -128},
        .zoom = {.base = 128}, .zoom_set = true, .near_plane = .1, .far_plane = 10000};
    SrNode *card = fx_add(&scene, NULL, SR_NODE_GROUP);
    CHECK(t, card != NULL);
    if (!card) { sr_scene_free(&scene); return; }
    card->card = true;
    card->source_line = 88;
    card->transform.x.base = (double)UINT32_MAX * .5;
    card->transform.y.base = 32;
    card->transform.rotation_y.base = 25;
    SrNode *leaf = fx_rect(&scene, card, 0, 0, 48, 48, (SrColor){1, 0, 0, 1}, 1);
    CHECK(t, leaf != NULL);
    if (!leaf) { sr_scene_free(&scene); return; }
    scene.has_cards = true;
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
    SrFrame frame = {0};
    CHECK_INT(t, sr_frame_init(&frame, 64, 64), SR_OK);
    SrCompositor compositor;
    sr_compositor_init(&compositor, 4);
    SrCompositeResources result;
    /* A finite projected x exceeds INT_MAX, but the declared output is 64 px.
     * Instrumented pre-fix compositor reports float-cast overflow here. */
    CHECK_INT(t, render(&compositor, &scene, 0, &frame, NULL, &result), SR_OK);
    for (size_t i = 0; i < 64 * 64 * 4; ++i) CHECK_NEAR(t, frame.px[i], 0, 0);
    sr_scene_invalidate_compositing(&scene);
    leaf->shape_width = DBL_MAX;
    leaf->transform.x.base = DBL_MAX;
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
    CHECK_INT(t, render(&compositor, &scene, 0, &frame, NULL, &result), SR_ERR_RENDER);
    CHECK_INT(t, result.bytes, 0);
    CHECK_INT(t, result.failure_owner.line, 88);
    sr_scene_invalidate_compositing(&scene);
    scene.project.width = 64;
    scene.cameras[0].z.base = -1e100;
    scene.cameras[0].zoom.base = 1e-100;
    scene.cameras[0].far_plane = 1e101;
    card->transform.x.base = card->transform.y.base = 0;
    card->transform.rotation_y.base = 1e-118;
    leaf->transform.x.base = 0;
    leaf->shape_width = leaf->shape_height = 1e202;
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
    /* Both plane extents are finite; their product must be checked before
     * division/sqrt can turn it into a zero scale and silently skip drawing. */
    CHECK_INT(t, render(&compositor, &scene, 0, &frame, NULL, &result), SR_ERR_RENDER);
    CHECK_INT(t, result.bytes, 0);
    CHECK_INT(t, result.failure_owner.line, 88);
    sr_compositor_free(&compositor);
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

static void warp_resolution_ceiling(sr_test_ctx *t) {
    SrCompositeResources resources;
    sr_composite_resources_init(&resources, NULL);
    CHECK(t, sr_composite_work(&resources, 2097152, SR_COMPOSITE_WARP_PIXEL_WORK));
    CHECK_INT(t, resources.work, SR_MAX_COMPOSITE_WORK);
    CHECK(t, !sr_composite_work(&resources, 1, SR_COMPOSITE_WARP_PIXEL_WORK));
    sr_composite_resources_init(&resources, NULL);
    CHECK(t, sr_composite_work(&resources, 1920 * 1080, SR_COMPOSITE_WARP_PIXEL_WORK));
    CHECK_INT(t, SR_MAX_COMPOSITE_WORK - resources.work, 12058624);
    sr_composite_resources_init(&resources, NULL);
    CHECK(t, !sr_composite_work(&resources, 3840 * 2160, SR_COMPOSITE_WARP_PIXEL_WORK));
    CHECK_INT(t, resources.work, 0);
}

const sr_test_case sr_tests_composite_evaluation[] = {
    {"sort_order_and_admission", sort_order_and_admission},
    {"scalar_costs_at_render_consumers", scalar_costs_at_render_consumers},
    {"malformed_consumed_tracks", malformed_consumed_tracks},
    {"entry_clock_counts_and_camera", entry_clock_counts_and_camera},
    {"card_history_and_quotas", card_history_and_quotas},
    {"checked_projective_coordinates", checked_projective_coordinates},
    {"warp_resolution_ceiling", warp_resolution_ceiling},
    {NULL, NULL}
};
