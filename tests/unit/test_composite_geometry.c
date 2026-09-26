/* SPDX-License-Identifier: Apache-2.0 */
#include "resource_fixture.h"
#include "compositing_internal.h"
#include "compositor_geometry_internal.h"
#include "length_frame.h"
#include "timeline_internal.h"

static void declared_work_bounds(sr_test_ctx *t) {
    SrCompositeResources r;
    sr_composite_resources_init(&r, NULL);
    SrAnimValue value = {0};
    CHECK(t, sr_composite_anim_work(&r, &value, false));
    CHECK_INT(t, r.work, 1);
    SrKeyframe key = {0};
    value.track.keys = &key;
    value.track.count = SR_MAX_TRACK_KEYS;
    CHECK(t, sr_composite_anim_work(&r, &value, false));
    CHECK_INT(t, r.work, 65);
    value.track.has_relative = true;
    CHECK(t, sr_composite_anim_work(&r, &value, true));
    uint64_t relative = 64 + 32 + (sizeof(SrTrack) + 3) / 4 +
        SR_TRACK_NEIGHBORHOOD * (2 + (sizeof(SrKeyframe) + 3) / 4);
    CHECK_INT(t, r.work, 65 + relative);
    value.track.count = SR_MAX_TRACK_KEYS + 1;
    CHECK(t, !sr_composite_anim_work(&r, &value, false));
    CHECK_INT(t, r.status, SR_ERR_RENDER);
    sr_composite_resources_init(&r, NULL);
    value.track.count = 1;
    value.track.keys = NULL;
    CHECK(t, !sr_composite_anim_work(&r, &value, false));
    CHECK_INT(t, r.work, 0);

    SrScene scene;
    fx_scene(&scene, 16, 16);
    SrAnimValue points[8] = {{0}};
    SrModifier mods[2] = {{.type = SR_MOD_MESH_WARP, .rows = 2, .cols = 2,
        .points = points}, {.type = SR_MOD_WAVE}};
    SrNode node = {.modifiers = mods, .modifier_count = 2};
    uint64_t pixel_work;
    sr_composite_resources_init(&r, NULL);
    CHECK(t, sr_composite_deform_admit(&r, &scene, &node, 0, &pixel_work));
    CHECK_INT(t, pixel_work, 2 + 16 + 8 * 3 * 3);
    /* 17 per modifier, five scalar parameters/bounds evaluations, and
     * eight grid scalars (three visits + one static evaluation each). */
    CHECK_INT(t, r.work, 2 * 17 + 5 + 8 * 4);
    points[0].base = 1000;
    sr_composite_resources_init(&r, NULL);
    CHECK(t, sr_composite_deform_admit(&r, &scene, &node, .5, &pixel_work));
    CHECK_INT(t, pixel_work, 90);
    mods[0].rows = UINT32_MAX;
    sr_composite_resources_init(&r, NULL);
    CHECK(t, !sr_composite_deform_admit(&r, &scene, &node, 0, &pixel_work));
    mods[0].rows = 2;
    mods[0].points = NULL;
    sr_composite_resources_init(&r, NULL);
    CHECK(t, !sr_composite_deform_admit(&r, &scene, &node, 0, &pixel_work));
    node.modifier_count = SR_MAX_COMPOSITE_MODIFIERS + 1;
    sr_composite_resources_init(&r, NULL);
    CHECK(t, !sr_composite_deform_admit(&r, &scene, &node, 0, &pixel_work));
    CHECK_INT(t, r.work, 0);
    sr_scene_free(&scene);
}

static SrStatus bounded_render(SrCompositor *compositor, SrScene *scene,
                                double time, SrFrame *frame,
                                const SrCompositeLimits *limits,
                                SrCompositeResources *result) {
    sr_frame_clear(frame, (float[4]){0, 0, 0, 0}, 1);
    SrCompositeScope scope;
    SrStatus status = sr_composite_scope_begin(&scope, compositor, scene, frame, limits);
    if (status == SR_OK)
        status = sr_compositor_render(compositor, scene, time, frame, NULL);
    status = sr_composite_scope_end(&scope, compositor, status, NULL);
    *result = scope.resources;
    return status;
}

static void geometry_ownership_and_history(sr_test_ctx *t) {
    SrScene scene;
    bool built = resource_geometry_scene(&scene);
    CHECK(t, built);
    if (!built) { sr_scene_free(&scene); return; }
    scene.project.width = 320;
    scene.project.height = 256;
    /* The first shape fills half each dimension. Its immediate deformation
     * draw must exceed the compositor's 16384-pixel parallel threshold. */
    CHECK(t, (scene.project.width / 2) * (scene.project.height / 2) > 16384);
    SrCompositor compositor;
    sr_compositor_init(&compositor, 1);
    SrFrame legacy = {0}, frame = {0};
    CHECK_INT(t, sr_frame_init(&legacy, scene.project.width, scene.project.height), SR_OK);
    CHECK_INT(t, sr_frame_init(&frame, scene.project.width, scene.project.height), SR_OK);
    size_t bytes = (size_t)frame.width * frame.height * 4 * sizeof(float);
    CHECK_INT(t, sr_compositor_render(&compositor, &scene, .25, &legacy, NULL), SR_OK);
    CHECK(t, compositor.lengths && !compositor.lengths->resources);
    compositor.threads = 4;
    CHECK_INT(t, sr_compositor_render(&compositor, &scene, .25, &frame, NULL), SR_OK);
    CHECK(t, memcmp(legacy.px, frame.px, bytes) == 0);
    compositor.threads = 1;
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
    SrCompositeResources first, next;
    CHECK_INT(t, bounded_render(&compositor, &scene, .25, &frame, NULL, &first), SR_OK);
    CHECK(t, memcmp(legacy.px, frame.px, bytes) == 0);
    CHECK(t, compositor.lengths == NULL && compositor.resources == NULL);
    SrCompositeLimits exact = {first.peak_bytes, first.peak_pixels, first.work};
    for (unsigned run = 0; run < 4; ++run) {
        compositor.threads = run % 2 ? 4 : 1;
        CHECK_INT(t, bounded_render(&compositor, &scene, run % 2 ? .75 : 0,
                                     &frame, NULL, &next), SR_OK);
        CHECK_INT(t, bounded_render(&compositor, &scene, .25, &frame, &exact, &next), SR_OK);
        CHECK_INT(t, next.peak_bytes, first.peak_bytes);
        CHECK_INT(t, next.work, first.work);
        CHECK_INT(t, next.bytes, 0);
        CHECK(t, memcmp(legacy.px, frame.px, bytes) == 0);
    }
    SrCompositeLimits low = exact;
    --low.bytes;
    CHECK_INT(t, bounded_render(&compositor, &scene, .25, &frame, &low, &next),
              SR_ERR_RENDER);
    CHECK_INT(t, next.bytes, 0);
    low = exact;
    --low.work;
    CHECK_INT(t, bounded_render(&compositor, &scene, .25, &frame, &low, &next),
              SR_ERR_RENDER);
    CHECK_INT(t, next.bytes, 0);
    CHECK_INT(t, bounded_render(&compositor, &scene, .25, &frame, &exact, &next), SR_OK);
    sr_compositor_free(&compositor);
    sr_frame_free(&legacy);
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

static void length_growth_and_zero_work(sr_test_ctx *t) {
    SrScene scene;
    fx_scene(&scene, 16, 16);
    CHECK(t, fx_rect(&scene, NULL, 0, 0, 8, 8, (SrColor){1, 0, 0, 1}, 1) != NULL);
    SrCompositeResources r;
    sr_composite_resources_init(&r, NULL);
    SrLengthFrame lengths = {.resources = &r};
    CHECK_INT(t, sr_length_frame_prepare(&lengths, &scene, 0, false, NULL), SR_OK);
    uint64_t used = r.bytes;
    CHECK(t, used >= 32 * sizeof(SrNodeGeometry));
    uint64_t before = r.work;
    CHECK_INT(t, sr_length_frame_prepare(&lengths, &scene, 0, false, NULL), SR_OK);
    /* Two nodes, one child edge, opacity and four transform scalars each,
     * plus the explicit zeroing pass over two previously used entries. */
    CHECK_INT(t, r.work - before, 2 * 8 + 2 + 2 * 5 +
        2 * sizeof(SrNodeGeometry) / 4);
    for (unsigned i = 0; i < 35; ++i) CHECK(t, fx_add(&scene, NULL, SR_NODE_GROUP));
    CHECK_INT(t, sr_length_frame_prepare(&lengths, &scene, .5, false, NULL), SR_OK);
    CHECK(t, r.peak_bytes >= used + r.bytes);
    sr_length_frame_free(&lengths);
    CHECK_INT(t, r.bytes, 0);
    CHECK(t, lengths.resources == NULL);
    sr_scene_free(&scene);
}

static void grid_fallback_reserved_before_pixels(sr_test_ctx *t) {
    SrCompositeResources probe;
    sr_composite_resources_init(&probe, NULL);
    void *p = sr_composite_alloc(&probe, 1, 1, 0);
    CHECK(t, p != NULL);
    uint64_t prefix = probe.bytes - 1;
    sr_composite_free(&probe, p);
    uint64_t wave_work = 0;
    for (unsigned mesh = 0; mesh < 2; ++mesh) {
        SrScene scene;
        fx_scene(&scene, 16, 16);
        SrNode *node = fx_rect(&scene, NULL, 0, 0, 16, 16,
                                (SrColor){1, 0, 0, 1}, 1);
        CHECK(t, node != NULL);
        if (!node) { sr_scene_free(&scene); return; }
        node->source_line = 77;
        node->modifiers = sr_alloc(sizeof(*node->modifiers));
        CHECK(t, node->modifiers != NULL);
        if (!node->modifiers) { sr_scene_free(&scene); return; }
        node->modifier_count = 1;
        node->modifiers[0].type = mesh ? SR_MOD_MESH_WARP : SR_MOD_WAVE;
        if (mesh) {
            node->modifiers[0].rows = node->modifiers[0].cols = 2;
            node->modifiers[0].points = sr_alloc(8 * sizeof(SrAnimValue));
            CHECK(t, node->modifiers[0].points != NULL);
        }
        CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
        SrCompositor compositor;
        sr_compositor_init(&compositor, 4);
        SrFrame frame = {0};
        CHECK_INT(t, sr_frame_init(&frame, 16, 16), SR_OK);
        SrCompositeResources resources;
        CHECK_INT(t, bounded_render(&compositor, &scene, 0, &frame, NULL, &resources), SR_OK);
        CHECK_NEAR(t, fx_px(&frame, 8, 8)[0], 1, 0);
        if (!mesh) wave_work = resources.work;
        else {
            /* Same clip/params/immediate dispatch. The mesh adds worst-case
             * inverse work even though this zero grid always exits early,
             * 8 scalar preparation/extent visits, and two extra allocations. */
            uint64_t extra = 16 * 16 * (16 + 8 * 3 * 3) +
                (8 * 4 - 3) + (2 * prefix + sizeof(double *) +
                                8 * sizeof(double)) / 4;
            CHECK_INT(t, resources.work - wave_work, extra);
            SrCompositeLimits shortfall = {SR_MAX_COMPOSITE_BYTES,
                SR_MAX_COMPOSITE_PIXELS, resources.work - 1};
            CHECK_INT(t, bounded_render(&compositor, &scene, 0, &frame,
                                         &shortfall, &resources), SR_ERR_RENDER);
            CHECK_INT(t, resources.failure_owner.line, 77);
            for (size_t i = 0; i < 16 * 16 * 4; ++i) CHECK_NEAR(t, frame.px[i], 0, 0);
            CHECK_INT(t, resources.bytes, 0);
        }
        sr_compositor_free(&compositor);
        sr_frame_free(&frame);
        sr_scene_free(&scene);
    }
}

static void invalid_physics_precedes_sampling(sr_test_ctx *t) {
    SrScene scene;
    bool built = resource_geometry_scene(&scene);
    CHECK(t, built);
    if (!built) { sr_scene_free(&scene); return; }
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
    SrNode *node = scene.root->children[0];
    SrSoftBody saved = node->soft_body;
    double step = scene.physics.fixed_step;
    SrCompositor compositor;
    sr_compositor_init(&compositor, 1);
    SrFrame frame = {0};
    CHECK_INT(t, sr_frame_init(&frame, 64, 64), SR_OK);
    for (unsigned test = 0; test < 6; ++test) {
        sr_scene_invalidate_compositing(&scene);
        node->soft_body = saved;
        scene.physics.fixed_step = step;
        if (test == 0) scene.physics.fixed_step = NAN;
        if (test == 1) scene.physics.fixed_step = 0;
        if (test == 2) node->soft_body.rows = UINT32_MAX;
        if (test == 3) node->soft_body.offsets = NULL;
        if (test == 4) node->soft_body.sample_count = SIZE_MAX;
        scene.has_relative_lengths = test != 5;
        if (test == 5) scene.physics.fixed_step = -1;
        CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
        SrCompositeResources resources;
        CHECK_INT(t, bounded_render(&compositor, &scene, 0, &frame, NULL, &resources),
                  SR_ERR_RENDER);
        CHECK_INT(t, resources.bytes, 0);
        CHECK_INT(t, resources.failure_owner.line, node->source_line);
        CHECK(t, strcmp(resources.failure_owner.element, "softBody") == 0);
    }
    sr_scene_invalidate_compositing(&scene);
    node->soft_body = saved;
    scene.physics.fixed_step = step;
    scene.has_relative_lengths = true;
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
    SrCompositeResources resources;
    CHECK_INT(t, bounded_render(&compositor, &scene, 0, &frame, NULL, &resources), SR_OK);
    sr_compositor_free(&compositor);
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

const sr_test_case sr_tests_composite_geometry[] = {
    {"declared_work_bounds", declared_work_bounds},
    {"geometry_ownership_and_history", geometry_ownership_and_history},
    {"length_growth_and_zero_work", length_growth_and_zero_work},
    {"grid_fallback_reserved_before_pixels", grid_fallback_reserved_before_pixels},
    {"invalid_physics_precedes_sampling", invalid_physics_precedes_sampling},
    {NULL, NULL}
};
