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


/* Reference for animated rates: the whole-history trapezoid grid from the
 * emitter start (the original algorithm), births linear inside each cell.
 * Fills births[] newest first for indices alive-candidate at `now`. */
static size_t reference_births(const SrNode *node, double now, double longest,
                               double *index, double *birth, size_t capacity)
{
    const double step = 1.0 / 240.0;
    size_t cells = (size_t)ceil(now / step) + 1, count = 0;
    double *total = calloc(cells + 1, sizeof(*total));
    if (!total) return 0;
    double previous = fmax(0.0, sr_anim_eval(&node->particle_rate, node->start_time));
    for (size_t k = 0; k < cells; ++k) {
        double next = fmax(0.0, sr_anim_eval(&node->particle_rate,
            node->start_time + (double)(k + 1) * step));
        total[k + 1] = total[k] + 0.5 * (previous + next) * step;
        previous = next;
    }
    for (size_t k = cells; k-- > 0;) {
        double t0 = (double)k * step;
        if (t0 + step < now - longest) break;
        double n0 = total[k], n1 = total[k + 1];
        if (!(n1 > n0)) continue;
        for (double i = ceil(n1) - 1.0; i >= ceil(n0); i -= 1.0) {
            double b = t0 + (i - n0) / (n1 - n0) * step;
            if (b > now || now - b > longest || count == capacity) continue;
            index[count] = i;
            birth[count++] = b;
        }
    }
    free(total);
    return count;
}

/* With speed s, direction 0 and no spread, variance, gravity or emitter
 * size, a particle's x is s * age: its birth is now - x / s. The segment
 * cache (bit-identical grid between keys, closed form outside them)
 * reproduces the whole-history grid: exactly between the keys, to
 * rounding before the first key and after the last. */
static void test_keyed_rate_matches_grid(sr_test_ctx *t)
{
    const char *xml =
        "<scene version=\"1.0\"><project width=\"64\" height=\"64\" fps=\"30\" duration=\"8\"/>"
        "<composition>"
        "<particleEmitter id=\"a\" start=\"0.25\" rate=\"1\" lifetime=\"0.93\" direction=\"0\" "
        "speed=\"100\" spread=\"0\" maxParticles=\"100000\">"
        "<animate property=\"rate\"><key time=\"0\" value=\"30\"/><key time=\"0.37\" value=\"400\"/>"
        "<key time=\"1.1\" value=\"0\"/><key time=\"1.3\" value=\"0\"/><key time=\"2.05\" value=\"90\"/></animate>"
        "</particleEmitter>"
        "<particleEmitter id=\"b\" rate=\"1\" lifetime=\"0.93\" direction=\"0\" speed=\"100\" "
        "spread=\"0\" maxParticles=\"100000\">"
        "<animate property=\"rate\"><key time=\"1.013\" value=\"50\"/><key time=\"2.5\" value=\"300\"/></animate>"
        "</particleEmitter>"
        "</composition></scene>";
    SrScene scene;
    if (st_load(t, "particles-grid.xml", xml, &scene, NULL) != SR_OK) { SR_FAIL(t, "load"); return; }
    static const char *const ids[] = {"a", "b"};
    static const double times[] = {0.3, 0.8, 1.2, 1.7, 2.2, 3.0, 5.5, 7.9};
    double index[4096], birth[4096];
    for (size_t e = 0; e < 2; ++e) {
        const SrNode *node = sr_scene_find_node(&scene, ids[e]);
        for (size_t k = 0; k < sizeof(times) / sizeof(times[0]); ++k) {
            double now = times[k] - node->start_time;
            size_t expected = reference_births(node, now, 0.93, index, birth, 4096);
            SrParticle *particles = NULL;
            size_t count = 0;
            CHECK(t, sr_particles_eval(&scene, node, times[k], &particles, &count) == SR_OK);
            if (count != expected) {
                SR_FAIL(t, "%s at %g: %zu particles, reference %zu", ids[e], times[k],
                        count, expected);
            } else {
                for (size_t i = 0; i < count; ++i) {
                    /* particles are oldest first, the reference newest first */
                    const SrParticle *p = &particles[count - 1 - i];
                    if ((double)p->index != index[i] ||
                        fabs((now - p->x / 100.0) - birth[i]) > 1e-9)
                        SR_FAIL(t, "%s at %g: particle %zu differs", ids[e], times[k], i);
                }
            }
            free(particles);
        }
    }
    sr_scene_free(&scene);
}

/* The per-evaluation work no longer grows with time: after the last key
 * the count is closed form, so an evaluation at 1e6 s is as fast as at
 * 10 s and yields the same live population; the review's 9.6e15 s time
 * (whose grid would need 2^61 cells) returns promptly. */
static void test_keyed_rate_bounded_late_time(sr_test_ctx *t)
{
    const char *xml =
        "<scene version=\"1.0\"><project width=\"64\" height=\"64\" fps=\"30\" duration=\"1000000\"/>"
        "<composition><particleEmitter id=\"p\" rate=\"1\" lifetime=\"1\" direction=\"0\" "
        "speed=\"10\" spread=\"0\">"
        "<animate property=\"rate\"><key time=\"0\" value=\"40\"/><key time=\"1\" value=\"100\"/></animate>"
        "</particleEmitter></composition></scene>";
    SrScene scene;
    if (st_load(t, "particles-late.xml", xml, &scene, NULL) != SR_OK) { SR_FAIL(t, "load"); return; }
    const SrNode *node = scene.root->children[0];
    SrParticle *particles = NULL;
    size_t early = 0, late = 0;
    CHECK(t, sr_particles_eval(&scene, node, 10.0, &particles, &early) == SR_OK);
    free(particles);
    CHECK(t, sr_particles_eval(&scene, node, 999999.5, &particles, &late) == SR_OK);
    free(particles);
    CHECK(t, early >= 99 && early <= 101);
    CHECK(t, late >= 99 && late <= 101);
    size_t huge = 0;
    CHECK(t, sr_particles_eval(&scene, node, 9607679205057058.0, &particles, &huge) == SR_OK);
    free(particles);
    sr_scene_free(&scene);
}

/* Project durations are bounded (the keyed-rate work is bounded by it). */
static void test_duration_bound(sr_test_ctx *t)
{
    SrScene scene;
    char *message = NULL;
    CHECK(t, st_load(t, "duration-huge.xml",
                     "<scene version=\"1.0\"><project width=\"8\" height=\"8\" fps=\"30\" "
                     "duration=\"1000001\"/><composition/></scene>", &scene, &message) == SR_ERR_XML);
    CHECK_CONTAINS(t, message, "at most 1e6 seconds");
    free(message);
    if (st_load(t, "duration-max.xml",
                "<scene version=\"1.0\"><project width=\"8\" height=\"8\" fps=\"30\" "
                "duration=\"1000000\"/><composition/></scene>", &scene, NULL) == SR_OK)
        sr_scene_free(&scene);
    else
        SR_FAIL(t, "a 1e6 s duration must load");
}

const sr_test_case sr_tests_particles[] = {
    {"frame_independent", test_frame_independent},
    {"thread_invariant", test_thread_invariant},
    {"max_particles_caps", test_max_particles_caps},
    {"presets_produce_particles", test_presets_produce_particles},
    {"emission_time_parameters", test_emission_time_parameters},
    {"keyed_rate_matches_grid", test_keyed_rate_matches_grid},
    {"keyed_rate_bounded_late_time", test_keyed_rate_bounded_late_time},
    {"duration_bound", test_duration_bound},
    {NULL, NULL},
};
