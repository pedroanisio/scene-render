/* SPDX-License-Identifier: Apache-2.0 */
#include "scene_render/mesh.h"

#include <stdlib.h>

#include "scene_text.h"
#include "scene_render/assets.h"
#include "scene_render/lighting.h"

static void test_load_octahedron_obj(sr_test_ctx *t)
{
    const char *path = sr_test_data_path("examples/assets/octahedron.obj");
    FILE *sink = tmpfile();
    CHECK(t, sink != NULL);
    if (!sink) return;
    SrDiagnostics diag;
    sr_diag_init(&diag, path, sink);
    SrMesh *mesh = NULL;
    CHECK(t, sr_mesh_load_obj(path, &mesh, 1, &diag) == SR_OK);
    CHECK(t, mesh && mesh->triangle_count == 8);
    if (mesh) {
        free(mesh->triangles);
        free(mesh);
    }
    fclose(sink);
}

#define MESH_HEAD "<scene version=\"1.0\"><project width=\"64\" height=\"64\" " \
                  "fps=\"12\" duration=\"1\" linearLight=\"false\"/>"
#define MESH_MATERIALS "<materials>" \
    "<material id=\"red\" baseColor=\"#FF0000\" emissive=\"#FF0000\"/>" \
    "<material id=\"green\" baseColor=\"#00FF00\" emissive=\"#00FF00\"/>" \
    "</materials>"

static bool write_obj(sr_test_ctx *t, const char *name, const char *text)
{
    FILE *file = fopen(sr_test_tmp_path(name), "w");
    CHECK(t, file != NULL);
    if (!file) return false;
    bool ok = fputs(text, file) >= 0;
    if (fclose(file) != 0) ok = false;
    CHECK(t, ok);
    return ok;
}

static bool mesh_frame(sr_test_ctx *t, const char *xml, SrFrame *frame)
{
    SrScene scene;
    if (st_load(t, "mesh-render.xml", xml, &scene, NULL) != SR_OK) {
        SR_FAIL(t, "cannot load mesh scene"); return false;
    }
    FILE *sink;
    SrDiagnostics diag;
    st_diag(&diag, &sink);
    SrStatus status = sr_assets_load(&scene, &diag);
    if (status == SR_OK) status = sr_frame_init(frame, 64, 64);
    if (status == SR_OK) {
        const float black[4] = {0, 0, 0, 1};
        sr_frame_clear(frame, black, 1);
        status = sr_lighting_render(&scene, 0.0, frame, &diag);
    }
    CHECK_INT(t, status, SR_OK);
    if (sink) fclose(sink);
    sr_scene_free(&scene);
    return status == SR_OK;
}

/* Both triangles project to (16,48), (48,48), (32,16). The sloped red
 * triangle is at depth ~2.5 at the centroid, in front of the flat z=4
 * green triangle. Affine depth interpolation incorrectly gives ~7. */
static void test_perspective_depth(sr_test_ctx *t)
{
    if (!write_obj(t, "depth-slope.obj", "v -.5 -.5 1\nv 5 -5 10\nv 0 5 10\nf 1 2 3\n") ||
        !write_obj(t, "depth-flat.obj", "v -2 -2 4\nv 2 -2 4\nv 0 2 4\nf 1 2 3\n")) return;
    const char *xml = MESH_HEAD
        "<assets><mesh id=\"s\" src=\"depth-slope.obj\"/>"
        "<mesh id=\"f\" src=\"depth-flat.obj\"/></assets>" MESH_MATERIALS
        "<composition><camera id=\"c\" fov=\"90\" near=\".1\" far=\"100\"/>"
        "<object3D id=\"a\" primitive=\"mesh\" mesh=\"s\" material=\"red\" radius=\"1\"/>"
        "<object3D id=\"b\" primitive=\"mesh\" mesh=\"f\" material=\"green\" radius=\"1\"/>"
        "</composition></scene>";
    SrFrame frame = {0};
    if (mesh_frame(t, xml, &frame)) {
        CHECK_NEAR(t, st_px(&frame, 32, 37)[0], 1.0, 1e-6);
        CHECK_NEAR(t, st_px(&frame, 32, 37)[1], 0.0, 1e-6);
        /* The right corner really is behind the green triangle. */
        CHECK_NEAR(t, st_px(&frame, 44, 45)[1], 1.0, 1e-6);
    }
    sr_frame_free(&frame);
}

/* Crossing either clipping plane keeps the visible portion; wholly
 * outside triangles still disappear. Test both camera projections. */
static void test_clip_planes(sr_test_ctx *t)
{
    for (int ortho = 0; ortho < 2; ++ortho) {
        for (int plane = 0; plane < 4; ++plane) {
            double z0 = plane == 0 ? .5 : plane == 1 ? 4.0 : plane == 2 ? .5 : 4.0;
            double z1 = plane < 2 ? 2.0 : z0;
            double size = ortho ? 16.0 : 1.0;
            char obj[256], xml[2048];
            snprintf(obj, sizeof obj, "v %g %g %g\nv %g %g %g\nv 0 %g %g\nf 1 2 3\n",
                     -size, -size, z0, size, -size, z1, size, z1);
            if (!write_obj(t, "clip-plane.obj", obj)) return;
            snprintf(xml, sizeof xml, MESH_HEAD
                "<assets><mesh id=\"m\" src=\"clip-plane.obj\"/></assets>" MESH_MATERIALS
                "<composition><camera id=\"c\" projection=\"%s\" fov=\"90\" near=\"1\" far=\"3\"/>"
                "<object3D id=\"a\" primitive=\"mesh\" mesh=\"m\" material=\"red\" radius=\"1\"/>"
                "</composition></scene>", ortho ? "orthographic" : "perspective");
            SrFrame frame = {0};
            if (mesh_frame(t, xml, &frame)) {
                size_t red = 0;
                for (size_t i = 0; i < 64 * 64; ++i) red += frame.px[i * 4] > .9f;
                if (plane < 2) {
                    CHECK(t, red > 80);
                    CHECK_NEAR(t, st_px(&frame, 32, 32)[0], 1.0, 1e-6);
                } else CHECK_INT(t, red, 0);
            }
            sr_frame_free(&frame);
        }
    }
}

/* Alpha-zero geometry never occludes, and two translucent layers over an
 * opaque layer compose in depth order regardless of XML order. A quad's
 * diagonal passes through sample centres, exercising shared-edge ownership. */
static void test_transparency_order(sr_test_ctx *t)
{
    if (!write_obj(t, "transparent-quad.obj",
                   "v 8 8 0\nv 56 8 0\nv 56 56 0\nv 8 56 0\nf 1 2 3 4\n")) return;
    static const unsigned orders[][3] = {{0,1,2}, {0,2,1}, {1,0,2}, {1,2,0}, {2,0,1}, {2,1,0}};
    for (int mesh = 0; mesh < 2; ++mesh) for (int transparent = 0; transparent < 2; ++transparent) {
        char objects[3][256];
        const char *names[] = {"red", "blue", "green"};
        for (int i = 0; i < 3; ++i) {
            snprintf(objects[i], sizeof objects[i],
                "<object3D id=\"o%d\" primitive=\"%s\" %s material=\"%s\" z=\"%d\"/>",
                i, mesh ? "mesh" : "box",
                mesh ? "mesh=\"quad\" radius=\"1\"" : "x=\"32\" y=\"32\" radius=\"24\"",
                names[i], 40 - i * 20);
        }
        for (size_t k = 0; k < 6; ++k) {
            char xml[2048];
            snprintf(xml, sizeof xml, MESH_HEAD
                "<assets><mesh id=\"quad\" src=\"transparent-quad.obj\"/></assets>"
                "<materials><material id=\"red\" baseColor=\"#FF0000%s\" emissive=\"#FF0000\"/>"
                "<material id=\"blue\" baseColor=\"#0000FF%s\" emissive=\"#0000FF\"/>"
                "<material id=\"green\" baseColor=\"#00FF00\" emissive=\"#00FF00\"/></materials>"
                "<composition>%s%s%s</composition></scene>",
                transparent ? "80" : "00", transparent ? "80" : "00",
                objects[orders[k][0]], objects[orders[k][1]], objects[orders[k][2]]);
            SrFrame frame = {0};
            if (mesh_frame(t, xml, &frame)) {
                double alpha = transparent ? 128.0 / 255.0 : 0.0;
                const float *pixel = st_px(&frame, 32, 32);
                CHECK_NEAR(t, pixel[0], alpha, 1e-6);
                CHECK_NEAR(t, pixel[1], (1-alpha)*(1-alpha), 1e-6);
                CHECK_NEAR(t, pixel[2], (1-alpha)*alpha, 1e-6);
                CHECK_NEAR(t, pixel[3], 1.0, 1e-6);
            }
            sr_frame_free(&frame);
        }
    }
}

/* Intersecting transparent triangles need per-pixel ordering: red is in
 * front near the left vertex but behind blue near the right vertex. An
 * object-level depth sort cannot produce both samples correctly. */
static void test_intersecting_transparency(sr_test_ctx *t)
{
    if (!write_obj(t, "alpha-slope.obj", "v -.5 -.5 1\nv 5 -5 10\nv 0 5 10\nf 1 2 3\n") ||
        !write_obj(t, "alpha-flat.obj", "v -2 -2 4\nv 2 -2 4\nv 0 2 4\nf 1 2 3\n")) return;
    const char *objects[] = {
        "<object3D id=\"a\" primitive=\"mesh\" mesh=\"s\" material=\"red\" radius=\"1\"/>",
        "<object3D id=\"b\" primitive=\"mesh\" mesh=\"f\" material=\"blue\" radius=\"1\"/>"
    };
    for (int order = 0; order < 2; ++order) {
        char xml[2048];
        snprintf(xml, sizeof xml, MESH_HEAD
            "<assets><mesh id=\"s\" src=\"alpha-slope.obj\"/><mesh id=\"f\" src=\"alpha-flat.obj\"/></assets>"
            "<materials><material id=\"red\" baseColor=\"#FF000080\" emissive=\"#FF0000\"/>"
            "<material id=\"blue\" baseColor=\"#0000FF80\" emissive=\"#0000FF\"/></materials>"
            "<composition><camera id=\"c\" fov=\"90\" near=\".1\" far=\"100\"/>"
            "%s%s</composition></scene>", objects[order], objects[1-order]);
        SrFrame frame = {0};
        if (mesh_frame(t, xml, &frame)) {
            double alpha = 128.0 / 255.0;
            CHECK_NEAR(t, st_px(&frame, 32, 37)[0], alpha, 1e-6);
            CHECK_NEAR(t, st_px(&frame, 32, 37)[2], alpha*(1-alpha), 1e-6);
            CHECK_NEAR(t, st_px(&frame, 44, 45)[0], alpha*(1-alpha), 1e-6);
            CHECK_NEAR(t, st_px(&frame, 44, 45)[2], alpha, 1e-6);
        }
        sr_frame_free(&frame);
    }
}

const sr_test_case sr_tests_mesh[] = {
    {"intersecting_transparency", test_intersecting_transparency},
    {"perspective_depth", test_perspective_depth},
    {"clip_planes", test_clip_planes},
    {"transparency_order", test_transparency_order},
    {"load_octahedron_obj", test_load_octahedron_obj},
    {NULL, NULL},
};
