/* SPDX-License-Identifier: Apache-2.0 */
/* B1-3 adjustment layers: effects on the composite below, limited by
 * opacity, masks and matte, inside the (isolated) parent buffer. */
#include "b13_fixture.h"
#include "fixture.h"
#include "scene_render/raster.h"

#define ARRAY_COUNT(a) (sizeof(a) / sizeof((a)[0]))

#define ADJ_EFFECTS "<effects><effect id=\"blur\" type=\"blur\" radius=\"4\"/>" \
    "<effect id=\"grey\" type=\"color-grade\" saturation=\"0\" brightness=\"0.05\"/>" \
    "<effect id=\"flat\" type=\"blur\" radius=\"12\"/></effects>"

static SrStatus adj_render(sr_test_ctx *t, const char *background, const char *body,
                           double time, SrFrame *frame, char **message) {
    char xml[4096];
    snprintf(xml, sizeof(xml), "<scene version=\"1.1\">\n"
             "<project width=\"48\" height=\"32\" fps=\"2\" duration=\"2\" "
             "background=\"%s\"/>\n<composition>%s</composition>" ADJ_EFFECTS
             "</scene>", background, body);
    return b13_render_xml(t, xml, time, 1, frame, message);
}

#define CONTENT "<shape id=\"r\" shape=\"rect\" x=\"8\" y=\"6\" width=\"14\" " \
    "height=\"18\" fill=\"#E04020\"/><shape id=\"e\" shape=\"ellipse\" x=\"20\" " \
    "y=\"8\" width=\"18\" height=\"16\" fill=\"#20A0E0C0\"/>"

static void group_effect_equivalence(sr_test_ctx *t) {
    SrFrame adjusted, grouped;
    CHECK_INT(t, adj_render(t, "#00000000", CONTENT
        "<adjustment id=\"a\" effects=\"blur\"/>", 0, &adjusted, NULL), SR_OK);
    CHECK_INT(t, adj_render(t, "#00000000", "<group id=\"g\" effects=\"blur\">" CONTENT
        "</group>", 0, &grouped, NULL), SR_OK);
    double worst = 0;
    for (uint32_t y = 0; y < 32; ++y)
        for (uint32_t x = 0; x < 48; ++x)
            for (int c = 0; c < 4; ++c)
                worst = fmax(worst, fabs((double)st_px(&adjusted, x, y)[c] -
                                         st_px(&grouped, x, y)[c]));
    CHECK(t, worst < 1e-5);
    sr_frame_free(&adjusted);
    sr_frame_free(&grouped);
}

static void opacity_mask_and_blend(sr_test_ctx *t) {
    SrFrame plain, full, partial, multiply;
    const char *bg = "#606060";
    CHECK_INT(t, adj_render(t, bg, CONTENT, 0, &plain, NULL), SR_OK);
    CHECK_INT(t, adj_render(t, bg, CONTENT "<adjustment id=\"a\" effects=\"grey\"/>",
                            0, &full, NULL), SR_OK);
    CHECK_INT(t, adj_render(t, bg, CONTENT "<adjustment id=\"a\" effects=\"grey\" "
        "opacity=\".5\"><mask type=\"rect\" width=\"16\" height=\"32\"/></adjustment>",
        0, &partial, NULL), SR_OK);
    CHECK_INT(t, adj_render(t, bg, CONTENT "<adjustment id=\"a\" effects=\"grey\" "
        "blend=\"multiply\"/>", 0, &multiply, NULL), SR_OK);
    for (uint32_t y = 0; y < 32; y += 3) {
        for (uint32_t x = 0; x < 48; x += 2) {
            const float *d = st_px(&plain, x, y), *f = st_px(&full, x, y);
            if (x < 15) {
                float expected[4];
                for (int c = 0; c < 4; ++c) expected[c] = d[c] + .5f * (f[c] - d[c]);
                CHECK(t, b13_near4(st_px(&partial, x, y), expected, 1e-6));
            } else if (x > 16) {
                CHECK(t, !memcmp(st_px(&partial, x, y), d, 4 * sizeof(float)));
            }
            float blended[4] = {d[0], d[1], d[2], d[3]};
            sr_blend_px(SR_BLEND_MULTIPLY, blended, f);
            CHECK(t, b13_near4(st_px(&multiply, x, y), blended, 1e-6));
        }
    }
    /* Premultiplied replacement, not source-over: an opaque grade over an
     * opaque backdrop keeps alpha one. */
    CHECK_NEAR(t, st_px(&full, 12, 12)[3], 1, 0);
    sr_frame_free(&plain);
    sr_frame_free(&full);
    sr_frame_free(&partial);
    sr_frame_free(&multiply);
}

static void effects_read_beyond_masks(sr_test_ctx *t) {
    SrFrame plain, masked;
    const char *body = "<shape id=\"w\" shape=\"rect\" width=\"24\" height=\"32\" "
                       "fill=\"#FFFFFF\"/>";
    char adjusted[512];
    snprintf(adjusted, sizeof(adjusted), "%s<adjustment id=\"a\" effects=\"flat\">"
             "<mask type=\"rect\" x=\"24\" width=\"24\" height=\"32\"/></adjustment>", body);
    CHECK_INT(t, adj_render(t, "#000000", body, 0, &plain, NULL), SR_OK);
    CHECK_INT(t, adj_render(t, "#000000", adjusted, 0, &masked, NULL), SR_OK);
    /* Inside the mask, the blur brings in white from outside it. */
    CHECK(t, st_px(&masked, 26, 16)[0] > st_px(&plain, 26, 16)[0] + .05f);
    CHECK(t, !memcmp(st_px(&masked, 20, 16), st_px(&plain, 20, 16), 4 * sizeof(float)));
    sr_frame_free(&plain);
    sr_frame_free(&masked);
}

static void scope_order_and_lifetime(sr_test_ctx *t) {
    SrFrame plain, grouped, above, inactive;
    const char *bg = "#304050";
    CHECK_INT(t, adj_render(t, bg, CONTENT, 0, &plain, NULL), SR_OK);
    /* Inside a group the backdrop is the group's own earlier children. */
    CHECK_INT(t, adj_render(t, bg, "<group id=\"g\">" CONTENT
        "<adjustment id=\"a\" effects=\"grey\"/></group>", 0, &grouped, NULL), SR_OK);
    CHECK(t, !memcmp(st_px(&grouped, 1, 1), st_px(&plain, 1, 1), 4 * sizeof(float)));
    CHECK(t, memcmp(st_px(&grouped, 12, 12), st_px(&plain, 12, 12), 4 * sizeof(float)));
    /* Later siblings are not affected. */
    CHECK_INT(t, adj_render(t, bg, "<adjustment id=\"a\" effects=\"grey\"/>" CONTENT,
                            0, &above, NULL), SR_OK);
    CHECK(t, !memcmp(st_px(&above, 12, 12), st_px(&plain, 12, 12), 4 * sizeof(float)));
    CHECK_INT(t, adj_render(t, bg, CONTENT "<adjustment id=\"a\" effects=\"grey\" "
                            "start=\"1\"/>", 0, &inactive, NULL), SR_OK);
    CHECK(t, st_frames_equal(&inactive, &plain));
    sr_frame_free(&plain);
    sr_frame_free(&grouped);
    sr_frame_free(&above);
    sr_frame_free(&inactive);
}

static void matte_limits_and_threads(sr_test_ctx *t) {
    char xml[4096];
    snprintf(xml, sizeof(xml), "<scene version=\"1.1\">\n"
             "<project width=\"48\" height=\"32\" fps=\"4\" duration=\"2\" "
             "background=\"#203040\"/>\n<composition>" CONTENT
             "<shape id=\"m\" shape=\"ellipse\" x=\"4\" y=\"4\" width=\"24\" height=\"24\" "
             "fill=\"#FFFFFF\"><animate property=\"position.x\"><key time=\"0\" value=\"0\"/>"
             "<key time=\"2\" value=\"20\"/></animate></shape>"
             "<adjustment id=\"a\" effects=\"grey blur\" matte=\"m\" skewX=\"10\">"
             "<mask type=\"star\" x=\"24\" y=\"16\" radius=\"20\" feather=\"2\"/>"
             "</adjustment></composition>" ADJ_EFFECTS "</scene>");
    SrScene scene;
    char *message = NULL;
    SrStatus status = b13_load(t, xml, &scene, &message);
    CHECK_INT(t, status, SR_OK);
    free(message);
    if (status != SR_OK) return;
    SrFrame one, four, warm;
    CHECK_INT(t, b13_render(&scene, 1, 1, &one, NULL), SR_OK);
    CHECK_INT(t, b13_render(&scene, .5, 4, &warm, NULL), SR_OK);
    CHECK_INT(t, b13_render(&scene, 1, 4, &four, NULL), SR_OK);
    CHECK(t, st_frames_equal(&one, &four));
    sr_frame_free(&one);
    sr_frame_free(&four);
    sr_frame_free(&warm);
    sr_scene_free(&scene);
}

static void loader_diagnostics(sr_test_ctx *t) {
    static const struct { const char *version, *body, *needle; } cases[] = {
        {"1.1", "<adjustment id=\"a\" effects=\"nope\"/>", "unknown effect id 'nope'"},
        {"1.0", "<adjustment id=\"a\" effects=\"blur\"/>", "requires version=\"1.1\""},
        {"1.1", "<adjustment id=\"a\" effects=\"blur\" threeD=\"true\"/>",
         "unsupported in this build"},
        {"1.1", "<adjustment id=\"a\"/>", "effects"},
        {"1.1", "<adjustment id=\"a\" effects=\"blur\" matte=\"a\"/>",
         "track matte dependency cycle"},
    };
    for (size_t i = 0; i < ARRAY_COUNT(cases); ++i) {
        char xml[2048];
        snprintf(xml, sizeof(xml), "<scene version=\"%s\">\n"
                 "<project width=\"8\" height=\"8\" fps=\"2\" duration=\"1\"/>\n"
                 "<composition>%s</composition>" ADJ_EFFECTS "</scene>",
                 cases[i].version, cases[i].body);
        SrScene scene;
        char *message = NULL;
        SrStatus status = b13_load(t, xml, &scene, &message);
        CHECK(t, status != SR_OK);
        CHECK_CONTAINS(t, message, cases[i].needle);
        CHECK_CONTAINS(t, message, "adjustment");
        if (status == SR_OK) sr_scene_free(&scene);
        free(message);
    }
}

static void direct_c_preparation(sr_test_ctx *t) {
    SrScene scene;
    fx_scene(&scene, 8, 8);
    SrNode *adjustment = fx_add(&scene, NULL, SR_NODE_ADJUSTMENT);
    CHECK(t, adjustment != NULL);
    if (!adjustment) { sr_scene_free(&scene); return; }
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_ERR_RENDER);
    SrEffect effect = {.type = SR_EFFECT_BLUR, .enabled = true,
                       .intensity = {.base = 1}, .radius = {.base = 2}};
    SrEffect *refs[1] = {&effect};
    adjustment->effect_refs = refs;
    adjustment->effect_ref_count = 1;
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
    SrFrame frame;
    const float background[4] = {.25f, .25f, .25f, 1};
    CHECK(t, fx_render(t, &scene, 0, background, &frame));
    CHECK(t, b13_near4(fx_px(&frame, 4, 4), background, 1e-6));
    sr_frame_free(&frame);
    sr_scene_invalidate_compositing(&scene);
    adjustment->card = true;
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_ERR_RENDER);
    adjustment->card = false;
    adjustment->effect_refs = NULL;
    adjustment->effect_ref_count = 0;
    sr_scene_free(&scene);
}

const sr_test_case sr_tests_adjustment[] = {
    {"group_effect_equivalence", group_effect_equivalence},
    {"opacity_mask_and_blend", opacity_mask_and_blend},
    {"effects_read_beyond_masks", effects_read_beyond_masks},
    {"scope_order_and_lifetime", scope_order_and_lifetime},
    {"matte_limits_and_threads", matte_limits_and_threads},
    {"loader_diagnostics", loader_diagnostics},
    {"direct_c_preparation", direct_c_preparation},
    {NULL, NULL}
};
