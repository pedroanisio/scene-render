/* SPDX-License-Identifier: Apache-2.0 */
#include "fixture.h"
#include "length_frame.h"
#include "scene_render/physics.h"
#include "scene_render/renderer.h"

#include <stdlib.h>
#include <unistd.h>

static void value(SrAnimValue *out, bool relative, double coefficient,
                    SrLengthUnit unit, double pixels) {
    out->base = relative ? coefficient : pixels;
    out->unit = relative ? unit : SR_LENGTH_PIXELS;
}

static bool physics_scene(SrScene *scene, bool relative, uint32_t width,
                           uint32_t height, bool soft_rigid) {
    fx_scene(scene, width, height);
    scene->has_relative_lengths = relative;
    scene->project.duration = .3;
    scene->physics.enabled = true;
    scene->physics.fixed_step = .01;
    scene->physics.gravity_y = 20;
    scene->scene360.enabled = true;
    scene->scene360.width = 2048;
    scene->scene360.height = 1024;
    SrNode *group = fx_add(scene, NULL, SR_NODE_GROUP);
    if (!group) return false;
    group->group_width_set = group->group_height_set = true;
    group->group_width = (SrLength){width / 2.0, SR_LENGTH_PIXELS};
    group->group_height = (SrLength){height / 2.0, SR_LENGTH_PIXELS};
    for (int i = 0; i < 3; ++i) {
        SrNode *node = fx_rect(scene, group, 0, 0, 20, 20,
                               (SrColor){.3, .6, .8, 1}, 1);
        if (!node) return false;
        node->id = sr_strdup(i == 0 ? "a" : i == 1 ? "b" : "soft");
        if (!node->id) return false;
        node->body.type = i == 1 ? SR_BODY_STATIC : SR_BODY_DYNAMIC;
        value(&node->transform.x, relative, i == 0 ? 25 : i == 1 ? 37.5 : 75,
              SR_LENGTH_PERCENT, width * (i == 0 ? 25 : i == 1 ? 37.5 : 75) / 200.0);
        value(&node->transform.y, relative, i == 2 ? 50 : 20, SR_LENGTH_VH,
              height * (i == 2 ? 50 : 20) / 100.0);
        if (i == 0) {
            node->shape_width = relative ? 10 : width * 10 / 100.0;
            node->shape_height = relative ? 20 : height * 20 / 200.0;
            node->shape_width_unit = relative ? SR_LENGTH_VW : SR_LENGTH_PIXELS;
            node->shape_height_unit = relative ? SR_LENGTH_PERCENT : SR_LENGTH_PIXELS;
            node->body.velocity_x = 15;
            node->transform.scale_x.base = 1.125;
        }
        if (i == 2) {
            if (!soft_rigid) node->body.type = SR_BODY_NONE;
            node->soft_body = (SrSoftBody){.enabled = true, .mass = 2, .stiffness = 10,
                .damping = .2, .rows = 3, .cols = 3, .pin = SR_PIN_TOP};
            node->shape_width = relative ? 50 : width / 4.0;
            node->shape_height = relative ? 25 : height / 4.0;
            node->shape_width_unit = relative ? SR_LENGTH_PERCENT : SR_LENGTH_PIXELS;
            node->shape_height_unit = relative ? SR_LENGTH_VH : SR_LENGTH_PIXELS;
            value(&node->transform.anchor_x, relative, 5, SR_LENGTH_PERCENT, width / 40.0);
            value(&node->transform.anchor_y, relative, 5, SR_LENGTH_PERCENT, height / 40.0);
            node->transform.rotation.base = 10;
        }
    }
    return true;
}

static SrStatus prepare_scene(SrScene *scene) {
    FILE *sink = tmpfile();
    SrDiagnostics diag;
    sr_diag_init(&diag, "relative-physics", sink ? sink : stderr);
    SrStatus status = sr_physics_prepare(scene, &diag);
    if (sink) fclose(sink);
    return status;
}

static void compare_samples(sr_test_ctx *t, const SrScene *a, const SrScene *b) {
    const SrNode *ga = a->root->children[0], *gb = b->root->children[0];
    for (size_t i = 0; i < ga->child_count; ++i) {
        const SrNode *na = ga->children[i], *nb = gb->children[i];
        CHECK_INT(t, na->physics_sample_count, nb->physics_sample_count);
        if (na->physics_sample_count && na->physics_sample_count == nb->physics_sample_count)
            CHECK(t, !memcmp(na->physics_samples, nb->physics_samples,
                              na->physics_sample_count * sizeof(*na->physics_samples)));
        CHECK_INT(t, na->soft_body.sample_count, nb->soft_body.sample_count);
        if (na->soft_body.sample_count &&
            na->soft_body.sample_count == nb->soft_body.sample_count) {
            size_t values = na->soft_body.sample_count * na->soft_body.rows *
                na->soft_body.cols * 2;
            CHECK(t, !memcmp(na->soft_body.offsets, nb->soft_body.offsets,
                              values * sizeof(double)));
        }
        const double times[] = {.29, .02, .17, 0, .17};
        for (size_t j = 0; j < sizeof(times) / sizeof(times[0]); ++j) {
            double pa[3] = {0}, pb[3] = {0};
            CHECK_INT(t, sr_physics_pose(a, na, times[j], &pa[0], &pa[1], &pa[2]),
                          sr_physics_pose(b, nb, times[j], &pb[0], &pb[1], &pb[2]));
            CHECK(t, !memcmp(pa, pb, sizeof(pa)));
        }
    }
}

static void rigid_soft_literal_reference(sr_test_ctx *t) {
    for (int mode = 0; mode < 4; ++mode) {
        uint32_t w = mode % 2 ? 240 : 320, h = mode % 2 ? 160 : 180;
        SrScene scenes[2];
        bool a = physics_scene(&scenes[0], false, w, h, mode >= 2);
        bool b = physics_scene(&scenes[1], true, w, h, mode >= 2);
        CHECK(t, a && b);
        if (a && b) {
            SrNode *node = scenes[1].root->children[0]->children[0];
            SrTransform authored = node->transform;
            CHECK_INT(t, prepare_scene(&scenes[0]), SR_OK);
            CHECK_INT(t, prepare_scene(&scenes[1]), SR_OK);
            compare_samples(t, &scenes[0], &scenes[1]);
            CHECK(t, !memcmp(&authored, &node->transform, sizeof(authored)));
            CHECK_NEAR(t, node->shape_width, 10, 0);
            CHECK_INT(t, node->shape_width_unit, SR_LENGTH_VW);
            CHECK_INT(t, sr_track_add(&node->transform.x.track,
                          (SrKeyframe){.time = 0, .value = 99, .unit = SR_LENGTH_PERCENT}), SR_OK);
            CHECK_INT(t, sr_track_finalize(&node->transform.x.track), SR_OK);
            CHECK_INT(t, prepare_scene(&scenes[1]), SR_OK);
            compare_samples(t, &scenes[0], &scenes[1]);
        }
        sr_scene_free(&scenes[0]); sr_scene_free(&scenes[1]);
    }
}

static void constraint_reference(sr_test_ctx *t) {
    for (SrConstraintType type = SR_CONSTRAINT_SPRING; type <= SR_CONSTRAINT_PIN; ++type) {
        for (int explicit_kind = 0; explicit_kind < 3; ++explicit_kind) {
            for (int rigid = 0; rigid < (type == SR_CONSTRAINT_PIN ? 2 : 1); ++rigid) {
                SrScene scenes[2];
                for (int i = 0; i < 2; ++i) {
                    CHECK(t, physics_scene(&scenes[i], i == 1, 320, 180, false));
                    SrNode *group = scenes[i].root->children[0];
                    SrNode *a = group->children[0], *b = group->children[1];
                    a->body.collider = b->body.collider = SR_COLLIDER_CIRCLE;
                    a->body.radius = b->body.radius = .01;
                    SrConstraint *c = scenes[i].physics.constraints = sr_alloc(sizeof(*c));
                    CHECK(t, c != NULL);
                    if (!c) continue;
                    scenes[i].physics.constraint_count = scenes[i].physics.constraint_capacity = 1;
                    *c = (SrConstraint){.type = type, .a = a, .b = b,
                        .x = 80, .y = 20, .stiffness = 4, .damping = .1,
                        .rigid = rigid, .rest_length_set = explicit_kind > 0,
                        .rest_length = explicit_kind == 2 ? 7 : 0};
                    if (i == 0 && (type == SR_CONSTRAINT_PIN ? explicit_kind == 0
                                                            : explicit_kind < 2))
                        c->rest_length = type == SR_CONSTRAINT_PIN ? hypot(40, -16) : 20;
                }
                SrConstraint saved = scenes[1].physics.constraints[0];
                CHECK_INT(t, prepare_scene(&scenes[0]), SR_OK);
                CHECK_INT(t, prepare_scene(&scenes[1]), SR_OK);
                compare_samples(t, &scenes[0], &scenes[1]);
                CHECK(t, !memcmp(&saved, scenes[1].physics.constraints, sizeof(saved)));
                sr_scene_free(&scenes[0]); sr_scene_free(&scenes[1]);
            }
        }
    }
}

static void cache_geometry_fingerprint(sr_test_ctx *t) {
    char path[1024];
    snprintf(path, sizeof(path), "%s", sr_test_tmp_path("relative.physics"));
    unlink(path);
    SrScene scene;
    CHECK(t, physics_scene(&scene, true, 320, 180, true));
    scene.physics.cache_path = sr_strdup(path);
    CHECK(t, scene.physics.cache_path != NULL);
    CHECK_INT(t, prepare_scene(&scene), SR_OK);
    CHECK(t, !scene.physics.cache_hit);
    CHECK_INT(t, prepare_scene(&scene), SR_OK);
    CHECK(t, scene.physics.cache_hit);
    SrNode *group = scene.root->children[0];
    for (int change = 0; change < 6; ++change) {
        if (change == 0) scene.project.width = 400;
        if (change == 1) scene.project.height = 200;
        if (change == 2) group->group_width.value = 190;
        if (change == 3) group->group_height.value = 95;
        if (change == 4) group->children[0]->transform.x.unit = SR_LENGTH_VW;
        /* 100vmin and 100vh resolve equally in this landscape project;
         * their authored units must still invalidate the cache. */
        if (change == 5) group->children[2]->shape_height_unit = SR_LENGTH_VMIN;
        CHECK_INT(t, prepare_scene(&scene), SR_OK);
        CHECK(t, !scene.physics.cache_hit);
        CHECK_INT(t, prepare_scene(&scene), SR_OK);
        CHECK(t, scene.physics.cache_hit);
    }
    FILE *file = fopen(path, "r+b");
    CHECK(t, file != NULL);
    if (file) {
        uint32_t obsolete = 4;
        CHECK_INT(t, fseek(file, 8, SEEK_SET), 0);
        CHECK_INT(t, fwrite(&obsolete, sizeof(obsolete), 1, file), 1);
        fclose(file);
        CHECK_INT(t, prepare_scene(&scene), SR_OK);
        CHECK(t, !scene.physics.cache_hit);
    }
    sr_scene_free(&scene);
    unlink(path);
}

static void viewport_uses_output_box(sr_test_ctx *t) {
    SrScene scenes[2];
    bool a = physics_scene(&scenes[0], false, 320, 180, false);
    bool b = physics_scene(&scenes[1], true, 320, 180, false);
    CHECK(t, a && b);
    uint64_t hashes[2][3] = {{0}};
    for (int i = 0; a && b && i < 2; ++i) {
        scenes[i].project.mode = SR_MODE_VIEWPORT;
        scenes[i].scene360.width = 640;
        scenes[i].scene360.height = 320;
        SrCamera *camera = scenes[i].cameras = sr_alloc(sizeof(*camera));
        CHECK(t, camera != NULL);
        if (!camera) continue;
        scenes[i].camera_count = scenes[i].camera_capacity = 1;
        camera->active = true;
        camera->fov.base = 25;
        camera->yaw.base = -157.5;
        camera->pitch.base = -69.75;
        const unsigned frames[] = {0, 3, 0};
        for (size_t k = 0; k < sizeof(frames) / sizeof(frames[0]); ++k) {
            const char *path = sr_test_tmp_path("length-viewport.ppm");
            SrRenderOptions options = {.preview = true, .preview_path = path,
                .preview_frame = frames[k], .encoder_threads = i ? 4 : 1};
            SrRenderMetrics metrics;
            FILE *sink = tmpfile();
            SrDiagnostics diag;
            sr_diag_init(&diag, "length-viewport", sink ? sink : stderr);
            CHECK_INT(t, sr_render(&scenes[i], &options, &metrics, &diag), SR_OK);
            hashes[i][k] = metrics.preview_hash;
            if (k == 2) CHECK(t, hashes[i][0] == hashes[i][2]);
            if (i == 0 && k == 2) {
                size_t children = scenes[i].root->child_count;
                scenes[i].root->child_count = 0;
                CHECK_INT(t, sr_render(&scenes[i], &options, &metrics, &diag), SR_OK);
                CHECK(t, hashes[i][k] != metrics.preview_hash);
                scenes[i].root->child_count = children;
            }
            if (sink) fclose(sink);
            unlink(path);
        }
    }
    if (a && b) CHECK(t, !memcmp(hashes[0], hashes[1], sizeof(hashes[0])));
    sr_scene_free(&scenes[0]); sr_scene_free(&scenes[1]);
}

static void constraint_limit(sr_test_ctx *t) {
    SrScene scene;
    CHECK(t, physics_scene(&scene, true, 320, 180, false));
    scene.physics.constraint_count = SR_MAX_LENGTH_CONSTRAINTS + 1;
    CHECK_INT(t, prepare_scene(&scene), SR_ERR_RENDER);
    scene.physics.constraint_count = 0;
    CHECK_INT(t, prepare_scene(&scene), SR_OK);
    sr_scene_free(&scene);
}

static void physics_pose_overrides_length_tracks(sr_test_ctx *t) {
    SrScene scenes[2];
    bool a = physics_scene(&scenes[0], false, 320, 180, false);
    bool b = physics_scene(&scenes[1], true, 320, 180, false);
    CHECK(t, a && b);
    if (a && b) {
        SrNode *body = scenes[1].root->children[0]->children[0];
        SrTrack *track = &body->transform.x.track;
        track->extrapolate_after = SR_EXTRAPOLATE_OFFSET;
        CHECK_INT(t, sr_track_add(track, (SrKeyframe){.time = 0,
                      .value = 0, .unit = SR_LENGTH_PERCENT}), SR_OK);
        CHECK_INT(t, sr_track_add(track, (SrKeyframe){.time = 1e-9,
                      .value = 100, .unit = SR_LENGTH_PERCENT}), SR_OK);
        CHECK_INT(t, sr_track_finalize(track), SR_OK);
        CHECK_INT(t, prepare_scene(&scenes[0]), SR_OK);
        CHECK_INT(t, prepare_scene(&scenes[1]), SR_OK);
        const double times[] = {10, 0, .15, 10};
        const float black[4] = {0, 0, 0, 1};
        for (size_t k = 0; k < sizeof(times) / sizeof(times[0]); ++k) {
            SrFrame frames[2] = {{0}};
            bool first = fx_render(t, &scenes[0], times[k], black, &frames[0]);
            bool second = fx_render(t, &scenes[1], times[k], black, &frames[1]);
            CHECK(t, first && second);
            if (first && second)
                CHECK(t, !memcmp(frames[0].px, frames[1].px,
                                  (size_t)320 * 180 * 4 * sizeof(float)));
            sr_frame_free(&frames[0]); sr_frame_free(&frames[1]);
        }
    }
    sr_scene_free(&scenes[0]); sr_scene_free(&scenes[1]);
}

static void nonphysics_bases_are_unused(sr_test_ctx *t) {
    for (int physics = 0; physics < 2; ++physics) {
        SrScene scene;
        fx_scene(&scene, 64, 48);
        scene.has_relative_lengths = true;
        scene.project.duration = .1;
        scene.physics.enabled = physics;
        SrNode *group = fx_add(&scene, NULL, SR_NODE_GROUP);
        CHECK(t, group != NULL);
        if (!group) { sr_scene_free(&scene); continue; }
        group->group_width_set = true;
        group->group_width = (SrLength){1e12, SR_LENGTH_PIXELS};
        for (int i = 0; i < 2; ++i) {
            SrNode *node = fx_rect(&scene, group, 1e6, 8, 8, 8, (SrColor){1, 0, 0, 1}, 1);
            CHECK(t, node != NULL);
            if (!node) continue;
            node->transform.x.unit = SR_LENGTH_PERCENT;
            node->visible = i == 1;
            if (i == 1) {
                CHECK_INT(t, sr_track_add(&node->transform.x.track,
                              (SrKeyframe){.time = 0, .value = 8}), SR_OK);
                CHECK_INT(t, sr_track_finalize(&node->transform.x.track), SR_OK);
            }
        }
        SrNode *body = fx_rect(&scene, group, 40, 30, 4, 4, (SrColor){0, 1, 0, 1}, 1);
        CHECK(t, body != NULL);
        if (body) {
            body->body.type = physics ? SR_BODY_DYNAMIC : SR_BODY_NONE;
            value(&body->transform.anchor_x, true, 1e6, SR_LENGTH_PERCENT, 0);
            CHECK_INT(t, sr_track_add(&body->transform.anchor_x.track,
                          (SrKeyframe){.time = 0, .value = 0}), SR_OK);
            CHECK_INT(t, sr_track_finalize(&body->transform.anchor_x.track), SR_OK);
        }
        const char *path = sr_test_tmp_path("unused-length-base.ppm");
        SrRenderOptions options = {.preview = true, .preview_path = path};
        SrRenderMetrics metrics;
        FILE *sink = tmpfile();
        SrDiagnostics diag;
        sr_diag_init(&diag, "unused-length-base", sink ? sink : stderr);
        CHECK_INT(t, sr_render(&scene, &options, &metrics, &diag), SR_OK);
        if (sink) fclose(sink);
        sr_scene_free(&scene);
        unlink(path);
    }
}

const sr_test_case sr_tests_length_physics[] = {
    {"rigid_soft_literal_reference", rigid_soft_literal_reference},
    {"constraint_reference", constraint_reference},
    {"cache_geometry_fingerprint", cache_geometry_fingerprint},
    {"viewport_uses_output_box", viewport_uses_output_box},
    {"constraint_limit", constraint_limit},
    {"nonphysics_bases_are_unused", nonphysics_bases_are_unused},
    {"physics_pose_overrides_length_tracks", physics_pose_overrides_length_tracks},
    {NULL, NULL}
};
