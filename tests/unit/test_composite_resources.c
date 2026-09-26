/* SPDX-License-Identifier: Apache-2.0 */
#include "fixture.h"
#include "resource_fixture.h"
#include "compositing_internal.h"
#include "compositor_internal.h"
#include "compositor_resources_internal.h"

static void atomic_limits(sr_test_ctx *t) {
    SrCompositeResources r;
    SrCompositeLimits unlimited = {UINT64_MAX, UINT64_MAX, UINT64_MAX};
    sr_composite_resources_init(&r, &unlimited);
    CHECK_INT(t, r.limits.bytes, SR_MAX_COMPOSITE_BYTES);
    CHECK_INT(t, r.limits.pixels, SR_MAX_COMPOSITE_PIXELS);
    CHECK_INT(t, r.limits.work, SR_MAX_COMPOSITE_WORK);
    CHECK(t, sr_composite_reserve(&r, r.limits.bytes, r.limits.pixels, r.limits.work));
    SrCompositeOwner owner = {37, "shape", "blend"};
    sr_composite_owner(&r, owner);
    CHECK(t, !sr_composite_reserve(&r, 1, 0, 0));
    CHECK_INT(t, r.bytes, r.limits.bytes);
    CHECK_INT(t, r.pixels, r.limits.pixels);
    CHECK_INT(t, r.work, r.limits.work);
    CHECK_INT(t, r.status, SR_ERR_RENDER);
    sr_composite_owner(&r, (SrCompositeOwner){1, "composition", NULL});
    CHECK(t, !sr_composite_resource_fail(&r, SR_ERR_MEMORY, "later failure"));
    CHECK_INT(t, r.failure_owner.line, 37);
    CHECK(t, strcmp(r.failure_owner.attribute, "blend") == 0);
    sr_composite_release(&r, r.bytes, r.pixels);
    CHECK_INT(t, r.bytes, 0);
    CHECK_INT(t, r.work, r.limits.work);
    CHECK(t, !sr_composite_reserve(&r, 0, 0, 0));

    const SrCompositeLimits small = {16, 4, 8};
    for (unsigned kind = 0; kind < 3; ++kind) {
        sr_composite_resources_init(&r, &small);
        CHECK(t, sr_composite_reserve(&r, 8, 2, 4));
        CHECK(t, !sr_composite_reserve(&r, kind == 0 ? 9 : 1,
            kind == 1 ? 3 : 1, kind == 2 ? 5 : 1));
        CHECK_INT(t, r.bytes, 8);
        CHECK_INT(t, r.pixels, 2);
        CHECK_INT(t, r.work, 4);
    }
    sr_composite_resources_init(&r, NULL);
    CHECK(t, !sr_composite_work(&r, UINT64_MAX, 2));
    CHECK_INT(t, r.work, 0);
    CHECK_INT(t, sr_composite_resource_status(&r), SR_ERR_RENDER);
    CHECK(t, sr_composite_work(NULL, UINT64_MAX, UINT64_MAX));
}

static void allocation_ownership(sr_test_ctx *t) {
    SrCompositeResources r;
    sr_composite_resources_init(&r, NULL);
    unsigned char *p = sr_composite_alloc(&r, 16, 1, 4);
    CHECK(t, p != NULL);
    if (!p) return;
    CHECK_INT(t, (uintptr_t)p % _Alignof(max_align_t), 0);
    for (size_t i = 0; i < 16; ++i) CHECK_INT(t, p[i], 0);
    p[0] = 91;
    uint64_t old_bytes = r.bytes;
    p = sr_composite_realloc(&r, p, 32, 1, 8);
    CHECK(t, p != NULL);
    if (!p) return;
    CHECK_INT(t, p[0], 91);
    CHECK_INT(t, r.peak_bytes, old_bytes + r.bytes);
    CHECK_INT(t, r.peak_pixels, 12);
    sr_composite_free(&r, p);
    CHECK_INT(t, r.bytes, 0);
    CHECK_INT(t, r.pixels, 0);
    CHECK(t, r.work > 0);

    SrCompositeLimits small = {old_bytes * 2 + 15, 100, 1000};
    sr_composite_resources_init(&r, &small);
    p = sr_composite_alloc(&r, 16, 1, 4);
    CHECK(t, p != NULL);
    if (p) {
        p[0] = 71;
        CHECK(t, sr_composite_realloc(&r, p, 32, 1, 8) == NULL);
        CHECK_INT(t, p[0], 71);
        CHECK_INT(t, r.bytes, old_bytes);
        sr_composite_free(&r, p);
    }
    sr_composite_resources_init(&r, NULL);
    CHECK(t, sr_composite_alloc(&r, SIZE_MAX, 2, 0) == NULL);
    CHECK_INT(t, r.bytes, 0);
    CHECK_INT(t, r.status, SR_ERR_RENDER);
    sr_composite_resources_init(&r, NULL);
    CHECK(t, sr_composite_alloc(&r, SIZE_MAX, 1, 0) == NULL);
    CHECK_INT(t, r.status, SR_ERR_RENDER);
    sr_composite_resources_init(&r, NULL);
    p = sr_composite_alloc(&r, 0, 0, 0);
    CHECK(t, p != NULL && r.bytes > 0);
    sr_composite_free(&r, p);
    sr_composite_free(&r, NULL);
    p = sr_composite_alloc(NULL, 4, 1, 0);
    CHECK(t, p != NULL);
    p = sr_composite_realloc(NULL, p, 8, 1, 0);
    CHECK(t, p != NULL);
    sr_composite_free(NULL, p);
}

static bool build_scene(SrScene *scene) {
    fx_scene(scene, 64, 64);
    SrNode *group = fx_add(scene, NULL, SR_NODE_GROUP);
    if (!group) return false;
    group->blend = SR_BLEND_COLOR_BURN;
    group->source_line = 23;
    for (size_t i = 0; i < 70; ++i) {
        SrNode *node = fx_rect(scene, group, i % 8, i % 5, 48, 48,
                               (SrColor){0.8, 0.5, 0.2, 0.1}, 1);
        if (!node) return false;
        node->source_line = 30 + i;
    }
    group->masks = sr_alloc(9 * sizeof(*group->masks));
    if (!group->masks) return false;
    group->mask_count = 9;
    for (size_t i = 0; i < 9; ++i)
        group->masks[i] = fx_mask(SR_MASK_RECT, 0, 0, 64, 64, false);
    return sr_scene_prepare_compositing(scene, NULL) == SR_OK;
}

static SrStatus render_quota(SrCompositor *compositor, SrScene *scene,
                              SrFrame *frame, const SrCompositeLimits *limits,
                              SrCompositeResources *result) {
    sr_frame_clear(frame, (float[4]){0, 0, 0, 0}, 1);
    SrCompositeScope scope;
    SrStatus status = sr_composite_scope_begin(&scope, compositor, scene, frame, limits);
    if (status == SR_OK)
        status = sr_compositor_render(compositor, scene, 0, frame, NULL);
    status = sr_composite_scope_end(&scope, compositor, status, NULL);
    *result = scope.resources;
    return status;
}

static void render_limits_and_history(sr_test_ctx *t) {
    SrScene scene;
    bool built = build_scene(&scene);
    CHECK(t, built);
    if (!built) { sr_scene_free(&scene); return; }
    SrFrame one = {0}, four = {0};
    CHECK_INT(t, sr_frame_init(&one, 64, 64), SR_OK);
    CHECK_INT(t, sr_frame_init(&four, 64, 64), SR_OK);
    SrCompositor compositor;
    sr_compositor_init(&compositor, 1);
    SrCompositeResources first, again;
    CHECK_INT(t, render_quota(&compositor, &scene, &one, NULL, &first), SR_OK);
    CHECK_INT(t, first.bytes, 0);
    CHECK_INT(t, first.pixels, 0);
    CHECK(t, compositor.resources == NULL && compositor.pool == NULL);
    SrCompositeLimits exact = {first.peak_bytes, first.peak_pixels, first.work};
    compositor.threads = 4;
    CHECK_INT(t, render_quota(&compositor, &scene, &four, &exact, &again), SR_OK);
    CHECK_INT(t, again.peak_bytes, first.peak_bytes);
    CHECK_INT(t, again.peak_pixels, first.peak_pixels);
    CHECK_INT(t, again.work, first.work);
    CHECK(t, memcmp(one.px, four.px, 64 * 64 * 4 * sizeof(float)) == 0);
    for (unsigned kind = 0; kind < 3; ++kind) {
        SrCompositeLimits shortfall = exact;
        if (kind == 0) --shortfall.bytes;
        if (kind == 1) --shortfall.pixels;
        if (kind == 2) --shortfall.work;
        CHECK_INT(t, render_quota(&compositor, &scene, &four, &shortfall, &again),
                  SR_ERR_RENDER);
        CHECK_INT(t, again.bytes, 0);
        CHECK_INT(t, again.pixels, 0);
        CHECK(t, again.failure_owner.element != NULL);
        CHECK_INT(t, compositor.threads, 4);
    }
    /* Larger legacy pools must not influence the next bounded admission. */
    SrScene legacy;
    fx_scene(&legacy, 128, 96);
    SrNode *group = fx_add(&legacy, NULL, SR_NODE_GROUP);
    if (group) group->opacity.base = 0.5;
    CHECK(t, fx_rect(&legacy, group, 0, 0, 128, 96,
                      (SrColor){1, 0, 0, 1}, 1) != NULL);
    SrFrame large = {0};
    CHECK_INT(t, sr_frame_init(&large, 128, 96), SR_OK);
    CHECK_INT(t, sr_compositor_render(&compositor, &legacy, 0, &large, NULL), SR_OK);
    CHECK(t, compositor.pool != NULL);
    CHECK_INT(t, render_quota(&compositor, &scene, &four, &exact, &again), SR_OK);
    CHECK_INT(t, again.peak_bytes, first.peak_bytes);
    CHECK_INT(t, again.work, first.work);
    CHECK(t, memcmp(one.px, four.px, 64 * 64 * 4 * sizeof(float)) == 0);
    sr_frame_free(&large);
    sr_scene_free(&legacy);
    sr_compositor_free(&compositor);
    sr_frame_free(&one);
    sr_frame_free(&four);
    sr_scene_free(&scene);
}

static void target_preflight_and_diagnostic(sr_test_ctx *t) {
    SrScene scene;
    fx_scene(&scene, 16, 16);
    scene.root->source_line = 12;
    scene.root->transform.skew_x.base = 1;
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
    float sentinel = 73;
    SrFrame frame = {UINT32_MAX, 1, &sentinel};
    SrCompositor compositor;
    sr_compositor_init(&compositor, 4);
    FILE *sink = tmpfile();
    SrDiagnostics diag;
    sr_diag_init(&diag, "resources", sink ? sink : stderr);
    CHECK_INT(t, sr_compositor_render(&compositor, &scene, 0, &frame, &diag),
              SR_ERR_RENDER);
    CHECK_INT(t, diag.errors, 1);
    CHECK_NEAR(t, sentinel, 73, 0);
    CHECK(t, compositor.resources == NULL);
    if (sink) {
        rewind(sink);
        char message[512] = {0};
        (void)fread(message, 1, sizeof(message) - 1, sink);
        CHECK(t, strstr(message, "12") && strstr(message, "composition") &&
                 strstr(message, "width/height"));
        fclose(sink);
    }
    frame.width = 0;
    CHECK_INT(t, sr_compositor_render_scene(&compositor, &scene, 0, &frame, NULL),
              SR_ERR_RENDER);
    sr_compositor_free(&compositor);
    sr_scene_free(&scene);
}

static void borrowed_depth_survives_scope(sr_test_ctx *t) {
    SrScene scene;
    fx_scene(&scene, 16, 16);
    SrNode *card = fx_rect(&scene, NULL, 0, 0, 16, 16,
                            (SrColor){1, 0, 0, 1}, 1);
    CHECK(t, card != NULL);
    if (!card) { sr_scene_free(&scene); return; }
    card->card = true;
    double samples[16 * 16];
    for (size_t i = 0; i < 16 * 16; ++i) samples[i] = -1;
    SrDepthBuffer depth = {samples, 16, 16, 1};
    SrCompositor compositor;
    sr_compositor_init(&compositor, 1);
    compositor.depth = &depth;
    SrFrame frame = {0};
    CHECK_INT(t, sr_frame_init(&frame, 16, 16), SR_OK);
    CHECK_INT(t, sr_compositor_render(&compositor, &scene, 0, &frame, NULL), SR_OK);
    CHECK_NEAR(t, fx_px(&frame, 8, 8)[0], 0, 0);
    card->transform.skew_x.base = 1;
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
    SrCompositeResources resources;
    CHECK_INT(t, render_quota(&compositor, &scene, &frame, NULL, &resources), SR_OK);
    CHECK(t, compositor.depth == &depth);
    CHECK_NEAR(t, fx_px(&frame, 8, 8)[0], 0, 0);
    for (size_t i = 0; i < 16 * 16; ++i) CHECK_NEAR(t, samples[i], -1, 0);
    SrCompositeLimits tight = {scene.compositing->owned_bytes +
        16 * 16 * (4 * sizeof(float) + sizeof(double)) - 1,
        SR_MAX_COMPOSITE_PIXELS, SR_MAX_COMPOSITE_WORK};
    CHECK_INT(t, render_quota(&compositor, &scene, &frame, &tight, &resources),
              SR_ERR_RENDER);
    CHECK_INT(t, resources.peak_bytes, 0);
    CHECK(t, compositor.depth == &depth);
    for (unsigned invalid = 0; invalid < 4; ++invalid) {
        depth.samples = invalid == 0 ? 0 : invalid == 1 ? 5 : 1;
        depth.width = invalid == 2 ? 15 : 16;
        depth.z = invalid == 3 ? NULL : samples;
        CHECK_INT(t, render_quota(&compositor, &scene, &frame, NULL, &resources),
                  SR_ERR_RENDER);
        CHECK(t, strcmp(resources.failure_owner.attribute, "depth") == 0);
        CHECK(t, compositor.depth == &depth);
    }
    depth.samples = 1;
    depth.width = 16;
    depth.z = samples;
    CHECK_INT(t, render_quota(&compositor, &scene, &frame, NULL, &resources), SR_OK);
    sr_compositor_free(&compositor);
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

static void mask_copy_work_includes_both_passes(sr_test_ctx *t) {
    uint64_t work[2] = {0};
    for (unsigned masked = 0; masked < 2; ++masked) {
        SrScene scene;
        fx_scene(&scene, 16, 16);
        SrNode *group = fx_add(&scene, NULL, SR_NODE_GROUP);
        SrNode *leaf = fx_rect(&scene, group, 0, 0, 16, 16,
                                (SrColor){1, 0, 0, 1}, 1);
        CHECK(t, group && leaf);
        if (!group || !leaf) { sr_scene_free(&scene); return; }
        if (masked) {
            group->masks = sr_alloc(sizeof(*group->masks));
            leaf->masks = sr_alloc(9 * sizeof(*leaf->masks));
            CHECK(t, group->masks && leaf->masks);
            if (!group->masks || !leaf->masks) { sr_scene_free(&scene); return; }
            group->mask_count = 1;
            leaf->mask_count = 9;
            group->masks[0] = fx_mask(SR_MASK_RECT, 0, 0, 16, 16, false);
            for (size_t i = 0; i < 9; ++i) leaf->masks[i] = group->masks[0];
        }
        CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
        SrCompositor compositor;
        sr_compositor_init(&compositor, 1);
        SrFrame frame = {0};
        CHECK_INT(t, sr_frame_init(&frame, 16, 16), SR_OK);
        SrCompositeResources resources;
        CHECK_INT(t, render_quota(&compositor, &scene, &frame, NULL, &resources), SR_OK);
        work[masked] = resources.work;
        sr_compositor_free(&compositor);
        sr_frame_free(&frame);
        sr_scene_free(&scene);
    }
    SrCompositeResources probe;
    sr_composite_resources_init(&probe, NULL);
    void *p = sr_composite_alloc(&probe, 1, 1, 0);
    CHECK(t, p != NULL);
    uint64_t prefix = probe.bytes - 1;
    sr_composite_free(&probe, p);
    uint64_t copies = 2 * sizeof(SrMaskLink) + 10 * sizeof(SrMaskEval);
    uint64_t heap = 9 * sizeof(SrMaskEval);
    /* Ten masks with five scalar evaluations and 16 setup/clip units each,
     * node visits, 12 linked-mask steps per raster pixel, three traversals of
     * two links, one heap array zero, chain zero AND copy. */
    uint64_t extra = 10 * (1 + 5 + 16) + 16 * 16 * 12 + 6 + (heap + prefix + 3) / 4 +
                     (copies + prefix + 3) / 4 + (copies + 3) / 4;
    CHECK_INT(t, work[1] - work[0], extra);
}

static void projected_card_ownership(sr_test_ctx *t) {
    SrScene scene;
    bool built = resource_card_scene(&scene);
    CHECK(t, built);
    if (!built) { sr_scene_free(&scene); return; }
    SrFrame legacy = {0}, bounded = {0};
    CHECK_INT(t, sr_frame_init(&legacy, 64, 64), SR_OK);
    CHECK_INT(t, sr_frame_init(&bounded, 64, 64), SR_OK);
    SrCompositor compositor;
    sr_compositor_init(&compositor, 1);
    CHECK_INT(t, sr_compositor_render_scene(&compositor, &scene, .5, &legacy, NULL), SR_OK);
    CHECK(t, compositor.plane != NULL);
    CHECK(t, compositor.depth_store.z != NULL);
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
    SrCompositeLimits exact = {0};
    for (unsigned run = 0; run < 3; ++run) {
        compositor.threads = run == 0 ? 1 : 4;
        sr_frame_clear(&bounded, (float[4]){0, 0, 0, 0}, 1);
        SrCompositeScope scope;
        SrStatus status = sr_composite_scope_begin(&scope, &compositor,
            &scene, &bounded, run ? &exact : NULL);
        if (status == SR_OK)
            status = sr_compositor_render_scene(&compositor, &scene, .5,
                                                  &bounded, NULL);
        if (run == 0)
            exact = (SrCompositeLimits){scope.resources.peak_bytes,
                scope.resources.peak_pixels, scope.resources.work};
        CHECK_INT(t, sr_composite_scope_end(&scope, &compositor, status, NULL), SR_OK);
        CHECK_INT(t, scope.resources.bytes, 0);
        CHECK_INT(t, scope.resources.pixels, 0);
        CHECK_INT(t, scope.resources.work, exact.work);
        CHECK(t, memcmp(legacy.px, bounded.px, 64 * 64 * 4 * sizeof(float)) == 0);
        CHECK(t, !compositor.plane && !compositor.depth_store.z);
    }
    sr_compositor_free(&compositor);
    sr_frame_free(&legacy);
    sr_frame_free(&bounded);
    sr_scene_free(&scene);
}

const sr_test_case sr_tests_composite_resources[] = {
    {"atomic_limits", atomic_limits},
    {"allocation_ownership", allocation_ownership},
    {"render_limits_and_history", render_limits_and_history},
    {"target_preflight_and_diagnostic", target_preflight_and_diagnostic},
    {"borrowed_depth_survives_scope", borrowed_depth_survives_scope},
    {"mask_copy_work_includes_both_passes", mask_copy_work_includes_both_passes},
    {"projected_card_ownership", projected_card_ownership},
    {NULL, NULL}
};
