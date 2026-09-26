/* SPDX-License-Identifier: Apache-2.0 */
#include "scene_text.h"
#include "fixture.h"
#include "length_frame.h"
#include "xml_internal.h"
#include "scene_render/assets.h"
#include "scene_render/physics.h"

#define PROJECT "<project width=\"128\" height=\"96\" fps=\"12\" duration=\"2\"/>"
#define BEGIN "<scene version=\"1.1\">" PROJECT "<composition>\n"
#define SHAPE "<shape id=\"s\" shape=\"rect\" width=\"8\" height=\"8\" "
#define END "</composition></scene>"

static void expect_xml(sr_test_ctx *t, const char *xml, SrStatus expected,
                        const char *diagnostic) {
    SrScene scene;
    char *message = NULL;
    SrStatus status = st_load(t, "length-input.xml", xml, &scene, &message);
    CHECK_INT(t, status, expected);
    if (diagnostic) CHECK_CONTAINS(t, message, diagnostic);
    if (status == SR_OK) sr_scene_free(&scene);
    free(message);
}

static void parsed_hosts_and_render(sr_test_ctx *t) {
    const char *paths[] = {"tests/data-lengths.xml", "tests/data-lengths-pixels.xml"};
    SrScene scenes[2];
    SrCompositor compositors[2];
    SrFrame frames[2] = {{0}};
    bool loaded[2] = {false, false}, prepared = true;
    FILE *sink;
    SrDiagnostics diag;
    st_diag(&diag, &sink);
    for (size_t i = 0; i < 2; ++i) {
        sr_compositor_init(&compositors[i], i ? 4 : 1);
        SrStatus status = sr_scene_load_xml(sr_test_data_path(paths[i]), &scenes[i], &diag);
        CHECK_INT(t, status, SR_OK);
        loaded[i] = status == SR_OK;
        if (!loaded[i]) { prepared = false; continue; }
        CHECK_INT(t, scenes[i].has_relative_lengths, i == 0);
        status = sr_assets_load(&scenes[i], &diag);
        CHECK_INT(t, status, SR_OK);
        if (status == SR_OK) status = sr_physics_prepare(&scenes[i], &diag);
        CHECK_INT(t, status, SR_OK);
        if (status == SR_OK) status = sr_frame_init(&frames[i], 128, 96);
        CHECK_INT(t, status, SR_OK);
        prepared = prepared && status == SR_OK;
    }
    if (prepared) {
        SrNode *animated = sr_scene_find_node(&scenes[0], "animated");
        SrNode *partial = sr_scene_find_node(&scenes[0], "partial");
        CHECK(t, animated && partial);
        if (animated && partial) {
            CHECK_INT(t, animated->shape_width_unit, SR_LENGTH_PERCENT);
            CHECK_INT(t, animated->transform.x.track.keys[1].unit, SR_LENGTH_VW);
            CHECK_INT(t, animated->transform.x.track.keys[2].unit, SR_LENGTH_VMIN);
            CHECK(t, animated->masks[0].source_line > animated->source_line);
            CHECK(t, partial->group_width_set && !partial->group_height_set);
            SrLengthFrame lengths = {0};
            CHECK_INT(t, sr_length_frame_prepare(&lengths, &scenes[0], 0, false, &diag),
                      SR_OK);
            const SrNodeGeometry *g = sr_length_node(&lengths, partial);
            CHECK_NEAR(t, g->box.width, 32, 0);
            CHECK_NEAR(t, g->box.height, 96, 0);
            sr_length_frame_free(&lengths);
        }
        const double times[] = {23.0 / 12, 0, 1, .25, 1, 0};
        const float background[4] = {0, 0, 0, 0};
        for (size_t f = 0; f < sizeof(times) / sizeof(times[0]); ++f) {
            for (size_t i = 0; i < 2; ++i) {
                sr_frame_clear(&frames[i], background, i ? 4 : 1);
                CHECK_INT(t, sr_compositor_render_scene(&compositors[i], &scenes[i],
                          times[f], &frames[i], &diag), SR_OK);
            }
            CHECK(t, st_frames_equal(&frames[0], &frames[1]));
            CHECK(t, st_px(&frames[0], 100, 28)[3] > 0);
        }
        if (animated) {
            CHECK_NEAR(t, animated->transform.x.base, 25, 0);
            CHECK_INT(t, animated->transform.x.unit, SR_LENGTH_PERCENT);
            CHECK_NEAR(t, animated->transform.x.track.keys[1].value, 37.5, 0);
        }
        CHECK_NEAR(t, scenes[0].physics.constraints[0].rest_length, 0, 0);
        CHECK(t, !scenes[0].physics.constraints[0].rest_length_set);
    }
    for (size_t i = 0; i < 2; ++i) {
        sr_compositor_free(&compositors[i]);
        sr_frame_free(&frames[i]);
        if (loaded[i]) sr_scene_free(&scenes[i]);
    }
    if (sink) fclose(sink);
}

static void spelling_versions_and_scope(sr_test_ctx *t) {
    static const char *const invalid[] = {
        "1px", "+1%", "1e2%", " 1%", "1% ", "0.0.1vh", "1VW",
        "1000001vmin", "-1000001vmax", "nan%", "infvh",
    };
    char xml[2048];
    for (size_t i = 0; i < sizeof(invalid) / sizeof(invalid[0]); ++i) {
        snprintf(xml, sizeof(xml), BEGIN SHAPE "x=\"%s\"/>" END, invalid[i]);
        expect_xml(t, xml, SR_ERR_XML, "<shape> @x:");
    }
    expect_xml(t, BEGIN "<group id=\"g\" width=\"0%\"/>" END, SR_ERR_XML,
               "<group> @width:");
    expect_xml(t, BEGIN "<shape id=\"s\" shape=\"rect\" width=\"-1%\" "
               "height=\"8\"/>" END, SR_ERR_XML, "<shape> @width:");
    expect_xml(t, BEGIN SHAPE "><mask type=\"rect\" width=\"0vw\" "
               "height=\"1\"/></shape>" END, SR_ERR_XML, "<mask> @width:");
    expect_xml(t, BEGIN SHAPE "x=\"-.5%\" y=\"1.vh\"/>" END, SR_OK, NULL);
    expect_xml(t, "<scene version=\"1.0\">" PROJECT "<composition>"
               "<group id=\"g\" width=\"50%\" height=\"25vh\"/>" END, SR_OK, NULL);
    expect_xml(t, "<scene version=\"1.0\">" PROJECT "<composition>"
               SHAPE "x=\"1%\"/>" END, SR_ERR_XML, "requires version=\"1.1\"");
    expect_xml(t, BEGIN "<group id=\"g\" layout=\"row\"/>" END, SR_ERR_XML,
               "unsupported in this build");
    expect_xml(t, BEGIN "<layer id=\"l\" asset=\"a\" boxWidth=\"50%\"/>" END,
               SR_ERR_XML, "unsupported in this build");
    char length[SR_MAX_LENGTH_BYTES + 2];
    memset(length, '0', sizeof(length));
    length[SR_MAX_LENGTH_BYTES - 2] = '1';
    length[SR_MAX_LENGTH_BYTES - 1] = '%';
    length[SR_MAX_LENGTH_BYTES] = '\0';
    snprintf(xml, sizeof(xml), BEGIN SHAPE "x=\"%s\"/>" END, length);
    expect_xml(t, xml, SR_OK, NULL);
    length[SR_MAX_LENGTH_BYTES - 1] = '0';
    length[SR_MAX_LENGTH_BYTES] = '%';
    length[SR_MAX_LENGTH_BYTES + 1] = '\0';
    snprintf(xml, sizeof(xml), BEGIN SHAPE "x=\"%s\"/>" END, length);
    expect_xml(t, xml, SR_ERR_XML, "relative spelling limit 128 bytes");
}

static void particle_mask_version_and_clip(sr_test_ctx *t) {
    const char *mask = "<mask type=\"rect\" x=\"0\" y=\"-24\" "
        "width=\"64\" height=\"48\"/>";
    for (unsigned version = 0; version < 2; ++version) {
        for (unsigned masked = 0; masked < 2; ++masked) {
            char xml[1024];
            snprintf(xml, sizeof(xml), "<scene version=\"1.%u\">" PROJECT
                "<composition><particleEmitter id=\"p\" x=\"64\" y=\"48\" "
                "rate=\"4\" lifetime=\"2\" speed=\"0\" size=\"24\" "
                "color=\"#FFFFFF\" seed=\"1\">%s</particleEmitter>" END,
                version, masked ? mask : "");
            if (version == 0 && masked) {
                expect_xml(t, xml, SR_ERR_XML, "requires version=\"1.1\": <mask>");
                continue;
            }
            SrScene scene;
            SrStatus status = st_load(t, "emitter-mask.xml", xml, &scene, NULL);
            CHECK_INT(t, status, SR_OK);
            if (status != SR_OK) continue;
            SrFrame frame = {0};
            const float clear[4] = {0};
            if (fx_render(t, &scene, 1, clear, &frame)) {
                CHECK(t, st_px(&frame, 68, 48)[3] > 0);
                CHECK_INT(t, st_px(&frame, 60, 48)[3] > 0, !masked);
            }
            sr_frame_free(&frame);
            sr_scene_free(&scene);
        }
    }
}

static void load_limits(sr_test_ctx *t) {
    SrScene scene;
    fx_scene(&scene, 128, 96);
    scene.has_relative_lengths = true;
    SrNode *node = fx_add(&scene, NULL, SR_NODE_SHAPE);
    CHECK(t, node != NULL);
    if (!node) { sr_scene_free(&scene); return; }
    node->source_line = 42;
    FILE *sink;
    SrDiagnostics diag;
    st_diag(&diag, &sink);
    ParseContext ctx = {.scene = &scene, .diag = &diag};
    node->order = SR_MAX_LENGTH_NODES - 1;
    CHECK(t, sr_xml_resolve_lengths(&ctx));
    ++node->order;
    CHECK(t, !sr_xml_resolve_scene(&ctx));
    scene.has_relative_lengths = false;
    CHECK(t, sr_xml_resolve_lengths(&ctx));
    scene.has_relative_lengths = true;
    node->order = 1;
    scene.root->mask_count = SR_MAX_LENGTH_MASKS / 2;
    node->mask_count = SR_MAX_LENGTH_MASKS / 2;
    CHECK(t, sr_xml_resolve_lengths(&ctx));
    ++node->mask_count;
    CHECK(t, !sr_xml_resolve_scene(&ctx));
    scene.root->mask_count = node->mask_count = 0;
    node->child_count = SR_MAX_LENGTH_NODES + 1;
    CHECK(t, !sr_xml_resolve_lengths(&ctx));
    node->child_count = 0;
    scene.physics.constraints = calloc(SR_MAX_LENGTH_CONSTRAINTS + 1,
                                        sizeof(*scene.physics.constraints));
    CHECK(t, scene.physics.constraints != NULL);
    if (scene.physics.constraints) {
        scene.physics.constraint_count = SR_MAX_LENGTH_CONSTRAINTS;
        CHECK(t, sr_xml_resolve_lengths(&ctx));
        scene.physics.constraints[SR_MAX_LENGTH_CONSTRAINTS].source_line = 43;
        ++scene.physics.constraint_count;
        CHECK(t, !sr_xml_resolve_scene(&ctx));
    }
    node->type = SR_NODE_GROUP;
    SrNode *parent = node;
    for (size_t i = 2; i < SR_MAX_LENGTH_DEPTH; ++i) {
        parent = fx_add(&scene, parent, SR_NODE_GROUP);
        CHECK(t, parent != NULL);
        if (!parent) break;
    }
    scene.physics.constraint_count = 0;
    CHECK(t, sr_xml_resolve_lengths(&ctx));
    if (parent) {
        parent = fx_add(&scene, parent, SR_NODE_GROUP);
        CHECK(t, parent != NULL);
        if (parent) CHECK(t, !sr_xml_resolve_scene(&ctx));
    }
    if (sink) {
        char message[2048] = {0};
        rewind(sink);
        CHECK(t, fread(message, 1, sizeof(message) - 1, sink) > 0);
        CHECK_CONTAINS(t, message, ":42: error: <shape>: relative length node limit");
        CHECK_CONTAINS(t, message, ":43: error: <constraint>: relative length constraint limit");
        fclose(sink);
    }
    sr_scene_free(&scene);
}

static void composition_limit_line(sr_test_ctx *t) {
    const char *xml = "<scene version=\"1.1\">\n" PROJECT
        "\n<composition>\n" SHAPE "x=\"25%\"/>" END;
    SrScene scene;
    SrStatus status = st_load(t, "length-composition-line.xml", xml, &scene, NULL);
    CHECK_INT(t, status, SR_OK);
    if (status != SR_OK) return;
    FILE *sink;
    SrDiagnostics diag;
    st_diag(&diag, &sink);
    ParseContext ctx = {.scene = &scene, .diag = &diag};
    size_t count = scene.root->child_count;
    scene.root->child_count = SR_MAX_LENGTH_NODES + 1;
    CHECK(t, !sr_xml_resolve_scene(&ctx));
    scene.root->child_count = count;
    if (sink) {
        char message[1024] = {0};
        rewind(sink);
        CHECK(t, fread(message, 1, sizeof(message) - 1, sink) > 0);
        CHECK_CONTAINS(t, message, ":3: error: <composition>: relative length node limit");
        fclose(sink);
    }
    sr_scene_free(&scene);
}

static void runtime_diagnostic(sr_test_ctx *t) {
    const char *xml = BEGIN "<group id=\"g\" width=\"1e200\">\n"
        SHAPE "x=\"100%\"/></group>" END;
    SrScene scene;
    SrStatus status = st_load(t, "length-runtime.xml", xml, &scene, NULL);
    CHECK_INT(t, status, SR_OK);
    if (status != SR_OK) return;
    FILE *sink;
    SrDiagnostics diag;
    st_diag(&diag, &sink);
    SrLengthFrame lengths = {0};
    CHECK_INT(t, sr_length_frame_prepare(&lengths, &scene, 0, false, &diag), SR_ERR_RENDER);
    if (sink) {
        char message[1024] = {0};
        rewind(sink);
        CHECK(t, fread(message, 1, sizeof(message) - 1, sink) > 0);
        CHECK_CONTAINS(t, message, ":3: error: <shape> @x:");
        fclose(sink);
    }
    sr_length_frame_free(&lengths);
    sr_scene_free(&scene);
}

static void source_fingerprint_and_mutations(sr_test_ctx *t) {
    const char *values[] = {"25%", "25vw", "25vh", "25vmin", "25vmax", "26%"};
    uint64_t hashes[6] = {0};
    char xml[1024];
    for (size_t i = 0; i < sizeof(values) / sizeof(values[0]); ++i) {
        snprintf(xml, sizeof(xml), BEGIN SHAPE "x=\"%s\"/>" END, values[i]);
        SrScene scene;
        SrStatus status = st_load(t, "length-hash.xml", xml, &scene, NULL);
        CHECK_INT(t, status, SR_OK);
        if (status == SR_OK) {
            hashes[i] = scene.source_hash;
            CHECK(t, hashes[i] == sr_fnv1a64(SR_FNV_OFFSET, xml, strlen(xml)));
            for (size_t j = 0; j < i; ++j) CHECK(t, hashes[i] != hashes[j]);
            sr_scene_free(&scene);
        }
    }
    const uint32_t seeds[] = {1, 0xabcdefu, 20260926};
    const char *valid = BEGIN "<group id=\"g\" width=\"50vw\">" SHAPE "x=\"25%\">"
        "<animate property=\"position.y\"><key time=\"0\" value=\"50vh\"/>"
        "<key time=\"1\" value=\"20vmin\"/></animate></shape></group>" END;
    for (size_t s = 0; s < sizeof(seeds) / sizeof(seeds[0]); ++s) {
        uint32_t state = seeds[s];
        for (unsigned i = 0; i < 96; ++i) {
            snprintf(xml, sizeof(xml), "%s", valid);
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            size_t at = state % strlen(xml);
            if (i % 3 == 0) xml[at] = '\0';
            else if (i % 3 == 1) xml[at] = "<&\"%vm012.-+"[(state >> 16) % 12];
            SrScene scene;
            SrStatus status = st_load(t, "length-fuzz.xml", xml, &scene, NULL);
            CHECK(t, status == SR_OK || status == SR_ERR_XML);
            if (i % 3 == 2) CHECK_INT(t, status, SR_OK);
            if (status == SR_OK) sr_scene_free(&scene);
        }
    }
}

const sr_test_case sr_tests_xml_lengths[] = {
    {"parsed_hosts_and_render", parsed_hosts_and_render},
    {"spelling_versions_and_scope", spelling_versions_and_scope},
    {"particle_mask_version_and_clip", particle_mask_version_and_clip},
    {"load_limits", load_limits},
    {"composition_limit_line", composition_limit_line},
    {"runtime_diagnostic", runtime_diagnostic},
    {"source_fingerprint_and_mutations", source_fingerprint_and_mutations},
    {NULL, NULL},
};
