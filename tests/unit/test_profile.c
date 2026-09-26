/* SPDX-License-Identifier: Apache-2.0 */
#include "scene_text.h"

static SrStatus report_load(sr_test_ctx *t, const char *xml, bool report,
                             SrScene *scene, char *message, size_t capacity) {
    const char *path = sr_test_tmp_path("profile.xml");
    FILE *file = fopen(path, "w");
    CHECK(t, file != NULL);
    if (!file) return SR_ERR_IO;
    fputs(xml, file);
    fclose(file);
    FILE *sink = tmpfile();
    CHECK(t, sink != NULL);
    if (!sink) return SR_ERR_IO;
    SrDiagnostics diag;
    sr_diag_init(&diag, path, sink);
    SrStatus status = sr_scene_load_xml_report(path, scene, &diag, report);
    rewind(sink);
    size_t length = fread(message, 1, capacity - 1, sink);
    message[length] = '\0';
    fclose(sink);
    return status;
}

#define PROJECT "<project width=\"16\" height=\"16\" fps=\"2\" duration=\"1\"/>"
#define SHAPE "<shape id=\"s\" shape=\"rect\" width=\"8\" height=\"8\" "

static void versioned_depth_cards(sr_test_ctx *t) {
    static const char *const versions[] = {"1.0", "1.1"};
    for (size_t i = 0; i < 2; ++i) {
        char xml[1024], message[2048];
        SrScene scene;
        snprintf(xml, sizeof(xml), "<scene version=\"%s\">" PROJECT
                 "<composition>" SHAPE "zDepth=\"4\" rotationX=\"10\"/>"
                 "</composition></scene>", versions[i]);
        SrStatus status = report_load(t, xml, false, &scene, message, sizeof(message));
        CHECK_INT(t, status, SR_OK);
        if (status == SR_OK) {
            CHECK_INT(t, scene.root->children[0]->card, i == 0);
            CHECK_NEAR(t, scene.root->children[0]->transform.z.base, 4, 0);
            if (i == 1) CHECK_CONTAINS(t, message, "require threeD=\"true\"");
            else CHECK(t, message[0] == '\0');
            sr_scene_free(&scene);
        }
        snprintf(xml, sizeof(xml), "<scene version=\"%s\">" PROJECT
                 "<composition>" SHAPE "threeD=\"true\" depth=\"4\"/>"
                 "</composition></scene>", versions[i]);
        status = report_load(t, xml, false, &scene, message, sizeof(message));
        CHECK_INT(t, status, SR_OK);
        if (status == SR_OK) {
            CHECK(t, scene.root->children[0]->card);
            sr_scene_free(&scene);
        }
    }
}

static void depth_animation_gate(sr_test_ctx *t) {
    SrScene scene;
    char message[2048];
    const char *xml = "<scene version=\"1.1\">" PROJECT "<composition>"
        SHAPE "><animate property=\"rotation.x\"><key time=\"0\" value=\"5\"/>"
        "</animate></shape></composition></scene>";
    SrStatus status = report_load(t, xml, false, &scene, message, sizeof(message));
    CHECK_INT(t, status, SR_OK);
    if (status == SR_OK) {
        CHECK(t, !scene.root->children[0]->card);
        CHECK_CONTAINS(t, message, "depth animation requires threeD");
        sr_scene_free(&scene);
    }
    xml = "<scene version=\"1.1\">" PROJECT "<composition>"
        SHAPE "depth=\"1\" zDepth=\"2\"/></composition></scene>";
    CHECK_INT(t, report_load(t, xml, false, &scene, message, sizeof(message)), SR_ERR_XML);
    CHECK_CONTAINS(t, message, "mutually exclusive");
}

static void version_and_capability_diagnostics(sr_test_ctx *t) {
    static const struct {
        const char *version;
        const char *value;
        const char *expected;
    } cases[] = {
        {"1.0", "vhs", "requires version=\"1.1\""},
        {"1.1", "vhs", "unsupported in this build: <effect type=\"vhs\">"},
        {"1.1", "bogus", "not an element of the set"}
    };
    for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); ++i) {
        char xml[1024], message[2048];
        SrScene scene;
        snprintf(xml, sizeof(xml), "<scene version=\"%s\">\n" PROJECT
                 "\n<composition/>\n<effects><effect id=\"e\" type=\"%s\"/>"
                 "</effects></scene>", cases[i].version, cases[i].value);
        CHECK_INT(t, report_load(t, xml, false, &scene, message, sizeof(message)),
                  SR_ERR_XML);
        CHECK_CONTAINS(t, message, cases[i].expected);
        CHECK_CONTAINS(t, message, ":4: error: <effect> @type:");
    }
}

static void relative_length_gates(sr_test_ctx *t) {
    static const char *const lengths[] = {"25%", "25vw", "25vh", "25vmin", "25vmax"};
    static const char *const properties[] = {"position.x", "opacity", "fill"};
    for (unsigned version = 10; version <= 11; ++version) {
        for (size_t i = 0; i < sizeof(lengths) / sizeof(lengths[0]); ++i) {
            char xml[1024], message[2048];
            SrScene scene;
            snprintf(xml, sizeof(xml), "<scene version=\"1.%u\">" PROJECT
                     "<composition>" SHAPE "x=\"%s\"/></composition></scene>",
                     version - 10, lengths[i]);
            SrStatus status = report_load(t, xml, false, &scene, message, sizeof(message));
            CHECK_INT(t, status, version == 10 ? SR_ERR_XML : SR_OK);
            if (version == 10) {
                CHECK_CONTAINS(t, message, "requires version=\"1.1\"");
                CHECK_CONTAINS(t, message, "<shape> @x:");
            }
            if (status == SR_OK) sr_scene_free(&scene);
            for (size_t j = 0; j < sizeof(properties) / sizeof(properties[0]); ++j) {
                snprintf(xml, sizeof(xml), "<scene version=\"1.%u\">" PROJECT
                         "<composition>" SHAPE "><animate property=\"%s\">"
                         "<key time=\"0\" value=\"%s\"/></animate></shape>"
                         "</composition></scene>", version - 10, properties[j], lengths[i]);
                status = report_load(t, xml, false, &scene, message, sizeof(message));
                CHECK_INT(t, status, version == 11 && j == 0 ? SR_OK : SR_ERR_XML);
                if (version == 10)
                    CHECK_CONTAINS(t, message, "requires version=\"1.1\"");
                if (version == 10 || j != 0) CHECK_CONTAINS(t, message, "<key> @value:");
                if (status == SR_OK) sr_scene_free(&scene);
            }
        }
    }
}

static void report_every_use(sr_test_ctx *t) {
    const char *xml = "<scene version=\"1.1\">\n" PROJECT
        "\n<composition>" SHAPE "condition=\"true\">\n"
        "<expression property=\"opacity\" enabled=\"false\">0.5</expression>"
        "</shape></composition>\n<effects><effect id=\"e\" type=\"vhs\"/>"
        "</effects></scene>";
    SrScene scene;
    char message[4096];
    CHECK_INT(t, report_load(t, xml, false, &scene, message, sizeof(message)), SR_ERR_XML);
    CHECK_CONTAINS(t, message, "@condition");
    CHECK(t, strstr(message, "<effect type=") == NULL);
    CHECK_INT(t, report_load(t, xml, true, &scene, message, sizeof(message)), SR_ERR_XML);
    CHECK_CONTAINS(t, message, "@condition");
    CHECK_CONTAINS(t, message, "unsupported in this build: <expression>");
    CHECK_CONTAINS(t, message, "<effect type=\"vhs\">");
}

static void unsupported_defaults_and_root_sections(sr_test_ctx *t) {
    SrScene scene;
    char message[2048];
    const char *xml = "<scene version=\"1.0\">" PROJECT
        "<composition>" SHAPE "motionBlur=\"inherit\"/></composition></scene>";
    CHECK_INT(t, report_load(t, xml, false, &scene, message, sizeof(message)), SR_ERR_XML);
    CHECK_CONTAINS(t, message, "unsupported in this build: <shape motionBlur=\"inherit\">");
    xml = "<scene version=\"1.0\">" PROJECT
        "<parameters><param id=\"p\" type=\"number\" default=\"1\"/>"
        "</parameters><composition/></scene>";
    CHECK_INT(t, report_load(t, xml, false, &scene, message, sizeof(message)), SR_ERR_XML);
    CHECK_CONTAINS(t, message, "requires version=\"1.1\": <parameters>");
    xml = "<scene version=\"1.1\">" PROJECT
        "<parameters><param id=\"p\" type=\"number\" default=\"1\"/>"
        "</parameters><composition/></scene>";
    CHECK_INT(t, report_load(t, xml, false, &scene, message, sizeof(message)), SR_ERR_XML);
    CHECK_CONTAINS(t, message, "unsupported in this build: <parameters>");
}

static void fixed_and_semantic_vocabularies(sr_test_ctx *t) {
    SrScene scene;
    char message[4096];
    const char *old = "<scene version=\"1.0\">" PROJECT
        "<scene360 layout=\"cubemap\"/><composition/></scene>";
    CHECK_INT(t, report_load(t, old, true, &scene, message, sizeof(message)), SR_ERR_XML);
    CHECK_CONTAINS(t, message, "requires version=\"1.1\": <scene360 layout=\"cubemap\">");
    const char *current = "<scene version=\"1.1\">" PROJECT
        "<scene360 layout=\"cubemap\"/><composition>" SHAPE ">"
        "<rigidBody shape=\"capsule\"/></shape></composition></scene>";
    CHECK_INT(t, report_load(t, current, true, &scene, message, sizeof(message)), SR_ERR_XML);
    CHECK_CONTAINS(t, message, "unsupported in this build: <scene360 layout=\"cubemap\">");
    CHECK_CONTAINS(t, message, "unsupported in this build: <rigidBody shape=\"capsule\">");
}

static void unsupported_uris_are_redacted(sr_test_ctx *t) {
    SrScene scene;
    char message[4096];
    const char *xml = "<scene version=\"1.1\">" PROJECT
        "<output path=\"test.mp4\" codec=\"h264\">"
        "<destination kind=\"http-put\" "
        "uri=\"https://user:private-password@example.test/?token=private-token\" "
        "credentials=\"private-profile\"/></output><composition/></scene>";
    CHECK_INT(t, report_load(t, xml, true, &scene, message, sizeof(message)), SR_ERR_XML);
    CHECK_CONTAINS(t, message, "<destination> @uri:");
    CHECK_CONTAINS(t, message, "[redacted]");
    CHECK(t, strstr(message, "private-") == NULL);
}

/* Fixed mutation seeds cover both parsers, including truncated attributes
 * and malformed nesting. Successful mutations must still obey the profile. */
static void seeded_profile_mutations(sr_test_ctx *t) {
    static const uint32_t seeds[] = {1, 110, 0x12345678};
    const char *valid = "<scene version=\"1.1\">" PROJECT "<composition>"
        SHAPE "threeD=\"true\" zDepth=\"3\"/></composition></scene>";
    size_t length = strlen(valid);
    for (size_t s = 0; s < sizeof(seeds) / sizeof(seeds[0]); ++s) {
        uint32_t state = seeds[s];
        for (int trial = 0; trial < 48; ++trial) {
            char xml[1024], message[2048];
            memcpy(xml, valid, length + 1);
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            size_t at = state % length;
            if (trial % 3 == 0) xml[at] = '\0';
            else if (trial % 3 == 1) memmove(xml + at, xml + at + 1, length - at);
            else xml[at] = "<&\"0123"[(state >> 16) % 7];
            SrScene scene;
            SrStatus status = report_load(t, xml, trial % 2, &scene,
                                          message, sizeof(message));
            CHECK(t, status == SR_OK || status == SR_ERR_XML);
            if (status == SR_OK) {
                CHECK(t, scene.format_version == 10 || scene.format_version == 11);
                sr_scene_free(&scene);
            }
        }
    }
}

const sr_test_case sr_tests_profile[] = {
    {"versioned_depth_cards", versioned_depth_cards},
    {"depth_animation_gate", depth_animation_gate},
    {"version_and_capability_diagnostics", version_and_capability_diagnostics},
    {"relative_length_gates", relative_length_gates},
    {"report_every_use", report_every_use},
    {"unsupported_defaults_and_root_sections", unsupported_defaults_and_root_sections},
    {"fixed_and_semantic_vocabularies", fixed_and_semantic_vocabularies},
    {"unsupported_uris_are_redacted", unsupported_uris_are_redacted},
    {"seeded_profile_mutations", seeded_profile_mutations},
    {NULL, NULL}
};
