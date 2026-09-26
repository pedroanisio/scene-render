/* SPDX-License-Identifier: Apache-2.0 */
#include "fixture.h"
#include "scene_text.h"
#include "scene_render/assets.h"
#include "scene_render/physics.h"
#include "scene_render/property.h"
#include "scene_render/random.h"

#include <float.h>
#include <math.h>

#define COUNT(a) (sizeof(a) / sizeof((a)[0]))

static void matrix_order(sr_test_ctx *t) {
    SrMat3 original = {2, -.5, 3, -0.0, 4, -7};
    SrMat3 zero = sr_mat_apply_skew(original, 0, -0.0);
    CHECK(t, !memcmp(&original, &zero, sizeof(zero)));
    const double angles[][2] = {{45, 0}, {0, -45}, {45, -30}, {-89, 89}};
    for (size_t i = 0; i < COUNT(angles); ++i) {
        double kx = tan(angles[i][0] * SR_PI / 180);
        double ky = tan(angles[i][1] * SR_PI / 180);
        SrMat3 m = sr_mat_apply_skew(original, angles[i][0], angles[i][1]);
        for (int x = -2; x <= 2; ++x) for (int y = -2; y <= 2; ++y) {
            double v = y + ky * x, u = x + kx * v;
            SrVec2 p = sr_mat_point(m, (SrVec2){x, y});
            CHECK_NEAR(t, p.x, 2*u - .5*v + 3, 1e-10);
            CHECK_NEAR(t, p.y, 4*v - 7, 1e-10);
        }
        SrMat3 inverse;
        CHECK(t, sr_mat_checked_inverse(m, &inverse));
        SrVec2 p = sr_mat_point(inverse, sr_mat_point(m, (SrVec2){2, -3}));
        CHECK_NEAR(t, p.x, 2, 1e-8);
        CHECK_NEAR(t, p.y, -3, 1e-8);
    }
    SrMat3 bad[] = {{NAN,0,0,0,1,0}, {1,INFINITY,0,0,1,0},
        {1,0,NAN,0,1,0}, {1,0,0,NAN,1,0}, {1,0,0,0,NAN,0},
        {1,0,0,0,1,NAN}, {0,0,0,0,0,0}, {1e308,0,0,0,1e308,0},
        {1e-14,0,DBL_MAX,0,1,DBL_MAX}};
    for (size_t i = 0; i < COUNT(bad); ++i) {
        SrMat3 inverse;
        CHECK(t, !sr_mat_checked_inverse(bad[i], &inverse));
    }
    CHECK(t, !sr_mat_checked_inverse(sr_mat_identity(), NULL));
}

static void xml(char *out, size_t size, unsigned host, const char *version,
                 const char *attrs, const char *animation) {
    const char *names[] = {"group", "layer", "shape", "particleEmitter"};
    const char *extra[] = {"", "asset=\"v\"", "shape=\"rect\" width=\"8\" height=\"8\"",
                           "rate=\"4\" lifetime=\"1\" speed=\"1\" size=\"2\""};
    snprintf(out, size, "<scene version=\"%s\">\n"
        "<project width=\"64\" height=\"64\" fps=\"12\" duration=\"2\"/>\n"
        "<assets><vector id=\"v\" shape=\"rect\" width=\"8\" height=\"8\"/></assets>\n"
        "<composition>\n<%s id=\"n\" %s %s>%s</%s>\n</composition></scene>",
        version, names[host], extra[host], attrs, animation, names[host]);
}

static void xml_hosts_and_limits(sr_test_ctx *t) {
    char text[4096];
    for (unsigned host = 0; host < 4; ++host) {
        for (unsigned version = 0; version < 2; ++version) {
            xml(text, sizeof(text), host, version ? "1.1" : "1.0",
                "skewX=\"89\" skewY=\"-89\"", version ?
                "<animate property=\"skew.x\"><key time=\"0\" value=\"-89\"/>"
                "<key time=\"1\" value=\"89\"/></animate>"
                "<animate property=\"skew.y\"><key time=\"0\" value=\"89\"/>"
                "<key time=\"1\" value=\"-89\"/></animate>" : "");
            SrScene scene;
            SrStatus status = st_load(t, "skew-host.xml", text, &scene, NULL);
            CHECK_INT(t, status, SR_OK);
            if (status != SR_OK) continue;
            SrNode *node = sr_scene_find_node(&scene, "n");
            CHECK(t, node != NULL);
            if (node) {
                CHECK_NEAR(t, node->transform.skew_x.base, 89, 0);
                CHECK_NEAR(t, node->transform.skew_y.base, -89, 0);
                CHECK(t, sr_node_property(node, "skew.x") == &node->transform.skew_x);
                CHECK(t, sr_node_property(node, "skew.y") == &node->transform.skew_y);
                CHECK_NEAR(t, sr_anim_eval(&node->transform.skew_x, .25),
                           version ? -44.5 : 89, 0);
                CHECK_NEAR(t, sr_anim_eval(&node->transform.skew_y, .25),
                           version ? 44.5 : -89, 0);
            }
            sr_scene_free(&scene);
        }
        xml(text, sizeof(text), host, "1.0", "",
            "<animate property=\"skew.x\"><key time=\"0\" value=\"0\"/></animate>");
        SrScene old;
        char *version_message = NULL;
        SrStatus version_status = st_load(t, "skew-version.xml", text, &old, &version_message);
        CHECK_INT(t, version_status, SR_ERR_XML);
        CHECK_CONTAINS(t, version_message, "requires version=\"1.1\"");
        if (version_status == SR_OK) sr_scene_free(&old);
        free(version_message);
        const char *invalid[] = {"89.00001", "-89.00001", "90", "NaN", "INF", "4deg"};
        for (size_t i = 0; i < COUNT(invalid); ++i) for (unsigned axis = 0; axis < 2; ++axis) {
            char attrs[128], animation[256];
            snprintf(attrs, sizeof(attrs), "skew%c=\"%s\"", axis ? 'Y' : 'X', invalid[i]);
            snprintf(animation, sizeof(animation),
                "<animate property=\"skew.%c\"><key time=\"0\" value=\"%s\"/></animate>",
                axis ? 'y' : 'x', invalid[i]);
            for (unsigned key = 0; key < 2; ++key) {
                xml(text, sizeof(text), host, "1.1", key ? "" : attrs, key ? animation : "");
                SrScene scene;
                char *message = NULL;
                SrStatus status = st_load(t, "skew-invalid.xml", text, &scene, &message);
                CHECK_INT(t, status, SR_ERR_XML);
                CHECK_CONTAINS(t, message, key ? "<key> @value" : axis ? "@skewY" : "@skewX");
                CHECK_CONTAINS(t, message, ":5:");
                if (status == SR_OK) sr_scene_free(&scene);
                free(message);
            }
        }
    }
    CHECK(t, sr_property_find(SR_PROPERTY_OBJECT3D, "skew.x") == NULL);
    CHECK(t, sr_property_find(SR_PROPERTY_CAMERA, "skew.y") == NULL);
}

/* Independently undo T R Kx Ky S T(-anchor) in scalar coordinates. */
static SrVec2 local_point(const SrTransform *tr, SrVec2 p) {
    p.x -= tr->x.base; p.y -= tr->y.base;
    double r = tr->rotation.base * SR_PI / 180, c = cos(r), s = sin(r);
    double x = c*p.x + s*p.y, y = -s*p.x + c*p.y;
    x -= tan(tr->skew_x.base * SR_PI / 180) * y;
    y -= tan(tr->skew_y.base * SR_PI / 180) * x;
    return (SrVec2){x/tr->scale_x.base + tr->anchor_x.base,
                    y/tr->scale_y.base + tr->anchor_y.base};
}

static void raster_order_and_masks(sr_test_ctx *t) {
    for (unsigned variant = 0; variant < 4; ++variant) {
        SrScene scene;
        fx_scene(&scene, 96, 80);
        SrNode *group = fx_add(&scene, NULL, SR_NODE_GROUP);
        SrNode *node = group ? fx_rect(&scene, group, 4, -3, 30, 24,
                                       (SrColor){1,0,0,1}, 1) : NULL;
        CHECK(t, node != NULL);
        if (!node) { sr_scene_free(&scene); continue; }
        group->transform.x.base = 48; group->transform.y.base = 40;
        group->transform.rotation.base = 17;
        group->transform.skew_y.base = -20;
        group->transform.scale_y.base = .8;
        node->transform.skew_x.base = variant & 1 ? -35 : 35;
        node->transform.skew_y.base = variant & 2 ? 25 : -25;
        node->transform.scale_x.base = variant & 1 ? -1.1 : 1.1;
        node->transform.anchor_x.base = 15; node->transform.anchor_y.base = 12;
        node->transform.rotation.base = -12;
        CHECK_INT(t, sr_node_add_mask(node, fx_mask(SR_MASK_RECT, 3, 4, 22, 16, false)), SR_OK);
        SrFrame frame = {0};
        const float clear[4] = {0};
        unsigned inside = 0, outside = 0;
        if (fx_render(t, &scene, 0, clear, &frame)) {
            for (uint32_t y = 0; y < 80; ++y) for (uint32_t x = 0; x < 96; ++x) {
                SrVec2 p = local_point(&group->transform, (SrVec2){x+.5, y+.5});
                p = local_point(&node->transform, p);
                const float *pixel = fx_px(&frame, x, y);
                if (p.x > 5 && p.x < 23 && p.y > 6 && p.y < 18) {
                    CHECK_NEAR(t, pixel[0], 1, 0); ++inside;
                } else if (p.x < 1 || p.x > 27 || p.y < 2 || p.y > 22) {
                    CHECK_NEAR(t, pixel[3], 0, 0); ++outside;
                }
            }
        }
        CHECK(t, inside > 100 && outside > 1000);
        sr_frame_free(&frame);
        sr_scene_free(&scene);
    }
}

static SrStatus render_status(SrScene *scene, SrCompositor *compositor,
                                SrFrame *frame, double time, char **message) {
    FILE *sink = tmpfile();
    SrDiagnostics diag;
    sr_diag_init(&diag, "skew-test", sink ? sink : stderr);
    const float clear[4] = {0};
    sr_frame_clear(frame, clear, 1);
    SrStatus status = sr_compositor_render(compositor, scene, time, frame,
                                           message ? &diag : NULL);
    if (message && sink) {
        fflush(sink);
        long size = ftell(sink);
        rewind(sink);
        *message = calloc(1, (size_t)size + 1);
        if (*message) (void)fread(*message, 1, (size_t)size, sink);
    }
    if (sink) fclose(sink);
    return status;
}

static void runtime_validation(sr_test_ctx *t) {
    SrScene scene;
    fx_scene(&scene, 32, 32);
    SrNode *group = fx_add(&scene, NULL, SR_NODE_GROUP);
    SrNode *node = group ? fx_rect(&scene, group, 4, 4, 8, 8, (SrColor){1,0,0,1}, 1) : NULL;
    CHECK(t, node != NULL);
    if (!node) { sr_scene_free(&scene); return; }
    node->source_line = 17;
    SrFrame frame = {0};
    SrCompositor compositor;
    sr_compositor_init(&compositor, 4);
    CHECK_INT(t, sr_frame_init(&frame, 32, 32), SR_OK);
    const double bad[] = {90, -90, INFINITY, NAN};
    for (size_t i = 0; i < COUNT(bad); ++i) for (unsigned axis = 0; axis < 2; ++axis) {
        SrAnimValue *value = axis ? &node->transform.skew_y : &node->transform.skew_x;
        value->base = bad[i];
        char *message = NULL;
        CHECK_INT(t, render_status(&scene, &compositor, &frame, 0, &message), SR_ERR_RENDER);
        CHECK_CONTAINS(t, message, ":17:");
        CHECK_CONTAINS(t, message, axis ? "@skewY" : "@skewX");
        CHECK_INT(t, render_status(&scene, &compositor, &frame, 0, NULL), SR_ERR_RENDER);
        free(message); value->base = 0;
    }
    SrAnimValue *value = &node->transform.skew_x;
    CHECK_INT(t, sr_track_add(&value->track,
                              (SrKeyframe){.time=0, .value=0, .curve=SR_CURVE_BACK_OUT}), SR_OK);
    CHECK_INT(t, sr_track_add(&value->track,
                              (SrKeyframe){.time=1, .value=88, .curve=SR_CURVE_LINEAR}), SR_OK);
    CHECK_INT(t, sr_track_finalize(&value->track), SR_OK);
    CHECK(t, sr_anim_eval(value, .6) > 89);
    CHECK_INT(t, render_status(&scene, &compositor, &frame, .6, NULL), SR_ERR_RENDER);
    CHECK_INT(t, render_status(&scene, &compositor, &frame, 0, NULL), SR_OK);
    value->track.keys[0].curve = SR_CURVE_LINEAR;
    value->track.extrapolate_after = SR_EXTRAPOLATE_LINEAR;
    CHECK_INT(t, sr_track_finalize(&value->track), SR_OK);
    CHECK_INT(t, render_status(&scene, &compositor, &frame, 2, NULL), SR_ERR_RENDER);
    value->base = 80; value->track.additive = true;
    CHECK_INT(t, render_status(&scene, &compositor, &frame, .5, NULL), SR_ERR_RENDER);
    sr_track_free(&value->track); value->base = 20;
    group->transform.scale_x.base = 0;
    CHECK_INT(t, render_status(&scene, &compositor, &frame, 0, NULL), SR_ERR_RENDER);
    group->opacity.base = .5;
    /* An isolated singular unskewed group culls before consuming children. */
    CHECK_INT(t, render_status(&scene, &compositor, &frame, 0, NULL), SR_OK);
    group->opacity.base = 1;
    group->transform.skew_x.base = 10;
    CHECK_INT(t, render_status(&scene, &compositor, &frame, 0, NULL), SR_ERR_RENDER);
    group->transform.scale_x.base = group->transform.scale_y.base = 1e308;
    CHECK_INT(t, render_status(&scene, &compositor, &frame, 0, NULL), SR_ERR_RENDER);
    group->transform.scale_x.base = group->transform.scale_y.base = 1;
    group->transform.skew_x.base = 0;
    node->transform.scale_x.base = 0;
    CHECK_INT(t, render_status(&scene, &compositor, &frame, 0, NULL), SR_ERR_RENDER);
    value->base = 0;
    CHECK_INT(t, render_status(&scene, &compositor, &frame, 0, NULL), SR_OK);
    node->transform.scale_x.base = 1;
    value->base = 100; node->opacity.base = 0;
    CHECK_INT(t, render_status(&scene, &compositor, &frame, 0, NULL), SR_OK);
    node->card = true;
    CHECK_INT(t, render_status(&scene, &compositor, &frame, 0, NULL), SR_ERR_RENDER);
    node->opacity.base = 1; value->base = 20;
    node->transform.rotation_y.base = 1e308;
    CHECK_INT(t, render_status(&scene, &compositor, &frame, 0, NULL), SR_ERR_RENDER);
    node->transform.rotation_y.base = 0;
    node->transform.z.base = INFINITY;
    CHECK_INT(t, render_status(&scene, &compositor, &frame, 0, NULL), SR_ERR_RENDER);
    node->transform.z.base = 0; node->card = false;
    group->card = true; group->transform.rotation_y.base = 25;
    node->opacity.base = 0; value->base = 100;
    /* Orthographic cards draw affinely, so the invisible child is unused. */
    CHECK_INT(t, render_status(&scene, &compositor, &frame, 0, NULL), SR_OK);
    scene.cameras = sr_alloc(sizeof(*scene.cameras));
    CHECK(t, scene.cameras != NULL);
    if (scene.cameras) {
        scene.camera_count = scene.camera_capacity = 1;
        scene.cameras[0] = (SrCamera){.active=true, .z={.base=-400},
            .zoom={.base=400}, .zoom_set=true, .near_plane=.1, .far_plane=10000};
        /* Perspective bounds consume the child's transform despite opacity. */
        CHECK_INT(t, render_status(&scene, &compositor, &frame, 0, NULL), SR_ERR_RENDER);
    }
    group->card = false; group->transform.rotation_y.base = 0;
    node->opacity.base = 1; value->base = 20;
    CHECK_INT(t, render_status(&scene, &compositor, &frame, 0, NULL), SR_OK);
    sr_compositor_free(&compositor); sr_frame_free(&frame); sr_scene_free(&scene);
}

static void parser_mutations(sr_test_ctx *t) {
    uint64_t seed = 912841;
    for (unsigned trial = 0; trial < 96; ++trial) {
        seed = sr_random_mix64(seed);
        char text[4096], attrs[128];
        int value = (int)(seed % 179) - 89;
        snprintf(attrs, sizeof(attrs), "skewX=\"%d\" skewY=\"%d\"", value, -value);
        xml(text, sizeof(text), trial % 4, "1.1", attrs, "");
        bool valid = trial % 3 == 0;
        if (trial % 3 == 1) {
            char *p = strstr(text, "skewX=\"") + 7;
            *p = 'q';
        } else if (trial % 3 == 2) text[seed % strlen(text)] = '\0';
        SrScene scene;
        SrStatus status = st_load(t, "skew-fuzz.xml", text, &scene, NULL);
        CHECK_INT(t, status, valid ? SR_OK : SR_ERR_XML);
        if (status == SR_OK) sr_scene_free(&scene);
    }
}

static SrStatus prepare(SrScene *scene) {
    SrDiagnostics diag;
    FILE *sink;
    st_diag(&diag, &sink);
    SrStatus status = sr_physics_prepare(scene, &diag);
    if (sink) fclose(sink);
    return status;
}

static void physics_rest_and_cache(sr_test_ctx *t) {
    SrScene scene;
    fx_scene(&scene, 512, 512);
    scene.project.duration = .02;
    scene.physics.fixed_step = .01;
    scene.physics.gravity_y = 20;
    SrNode *node = fx_rect(&scene, NULL, 128, 128, 12, 12, (SrColor){1,0,0,1}, 1);
    CHECK(t, node != NULL);
    if (!node) { sr_scene_free(&scene); return; }
    node->id = sr_strdup("sheet");
    node->transform.skew_x.base = 45;
    node->transform.skew_y.base = -30;
    node->transform.scale_x.base = 2;
    node->transform.scale_y.base = 3;
    node->soft_body = (SrSoftBody){.enabled=true, .rows=2, .cols=2, .mass=1};
    const char *cache = sr_test_tmp_path("skew.physics");
    unlink(cache);
    scene.physics.cache_path = sr_strdup(cache);
    CHECK_INT(t, prepare(&scene), SR_OK);
    CHECK(t, !scene.physics.cache_hit);
    CHECK_INT(t, node->soft_body.sample_count, 3);
    if (node->soft_body.sample_count == 3) {
        /* Unpinned, zero stiffness/damping: semi-implicit Euler gives
         * gravity * dt^2 * (1+2) in world Y after two steps. */
        double dy = 20 * .01 * .01 * 3;
        double x = -dy * tan(45 * SR_PI / 180);
        double y = dy - x * tan(-30 * SR_PI / 180);
        for (size_t i = 0; i < 4; ++i) {
            CHECK_NEAR(t, node->soft_body.offsets[16 + 2*i], x/2, 1e-12);
            CHECK_NEAR(t, node->soft_body.offsets[17 + 2*i], y/3, 1e-12);
        }
    }
    double saved[24];
    if (node->soft_body.offsets) memcpy(saved, node->soft_body.offsets, sizeof(saved));
    CHECK_INT(t, prepare(&scene), SR_OK);
    CHECK(t, scene.physics.cache_hit);
    if (node->soft_body.offsets)
        CHECK(t, !memcmp(saved, node->soft_body.offsets, sizeof(saved)));
    for (unsigned axis = 0; axis < 2; ++axis) {
        SrAnimValue *v = axis ? &node->transform.skew_y : &node->transform.skew_x;
        v->base += 1;
        CHECK_INT(t, prepare(&scene), SR_OK);
        CHECK(t, !scene.physics.cache_hit);
        CHECK_INT(t, prepare(&scene), SR_OK);
        CHECK(t, scene.physics.cache_hit);
    }
    FILE *file = fopen(cache, "r+b");
    CHECK(t, file != NULL);
    if (file) {
        uint32_t version = 5;
        CHECK_INT(t, fseek(file, 8, SEEK_SET), 0);
        CHECK_INT(t, fwrite(&version, sizeof(version), 1, file), 1);
        fclose(file);
        CHECK_INT(t, prepare(&scene), SR_OK);
        CHECK(t, !scene.physics.cache_hit);
    }
    node->transform.scale_x.base = 0;
    CHECK_INT(t, prepare(&scene), SR_ERR_RENDER);
    node->transform.scale_x.base = 2;
    node->transform.skew_x.base = 90;
    CHECK_INT(t, prepare(&scene), SR_ERR_RENDER);
    node->transform.skew_x.base = 0;
    node->transform.skew_y.base = NAN;
    CHECK_INT(t, prepare(&scene), SR_ERR_RENDER);
    unlink(cache);
    sr_scene_free(&scene);
}

static void rigid_shape_unchanged(sr_test_ctx *t) {
    SrScene scene;
    fx_scene(&scene, 64, 64);
    scene.project.duration = .1;
    scene.physics.enabled = true;
    scene.physics.fixed_step = .01;
    scene.physics.gravity_y = 20;
    SrNode *node = fx_rect(&scene, NULL, 25, 25, 8, 8, (SrColor){1,0,0,1}, 1);
    CHECK(t, node != NULL);
    if (!node) { sr_scene_free(&scene); return; }
    node->body.type = SR_BODY_DYNAMIC;
    node->body.velocity_x = 8;
    CHECK_INT(t, prepare(&scene), SR_OK);
    SrPhysicsSample saved[11];
    CHECK_INT(t, node->physics_sample_count, COUNT(saved));
    if (node->physics_sample_count == COUNT(saved)) {
        memcpy(saved, node->physics_samples, sizeof(saved));
        node->transform.skew_x.base = 40; node->transform.skew_y.base = -20;
        CHECK_INT(t, prepare(&scene), SR_OK);
        CHECK(t, !memcmp(saved, node->physics_samples, sizeof(saved)));
    }
    sr_scene_free(&scene);
}

static void render_paths_threads_and_defaults(sr_test_ctx *t) {
    SrScene scene;
    SrDiagnostics diag;
    FILE *sink;
    st_diag(&diag, &sink);
    SrStatus status = sr_scene_load_xml(sr_test_data_path("tests/golden/skew.xml"),
                                         &scene, &diag);
    CHECK_INT(t, status, SR_OK);
    if (status != SR_OK) { if (sink) fclose(sink); return; }
    CHECK_INT(t, sr_assets_load(&scene, &diag), SR_OK);
    CHECK_INT(t, sr_physics_prepare(&scene, &diag), SR_OK);
    SrNode *animated = sr_scene_find_node(&scene, "animated");
    CHECK(t, animated != NULL);
    SrNode snapshot = animated ? *animated : (SrNode){0};
    uint64_t source = scene.source_hash;
    SrCompositor warm;
    sr_compositor_init(&warm, 1);
    SrFrame frames[2] = {{0}, {0}};
    CHECK_INT(t, sr_frame_init(&frames[0], 320, 180), SR_OK);
    CHECK_INT(t, sr_frame_init(&frames[1], 320, 180), SR_OK);
    const double times[] = {23.0/12, 0, 1, .25, 1, 0};
    const float clear[4] = {0};
    for (size_t i = 0; i < COUNT(times); ++i) {
        SrCompositor fresh;
        sr_compositor_init(&fresh, 4);
        sr_frame_clear(&frames[0], clear, 1); sr_frame_clear(&frames[1], clear, 4);
        CHECK_INT(t, sr_compositor_render_scene(&warm, &scene, times[i],
                                                &frames[0], &diag), SR_OK);
        CHECK_INT(t, sr_compositor_render_scene(&fresh, &scene, times[i],
                                                &frames[1], &diag), SR_OK);
        CHECK(t, st_frames_equal(&frames[0], &frames[1]));
        sr_compositor_free(&fresh);
    }
    if (animated) CHECK(t, !memcmp(animated, &snapshot, sizeof(snapshot)));
    CHECK(t, scene.source_hash == source);
    sr_compositor_free(&warm);
    for (size_t i = 0; i < 2; ++i) sr_frame_free(&frames[i]);
    sr_scene_free(&scene);
    if (sink) fclose(sink);

    fx_scene(&scene, 32, 32);
    SrNode *node = fx_rect(&scene, NULL, 4, 5, 12, 10, (SrColor){.3,.5,.7,1}, 1);
    CHECK(t, node != NULL);
    if (node) {
        CHECK(t, fx_render(t, &scene, 0, clear, &frames[0]));
        node->transform.skew_x.base = -0.0;
        CHECK_INT(t, sr_track_add(&node->transform.skew_y.track,
                                  (SrKeyframe){.time=0, .value=0}), SR_OK);
        CHECK_INT(t, sr_track_finalize(&node->transform.skew_y.track), SR_OK);
        CHECK(t, fx_render(t, &scene, 1, clear, &frames[1]));
        CHECK(t, st_frames_equal(&frames[0], &frames[1]));
        for (size_t i = 0; i < 2; ++i) sr_frame_free(&frames[i]);
    }
    sr_scene_free(&scene);
}

const sr_test_case sr_tests_skew[] = {
    {"matrix_order", matrix_order},
    {"xml_hosts_and_limits", xml_hosts_and_limits},
    {"raster_order_and_masks", raster_order_and_masks},
    {"runtime_validation", runtime_validation},
    {"parser_mutations", parser_mutations},
    {"physics_rest_and_cache", physics_rest_and_cache},
    {"rigid_shape_unchanged", rigid_shape_unchanged},
    {"render_paths_threads_and_defaults", render_paths_threads_and_defaults},
    {NULL, NULL},
};
