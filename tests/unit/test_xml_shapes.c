/* SPDX-License-Identifier: Apache-2.0 */
/* B1-4 loading: every construct of tests/data-shapes.xml, rejection
 * diagnostics with lines and attributes, version gates, seeded document
 * mutations, and rendering contracts (legacy identity, exact area, thread
 * and frame-order invariance, scene immutability, deformation, cards,
 * ledger quotas). */
#include "scene_render/paint.h"
#include "scene_render/renderer.h"
#include "scene_render/random.h"
#include "vector_path_internal.h"
#include "compositing_internal.h"

#include <stdlib.h>

#include "harness.h"
#include "scene_text.h"

static SrNode *child(SrScene *scene, const char *id) {
    return sr_scene_find_node(scene, id);
}

static const SrPaint *paint(const SrScene *scene, const char *id) {
    for (size_t i = 0; i < scene->paint_count; ++i)
        if (!strcmp(scene->paints[i].id, id)) return &scene->paints[i];
    return NULL;
}

static void fixture_constructs(sr_test_ctx *t) {
    SrScene scene;
    SrDiagnostics diag;
    FILE *sink;
    st_diag(&diag, &sink);
    SrStatus status = sr_scene_load_xml(sr_test_data_path("tests/data-shapes.xml"),
                                        &scene, &diag);
    if (sink) fclose(sink);
    CHECK_INT(t, status, SR_OK);
    if (status != SR_OK) return;
    CHECK(t, scene.compositing_required && scene.compositing);
    CHECK_INT(t, scene.paint_count, 6);
    CHECK(t, scene.background_paint.paint == paint(&scene, "sky"));
    const SrPaint *glow = paint(&scene, "glow");
    CHECK(t, glow && glow->type == SR_PAINT_RADIAL && glow->fx_set && glow->fy_set);
    CHECK(t, glow && glow->space == SR_INTERP_OKLCH && glow->stop_count == 3);
    CHECK(t, glow && glow->stops[1].color.r.count == 2);
    CHECK(t, glow && sr_paint_has_animation(glow));
    CHECK(t, glow && glow->dither_key == sr_paint_dither_key("glow", 11));
    const SrPaint *warm = paint(&scene, "warm");
    CHECK(t, warm && warm->spread == SR_SPREAD_REFLECT && !warm->dither);
    CHECK(t, warm && warm->stops[0].midpoint.base == 0.3);
    CHECK(t, warm && warm->stops[0].color.base.r > 0.9);   /* var(--accent) */
    CHECK(t, paint(&scene, "cool")->units == SR_PAINT_UNITS_USER);
    SrNode *box = child(&scene, "box");
    CHECK(t, box->shape_style.extended && box->shape == SR_SHAPE_RECT);
    CHECK(t, box->shape_style.fill_paint.paint == glow);
    CHECK_INT(t, box->shape_style.radius.track.count, 2);
    SrNode *corners = child(&scene, "corners");
    CHECK_NEAR(t, corners->shape_style.corner_radii[1], 6.0, 0.0);
    CHECK_INT(t, corners->shape_style.stroke.position, SR_STROKE_OUTSIDE);
    CHECK_INT(t, corners->shape_style.stroke.dash_count, 2);
    SrNode *line = child(&scene, "rule-line");
    CHECK_INT(t, line->shape, SR_SHAPE_LINE);
    CHECK_INT(t, line->shape_style.stroke.dash_count, 6);   /* odd list doubled */
    CHECK_NEAR(t, line->shape_style.stroke.dash[3], 5.0, 0.0);
    CHECK_INT(t, line->shape_style.stroke.cap, SR_LINE_CAP_SQUARE);
    SrNode *star = child(&scene, "star");
    CHECK(t, star->shape_style.inner_radius_set && star->shape_style.outer_radius_set);
    CHECK_INT(t, star->shape_style.stroke.order, SR_PAINT_STROKE_FILL);
    SrNode *draw = child(&scene, "draw");
    CHECK(t, draw->shape_style.path && draw->shape_style.path->count == 2);
    CHECK_INT(t, draw->shape_style.trim_mode, SR_TRIM_SEQUENTIAL);
    CHECK_NEAR(t, draw->shape_style.path_bounds[2], 58.0, 1e-12);
    CHECK_INT(t, child(&scene, "loops")->shape_style.fill_rule, SR_FILL_EVENODD);
    const SrAsset *pill = sr_scene_find_asset(&scene, "pill");
    CHECK(t, pill && pill->vector_ext && pill->vector_ext->extended);
    CHECK(t, pill && pill->vector_ext->fill_paint.paint == warm);
    const SrAsset *spark = sr_scene_find_asset(&scene, "spark");
    CHECK(t, spark && spark->vector_ext->inner_radius_set && spark->vector_ext->points == 4);
    sr_scene_free(&scene);
}

static const char *const header = "<scene version=\"1.1\"><project width=\"32\" "
    "height=\"24\" fps=\"4\" duration=\"1\"/>";

static void expect_rejection(sr_test_ctx *t, const char *body, const char *needle,
                             const char *version) {
    char xml[4096];
    snprintf(xml, sizeof(xml), "<scene version=\"%s\"><project width=\"32\" "
             "height=\"24\" fps=\"4\" duration=\"1\"/>%s</scene>", version, body);
    SrScene scene;
    char *message = NULL;
    SrStatus status = st_load(t, "shape-reject.xml", xml, &scene, &message);
    if (status == SR_OK) {
        SR_FAIL(t, "accepted: %s", body);
        sr_scene_free(&scene);
    } else {
        CHECK_CONTAINS(t, message, needle);
    }
    free(message);
}

static void rejections_name_attributes(sr_test_ctx *t) {
    const struct { const char *body, *needle; } cases[] = {
        {"<composition><shape id=\"a\" shape=\"polygon\" width=\"8\" height=\"8\" "
         "points=\"2\"/></composition>", "@points"},
        {"<composition><shape id=\"a\" shape=\"ellipse\" width=\"8\" height=\"8\" "
         "radius=\"2\"/></composition>", "corner radii require"},
        {"<composition><shape id=\"a\" shape=\"polygon\" width=\"8\" height=\"8\" "
         "innerRadius=\"2\"/></composition>", "requires shape=star"},
        {"<composition><shape id=\"a\" shape=\"rect\" width=\"8\" height=\"8\" "
         "points=\"6\"/></composition>", "requires shape=polygon"},
        {"<composition><shape id=\"a\" shape=\"path\" width=\"8\" height=\"8\"/>"
         "</composition>", "path is required"},
        {"<composition><shape id=\"a\" shape=\"rect\" width=\"8\" height=\"8\" "
         "path=\"M0 0 L1 1\"/></composition>", "path requires shape=path"},
        {"<composition><shape id=\"a\" shape=\"path\" width=\"8\" height=\"8\" "
         "path=\"M0 0 X\"/></composition>", "invalid or oversized path at byte 5"},
        {"<composition><shape id=\"a\" shape=\"rect\" width=\"8\" height=\"8\" "
         "miterLimit=\"0.5\"/></composition>", "miter limit in [1, 1e6]"},
        {"<composition><shape id=\"a\" shape=\"rect\" width=\"8\" height=\"8\" "
         "dash=\"1 -2\"/></composition>", "non-negative lengths"},
        {"<composition><shape id=\"a\" shape=\"rect\" width=\"8\" height=\"8\" "
         "cornerRadii=\"1 2 3 4 5\"/></composition>", "one to four"},
        {"<composition><shape id=\"a\" shape=\"line\" width=\"8\" height=\"8\" "
         "strokePosition=\"inside\"/></composition>", "accepts only center"},
        {"<composition><shape id=\"a\" shape=\"star\" width=\"8\" height=\"8\" "
         "innerRadius=\"5\" outerRadius=\"3\"/></composition>", "must not exceed"},
        {"<composition><shape id=\"a\" shape=\"rect\" width=\"8\" height=\"8\" "
         "fill=\"url(#missing)\"/></composition>", "unknown paint id 'missing'"},
        {"<composition><shape id=\"a\" shape=\"rect\" width=\"8\" height=\"8\" "
         "fill=\"url(#b)\"/><shape id=\"b\" shape=\"rect\" width=\"8\" height=\"8\"/>"
         "</composition>", "'b' is not a gradient paint"},
        {"<paints><linearGradient id=\"g\"/></paints><composition/>",
         "needs at least one stop"},
        {"<paints><linearGradient id=\"g\"><stop offset=\"0\" color=\"#FFFFFF\"/>"
         "</linearGradient><linearGradient id=\"g2\"><stop offset=\"0\" "
         "color=\"#000000\"/></linearGradient></paints><composition><shape id=\"g\" "
         "shape=\"rect\" width=\"8\" height=\"8\"/></composition>", "xs:ID"},
        {"<assets><vector id=\"v\" shape=\"rect\" width=\"8\" height=\"8\" "
         "fill=\"url(#g)\"/></assets><paints><linearGradient id=\"g\"><animate "
         "property=\"x1\"><key time=\"0\" value=\"0\"/><key time=\"1\" value=\"1\"/>"
         "</animate><stop offset=\"0\" color=\"#FFFFFF\"/></linearGradient></paints>"
         "<composition/>", "rasterized once"},
        {"<paints><linearGradient id=\"g\"><stop offset=\"0\" color=\"#FFFFFF\"/>"
         "</linearGradient></paints><composition><shape id=\"a\" shape=\"rect\" "
         "width=\"8\" height=\"8\" fill=\"url(#g)\"><animate property=\"fill\">"
         "<key time=\"0\" value=\"#FF0000\"/></animate></shape></composition>",
         "cannot also have a colour animation"},
        {"<composition><shape id=\"a\" shape=\"ellipse\" width=\"8\" height=\"8\">"
         "<animate property=\"radius\"><key time=\"0\" value=\"1\"/></animate>"
         "</shape></composition>", "does not apply"},
        {"<composition><shape id=\"a\" shape=\"rect\" width=\"8\" height=\"8\">"
         "<animate property=\"trimEnd\"><key time=\"0\" value=\"2\"/></animate>"
         "</shape></composition>", "trimEnd keys must be in [0,1]"},
        {"<assets><vector id=\"v\" shape=\"polygon\" width=\"16385\" height=\"1\"/>"
         "</assets><composition/>", "16384 px per side"},
        {"<paints><radialGradient id=\"g\"><stop offset=\"0\" color=\"#FFFFFF\">"
         "<animate property=\"offset\"><key time=\"0\" value=\"3\"/></animate>"
         "</stop></radialGradient></paints><composition/>", "stop offset keys"},
    };
    for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); ++i)
        expect_rejection(t, cases[i].body, cases[i].needle, "1.1");
    /* Diagnostics carry the source line of the offending element. */
    char *message = NULL;
    SrScene scene;
    char xml[1024];
    snprintf(xml, sizeof(xml), "%s\n<composition>\n\n<shape id=\"a\" shape=\"star\" "
             "width=\"8\" height=\"8\" points=\"9000\"/></composition></scene>", header);
    CHECK(t, st_load(t, "shape-line.xml", xml, &scene, &message) != SR_OK);
    CHECK_CONTAINS(t, message, ":4:");
    free(message);
}

/* B1-5 key markers reach the B1-4 paint and stop hosts (project clock). */
static void key_markers_on_paint_hosts(sr_test_ctx *t) {
    const char *xml = "<scene version=\"1.1\"><project width=\"32\" height=\"24\" "
        "fps=\"4\" duration=\"2\"/><paints><linearGradient id=\"g\"><animate "
        "property=\"x2\"><key time=\"0\" value=\"1\"/><key marker=\"cue\" "
        "time=\"0\" value=\"0.5\"/></animate><stop offset=\"0\" color=\"#FFFFFF\">"
        "<animate property=\"opacity\"><key time=\"0\" value=\"1\"/><key "
        "marker=\"cue\" time=\"0.25\" value=\"0\"/></animate></stop>"
        "</linearGradient></paints><markers><marker id=\"cue\" time=\"1.5\"/>"
        "</markers><composition><shape id=\"a\" shape=\"rect\" width=\"8\" "
        "height=\"8\" fill=\"url(#g)\" name=\"swatch\" tags=\"ui\"/></composition>"
        "</scene>";
    SrScene scene;
    char *message = NULL;
    SrStatus status = st_load(t, "paint-markers.xml", xml, &scene, &message);
    CHECK_INT(t, status, SR_OK);
    if (status != SR_OK) {
        fprintf(stderr, "%s", message ? message : "");
        free(message);
        return;
    }
    free(message);
    const SrPaint *g = paint(&scene, "g");
    CHECK(t, g && g->x2.track.count == 2);
    CHECK_NEAR(t, g->x2.track.keys[1].time, 1.5, 1e-12);
    CHECK_NEAR(t, g->stops[0].opacity.track.keys[1].time, 1.75, 1e-12);
    sr_scene_free(&scene);
}

static void version_gates(sr_test_ctx *t) {
    /* New kinds and elements need 1.1; new attributes on 1.0 elements do not. */
    expect_rejection(t, "<composition><shape id=\"a\" shape=\"polygon\" width=\"8\" "
                     "height=\"8\"/></composition>", "requires version=\"1.1\"", "1.0");
    expect_rejection(t, "<paints/><composition/>", "requires version", "1.0");
    expect_rejection(t, "<composition><shape id=\"a\" shape=\"rect\" width=\"8\" "
                     "height=\"8\"><animate property=\"trimEnd\"><key time=\"0\" "
                     "value=\"1\"/></animate></shape></composition>",
                     "requires version", "1.0");
    const char *xml = "<scene version=\"1.0\"><project width=\"32\" height=\"24\" "
        "fps=\"4\" duration=\"1\"/><composition><shape id=\"a\" shape=\"rect\" "
        "width=\"8\" height=\"8\" strokeWidth=\"2\" stroke=\"#FFFFFF\" "
        "strokeJoin=\"round\" dash=\"2 2\" trimEnd=\"0.5\"/></composition></scene>";
    SrScene scene;
    CHECK_INT(t, st_load(t, "shape-10.xml", xml, &scene, NULL), SR_OK);
    if (scene.root) {
        CHECK(t, child(&scene, "a")->shape_style.extended);
        sr_scene_free(&scene);
    }
    /* The B1-4 stroke-width bound does not apply to 1.0 shapes. */
    xml = "<scene version=\"1.0\"><project width=\"32\" height=\"24\" fps=\"4\" "
          "duration=\"1\"/><composition><shape id=\"a\" shape=\"rect\" "
          "width=\"8\" height=\"8\" strokeWidth=\"1000000001\"/></composition></scene>";
    CHECK_INT(t, st_load(t, "shape-wide.xml", xml, &scene, NULL), SR_OK);
    if (scene.root) sr_scene_free(&scene);
    /* Explicit schema defaults are harmless on any kind. */
    xml = "<scene version=\"1.1\"><project width=\"32\" height=\"24\" fps=\"4\" "
          "duration=\"1\"/><composition><shape id=\"a\" shape=\"ellipse\" "
          "width=\"8\" height=\"8\" radius=\"0\" points=\"5\" "
          "innerRoundness=\"0\"/></composition></scene>";
    CHECK_INT(t, st_load(t, "shape-defaults.xml", xml, &scene, NULL), SR_OK);
    if (scene.root) sr_scene_free(&scene);
}

/* Seeded valid, mutated and truncated shape/paint documents: every load
 * either succeeds or fails cleanly, and repeats deterministically. */
static void seeded_document_mutations(sr_test_ctx *t) {
    static const char *const kinds[] = {"rect", "rounded-rect", "ellipse",
        "polygon", "star", "line", "path"};
    static const char *const common[] = {
        " strokeCap=\"round\"", " strokeJoin=\"bevel\"", " miterLimit=\"3\"",
        " dash=\"2 1 3\"", " dashOffset=\"-4\"", " paintOrder=\"stroke-fill\"",
        " trimStart=\"0.2\"", " trimEnd=\"0.7\"", " trimOffset=\"1.5\"",
        " trimMode=\"sequential\"", " fillRule=\"evenodd\"", " fill=\"url(#g)\"",
        " stroke=\"url(#r)\"", " strokeWidth=\"2\"", " strokePosition=\"outside\""};
    static const char *const extras[7][4] = {
        {" cornerRadii=\"1 2\"", " radius=\"3\"", NULL, NULL},
        {" cornerRadii=\"4\"", " radius=\"1\"", NULL, NULL},
        {NULL, NULL, NULL, NULL},
        {" points=\"7\"", " outerRadius=\"5\"", " outerRoundness=\"1\"", NULL},
        {" points=\"3\"", " innerRadius=\"2\"", " outerRadius=\"5\"",
         " innerRoundness=\"0.5\""},
        {NULL, NULL, NULL, NULL},
        {NULL, NULL, NULL, NULL}};
    const char mutations[] = "<>\"=/ #0.-9eu(Z";
    const uint64_t seeds[] = {1, 2, 3, UINT64_C(0x9E3779B97F4A7C15)};
    size_t accepted = 0, rejected = 0;
    for (size_t s = 0; s < sizeof(seeds) / sizeof(seeds[0]); ++s) {
        for (unsigned round = 0; round < 24; ++round) {
            uint64_t v = sr_random_mix64(seeds[s] * 131 + round);
            char body[2048];
            int n = snprintf(body, sizeof(body), "<paints><linearGradient id=\"g\" "
                "spread=\"repeat\"><stop offset=\"0\" color=\"#FF0000\"/><stop "
                "offset=\"1\" color=\"#0000FF\"/></linearGradient><radialGradient "
                "id=\"r\" fx=\"0.2\"><stop offset=\"0.5\" color=\"#00FF00\"/>"
                "</radialGradient></paints><composition><shape id=\"s\" shape=\"%s\" "
                "width=\"12\" height=\"10\"%s", kinds[v % 7],
                v % 7 == 6 ? " path=\"M1 1 L9 2 Q5 9 2 8 Z\"" : "");
            size_t kind = v % 7;
            uint64_t mask = sr_random_mix64(v + 1);
            for (size_t k = 0; k < sizeof(common) / sizeof(common[0]); ++k)
                if ((mask >> k) & 1 && !(kind == 5 && k == 14))
                    n += snprintf(body + n, sizeof(body) - (size_t)n, "%s", common[k]);
            for (size_t k = 0; k < 4; ++k)
                if (extras[kind][k] && (mask >> (20 + k)) & 1)
                    n += snprintf(body + n, sizeof(body) - (size_t)n, "%s", extras[kind][k]);
            n += snprintf(body + n, sizeof(body) - (size_t)n, "/></composition>");
            unsigned mode = round % 3;       /* valid, mutated, truncated */
            if (mode == 1) body[(v >> 20) % (size_t)n] = mutations[(v >> 40) % (sizeof(mutations) - 1)];
            if (mode == 2) body[(v >> 20) % (size_t)n] = '\0';
            char xml[4096];
            snprintf(xml, sizeof(xml), "%s%s</scene>", header, body);
            SrScene first, second;
            SrStatus a = st_load(t, "shape-fuzz.xml", xml, &first, NULL);
            SrStatus b = st_load(t, "shape-fuzz.xml", xml, &second, NULL);
            CHECK_INT(t, a, b);
            CHECK(t, a == SR_OK || a == SR_ERR_XML);
            if (a == SR_OK) {
                ++accepted;
                SrFrame frame;
                if (sr_frame_init(&frame, 32, 24) == SR_OK) {
                    SrDiagnostics diag;
                    FILE *sink;
                    st_diag(&diag, &sink);
                    SrStatus rendered = sr_composite_scene(&first, 0.5, &frame, &diag);
                    CHECK(t, rendered == SR_OK || rendered == SR_ERR_RENDER);
                    if (sink) fclose(sink);
                    sr_frame_free(&frame);
                }
                sr_scene_free(&first);
            } else {
                ++rejected;
            }
            if (b == SR_OK) sr_scene_free(&second);
        }
    }
    if (!(accepted > 10 && rejected > 10))
        SR_FAIL(t, "accepted %zu, rejected %zu", accepted, rejected);
}

/* ---- rendering --------------------------------------------------------- */

static SrStatus render(SrScene *scene, unsigned threads, double time, SrFrame *frame) {
    SrDiagnostics diag;
    FILE *sink;
    st_diag(&diag, &sink);
    SrCompositor compositor;
    sr_compositor_init(&compositor, threads);
    memset(frame->px, 0, (size_t)frame->width * frame->height * 4 * sizeof(float));
    SrStatus status = sr_compositor_render(&compositor, scene, time, frame, &diag);
    sr_compositor_free(&compositor);
    if (sink) fclose(sink);
    return status;
}

static void legacy_identity_and_exact_area(sr_test_ctx *t) {
    /* An integer-aligned rect: the analytic and exact-area renderers agree
     * bit for bit, so adding an explicit B1-4 default changes nothing. */
    const char *legacy = "<scene version=\"1.1\"><project width=\"32\" height=\"24\" "
        "fps=\"4\" duration=\"1\"/><composition><shape id=\"a\" shape=\"rect\" x=\"3\" "
        "y=\"4\" width=\"10\" height=\"6\" fill=\"#FF8040\"/></composition></scene>";
    const char *extended = "<scene version=\"1.1\"><project width=\"32\" height=\"24\" "
        "fps=\"4\" duration=\"1\"/><composition><shape id=\"a\" shape=\"rect\" x=\"3\" "
        "y=\"4\" width=\"10\" height=\"6\" fill=\"#FF8040\" paintOrder=\"fill-stroke\"/>"
        "</composition></scene>";
    SrScene a, b;
    CHECK_INT(t, st_load(t, "legacy.xml", legacy, &a, NULL), SR_OK);
    CHECK_INT(t, st_load(t, "extended.xml", extended, &b, NULL), SR_OK);
    CHECK(t, !child(&a, "a")->shape_style.extended && child(&b, "a")->shape_style.extended);
    CHECK(t, !a.compositing_required && b.compositing_required);
    SrFrame fa, fb;
    CHECK_INT(t, sr_frame_init(&fa, 32, 24), SR_OK);
    CHECK_INT(t, sr_frame_init(&fb, 32, 24), SR_OK);
    CHECK_INT(t, render(&a, 1, 0.0, &fa), SR_OK);
    CHECK_INT(t, render(&b, 1, 0.0, &fb), SR_OK);
    CHECK(t, st_frames_equal(&fa, &fb));
    sr_scene_free(&a);
    sr_scene_free(&b);
    /* Fractional placement: the exact-area fill integrates to the area. */
    const char *fractional = "<scene version=\"1.1\"><project width=\"32\" height=\"24\" "
        "fps=\"4\" duration=\"1\"/><composition><shape id=\"a\" shape=\"polygon\" "
        "points=\"4\" x=\"3.3\" y=\"2.7\" width=\"16\" height=\"16\" fill=\"#FFFFFF\"/>"
        "</composition></scene>";
    CHECK_INT(t, st_load(t, "fractional.xml", fractional, &a, NULL), SR_OK);
    CHECK_INT(t, render(&a, 1, 0.0, &fa), SR_OK);
    double alpha = 0.0;
    for (size_t i = 0; i < 32 * 24; ++i) alpha += fa.px[i * 4 + 3];
    CHECK_NEAR(t, alpha, 2.0 * 8.0 * 8.0, 1e-3);   /* diamond: 2 R^2 */
    sr_scene_free(&a);
    sr_frame_free(&fa);
    sr_frame_free(&fb);
}

static void threads_order_and_immutability(sr_test_ctx *t) {
    SrScene scene;
    SrDiagnostics diag;
    FILE *sink;
    st_diag(&diag, &sink);
    SrStatus status = sr_scene_load_xml(sr_test_data_path("tests/data-shapes.xml"),
                                        &scene, &diag);
    if (sink) fclose(sink);
    CHECK_INT(t, status, SR_OK);
    if (status != SR_OK) return;
    SrNode *draw = child(&scene, "draw");
    SrShapeStyle saved;
    memcpy(&saved, &draw->shape_style, sizeof(saved));
    SrPaint saved_paint;
    memcpy(&saved_paint, paint(&scene, "glow"), sizeof(saved_paint));
    SrFrame one, four, warm;
    CHECK_INT(t, sr_frame_init(&one, 160, 96), SR_OK);
    CHECK_INT(t, sr_frame_init(&four, 160, 96), SR_OK);
    CHECK_INT(t, sr_frame_init(&warm, 160, 96), SR_OK);
    const double times[] = {1.25, 0.0, 1.9, 0.6};
    for (size_t i = 0; i < 4; ++i) {
        CHECK_INT(t, render(&scene, 1, times[i], &one), SR_OK);
        CHECK_INT(t, render(&scene, 4, times[i], &four), SR_OK);
        CHECK(t, st_frames_equal(&one, &four));
    }
    /* A warm compositor rendering shuffled times matches fresh renders. */
    SrCompositor compositor;
    sr_compositor_init(&compositor, 3);
    st_diag(&diag, &sink);
    for (size_t i = 0; i < 4; ++i) {
        memset(warm.px, 0, 160 * 96 * 4 * sizeof(float));
        CHECK_INT(t, sr_compositor_render(&compositor, &scene, times[3 - i], &warm,
                                          &diag), SR_OK);
        CHECK_INT(t, render(&scene, 1, times[3 - i], &one), SR_OK);
        CHECK(t, st_frames_equal(&one, &warm));
    }
    if (sink) fclose(sink);
    sr_compositor_free(&compositor);
    CHECK(t, !memcmp(&saved, &draw->shape_style, sizeof(saved)));
    CHECK(t, !memcmp(&saved_paint, paint(&scene, "glow"), sizeof(saved_paint)));
    /* The animated trim draws more of the outline later. */
    CHECK_INT(t, render(&scene, 1, 0.0, &one), SR_OK);
    CHECK_INT(t, render(&scene, 1, 2.0, &four), SR_OK);
    CHECK(t, !st_frames_equal(&one, &four));
    sr_frame_free(&one);
    sr_frame_free(&four);
    sr_frame_free(&warm);
    sr_scene_free(&scene);
}

static void deformation_and_cards(sr_test_ctx *t) {
    const char *xml = "<scene version=\"1.1\"><project width=\"64\" height=\"48\" "
        "fps=\"4\" duration=\"1\"/><composition><camera id=\"c\" z=\"-200\" "
        "zoom=\"200\"/><shape id=\"w\" shape=\"star\" x=\"4\" y=\"4\" width=\"24\" "
        "height=\"24\" fill=\"#FFFFFF\" stroke=\"#FF0000\" strokeWidth=\"2\">"
        "<deform><modifier type=\"wave\" amount=\"3\" frequency=\"1\"/></deform>"
        "</shape><shape id=\"card\" shape=\"polygon\" x=\"36\" y=\"8\" width=\"20\" "
        "height=\"20\" threeD=\"true\" rotationY=\"30\" fill=\"#00FF00\" "
        "strokeWidth=\"2\" stroke=\"#0000FF\" strokeJoin=\"round\"/></composition></scene>";
    SrScene scene;
    char *message = NULL;
    SrStatus status = st_load(t, "deform-card.xml", xml, &scene, &message);
    CHECK_INT(t, status, SR_OK);
    if (status != SR_OK) {
        fprintf(stderr, "%s", message ? message : "");
        free(message);
        return;
    }
    free(message);
    SrFrame one, four;
    CHECK_INT(t, sr_frame_init(&one, 64, 48), SR_OK);
    CHECK_INT(t, sr_frame_init(&four, 64, 48), SR_OK);
    SrDiagnostics diag;
    FILE *sink;
    st_diag(&diag, &sink);
    SrCompositor a, b;
    sr_compositor_init(&a, 1);
    sr_compositor_init(&b, 4);
    memset(one.px, 0, 64 * 48 * 16);
    memset(four.px, 0, 64 * 48 * 16);
    CHECK_INT(t, sr_compositor_render_scene(&a, &scene, 0.25, &one, &diag), SR_OK);
    CHECK_INT(t, sr_compositor_render_scene(&b, &scene, 0.25, &four, &diag), SR_OK);
    CHECK(t, st_frames_equal(&one, &four));
    double left = 0.0, right = 0.0;
    for (uint32_t y = 0; y < 48; ++y)
        for (uint32_t x = 0; x < 64; ++x) {
            if (x < 32) left += st_px(&one, x, y)[3];
            else right += st_px(&one, x, y)[3];
        }
    CHECK(t, left > 100.0 && right > 100.0);
    sr_compositor_free(&a);
    sr_compositor_free(&b);
    if (sink) fclose(sink);
    sr_frame_free(&one);
    sr_frame_free(&four);
    sr_scene_free(&scene);
}

static void runtime_limits(sr_test_ctx *t) {
    /* A dash pattern producing too many pieces fails the render with the
     * owning element instead of truncating. */
    const char *xml = "<scene version=\"1.1\"><project width=\"32\" height=\"24\" "
        "fps=\"4\" duration=\"1\"/><composition><shape id=\"a\" shape=\"rect\" "
        "width=\"4000\" height=\"4000\" stroke=\"#FFFFFF\" strokeWidth=\"1\" "
        "dash=\"0.001 0.001\"/></composition></scene>";
    SrScene scene;
    CHECK_INT(t, st_load(t, "dash-limit.xml", xml, &scene, NULL), SR_OK);
    if (!scene.root) return;
    SrFrame frame;
    CHECK_INT(t, sr_frame_init(&frame, 32, 24), SR_OK);
    FILE *sink = tmpfile();
    SrDiagnostics diag;
    sr_diag_init(&diag, "limit", sink);
    SrCompositor compositor;
    sr_compositor_init(&compositor, 1);
    CHECK_INT(t, sr_compositor_render(&compositor, &scene, 0.0, &frame, &diag), SR_ERR_RENDER);
    sr_compositor_free(&compositor);
    char text[512] = {0};
    if (sink) {
        rewind(sink);
        size_t got = fread(text, 1, sizeof(text) - 1, sink);
        text[got] = '\0';
        fclose(sink);
    }
    CHECK_CONTAINS(t, text, "1048576");
    CHECK_CONTAINS(t, text, "<shape>");
    sr_scene_free(&scene);
    /* An overflowing transform fails instead of disappearing. */
    xml = "<scene version=\"1.1\"><project width=\"32\" height=\"24\" "
        "fps=\"4\" duration=\"1\"/><composition><shape id=\"a\" shape=\"star\" "
        "width=\"8\" height=\"8\" scaleX=\"1e200\"/></composition></scene>";
    CHECK_INT(t, st_load(t, "scale-limit.xml", xml, &scene, NULL), SR_OK);
    if (scene.root) {
        sink = tmpfile();
        sr_diag_init(&diag, "scale", sink);
        sr_compositor_init(&compositor, 1);
        CHECK_INT(t, sr_compositor_render(&compositor, &scene, 0.0, &frame, &diag),
                  SR_ERR_RENDER);
        CHECK(t, diag.errors > 0);
        sr_compositor_free(&compositor);
        if (sink) fclose(sink);
        sr_scene_free(&scene);
    }
    /* The same check applies on the deformation (local grid) path. */
    xml = "<scene version=\"1.1\"><project width=\"32\" height=\"24\" "
        "fps=\"4\" duration=\"1\"/><composition><shape id=\"a\" shape=\"star\" "
        "width=\"8\" height=\"8\" scaleX=\"1e200\"><deform><modifier "
        "type=\"wave\" amount=\"0\" frequency=\"1\"/></deform></shape>"
        "</composition></scene>";
    CHECK_INT(t, st_load(t, "scale-deform.xml", xml, &scene, NULL), SR_OK);
    if (scene.root) {
        sink = tmpfile();
        sr_diag_init(&diag, "scale", sink);
        sr_compositor_init(&compositor, 1);
        CHECK_INT(t, sr_compositor_render(&compositor, &scene, 0.0, &frame, &diag),
                  SR_ERR_RENDER);
        sr_compositor_free(&compositor);
        if (sink) fclose(sink);
        sr_scene_free(&scene);
    }
    /* Direct-C preparation rejects malformed extended storage. */
    SrScene direct;
    sr_scene_init(&direct);
    SrNode *node = sr_node_create(&direct, SR_NODE_SHAPE);
    node->id = strdup("n");
    node->shape = SR_SHAPE_STAR;
    node->shape_width = node->shape_height = 8;
    node->shape_style.extended = true;
    node->shape_style.points = 1;
    CHECK_INT(t, sr_node_add_child(direct.root, node), SR_OK);
    sink = tmpfile();
    sr_diag_init(&diag, "direct", sink);
    CHECK_INT(t, sr_scene_prepare_compositing(&direct, &diag), SR_ERR_RENDER);
    node->shape_style.points = 5;
    CHECK_INT(t, sr_scene_prepare_compositing(&direct, &diag), SR_OK);
    /* A path contour without storage is rejected before geometry reads it. */
    SrPathContour empty = {NULL, 2, 2, false};
    SrPreparedPath bad = {&empty, 1, 1};
    node->shape = SR_SHAPE_PATH;
    node->shape_style.path = &bad;
    CHECK_INT(t, sr_scene_prepare_compositing(&direct, &diag), SR_ERR_RENDER);
    node->shape_style.path = NULL;
    if (sink) fclose(sink);
    sr_frame_free(&frame);
    sr_scene_free(&direct);
}

const sr_test_case sr_tests_xml_shapes[] = {
    {"fixture_constructs", fixture_constructs},
    {"rejections_name_attributes", rejections_name_attributes},
    {"key_markers_on_paint_hosts", key_markers_on_paint_hosts},
    {"version_gates", version_gates},
    {"seeded_document_mutations", seeded_document_mutations},
    {"legacy_identity_and_exact_area", legacy_identity_and_exact_area},
    {"threads_order_and_immutability", threads_order_and_immutability},
    {"deformation_and_cards", deformation_and_cards},
    {"runtime_limits", runtime_limits},
    {NULL, NULL},
};
