/* SPDX-License-Identifier: Apache-2.0 */
#include "resource_fixture.h"
#include "compositing_internal.h"
#include "particles_internal.h"

#include <stdlib.h>

static SrStatus render(SrCompositor *compositor, SrScene *scene, double time,
                        SrFrame *frame, const SrCompositeLimits *limits,
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

static void cache_capacity_and_metadata(sr_test_ctx *t) {
    SrKeyframe keys[3] = {{.time = 0}, {.time = 5}, {.time = 10}};
    SrNode node = {.type = SR_NODE_PARTICLES, .particle_max = 1};
    uint64_t empty, two, three;
    CHECK(t, sr_particles_cache_bound(&node, &empty));
    CHECK_INT(t, empty, 0);
    node.particle_rate.track = (SrTrack){.keys = keys, .count = 2};
    keys[1].time = 10;
    CHECK(t, sr_particles_cache_bound(&node, &two));
    keys[1].time = 5;
    node.particle_rate.track.count = 3;
    CHECK(t, sr_particles_cache_bound(&node, &three));
    /* One additional slot holds three u64s, one total and one checkpoint. */
    CHECK_INT(t, three - two, 3 * sizeof(uint64_t) + 2 * sizeof(double));
    node.particle_max = SR_MAX_PARTICLE_OUTPUT + 1;
    CHECK(t, !sr_particles_cache_bound(&node, &empty));
    node.particle_max = 1;
    node.particle_rate.track.count = SR_MAX_TRACK_KEYS + 1;
    CHECK(t, !sr_particles_cache_bound(&node, &empty));
    node.particle_rate.track.count = 1;
    node.particle_rate.track.keys = NULL;
    CHECK(t, !sr_particles_cache_bound(&node, &empty));
    node.particle_rate.track.keys = keys;
    node.start_time = NAN;
    CHECK(t, !sr_particles_cache_bound(&node, &empty));
    node.start_time = 0;
    keys[0].time = SR_MAX_ANIMATION_TIME + 1;
    CHECK(t, !sr_particles_cache_bound(&node, &empty));
    keys[0].time = 0;
    node.particle_lifetime.track.count = 1;
    CHECK(t, !sr_particles_cache_bound(&node, &empty));
    node.particle_lifetime.track.count = 0;
    node.id = sr_alloc(SR_MAX_PARTICLE_ID_BYTES + 2);
    CHECK(t, node.id != NULL);
    if (node.id) {
        memset(node.id, 'x', SR_MAX_PARTICLE_ID_BYTES + 1);
        CHECK(t, !sr_particles_cache_bound(&node, &empty));
        node.id[SR_MAX_PARTICLE_ID_BYTES] = 0;
        CHECK(t, sr_particles_cache_bound(&node, &empty));
        free(node.id);
    }
}

static void cache_history_case(sr_test_ctx *t, bool extended) {
    SrScene scene;
    bool built = resource_particle_scene(&scene);
    CHECK(t, built);
    if (!built) { sr_scene_free(&scene); return; }
    SrNode *node = scene.root->children[0];
    if (extended) {
        node->particle_rate.track.clock_set = true;
        node->particle_rate.track.clock_scale = .5;
        node->particle_rate.track.domain_end = 24;
        CHECK_INT(t, sr_track_finalize(&node->particle_rate.track), SR_OK);
        for (unsigned i = 0; i < 2; ++i) {
            SrKeyframe key = {.time = i * 24, .value = i ? 3 : 5,
                              .curve = SR_CURVE_CATMULL_ROM};
            CHECK_INT(t, sr_track_add(&node->particle_lifetime.track, key), SR_OK);
            key.curve = SR_CURVE_LINEAR;
            CHECK_INT(t, sr_anim_color_add_key(&node->particle_color, key,
                (SrColor){.8, i ? .8 : .2, .2, .3}), SR_OK);
        }
        CHECK_INT(t, sr_track_finalize(&node->particle_lifetime.track), SR_OK);
        CHECK_INT(t, sr_anim_color_finalize(&node->particle_color), SR_OK);
    }
    SrFrame legacy = {0}, frame = {0};
    CHECK_INT(t, sr_frame_init(&legacy, 96, 96), SR_OK);
    CHECK_INT(t, sr_frame_init(&frame, 96, 96), SR_OK);
    SrCompositor compositor;
    sr_compositor_init(&compositor, 1);
    CHECK_INT(t, sr_compositor_render(&compositor, &scene, 12.5, &legacy, NULL), SR_OK);
    CHECK(t, node->particle_rate_cache != NULL);
    compositor.threads = 4;
    CHECK_INT(t, sr_compositor_render(&compositor, &scene, 12.5, &frame, NULL), SR_OK);
    size_t bytes = (size_t)96 * 96 * 4 * sizeof(float);
    CHECK(t, memcmp(legacy.px, frame.px, bytes) == 0);
    uint64_t cache_bytes;
    CHECK(t, sr_particles_cache_bound(node, &cache_bytes));
    SrCompositePlan *structural = NULL;
    CHECK_INT(t, sr_composite_plan_build(&scene, &structural, NULL), SR_OK);
    uint64_t structural_bytes = structural ? structural->owned_bytes : 0;
    sr_composite_plan_free(structural);
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
    CHECK(t, node->particle_rate_cache == NULL);
    CHECK_INT(t, scene.compositing->owned_bytes, structural_bytes + cache_bytes);
    SrCompositeResources first, next;
    CHECK_INT(t, render(&compositor, &scene, 12.5, &frame, NULL, &first), SR_OK);
    CHECK(t, memcmp(legacy.px, frame.px, bytes) == 0);
    SrCompositeLimits exact = {first.peak_bytes, first.peak_pixels, first.work};
    const double warm[] = {23, 0, 8.1, 19};
    for (size_t i = 0; i < 4; ++i) {
        compositor.threads = i % 2 ? 4 : 1;
        CHECK_INT(t, render(&compositor, &scene, warm[i], &frame, NULL, &next), SR_OK);
        CHECK_INT(t, render(&compositor, &scene, 12.5, &frame, &exact, &next), SR_OK);
        CHECK_INT(t, next.peak_bytes, first.peak_bytes);
        CHECK_INT(t, next.peak_pixels, first.peak_pixels);
        CHECK_INT(t, next.work, first.work);
        CHECK_INT(t, next.bytes, 0);
        CHECK(t, memcmp(legacy.px, frame.px, bytes) == 0);
    }
    for (unsigned cold = 0; cold < 2; ++cold) {
        if (cold) sr_particles_invalidate(node);
        SrCompositeLimits low = exact;
        --low.work;
        CHECK_INT(t, render(&compositor, &scene, 12.5, &frame, &low, &next), SR_ERR_RENDER);
        CHECK_INT(t, next.failure_owner.line, 91);
        CHECK_INT(t, next.bytes, 0);
        low = exact;
        --low.bytes;
        CHECK_INT(t, render(&compositor, &scene, 12.5, &frame, &low, &next), SR_ERR_RENDER);
        CHECK_INT(t, next.bytes, 0);
        CHECK_INT(t, render(&compositor, &scene, 12.5, &frame, &exact, &next), SR_OK);
    }
    sr_compositor_free(&compositor);
    sr_frame_free(&legacy);
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

static void cache_history_and_threads(sr_test_ctx *t) {
    cache_history_case(t, false);
    cache_history_case(t, true);
}

static bool replace_rate(SrNode *node) {
    sr_track_free(&node->particle_rate.track);
    node->particle_rate.base = 10;
    return sr_track_add(&node->particle_rate.track,
        (SrKeyframe){.time = 0, .value = 10, .curve = SR_CURVE_LINEAR}) == SR_OK &&
        sr_track_finalize(&node->particle_rate.track) == SR_OK;
}

static void invalidate_edit_and_shrink(sr_test_ctx *t) {
    SrScene edited, fresh;
    bool a = resource_particle_scene(&edited), b = resource_particle_scene(&fresh);
    CHECK(t, a && b);
    if (!a || !b) { sr_scene_free(&edited); sr_scene_free(&fresh); return; }
    SrNode *node = edited.root->children[0];
    CHECK_INT(t, sr_scene_prepare_compositing(&edited, NULL), SR_OK);
    uint64_t before = edited.compositing->owned_bytes;
    SrCompositor compositor;
    sr_compositor_init(&compositor, 4);
    SrFrame left = {0}, right = {0};
    CHECK_INT(t, sr_frame_init(&left, 96, 96), SR_OK);
    CHECK_INT(t, sr_frame_init(&right, 96, 96), SR_OK);
    SrCompositeResources one, two;
    CHECK_INT(t, render(&compositor, &edited, 19, &left, NULL, &one), SR_OK);
    CHECK(t, node->particle_rate_cache != NULL);
    sr_scene_invalidate_compositing(&edited);
    CHECK(t, node->particle_rate_cache == NULL);
    CHECK(t, replace_rate(node));
    CHECK(t, replace_rate(fresh.root->children[0]));
    CHECK_INT(t, sr_scene_prepare_compositing(&edited, NULL), SR_OK);
    CHECK_INT(t, sr_scene_prepare_compositing(&fresh, NULL), SR_OK);
    CHECK(t, edited.compositing->owned_bytes < before);
    CHECK_INT(t, edited.compositing->owned_bytes, fresh.compositing->owned_bytes);
    CHECK_INT(t, render(&compositor, &edited, 12.5, &left, NULL, &one), SR_OK);
    SrCompositeLimits exact = {one.peak_bytes, one.peak_pixels, one.work};
    CHECK_INT(t, render(&compositor, &fresh, 12.5, &right, &exact, &two), SR_OK);
    CHECK_INT(t, one.work, two.work);
    CHECK(t, memcmp(left.px, right.px, 96 * 96 * 4 * sizeof(float)) == 0);
    sr_compositor_free(&compositor);
    sr_frame_free(&left);
    sr_frame_free(&right);
    sr_scene_free(&edited);
    sr_scene_free(&fresh);
}

static void cold_work_without_particles(sr_test_ctx *t) {
    SrScene scene;
    fx_scene(&scene, 16, 16);
    SrNode *node = fx_add(&scene, NULL, SR_NODE_PARTICLES);
    CHECK(t, node != NULL);
    if (!node) { sr_scene_free(&scene); return; }
    CHECK_INT(t, sr_track_add(&node->particle_rate.track,
        (SrKeyframe){.time = 0, .value = 0, .curve = SR_CURVE_LINEAR}), SR_OK);
    CHECK_INT(t, sr_track_add(&node->particle_rate.track,
        (SrKeyframe){.time = 20, .value = 0, .curve = SR_CURVE_LINEAR}), SR_OK);
    CHECK_INT(t, sr_track_finalize(&node->particle_rate.track), SR_OK);
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
    uint64_t work[3];
    for (size_t i = 0; i < 3; ++i) {
        SrCompositeResources r;
        sr_composite_resources_init(&r, NULL);
        CHECK(t, sr_composite_reserve(&r, scene.compositing->owned_bytes, 0, 0));
        SrParticle *particles = NULL;
        size_t count;
        CHECK_INT(t, sr_particles_eval_composite(&scene, node, i ? 10 : 0,
            &r, &particles, &count), SR_OK);
        CHECK_INT(t, count, 0);
        CHECK(t, particles == NULL);
        sr_composite_release(&r, scene.compositing->owned_bytes, 0);
        CHECK_INT(t, r.bytes, 0);
        work[i] = r.work;
    }
    /* 2400 additional grid cells, three rate passes plus dispatch headroom,
     * and two additional doubles in the temporary checkpoint allocation. */
    CHECK_INT(t, work[1] - work[0], 2400 * 4 * 80 + 2 * sizeof(double) / 4);
    CHECK_INT(t, work[2], work[1]);
    sr_particles_invalidate(node);
    SrCompositeLimits limits = {SR_MAX_COMPOSITE_BYTES, SR_MAX_COMPOSITE_PIXELS, 1};
    SrCompositeResources r;
    sr_composite_resources_init(&r, &limits);
    SrParticle *particles = NULL;
    size_t count;
    CHECK_INT(t, sr_particles_eval_composite(&scene, node, 10, &r,
        &particles, &count), SR_ERR_RENDER);
    CHECK(t, node->particle_rate_cache == NULL);
    CHECK(t, particles == NULL);
    CHECK_INT(t, r.bytes, 0);
    sr_scene_free(&scene);
}


static void emission_index_failures(sr_test_ctx *t) {
    for (unsigned mode = 0; mode < 3; ++mode) {
        SrScene scene;
        fx_scene(&scene, 16, 16);
        SrNode *node = fx_add(&scene, NULL, SR_NODE_PARTICLES);
        CHECK(t, node != NULL);
        if (!node) { sr_scene_free(&scene); return; }
        node->source_line = 113;
        node->particle_rate.base = 1e16;
        node->particle_max = 1;
        double time = mode == 1 ? 2 : 1;
        if (mode) {
            for (unsigned k = 0; k < 2; ++k)
                CHECK_INT(t, sr_track_add(&node->particle_rate.track, (SrKeyframe){
                    .time = k + (mode == 2 ? 2 : 0), .value = 1e16,
                    .curve = SR_CURVE_LINEAR}), SR_OK);
            CHECK_INT(t, sr_track_finalize(&node->particle_rate.track), SR_OK);
        }
        SrParticle *particles = NULL;
        size_t count;
        CHECK_INT(t, sr_particles_eval(&scene, node, time, &particles, &count), SR_OK);
        CHECK_INT(t, count, 0);
        free(particles);
        CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
        SrCompositeResources r;
        sr_composite_resources_init(&r, NULL);
        CHECK(t, sr_composite_reserve(&r, scene.compositing->owned_bytes, 0, 0));
        CHECK_INT(t, sr_particles_eval_composite(&scene, node, time, &r,
            &particles, &count), SR_ERR_RENDER);
        CHECK_INT(t, r.status, SR_ERR_RENDER);
        CHECK_INT(t, r.failure_owner.line, 113);
        CHECK(t, particles == NULL);
        sr_composite_release(&r, scene.compositing->owned_bytes, 0);
        CHECK_INT(t, r.bytes, 0);
        sr_scene_free(&scene);
    }
}

static void candidate_work_and_failure_cleanup(sr_test_ctx *t) {
    SrScene scene;
    fx_scene(&scene, 16, 16);
    SrNode *node = fx_add(&scene, NULL, SR_NODE_PARTICLES);
    CHECK(t, node != NULL);
    if (!node) { sr_scene_free(&scene); return; }
    node->particle_rate.base = 1;
    node->particle_lifetime.base = 10;
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
    uint64_t work[2];
    for (unsigned i = 0; i < 2; ++i) {
        SrCompositeResources r;
        sr_composite_resources_init(&r, NULL);
        SrParticle *particles = NULL;
        size_t count;
        CHECK_INT(t, sr_particles_eval_composite(&scene, node, i, &r,
            &particles, &count), SR_OK);
        CHECK_INT(t, count, i + 1);
        work[i] = r.work;
        sr_composite_free(&r, particles);
        CHECK_INT(t, r.bytes, 0);
    }
    /* One additional candidate: five scalars, eight color-channel bounds,
     * fixed particle math and record copy; one more reverse/draw record. */
    CHECK_INT(t, work[1] - work[0], 5 + 8 + 64 + sizeof(SrParticle) / 4 +
        3 * sizeof(SrParticle) / 4 + 64);
    SrCompositeLimits limits = {SR_MAX_COMPOSITE_BYTES, SR_MAX_COMPOSITE_PIXELS,
        work[1] - 1};
    SrCompositeResources r;
    sr_composite_resources_init(&r, &limits);
    SrParticle *particles = NULL;
    size_t count;
    CHECK_INT(t, sr_particles_eval_composite(&scene, node, 1, &r,
        &particles, &count), SR_ERR_RENDER);
    CHECK(t, particles == NULL);
    CHECK_INT(t, count, 0);
    CHECK_INT(t, r.bytes, 0);
    const double invalid_times[] = {NAN, SR_MAX_DURATION + 1, -SR_MAX_DURATION - 1};
    for (size_t i = 0; i < 3; ++i) {
        sr_composite_resources_init(&r, NULL);
        CHECK_INT(t, r.status, SR_OK);
        CHECK_INT(t, sr_particles_eval_composite(&scene, node, invalid_times[i], &r,
            &particles, &count), SR_ERR_RENDER);
        CHECK_INT(t, r.status, SR_ERR_RENDER);
        CHECK(t, particles == NULL);
        CHECK_INT(t, r.bytes, 0);
    }
    sr_scene_free(&scene);
}

static void bounded_id_diagnostic(sr_test_ctx *t) {
    SrScene scene;
    fx_scene(&scene, 16, 16);
    SrNode *node = fx_add(&scene, NULL, SR_NODE_PARTICLES);
    CHECK(t, node != NULL);
    if (!node) { sr_scene_free(&scene); return; }
    node->source_line = 123;
    node->id = sr_alloc(SR_MAX_PARTICLE_ID_BYTES + 2);
    CHECK(t, node->id != NULL);
    if (node->id) {
        memset(node->id, 'x', SR_MAX_PARTICLE_ID_BYTES + 1);
        FILE *sink = tmpfile();
        CHECK(t, sink != NULL);
        if (sink) {
            SrDiagnostics diag;
            sr_diag_init(&diag, "particles", sink);
            CHECK_INT(t, sr_scene_prepare_compositing(&scene, &diag), SR_ERR_RENDER);
            CHECK(t, scene.compositing == NULL);
            fflush(sink);
            CHECK(t, ftell(sink) > 0 && ftell(sink) < 512);
            fclose(sink);
        }
    }
    sr_scene_free(&scene);
}


static void aggregate_cache_limit(sr_test_ctx *t) {
    SrScene scene;
    fx_scene(&scene, 16, 16);
    SrNode *first = fx_add(&scene, NULL, SR_NODE_PARTICLES);
    CHECK(t, first != NULL);
    if (!first) { sr_scene_free(&scene); return; }
    SrKeyframe *keys = sr_alloc(SR_MAX_TRACK_KEYS * sizeof(*keys));
    CHECK(t, keys != NULL);
    if (!keys) { sr_scene_free(&scene); return; }
    for (size_t i = 0; i < SR_MAX_TRACK_KEYS; ++i)
        keys[i].time = (double)i * SR_MAX_DURATION / (SR_MAX_TRACK_KEYS - 1);
    first->particle_rate.track = (SrTrack){.keys = keys, .count = SR_MAX_TRACK_KEYS,
                                         .capacity = SR_MAX_TRACK_KEYS};
    CHECK_INT(t, sr_track_finalize(&first->particle_rate.track), SR_OK);
    uint64_t cache_bytes;
    CHECK(t, sr_particles_cache_bound(first, &cache_bytes));
    SrParticle *particles = NULL;
    size_t count;
    CHECK_INT(t, sr_particles_eval(&scene, first, 0, &particles, &count), SR_OK);
    free(particles);
    CHECK(t, first->particle_rate_cache != NULL);
    size_t copies = (size_t)(SR_MAX_COMPOSITE_BYTES / cache_bytes) + 1;
    for (size_t i = 1; i < copies; ++i) {
        SrNode *node = fx_add(&scene, NULL, SR_NODE_PARTICLES);
        CHECK(t, node != NULL);
        if (!node) break;
        /* Test-only shared immutable storage avoids allocating gigabytes of
         * keys; restore sole ownership before ordinary scene destruction. */
        node->particle_rate.track = first->particle_rate.track;
    }
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_ERR_RENDER);
    CHECK(t, scene.compositing == NULL);
    CHECK(t, first->particle_rate_cache != NULL);
    for (size_t i = 1; i < scene.root->child_count; ++i)
        scene.root->children[i]->particle_rate.track = (SrTrack){0};
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
    CHECK(t, first->particle_rate_cache == NULL);
    sr_scene_free(&scene);
}

const sr_test_case sr_tests_composite_particles[] = {
    {"cache_capacity_and_metadata", cache_capacity_and_metadata},
    {"cache_history_and_threads", cache_history_and_threads},
    {"invalidate_edit_and_shrink", invalidate_edit_and_shrink},
    {"cold_work_without_particles", cold_work_without_particles},
    {"emission_index_failures", emission_index_failures},
    {"candidate_work_and_failure_cleanup", candidate_work_and_failure_cleanup},
    {"bounded_id_diagnostic", bounded_id_diagnostic},
    {"aggregate_cache_limit", aggregate_cache_limit},
    {NULL, NULL}
};
