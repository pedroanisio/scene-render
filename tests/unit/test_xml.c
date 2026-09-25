/* SPDX-License-Identifier: Apache-2.0 */
#include "scene_render/assets.h"
#include "scene_render/xml.h"

#include "xml_schema.h"

#include <signal.h>
#include <stdlib.h>
#include <sys/stat.h>
#include <unistd.h>

#include "harness.h"

/* Loads `path` and returns the status; the diagnostics land in `log`. */
static SrStatus load_capture(const char *path, char *log, size_t size)
{
    FILE *sink = tmpfile();
    log[0] = '\0';
    if (!sink) return SR_ERR_IO;
    SrDiagnostics diag;
    sr_diag_init(&diag, path, sink);
    SrScene scene;
    SrStatus status = sr_scene_load_xml(path, &scene, &diag);
    if (status == SR_OK) sr_scene_free(&scene);
    fflush(sink);
    rewind(sink);
    size_t count = fread(log, 1, size - 1, sink);
    log[count] = '\0';
    fclose(sink);
    return status;
}

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

static void test_too_deep_rejected(sr_test_ctx *t)
{
    const char *path = sr_test_tmp_path("too-deep.xml");
    FILE *file = fopen(path, "w");
    CHECK(t, file != NULL);
    if (!file) return;
    /* scene + composition + 255 groups = 257 open elements. */
    fputs("<scene version=\"1.0\"><project width=\"16\" height=\"16\" "
          "fps=\"1\" duration=\"1\"/><composition>", file);
    for (int i = 0; i < 255; ++i) fprintf(file, "<group id=\"g%d\">", i);
    for (int i = 0; i < 255; ++i) fputs("</group>", file);
    fputs("</composition></scene>", file);
    CHECK(t, fclose(file) == 0);
    char log[2048];
    CHECK_INT(t, load_capture(path, log, sizeof(log)), SR_ERR_XML);
    CHECK_CONTAINS(t, log, "nesting exceeds 256 levels");
    unlink(path);
}

/* Both a local file and a FIFO as external DTD/entity targets: opening the
 * FIFO for reading would block until SIGALRM ends the test run, so passing
 * proves neither parser touched it. */
static void test_external_entity_never_loaded(sr_test_ctx *t)
{
    char fifo[512];
    snprintf(fifo, sizeof(fifo), "%s", sr_test_tmp_path("xxe.fifo"));
    unlink(fifo);
    CHECK(t, mkfifo(fifo, 0600) == 0);
    char document[2048];
    snprintf(document, sizeof(document),
             "<?xml version=\"1.0\"?>\n"
             "<!-- a comment first -->\n"
             "<!DOCTYPE scene SYSTEM \"file://%s\" [\n"
             "  <!ENTITY %% remote SYSTEM \"file://%s\"> %%remote;\n"
             "  <!ENTITY leak SYSTEM \"file://%s\">\n"
             "]>\n"
             "<scene version=\"1.0\"><project width=\"16\" height=\"16\" "
             "fps=\"1\" duration=\"1\"/><composition name=\"&leak;\"/></scene>\n",
             fifo, fifo, fifo);
    const char *path = sr_test_tmp_path("xxe.xml");
    FILE *file = fopen(path, "w");
    CHECK(t, file != NULL);
    if (!file) { unlink(fifo); return; }
    fputs(document, file);
    fclose(file);
    alarm(20);
    size_t refused = sr_xml_schema_refused_loads();
    char log[2048];
    CHECK_INT(t, load_capture(path, log, sizeof(log)), SR_ERR_XML);
    CHECK_CONTAINS(t, log, ":3: error: <DOCTYPE>");
    CHECK_CONTAINS(t, log, "document type declarations are not allowed");
    /* The pre-scan stops it before any parser ran. */
    CHECK_INT(t, sr_xml_schema_refused_loads(), refused);
    /* The libxml2 pass on its own (no pre-scan) still fetches nothing:
     * it defers the DOCTYPE, and any load attempt meets the refusing
     * loader. */
    FILE *sink = tmpfile();
    SrDiagnostics diag;
    sr_diag_init(&diag, path, sink ? sink : stderr);
    SrSchemaDeferral deferral;
    SrStatus status = sr_xml_schema_check(document, strlen(document), path, &diag,
                                          &deferral);
    CHECK(t, status == SR_OK || status == SR_ERR_XML);
    CHECK(t, deferral.deferred || status == SR_ERR_XML);
    alarm(0);
    if (sink) fclose(sink);
    unlink(path);
    unlink(fifo);
}

static void test_oversized_rejected(sr_test_ctx *t)
{
    const char *path = sr_test_tmp_path("oversized.xml");
    FILE *file = fopen(path, "w");
    CHECK(t, file != NULL);
    if (!file) return;
    fputs("<scene version=\"1.0\">", file);
    fclose(file);
    CHECK(t, truncate(path, (off_t)SR_XML_MAX_BYTES + 1) == 0);  /* sparse */
    char log[2048];
    CHECK_INT(t, load_capture(path, log, sizeof(log)), SR_ERR_XML);
    CHECK_CONTAINS(t, log, "scene file too large");
    unlink(path);
}

static void test_source_hash_is_parsed_bytes(sr_test_ctx *t)
{
    const char *text = "<scene version=\"1.0\"><project width=\"16\" height=\"16\" "
                       "fps=\"1\" duration=\"1\"/><composition/></scene>\n";
    const char *path = sr_test_tmp_path("hashed.xml");
    FILE *file = fopen(path, "w");
    CHECK(t, file != NULL);
    if (!file) return;
    fputs(text, file);
    fclose(file);
    SrDiagnostics diag;
    sr_diag_init(&diag, path, stderr);
    SrScene scene;
    CHECK_INT(t, sr_scene_load_xml(path, &scene, &diag), SR_OK);
    CHECK(t, scene.source_hash == sr_fnv1a64(SR_FNV_OFFSET, text, strlen(text)));
    sr_scene_free(&scene);
    unlink(path);
}

const sr_test_case sr_tests_xml[] = {
    {"load_basic_multilayer", test_load_basic_multilayer},
    {"invalid_reports_line", test_invalid_reports_line},
    {"doctype_rejected", test_doctype_rejected},
    {"duplicate_id_rejected", test_duplicate_id_rejected},
    {"deep_group_nesting", test_deep_group_nesting},
    {"too_deep_rejected", test_too_deep_rejected},
    {"external_entity_never_loaded", test_external_entity_never_loaded},
    {"oversized_rejected", test_oversized_rejected},
    {"source_hash_is_parsed_bytes", test_source_hash_is_parsed_bytes},
    {NULL, NULL},
};
