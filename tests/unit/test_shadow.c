/* SPDX-License-Identifier: Apache-2.0 */
/* 3D shadow maps (directional light) and 3D-pass supersampling. */
#include "scene_render/effects.h"
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
    /* The XSD rejects it before the loader's own range check runs. */
    CHECK_CONTAINS(t, message, ":1: error: <project> @antialias3d:");
    CHECK_CONTAINS(t, message, "maximum value allowed ('4')");
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


/* Spot caster bounds use the exact tangent cone. The review's case: light-
 * space centre (10, 0, 10), radius 1. The old estimate u +- r/(z-r) gave a
 * largest slope of 1.1111, but the silhouette reaches tan(45 deg +
 * asin(1/sqrt(200))) = 1.1526; the bounds contain every silhouette
 * direction and are tight. */
static void test_spot_cone_bounds_exact(sr_test_ctx *t)
{
    double low, high;
    CHECK(t, sr_light_cone_slopes(10.0, 10.0, 1.0, &low, &high));
    double exact = tan(SR_PI / 4.0 + asin(1.0 / sqrt(200.0)));
    CHECK_NEAR(t, high, exact, 1e-12);
    CHECK_NEAR(t, high, 1.1526, 1e-4);
    CHECK(t, high > 10.0 / 10.0 + 1.0 / (10.0 - 1.0) + 0.04);
    /* Brute force over the sphere surface (x, y, z) = c + r n. */
    double seen_low = INFINITY, seen_high = -INFINITY;
    for (int i = 0; i <= 400; ++i) for (int j = 0; j < 400; ++j) {
        double theta = SR_PI * i / 400.0, phi = 2.0 * SR_PI * j / 400.0;
        double x = 10.0 + sin(theta) * cos(phi), z = 10.0 + cos(theta);
        seen_low = fmin(seen_low, x / z);
        seen_high = fmax(seen_high, x / z);
    }
    CHECK(t, seen_high <= high + 1e-12 && seen_high > high - 1e-4);
    CHECK(t, seen_low >= low - 1e-12 && seen_low < low + 1e-4);
    /* On axis: symmetric +-tan(asin(r / z)). */
    CHECK(t, sr_light_cone_slopes(0.0, 10.0, 1.0, &low, &high));
    CHECK_NEAR(t, high, tan(asin(0.1)), 1e-12);
    CHECK_NEAR(t, low, -high, 1e-12);
    /* A sphere reaching the light plane has no bounded cone. */
    CHECK(t, !sr_light_cone_slopes(3.0, 1.0, 1.0, &low, &high));
}

/* Out-of-range light values set directly (below the XML bounds) keep every
 * texel and pixel bound defined: a 1e-9 degree spot with an off-axis
 * caster renders (the old lower texel bound was ~9.5e13). */
static void test_spot_degenerate_angle_defined(sr_test_ctx *t)
{
    const char *xml =
        "<scene version=\"1.0\"><project width=\"64\" height=\"64\" fps=\"10\" "
        "duration=\"1\" linearLight=\"false\"/><composition>"
        "<object3D id=\"wall\" primitive=\"plane\" x=\"32\" y=\"32\" z=\"-50\" radius=\"30\"/>"
        "<object3D id=\"ball\" primitive=\"sphere\" x=\"50\" y=\"20\" z=\"10\" radius=\"6\"/>"
        "</composition><lights>"
        "<light id=\"spot\" type=\"spot\" x=\"32\" y=\"32\" z=\"200\" spotAngle=\"30\" "
        "castShadow=\"true\" shadowMapSize=\"64\"/></lights></scene>";
    SrScene scene;
    if (st_load(t, "spot-tiny.xml", xml, &scene, NULL) != SR_OK) { SR_FAIL(t, "load"); return; }
    scene.lights[0].spot_angle = 1e-9;
    scene.objects3d[1].transform.x.base = 1e12;
    SrFrame frame = {0};
    if (sr_frame_init(&frame, 64, 64) == SR_OK) {
        const float black[4] = {0, 0, 0, 1};
        sr_frame_clear(&frame, black, 1);
        FILE *sink;
        SrDiagnostics diag;
        st_diag(&diag, &sink);
        CHECK(t, sr_lighting_render(&scene, 0.0, &frame, &diag) == SR_OK);
        if (sink) fclose(sink);
    } else {
        SR_FAIL(t, "frame");
    }
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

static bool material_render(sr_test_ctx *t, SrScene *scene, double time,
                            SrFrame *frame) {
    if (!frame->px && sr_frame_init(frame, 80, 48) != SR_OK) {
        SR_FAIL(t, "material frame allocation");
        return false;
    }
    const float black[4] = {0, 0, 0, 1};
    sr_frame_clear(frame, black, 1);
    FILE *sink;
    SrDiagnostics diag;
    st_diag(&diag, &sink);
    SrStatus status = sr_lighting_render(scene, time, frame, &diag);
    if (sink) fclose(sink);
    CHECK_INT(t, status, SR_OK);
    return status == SR_OK;
}

/* The animated render must match independently computed static material
 * values, including alpha/shadows and a material shared by two objects.
 * Back-out overshoot must clamp metallic/roughness before shading. */
static void animated_material_matches_static(sr_test_ctx *t) {
    static const char format[] =
        "<scene version=\"1.1\"><project width=\"80\" height=\"48\" fps=\"12\" "
        "duration=\"1\" linearLight=\"%s\"/><materials>"
        "<material id=\"m\">%s</material>"
        "<material id=\"wall\" baseColor=\"#606060\" roughness=\"1\"/></materials>"
        "<composition><object3D id=\"a\" primitive=\"sphere\" material=\"m\" "
        "x=\"27\" y=\"24\" z=\"20\" radius=\"12\"/>"
        "<object3D id=\"b\" primitive=\"box\" material=\"m\" "
        "x=\"55\" y=\"24\" z=\"15\" radius=\"9\"/>"
        "<object3D id=\"w\" primitive=\"plane\" material=\"wall\" "
        "x=\"40\" y=\"24\" z=\"-20\" radius=\"40\"/></composition>"
        "<lights><light id=\"ambient\" type=\"ambient\" intensity=\"0.1\"/>"
        "<light id=\"sun\" type=\"directional\" intensity=\"0.5\" yaw=\"-25\" "
        "pitch=\"15\" castShadow=\"true\" shadowMapSize=\"64\"/></lights></scene>";
    static const char tracks[] =
        "<animate property=\"baseColor\"><key time=\"0\" value=\"#00000000\"/>"
        "<key time=\"1\" value=\"#FFFFFFCC\"/></animate>"
        "<animate property=\"emissive\"><key time=\"0\" value=\"#000000\"/>"
        "<key time=\"1\" value=\"0.0625,0,0\"/></animate>"
        "<animate property=\"metallic\" defaultInterpolation=\"%s\">"
        "<key time=\"0\" value=\"0\"/><key time=\"1\" value=\"1\"/></animate>"
        "<animate property=\"roughness\" defaultInterpolation=\"%s\">"
        "<key time=\"0\" value=\"1\"/><key time=\"1\" value=\"0\"/></animate>";
    for (int variant = 0; variant < 6; ++variant) {
        bool overshoot = variant % 2 != 0;
        double time = variant >= 4 ? 0 : overshoot ? 0.6 : 0.5;
        const char *curve = overshoot ? "back-out" : "linear";
        const char *linear_light = variant < 2 ? "false" : "true";
        char keys[2048], xml[4096];
        snprintf(keys, sizeof(keys), tracks, curve, curve);
        snprintf(xml, sizeof(xml), format, linear_light, keys);
        SrScene animated, reference;
        if (st_load(t, "material-animated.xml", xml, &animated, NULL) != SR_OK) {
            SR_FAIL(t, "animated material load");
            return;
        }
        snprintf(xml, sizeof(xml), format, linear_light, "");
        if (st_load(t, "material-static.xml", xml, &reference, NULL) != SR_OK) {
            sr_scene_free(&animated);
            SR_FAIL(t, "static material load");
            return;
        }
        /* IEC sRGB transfer, computed without the animation/color helpers. */
        double encoded = time == 0 ? 0 : 1.055 * pow(time, 1.0 / 2.4) - 0.055;
        double emissive = 12.92 * time * pow((0.0625 + 0.055) / 1.055, 2.4);
        reference.materials[0].base_color.base =
            (SrColor){encoded, encoded, encoded, 0.8 * time};
        reference.materials[0].emissive.base = (SrColor){emissive, 0, 0, 1};
        reference.materials[0].metallic.base = time == 0 ? 0 : overshoot ? 1 : 0.5;
        reference.materials[0].roughness.base = time == 0 ? 1 : overshoot ? 0 : 0.5;
        SrMaterial before;
        memcpy(&before, &animated.materials[0], sizeof(before));
        SrFrame actual = {0}, expected = {0}, repeated = {0};
        if (material_render(t, &animated, time, &actual) &&
            material_render(t, &reference, time, &expected)) {
            double largest = 0;
            for (size_t i = 0; i < 80 * 48 * 4; ++i)
                largest = fmax(largest, fabs(actual.px[i] - expected.px[i]));
            CHECK_NEAR(t, largest, 0, 1e-6);
            CHECK(t, material_render(t, &animated, 0.9, &repeated));
            CHECK(t, !st_frames_equal(&actual, &repeated));
            CHECK(t, material_render(t, &animated, time, &repeated));
            CHECK(t, st_frames_equal(&actual, &repeated));
            CHECK(t, !memcmp(&before, &animated.materials[0], sizeof(before)));
        }
        sr_frame_free(&actual);
        sr_frame_free(&expected);
        sr_frame_free(&repeated);
        sr_scene_free(&animated);
        sr_scene_free(&reference);
    }
}

const sr_test_case sr_tests_shadow[] = {
    {"animated_material_matches_static", animated_material_matches_static},
    {"plane_receives_shadow", test_plane_receives_shadow},
    {"supersampling_changes_only_edges", test_supersampling_changes_only_edges},
    {"antialias_attribute", test_antialias_attribute},
    {"spot_cone_bounds_exact", test_spot_cone_bounds_exact},
    {"spot_degenerate_angle_defined", test_spot_degenerate_angle_defined},
    {NULL, NULL},
};
