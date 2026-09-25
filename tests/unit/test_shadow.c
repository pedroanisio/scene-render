/* SPDX-License-Identifier: Apache-2.0 */
/* 3D shadow maps (directional light) and 3D-pass supersampling. */
#include "scene_render/lighting.h"

#include "scene_text.h"

/* No camera: world x/y are screen pixels and the viewer looks down -z. A
 * large plane far back (z = -100) receives the shadow of a cube sprite in
 * front (z = 40). The light leans 15 degrees toward +x, so the shadow
 * falls 140 * tan(15 deg) = 37.5 px to the left of the cube. */
static const char *scene_xml(char *buffer, size_t size, bool shadows,
                             int antialias, const char *light_extra)
{
    snprintf(buffer, size,
        "<scene version=\"1.0\"><project width=\"200\" height=\"200\" fps=\"10\" "
        "duration=\"1\" linearLight=\"false\" antialias3d=\"%d\"/>"
        "<materials><material id=\"grey\" baseColor=\"#B0B0B0\" roughness=\"1\"/></materials>"
        "<composition>"
        "<object3D id=\"wall\" primitive=\"plane\" material=\"grey\" x=\"100\" y=\"100\" "
        "z=\"-100\" radius=\"80\"/>"
        "<object3D id=\"cube\" primitive=\"box\" material=\"grey\" x=\"100\" y=\"100\" "
        "z=\"40\" radius=\"15\"/>"
        "</composition><lights>"
        "<light id=\"amb\" type=\"ambient\" intensity=\"0.2\"/>"
        "<light id=\"sun\" type=\"directional\" yaw=\"-15\" pitch=\"0\" castShadow=\"%s\" %s/>"
        "</lights></scene>", antialias, shadows ? "true" : "false", light_extra);
    return buffer;
}

static bool render3d(sr_test_ctx *t, const char *name, const char *xml, SrFrame *frame)
{
    SrScene scene;
    if (st_load(t, name, xml, &scene, NULL) != SR_OK) { SR_FAIL(t, "load %s", name); return false; }
    const float black[4] = {0, 0, 0, 1};
    bool ok = sr_frame_init(frame, 200, 200) == SR_OK;
    CHECK(t, ok);
    if (ok) {
        sr_frame_clear(frame, black, 1);
        FILE *sink;
        SrDiagnostics diag;
        st_diag(&diag, &sink);
        ok = sr_lighting_render(&scene, 0.0, frame, &diag) == SR_OK;
        CHECK(t, ok);
        if (sink) fclose(sink);
    }
    sr_scene_free(&scene);
    return ok;
}

static void test_plane_receives_shadow(sr_test_ctx *t)
{
    char xml[2048];
    SrFrame lit = {0}, shadowed = {0};
    if (render3d(t, "shadow-off.xml", scene_xml(xml, sizeof xml, false, 1, ""), &lit) &&
        render3d(t, "shadow-on.xml", scene_xml(xml, sizeof xml, true, 1, "shadowMapSize=\"1024\""),
                 &shadowed)) {
        double albedo = 0xB0 / 255.0, diffuse = cos(15.0 * SR_PI / 180.0);
        /* Inside the visible shadow (x 48..77): ambient only (the shading
         * model adds ambient light unscaled by albedo). */
        CHECK_NEAR(t, st_px(&shadowed, 62, 100)[0], 0.2, 1e-6);
        CHECK_NEAR(t, st_px(&shadowed, 70, 90)[0], 0.2, 1e-6);
        /* Lit wall: unshadowed and identical to the shadow-free render. */
        CHECK(t, st_px(&shadowed, 140, 100)[0] > 0.2 + 0.95 * albedo * diffuse);
        CHECK_NEAR(t, st_px(&shadowed, 140, 100)[0], st_px(&lit, 140, 100)[0], 1e-6);
        CHECK_NEAR(t, st_px(&shadowed, 62, 60)[0], st_px(&lit, 62, 60)[0], 1e-6);
        CHECK_NEAR(t, st_px(&shadowed, 40, 100)[0], st_px(&lit, 40, 100)[0], 1e-6);
        /* The cube itself (nearest to the light) is lit. */
        CHECK_NEAR(t, st_px(&shadowed, 100, 100)[0], st_px(&lit, 100, 100)[0], 1e-6);
        /* No screen-space blob is drawn for directional shadows. */
        CHECK_NEAR(t, st_px(&shadowed, 160, 30)[0], st_px(&lit, 160, 30)[0], 1e-6);
    }
    sr_frame_free(&lit); sr_frame_free(&shadowed);
}

/* antialias3d="2" keeps every pixel whose 3 x 3 neighbourhood is uniform
 * at antialias3d="1" and changes only edge pixels. */
static void test_supersampling_changes_only_edges(sr_test_ctx *t)
{
    char xml[2048];
    SrFrame one = {0}, two = {0};
    if (render3d(t, "ss-1.xml", scene_xml(xml, sizeof xml, true, 1, "shadowMapSize=\"1024\""), &one) &&
        render3d(t, "ss-2.xml", scene_xml(xml, sizeof xml, true, 2, "shadowMapSize=\"1024\""), &two)) {
        size_t interior = 0, edges_changed = 0, interior_changed = 0;
        for (uint32_t y = 1; y + 1 < 200; ++y) for (uint32_t x = 1; x + 1 < 200; ++x) {
            bool uniform = true;
            for (int dy = -1; dy <= 1 && uniform; ++dy) for (int dx = -1; dx <= 1; ++dx)
                if (memcmp(st_px(&one, x + dx, y + dy), st_px(&one, x, y), 4 * sizeof(float))) {
                    uniform = false; break;
                }
            float diff = 0.0f;
            for (int c = 0; c < 4; ++c)
                diff = fmaxf(diff, fabsf(st_px(&one, x, y)[c] - st_px(&two, x, y)[c]));
            if (uniform) {
                ++interior;
                if (diff > 1e-5f) ++interior_changed;
            } else if (diff > 1e-3f) {
                ++edges_changed;
            }
        }
        CHECK(t, interior > 20000);
        CHECK_INT(t, interior_changed, 0);
        CHECK(t, edges_changed > 50);
    }
    sr_frame_free(&one); sr_frame_free(&two);
}

/* antialias3d defaults to 1 and is validated. */
static void test_antialias_attribute(sr_test_ctx *t)
{
    SrScene scene;
    char *message = NULL;
    const char *bad =
        "<scene version=\"1.0\"><project width=\"8\" height=\"8\" fps=\"10\" duration=\"1\" "
        "antialias3d=\"5\"/><composition/></scene>";
    CHECK(t, st_load(t, "ss-bad.xml", bad, &scene, &message) == SR_ERR_XML);
    CHECK_CONTAINS(t, message, "<project> @antialias3d: expected 1, 2, 3, or 4");
    free(message);
    const char *good =
        "<scene version=\"1.0\"><project width=\"8\" height=\"8\" fps=\"10\" duration=\"1\"/>"
        "<composition/></scene>";
    if (st_load(t, "ss-good.xml", good, &scene, NULL) == SR_OK) {
        CHECK_INT(t, scene.project.antialias3d, 1);
        sr_scene_free(&scene);
    } else {
        SR_FAIL(t, "load");
    }
}

const sr_test_case sr_tests_shadow[] = {
    {"plane_receives_shadow", test_plane_receives_shadow},
    {"supersampling_changes_only_edges", test_supersampling_changes_only_edges},
    {"antialias_attribute", test_antialias_attribute},
    {NULL, NULL},
};
