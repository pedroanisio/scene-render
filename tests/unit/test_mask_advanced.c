/* SPDX-License-Identifier: Apache-2.0 */
/* B1-3 advanced masks: modes, opacity, feather, expansion, path, polygon
 * and star coverage, against closed-form and independent references. */
#include "b13_fixture.h"
#include "fixture.h"
#include "compositor_coverage_internal.h"

#define ARRAY_COUNT(a) (sizeof(a) / sizeof((a)[0]))

/* A full-frame opaque white shape: frame alpha equals mask coverage, as
 * local pixel centres coincide with the stored coverage samples. */
static SrStatus coverage_render(sr_test_ctx *t, const char *version,
                                const char *masks, double time, SrFrame *frame,
                                char **message) {
    char xml[4096];
    snprintf(xml, sizeof(xml), "<scene version=\"%s\">\n"
             "<project width=\"64\" height=\"64\" fps=\"2\" duration=\"2\" "
             "background=\"#00000000\"/>\n<composition>"
             "<shape id=\"s\" shape=\"rect\" width=\"64\" height=\"64\" "
             "fill=\"#FFFFFF\">\n%s\n</shape></composition></scene>",
             version, masks);
    return b13_render_xml(t, xml, time, 1, frame, message);
}

static double alpha_at(const SrFrame *frame, uint32_t x, uint32_t y) {
    return st_px(frame, x, y)[3];
}

static double combine(int mode, double a, double b) {
    switch (mode) {
    case 1: return a + b - a * b;
    case 2: return a * (1 - b);
    case 3: return fmax(a, b);
    case 4: return fmin(a, b);
    case 5: return fabs(a - b);
    case 6: return a;
    default: return a * b;
    }
}

static void modes_in_order(sr_test_ctx *t) {
    static const char *const modes[] = {"intersect", "add", "subtract",
        "lighten", "darken", "difference", "none"};
    static const uint32_t points[4][2] = {{16, 16}, {48, 16}, {16, 48}, {48, 48}};
    for (int first = 0; first < 7; ++first) {
        for (int second = 0; second < 7; ++second) {
            char masks[512];
            snprintf(masks, sizeof(masks),
                     "<mask type=\"rect\" width=\"32\" height=\"64\" mode=\"%s\"/>"
                     "<mask type=\"rect\" width=\"64\" height=\"32\" mode=\"%s\"/>",
                     modes[first], modes[second]);
            SrFrame frame;
            SrStatus status = coverage_render(t, "1.1", masks, 0, &frame, NULL);
            CHECK_INT(t, status, SR_OK);
            if (status != SR_OK) continue;
            for (size_t p = 0; p < 4; ++p) {
                double a = points[p][0] < 32, b = points[p][1] < 32;
                /* The first participating mode chooses the accumulator. */
                double expected;
                if (first == 6 && second == 6) expected = 1;
                else if (first == 6) expected = combine(second,
                    second == 1 || second == 3 || second == 5 ? 0 : 1, b);
                else {
                    double start = first == 1 || first == 3 || first == 5 ? 0 : 1;
                    expected = combine(second, combine(first, start, a), b);
                }
                CHECK_NEAR(t, alpha_at(&frame, points[p][0], points[p][1]),
                           expected, 1e-6);
            }
            sr_frame_free(&frame);
        }
    }
}

static void opacity_invert_exterior(sr_test_ctx *t) {
    SrFrame frame;
    CHECK_INT(t, coverage_render(t, "1.1",
        "<mask type=\"rect\" x=\"8\" y=\"8\" width=\"16\" height=\"16\" opacity=\".5\"/>",
        0, &frame, NULL), SR_OK);
    CHECK_NEAR(t, alpha_at(&frame, 16, 16), .5, 1e-6);
    CHECK_NEAR(t, alpha_at(&frame, 40, 40), 0, 0);
    sr_frame_free(&frame);
    CHECK_INT(t, coverage_render(t, "1.1",
        "<mask type=\"rect\" x=\"8\" y=\"8\" width=\"16\" height=\"16\" opacity=\".5\" "
        "invert=\"true\"/>", 0, &frame, NULL), SR_OK);
    CHECK_NEAR(t, alpha_at(&frame, 16, 16), 0, 1e-6);
    CHECK_NEAR(t, alpha_at(&frame, 63, 63), .5, 1e-6);
    CHECK_NEAR(t, alpha_at(&frame, 0, 63), .5, 1e-6);
    sr_frame_free(&frame);
    /* Later add restores coverage after an empty intersect. */
    CHECK_INT(t, coverage_render(t, "1.1",
        "<mask type=\"rect\" x=\"100\" y=\"100\" width=\"4\" height=\"4\"/>"
        "<mask type=\"ellipse\" x=\"0\" y=\"0\" width=\"32\" height=\"32\" mode=\"add\"/>",
        0, &frame, NULL), SR_OK);
    CHECK_NEAR(t, alpha_at(&frame, 16, 16), 1, 1e-6);
    CHECK_NEAR(t, alpha_at(&frame, 50, 50), 0, 0);
    sr_frame_free(&frame);
}

/* Independent 1D reference: zero-exterior box passes on a long line. */
static void box3(double *line, size_t n, int q) {
    double *tmp = calloc(n, sizeof(*tmp));
    for (int pass = 0; pass < 3; ++pass) {
        for (size_t i = 0; i < n; ++i) {
            double sum = 0;
            for (int k = -q; k <= q; ++k) {
                long j = (long)i + k;
                if (j >= 0 && j < (long)n) sum += line[j];
            }
            tmp[i] = sum / (2 * q + 1);
        }
        memcpy(line, tmp, n * sizeof(*line));
    }
    free(tmp);
}

static void feather_reference(sr_test_ctx *t) {
    static const double feathers[] = {6, 5, 1};
    for (size_t f = 0; f < ARRAY_COUNT(feathers); ++f) {
        char masks[256];
        snprintf(masks, sizeof(masks),
                 "<mask type=\"rect\" x=\"0\" y=\"-100\" width=\"32\" height=\"300\" "
                 "feather=\"%g\"/>", feathers[f]);
        SrFrame frame;
        CHECK_INT(t, coverage_render(t, "1.1", masks, 0, &frame, NULL), SR_OK);
        /* Line sample i is local x = i - 100 + 0.5. */
        enum { N = 300 };
        double low[N], high[N];
        for (size_t i = 0; i < N; ++i)
            low[i] = high[i] = ((double)i - 100 + .5 >= 0 && (double)i - 100 + .5 < 32);
        double sigma = feathers[f] / 2;
        int lo = (int)floor(sigma);
        double frac = sigma - lo;
        if (lo > 0) box3(low, N, lo);
        if (frac > 0) box3(high, N, lo + 1);
        for (uint32_t x = 0; x < 64; ++x) {
            double expected = frac > 0 ? low[x + 100] + (high[x + 100] - low[x + 100]) * frac
                                       : low[x + 100];
            CHECK_NEAR(t, alpha_at(&frame, x, 32), expected, 2e-6);
        }
        sr_frame_free(&frame);
    }
}

static void expansion_reference(sr_test_ctx *t) {
    static const struct { double amount; uint32_t x; double expected; } cases[] = {
        {3, 50, 1}, {3, 51, 0}, {-3, 44, 1}, {-3, 45, 0},
        {2.5, 50, .5}, {2.5, 49, 1}, {-2.5, 45, .5}, {0, 47, 1}, {0, 48, 0}};
    for (size_t i = 0; i < ARRAY_COUNT(cases); ++i) {
        char masks[256];
        snprintf(masks, sizeof(masks),
                 "<mask type=\"rect\" x=\"16\" y=\"-100\" width=\"32\" height=\"300\" "
                 "expansion=\"%g\"/>", cases[i].amount);
        SrFrame frame;
        CHECK_INT(t, coverage_render(t, "1.1", masks, 0, &frame, NULL), SR_OK);
        CHECK_NEAR(t, alpha_at(&frame, cases[i].x, 32), cases[i].expected, 1e-6);
        sr_frame_free(&frame);
    }
    /* A diagonal neighbour at distance sqrt(8) > 2 is outside a radius-2
     * disk; (2, 0) is inside. */
    SrFrame frame;
    CHECK_INT(t, coverage_render(t, "1.1",
        "<mask type=\"rect\" x=\"32\" y=\"32\" width=\"1\" height=\"1\" expansion=\"2\"/>",
        0, &frame, NULL), SR_OK);
    CHECK_NEAR(t, alpha_at(&frame, 34, 32), 1, 1e-6);
    CHECK_NEAR(t, alpha_at(&frame, 34, 34), 0, 1e-6);
    CHECK_NEAR(t, alpha_at(&frame, 33, 33), 1, 1e-6);
    sr_frame_free(&frame);
}

static void polygon_star_geometry(sr_test_ctx *t) {
    SrFrame frame;
    CHECK_INT(t, coverage_render(t, "1.1",
        "<mask type=\"polygon\" x=\"32\" y=\"32\" radius=\"20\" points=\"4\"/>",
        0, &frame, NULL), SR_OK);
    CHECK_NEAR(t, alpha_at(&frame, 32, 32), 1, 1e-6);
    CHECK_NEAR(t, alpha_at(&frame, 38, 38), 1, 1e-6);
    CHECK_NEAR(t, alpha_at(&frame, 45, 45), 0, 1e-6);
    CHECK_NEAR(t, alpha_at(&frame, 32, 13), 1, 1e-6);  /* first vertex up */
    sr_frame_free(&frame);
    CHECK_INT(t, coverage_render(t, "1.1",
        "<mask type=\"star\" x=\"32\" y=\"32\" radius=\"25\" innerRadius=\"12\"/>",
        0, &frame, NULL), SR_OK);
    CHECK_NEAR(t, alpha_at(&frame, 32, 32), 1, 1e-6);
    CHECK_NEAR(t, alpha_at(&frame, 32, 12), 1, 1e-6);
    CHECK_NEAR(t, alpha_at(&frame, 43, 15), 0, 1e-6);
    sr_frame_free(&frame);
    /* Automatic inner radius follows half the evaluated outer radius. */
    SrFrame animated, fixed;
    CHECK_INT(t, coverage_render(t, "1.1",
        "<mask type=\"star\" x=\"32\" y=\"32\" radius=\"10\" points=\"6\">"
        "<animate property=\"radius\"><key time=\"0\" value=\"10\"/>"
        "<key time=\"2\" value=\"30\"/></animate></mask>", 1, &animated, NULL), SR_OK);
    CHECK_INT(t, coverage_render(t, "1.1",
        "<mask type=\"star\" x=\"32\" y=\"32\" radius=\"20\" innerRadius=\"10\" "
        "points=\"6\"/>", 0, &fixed, NULL), SR_OK);
    CHECK(t, st_frames_equal(&animated, &fixed));
    sr_frame_free(&animated);
    /* An authored innerRadius track overrides the automatic relation. */
    CHECK_INT(t, coverage_render(t, "1.1",
        "<mask type=\"star\" x=\"32\" y=\"32\" radius=\"20\" points=\"6\">"
        "<animate property=\"innerRadius\"><key time=\"0\" value=\"2\"/>"
        "<key time=\"2\" value=\"18\"/></animate></mask>", 1, &animated, NULL), SR_OK);
    CHECK(t, st_frames_equal(&animated, &fixed));
    sr_frame_free(&animated);
    sr_frame_free(&fixed);
}

static void path_masks(sr_test_ctx *t) {
    static const char *const rings[] = {"evenodd", "nonzero"};
    for (size_t r = 0; r < 2; ++r) {
        char masks[512];
        snprintf(masks, sizeof(masks),
                 "<mask type=\"path\" fillRule=\"%s\" path=\"M 8 8 L 56 8 L 56 56 L 8 56 Z "
                 "M 20 20 L 44 20 L 44 44 L 20 44 Z\"/>", rings[r]);
        SrFrame frame;
        CHECK_INT(t, coverage_render(t, "1.1", masks, 0, &frame, NULL), SR_OK);
        CHECK_NEAR(t, alpha_at(&frame, 12, 32), 1, 1e-6);
        CHECK_NEAR(t, alpha_at(&frame, 32, 32), r ? 1 : 0, 1e-6);
        CHECK_NEAR(t, alpha_at(&frame, 2, 2), 0, 0);
        sr_frame_free(&frame);
    }
    SrFrame frame;
    CHECK_INT(t, coverage_render(t, "1.1",
        "<mask type=\"path\" x=\"4\" y=\"0\" width=\"20\" height=\"64\" "
        "path=\"M 0 0 L 40 0 L 40 64 L 0 64 Z\"/>", 0, &frame, NULL), SR_OK);
    CHECK_NEAR(t, alpha_at(&frame, 3, 10), 0, 0);
    CHECK_NEAR(t, alpha_at(&frame, 10, 10), 1, 1e-6);
    CHECK_NEAR(t, alpha_at(&frame, 30, 10), 0, 1e-6);  /* outside the box */
    sr_frame_free(&frame);
    /* Half-covered edge pixel: exact-area coverage. */
    CHECK_INT(t, coverage_render(t, "1.1",
        "<mask type=\"path\" path=\"M 0 0 L 10.5 0 L 10.5 64 L 0 64 Z\"/>",
        0, &frame, NULL), SR_OK);
    CHECK_NEAR(t, alpha_at(&frame, 10, 10), .5, 1e-6);
    sr_frame_free(&frame);
}

static void explicit_defaults_stay_legacy(sr_test_ctx *t) {
    SrFrame plain, explicit_frame;
    CHECK_INT(t, coverage_render(t, "1.0",
        "<mask type=\"ellipse\" x=\"3\" y=\"5\" width=\"40\" height=\"30\"/>",
        0, &plain, NULL), SR_OK);
    CHECK_INT(t, coverage_render(t, "1.0",
        "<mask type=\"ellipse\" x=\"3\" y=\"5\" width=\"40\" height=\"30\" mode=\"intersect\" "
        "opacity=\"1\" feather=\"0\" expansion=\"0\"/>", 0, &explicit_frame, NULL), SR_OK);
    CHECK(t, st_frames_equal(&plain, &explicit_frame));
    sr_frame_free(&plain);
    sr_frame_free(&explicit_frame);
    SrMask mask = {.type = SR_MASK_RECT, .width = {.base = 4}, .height = {.base = 4}};
    CHECK(t, !sr_mask_advanced(&mask));
    mask.opacity_set = true;
    mask.opacity.base = 1;
    CHECK(t, !sr_mask_advanced(&mask));
    mask.opacity.base = .5;
    CHECK(t, sr_mask_advanced(&mask));
    mask.opacity.base = 1;
    mask.mode = SR_MASK_MODE_NONE;
    CHECK(t, sr_mask_advanced(&mask));
}

static void loader_diagnostics(sr_test_ctx *t) {
    static const struct { const char *version, *masks, *needle; } cases[] = {
        {"1.1", "<mask type=\"rect\" width=\"4\" height=\"4\" path=\"M 0 0 L 1 1\"/>",
         "path is valid only"},
        {"1.1", "<mask type=\"polygon\" radius=\"4\" innerRadius=\"1\"/>",
         "innerRadius is valid only for star masks"},
        {"1.1", "<mask type=\"path\" width=\"4\" path=\"M 0 0 L 4 0 L 4 4 Z\"/>",
         "needs both width and height"},
        {"1.1", "<mask type=\"star\" radius=\"4\" points=\"2\"/>", "[3,4096]"},
        {"1.1", "<mask type=\"rect\" width=\"4\" height=\"4\" feather=\"5000\"/>",
         "feather in [0,4096]"},
        {"1.1", "<mask type=\"rect\" width=\"4\" height=\"4\" expansion=\"-5000\"/>",
         "expansion within"},
        {"1.1", "<mask type=\"path\" path=\"M 0 0 L\"/>", "malformed path data at byte"},
        {"1.1", "<mask type=\"star\" radius=\"4\" innerRadius=\"5\"/>",
         "innerRadius must be in [0, radius]"},
        {"1.1", "<mask type=\"polygon\" radius=\"0\"/>", "positive radius"},
        {"1.0", "<mask type=\"path\" path=\"M 0 0 L 4 0 L 4 4 Z\"/>",
         "requires version=\"1.1\""},
        {"1.0", "<mask type=\"rect\" width=\"4\" height=\"4\"><animate property=\"feather\">"
         "<key time=\"0\" value=\"1\"/></animate></mask>", "requires version=\"1.1\""},
        {"1.1", "<mask type=\"rect\" width=\"4\" height=\"4\"><animate property=\"opacity\">"
         "<key time=\"0\" value=\"2\"/></animate></mask>", "mask opacity keys"},
    };
    for (size_t i = 0; i < ARRAY_COUNT(cases); ++i) {
        SrFrame frame;
        char *message = NULL;
        SrStatus status = coverage_render(t, cases[i].version, cases[i].masks, 0,
                                          &frame, &message);
        CHECK(t, status != SR_OK);
        CHECK_CONTAINS(t, message, cases[i].needle);
        CHECK_CONTAINS(t, message, "mask");
        if (status == SR_OK) sr_frame_free(&frame);
        free(message);
    }
    /* Explicit new attributes are legal in 1.0 documents. */
    SrFrame frame;
    CHECK_INT(t, coverage_render(t, "1.0",
        "<mask type=\"rect\" width=\"32\" height=\"32\" mode=\"add\" feather=\"2\"/>",
        0, &frame, NULL), SR_OK);
    sr_frame_free(&frame);
}

static void runtime_ranges_and_limits(sr_test_ctx *t) {
    SrFrame frame;
    char *message = NULL;
    SrStatus status = coverage_render(t, "1.1",
        "<mask type=\"rect\" width=\"32\" height=\"32\"><animate property=\"feather\">"
        "<key time=\"0\" value=\"0\" interpolation=\"back-out\"/>"
        "<key time=\"2\" value=\"4096\"/></animate></mask>", 1, &frame, &message);
    CHECK_INT(t, status, SR_ERR_RENDER);
    CHECK_CONTAINS(t, message, "evaluated mask feather");
    CHECK_CONTAINS(t, message, "mask");
    free(message);
    message = NULL;
    char xml[2048];
    snprintf(xml, sizeof(xml), "<scene version=\"1.1\">\n"
             "<project width=\"64\" height=\"64\" fps=\"2\" duration=\"2\"/>\n"
             "<composition><shape id=\"s\" shape=\"rect\" width=\"640000\" height=\"10\" "
             "scaleX=\"0.0001\" fill=\"#FFFFFF\"><mask type=\"rect\" width=\"1000000\" "
             "height=\"10\" feather=\"1\"/></shape></composition></scene>");
    status = b13_render_xml(t, xml, 0, 1, &frame, &message);
    CHECK_INT(t, status, SR_ERR_RENDER);
    CHECK_CONTAINS(t, message, "dimension limit");
    free(message);
}

static void hosts_cards_and_threads(sr_test_ctx *t) {
    const char *xml = "<scene version=\"1.1\">\n"
        "<project width=\"96\" height=\"64\" fps=\"4\" duration=\"2\" seed=\"2\" "
        "background=\"#00000000\"/>\n"
        "<composition><camera id=\"cam\" z=\"-400\" fov=\"50\" active=\"true\"/>"
        "<group id=\"pass\"><mask type=\"star\" x=\"30\" y=\"30\" radius=\"26\" "
        "feather=\"3\"/><shape id=\"a\" shape=\"rect\" width=\"60\" height=\"60\" "
        "fill=\"#E08030\"/><particleEmitter id=\"p\" x=\"30\" y=\"30\" rate=\"30\" "
        "lifetime=\"2\" speed=\"25\" size=\"3\" seed=\"4\" color=\"#FFFFFF\">"
        "<mask type=\"polygon\" x=\"0\" y=\"0\" radius=\"18\" points=\"5\" expansion=\"1.5\"/>"
        "</particleEmitter></group>"
        "<group id=\"card\" x=\"70\" y=\"32\" anchorX=\"20\" anchorY=\"20\" threeD=\"true\" "
        "rotationY=\"40\"><mask type=\"path\" path=\"M 0 0 L 40 0 L 20 40 Z\" feather=\"2\" "
        "mode=\"add\"/><shape id=\"b\" shape=\"rect\" width=\"40\" height=\"40\" "
        "fill=\"#3080E0\"/></group></composition></scene>";
    SrScene scene;
    char *message = NULL;
    SrStatus status = b13_load(t, xml, &scene, &message);
    CHECK_INT(t, status, SR_OK);
    free(message);
    if (status != SR_OK) return;
    const SrNode *pass = scene.root->children[0];
    unsigned char saved[sizeof(SrMask)];
    memcpy(saved, pass->masks, sizeof(saved));
    SrFrame one, four, warm;
    CHECK_INT(t, b13_render(&scene, 1, 1, &one, NULL), SR_OK);
    CHECK_INT(t, b13_render(&scene, 1.5, 4, &warm, NULL), SR_OK);
    CHECK_INT(t, b13_render(&scene, 1, 4, &four, NULL), SR_OK);
    CHECK(t, st_frames_equal(&one, &four));
    CHECK(t, !memcmp(saved, pass->masks, sizeof(saved)));
    CHECK(t, st_px(&one, 30, 30)[3] > .99f);
    CHECK(t, st_px(&one, 1, 62)[3] < 1e-6f);
    sr_frame_free(&one);
    sr_frame_free(&four);
    sr_frame_free(&warm);
    sr_scene_free(&scene);
}

static void direct_c_preparation(sr_test_ctx *t) {
    SrScene scene;
    fx_scene(&scene, 32, 32);
    SrNode *node = fx_rect(&scene, NULL, 0, 0, 32, 32, (SrColor){1, 1, 1, 1}, 1);
    CHECK(t, node != NULL);
    if (!node) { sr_scene_free(&scene); return; }
    SrMask mask = {.type = SR_MASK_PATH, .fill_rule = SR_FILL_NONZERO,
                   .path = sr_strdup("M 0 0 L 16 0 L 16 32 L 0 32 Z"), .source_line = 7};
    CHECK_INT(t, sr_node_add_mask(node, mask), SR_OK);
    SrFrame frame = {0};
    CHECK_INT(t, sr_frame_init(&frame, 32, 32), SR_OK);
    FILE *sink;
    SrDiagnostics diag;
    st_diag(&diag, &sink);
    /* New features require explicit preparation. */
    CHECK_INT(t, sr_composite_scene(&scene, 0, &frame, &diag), SR_ERR_RENDER);
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, &diag), SR_OK);
    CHECK(t, node->masks[0].prepared != NULL);
    const float clear[4] = {0, 0, 0, 0};
    sr_frame_clear(&frame, clear, 1);
    CHECK_INT(t, sr_composite_scene(&scene, 0, &frame, &diag), SR_OK);
    CHECK_NEAR(t, st_px(&frame, 4, 4)[3], 1, 1e-6);
    CHECK_NEAR(t, st_px(&frame, 20, 4)[3], 0, 0);
    sr_scene_invalidate_compositing(&scene);
    node->masks[0].type = SR_MASK_POLYGON;
    free(node->masks[0].path);
    node->masks[0].path = NULL;
    node->masks[0].points = 2;
    node->masks[0].radius.base = 4;
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, &diag), SR_ERR_RENDER);
    char *text = b13_sink_text(sink);
    CHECK_CONTAINS(t, text, ":7:");
    CHECK_CONTAINS(t, text, "points must be in [3,4096]");
    free(text);
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

const sr_test_case sr_tests_mask_advanced[] = {
    {"modes_in_order", modes_in_order},
    {"opacity_invert_exterior", opacity_invert_exterior},
    {"feather_reference", feather_reference},
    {"expansion_reference", expansion_reference},
    {"polygon_star_geometry", polygon_star_geometry},
    {"path_masks", path_masks},
    {"explicit_defaults_stay_legacy", explicit_defaults_stay_legacy},
    {"loader_diagnostics", loader_diagnostics},
    {"runtime_ranges_and_limits", runtime_ranges_and_limits},
    {"hosts_cards_and_threads", hosts_cards_and_threads},
    {"direct_c_preparation", direct_c_preparation},
    {NULL, NULL}
};
