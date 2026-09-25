/* SPDX-License-Identifier: Apache-2.0 */
#include "scene_render/physics.h"
#include "scene_render/xml.h"

#include <unistd.h>

#include "harness.h"
#include "scene_text.h"

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

static SrStatus prepare(sr_test_ctx *t, SrScene *scene, char **info)
{
    FILE *sink = tmpfile();
    SrDiagnostics diag;
    sr_diag_init(&diag, "physics", sink ? sink : stderr);
    diag.verbose = true;
    SrStatus status = sr_physics_prepare(scene, &diag);
    if (info && sink) {
        fflush(sink);
        long size = ftell(sink);
        rewind(sink);
        *info = calloc(1, (size_t)(size > 0 ? size : 0) + 1);
        if (*info && size > 0 && fread(*info, 1, (size_t)size, sink) == 0) (*info)[0] = 0;
    }
    if (sink) fclose(sink);
    CHECK(t, status == SR_OK);
    return status;
}

#define SOFT_SCENE(cache) \
    "<scene version=\"1.0\"><project width=\"400\" height=\"300\" fps=\"30\" duration=\"4\"/>" \
    "<composition><shape id=\"sheet\" shape=\"rect\" width=\"60\" height=\"60\" x=\"100\" y=\"40\">" \
    "<softBody mass=\"1\" stiffness=\"20\" damping=\"0.6\" rows=\"4\" cols=\"5\" pin=\"top\"/>" \
    "</shape></composition><physics fixedStep=\"0.008333333333333333\" gravityY=\"400\"" cache "/></scene>"

/* A top-pinned sheet sags under gravity (its bottom row moves down, the
 * top row stays put) and settles: the last samples no longer move. */
static void test_softbody_pinned_top_sags_and_rests(sr_test_ctx *t)
{
    SrScene scene;
    if (st_load(t, "soft-sag.xml", SOFT_SCENE(""), &scene, NULL) != SR_OK) { SR_FAIL(t, "load"); return; }
    if (prepare(t, &scene, NULL) != SR_OK) { sr_scene_free(&scene); return; }
    SrNode *sheet = sr_scene_find_node(&scene, "sheet");
    const SrSoftBody *soft = &sheet->soft_body;
    CHECK(t, soft->sample_count > 400);
    if (soft->sample_count > 400) {
        size_t values = (size_t)soft->rows * soft->cols * 2;
        const double *last = soft->offsets + (soft->sample_count - 1) * values;
        const double *before = last - 30 * values;
        for (uint32_t c = 0; c < soft->cols; ++c) {
            CHECK_NEAR(t, last[2 * c], 0.0, 1e-12);            /* pinned top */
            CHECK_NEAR(t, last[2 * c + 1], 0.0, 1e-12);
            double sag = last[2 * ((soft->rows - 1) * soft->cols + c) + 1];
            if (!(sag > 2.0)) SR_FAIL(t, "bottom column %u sag %g", c, sag);
        }
        double drift = 0.0;
        for (size_t i = 0; i < values; ++i) drift = fmax(drift, fabs(last[i] - before[i]));
        if (!(drift < 1e-3)) SR_FAIL(t, "still moving: %g px over 30 samples", drift);
        double offsets[40];
        CHECK(t, sr_physics_soft_offsets(&scene, sheet, 3.9, offsets));
    }
    sr_scene_free(&scene);
}

/* Two independent simulations agree bit for bit, and rendering frame N
 * alone equals rendering it after frames 0..N-1 (poses are sampled from
 * the precomputed fixed steps, never integrated per frame). */
static void test_softbody_deterministic(sr_test_ctx *t)
{
    SrScene a, b;
    if (st_load(t, "soft-det-a.xml", SOFT_SCENE(""), &a, NULL) != SR_OK) { SR_FAIL(t, "load"); return; }
    if (st_load(t, "soft-det-b.xml", SOFT_SCENE(""), &b, NULL) != SR_OK) {
        SR_FAIL(t, "load"); sr_scene_free(&a); return;
    }
    if (prepare(t, &a, NULL) == SR_OK && prepare(t, &b, NULL) == SR_OK) {
        const SrSoftBody *sa = &sr_scene_find_node(&a, "sheet")->soft_body;
        const SrSoftBody *sb = &sr_scene_find_node(&b, "sheet")->soft_body;
        CHECK_INT(t, sa->sample_count, sb->sample_count);
        CHECK(t, memcmp(sa->offsets, sb->offsets, sa->sample_count * sa->rows *
                        sa->cols * 2 * sizeof(double)) == 0);
        const float black[4] = {0, 0, 0, 1};
        SrFrame alone = {0}, sequence = {0};
        CHECK(t, sr_frame_init(&alone, 400, 300) == SR_OK);
        CHECK(t, sr_frame_init(&sequence, 400, 300) == SR_OK);
        SrCompositor ca, cb;
        sr_compositor_init(&ca, 1);
        sr_compositor_init(&cb, 3);
        if (alone.px && sequence.px) {
            sr_frame_clear(&alone, black, 1);
            CHECK(t, sr_compositor_render(&ca, &a, 25 / 30.0, &alone, NULL) == SR_OK);
            for (int i = 0; i <= 25; ++i) {
                sr_frame_clear(&sequence, black, 1);
                CHECK(t, sr_compositor_render(&cb, &b, i / 30.0, &sequence, NULL) == SR_OK);
            }
            CHECK(t, st_frames_equal(&alone, &sequence));
        }
        sr_frame_free(&alone); sr_frame_free(&sequence);
        sr_compositor_free(&ca); sr_compositor_free(&cb);
    }
    sr_scene_free(&a); sr_scene_free(&b);
}

/* The cache written by the first run is replayed by the second and holds
 * the same soft-body samples. */
static void test_softbody_cache_round_trip(sr_test_ctx *t)
{
    const char *cache = sr_test_tmp_path("soft.physics");
    unlink(cache);
    SrScene a, b;
    if (st_load(t, "soft-cache-a.xml", SOFT_SCENE(" cache=\"soft.physics\""), &a, NULL) != SR_OK) {
        SR_FAIL(t, "load"); return;
    }
    char *info = NULL;
    if (prepare(t, &a, &info) == SR_OK) CHECK_CONTAINS(t, info, "simulated");
    free(info); info = NULL;
    if (st_load(t, "soft-cache-b.xml", SOFT_SCENE(" cache=\"soft.physics\""), &b, NULL) != SR_OK) {
        SR_FAIL(t, "load"); sr_scene_free(&a); return;
    }
    if (prepare(t, &b, &info) == SR_OK) {
        CHECK_CONTAINS(t, info, "loaded physics cache");
        const SrSoftBody *sa = &sr_scene_find_node(&a, "sheet")->soft_body;
        const SrSoftBody *sb = &sr_scene_find_node(&b, "sheet")->soft_body;
        CHECK_INT(t, sa->sample_count, sb->sample_count);
        if (sa->sample_count == sb->sample_count && sb->offsets)
            CHECK(t, memcmp(sa->offsets, sb->offsets, sa->sample_count * sa->rows *
                            sa->cols * 2 * sizeof(double)) == 0);
    }
    free(info);
    sr_scene_free(&a); sr_scene_free(&b);
    unlink(cache);
}

/* A vortex pushes a body at rest tangentially: to the right of the center
 * (y down) the first step moves it straight down, clockwise on screen. */
static void test_vortex_tangential(sr_test_ctx *t)
{
    const char *xml =
        "<scene version=\"1.0\"><project width=\"400\" height=\"300\" fps=\"30\" duration=\"1\"/>"
        "<composition><shape id=\"b\" shape=\"ellipse\" width=\"10\" height=\"10\" x=\"200\" y=\"100\">"
        "<rigidBody shape=\"circle\" radius=\"5\" linearDamping=\"0\"/></shape></composition>"
        "<physics fixedStep=\"0.01\" gravityY=\"0\">"
        "<forceField id=\"swirl\" type=\"vortex\" x=\"100\" y=\"100\" strength=\"500\" falloff=\"0\"/>"
        "</physics></scene>";
    SrScene scene;
    if (st_load(t, "vortex.xml", xml, &scene, NULL) != SR_OK) { SR_FAIL(t, "load"); return; }
    if (prepare(t, &scene, NULL) == SR_OK) {
        const SrNode *body = sr_scene_find_node(&scene, "b");
        const SrPhysicsSample *s = body->physics_samples;
        double vx = (s[1].x.base - s[0].x.base) / 0.01, vy = (s[1].y.base - s[0].y.base) / 0.01;
        CHECK_NEAR(t, vx, 0.0, 1e-9);
        CHECK_NEAR(t, vy, 500.0 * 0.01, 1e-9);
        /* Keeps circulating: after a while it has moved left of its start. */
        CHECK(t, s[40].x.base < 200.0 && s[40].y.base > 100.0);
    }
    sr_scene_free(&scene);
}

/* A rigid pin holds its body on the anchor (restLength 0) or on a circle
 * around it (pendulum, restLength = initial distance). */
static void test_pin_holds_anchor(sr_test_ctx *t)
{
    const char *xml =
        "<scene version=\"1.0\"><project width=\"400\" height=\"300\" fps=\"30\" duration=\"2\"/>"
        "<composition>"
        "<shape id=\"fixed\" shape=\"rect\" width=\"10\" height=\"10\" x=\"50\" y=\"50\">"
        "<rigidBody velocityX=\"30\"/></shape>"
        "<shape id=\"bob\" shape=\"ellipse\" width=\"10\" height=\"10\" x=\"250\" y=\"50\">"
        "<rigidBody shape=\"circle\" radius=\"5\"/></shape></composition>"
        "<physics gravityY=\"500\">"
        "<constraint id=\"p1\" type=\"pin\" a=\"fixed\" x=\"50\" y=\"50\" restLength=\"0\"/>"
        "<constraint id=\"p2\" type=\"pin\" a=\"bob\" x=\"200\" y=\"50\"/>"
        "</physics></scene>";
    SrScene scene;
    if (st_load(t, "pin.xml", xml, &scene, NULL) != SR_OK) { SR_FAIL(t, "load"); return; }
    CHECK_NEAR(t, scene.physics.constraints[1].rest_length, 50.0, 1e-12);
    if (prepare(t, &scene, NULL) == SR_OK) {
        const SrNode *fixed = sr_scene_find_node(&scene, "fixed");
        const SrNode *bob = sr_scene_find_node(&scene, "bob");
        double worst_fixed = 0.0, worst_bob = 0.0, lowest = 0.0;
        for (size_t s = 0; s < bob->physics_sample_count; ++s) {
            const SrPhysicsSample *a = &fixed->physics_samples[s];
            const SrPhysicsSample *b = &bob->physics_samples[s];
            worst_fixed = fmax(worst_fixed, hypot(a->x.base - 50.0, a->y.base - 50.0));
            worst_bob = fmax(worst_bob, fabs(hypot(b->x.base - 200.0, b->y.base - 50.0) - 50.0));
            lowest = fmax(lowest, b->y.base);
        }
        CHECK(t, worst_fixed < 1e-9);
        CHECK(t, worst_bob < 0.05);
        CHECK(t, lowest > 99.0);      /* the pendulum swings through the bottom */
    }
    sr_scene_free(&scene);
}

/* A circle dropped onto the top face of a wide static box, near its end,
 * comes to rest on the face. Bounding circles would have treated the box as
 * a radius-50 disc and shoved the circle sideways and up. */
static void test_circle_box_contact(sr_test_ctx *t)
{
    const char *xml =
        "<scene version=\"1.0\"><project width=\"400\" height=\"300\" fps=\"30\" duration=\"2\"/>"
        "<composition>"
        "<shape id=\"slab\" shape=\"rect\" width=\"100\" height=\"20\" x=\"100\" y=\"100\">"
        "<rigidBody type=\"static\"/></shape>"
        "<shape id=\"ball\" shape=\"ellipse\" width=\"20\" height=\"20\" x=\"140\" y=\"75\">"
        "<rigidBody shape=\"circle\" radius=\"10\" restitution=\"0\" friction=\"0.5\"/></shape>"
        "</composition><physics gravityY=\"300\"/></scene>";
    SrScene scene;
    if (st_load(t, "circle-box.xml", xml, &scene, NULL) != SR_OK) { SR_FAIL(t, "load"); return; }
    if (prepare(t, &scene, NULL) == SR_OK) {
        const SrNode *ball = sr_scene_find_node(&scene, "ball");
        const SrPhysicsSample *end = &ball->physics_samples[ball->physics_sample_count - 1];
        CHECK_NEAR(t, end->y.base, 80.0, 0.5);    /* resting on the face at y = 90 */
        CHECK_NEAR(t, end->x.base, 140.0, 1e-6);
    }
    sr_scene_free(&scene);
}


/* A cache whose header matches but whose payload holds a non-finite soft
 * offset is discarded (with a diagnostic) and the scene re-simulated. */
static void test_cache_payload_validated(sr_test_ctx *t)
{
    const char *cache = sr_test_tmp_path("soft-bad.physics");
    unlink(cache);
    SrScene a, b;
    if (st_load(t, "soft-bad-a.xml", SOFT_SCENE(" cache=\"soft-bad.physics\""), &a, NULL) != SR_OK) {
        SR_FAIL(t, "load"); return;
    }
    char *info = NULL;
    CHECK(t, prepare(t, &a, &info) == SR_OK);
    free(info); info = NULL;
    /* Overwrite the last payload double (a soft offset) with NaN. */
    FILE *file = fopen(cache, "r+b");
    CHECK(t, file != NULL);
    if (file) {
        double nan_value = NAN;
        CHECK(t, fseek(file, -(long)sizeof(double), SEEK_END) == 0);
        CHECK(t, fwrite(&nan_value, sizeof(nan_value), 1, file) == 1);
        CHECK(t, fclose(file) == 0);
    }
    if (st_load(t, "soft-bad-b.xml", SOFT_SCENE(" cache=\"soft-bad.physics\""), &b, NULL) != SR_OK) {
        SR_FAIL(t, "load"); sr_scene_free(&a); return;
    }
    if (prepare(t, &b, &info) == SR_OK) {
        CHECK_CONTAINS(t, info, "discarding it and re-simulating");
        CHECK_CONTAINS(t, info, "simulated");
        const SrSoftBody *sa = &sr_scene_find_node(&a, "sheet")->soft_body;
        const SrSoftBody *sb = &sr_scene_find_node(&b, "sheet")->soft_body;
        CHECK_INT(t, sa->sample_count, sb->sample_count);
        if (sa->sample_count == sb->sample_count && sb->offsets)
            CHECK(t, memcmp(sa->offsets, sb->offsets, sa->sample_count * sa->rows *
                            sa->cols * 2 * sizeof(double)) == 0);
    }
    free(info);
    sr_scene_free(&a); sr_scene_free(&b);
    unlink(cache);
}

/* The review's stiff sheet (2x2, mass 1, stiffness 1e12, default step)
 * needs 188,562 substeps: validation rejects it naming stiffness, mass and
 * fixedStep, whichever of <softBody> and <physics> comes first. */
static void test_soft_body_substeps_validated(sr_test_ctx *t)
{
    const char *stiff =
        "<scene version=\"1.0\"><project width=\"400\" height=\"300\" fps=\"30\" duration=\"1\"/>"
        "<composition><shape id=\"sheet\" shape=\"rect\" width=\"60\" height=\"60\" x=\"100\" y=\"40\">"
        "<softBody mass=\"1\" stiffness=\"1e12\" damping=\"0.1\" rows=\"2\" cols=\"2\" pin=\"top\"/>"
        "</shape></composition></scene>";
    SrSoftBody body = {.enabled = true, .mass = 1, .stiffness = 1e12, .damping = 0.1,
                       .rows = 2, .cols = 2};
    CHECK_NEAR(t, sr_soft_body_substeps(&body, false, 1.0 / 120.0), 188562.0, 0.0);
    SrScene scene;
    char *message = NULL;
    CHECK(t, st_load(t, "soft-stiff.xml", stiff, &scene, &message) == SR_ERR_XML);
    CHECK_CONTAINS(t, message, "stiffness 1e+12 with mass 1");
    CHECK_CONTAINS(t, message, "fixedStep");
    free(message); message = NULL;
    /* Fine at the default step, too stiff for the step <physics> sets later. */
    const char *late =
        "<scene version=\"1.0\"><project width=\"400\" height=\"300\" fps=\"30\" duration=\"1\"/>"
        "<composition><shape id=\"sheet\" shape=\"rect\" width=\"60\" height=\"60\" x=\"100\" y=\"40\">"
        "<softBody mass=\"1\" stiffness=\"200000\" rows=\"2\" cols=\"2\" pin=\"top\"/>"
        "</shape></composition><physics fixedStep=\"0.5\"/></scene>";
    CHECK(t, st_load(t, "soft-late.xml", late, &scene, &message) == SR_ERR_XML);
    CHECK_CONTAINS(t, message, "fixedStep 0.5");
    free(message);
    const char *fine =
        "<scene version=\"1.0\"><project width=\"400\" height=\"300\" fps=\"30\" duration=\"1\"/>"
        "<composition><shape id=\"sheet\" shape=\"rect\" width=\"60\" height=\"60\" x=\"100\" y=\"40\">"
        "<softBody mass=\"1\" stiffness=\"200000\" rows=\"2\" cols=\"2\" pin=\"top\"/>"
        "</shape></composition></scene>";
    if (st_load(t, "soft-fine.xml", fine, &scene, NULL) == SR_OK) sr_scene_free(&scene);
    else SR_FAIL(t, "a stable stiff sheet must load");
}

/* A simulation that diverges (an overflowing field) fails with a
 * diagnostic instead of recording non-finite samples. */
static void test_divergence_is_an_error(sr_test_ctx *t)
{
    static const char *const bodies[] = {
        "<shape id=\"b\" shape=\"rect\" width=\"10\" height=\"10\" x=\"100\" y=\"40\">"
        "<rigidBody/></shape>",
        "<shape id=\"b\" shape=\"rect\" width=\"60\" height=\"60\" x=\"100\" y=\"40\">"
        "<softBody rows=\"3\" cols=\"3\"/></shape>"};
    static const char *const kinds[] = {"diverged", "soft body 'b' diverged"};
    for (int i = 0; i < 2; ++i) {
        char xml[1024];
        snprintf(xml, sizeof xml,
                 "<scene version=\"1.0\"><project width=\"400\" height=\"300\" fps=\"30\" "
                 "duration=\"1\"/><composition>%s</composition><physics gravityY=\"1e308\">"
                 "<forceField id=\"f\" type=\"directional\" forceY=\"1e308\"/>"
                 "</physics></scene>", bodies[i]);
        SrScene scene;
        if (st_load(t, "diverge.xml", xml, &scene, NULL) != SR_OK) { SR_FAIL(t, "load"); continue; }
        FILE *sink = tmpfile();
        SrDiagnostics diag;
        sr_diag_init(&diag, "physics", sink ? sink : stderr);
        CHECK_INT(t, sr_physics_prepare(&scene, &diag), SR_ERR_RENDER);
        if (sink) {
            char text[1024] = {0};
            fflush(sink);
            rewind(sink);
            if (fread(text, 1, sizeof text - 1, sink) == 0) text[0] = 0;
            CHECK_CONTAINS(t, text, kinds[i]);
            fclose(sink);
        }
        const SrNode *node = sr_scene_find_node(&scene, "b");
        CHECK_INT(t, node->physics_sample_count, 0);
        CHECK_INT(t, node->soft_body.sample_count, 0);
        sr_scene_free(&scene);
    }
}

const sr_test_case sr_tests_physics[] = {
    {"heavy_damping_never_flips_sign", test_heavy_damping_never_flips_sign},
    {"softbody_pinned_top_sags_and_rests", test_softbody_pinned_top_sags_and_rests},
    {"softbody_deterministic", test_softbody_deterministic},
    {"softbody_cache_round_trip", test_softbody_cache_round_trip},
    {"vortex_tangential", test_vortex_tangential},
    {"pin_holds_anchor", test_pin_holds_anchor},
    {"circle_box_contact", test_circle_box_contact},
    {"cache_payload_validated", test_cache_payload_validated},
    {"soft_body_substeps_validated", test_soft_body_substeps_validated},
    {"divergence_is_an_error", test_divergence_is_an_error},
    {NULL, NULL},
};
