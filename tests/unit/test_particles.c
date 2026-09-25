/* SPDX-License-Identifier: Apache-2.0 */
/* Parametric particles: stateless per frame, thread invariant, capped. */
#include "scene_render/particles.h"

#include "scene_text.h"

static const char *const emitter_scene =
    "<scene version=\"1.0\"><project width=\"96\" height=\"64\" fps=\"30\" "
    "duration=\"2\" seed=\"7\" linearLight=\"false\"/><composition>"
    "<particleEmitter id=\"burst\" x=\"48\" y=\"40\" rate=\"120\" lifetime=\"0.8\" "
    "lifetimeVariance=\"0.2\" speed=\"60\" speedVariance=\"20\" direction=\"-90\" "
    "spread=\"40\" gravityY=\"90\" size=\"2\" sizeEnd=\"0.5\" color=\"#FFE070\" "
    "colorEnd=\"#FF402000\" emitterWidth=\"10\" blend=\"add\">"
    "<animate property=\"rate\"><key time=\"0\" value=\"40\"/><key time=\"1\" value=\"200\"/></animate>"
    "<animate property=\"direction\"><key time=\"0\" value=\"-120\"/><key time=\"2\" value=\"-60\"/></animate>"
    "</particleEmitter>"
    "<particleEmitter id=\"sq\" x=\"20\" y=\"20\" rate=\"30\" lifetime=\"1\" speed=\"20\" "
    "spread=\"180\" size=\"1.5\" shape=\"square\"/>"
    "</composition></scene>";

static bool render_with(sr_test_ctx *t, SrCompositor *compositor, SrScene *scene,
                        double time, SrFrame *frame)
{
    const float black[4] = {0, 0, 0, 1};
    if (!frame->px && sr_frame_init(frame, scene->project.width,
                                    scene->project.height) != SR_OK) {
        SR_FAIL(t, "frame allocation failed");
        return false;
    }
    sr_frame_clear(frame, black, 1);
    SrStatus status = sr_compositor_render(compositor, scene, time, frame, NULL);
    CHECK(t, status == SR_OK);
    return status == SR_OK;
}

/* Frame N rendered alone equals frame N rendered after frames 0..N-1. */
static void test_frame_independent(sr_test_ctx *t)
{
    SrScene scene;
    if (st_load(t, "particles-a.xml", emitter_scene, &scene, NULL) != SR_OK) {
        SR_FAIL(t, "load"); return;
    }
    const int n = 40;
    double time = n / 30.0;
    SrCompositor alone, sequence;
    sr_compositor_init(&alone, 1);
    sr_compositor_init(&sequence, 1);
    SrFrame a = {0}, b = {0};
    bool ok = render_with(t, &alone, &scene, time, &a);
    for (int i = 0; ok && i <= n; ++i) ok = render_with(t, &sequence, &scene, i / 30.0, &b);
    if (ok) CHECK(t, st_frames_equal(&a, &b));
    sr_frame_free(&a); sr_frame_free(&b);
    sr_compositor_free(&alone); sr_compositor_free(&sequence);
    sr_scene_free(&scene);
}

/* One and seven compositor threads give byte-identical frames. */
static void test_thread_invariant(sr_test_ctx *t)
{
    SrScene scene;
    if (st_load(t, "particles-b.xml", emitter_scene, &scene, NULL) != SR_OK) {
        SR_FAIL(t, "load"); return;
    }
    SrCompositor one, seven;
    sr_compositor_init(&one, 1);
    sr_compositor_init(&seven, 7);
    SrFrame a = {0}, b = {0};
    for (int i = 0; i < 60; i += 7)
        if (render_with(t, &one, &scene, i / 30.0, &a) &&
            render_with(t, &seven, &scene, i / 30.0, &b))
            CHECK(t, st_frames_equal(&a, &b));
    sr_frame_free(&a); sr_frame_free(&b);
    sr_compositor_free(&one); sr_compositor_free(&seven);
    sr_scene_free(&scene);
}

/* maxParticles bounds the live count and keeps the newest particles. */
static void test_max_particles_caps(sr_test_ctx *t)
{
    const char *xml =
        "<scene version=\"1.0\"><project width=\"64\" height=\"64\" fps=\"30\" duration=\"4\"/>"
        "<composition>"
        "<particleEmitter id=\"free\" rate=\"1000\" lifetime=\"2\" speed=\"10\"/>"
        "<particleEmitter id=\"capped\" rate=\"1000\" lifetime=\"2\" speed=\"10\" maxParticles=\"50\"/>"
        "</composition></scene>";
    SrScene scene;
    if (st_load(t, "particles-cap.xml", xml, &scene, NULL) != SR_OK) { SR_FAIL(t, "load"); return; }
    SrParticle *particles = NULL;
    size_t count = 0;
    CHECK(t, sr_particles_eval(&scene, sr_scene_find_node(&scene, "free"), 1.0,
                               &particles, &count) == SR_OK);
    CHECK_INT(t, count, 1001);
    free(particles);
    CHECK(t, sr_particles_eval(&scene, sr_scene_find_node(&scene, "capped"), 1.0,
                               &particles, &count) == SR_OK);
    CHECK_INT(t, count, 50);
    if (count == 50) {
        CHECK_INT(t, particles[49].index, 1000);   /* newest kept, drawn last */
        CHECK_INT(t, particles[0].index, 951);
    }
    free(particles);
    sr_scene_free(&scene);
}

/* Every preset (now optional) still produces particles that draw. */
static void test_presets_produce_particles(sr_test_ctx *t)
{
    static const char *const presets[] = {"smoke", "sparks", "dust", "rain"};
    for (size_t i = 0; i < 4; ++i) {
        char xml[768];
        snprintf(xml, sizeof xml,
                 "<scene version=\"1.0\"><project width=\"96\" height=\"96\" fps=\"30\" "
                 "duration=\"2\"/><composition><particleEmitter id=\"p\" preset=\"%s\" "
                 "x=\"48\" y=\"48\" rate=\"30\" lifetime=\"1\" speed=\"40\" spread=\"30\" "
                 "size=\"3\"/></composition></scene>", presets[i]);
        SrScene scene;
        if (st_load(t, "particles-preset.xml", xml, &scene, NULL) != SR_OK) {
            SR_FAIL(t, "load %s", presets[i]); continue;
        }
        SrParticle *particles = NULL;
        size_t count = 0;
        CHECK(t, sr_particles_eval(&scene, scene.root->children[0], 0.5, &particles,
                                   &count) == SR_OK);
        if (count < 10) SR_FAIL(t, "%s produced %zu particles", presets[i], count);
        free(particles);
        SrCompositor compositor;
        sr_compositor_init(&compositor, 1);
        SrFrame frame = {0};
        if (render_with(t, &compositor, &scene, 0.5, &frame)) {
            double lit = 0.0;
            for (size_t p = 0; p < (size_t)96 * 96; ++p) lit += frame.px[p * 4];
            if (!(lit > 1.0)) SR_FAIL(t, "%s drew nothing", presets[i]);
        }
        sr_frame_free(&frame);
        sr_compositor_free(&compositor);
        sr_scene_free(&scene);
    }
}

/* Parameters are sampled at emission: a particle keeps the speed it was
 * born with although the track later changes. */
static void test_emission_time_parameters(sr_test_ctx *t)
{
    const char *xml =
        "<scene version=\"1.0\"><project width=\"64\" height=\"64\" fps=\"30\" duration=\"4\"/>"
        "<composition><particleEmitter id=\"p\" rate=\"1\" lifetime=\"5\" direction=\"0\" "
        "colorEnd=\"#FFFFFFFF\">"
        "<animate property=\"speed\" defaultInterpolation=\"step\"><key time=\"0\" value=\"10\"/>"
        "<key time=\"0.5\" value=\"1000\"/></animate>"
        "</particleEmitter></composition></scene>";
    SrScene scene;
    if (st_load(t, "particles-birth.xml", xml, &scene, NULL) != SR_OK) { SR_FAIL(t, "load"); return; }
    SrParticle *particles = NULL;
    size_t count = 0;
    CHECK(t, sr_particles_eval(&scene, scene.root->children[0], 2.0, &particles,
                               &count) == SR_OK);
    CHECK_INT(t, count, 3);
    if (count == 3) {
        CHECK_NEAR(t, particles[0].x, 20.0, 1e-9);     /* born at 0: 10 px/s */
        CHECK_NEAR(t, particles[1].x, 1000.0, 1e-9);   /* born at 1 */
        CHECK_NEAR(t, particles[2].x, 0.0, 1e-9);      /* born at 2 */
        CHECK_NEAR(t, particles[0].color.a, 1.0, 1e-12);
    }
    free(particles);
    sr_scene_free(&scene);
}

const sr_test_case sr_tests_particles[] = {
    {"frame_independent", test_frame_independent},
    {"thread_invariant", test_thread_invariant},
    {"max_particles_caps", test_max_particles_caps},
    {"presets_produce_particles", test_presets_produce_particles},
    {"emission_time_parameters", test_emission_time_parameters},
    {NULL, NULL},
};
