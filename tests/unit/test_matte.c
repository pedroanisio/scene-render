/* SPDX-License-Identifier: Apache-2.0 */
/* B1-3 track mattes: modes, suppression, dependency validation, source
 * placement and lifetime, captures shared across consumers. */
#include "b13_fixture.h"
#include "fixture.h"
#include "scene_render/raster.h"
#include "compositing_internal.h"

#define ARRAY_COUNT(a) (sizeof(a) / sizeof((a)[0]))

#define MATTE_PROJECT "<project width=\"32\" height=\"32\" fps=\"2\" duration=\"2\" " \
                      "background=\"#00000000\"/>\n"

static void modes_and_suppression(sr_test_ctx *t) {
    static const char *const modes[] = {"alpha", "alpha-inverted", "luma",
                                        "luma-inverted"};
    const SrColor source = {128 / 255.0, 192 / 255.0, 64 / 255.0, 191 / 255.0};
    for (size_t m = 0; m < ARRAY_COUNT(modes); ++m) {
        char xml[1024];
        snprintf(xml, sizeof(xml), "<scene version=\"1.1\">\n" MATTE_PROJECT
                 "<composition><shape id=\"src\" shape=\"rect\" width=\"16\" height=\"32\" "
                 "fill=\"#80C040BF\"/><shape id=\"c\" shape=\"rect\" width=\"32\" "
                 "height=\"32\" fill=\"#FFFFFF\" matte=\"src\" matteMode=\"%s\"/>"
                 "</composition></scene>", modes[m]);
        SrScene scene;
        SrStatus status = b13_load(t, xml, &scene, NULL);
        CHECK_INT(t, status, SR_OK);
        if (status != SR_OK) continue;
        SrFrame frame;
        CHECK_INT(t, b13_render(&scene, 0, 1, &frame, NULL), SR_OK);
        float premultiplied[4];
        sr_color_to_blend(&scene.project, source, premultiplied);
        SrLumaConfig luma;
        sr_luma_config_init(&scene.project, &luma);
        double alpha = premultiplied[3];
        double k = m < 2 ? alpha : alpha * sr_luma_px(&luma, premultiplied);
        double inside = m % 2 ? 1 - k : k, outside = m % 2 ? 1 : 0;
        CHECK_NEAR(t, st_px(&frame, 8, 16)[3], inside, 2e-6);
        CHECK_NEAR(t, st_px(&frame, 8, 16)[0], inside, 2e-6);  /* white only */
        CHECK_NEAR(t, st_px(&frame, 24, 16)[3], outside, 0);
        sr_frame_free(&frame);
        sr_scene_free(&scene);
    }
}

static void visibility_is_an_or(sr_test_ctx *t) {
    static const char *const flags[][2] = {{"false", "false"}, {"true", "false"},
                                           {"false", "true"}};
    for (size_t i = 0; i < ARRAY_COUNT(flags); ++i) {
        char xml[1536];
        snprintf(xml, sizeof(xml), "<scene version=\"1.1\">\n" MATTE_PROJECT
                 "<composition><shape id=\"src\" shape=\"rect\" x=\"20\" width=\"12\" "
                 "height=\"12\" fill=\"#FF0000\"/>"
                 "<shape id=\"a\" shape=\"rect\" width=\"8\" height=\"8\" fill=\"#FFFFFF\" "
                 "matte=\"src\" matteVisible=\"%s\"/>"
                 "<shape id=\"b\" shape=\"rect\" width=\"8\" height=\"8\" start=\"1\" "
                 "fill=\"#FFFFFF\" matte=\"src\" matteVisible=\"%s\"/></composition></scene>",
                 flags[i][0], flags[i][1]);
        SrFrame frame;
        CHECK_INT(t, b13_render_xml(t, xml, 0, 1, &frame, NULL), SR_OK);
        /* Visibility is static and ignores consumer activity. */
        CHECK_NEAR(t, st_px(&frame, 26, 6)[3], i ? 1 : 0, 0);
        sr_frame_free(&frame);
    }
}

static void dependency_validation(sr_test_ctx *t) {
    static const struct { const char *body, *needle; } bad[] = {
        {"<shape id=\"a\" shape=\"rect\" width=\"4\" height=\"4\" matte=\"b\"/>"
         "<shape id=\"b\" shape=\"rect\" width=\"4\" height=\"4\" matte=\"a\"/>",
         "track matte dependency cycle"},
        {"<shape id=\"a\" shape=\"rect\" width=\"4\" height=\"4\" matte=\"a\"/>",
         "track matte dependency cycle: a -> a"},
        {"<group id=\"g\"><shape id=\"a\" shape=\"rect\" width=\"4\" height=\"4\" "
         "matte=\"g\"/></group>", "track matte dependency cycle"},
        {"<shape id=\"a\" shape=\"rect\" width=\"4\" height=\"4\" matte=\"v\"/>",
         "is not a group, layer, shape"},
        {"<shape id=\"a\" shape=\"rect\" width=\"4\" height=\"4\" matte=\"missing\"/>",
         "unknown matte id 'missing'"},
        {"<shape id=\"a\" shape=\"rect\" width=\"4\" height=\"4\" matte=\"c\"/>"
         "<shape id=\"c\" shape=\"rect\" width=\"4\" height=\"4\" matte=\"d\"/>"
         "<shape id=\"d\" shape=\"rect\" width=\"4\" height=\"4\" matte=\"c\"/>",
         "c -> d -> c"},
    };
    for (size_t i = 0; i < ARRAY_COUNT(bad); ++i) {
        char xml[1536];
        snprintf(xml, sizeof(xml), "<scene version=\"1.1\">\n" MATTE_PROJECT
                 "<assets><vector id=\"v\" shape=\"rect\" width=\"4\" height=\"4\"/></assets>"
                 "<composition>%s</composition></scene>", bad[i].body);
        SrScene scene;
        char *message = NULL;
        SrStatus status = b13_load(t, xml, &scene, &message);
        CHECK(t, status != SR_OK);
        CHECK_CONTAINS(t, message, bad[i].needle);
        CHECK_CONTAINS(t, message, "@matte");
        if (status == SR_OK) sr_scene_free(&scene);
        free(message);
    }
    /* A group may use its own descendant; captures exclude ancestor mattes.
     * Matte attributes are new attributes, legal in 1.0 documents. */
    const char *valid = "<scene version=\"1.0\">\n" MATTE_PROJECT
        "<composition><group id=\"g\" matte=\"inner\"><shape id=\"inner\" "
        "shape=\"ellipse\" width=\"16\" height=\"16\" fill=\"#FFFFFF\"/>"
        "<shape id=\"body\" shape=\"rect\" width=\"32\" height=\"32\" fill=\"#00FF00\"/>"
        "</group></composition></scene>";
    SrFrame frame;
    CHECK_INT(t, b13_render_xml(t, valid, 0, 1, &frame, NULL), SR_OK);
    CHECK_NEAR(t, st_px(&frame, 8, 8)[3], 1, 1e-6);
    CHECK_NEAR(t, st_px(&frame, 8, 8)[0], 0, 1e-6);   /* inner suppressed */
    CHECK_NEAR(t, st_px(&frame, 28, 28)[3], 0, 0);
    sr_frame_free(&frame);
}

static void source_lifetime_and_placement(sr_test_ctx *t) {
    const char *xml = "<scene version=\"1.1\">\n" MATTE_PROJECT
        "<composition><group id=\"holder\" x=\"16\" opacity=\".5\">"
        "<mask type=\"rect\" x=\"100\" y=\"100\" width=\"2\" height=\"2\"/>"
        "<shape id=\"src\" shape=\"rect\" width=\"8\" height=\"8\" end=\"1\" "
        "fill=\"#FFFFFF\"/></group>"
        "<shape id=\"a\" shape=\"rect\" width=\"32\" height=\"16\" fill=\"#FFFFFF\" "
        "matte=\"src\"/>"
        "<shape id=\"b\" shape=\"rect\" y=\"16\" width=\"32\" height=\"16\" "
        "fill=\"#FFFFFF\" matte=\"src\" matteMode=\"alpha-inverted\"/>"
        "</composition></scene>";
    SrScene scene;
    CHECK_INT(t, b13_load(t, xml, &scene, NULL), SR_OK);
    SrFrame early, late;
    CHECK_INT(t, b13_render(&scene, 0, 1, &early, NULL), SR_OK);
    /* Ancestor placement and opacity apply; the ancestor's mask does not. */
    CHECK_NEAR(t, st_px(&early, 20, 4)[3], .5, 1e-6);
    CHECK_NEAR(t, st_px(&early, 4, 4)[3], 0, 0);
    CHECK_NEAR(t, st_px(&early, 4, 20)[3], 1, 0);
    CHECK_INT(t, b13_render(&scene, 1.5, 1, &late, NULL), SR_OK);
    CHECK_NEAR(t, st_px(&late, 20, 4)[3], 0, 0);
    CHECK_NEAR(t, st_px(&late, 20, 20)[3], 1, 0);
    sr_frame_free(&early);
    sr_frame_free(&late);
    sr_scene_free(&scene);
}

static void chains_diamonds_and_threads(sr_test_ctx *t) {
    const char *xml = "<scene version=\"1.1\">\n"
        "<project width=\"64\" height=\"48\" fps=\"4\" duration=\"2\" seed=\"3\" "
        "background=\"#00000000\"/>\n"
        "<composition><camera id=\"cam\" z=\"-400\" fov=\"50\" active=\"true\"/>"
        "<shape id=\"c\" shape=\"ellipse\" x=\"8\" y=\"8\" width=\"40\" height=\"32\" "
        "fill=\"#FFFFFF\"><animate property=\"position.x\"><key time=\"0\" value=\"0\"/>"
        "<key time=\"2\" value=\"20\"/></animate></shape>"
        "<shape id=\"b\" shape=\"rect\" width=\"32\" height=\"48\" fill=\"#FFFFFF\" "
        "matte=\"c\" matteMode=\"luma\"/>"
        "<shape id=\"x\" shape=\"rect\" width=\"64\" height=\"48\" fill=\"#FF8040\" "
        "matte=\"b\"/>"
        "<group id=\"card\" x=\"32\" y=\"24\" anchorX=\"24\" anchorY=\"16\" threeD=\"true\" "
        "rotationY=\"30\"><shape id=\"y\" shape=\"rect\" width=\"48\" height=\"32\" "
        "fill=\"#4080FF\" matte=\"b\" matteMode=\"alpha-inverted\"/></group>"
        "<particleEmitter id=\"p\" x=\"32\" y=\"24\" rate=\"30\" lifetime=\"2\" "
        "speed=\"20\" size=\"3\" seed=\"1\" color=\"#FFFF00\" matte=\"c\"/>"
        "</composition></scene>";
    SrScene scene;
    char *message = NULL;
    SrStatus status = b13_load(t, xml, &scene, &message);
    CHECK_INT(t, status, SR_OK);
    free(message);
    if (status != SR_OK) return;
    unsigned char before[sizeof(SrNode)];
    memcpy(before, scene.root->children[1], sizeof(before));
    SrFrame fresh, other, warm;
    CHECK_INT(t, b13_render(&scene, 1, 1, &fresh, NULL), SR_OK);
    CHECK_INT(t, b13_render(&scene, .25, 4, &other, NULL), SR_OK);
    CHECK_INT(t, b13_render(&scene, 1, 4, &warm, NULL), SR_OK);
    CHECK(t, st_frames_equal(&fresh, &warm));
    CHECK(t, !memcmp(before, scene.root->children[1], sizeof(before)));
    /* x shows only where b (itself gated by c) has alpha: left half. */
    CHECK_NEAR(t, st_px(&fresh, 50, 2)[3], 0, 1e-6);
    CHECK(t, scene.compositing->matte_source_count == 2);
    sr_frame_free(&fresh);
    sr_frame_free(&other);
    sr_frame_free(&warm);
    sr_scene_free(&scene);
}

static void adjustment_source(sr_test_ctx *t) {
    const char *xml = "<scene version=\"1.1\">\n" MATTE_PROJECT
        "<composition><shape id=\"left\" shape=\"rect\" width=\"16\" height=\"32\" "
        "fill=\"#808080\"/><adjustment id=\"adj\" effects=\"grade\" opacity=\".5\"/>"
        "<shape id=\"c\" shape=\"rect\" width=\"32\" height=\"32\" fill=\"#FFFFFF\" "
        "matte=\"adj\" z=\"1\"/></composition>"
        "<effects><effect id=\"grade\" type=\"color-grade\" brightness=\"0.2\"/></effects>"
        "</scene>";
    SrFrame frame;
    char *message = NULL;
    SrStatus status = b13_render_xml(t, xml, 0, 1, &frame, &message);
    CHECK_INT(t, status, SR_OK);
    if (status != SR_OK) {
        fprintf(stderr, "%s\n", message ? message : "");
        free(message);
        return;
    }
    free(message);
    /* The captured image is the effected prefix times opacity; the
     * suppressed adjustment leaves the drawn prefix untouched. */
    CHECK_NEAR(t, st_px(&frame, 24, 8)[3], 0, 0);
    CHECK_NEAR(t, st_px(&frame, 8, 8)[3], 1, 1e-6);
    sr_frame_free(&frame);
}

static void direct_c_references(sr_test_ctx *t) {
    SrScene scene, other;
    fx_scene(&scene, 16, 16);
    fx_scene(&other, 16, 16);
    SrNode *source = fx_rect(&scene, NULL, 0, 0, 8, 8, (SrColor){1, 1, 1, 1}, 1);
    SrNode *consumer = fx_rect(&scene, NULL, 0, 0, 16, 16, (SrColor){1, 0, 0, 1}, 1);
    SrNode *foreign = fx_rect(&other, NULL, 0, 0, 8, 8, (SrColor){1, 1, 1, 1}, 1);
    CHECK(t, source && consumer && foreign);
    if (!source || !consumer || !foreign) {
        sr_scene_free(&scene);
        sr_scene_free(&other);
        return;
    }
    consumer->matte = source;
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
    SrFrame frame;
    const float clear[4] = {0, 0, 0, 0};
    CHECK(t, fx_render(t, &scene, 0, clear, &frame));
    CHECK_NEAR(t, fx_px(&frame, 4, 4)[0], 1, 1e-6);
    CHECK_NEAR(t, fx_px(&frame, 4, 4)[1], 0, 1e-6);
    CHECK_NEAR(t, fx_px(&frame, 12, 12)[3], 0, 0);
    sr_frame_free(&frame);
    sr_scene_invalidate_compositing(&scene);
    consumer->matte = foreign;
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_ERR_RENDER);
    consumer->matte = NULL;
    sr_scene_free(&scene);
    sr_scene_free(&other);
}

const sr_test_case sr_tests_matte[] = {
    {"modes_and_suppression", modes_and_suppression},
    {"visibility_is_an_or", visibility_is_an_or},
    {"dependency_validation", dependency_validation},
    {"source_lifetime_and_placement", source_lifetime_and_placement},
    {"chains_diamonds_and_threads", chains_diamonds_and_threads},
    {"adjustment_source", adjustment_source},
    {"direct_c_references", direct_c_references},
    {NULL, NULL}
};
