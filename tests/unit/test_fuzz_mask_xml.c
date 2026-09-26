/* SPDX-License-Identifier: Apache-2.0 */
/* Seeded valid, mutated and truncated XML for the B1-3 loader additions:
 * advanced mask attributes, track mattes and adjustment layers. Every
 * input must load or fail with a diagnostic, and every loaded scene must
 * render or fail cleanly (under ASan/UBSan in CI). */
#include "b13_fixture.h"
#include "scene_render/random.h"

#define ARRAY_COUNT(a) (sizeof(a) / sizeof((a)[0]))

typedef struct {
    uint64_t state;
} Generator;

static uint64_t next(Generator *g) {
    g->state = sr_random_mix64(g->state);
    return g->state;
}

/* The last item of each pool is invalid and chosen one time in sixteen. */
static const char *pick(Generator *g, const char *const *items, size_t count) {
    if (next(g) % 16 == 0) return items[count - 1];
    return items[next(g) % (count - 1)];
}

/* Appends prefix, value and suffix. */
static void append3(char *out, size_t size, const char *prefix, const char *value,
                    const char *suffix) {
    size_t used = strlen(out);
    if (used + 1 < size) snprintf(out + used, size - used, "%s%s%s", prefix, value, suffix);
}

static void mask_element(Generator *g, char *out, size_t size) {
    static const char *const types[] = {"rect", "ellipse", "rounded-rect", "path",
                                        "polygon", "star", "hexagon"};
    static const char *const coordinates[] = {"0", "4", "-2", "2.5", "nan"};
    static const char *const sizes[] = {"8", "12", "3.5", "0"};
    static const char *const radii[] = {"0", "3", "6", "-1"};
    static const char *const feathers[] = {"0", "1", "2.5", "6", "5000"};
    static const char *const expansions[] = {"0", "1", "-1", "2.5", "-3", "nan"};
    static const char *const opacities[] = {"1", ".5", "0", "3"};
    static const char *const inner[] = {"0", "2", "5", "-1"};
    static const char *const points[] = {"3", "5", "7", "4096", "2"};
    static const char *const modes[] = {"intersect", "add", "subtract", "lighten",
                                        "darken", "difference", "none", "xor"};
    static const char *const paths[] = {"M 0 0 L 8 0 L 8 8 Z", "m 1 1 l 4 0 l 0 4 z",
        "M 0 0 C 4 0 4 8 0 8 Q 2 4 0 0 Z", "M 0 0 L", "M 1e10 0 L 0 0 L 0 1 Z",
        "M 0 0 H 8 V 8 H 0 Z M 2 2 h 4 v 4 h -4 z", "Z", "M 0 0 L 1 1 L", ""};
    const char *type = pick(g, types, ARRAY_COUNT(types));
    snprintf(out, size, "<mask type=\"%s\"", type);
    struct { const char *name; const char *const *pool; size_t count; } fields[] = {
        {"x", coordinates, ARRAY_COUNT(coordinates)},
        {"y", coordinates, ARRAY_COUNT(coordinates)},
        {"radius", radii, ARRAY_COUNT(radii)},
        {"feather", feathers, ARRAY_COUNT(feathers)},
        {"expansion", expansions, ARRAY_COUNT(expansions)},
        {"opacity", opacities, ARRAY_COUNT(opacities)},
        {"innerRadius", inner, ARRAY_COUNT(inner)},
        {"points", points, ARRAY_COUNT(points)}};
    bool simple = !strcmp(type, "rect") || !strcmp(type, "ellipse") ||
                  !strcmp(type, "rounded-rect");
    bool polygonal = !strcmp(type, "polygon") || !strcmp(type, "star");
    if (simple || next(g) % 3 == 0) {
        append3(out, size, " width=\"", pick(g, sizes, ARRAY_COUNT(sizes)), "\"");
        append3(out, size, " height=\"", pick(g, sizes, ARRAY_COUNT(sizes)), "\"");
    }
    for (size_t i = 0; i < ARRAY_COUNT(fields); ++i) {
        bool fitting = (i != 6 || !strcmp(type, "star")) && (i != 7 || polygonal);
        if (next(g) % 3 || (!fitting && next(g) % 8)) continue;
        char prefix[64];
        snprintf(prefix, sizeof(prefix), " %s=\"", fields[i].name);
        append3(out, size, prefix, pick(g, fields[i].pool, fields[i].count), "\"");
    }
    if (next(g) % 2) append3(out, size, " mode=\"", pick(g, modes, ARRAY_COUNT(modes)), "\"");
    if (!strcmp(type, "path") || next(g) % 16 == 0)
        append3(out, size, " path=\"", pick(g, paths, ARRAY_COUNT(paths)), "\"");
    if (next(g) % 4 == 0) append3(out, size, " fillRule=\"",
                                   next(g) % 2 ? "evenodd" : "nonzero", "\"");
    if (next(g) % 4 == 0) append3(out, size, " invert=\"", "true", "\"");
    if (next(g) % 5 == 0)
        append3(out, size, "", "><animate property=\"feather\"><key time=\"0\" "
                "value=\"0\" interpolation=\"back-out\"/><key time=\"1\" "
                "value=\"8\"/></animate></mask>", "");
    else
        append3(out, size, "", "/>", "");
}

static void build(Generator *g, char *xml, size_t size) {
    static const char *const blends[] = {"normal", "dissolve", "stencil-alpha",
        "stencil-luma", "silhouette-alpha", "silhouette-luma", "alpha-add",
        "behind", "multiply", "bogus"};
    static const char *const modes[] = {"alpha", "alpha-inverted", "luma",
                                        "luma-inverted", "red"};
    static const char *const ids[] = {"a", "b", "g", "adj", "zz"};
    char masks[3][1024];
    for (size_t i = 0; i < 3; ++i) {
        masks[i][0] = '\0';
        if (next(g) % 2) mask_element(g, masks[i], sizeof(masks[i]));
    }
    char matte_a[96] = "", matte_b[96] = "", matte_adj[96] = "";
    if (next(g) % 2) snprintf(matte_a, sizeof(matte_a), " matte=\"%s\" matteMode=\"%s\"",
                              pick(g, ids, ARRAY_COUNT(ids)), pick(g, modes, ARRAY_COUNT(modes)));
    if (next(g) % 3 == 0) snprintf(matte_b, sizeof(matte_b), " matte=\"%s\" matteVisible=\"%s\"",
                                   pick(g, ids, ARRAY_COUNT(ids)), next(g) % 2 ? "true" : "false");
    if (next(g) % 3 == 0) snprintf(matte_adj, sizeof(matte_adj), " matte=\"%s\"",
                                   pick(g, ids, ARRAY_COUNT(ids)));
    snprintf(xml, size, "<scene version=\"%s\">\n"
             "<project width=\"16\" height=\"16\" fps=\"2\" duration=\"1\"/>\n"
             "<composition><group id=\"g\" blend=\"%s\">"
             "<shape id=\"a\" shape=\"rect\" width=\"12\" height=\"12\" blend=\"%s\"%s>%s</shape>"
             "<shape id=\"b\" shape=\"ellipse\" x=\"4\" width=\"10\" height=\"10\"%s>%s</shape>"
             "</group><adjustment id=\"adj\" effects=\"%s\"%s>%s</adjustment>"
             "</composition><effects><effect id=\"fx\" type=\"blur\" radius=\"2\"/>"
             "</effects></scene>",
             next(g) % 8 ? "1.1" : "1.0", pick(g, blends, ARRAY_COUNT(blends)),
             pick(g, blends, ARRAY_COUNT(blends)), matte_a, masks[0], matte_b, masks[1],
             next(g) % 6 ? "fx" : "missing", matte_adj, masks[2]);
}

static void seeded_inputs(sr_test_ctx *t) {
    size_t loaded = 0, rendered = 0;
    for (uint64_t seed = 1; seed <= 1200; ++seed) {
        Generator g = {seed * UINT64_C(0x9E3779B97F4A7C15)};
        char xml[8192];
        build(&g, xml, sizeof(xml));
        size_t length = strlen(xml);
        unsigned variant = (unsigned)(seed % 3);
        if (variant == 1) {
            for (unsigned k = 0; k < 3; ++k) {
                size_t at = next(&g) % length;
                xml[at] = (char)(' ' + next(&g) % 90);
            }
        } else if (variant == 2) {
            xml[next(&g) % length] = '\0';
        }
        SrScene scene;
        char *message = NULL;
        SrStatus status = b13_load(t, xml, &scene, &message);
        CHECK(t, status == SR_OK || status == SR_ERR_XML || status == SR_ERR_RENDER ||
                 status == SR_ERR_ASSET);
        if (status != SR_OK) CHECK(t, message && strstr(message, "error"));
        if (status != SR_OK && getenv("SR_DEBUG_FUZZ") && variant == 0)
            fprintf(stderr, "%s\n", message ? message : "");
        free(message);
        if (status != SR_OK) continue;
        ++loaded;
        SrFrame frame;
        char *failure = NULL;
        status = b13_render(&scene, .5, (unsigned)(1 + seed % 2 * 3), &frame, &failure);
        if (status != SR_OK && getenv("SR_DEBUG_FUZZ"))
            fprintf(stderr, "render: %s\n", failure ? failure : "");
        free(failure);
        CHECK(t, status == SR_OK || status == SR_ERR_RENDER);
        rendered += status == SR_OK;
        sr_frame_free(&frame);
        sr_scene_free(&scene);
    }
    if (getenv("SR_DEBUG_FUZZ"))
        fprintf(stderr, "loaded %zu rendered %zu\n", loaded, rendered);
    CHECK(t, loaded >= 25);
    CHECK(t, rendered >= 25);
}

const sr_test_case sr_tests_fuzz_mask_xml[] = {
    {"seeded_inputs", seeded_inputs},
    {NULL, NULL}
};
