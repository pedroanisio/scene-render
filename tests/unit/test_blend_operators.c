/* SPDX-License-Identifier: Apache-2.0 */
/* B1-3 flattened-node operators: dissolve, stencil/silhouette alpha and
 * luma, alpha-add and behind (docs/design/b1-3-compositing.md). */
#include "scene_render/raster.h"
#include "scene_render/random.h"
#include "b13_fixture.h"

#define ARRAY_COUNT(a) (sizeof(a) / sizeof((a)[0]))

static void kernel_references(sr_test_ctx *t) {
    const float d0[4] = {.2f, .1f, .05f, .5f};
    const float s0[4] = {.3f, .3f, .3f, .6f};
    float d[4];
    memcpy(d, d0, sizeof(d));
    sr_blend_operator_px(SR_BLEND_BEHIND, NULL, d, s0);
    CHECK_NEAR(t, d[0], .2 + .5 * .3, 1e-7);
    CHECK_NEAR(t, d[1], .1 + .5 * .3, 1e-7);
    CHECK_NEAR(t, d[3], .5 + .5 * .6, 1e-7);
    memcpy(d, d0, sizeof(d));
    sr_blend_operator_px(SR_BLEND_STENCIL_ALPHA, NULL, d, s0);
    for (int c = 0; c < 4; ++c) CHECK_NEAR(t, d[c], d0[c] * .6, 1e-7);
    memcpy(d, d0, sizeof(d));
    sr_blend_operator_px(SR_BLEND_SILHOUETTE_ALPHA, NULL, d, s0);
    for (int c = 0; c < 4; ++c) CHECK_NEAR(t, d[c], d0[c] * .4, 1e-7);
    memcpy(d, d0, sizeof(d));
    sr_blend_operator_px(SR_BLEND_ALPHA_ADD, NULL, d, s0);
    CHECK_NEAR(t, d[3], 1.0, 0);
    for (int c = 0; c < 3; ++c)
        CHECK_NEAR(t, d[c], ((double)d0[c] + s0[c]) / 1.1, 1e-7);
    const float s1[4] = {.1f, .05f, 0, .25f};
    memcpy(d, d0, sizeof(d));
    sr_blend_operator_px(SR_BLEND_ALPHA_ADD, NULL, d, s1);
    CHECK_NEAR(t, d[3], .75, 1e-7);
    CHECK_NEAR(t, d[0], .3, 1e-7);
    const float clear[4] = {0, 0, 0, 0};
    float empty[4] = {0, 0, 0, 0};
    sr_blend_operator_px(SR_BLEND_ALPHA_ADD, NULL, empty, clear);
    for (int c = 0; c < 4; ++c) CHECK_NEAR(t, empty[c], 0, 0);
    /* A transparent source is significant: stencil clears, the others keep. */
    memcpy(d, d0, sizeof(d));
    sr_blend_operator_px(SR_BLEND_STENCIL_ALPHA, NULL, d, clear);
    for (int c = 0; c < 4; ++c) CHECK_NEAR(t, d[c], 0, 0);
    static const SrBlendMode identity[] = {SR_BLEND_SILHOUETTE_ALPHA,
        SR_BLEND_SILHOUETTE_LUMA, SR_BLEND_BEHIND, SR_BLEND_ALPHA_ADD};
    SrProject project = {.linear_light = true};
    SrLumaConfig luma;
    sr_luma_config_init(&project, &luma);
    for (size_t i = 0; i < ARRAY_COUNT(identity); ++i) {
        memcpy(d, d0, sizeof(d));
        sr_blend_operator_px(identity[i], &luma, d, clear);
        for (int c = 0; c < 4; ++c) CHECK_NEAR(t, d[c], d0[c], 1e-7);
    }
}

static void luma_operators(sr_test_ctx *t) {
    SrProject linear = {.linear_light = true, .working_color_space = SR_COLOR_SRGB};
    SrProject encoded = {.linear_light = false, .working_color_space = SR_COLOR_SRGB};
    SrLumaConfig a, b;
    sr_luma_config_init(&linear, &a);
    sr_luma_config_init(&encoded, &b);
    CHECK_NEAR(t, a.row[0], .21267290, 1e-12);
    CHECK_NEAR(t, a.row[1], .71515220, 1e-12);
    CHECK_NEAR(t, a.row[2], .07217500, 1e-12);
    SrProject p3 = {.linear_light = true, .working_color_space = SR_COLOR_DISPLAY_P3};
    SrProject bt2020 = {.linear_light = true, .working_color_space = SR_COLOR_REC2020};
    SrLumaConfig c;
    sr_luma_config_init(&p3, &c);
    CHECK_NEAR(t, c.row[0], .22897456, 1e-12);
    sr_luma_config_init(&bt2020, &c);
    CHECK_NEAR(t, c.row[1], .67799807, 1e-12);
    /* Straight grey .5 at alpha .5 (premultiplied .25). */
    const float grey[4] = {.25f, .25f, .25f, .5f};
    double sum = a.row[0] + a.row[1] + a.row[2];
    CHECK_NEAR(t, sr_luma_px(&a, grey), .5 * sum, 1e-9);
    CHECK_NEAR(t, sr_luma_px(&b, grey), sr_color_decode(.5, SR_COLOR_SRGB) * sum, 1e-9);
    const float hdr[4] = {4, -1, .5f, 1};
    CHECK_NEAR(t, sr_luma_px(&a, hdr), a.row[0] + .5 * a.row[2], 1e-9);
    CHECK_NEAR(t, sr_luma_px(&a, (float[4]){1, 1, 1, 0}), 0, 0);
    const float d0[4] = {.4f, .2f, .1f, .8f};
    float d[4];
    memcpy(d, d0, sizeof(d));
    sr_blend_operator_px(SR_BLEND_STENCIL_LUMA, &b, d, grey);
    double k = .5 * sr_luma_px(&b, grey);
    for (int i = 0; i < 4; ++i) CHECK_NEAR(t, d[i], d0[i] * k, 1e-7);
    memcpy(d, d0, sizeof(d));
    sr_blend_operator_px(SR_BLEND_SILHOUETTE_LUMA, &b, d, grey);
    for (int i = 0; i < 4; ++i) CHECK_NEAR(t, d[i], d0[i] * (1 - k), 1e-7);
}

static void dissolve_kernel_and_random(sr_test_ctx *t) {
    const float src[4] = {.25f, .1f, .05f, .5f};
    float d[4] = {.1f, .1f, .1f, 1};
    sr_dissolve_px(d, src, .4999);
    CHECK_NEAR(t, d[0], .5, 1e-7);
    CHECK_NEAR(t, d[1], .2, 1e-7);
    CHECK_NEAR(t, d[3], 1, 0);
    float e[4] = {.1f, .1f, .1f, 1};
    sr_dissolve_px(e, src, .5);
    CHECK_NEAR(t, e[0], .1, 1e-7);
    /* Canonical FNV-1a over "ab", a zero byte and "d", then XOR seed. */
    uint64_t hash = UINT64_C(14695981039346656037);
    const unsigned char bytes[] = {'a', 'b', 0, 'd'};
    for (size_t i = 0; i < sizeof(bytes); ++i) {
        hash ^= bytes[i];
        hash *= UINT64_C(1099511628211);
    }
    CHECK(t, sr_random_property_seed(0, "ab", "d") == hash);
    CHECK(t, sr_random_property_seed(7, "ab", "d") == (hash ^ 7));
    CHECK(t, sr_random_property_seed(0, NULL, NULL) ==
             UINT64_C(14695981039346656037) * UINT64_C(1099511628211));
    double mean = 0.0;
    size_t count = 0, swapped = 0;
    for (int64_t y = -40; y < 40; ++y) {
        for (int64_t x = -40; x < 40; ++x) {
            double u = sr_random_pixel_value(hash, x, y);
            CHECK(t, u >= 0.0 && u < 1.0);
            CHECK(t, u == sr_random_pixel_value(hash, x, y));
            swapped += x != y && u == sr_random_pixel_value(hash, y, x);
            mean += u;
            ++count;
        }
    }
    CHECK_INT(t, swapped, 0);
    CHECK_NEAR(t, mean / (double)count, .5, .02);
    CHECK(t, sr_random_pixel_value(hash, 3, 4) != sr_random_pixel_value(hash + 1, 3, 4));
}

#define PROJECT32 "<project width=\"32\" height=\"32\" fps=\"2\" duration=\"2\" " \
                  "linearLight=\"false\" background=\"#102030\"/>\n"

static void parent_expected(const SrProject *project, SrColor parent_color,
                            SrColor op_color, SrBlendMode mode, float out[4]) {
    float d[4], s[4], background[4];
    sr_color_to_blend(project, parent_color, d);
    sr_color_to_blend(project, op_color, s);
    SrLumaConfig luma;
    sr_luma_config_init(project, &luma);
    sr_blend_operator_px(mode, &luma, d, s);
    sr_color_to_blend(project, project->background, background);
    memcpy(out, background, sizeof(background));
    sr_blend_px(SR_BLEND_NORMAL, out, d);
}

static void isolated_parent_operators(sr_test_ctx *t) {
    static const char *const modes[] = {"stencil-alpha", "stencil-luma",
        "silhouette-alpha", "silhouette-luma", "behind", "alpha-add"};
    static const SrBlendMode values[] = {SR_BLEND_STENCIL_ALPHA,
        SR_BLEND_STENCIL_LUMA, SR_BLEND_SILHOUETTE_ALPHA,
        SR_BLEND_SILHOUETTE_LUMA, SR_BLEND_BEHIND, SR_BLEND_ALPHA_ADD};
    const SrColor parent = {1, 128.0 / 255, 0, 191.0 / 255};
    const SrColor op = {204.0 / 255, 153.0 / 255, 102.0 / 255, 128.0 / 255};
    const SrColor clear = {0, 0, 0, 0};
    for (size_t m = 0; m < ARRAY_COUNT(modes); ++m) {
        char xml[1024];
        snprintf(xml, sizeof(xml), "<scene version=\"1.1\">\n" PROJECT32
                 "<composition><group id=\"p\">"
                 "<shape id=\"b\" shape=\"rect\" width=\"32\" height=\"32\" fill=\"#FF8000BF\"/>"
                 "<shape id=\"s\" shape=\"rect\" x=\"8\" y=\"8\" width=\"16\" height=\"16\" "
                 "fill=\"#CC996680\" blend=\"%s\"/></group></composition></scene>",
                 modes[m]);
        SrScene scene;
        char *message = NULL;
        SrStatus status = b13_load(t, xml, &scene, &message);
        CHECK_INT(t, status, SR_OK);
        free(message);
        if (status != SR_OK) continue;
        SrFrame one, four;
        CHECK_INT(t, b13_render(&scene, 0, 1, &one, NULL), SR_OK);
        CHECK_INT(t, b13_render(&scene, .5, 4, &four, NULL), SR_OK);
        CHECK(t, st_frames_equal(&one, &four));
        float inside[4], outside[4];
        parent_expected(&scene.project, parent, op, values[m], inside);
        parent_expected(&scene.project, parent, clear, values[m], outside);
        CHECK(t, b13_near4(st_px(&one, 16, 16), inside, 2e-6));
        CHECK(t, b13_near4(st_px(&one, 2, 2), outside, 2e-6));
        sr_frame_free(&one);
        sr_frame_free(&four);
        sr_scene_free(&scene);
    }
}

static void stencil_lifecycle(sr_test_ctx *t) {
    /* An empty active stencil clears the parent; an inactive one is absent;
     * at the root the backdrop (background) itself is cut. */
    const char *const bodies[] = {
        "<group id=\"p\"><shape id=\"b\" shape=\"rect\" width=\"32\" height=\"32\" "
        "fill=\"#FF8000\"/><shape id=\"s\" shape=\"rect\" width=\"4\" height=\"4\" "
        "fill=\"#FFFFFF00\" blend=\"stencil-alpha\"/></group>",
        "<group id=\"p\"><shape id=\"b\" shape=\"rect\" width=\"32\" height=\"32\" "
        "fill=\"#FF8000\"/><shape id=\"s\" shape=\"rect\" width=\"4\" height=\"4\" "
        "start=\"1\" fill=\"#FFFFFF\" blend=\"stencil-alpha\"/></group>",
        "<shape id=\"s\" shape=\"rect\" x=\"8\" y=\"8\" width=\"16\" height=\"16\" "
        "fill=\"#FFFFFF\" blend=\"stencil-alpha\"/>",
        "<group id=\"p\"><shape id=\"b\" shape=\"rect\" width=\"32\" height=\"32\" "
        "fill=\"#FF8000\"/><shape id=\"s\" shape=\"rect\" width=\"4\" height=\"4\" "
        "opacity=\"0\" fill=\"#FFFFFF\" blend=\"stencil-alpha\"/></group>"};
    for (size_t i = 0; i < ARRAY_COUNT(bodies); ++i) {
        char xml[1024];
        snprintf(xml, sizeof(xml), "<scene version=\"1.1\">\n" PROJECT32
                 "<composition>%s</composition></scene>", bodies[i]);
        SrFrame frame;
        SrStatus status = b13_render_xml(t, xml, 0, 1, &frame, NULL);
        CHECK_INT(t, status, SR_OK);
        if (status != SR_OK) continue;
        SrProject project = {.working_color_space = SR_COLOR_SRGB,
                             .background = {16 / 255.0, 32 / 255.0, 48 / 255.0, 1}};
        float background[4], orange[4];
        sr_color_to_blend(&project, project.background, background);
        sr_color_to_blend(&project, (SrColor){1, 128 / 255.0, 0, 1}, orange);
        const float zero[4] = {0, 0, 0, 0};
        if (i == 0) {
            CHECK(t, b13_near4(st_px(&frame, 20, 20), background, 0));
        } else if (i == 1 || i == 3) {
            CHECK(t, b13_near4(st_px(&frame, 1, 1), orange, 0));
            CHECK(t, b13_near4(st_px(&frame, 20, 20), orange, 0));
        } else {
            CHECK(t, b13_near4(st_px(&frame, 2, 2), zero, 0));
            CHECK(t, b13_near4(st_px(&frame, 16, 16), background, 1e-6));
        }
        sr_frame_free(&frame);
    }
}

static size_t kept_pixels(const SrFrame *frame, const float color[4], int x0,
                          int y0, int x1, int y1) {
    size_t kept = 0;
    for (int y = y0; y < y1; ++y)
        for (int x = x0; x < x1; ++x)
            kept += b13_near4(st_px(frame, (uint32_t)x, (uint32_t)y), color, 1e-6);
    return kept;
}

static void dissolve_render(sr_test_ctx *t) {
#define DISSOLVE_FORMAT "<scene version=\"1.1\">\n" \
        "<project width=\"64\" height=\"64\" fps=\"2\" duration=\"2\" seed=\"%d\" " \
        "background=\"#00000000\"/>\n<composition>" \
        "<shape id=\"d\" shape=\"rect\" x=\"%d\" y=\"0\" width=\"64\" height=\"64\" " \
        "fill=\"#40C08080\" blend=\"dissolve\"/></composition></scene>"
    char xml[1024];
    snprintf(xml, sizeof(xml), DISSOLVE_FORMAT, 3, 0);
    SrScene scene;
    CHECK_INT(t, b13_load(t, xml, &scene, NULL), SR_OK);
    SrFrame a, b, c;
    CHECK_INT(t, b13_render(&scene, 0, 1, &a, NULL), SR_OK);
    CHECK_INT(t, b13_render(&scene, 1.5, 4, &b, NULL), SR_OK);
    CHECK(t, st_frames_equal(&a, &b));
    float opaque[4];
    sr_color_to_blend(&scene.project, (SrColor){64 / 255.0, 192 / 255.0, 128 / 255.0, 1},
                      opaque);
    size_t kept = kept_pixels(&a, opaque, 0, 0, 64, 64);
    size_t transparent = kept_pixels(&a, (float[4]){0, 0, 0, 0}, 0, 0, 64, 64);
    CHECK_INT(t, kept + transparent, 64 * 64);
    CHECK(t, kept > 64 * 64 * 45 / 100 && kept < 64 * 64 * 55 / 100);
    sr_scene_free(&scene);
    /* Choices follow composition pixels, not the node's position. */
    snprintf(xml, sizeof(xml), DISSOLVE_FORMAT, 3, 5);
    CHECK_INT(t, b13_load(t, xml, &scene, NULL), SR_OK);
    CHECK_INT(t, b13_render(&scene, 0, 1, &c, NULL), SR_OK);
    size_t same = 0, total = 0;
    for (uint32_t y = 0; y < 64; ++y)
        for (uint32_t x = 6; x < 63; ++x, ++total)
            same += !memcmp(st_px(&a, x, y), st_px(&c, x, y), 4 * sizeof(float));
    CHECK_INT(t, same, total);
    sr_scene_free(&scene);
    sr_frame_free(&c);
    snprintf(xml, sizeof(xml), DISSOLVE_FORMAT, 4, 0);
    CHECK_INT(t, b13_load(t, xml, &scene, NULL), SR_OK);
    CHECK_INT(t, b13_render(&scene, 0, 1, &c, NULL), SR_OK);
    CHECK(t, !st_frames_equal(&a, &c));
    sr_scene_free(&scene);
    sr_frame_free(&a);
    sr_frame_free(&b);
    sr_frame_free(&c);
}

static void operator_cards_and_particles(sr_test_ctx *t) {
    /* Operators inside a projective card plane and on an emitter render
     * identically at one and four threads and at shuffled times. */
    const char *xml = "<scene version=\"1.1\">\n"
        "<project width=\"96\" height=\"64\" fps=\"4\" duration=\"2\" seed=\"9\"/>\n"
        "<composition><camera id=\"cam\" z=\"-400\" fov=\"50\" active=\"true\"/>"
        "<group id=\"card\" x=\"48\" y=\"32\" anchorX=\"40\" anchorY=\"28\" threeD=\"true\" "
        "rotationY=\"35\"><shape id=\"b\" shape=\"rect\" width=\"80\" height=\"56\" "
        "fill=\"#E08030\"/><shape id=\"d\" shape=\"ellipse\" x=\"10\" y=\"8\" width=\"50\" "
        "height=\"40\" fill=\"#3060E0A0\" blend=\"dissolve\"/><shape id=\"s\" "
        "shape=\"rect\" x=\"30\" y=\"20\" width=\"30\" height=\"20\" fill=\"#FFFFFF\" "
        "blend=\"silhouette-alpha\"/></group>"
        "<particleEmitter id=\"p\" x=\"20\" y=\"50\" rate=\"40\" lifetime=\"2\" "
        "speed=\"20\" size=\"3\" seed=\"5\" color=\"#FFFFFF\" blend=\"behind\"/>"
        "</composition></scene>";
    SrScene scene;
    char *message = NULL;
    SrStatus status = b13_load(t, xml, &scene, &message);
    CHECK_INT(t, status, SR_OK);
    if (status != SR_OK) {
        fprintf(stderr, "%s\n", message ? message : "");
        free(message);
        return;
    }
    free(message);
    SrFrame first, again, other;
    CHECK_INT(t, b13_render(&scene, 1, 1, &first, NULL), SR_OK);
    CHECK_INT(t, b13_render(&scene, 1.75, 4, &other, NULL), SR_OK);
    sr_frame_free(&other);
    CHECK_INT(t, b13_render(&scene, 1, 4, &again, NULL), SR_OK);
    CHECK(t, st_frames_equal(&first, &again));
    sr_frame_free(&first);
    sr_frame_free(&again);
    sr_scene_free(&scene);
}

const sr_test_case sr_tests_blend_operators[] = {
    {"kernel_references", kernel_references},
    {"luma_operators", luma_operators},
    {"dissolve_kernel_and_random", dissolve_kernel_and_random},
    {"isolated_parent_operators", isolated_parent_operators},
    {"stencil_lifecycle", stencil_lifecycle},
    {"dissolve_render", dissolve_render},
    {"operator_cards_and_particles", operator_cards_and_particles},
    {NULL, NULL}
};
