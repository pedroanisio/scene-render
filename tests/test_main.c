#include "scene_render/assets.h"
#include "scene_render/compositor.h"
#include "scene_render/scene.h"
#include "scene_render/timeline.h"
#include "scene_render/xml.h"

#include <math.h>
#include <stdio.h>
#include <string.h>

static int failures;

#define CHECK(condition)                                                        \
    do {                                                                        \
        if (!(condition)) {                                                     \
            fprintf(stderr, "FAIL %s:%d: %s\n", __FILE__, __LINE__, #condition); \
            ++failures;                                                         \
        }                                                                       \
    } while (0)

static bool near(double a, double b) {
    return fabs(a - b) < 1e-6;
}

static void test_timeline(void) {
    SrTrack track = {0};
    CHECK(sr_track_add(&track, (SrKeyframe){.time = 2, .value = 20,
                                             .curve = SR_CURVE_LINEAR}) == SR_OK);
    CHECK(sr_track_add(&track, (SrKeyframe){.time = 0, .value = 0,
                                             .curve = SR_CURVE_LINEAR}) == SR_OK);
    CHECK(sr_track_finalize(&track) == SR_OK);
    CHECK(near(sr_track_eval(&track, -1, -1), 0));
    CHECK(near(sr_track_eval(&track, -1, 1), 10));
    CHECK(near(sr_track_eval(&track, -1, 3), 20));
    track.keys[0].curve = SR_CURVE_STEP;
    CHECK(near(sr_track_eval(&track, -1, 1.999), 0));
    track.keys[0].curve = SR_CURVE_EASE_IN;
    CHECK(near(sr_track_eval(&track, -1, 1), 2.5));
    track.keys[0].curve = SR_CURVE_EASE_OUT;
    CHECK(near(sr_track_eval(&track, -1, 1), 17.5));
    track.keys[0].curve = SR_CURVE_EASE_IN_OUT;
    CHECK(near(sr_track_eval(&track, -1, 1), 10));
    track.keys[0].curve = SR_CURVE_BEZIER;
    track.keys[0].x1 = 0;
    track.keys[0].y1 = 0;
    track.keys[0].x2 = 1;
    track.keys[0].y2 = 1;
    CHECK(fabs(sr_track_eval(&track, -1, 1) - 10) < 1e-5);
    sr_track_free(&track);

    SrTrack duplicate = {0};
    sr_track_add(&duplicate, (SrKeyframe){.time = 1});
    sr_track_add(&duplicate, (SrKeyframe){.time = 1});
    CHECK(sr_track_finalize(&duplicate) == SR_ERR_XML);
    sr_track_free(&duplicate);
}

static void test_matrices(void) {
    SrMat3 matrix = sr_mat_multiply(sr_mat_translate(10, 20), sr_mat_scale(2, 3));
    SrVec2 point = sr_mat_point(matrix, (SrVec2){4, 5});
    CHECK(near(point.x, 18));
    CHECK(near(point.y, 35));
    SrMat3 inverse;
    CHECK(sr_mat_inverse(matrix, &inverse));
    point = sr_mat_point(inverse, point);
    CHECK(near(point.x, 4));
    CHECK(near(point.y, 5));
}

static void test_blends(void) {
    SrColor black = {0, 0, 0, 1};
    SrColor white = {1, 1, 1, 1};
    SrColor result = sr_blend_pixel(black, white, 0.5, SR_BLEND_NORMAL, false);
    CHECK(near(result.r, 0.5) && near(result.g, 0.5) && near(result.b, 0.5));
    CHECK(near(result.a, 1));
    result = sr_blend_pixel((SrColor){0.2, 0.4, 0.8, 1},
                            (SrColor){0.5, 0.5, 0.5, 1}, 1,
                            SR_BLEND_MULTIPLY, false);
    CHECK(near(result.r, 0.1));
    CHECK(near(result.g, 0.2));
    CHECK(near(result.b, 0.4));
    result = sr_blend_pixel((SrColor){0.2, 0.2, 0.2, 1},
                            (SrColor){0.5, 0.5, 0.5, 1}, 1,
                            SR_BLEND_ADD, false);
    CHECK(near(result.r, 0.7));
    result = sr_blend_pixel((SrColor){0.2, 0.2, 0.2, 1},
                            (SrColor){0.5, 0.5, 0.5, 1}, 1,
                            SR_BLEND_SCREEN, false);
    CHECK(near(result.r, 0.6));
    result = sr_blend_pixel((SrColor){0.2, 0.2, 0.2, 1},
                            (SrColor){0.8, 0.8, 0.8, 1}, 1,
                            SR_BLEND_OVERLAY, false);
    CHECK(near(result.r, 0.32));
    result = sr_blend_pixel((SrColor){0.2, 0.4, 0.8, 1},
                            (SrColor){0.5, 0.5, 0.5, 1}, 1,
                            SR_BLEND_DIFFERENCE, false);
    CHECK(near(result.r, 0.3));
    CHECK(near(result.g, 0.1));
    CHECK(near(result.b, 0.3));
}

static void test_xml_and_cache(const char *root) {
    char valid[1024], invalid[1024], doctype[1024], duplicate[1024];
    snprintf(valid, sizeof(valid), "%s/examples/basic-multilayer.xml", root);
    snprintf(invalid, sizeof(invalid), "%s/tests/data-invalid.xml", root);
    snprintf(doctype, sizeof(doctype), "%s/tests/data-doctype.xml", root);
    snprintf(duplicate, sizeof(duplicate), "%s/tests/data-duplicate.xml", root);
    FILE *sink = tmpfile();
    SrDiagnostics diag;
    sr_diag_init(&diag, valid, sink);
    SrScene scene;
    CHECK(sr_scene_load_xml(valid, &scene, &diag) == SR_OK);
    CHECK(scene.project.width == 3840 && scene.project.height == 2160);
    CHECK(scene.asset_count == 1);
    CHECK(scene.root->child_count == 1);
    CHECK(sr_assets_load(&scene, &diag) == SR_OK);
    SrNode *group = scene.root->children[0];
    CHECK(group->child_count == 3);
    CHECK(group->children[0]->asset->decoded == group->children[1]->asset->decoded);
    sr_scene_free(&scene);

    sr_diag_init(&diag, invalid, sink);
    CHECK(sr_scene_load_xml(invalid, &scene, &diag) == SR_ERR_XML);
    CHECK(diag.errors == 1);

    sr_diag_init(&diag, doctype, sink);
    CHECK(sr_scene_load_xml(doctype, &scene, &diag) == SR_ERR_XML);
    CHECK(diag.errors == 1);

    sr_diag_init(&diag, duplicate, sink);
    CHECK(sr_scene_load_xml(duplicate, &scene, &diag) == SR_ERR_XML);
    CHECK(diag.errors == 1);
    fclose(sink);
}

int main(int argc, char **argv) {
    if (argc != 2) {
        fprintf(stderr, "usage: sr-unit-tests PROJECT_ROOT\n");
        return 2;
    }
    test_timeline();
    test_matrices();
    test_blends();
    test_xml_and_cache(argv[1]);
    if (failures) {
        fprintf(stderr, "%d unit test(s) failed\n", failures);
        return 1;
    }
    puts("all unit tests passed");
    return 0;
}
