/* SPDX-License-Identifier: Apache-2.0 */
#include "scene_render/xml.h"

#include "fixture.h"

static const float clear[4] = {0, 0, 0, 0};
static const SrColor white = {1, 1, 1, 1};

static void test_two_masks_multiply(sr_test_ctx *t)
{
    SrScene scene;
    fx_scene(&scene, 20, 20);
    SrNode *rect = fx_rect(&scene, NULL, 0, 0, 20, 20, white, 1.0);
    CHECK(t, rect != NULL);
    if (rect) {
        CHECK(t, sr_node_add_mask(rect, fx_mask(SR_MASK_RECT, 0, 0, 10.5, 20, false)) == SR_OK);
        CHECK(t, sr_node_add_mask(rect, fx_mask(SR_MASK_RECT, 0, 0, 20, 10.5, false)) == SR_OK);
    }
    SrFrame frame = {0};
    if (fx_render(t, &scene, 0.0, clear, &frame)) {
        CHECK_NEAR(t, fx_px(&frame, 5, 5)[3], 1.0, 1e-6);
        CHECK_NEAR(t, fx_px(&frame, 10, 5)[3], 0.5, 1e-6);
        CHECK_NEAR(t, fx_px(&frame, 5, 10)[3], 0.5, 1e-6);
        CHECK_NEAR(t, fx_px(&frame, 10, 10)[3], 0.25, 1e-6);
        CHECK_NEAR(t, fx_px(&frame, 15, 5)[3], 0.0, 1e-6);
        CHECK_NEAR(t, fx_px(&frame, 5, 15)[3], 0.0, 1e-6);
    }
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

static void test_inverted_mask(sr_test_ctx *t)
{
    SrScene scene;
    fx_scene(&scene, 20, 4);
    SrNode *rect = fx_rect(&scene, NULL, 0, 0, 20, 4, white, 1.0);
    if (rect)
        CHECK(t, sr_node_add_mask(rect, fx_mask(SR_MASK_RECT, 0, 0, 10.5, 4, true)) == SR_OK);
    SrFrame frame = {0};
    if (fx_render(t, &scene, 0.0, clear, &frame)) {
        CHECK_NEAR(t, fx_px(&frame, 5, 2)[3], 0.0, 1e-6);
        CHECK_NEAR(t, fx_px(&frame, 10, 2)[3], 0.5, 1e-6);
        CHECK_NEAR(t, fx_px(&frame, 15, 2)[3], 1.0, 1e-6);
    }
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

static void test_group_mask(sr_test_ctx *t)
{
    SrScene scene;
    fx_scene(&scene, 20, 4);
    SrNode *group = fx_add(&scene, NULL, SR_NODE_GROUP);
    if (!group) { sr_scene_free(&scene); return; }
    group->transform.x.base = 2.0;
    CHECK(t, sr_node_add_mask(group, fx_mask(SR_MASK_RECT, 0, 0, 8.5, 4, false)) == SR_OK);
    fx_rect(&scene, group, -2, 0, 20, 4, white, 1.0);
    SrFrame frame = {0};
    if (fx_render(t, &scene, 0.0, clear, &frame)) {
        CHECK_NEAR(t, fx_px(&frame, 1, 2)[3], 0.0, 1e-6);
        CHECK_NEAR(t, fx_px(&frame, 5, 2)[3], 1.0, 1e-6);
        CHECK_NEAR(t, fx_px(&frame, 10, 2)[3], 0.5, 1e-6);
        CHECK_NEAR(t, fx_px(&frame, 12, 2)[3], 0.0, 1e-6);
    }
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

static void test_animated_width(sr_test_ctx *t)
{
    SrScene scene;
    fx_scene(&scene, 20, 4);
    SrNode *rect = fx_rect(&scene, NULL, 0, 0, 20, 4, white, 1.0);
    if (!rect) { sr_scene_free(&scene); return; }
    SrMask mask = fx_mask(SR_MASK_RECT, 0, 0, 5, 4, false);
    SrKeyframe k0 = {.time = 0.0, .value = 5.0, .curve = SR_CURVE_LINEAR};
    SrKeyframe k1 = {.time = 1.0, .value = 15.0, .curve = SR_CURVE_LINEAR};
    CHECK(t, sr_track_add(&mask.width.track, k0) == SR_OK);
    CHECK(t, sr_track_add(&mask.width.track, k1) == SR_OK);
    CHECK(t, sr_track_finalize(&mask.width.track) == SR_OK);
    CHECK(t, sr_node_add_mask(rect, mask) == SR_OK);
    SrFrame start = {0}, middle = {0}, end = {0};
    if (fx_render(t, &scene, 0.0, clear, &start) &&
        fx_render(t, &scene, 0.55, clear, &middle) &&
        fx_render(t, &scene, 1.0, clear, &end)) {
        CHECK_NEAR(t, fx_px(&start, 10, 2)[3], 0.0, 1e-6);
        CHECK_NEAR(t, fx_px(&middle, 10, 2)[3], 0.5, 1e-5);
        CHECK_NEAR(t, fx_px(&end, 10, 2)[3], 1.0, 1e-6);
    }
    sr_frame_free(&start);
    sr_frame_free(&middle);
    sr_frame_free(&end);
    sr_scene_free(&scene);
}

static const char two_masks_xml[] =
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"
    "<scene version=\"1.0\">\n"
    "  <project width=\"64\" height=\"32\" fps=\"10\" duration=\"1\"/>\n"
    "  <output path=\"out.mp4\" codec=\"h264\"/>\n"
    "  <assets><vector id=\"box\" shape=\"rect\" width=\"32\" height=\"32\"/></assets>\n"
    "  <composition>\n"
    "    <layer id=\"a\" asset=\"box\">\n"
    "      <mask type=\"rect\" x=\"1\" y=\"2\" width=\"30\" height=\"20\"/>\n"
    "      <mask type=\"ellipse\" x=\"4\" y=\"5\" width=\"10\" height=\"12\" invert=\"true\">\n"
    "        <animate property=\"width\">\n"
    "          <key time=\"0\" value=\"10\"/><key time=\"1\" value=\"20\"/>\n"
    "        </animate>\n"
    "        <animate property=\"x\"><key time=\"0.5\" value=\"7\"/></animate>\n"
    "      </mask>\n"
    "    </layer>\n"
    "  </composition>\n"
    "</scene>\n";

static void test_xml_keeps_every_mask(sr_test_ctx *t)
{
    const char *path = sr_test_tmp_path("two-masks.xml");
    FILE *file = fopen(path, "w");
    CHECK(t, file != NULL);
    if (!file) return;
    fputs(two_masks_xml, file);
    fclose(file);
    FILE *sink = tmpfile();
    SrDiagnostics diag;
    sr_diag_init(&diag, path, sink ? sink : stderr);
    SrScene scene;
    SrStatus status = sr_scene_load_xml(path, &scene, &diag);
    CHECK(t, status == SR_OK);
    if (status == SR_OK) {
        SrNode *layer = sr_scene_find_node(&scene, "a");
        CHECK(t, layer != NULL);
        if (layer) {
            CHECK_INT(t, layer->mask_count, 2);
            if (layer->mask_count == 2) {
                CHECK(t, layer->masks[0].type == SR_MASK_RECT);
                CHECK(t, !layer->masks[0].invert);
                CHECK_NEAR(t, layer->masks[0].width.base, 30.0, 0.0);
                CHECK(t, layer->masks[1].type == SR_MASK_ELLIPSE);
                CHECK(t, layer->masks[1].invert);
                CHECK_INT(t, layer->masks[1].width.track.count, 2);
                CHECK_NEAR(t, sr_anim_eval(&layer->masks[1].width, 0.5), 15.0, 1e-9);
                CHECK_NEAR(t, sr_anim_eval(&layer->masks[1].x, 0.0), 7.0, 1e-9);
                CHECK_NEAR(t, sr_anim_eval(&layer->masks[1].y, 0.0), 5.0, 1e-9);
            }
            /* Node properties stay untouched by mask animation. */
            CHECK_INT(t, layer->transform.x.track.count, 0);
        }
        sr_scene_free(&scene);
    }
    if (sink) fclose(sink);
}

const sr_test_case sr_tests_mask[] = {
    {"two_masks_multiply", test_two_masks_multiply},
    {"inverted_mask", test_inverted_mask},
    {"group_mask", test_group_mask},
    {"animated_width", test_animated_width},
    {"xml_keeps_every_mask", test_xml_keeps_every_mask},
    {NULL, NULL},
};
