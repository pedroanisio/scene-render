/* SPDX-License-Identifier: Apache-2.0 */
#include "scene_render/raster.h"
#include "scene_render/assets.h"
#include "scene_render/color.h"
#include "scene_render/random.h"
#include "scene_text.h"
#include "blend_vectors.h"

#define ARRAY_COUNT(a) (sizeof(a) / sizeof((a)[0]))

static const char *const color_names[] = {
    "plus-lighter", "exclusion", "subtract", "divide", "darken", "lighten",
    "darker-color", "lighter-color", "color-dodge", "color-burn",
    "linear-dodge", "linear-burn", "soft-light", "hard-light", "linear-light",
    "vivid-light", "pin-light", "hard-mix", "hue", "saturation", "color",
    "luminosity"
};

static void naming(sr_test_ctx *t) {
    CHECK_INT(t, SR_BLEND_NORMAL, 0);
    CHECK_INT(t, SR_BLEND_DIFFERENCE, 5);
    CHECK_INT(t, ARRAY_COUNT(color_names), SR_BLEND_COUNT - 6);
    for (size_t i = 0; i < ARRAY_COUNT(color_names); ++i) {
        SrBlendMode mode = SR_BLEND_NORMAL;
        CHECK(t, sr_blend_parse(color_names[i], &mode));
        CHECK_INT(t, mode, i + 6);
        CHECK_STR(t, sr_blend_name(mode), color_names[i]);
    }
    SrBlendMode mode = SR_BLEND_ADD;
    CHECK(t, !sr_blend_parse(NULL, &mode));
    CHECK(t, !sr_blend_parse("hue", NULL));
    CHECK(t, !sr_blend_parse("Hue", &mode));
    CHECK(t, !sr_blend_parse("color ", &mode));
    CHECK(t, !sr_blend_parse("", &mode));
    CHECK_INT(t, mode, SR_BLEND_ADD);
    CHECK_STR(t, sr_blend_name((SrBlendMode)-1), "unknown");
    CHECK_STR(t, sr_blend_name(SR_BLEND_COUNT), "unknown");
}

static void scalar_references(sr_test_ctx *t) {
    for (size_t i = 0; i < ARRAY_COUNT(scalar_vectors); ++i) {
        for (size_t j = 0; j < ARRAY_COUNT(scalar_pairs); ++j) {
            double b = scalar_pairs[j][0], s = scalar_pairs[j][1];
            for (unsigned translucent = 0; translucent < 2; ++translucent) {
                float ab = translucent ? .5f : 1.0f;
                float as = translucent ? .25f : 1.0f;
                float dst[4] = {(float)b * ab, (float)b * ab,
                                (float)b * ab, ab};
                float src[4] = {(float)s * as, (float)s * as,
                                (float)s * as, as};
                double expected = (1 - ab) * s * as + (1 - as) * b * ab +
                                  as * ab * scalar_vectors[i].result[j];
                sr_blend_px(scalar_vectors[i].mode, dst, src);
                for (size_t c = 0; c < 3; ++c)
                    CHECK_NEAR(t, dst[c], expected, 2e-7);
                CHECK_NEAR(t, dst[3], as + ab * (1 - as), 0);
            }
        }
    }
}

static void nonseparable_references(sr_test_ctx *t) {
    for (size_t i = 0; i < ARRAY_COUNT(color_vectors); ++i) {
        for (unsigned m = 0; m < 4; ++m) {
            for (unsigned translucent = 0; translucent < 2; ++translucent) {
                float ab = translucent ? .5f : 1;
                float as = translucent ? .25f : 1;
                float dst[4] = {0, 0, 0, ab}, src[4] = {0, 0, 0, as};
                for (size_t c = 0; c < 3; ++c) {
                    dst[c] = (float)color_vectors[i].backdrop[c] * ab;
                    src[c] = (float)color_vectors[i].source[c] * as;
                }
                sr_blend_px((SrBlendMode)(SR_BLEND_HUE + m), dst, src);
                for (size_t c = 0; c < 3; ++c) {
                    double expected = (1 - ab) * as * color_vectors[i].source[c]
                        + (1 - as) * ab * color_vectors[i].backdrop[c]
                        + as * ab * color_vectors[i].result[m][c];
                    CHECK_NEAR(t, dst[c], expected, 2e-7);
                }
                CHECK_NEAR(t, dst[3], as + ab * (1 - as), 0);
            }
        }
    }
}

static double luminance(const float color[4]) {
    return .30 * color[0] + .59 * color[1] + .11 * color[2];
}

/* Exhaustive five-level RGB pairs exercise every channel ordering, ties,
 * grays and clipping direction without a second copy of the blend algorithm. */
static void nonseparable_invariants(sr_test_ctx *t) {
    for (unsigned b = 0; b < 125; ++b) {
        float backdrop[4] = {(b % 5) * .25f, ((b / 5) % 5) * .25f,
                            (b / 25) * .25f, 1};
        for (unsigned s = 0; s < 125; ++s) {
            float src[4] = {(s % 5) * .25f, ((s / 5) % 5) * .25f,
                           (s / 25) * .25f, 1};
            for (int m = SR_BLEND_HUE; m <= SR_BLEND_LUMINOSITY; ++m) {
                float dst[4];
                memcpy(dst, backdrop, sizeof(dst));
                sr_blend_px((SrBlendMode)m, dst, src);
                for (size_t c = 0; c < 3; ++c)
                    CHECK(t, isfinite(dst[c]) && dst[c] >= 0 && dst[c] <= 1);
                CHECK_NEAR(t, luminance(dst),
                    luminance(m == SR_BLEND_LUMINOSITY ? src : backdrop), 1e-7);
                if (m == SR_BLEND_COLOR) {
                    float inverse[4];
                    memcpy(inverse, src, sizeof(inverse));
                    sr_blend_px(SR_BLEND_LUMINOSITY, inverse, backdrop);
                    CHECK(t, !memcmp(dst, inverse, sizeof(dst)));
                }
            }
        }
    }
}

static void whole_color_and_alpha_addition(sr_test_ctx *t) {
    const float backdrop[4] = {0, 0, .9375f, 1};
    const float src[4] = {.34375f, 0, 0, 1};
    CHECK_NEAR(t, luminance(backdrop), luminance(src), 0);
    const SrBlendMode modes[] = {SR_BLEND_DARKER_COLOR, SR_BLEND_LIGHTER_COLOR};
    for (size_t i = 0; i < ARRAY_COUNT(modes); ++i) {
        float dst[4];
        memcpy(dst, backdrop, sizeof(dst));
        sr_blend_px(modes[i], dst, src);
        CHECK(t, !memcmp(dst, backdrop, sizeof(dst)));
        const float green[4] = {0, 1, 0, 1};
        sr_blend_px(modes[i], dst, green);
        CHECK(t, !memcmp(dst, i ? green : backdrop, sizeof(dst)));
        memcpy(dst, green, sizeof(dst));
        sr_blend_px(modes[i], dst, src);
        CHECK(t, !memcmp(dst, i ? green : src, sizeof(dst)));
    }
    float dst[4] = {.2f, .1f, 0, .5f};
    const float half[4] = {.1f, 0, .3f, .5f};
    sr_blend_px(SR_BLEND_PLUS_LIGHTER, dst, half);
    const double expected[4] = {.3, .1, .3, 1};
    for (size_t c = 0; c < 4; ++c) CHECK_NEAR(t, dst[c], expected[c], 2e-7);
    const float hdr[4] = {2, -.5f, .5f, 1};
    sr_blend_px(SR_BLEND_PLUS_LIGHTER, dst, hdr);
    CHECK_NEAR(t, dst[0], 1, 0);
    CHECK_NEAR(t, dst[1], 0, 0);
    CHECK_NEAR(t, dst[2], .8, 2e-7);
    CHECK_NEAR(t, dst[3], 1, 0);
}

static void transparency_hdr_and_dispatch(sr_test_ctx *t) {
    const float backdrop[4] = {.25f, .125f, .375f, .5f};
    const float clear[4] = {0, 0, 0, 0};
    const float tiny[4] = {1e-40f, 2e-40f, 3e-40f, 1e-40f};
    const float source[4] = {.0625f, .125f, .1875f, .25f};
    for (int m = SR_BLEND_PLUS_LIGHTER; m < SR_BLEND_COUNT; ++m) {
        float dst[4], expected[4];
        memcpy(dst, backdrop, sizeof(dst));
        sr_blend_px((SrBlendMode)m, dst, clear);
        CHECK(t, !memcmp(dst, backdrop, sizeof(dst)));
        memset(dst, 0, sizeof(dst));
        sr_blend_px((SrBlendMode)m, dst, source);
        CHECK(t, !memcmp(dst, source, sizeof(dst)));
        memcpy(dst, backdrop, sizeof(dst));
        memcpy(expected, dst, sizeof(expected));
        sr_blend_px_inline((SrBlendMode)m, dst, source);
        sr_blend_px((SrBlendMode)m, expected, source);
        CHECK(t, !memcmp(dst, expected, sizeof(dst)));
        if (m == SR_BLEND_PLUS_LIGHTER) continue;
        memcpy(dst, backdrop, sizeof(dst));
        memcpy(expected, dst, sizeof(expected));
        sr_blend_px((SrBlendMode)m, dst, tiny);
        sr_blend_px(SR_BLEND_NORMAL, expected, tiny);
        CHECK(t, !memcmp(dst, expected, sizeof(dst)));
        memcpy(dst, tiny, sizeof(dst));
        memcpy(expected, dst, sizeof(expected));
        sr_blend_px((SrBlendMode)m, dst, source);
        sr_blend_px(SR_BLEND_NORMAL, expected, source);
        CHECK(t, !memcmp(dst, expected, sizeof(dst)));
        float extended[4] = {1, -.25f, .25f, .5f};
        const float glow[4] = {-.125f, .5f, .125f, .25f};
        float clipped[4] = {1, 0, .5f, 1};
        const float bounded[4] = {0, 1, .5f, 1};
        sr_blend_px((SrBlendMode)m, clipped, bounded);
        sr_blend_px((SrBlendMode)m, extended, glow);
        const double raw_b[3] = {1, -.25, .25};
        for (size_t c = 0; c < 3; ++c)
            CHECK_NEAR(t, extended[c], .5 * glow[c] + .75 * raw_b[c] +
                       .125 * clipped[c], 2e-7);
        CHECK_NEAR(t, extended[3], .625, 0);
    }
}

static void transparent_plus_lighter_hdr(sr_test_ctx *t) {
    float dst[4] = {2, -.25f, .5f, 1};
    const float clear[4] = {0, 0, 0, 0};
    sr_blend_px(SR_BLEND_PLUS_LIGHTER, dst, clear);
    const float expected[4] = {2, -.25f, .5f, 1};
    for (size_t c = 0; c < 4; ++c) CHECK_NEAR(t, dst[c], expected[c], 0);
    const float tiny[4] = {0, 0, 0, 1e-40f};
    sr_blend_px(SR_BLEND_PLUS_LIGHTER, dst, tiny);
    CHECK_NEAR(t, dst[0], 1, 0);
    CHECK_NEAR(t, dst[1], 0, 0);
}

static void hard_mix_boundary(sr_test_ctx *t) {
    const float src[4] = {.75f, .75f, .75f, 1};
    float dst[4] = {nextafterf(.25f, 0), .25f, nextafterf(.25f, 1), 1};
    sr_blend_px(SR_BLEND_HARD_MIX, dst, src);
    CHECK_NEAR(t, dst[0], 0, 0);
    CHECK_NEAR(t, dst[1], 1, 0);
    CHECK_NEAR(t, dst[2], 1, 0);
}

#define PROJECT "<project width=\"16\" height=\"16\" fps=\"2\" " \
                "duration=\"2\" linearLight=\"false\"/>\n"
#define ASSET "<assets><vector id=\"a\" shape=\"rect\" width=\"8\" " \
              "height=\"8\" fill=\"#E02060C0\"/></assets>\n"

static void host_xml(char *xml, size_t size, const char *version,
                      unsigned host, const char *mode) {
    static const char *const bodies[] = {
        "<shape id=\"n\" shape=\"rect\" width=\"8\" height=\"8\" "
        "x=\"4\" y=\"4\" fill=\"#E02060C0\" opacity=\".75\" blend=\"%s\"/>",
        "<group id=\"n\" x=\"4\" y=\"4\" opacity=\".75\" blend=\"%s\">"
        "<shape id=\"child\" shape=\"rect\" width=\"8\" height=\"8\" "
        "fill=\"#E02060C0\"/></group>",
        "<layer id=\"n\" asset=\"a\" x=\"4\" y=\"4\" opacity=\".75\" blend=\"%s\"/>",
        "<particleEmitter id=\"n\" x=\"8\" y=\"8\" size=\"4\" rate=\"1\" "
        "lifetime=\"2\" speed=\"0\" color=\"#E02060C0\" seed=\"7\" "
        "opacity=\".75\" blend=\"%s\"/>"
    };
    char node[768];
    const char *slot = strstr(bodies[host], "%s");
    snprintf(node, sizeof(node), "%.*s%s%s", (int)(slot - bodies[host]),
             bodies[host], mode, slot + 2);
    snprintf(xml, size, "<scene version=\"%s\">\n" PROJECT ASSET
             "<composition>\n%s</composition></scene>", version, node);
}

static void xml_hosts_and_pixels(sr_test_ctx *t) {
    for (unsigned host = 0; host < 4; ++host) {
        for (size_t m = 0; m < ARRAY_COUNT(color_names); ++m) {
            char xml[1536];
            SrScene scene;
            host_xml(xml, sizeof(xml), "1.1", host, color_names[m]);
            SrStatus status = st_load(t, "blend-host.xml", xml, &scene, NULL);
            CHECK_INT(t, status, SR_OK);
            if (status != SR_OK) continue;
            SrNode *node = sr_scene_find_node(&scene, "n");
            CHECK(t, node != NULL);
            if (!node) { sr_scene_free(&scene); continue; }
            CHECK_INT(t, node->blend, SR_BLEND_PLUS_LIGHTER + m);
            unsigned char saved[sizeof(*node)];
            memcpy(saved, node, sizeof(saved));
            uint64_t hash = scene.source_hash;
            FILE *sink;
            SrDiagnostics diag;
            st_diag(&diag, &sink);
            status = sr_assets_load(&scene, &diag);
            CHECK_INT(t, status, SR_OK);
            const float background[4] = {.125f, .375f, .25f, .5f};
            float expected[4], source[4];
            memcpy(expected, background, sizeof(expected));
            sr_color_to_blend(&scene.project,
                (SrColor){224.0/255, 32.0/255, 96.0/255, 192.0/255}, source);
            for (size_t c = 0; c < 4; ++c) source[c] *= .75f;
            sr_blend_px(node->blend, expected, source);
            for (unsigned threads = 1; status == SR_OK && threads <= 4;
                 threads += 3) {
                SrCompositor compositor;
                SrFrame frame = {0};
                sr_compositor_init(&compositor, threads);
                status = sr_frame_init(&frame, 16, 16);
                CHECK_INT(t, status, SR_OK);
                const double times[] = {.5, 0, 1, 0};
                for (size_t f = 0; status == SR_OK && f < ARRAY_COUNT(times); ++f) {
                    sr_frame_clear(&frame, background, threads);
                    status = sr_compositor_render_scene(&compositor, &scene,
                        times[f], &frame, &diag);
                    CHECK_INT(t, status, SR_OK);
                    if (times[f] == 0 && status == SR_OK) {
                        for (size_t c = 0; c < 4; ++c)
                            CHECK_NEAR(t, st_px(&frame, 8, 8)[c], expected[c], 2e-6);
                        CHECK(t, !memcmp(st_px(&frame, 0, 0), background,
                                         sizeof(background)));
                    }
                }
                sr_frame_free(&frame);
                sr_compositor_free(&compositor);
            }
            CHECK(t, scene.source_hash == hash);
            CHECK(t, !memcmp(node, saved, sizeof(saved)));
            if (sink) fclose(sink);
            sr_scene_free(&scene);
        }
    }
}

static void version_and_pending_gates(sr_test_ctx *t) {
    static const char *const pending[] = {"dissolve", "stencil-alpha",
        "stencil-luma", "silhouette-alpha", "silhouette-luma", "alpha-add",
        "behind"};
    for (unsigned host = 0; host < 4; ++host) {
        for (size_t m = 0; m < ARRAY_COUNT(color_names) + ARRAY_COUNT(pending); ++m) {
            bool implemented = m < ARRAY_COUNT(color_names);
            const char *mode = implemented ? color_names[m]
                : pending[m - ARRAY_COUNT(color_names)];
            char xml[1536];
            host_xml(xml, sizeof(xml), implemented ? "1.0" : "1.1", host, mode);
            char *message = NULL;
            SrScene scene;
            SrStatus status = st_load(t, "blend-gate.xml", xml, &scene, &message);
            CHECK_INT(t, status, SR_ERR_XML);
            CHECK_CONTAINS(t, message, implemented ? "requires version=\"1.1\""
                                                  : "unsupported in this build");
            CHECK_CONTAINS(t, message, ":5: error:");
            CHECK_CONTAINS(t, message, "@blend:");
            if (status == SR_OK) sr_scene_free(&scene);
            free(message);
        }
    }
}

static void seeded_enum_inputs(sr_test_ctx *t) {
    uint64_t state = 0x1122;
    for (unsigned trial = 0; trial < 96; ++trial) {
        state = sr_random_mix64(state);
        const char *mode = color_names[state % ARRAY_COUNT(color_names)];
        char xml[1536];
        host_xml(xml, sizeof(xml), "1.1", (unsigned)(state % 4), mode);
        if (trial % 3 == 1) {
            char *value = strstr(xml, "blend=\"") + strlen("blend=\"");
            value[(state >> 8) % strlen(mode)] = '?';
        } else if (trial % 3 == 2) {
            xml[(state >> 8) % strlen(xml)] = '\0';
        }
        SrScene scene;
        SrStatus status = st_load(t, "blend-seeded.xml", xml, &scene, NULL);
        CHECK_INT(t, status, trial % 3 == 0 ? SR_OK : SR_ERR_XML);
        if (status == SR_OK) sr_scene_free(&scene);
    }
}

static void render_paths_and_threads(sr_test_ctx *t) {
    FILE *sink;
    SrDiagnostics diag;
    st_diag(&diag, &sink);
    SrScene scene;
    SrStatus status = sr_scene_load_xml(sr_test_data_path(
        "tests/data-blend-colors.xml"), &scene, &diag);
    CHECK_INT(t, status, SR_OK);
    if (status != SR_OK) { if (sink) fclose(sink); return; }
    status = sr_assets_load(&scene, &diag);
    CHECK_INT(t, status, SR_OK);
    SrFrame frames[2] = {{0}};
    SrCompositor compositors[2];
    for (size_t i = 0; i < 2; ++i) {
        sr_compositor_init(&compositors[i], i ? 4 : 1);
        if (status == SR_OK) status = sr_frame_init(&frames[i], 192, 128);
        CHECK_INT(t, status, SR_OK);
    }
    uint64_t hash = scene.source_hash;
    float background[4];
    sr_color_to_blend(&scene.project, scene.project.background, background);
    const double times[] = {23.0 / 12, 0, 1, .25, 1, 0};
    for (size_t f = 0; status == SR_OK && f < ARRAY_COUNT(times); ++f) {
        sr_compositor_free(&compositors[1]);
        sr_compositor_init(&compositors[1], 4);
        for (size_t i = 0; i < 2; ++i) {
            sr_frame_clear(&frames[i], background, i ? 4 : 1);
            status = sr_compositor_render_scene(&compositors[i], &scene,
                times[f], &frames[i], &diag);
            CHECK_INT(t, status, SR_OK);
            if (status != SR_OK) break;
        }
        if (status == SR_OK) CHECK(t, st_frames_equal(&frames[0], &frames[1]));
    }
    CHECK(t, scene.source_hash == hash);
    for (size_t i = 0; i < 2; ++i) {
        sr_compositor_free(&compositors[i]);
        sr_frame_free(&frames[i]);
    }
    sr_scene_free(&scene);
    if (sink) fclose(sink);
}

const sr_test_case sr_tests_blend_color[] = {
    {"naming", naming},
    {"scalar_references", scalar_references},
    {"nonseparable_references", nonseparable_references},
    {"nonseparable_invariants", nonseparable_invariants},
    {"whole_color_and_alpha_addition", whole_color_and_alpha_addition},
    {"transparency_hdr_and_dispatch", transparency_hdr_and_dispatch},
    {"transparent_plus_lighter_hdr", transparent_plus_lighter_hdr},
    {"hard_mix_boundary", hard_mix_boundary},
    {"xml_hosts_and_pixels", xml_hosts_and_pixels},
    {"version_and_pending_gates", version_and_pending_gates},
    {"seeded_enum_inputs", seeded_enum_inputs},
    {"render_paths_and_threads", render_paths_and_threads},
    {NULL, NULL}
};
