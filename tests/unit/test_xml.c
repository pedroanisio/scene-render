/* SPDX-License-Identifier: Apache-2.0 */
#include "scene_render/assets.h"
#include "scene_render/xml.h"

#include <unistd.h>

#include "harness.h"

static void test_load_basic_multilayer(sr_test_ctx *t)
{
    const char *valid = sr_test_data_path("examples/basic-multilayer.xml");
    FILE *sink = tmpfile();
    CHECK(t, sink != NULL);
    if (!sink) return;
    SrDiagnostics diag;
    sr_diag_init(&diag, valid, sink);
    SrScene scene;
    SrStatus status = sr_scene_load_xml(valid, &scene, &diag);
    CHECK(t, status == SR_OK);
    if (status != SR_OK) { fclose(sink); return; }
    CHECK(t, scene.project.width == 3840 && scene.project.height == 2160);
    CHECK_INT(t, scene.asset_count, 1);
    CHECK_INT(t, scene.root->child_count, 1);
    CHECK(t, sr_assets_load(&scene, &diag) == SR_OK);
    SrNode *group = scene.root->children[0];
    CHECK_INT(t, group->child_count, 3);
    if (group->child_count >= 2)
        CHECK(t, group->children[0]->asset->decoded ==
                 group->children[1]->asset->decoded);
    sr_scene_free(&scene);
    fclose(sink);
}

static void test_invalid_reports_line(sr_test_ctx *t)
{
    const char *invalid = sr_test_data_path("tests/data-invalid.xml");
    FILE *sink = tmpfile();
    CHECK(t, sink != NULL);
    if (!sink) return;
    SrDiagnostics diag;
    SrScene scene;
    sr_diag_init(&diag, invalid, sink);
    CHECK(t, sr_scene_load_xml(invalid, &scene, &diag) == SR_ERR_XML);
    CHECK_INT(t, diag.errors, 1);
    fflush(sink);
    rewind(sink);
    char message[1024] = {0};
    CHECK(t, fread(message, 1, sizeof(message) - 1, sink) > 0);
    CHECK_CONTAINS(t, message, ":3: error: <project> @width:");
    fclose(sink);
}

static void expect_single_xml_error(sr_test_ctx *t, const char *relative)
{
    const char *path = sr_test_data_path(relative);
    FILE *sink = tmpfile();
    CHECK(t, sink != NULL);
    if (!sink) return;
    SrDiagnostics diag;
    SrScene scene;
    sr_diag_init(&diag, path, sink);
    CHECK(t, sr_scene_load_xml(path, &scene, &diag) == SR_ERR_XML);
    CHECK_INT(t, diag.errors, 1);
    fclose(sink);
}

static void test_doctype_rejected(sr_test_ctx *t)
{
    expect_single_xml_error(t, "tests/data-doctype.xml");
}

static void test_duplicate_id_rejected(sr_test_ctx *t)
{
    expect_single_xml_error(t, "tests/data-duplicate.xml");
}

static void test_deep_group_nesting(sr_test_ctx *t)
{
    const char *path = sr_test_tmp_path("deep-layers.xml");
    FILE *file = fopen(path, "w");
    CHECK(t, file != NULL);
    if (!file) return;
    fputs("<scene version=\"1.0\"><project width=\"16\" height=\"16\" "
          "fps=\"1\" duration=\"1\"/><composition>", file);
    for (int i = 0; i < 160; ++i) fprintf(file, "<group id=\"g%d\">", i);
    for (int i = 0; i < 160; ++i) fputs("</group>", file);
    fputs("</composition></scene>", file);
    CHECK(t, fclose(file) == 0);
    FILE *sink = tmpfile();
    CHECK(t, sink != NULL);
    if (!sink) { unlink(path); return; }
    SrDiagnostics diag;
    sr_diag_init(&diag, path, sink);
    SrScene scene;
    SrStatus status = sr_scene_load_xml(path, &scene, &diag);
    CHECK(t, status == SR_OK);
    if (status == SR_OK) {
        SrNode *node = scene.root;
        size_t depth = 0;
        while (node && node->child_count == 1) { node = node->children[0]; ++depth; }
        CHECK_INT(t, depth, 160);
        sr_scene_free(&scene);
    }
    fclose(sink);
    unlink(path);
}

const sr_test_case sr_tests_xml[] = {
    {"load_basic_multilayer", test_load_basic_multilayer},
    {"invalid_reports_line", test_invalid_reports_line},
    {"doctype_rejected", test_doctype_rejected},
    {"duplicate_id_rejected", test_duplicate_id_rejected},
    {"deep_group_nesting", test_deep_group_nesting},
    {NULL, NULL},
};
