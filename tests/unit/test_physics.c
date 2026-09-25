/* SPDX-License-Identifier: Apache-2.0 */
#include "scene_render/physics.h"
#include "scene_render/xml.h"

#include <unistd.h>

#include "harness.h"

/* A single free body with no gravity. damping*dt = 500 * 0.01 = 5, far past
 * the point where the old linear factor (1 - damping*dt) turned negative. */
#define DT 0.01
#define DAMPING 500.0
#define V0 100.0
#define W0 90.0

static void test_heavy_damping_never_flips_sign(sr_test_ctx *t)
{
    const char *path = sr_test_tmp_path("heavy-damping.xml");
    FILE *file = fopen(path, "w");
    CHECK(t, file != NULL);
    if (!file) return;
    fprintf(file,
        "<scene version=\"1.0\"><project width=\"16\" height=\"16\" fps=\"10\" "
        "duration=\"0.2\"/><composition>"
        "<shape id=\"body\" shape=\"rect\" width=\"4\" height=\"4\" x=\"0\" y=\"0\">"
        "<rigidBody type=\"dynamic\" shape=\"box\" mass=\"1\" velocityX=\"%g\" "
        "angularVelocity=\"%g\" linearDamping=\"%g\" angularDamping=\"%g\"/>"
        "</shape></composition>"
        "<physics fixedStep=\"%g\" gravityX=\"0\" gravityY=\"0\"/></scene>",
        V0, W0, DAMPING, DAMPING, DT);
    CHECK(t, fclose(file) == 0);
    FILE *sink = tmpfile();
    CHECK(t, sink != NULL);
    if (!sink) { unlink(path); return; }
    SrDiagnostics diag;
    sr_diag_init(&diag, path, sink);
    SrScene scene;
    SrStatus status = sr_scene_load_xml(path, &scene, &diag);
    CHECK(t, status == SR_OK);
    if (status != SR_OK) { fclose(sink); unlink(path); return; }
    CHECK(t, sr_physics_prepare(&scene, &diag) == SR_OK);
    SrNode *body = sr_scene_find_node(&scene, "body");
    CHECK(t, body != NULL);
    if (body && body->physics_sample_count > 2) {
        double x0, y0, r0;
        CHECK(t, sr_physics_pose(&scene, body, 0.0, &x0, &y0, &r0));
        double previous_x = x0, previous_r = r0;
        for (size_t s = 1; s < body->physics_sample_count; ++s) {
            double x = body->physics_samples[s].x.base;
            double r = body->physics_samples[s].rotation.base;
            if (x < previous_x) SR_FAIL(t, "x moved backwards at step %zu", s);
            if (r < previous_r) SR_FAIL(t, "rotation reversed at step %zu", s);
            previous_x = x;
            previous_r = r;
        }
        /* Exponential decay: the first step keeps exp(-damping*dt) of the
         * velocity instead of clamping it to zero. */
        double factor = exp(-DAMPING * DT);
        CHECK_NEAR(t, body->physics_samples[1].x.base - x0,
                   V0 * factor * DT, 1e-12);
        CHECK_NEAR(t, body->physics_samples[1].rotation.base - r0,
                   W0 * factor * DT, 1e-12);
        CHECK(t, body->physics_samples[1].x.base > x0);
        CHECK_NEAR(t, body->physics_samples[1].y.base, y0, 1e-12);
    } else {
        SR_FAIL(t, "expected physics samples for the body");
    }
    sr_scene_free(&scene);
    fclose(sink);
    unlink(path);
}

const sr_test_case sr_tests_physics[] = {
    {"heavy_damping_never_flips_sign", test_heavy_damping_never_flips_sign},
    {NULL, NULL},
};
