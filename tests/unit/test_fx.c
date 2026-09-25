/* SPDX-License-Identifier: Apache-2.0 */
/* Group effects, drop-shadow, 2D lighting and animated effect parameters. */
#include "scene_render/effects.h"

#include "scene_text.h"

static const float clear[4] = {0, 0, 0, 0};

/* Renders `scene` at `time` over transparent black, then applies the
 * whole-frame (unreferenced) effects, as the renderer does. */
static bool render_fx(sr_test_ctx *t, SrScene *scene, double time, SrFrame *frame)
{
    if (sr_frame_init(frame, scene->project.width, scene->project.height) != SR_OK) {
        SR_FAIL(t, "frame allocation failed");
        return false;
    }
    sr_frame_clear(frame, clear, 1);
    SrStatus status = sr_composite_scene(scene, time, frame, NULL);
    if (status == SR_OK) status = sr_effects_apply(scene, time, frame, 1, NULL);
    CHECK(t, status == SR_OK);
    return status == SR_OK;
}

#define PROJECT "<project width=\"64\" height=\"32\" fps=\"10\" duration=\"1\" linearLight=\"false\"/>"

/* A brightness grade referenced by one group lifts only that group's rect;
 * the rect outside the group, and the frame as a whole, are untouched. */
static void test_group_effect_only_inside_group(sr_test_ctx *t)
{
    const char *xml =
        "<scene version=\"1.0\">" PROJECT "<composition>"
        "<group id=\"g\" effects=\"lift\">"
        "<shape id=\"a\" shape=\"rect\" width=\"20\" height=\"20\" x=\"4\" y=\"4\" fill=\"#404040\"/>"
        "</group>"
        "<shape id=\"b\" shape=\"rect\" width=\"20\" height=\"20\" x=\"36\" y=\"4\" fill=\"#404040\"/>"
        "</composition><effects>"
        "<effect id=\"lift\" type=\"color-grade\" brightness=\"0.25\"/>"
        "</effects></scene>";
    SrScene scene;
    if (st_load(t, "fx-group.xml", xml, &scene, NULL) != SR_OK) { SR_FAIL(t, "load"); return; }
    CHECK(t, scene.effects[0].referenced);
    SrFrame frame = {0};
    if (render_fx(t, &scene, 0.0, &frame)) {
        double base = 0x40 / 255.0;
        CHECK_NEAR(t, st_px(&frame, 14, 14)[0], base + 0.25, 1e-5);
        CHECK_NEAR(t, st_px(&frame, 46, 14)[0], base, 1e-6);
        CHECK_NEAR(t, st_px(&frame, 30, 28)[3], 0.0, 0.0);
    }
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

/* The same effect with no group reference keeps its whole-frame role. */
static void test_unreferenced_effect_whole_frame(sr_test_ctx *t)
{
    const char *xml =
        "<scene version=\"1.0\">" PROJECT "<composition>"
        "<group id=\"g\">"
        "<shape id=\"a\" shape=\"rect\" width=\"20\" height=\"20\" x=\"4\" y=\"4\" fill=\"#404040\"/>"
        "</group>"
        "<shape id=\"b\" shape=\"rect\" width=\"20\" height=\"20\" x=\"36\" y=\"4\" fill=\"#404040\"/>"
        "</composition><effects>"
        "<effect id=\"lift\" type=\"color-grade\" brightness=\"0.25\"/>"
        "</effects></scene>";
    SrScene scene;
    if (st_load(t, "fx-frame.xml", xml, &scene, NULL) != SR_OK) { SR_FAIL(t, "load"); return; }
    CHECK(t, !scene.effects[0].referenced);
    SrFrame frame = {0};
    if (render_fx(t, &scene, 0.0, &frame)) {
        double lifted = 0x40 / 255.0 + 0.25;
        CHECK_NEAR(t, st_px(&frame, 14, 14)[0], lifted, 1e-5);
        CHECK_NEAR(t, st_px(&frame, 46, 14)[0], lifted, 1e-5);
    }
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

/* A blurred group spreads past the dirty rectangle of its content by the
 * blur radius and matches blurring the whole buffer exactly. */
static void test_blur_dirty_rect_expansion(sr_test_ctx *t)
{
    const char *grouped =
        "<scene version=\"1.0\">" PROJECT "<composition>"
        "<group id=\"g\" effects=\"soft\">"
        "<shape id=\"a\" shape=\"rect\" width=\"8\" height=\"8\" x=\"28\" y=\"12\" fill=\"#FFFFFF\"/>"
        "</group></composition><effects>"
        "<effect id=\"soft\" type=\"blur\" radius=\"6\"/>"
        "</effects></scene>";
    const char *whole =
        "<scene version=\"1.0\">" PROJECT "<composition>"
        "<shape id=\"a\" shape=\"rect\" width=\"8\" height=\"8\" x=\"28\" y=\"12\" fill=\"#FFFFFF\"/>"
        "</composition><effects>"
        "<effect id=\"soft\" type=\"blur\" radius=\"6\"/>"
        "</effects></scene>";
    SrScene a, b;
    if (st_load(t, "fx-blur-a.xml", grouped, &a, NULL) != SR_OK) { SR_FAIL(t, "load a"); return; }
    if (st_load(t, "fx-blur-b.xml", whole, &b, NULL) != SR_OK) {
        SR_FAIL(t, "load b"); sr_scene_free(&a); return;
    }
    SrFrame fa = {0}, fb = {0};
    if (render_fx(t, &a, 0.0, &fa) && render_fx(t, &b, 0.0, &fb)) {
        float worst = 0.0f;
        for (size_t i = 0; i < (size_t)64 * 32 * 4; ++i)
            worst = fmaxf(worst, fabsf(fa.px[i] - fb.px[i]));
        CHECK(t, worst < 1e-6f);
        /* Six pixels left of the content (x = 22) still receives blur. */
        CHECK(t, st_px(&fa, 22, 16)[3] > 0.0f);
        CHECK_NEAR(t, st_px(&fa, 21, 16)[3], 0.0, 0.0);
    }
    sr_frame_free(&fa); sr_frame_free(&fb);
    sr_scene_free(&a); sr_scene_free(&b);
}

/* Drop shadow: the group's alpha offset by (6, 4), tinted, at `intensity`
 * opacity, under the content. */
static void test_drop_shadow_offset_colour(sr_test_ctx *t)
{
    const char *xml =
        "<scene version=\"1.0\">" PROJECT "<composition>"
        "<group id=\"g\" effects=\"shadow\">"
        "<shape id=\"a\" shape=\"rect\" width=\"10\" height=\"10\" x=\"10\" y=\"10\" fill=\"#FFFFFF\"/>"
        "</group></composition><effects>"
        "<effect id=\"shadow\" type=\"drop-shadow\" offsetX=\"6\" offsetY=\"4\" radius=\"0\""
        " color=\"#FF0000\" intensity=\"0.5\"/>"
        "</effects></scene>";
    SrScene scene;
    if (st_load(t, "fx-shadow.xml", xml, &scene, NULL) != SR_OK) { SR_FAIL(t, "load"); return; }
    SrFrame frame = {0};
    if (render_fx(t, &scene, 0.0, &frame)) {
        const float *shadow = st_px(&frame, 23, 21);    /* inside shadow only */
        CHECK_NEAR(t, shadow[0], 0.5, 1e-6);
        CHECK_NEAR(t, shadow[1], 0.0, 1e-6);
        CHECK_NEAR(t, shadow[3], 0.5, 1e-6);
        const float *content = st_px(&frame, 15, 15);   /* content covers it */
        CHECK_NEAR(t, content[0], 1.0, 1e-6);
        CHECK_NEAR(t, content[1], 1.0, 1e-6);
        CHECK_NEAR(t, st_px(&frame, 12, 22)[3], 0.0, 0.0); /* neither */
        CHECK_NEAR(t, st_px(&frame, 25, 16)[3], 0.5, 1e-6); /* shadow edge */
        CHECK_NEAR(t, st_px(&frame, 26, 16)[3], 0.0, 0.0);
    }
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

static void test_falloff_curves(sr_test_ctx *t)
{
    CHECK_NEAR(t, sr_light_falloff(SR_FALLOFF_LINEAR, 0.25), 0.75, 1e-12);
    CHECK_NEAR(t, sr_light_falloff(SR_FALLOFF_QUADRATIC, 0.25), 0.5625, 1e-12);
    CHECK_NEAR(t, sr_light_falloff(SR_FALLOFF_SMOOTH, 0.5), 0.5625, 1e-12);
    CHECK_NEAR(t, sr_light_falloff(SR_FALLOFF_NONE, 0.99), 1.0, 0.0);
    CHECK_NEAR(t, sr_light_falloff(SR_FALLOFF_LINEAR, 1.0), 0.0, 0.0);
    CHECK_NEAR(t, sr_light_falloff(SR_FALLOFF_SMOOTH, 1.5), 0.0, 0.0);
}

/* A point light of range 100 at the origin over an opaque white strip:
 * each lit pixel equals the falloff at its center's distance. */
static void test_point_light_falloff_values(sr_test_ctx *t)
{
    static const char *const names[] = {"linear", "quadratic", "smooth"};
    static const SrFalloff curves[] = {SR_FALLOFF_LINEAR, SR_FALLOFF_QUADRATIC,
                                       SR_FALLOFF_SMOOTH};
    for (int k = 0; k < 3; ++k) {
        char xml[1024];
        snprintf(xml, sizeof xml,
            "<scene version=\"1.0\"><project width=\"128\" height=\"8\" fps=\"10\" "
            "duration=\"1\" linearLight=\"false\"/><composition>"
            "<group id=\"g\" effects=\"lit\">"
            "<shape id=\"a\" shape=\"rect\" width=\"128\" height=\"8\" fill=\"#FFFFFF\"/>"
            "</group></composition>"
            "<lights><light id=\"p\" type=\"point\" x=\"0\" y=\"0\" range=\"100\"/></lights>"
            "<effects><effect id=\"lit\" type=\"lighting\" lights=\"p\" falloff=\"%s\"/>"
            "</effects></scene>", names[k]);
        SrScene scene;
        if (st_load(t, "fx-falloff.xml", xml, &scene, NULL) != SR_OK) { SR_FAIL(t, "load"); return; }
        CHECK(t, scene.lights[0].used_2d);
        SrFrame frame = {0};
        if (render_fx(t, &scene, 0.0, &frame)) {
            const int xs[] = {10, 49, 90, 110};
            for (size_t i = 0; i < 4; ++i) {
                double d = hypot(xs[i] + 0.5, 0.5);
                CHECK_NEAR(t, st_px(&frame, (uint32_t)xs[i], 0)[0],
                           sr_light_falloff(curves[k], d / 100.0), 2e-6);
                CHECK_NEAR(t, st_px(&frame, (uint32_t)xs[i], 0)[3], 1.0, 0.0);
            }
        }
        sr_frame_free(&frame);
        sr_scene_free(&scene);
    }
}

/* With relief, normals follow the alpha slope: under a directional light
 * shining toward +x the left (lit) edge of a disc is brighter than the
 * mirror-image right edge. */
static void test_relief_brightens_lit_side(sr_test_ctx *t)
{
    const char *xml =
        "<scene version=\"1.0\"><project width=\"48\" height=\"48\" fps=\"10\" "
        "duration=\"1\" linearLight=\"false\"/><composition>"
        "<group id=\"g\" effects=\"lit\">"
        "<shape id=\"a\" shape=\"ellipse\" width=\"30\" height=\"30\" x=\"9\" y=\"9\" fill=\"#FFFFFF\"/>"
        "</group></composition>"
        "<lights><light id=\"sun\" type=\"directional\" yaw=\"0\" pitch=\"30\"/></lights>"
        "<effects><effect id=\"lit\" type=\"lighting\" lights=\"sun\" relief=\"6\"/>"
        "</effects></scene>";
    SrScene scene;
    if (st_load(t, "fx-relief.xml", xml, &scene, NULL) != SR_OK) { SR_FAIL(t, "load"); return; }
    SrFrame frame = {0};
    if (render_fx(t, &scene, 0.0, &frame)) {
        const float *left = st_px(&frame, 9, 24), *right = st_px(&frame, 38, 24);
        CHECK_NEAR(t, left[3], right[3], 1e-6);
        CHECK(t, left[3] > 0.05f);
        CHECK(t, left[0] / left[3] > right[0] / right[3] + 0.1f);
        /* The flat interior faces the viewer: lit by sin(pitch) = 0.5. */
        CHECK_NEAR(t, st_px(&frame, 24, 24)[0], 0.5, 1e-5);
    }
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

/* Animated brightness: the grade changes over time and follows the keys. */
static void test_animated_effect_param(sr_test_ctx *t)
{
    const char *xml =
        "<scene version=\"1.0\">" PROJECT "<composition>"
        "<shape id=\"a\" shape=\"rect\" width=\"64\" height=\"32\" fill=\"#404040\"/>"
        "</composition><effects>"
        "<effect id=\"lift\" type=\"color-grade\">"
        "<animate property=\"brightness\"><key time=\"0\" value=\"0\"/>"
        "<key time=\"1\" value=\"0.4\"/></animate></effect>"
        "</effects></scene>";
    SrScene scene;
    if (st_load(t, "fx-anim.xml", xml, &scene, NULL) != SR_OK) { SR_FAIL(t, "load"); return; }
    SrFrame a = {0}, b = {0};
    if (render_fx(t, &scene, 0.0, &a) && render_fx(t, &scene, 0.5, &b)) {
        double base = 0x40 / 255.0;
        CHECK_NEAR(t, st_px(&a, 5, 5)[0], base, 1e-6);
        CHECK_NEAR(t, st_px(&b, 5, 5)[0], base + 0.2, 1e-5);
        CHECK(t, !st_frames_equal(&a, &b));
    }
    sr_frame_free(&a); sr_frame_free(&b);
    sr_scene_free(&scene);
}

/* Unknown group effect ids are rejected with the group's line and the
 * `effects` attribute. */
static void test_unknown_effect_id(sr_test_ctx *t)
{
    const char *xml =
        "<scene version=\"1.0\">" PROJECT "<composition>\n"
        "<group id=\"g\" effects=\"missing\"/>\n"
        "</composition></scene>";
    SrScene scene;
    char *message = NULL;
    CHECK(t, st_load(t, "fx-unknown.xml", xml, &scene, &message) == SR_ERR_XML);
    CHECK_CONTAINS(t, message, ":2: error: <group> @effects: unknown effect id 'missing'");
    free(message);
}


/* Group effects run before the group's own masks cut the result: a shape
 * outside a rect group mask whose drop shadow falls inside the mask
 * leaves shadow pixels inside it (the shape itself stays masked out). */
static void test_group_mask_after_effects(sr_test_ctx *t)
{
    const char *xml =
        "<scene version=\"1.0\">" PROJECT "<composition>"
        "<group id=\"g\" effects=\"shadow\">"
        "<mask type=\"rect\" x=\"30\" y=\"0\" width=\"30\" height=\"32\"/>"
        "<shape id=\"a\" shape=\"rect\" width=\"10\" height=\"10\" x=\"10\" y=\"10\" fill=\"#FFFFFF\"/>"
        "</group></composition><effects>"
        "<effect id=\"shadow\" type=\"drop-shadow\" offsetX=\"22\" offsetY=\"0\" radius=\"0\""
        " color=\"#000000\" intensity=\"1\"/>"
        "</effects></scene>";
    SrScene scene;
    if (st_load(t, "fx-mask-shadow.xml", xml, &scene, NULL) != SR_OK) { SR_FAIL(t, "load"); return; }
    SrFrame frame = {0};
    if (render_fx(t, &scene, 0.0, &frame)) {
        CHECK_NEAR(t, st_px(&frame, 36, 15)[3], 1.0, 1e-6);   /* shadow in mask */
        CHECK_NEAR(t, st_px(&frame, 15, 15)[3], 0.0, 0.0);    /* shape masked */
        CHECK_NEAR(t, st_px(&frame, 28, 15)[3], 0.0, 0.0);    /* outside mask */
    }
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

/* A light far beyond the XML bound (set directly) over black: the effect
 * leaves every channel finite (0 x inf used to give NaN color). */
static void test_huge_light_stays_finite(sr_test_ctx *t)
{
    const char *xml =
        "<scene version=\"1.0\">" PROJECT "<composition>"
        "<group id=\"g\" effects=\"lit\">"
        "<shape id=\"black\" shape=\"rect\" width=\"32\" height=\"32\" fill=\"#000000\"/>"
        "<shape id=\"grey\" shape=\"rect\" width=\"32\" height=\"32\" x=\"32\" fill=\"#808080\"/>"
        "</group></composition>"
        "<lights><light id=\"sun\" type=\"ambient\" intensity=\"1000000\"/></lights>"
        "<effects><effect id=\"lit\" type=\"lighting\" lights=\"sun\"/></effects></scene>";
    SrScene scene;
    if (st_load(t, "fx-huge-light.xml", xml, &scene, NULL) != SR_OK) { SR_FAIL(t, "load"); return; }
    scene.lights[0].intensity.base = 1e100;
    SrFrame frame = {0};
    if (render_fx(t, &scene, 0.0, &frame)) {
        bool finite = true;
        for (size_t i = 0; i < (size_t)64 * 32 * 4; ++i) finite = finite && isfinite(frame.px[i]);
        CHECK(t, finite);
        CHECK_NEAR(t, st_px(&frame, 10, 10)[0], 0.0, 0.0);    /* black stays black */
        CHECK_NEAR(t, st_px(&frame, 10, 10)[3], 1.0, 0.0);    /* alpha untouched */
        CHECK_NEAR(t, st_px(&frame, 40, 10)[3], 1.0, 0.0);
    }
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

/* Offsets far beyond the XML bound (set directly) stay defined: the
 * shadow simply leaves the frame. */
static void test_huge_shadow_offset_defined(sr_test_ctx *t)
{
    const char *xml =
        "<scene version=\"1.0\">" PROJECT "<composition>"
        "<group id=\"g\" effects=\"shadow\">"
        "<shape id=\"a\" shape=\"rect\" width=\"10\" height=\"10\" x=\"10\" y=\"10\" fill=\"#FFFFFF\"/>"
        "</group></composition><effects>"
        "<effect id=\"shadow\" type=\"drop-shadow\" radius=\"3\"/></effects></scene>";
    SrScene scene;
    if (st_load(t, "fx-huge-offset.xml", xml, &scene, NULL) != SR_OK) { SR_FAIL(t, "load"); return; }
    scene.effects[0].offset_x.base = 1e30;
    scene.effects[0].offset_y.base = -1e30;
    scene.effects[0].radius.base = 1e300;
    SrFrame frame = {0};
    if (render_fx(t, &scene, 0.0, &frame)) {
        CHECK_NEAR(t, st_px(&frame, 15, 15)[3], 1.0, 1e-6);   /* content kept */
        CHECK_NEAR(t, st_px(&frame, 40, 20)[3], 0.0, 0.0);    /* no shadow */
    }
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

static void expect_rejected(sr_test_ctx *t, const char *body, const char *needle)
{
    char xml[1024];
    snprintf(xml, sizeof xml, "<scene version=\"1.0\">" PROJECT
             "<composition/>%s</scene>", body);
    SrScene scene;
    char *message = NULL;
    if (st_load(t, "fx-bounds.xml", xml, &scene, &message) == SR_OK) {
        SR_FAIL(t, "accepted: %s", body);
        sr_scene_free(&scene);
    } else {
        CHECK_CONTAINS(t, message, needle);
    }
    free(message);
}

/* XML bounds: light intensity <= 1e6 (attribute and keys), spotAngle in
 * [0.5, 179], effect offsets within +-1e5 px and radii <= 4096 px. */
static void test_light_and_effect_bounds(sr_test_ctx *t)
{
    expect_rejected(t, "<lights><light id=\"l\" type=\"point\" intensity=\"2e6\"/></lights>",
                    "at most 1e6");
    expect_rejected(t, "<lights><light id=\"l\" type=\"point\"><animate property=\"intensity\">"
                    "<key time=\"0\" value=\"1e100\"/></animate></light></lights>", "at most 1e6");
    expect_rejected(t, "<lights><light id=\"l\" type=\"spot\" spotAngle=\"1e-9\"/></lights>",
                    "[0.5, 179]");
    expect_rejected(t, "<lights><light id=\"l\" type=\"spot\" spotAngle=\"179.5\"/></lights>",
                    "[0.5, 179]");
    expect_rejected(t, "<effects><effect id=\"e\" type=\"drop-shadow\" offsetX=\"1e30\"/></effects>",
                    "+-1e5");
    expect_rejected(t, "<effects><effect id=\"e\" type=\"drop-shadow\"><animate property=\"offsetY\">"
                    "<key time=\"0\" value=\"-200000\"/></animate></effect></effects>", "+-1e5");
    expect_rejected(t, "<effects><effect id=\"e\" type=\"blur\" radius=\"5000\"/></effects>",
                    "4096");
    expect_rejected(t, "<effects><effect id=\"e\" type=\"blur\"><animate property=\"radius\">"
                    "<key time=\"0\" value=\"4097\"/></animate></effect></effects>", "4096");
    const char *good = "<scene version=\"1.0\">" PROJECT "<composition/>"
        "<lights><light id=\"l\" type=\"spot\" spotAngle=\"0.5\" intensity=\"1e6\"/>"
        "<light id=\"m\" type=\"spot\" spotAngle=\"179\"/></lights>"
        "<effects><effect id=\"e\" type=\"drop-shadow\" offsetX=\"-1e5\" offsetY=\"1e5\""
        " radius=\"4096\"/></effects></scene>";
    SrScene scene;
    if (st_load(t, "fx-bounds-good.xml", good, &scene, NULL) == SR_OK) sr_scene_free(&scene);
    else SR_FAIL(t, "boundary values must load");
}

const sr_test_case sr_tests_fx[] = {
    {"group_effect_only_inside_group", test_group_effect_only_inside_group},
    {"unreferenced_effect_whole_frame", test_unreferenced_effect_whole_frame},
    {"blur_dirty_rect_expansion", test_blur_dirty_rect_expansion},
    {"drop_shadow_offset_colour", test_drop_shadow_offset_colour},
    {"falloff_curves", test_falloff_curves},
    {"point_light_falloff_values", test_point_light_falloff_values},
    {"relief_brightens_lit_side", test_relief_brightens_lit_side},
    {"animated_effect_param", test_animated_effect_param},
    {"unknown_effect_id", test_unknown_effect_id},
    {"group_mask_after_effects", test_group_mask_after_effects},
    {"huge_light_stays_finite", test_huge_light_stays_finite},
    {"huge_shadow_offset_defined", test_huge_shadow_offset_defined},
    {"light_and_effect_bounds", test_light_and_effect_bounds},
    {NULL, NULL},
};
