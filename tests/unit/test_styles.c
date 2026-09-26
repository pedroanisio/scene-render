/* SPDX-License-Identifier: Apache-2.0 */
#include "scene_text.h"
#include "scene_render/color.h"
#include "scene_render/renderer.h"

#define PROJECT "<project width=\"16\" height=\"16\" fps=\"2\" duration=\"1\"/>"
#define BEGIN "<scene version=\"1.1\">" PROJECT "<styles>"
#define END "</styles><composition/></scene>"

static void expect_xml(sr_test_ctx *t, const char *xml, SrStatus expected,
                        const char *diagnostic) {
    SrScene scene;
    char *message = NULL;
    SrStatus status = st_load(t, "styles.xml", xml, &scene, &message);
    CHECK_INT(t, status, expected);
    if (diagnostic) CHECK_CONTAINS(t, message, diagnostic);
    if (status == SR_OK) sr_scene_free(&scene);
    free(message);
}

static void check_color(sr_test_ctx *t, SrColor color) {
    CHECK_NEAR(t, color.r, 0.2, 1e-12);
    CHECK_NEAR(t, color.g, 0.4, 1e-12);
    CHECK_NEAR(t, color.b, 0.6, 1e-12);
    CHECK_NEAR(t, color.a, 0.8, 1e-12);
}

static void all_color_hosts(sr_test_ctx *t) {
    SrScene scene;
    FILE *sink;
    SrDiagnostics diag;
    st_diag(&diag, &sink);
    SrStatus status = sr_scene_load_xml(sr_test_data_path("tests/data-styles.xml"),
                                       &scene, &diag);
    CHECK_INT(t, status, SR_OK);
    if (status == SR_OK) {
        CHECK_INT(t, scene.token_count, 17);
        CHECK_INT(t, scene.tokens[0].resolved_index, 2);
        CHECK_INT(t, scene.tokens[1].resolved_index, 3);
        CHECK_INT(t, scene.tokens[8].resolved_index, 3);
        CHECK_INT(t, scene.tokens[8].resolved_hops, 2);
        CHECK_STR(t, scene.tokens[8].name, "--dashed");
        CHECK_NEAR(t, scene.project.background.r, 16.0 / 255.0, 0);
        CHECK_NEAR(t, scene.project.background.g, 32.0 / 255.0, 0);
        CHECK_NEAR(t, scene.project.background.b, 48.0 / 255.0, 0);
        check_color(t, scene.assets[0].color);
        check_color(t, scene.assets[1].color);
        check_color(t, scene.assets[1].vector_stroke);
        const SrNode *shape = sr_scene_find_node(&scene, "shape");
        const SrNode *particles = sr_scene_find_node(&scene, "particles");
        CHECK(t, shape && particles);
        if (shape && particles) {
            const SrAnimColor *colors[] = {
                &shape->fill, &shape->stroke,
                &particles->particle_color, &particles->particle_color_end,
                &scene.materials[0].base_color, &scene.materials[0].emissive,
                &scene.lights[0].color, &scene.effects[0].color,
            };
            for (size_t i = 0; i < sizeof(colors) / sizeof(colors[0]); ++i) {
                check_color(t, colors[i]->base);
                check_color(t, sr_anim_color_eval(colors[i], 0.0));
            }
            SrColor end = sr_anim_color_eval(&shape->fill, 2.0);
            CHECK_NEAR(t, end.r, 224.0 / 255.0, 1e-12);
            CHECK_NEAR(t, end.a, 128.0 / 255.0, 1e-12);
        }
        sr_scene_free(&scene);
        CHECK(t, !scene.tokens && !scene.token_count);
    }
    if (sink) fclose(sink);
}

static void errors_and_scope(sr_test_ctx *t) {
    static const struct { const char *xml, *diagnostic; } cases[] = {
        {BEGIN "<token name=\"x\" value=\"0\"/><token name=\"x\" value=\"1\"/>" END,
         "<token> @name: duplicate token 'x'"},
        {BEGIN "<token name=\"x\" value=\"var(--missing)\"/>" END,
         "<token> @value: unknown token 'missing'"},
        {BEGIN "<token name=\"x\" value=\"var(--x)\"/>" END, "token alias cycle"},
        {BEGIN "<token name=\"x\" value=\"var(--y)\"/>"
               "<token name=\"y\" value=\"var(--x)\"/>" END, "token alias cycle"},
        {BEGIN "<token name=\"x\" value=\"var(foo)\"/>" END, "malformed token reference"},
        {BEGIN "<token name=\"x\" value=\"var(--)\"/>" END, "malformed token reference"},
        {BEGIN "<token name=\"x\" value=\"var(--x)extra\"/>" END,
         "malformed token reference"},
        {BEGIN "<token name=\"bad.name\" value=\"0\"/>" END, "<token> @name:"},
        {"<scene version=\"1.1\">\n<project width=\"16\" height=\"16\" fps=\"1\" "
         "duration=\"1\" background=\"var(--missing)\"/><composition/></scene>",
         ":2: error: <project> @background: unknown token 'missing'"},
        {"<scene version=\"1.1\">" PROJECT "\n<styles>\n"
         "<token name=\"x\" value=\"var(--missing)\"/></styles><composition/></scene>",
         ":3: error: <token> @value: unknown token 'missing'"},
        {"<scene version=\"1.0\">" PROJECT "<styles/><composition/></scene>",
         "requires version=\"1.1\": <styles>"},
        {BEGIN "<textStyle id=\"s\"/>" END, "unsupported in this build: <textStyle>"},
        {"<scene version=\"1.1\">" PROJECT "<styles extra=\"x\"/>"
         "<composition/></scene>", "<styles> @extra:"},
        {BEGIN "<token name=\"x\" value=\"1\" extra=\"x\"/>" END,
         "<token> @extra:"},
        {BEGIN "<token name=\"x\"/>" END, "value"},
        {"<scene version=\"1.1\">" PROJECT "<composition><styles/>"
         "</composition></scene>", "styles"},
        {"<scene version=\"1.1\"><project width=\"16\" height=\"16\" fps=\"1\" "
         "duration=\"1\" background=\"var(--x)\"/><styles>"
         "<token name=\"x\" value=\"42\"/></styles><composition/></scene>",
         "token 'x' does not resolve to a color"},
        {"<scene version=\"1.1\">" PROJECT "<composition>"
         "<shape id=\"s\" shape=\"rect\" width=\"8\" height=\"8\">"
         "<animate property=\"fill\"><key time=\"0\" value=\"var(--)\"/>"
         "</animate></shape></composition></scene>", "malformed token reference"},
        {BEGIN "<token name=\"x\" value=\"1\"/></styles><composition>"
         "<shape id=\"s\" shape=\"rect\" width=\"8\" height=\"8\">"
         "<animate property=\"opacity\"><key time=\"0\" value=\"var(--x)\"/>"
         "</animate></shape></composition></scene>", "<key> @value:"},
    };
    for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); ++i)
        expect_xml(t, cases[i].xml, SR_ERR_XML, cases[i].diagnostic);
    expect_xml(t, BEGIN "<token name=\"number\" value=\"42\"/>"
               "<token name=\"empty\" value=\"\"/>"
               "<token name=\"Number\" value=\"43\"/>" END, SR_OK, NULL);
    expect_xml(t, "<scene version=\"1.1\"><project width=\"16\" height=\"16\" "
               "fps=\"1\" duration=\"1\" background=\"var(--b&#114;and)\"/>"
               "<styles><token name=\"brand\" value=\"#204060\"/></styles>"
               "<composition/></scene>", SR_OK, NULL);
}

static uint32_t random_next(uint32_t *state) {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    return *state;
}

static void alias_depth_orders(sr_test_ctx *t) {
    for (unsigned edges = 64; edges <= 65; ++edges) {
        for (unsigned ordering = 0; ordering < 3; ++ordering) {
            unsigned order[66];
            for (unsigned i = 0; i <= edges; ++i)
                order[i] = ordering == 1 ? edges - i : i;
            if (ordering == 2) {
                uint32_t state = 20260926;
                for (unsigned i = edges; i > 0; --i) {
                    unsigned j = random_next(&state) % (i + 1);
                    unsigned swap = order[i]; order[i] = order[j]; order[j] = swap;
                }
            }
            char xml[8192];
            size_t used = (size_t)snprintf(xml, sizeof(xml), "%s", BEGIN);
            for (unsigned i = 0; i <= edges; ++i) {
                unsigned n = order[i];
                if (n == edges)
                    used += (size_t)snprintf(xml + used, sizeof(xml) - used,
                             "<token name=\"n%u\" value=\"#204060\"/>", n);
                else
                    used += (size_t)snprintf(xml + used, sizeof(xml) - used,
                             "<token name=\"n%u\" value=\"var(--n%u)\"/>", n, n + 1);
            }
            snprintf(xml + used, sizeof(xml) - used, "%s", END);
            expect_xml(t, xml, edges == 64 ? SR_OK : SR_ERR_XML,
                       edges == 64 ? NULL : "alias chain exceeds 64 hops");
        }
    }
}

static void resource_limits(sr_test_ctx *t) {
    char name[SR_MAX_TOKEN_NAME + 2], value[SR_MAX_TOKEN_VALUE + 2];
    char xml[SR_MAX_TOKEN_VALUE + SR_MAX_TOKEN_NAME + 1024];
    memset(name, 'n', sizeof(name));
    memset(value, 'v', sizeof(value));
    for (unsigned over = 0; over <= 1; ++over) {
        name[SR_MAX_TOKEN_NAME + over] = '\0';
        snprintf(xml, sizeof(xml), BEGIN "<token name=\"%s\" value=\"1\"/>" END, name);
        expect_xml(t, xml, over ? SR_ERR_XML : SR_OK, over ? "<token> @name:" : NULL);
        name[SR_MAX_TOKEN_NAME] = 'n';
        value[SR_MAX_TOKEN_VALUE + over] = '\0';
        snprintf(xml, sizeof(xml), BEGIN "<token name=\"x\" value=\"%s\"/>" END, value);
        expect_xml(t, xml, over ? SR_ERR_XML : SR_OK,
                   over ? "token value exceeds 4096 bytes" : NULL);
        value[SR_MAX_TOKEN_VALUE] = 'v';
    }
    size_t capacity = (SR_MAX_STYLE_TOKENS + 1) * 64 + 512;
    char *many = malloc(capacity);
    CHECK(t, many != NULL);
    if (!many) return;
    for (unsigned over = 0; over <= 1; ++over) {
        size_t used = (size_t)snprintf(many, capacity, "%s", BEGIN);
        for (unsigned i = 0; i < SR_MAX_STYLE_TOKENS + over; ++i)
            used += (size_t)snprintf(many + used, capacity - used,
                     "<token name=\"t%u\" value=\"#204060\"/>", i);
        snprintf(many + used, capacity - used, "%s", END);
        expect_xml(t, many, over ? SR_ERR_XML : SR_OK,
                   over ? "style token count exceeds 4096" : NULL);
    }
    free(many);
}

static void source_fingerprint(sr_test_ctx *t) {
    const char *docs[] = {BEGIN "<token name=\"x\" value=\"#204060\"/>" END,
                          BEGIN "<token name=\"x\" value=\"#204061\"/>" END};
    uint64_t hashes[2] = {0};
    for (size_t i = 0; i < 2; ++i) {
        SrScene scene;
        SrStatus status = st_load(t, "styles-hash.xml", docs[i], &scene, NULL);
        CHECK_INT(t, status, SR_OK);
        if (status == SR_OK) {
            hashes[i] = scene.source_hash;
            CHECK(t, hashes[i] == sr_fnv1a64(SR_FNV_OFFSET, docs[i], strlen(docs[i])));
            sr_scene_free(&scene);
        }
    }
    CHECK(t, hashes[0] != hashes[1]);
}

static bool files_equal(const char *a, const char *b) {
    FILE *left = fopen(a, "rb"), *right = fopen(b, "rb");
    bool equal = left && right;
    if (equal) {
        unsigned char x[4096], y[4096];
        size_t nx, ny;
        do {
            nx = fread(x, 1, sizeof(x), left);
            ny = fread(y, 1, sizeof(y), right);
            equal = nx == ny && !memcmp(x, y, nx);
        } while (equal && nx);
        equal = equal && !ferror(left) && !ferror(right);
    }
    if (left) fclose(left);
    if (right) fclose(right);
    return equal;
}

static void literal_render_equivalence(sr_test_ctx *t) {
    const char *fixtures[] = {"tests/golden/material-animation.xml",
                              "tests/golden/styles.xml"};
    SrScene scenes[2];
    bool loaded[2] = {false, false};
    SrDiagnostics diag;
    FILE *sink;
    st_diag(&diag, &sink);
    for (size_t i = 0; i < 2; ++i) {
        SrStatus status = sr_scene_load_xml(sr_test_data_path(fixtures[i]),
                                            &scenes[i], &diag);
        loaded[i] = status == SR_OK;
        CHECK_INT(t, status, SR_OK);
    }
    if (loaded[0] && loaded[1]) {
        char literal[1024], tokens[1024];
        snprintf(literal, sizeof(literal), "%s", sr_test_tmp_path("literal.png"));
        snprintf(tokens, sizeof(tokens), "%s", sr_test_tmp_path("tokens.png"));
        const uint64_t frames[] = {20, 0, 12, 0};
        for (size_t f = 0; f < sizeof(frames) / sizeof(frames[0]); ++f) {
            for (size_t i = 0; i < 2; ++i) {
                SrRenderOptions options = {.preview = true, .preview_frame = frames[f],
                    .preview_path = i ? tokens : literal, .encoder_threads = i ? 4 : 1};
                SrRenderMetrics metrics;
                CHECK_INT(t, sr_render(&scenes[i], &options, &metrics, &diag), SR_OK);
            }
            CHECK(t, files_equal(literal, tokens));
        }
        CHECK_INT(t, scenes[1].tokens[0].resolved_hops, 1);
        CHECK_STR(t, scenes[1].tokens[0].value, "var(--background)");
        unlink(literal);
        unlink(tokens);
    }
    for (size_t i = 0; i < 2; ++i)
        if (loaded[i]) sr_scene_free(&scenes[i]);
    if (sink) fclose(sink);
}

static void seeded_mutations(sr_test_ctx *t) {
    const char *valid = BEGIN "<token name=\"alias\" value=\"var(--ink)\"/>"
        "<token name=\"ink\" value=\"#204060\"/></styles><composition>"
        "<shape id=\"s\" shape=\"rect\" width=\"8\" height=\"8\" fill=\"var(--alias)\"/>"
        "</composition></scene>";
    const uint32_t seeds[] = {1, 110, 0x12345678};
    size_t length = strlen(valid);
    for (size_t seed = 0; seed < sizeof(seeds) / sizeof(seeds[0]); ++seed) {
        uint32_t state = seeds[seed];
        for (unsigned trial = 0; trial < 72; ++trial) {
            char xml[1024];
            memcpy(xml, valid, length + 1);
            size_t at = random_next(&state) % length;
            if (trial % 3 == 0) xml[at] = '\0';
            else if (trial % 3 == 1) memmove(xml + at, xml + at + 1, length - at);
            else xml[at] = "<&\"()-%0123"[(state >> 16) % 11];
            SrScene scene;
            SrStatus status = st_load(t, "styles-fuzz.xml", xml, &scene, NULL);
            CHECK(t, status == SR_OK || status == SR_ERR_XML);
            if (status == SR_OK) {
                for (size_t i = 0; i < scene.token_count; ++i) {
                    CHECK(t, scene.tokens[i].resolved_index < scene.token_count);
                    CHECK(t, scene.tokens[i].resolved_hops <= SR_MAX_TOKEN_DEPTH);
                }
                sr_scene_free(&scene);
            }
        }
    }
}

const sr_test_case sr_tests_styles[] = {
    {"all_color_hosts", all_color_hosts},
    {"errors_and_scope", errors_and_scope},
    {"alias_depth_orders", alias_depth_orders},
    {"resource_limits", resource_limits},
    {"source_fingerprint", source_fingerprint},
    {"literal_render_equivalence", literal_render_equivalence},
    {"seeded_mutations", seeded_mutations},
    {NULL, NULL},
};
