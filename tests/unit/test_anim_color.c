/* SPDX-License-Identifier: Apache-2.0 */
/* Color keyframes: linear-light interpolation, shared easing, XML parsing. */
#include "scene_render/color.h"

#include "scene_text.h"

static SrKeyframe key_at(double time, SrCurve curve)
{
    return (SrKeyframe){.time = time, .curve = curve, .x1 = 0.25, .y1 = 0.1,
                        .x2 = 0.25, .y2 = 1.0};
}

/* The midpoint of black -> white is the linear-light midpoint re-encoded,
 * not the encoded midpoint 0.5. */
static void test_midpoint_is_linear_light(sr_test_ctx *t)
{
    SrAnimColor color = sr_anim_color_static((SrColor){0, 0, 0, 1});
    color.space = SR_COLOR_SRGB;
    CHECK(t, sr_anim_color_add_key(&color, key_at(0.0, SR_CURVE_LINEAR),
                                   (SrColor){0, 0, 0, 1}) == SR_OK);
    CHECK(t, sr_anim_color_add_key(&color, key_at(1.0, SR_CURVE_LINEAR),
                                   (SrColor){1, 0.5, 0.2, 0}) == SR_OK);
    CHECK(t, sr_anim_color_finalize(&color) == SR_OK);
    SrColor mid = sr_anim_color_eval(&color, 0.5);
    CHECK_NEAR(t, mid.r, sr_color_encode(0.5, SR_COLOR_SRGB), 1e-12);
    CHECK_NEAR(t, mid.g, sr_color_encode(0.5 * sr_color_decode(0.5, SR_COLOR_SRGB),
                                         SR_COLOR_SRGB), 1e-12);
    CHECK_NEAR(t, mid.a, 0.5, 1e-12);
    CHECK(t, fabs(mid.r - 0.5) > 0.2);
    /* Endpoints round-trip. */
    SrColor end = sr_anim_color_eval(&color, 2.0);
    CHECK_NEAR(t, end.r, 1.0, 1e-12);
    CHECK_NEAR(t, end.g, 0.5, 1e-9);
    CHECK_NEAR(t, end.b, 0.2, 1e-9);
    sr_anim_color_free(&color);
}

/* One easing curve drives every channel identically. */
static void test_easing_shared_by_channels(sr_test_ctx *t)
{
    SrAnimColor color = sr_anim_color_static((SrColor){0, 0, 0, 1});
    CHECK(t, sr_anim_color_add_key(&color, key_at(0.0, SR_CURVE_EASE_IN),
                                   (SrColor){0, 1, 0, 1}) == SR_OK);
    CHECK(t, sr_anim_color_add_key(&color, key_at(1.0, SR_CURVE_LINEAR),
                                   (SrColor){1, 0, 0, 0}) == SR_OK);
    CHECK(t, sr_anim_color_finalize(&color) == SR_OK);
    for (int i = 1; i < 10; ++i) {
        double time = i / 10.0, eased = time * time * time;
        SrColor c = sr_anim_color_eval(&color, time);
        CHECK_NEAR(t, sr_color_decode(c.r, SR_COLOR_SRGB), eased, 1e-9);
        CHECK_NEAR(t, sr_color_decode(c.g, SR_COLOR_SRGB), 1.0 - eased, 1e-9);
        CHECK_NEAR(t, c.a, 1.0 - eased, 1e-12);
    }
    sr_anim_color_free(&color);
}

/* No keys: exactly the static color. */
static void test_static_is_exact(sr_test_ctx *t)
{
    SrColor base = {0.123456789, 0.5, 0.987654321, 0.25};
    SrAnimColor color = sr_anim_color_static(base);
    SrColor out = sr_anim_color_eval(&color, 3.0);
    CHECK(t, memcmp(&out, &base, sizeof base) == 0);
}

/* <animate property="fill"> with color keys drives a shape; in a
 * linear-light project the blend value at the midpoint is exactly the
 * mean of the endpoint blend values. */
static void test_shape_fill_keys(sr_test_ctx *t)
{
    const char *xml =
        "<scene version=\"1.0\"><project width=\"8\" height=\"8\" fps=\"10\" duration=\"1\"/>"
        "<composition><shape id=\"s\" shape=\"rect\" width=\"8\" height=\"8\" fill=\"#000000\">"
        "<animate property=\"fill\" defaultInterpolation=\"linear\">"
        "<key time=\"0\" value=\"#FF0000\"/><key time=\"1\" value=\"#0000FF\"/>"
        "</animate></shape></composition></scene>";
    SrScene scene;
    if (st_load(t, "color-fill.xml", xml, &scene, NULL) != SR_OK) { SR_FAIL(t, "load"); return; }
    SrNode *shape = sr_scene_find_node(&scene, "s");
    CHECK(t, shape && shape->fill.r.count == 2);
    SrFrame frame = {0};
    const float black[4] = {0, 0, 0, 1};
    CHECK(t, sr_frame_init(&frame, 8, 8) == SR_OK);
    if (frame.px && shape) {
        sr_frame_clear(&frame, black, 1);
        CHECK(t, sr_composite_scene(&scene, 0.5, &frame, NULL) == SR_OK);
        CHECK_NEAR(t, st_px(&frame, 4, 4)[0], 0.5, 1e-5);
        CHECK_NEAR(t, st_px(&frame, 4, 4)[2], 0.5, 1e-5);
        CHECK_NEAR(t, st_px(&frame, 4, 4)[1], 0.0, 1e-6);
    }
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

/* Color properties reject numeric keys; numeric properties reject colors;
 * lights, effects and particles accept color tracks. */
static void test_xml_color_keys(sr_test_ctx *t)
{
    char *message = NULL;
    SrScene scene;
    const char *bad =
        "<scene version=\"1.0\"><project width=\"8\" height=\"8\" fps=\"10\" duration=\"1\"/>"
        "<composition><shape id=\"s\" shape=\"rect\" width=\"8\" height=\"8\">"
        "<animate property=\"stroke\"><key time=\"0\" value=\"0.5\"/></animate>"
        "</shape></composition></scene>";
    CHECK(t, st_load(t, "color-bad.xml", bad, &scene, &message) == SR_ERR_XML);
    CHECK_CONTAINS(t, message, "<key> @value: expected a color");
    free(message);
    const char *good =
        "<scene version=\"1.0\"><project width=\"8\" height=\"8\" fps=\"10\" duration=\"1\"/>"
        "<composition><particleEmitter id=\"p\" rate=\"5\">"
        "<animate property=\"color\"><key time=\"0\" value=\"#FFFFFF\"/><key time=\"1\" value=\"#FF0000\"/></animate>"
        "<animate property=\"colorEnd\"><key time=\"0\" value=\"#00000000\"/></animate>"
        "</particleEmitter></composition>"
        "<lights><light id=\"l\" type=\"ambient\"><animate property=\"color\">"
        "<key time=\"0\" value=\"#FFFFFF\"/></animate></light></lights>"
        "<effects><effect id=\"e\" type=\"lens-flare\"><animate property=\"color\">"
        "<key time=\"0\" value=\"#FFFFFF\"/></animate></effect></effects></scene>";
    SrStatus status = st_load(t, "color-good.xml", good, &scene, &message);
    if (status != SR_OK) SR_FAIL(t, "load failed: %s", message ? message : "");
    free(message);
    if (status == SR_OK) {
        SrNode *p = sr_scene_find_node(&scene, "p");
        CHECK(t, p && p->particle_color.r.count == 2 && p->particle_color_end_set);
        CHECK_INT(t, scene.lights[0].color.r.count, 1);
        CHECK_INT(t, scene.effects[0].color.a.count, 1);
        sr_scene_free(&scene);
    }
}

const sr_test_case sr_tests_anim_color[] = {
    {"midpoint_is_linear_light", test_midpoint_is_linear_light},
    {"easing_shared_by_channels", test_easing_shared_by_channels},
    {"static_is_exact", test_static_is_exact},
    {"shape_fill_keys", test_shape_fill_keys},
    {"xml_color_keys", test_xml_color_keys},
    {NULL, NULL},
};
