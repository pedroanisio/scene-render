/* SPDX-License-Identifier: Apache-2.0 */
/* Depth cards (2.5D): camera projection, parallax, depth scaling, tilt,
 * run sorting, per-sample occlusion with 3D objects, depth of field and
 * validation. Scenes are 200 x 100, non-linear sRGB, black background, so
 * blend values equal the XML colors. The usual camera sits at z = -1000
 * with zoom 1000: a card at depth 0 renders 1:1 and depth d scales by
 * 1000 / (1000 + d). */
#include "scene_render/card.h"
#include "scene_render/renderer.h"

#include "scene_text.h"

#define W 200
#define H 100

#define HEAD \
    "<scene version=\"1.0\"><project width=\"200\" height=\"100\" fps=\"10\" " \
    "duration=\"2\" linearLight=\"false\" background=\"#000000\"%s/>" \
    "<materials><material id=\"white\" baseColor=\"#000000\" emissive=\"#FFFFFF\"/>" \
    "<material id=\"red\" baseColor=\"#000000\" emissive=\"#FF0000\"/>" \
    "<material id=\"alpha-red\" baseColor=\"#00000080\" emissive=\"#FF0000\"/>" \
    "<material id=\"alpha-blue\" baseColor=\"#00000080\" emissive=\"#0000FF\"/>" \
    "<material id=\"blue\" baseColor=\"#000000\" emissive=\"#0000FF\"/></materials>" \
    "<composition>"
#define CAMERA \
    "<camera id=\"cam\" x=\"%g\" y=\"0\" z=\"%g\" zoom=\"%g\" yaw=\"%g\" " \
    "aperture=\"%g\" focusDistance=\"1000\"/>"

/* Camera distance and focal length: 1000 is the near-telephoto reference
 * (a 6 degree vertical field of view over 100 px); tests of perspective
 * foreshortening switch to 150 (37 degrees). */
static double lens = 1000.0;

/* Builds a scene: project extras, optional camera (x, yaw, aperture), body. */
static const char *scene(char *buffer, size_t size, const char *project,
                         bool camera, double cam_x, double yaw,
                         double aperture, const char *body)
{
    int n = snprintf(buffer, size, HEAD, project ? project : "");
    if (camera)
        n += snprintf(buffer + n, size - (size_t)n, CAMERA, cam_x, -lens, lens,
                      yaw, aperture);
    snprintf(buffer + n, size - (size_t)n, "%s</composition></scene>", body);
    return buffer;
}

static bool render_xml(sr_test_ctx *t, const char *name, const char *xml,
                       double time, unsigned threads, SrFrame *frame)
{
    SrScene scene;
    char *message = NULL;
    if (st_load(t, name, xml, &scene, &message) != SR_OK) {
        SR_FAIL(t, "load %s: %s", name, message ? message : "");
        free(message);
        return false;
    }
    free(message);
    const float black[4] = {0, 0, 0, 1};
    bool ok = sr_frame_init(frame, W, H) == SR_OK;
    CHECK(t, ok);
    if (ok) {
        sr_frame_clear(frame, black, 1);
        SrCompositor compositor;
        sr_compositor_init(&compositor, threads);
        FILE *sink;
        SrDiagnostics diag;
        st_diag(&diag, &sink);
        ok = sr_compositor_render_scene(&compositor, &scene, time, frame,
                                        &diag) == SR_OK;
        CHECK(t, ok);
        sr_compositor_free(&compositor);
        if (sink) fclose(sink);
    }
    sr_scene_free(&scene);
    return ok;
}

static bool is(const SrFrame *frame, int x, int y, float r, float g, float b)
{
    const float *p = st_px(frame, (uint32_t)x, (uint32_t)y);
    return fabsf(p[0] - r) < 1e-3f && fabsf(p[1] - g) < 1e-3f &&
           fabsf(p[2] - b) < 1e-3f;
}

#define RED(f, x, y) is(f, x, y, 1, 0, 0)
#define BLUE(f, x, y) is(f, x, y, 0, 0, 1)
#define WHITE(f, x, y) is(f, x, y, 1, 1, 1)
#define BLACK(f, x, y) is(f, x, y, 0, 0, 0)

/* Longest run of pixels with red above 0.5 on row y / column x. */
static int red_run_row(const SrFrame *f, int y, int *first)
{
    int best = 0, run = 0;
    for (int x = 0; x < W; ++x) {
        run = st_px(f, (uint32_t)x, (uint32_t)y)[0] > 0.5f ? run + 1 : 0;
        if (run > best) { best = run; if (first) *first = x - run + 1; }
    }
    return best;
}

static int red_run_col(const SrFrame *f, int x)
{
    int count = 0;
    for (int y = 0; y < H; ++y) count += st_px(f, (uint32_t)x, (uint32_t)y)[0] > 0.5f;
    return count;
}

/* A card at depth 0 under the reference camera equals the plain node. */
static void test_identity(sr_test_ctx *t)
{
    char a[4096], b[4096];
    SrFrame card = {0}, plain = {0};
    if (render_xml(t, "id-card.xml", scene(a, sizeof a, NULL, true, 0, 0, 0,
            "<shape id=\"r\" shape=\"rect\" x=\"40.5\" y=\"20.25\" width=\"60\" "
            "height=\"40\" fill=\"#FF8000\" depth=\"0\"/>"), 0, 1, &card) &&
        render_xml(t, "id-plain.xml", scene(b, sizeof b, NULL, false, 0, 0, 0,
            "<shape id=\"r\" shape=\"rect\" x=\"40.5\" y=\"20.25\" width=\"60\" "
            "height=\"40\" fill=\"#FF8000\"/>"), 0, 1, &plain)) {
        double worst = 0;
        for (size_t i = 0; i < (size_t)W * H * 4; ++i)
            worst = fmax(worst, fabs(card.px[i] - plain.px[i]));
        CHECK(t, worst < 1e-5);
    }
    sr_frame_free(&card); sr_frame_free(&plain);
}

/* Moving the camera 50 px shifts a depth-0 card by 50 px and a depth-1000
 * card by 25 px, which also renders at half size. */
static void test_parallax_and_scale(sr_test_ctx *t)
{
    const char *near_body =
        "<shape id=\"r\" shape=\"rect\" x=\"90\" y=\"40\" width=\"20\" height=\"20\" "
        "fill=\"#FF0000\" depth=\"0\"/>";
    const char *far_body =
        "<shape id=\"r\" shape=\"rect\" x=\"80\" y=\"30\" width=\"40\" height=\"40\" "
        "fill=\"#FF0000\" depth=\"1000\"/>";
    char xml[4096];
    SrFrame f = {0};
    int first = 0;
    if (render_xml(t, "px-near.xml", scene(xml, sizeof xml, NULL, true, 50, 0, 0,
                                           near_body), 0, 1, &f)) {
        CHECK_INT(t, red_run_row(&f, 50, &first), 20);
        CHECK_INT(t, first, 40);
    }
    sr_frame_free(&f);
    if (render_xml(t, "px-far0.xml", scene(xml, sizeof xml, NULL, true, 0, 0, 0,
                                           far_body), 0, 1, &f)) {
        CHECK_INT(t, red_run_row(&f, 50, &first), 20);    /* 40 px at half size */
        CHECK_INT(t, first, 90);
        CHECK_INT(t, red_run_col(&f, 100), 20);
    }
    sr_frame_free(&f);
    if (render_xml(t, "px-far.xml", scene(xml, sizeof xml, NULL, true, 50, 0, 0,
                                          far_body), 0, 1, &f)) {
        CHECK_INT(t, red_run_row(&f, 50, &first), 20);
        CHECK_INT(t, first, 65);                          /* shifted by 25 */
    }
    sr_frame_free(&f);
}

/* Consecutive card siblings draw far to near whatever their XML order. */
static void test_run_sorting(sr_test_ctx *t)
{
    char xml[4096];
    SrFrame f = {0};
#define FMT1 \
        "<shape id=\"a\" shape=\"rect\" x=\"50\" y=\"20\" width=\"100\" height=\"60\" " \
        "fill=\"#FF0000\" depth=\"%d\"/>" \
        "<shape id=\"b\" shape=\"rect\" x=\"50\" y=\"20\" width=\"100\" height=\"60\" " \
        "fill=\"#0000FF\" depth=\"%d\"/>"
    char body[1024];
    snprintf(body, sizeof body, FMT1, 0, 10);      /* red nearer, listed first */
    if (render_xml(t, "sort-a.xml", scene(xml, sizeof xml, NULL, true, 0, 0, 0, body),
                   0, 1, &f))
        CHECK(t, RED(&f, 100, 50));
    sr_frame_free(&f);
    snprintf(body, sizeof body, FMT1, 10, 0);
    if (render_xml(t, "sort-b.xml", scene(xml, sizeof xml, NULL, true, 0, 0, 0, body),
                   0, 1, &f))
        CHECK(t, BLUE(&f, 100, 50));
    sr_frame_free(&f);
}

/* rotationY = 45 turns the right edge away: a trapezoid whose right side
 * is shorter; the perspective (projective) path draws it. */
/* Sorting matters for translucency: a half-transparent near card listed
 * first must still be drawn over the far card (the depth test alone would
 * let the far card cover it, since translucent cards write no depth). */
static void test_run_sorting_translucent(sr_test_ctx *t)
{
    char xml[4096];
    SrFrame f = {0};
    if (render_xml(t, "sort-t.xml", scene(xml, sizeof xml, NULL, true, 0, 0, 0,
            "<shape id=\"a\" shape=\"rect\" x=\"50\" y=\"20\" width=\"100\" "
            "height=\"60\" fill=\"#FF000080\" depth=\"0\"/>"
            "<shape id=\"b\" shape=\"rect\" x=\"0\" y=\"0\" width=\"200\" "
            "height=\"100\" fill=\"#0000FF\" depth=\"10\"/>"), 0, 1, &f)) {
        const float *p = st_px(&f, 100, 50);
        CHECK_NEAR(t, p[0], 128.0 / 255.0, 2e-3);
        CHECK_NEAR(t, p[2], 1.0 - 128.0 / 255.0, 2e-3);
    }
    sr_frame_free(&f);
}

/* 3D objects interleave with root cards by depth: a translucent card in
 * front of a sphere veils it (drawn after it), one behind does not. */
static void test_sphere_behind_translucent_card(sr_test_ctx *t)
{
    char xml[4096];
    SrFrame f = {0};
    if (render_xml(t, "veil.xml", scene(xml, sizeof xml, NULL, true, 0, 0, 0,
            "<shape id=\"veil\" shape=\"rect\" x=\"0\" y=\"0\" width=\"200\" "
            "height=\"100\" fill=\"#FF000080\" depth=\"0\"/>"
            "<object3D id=\"ball\" primitive=\"sphere\" material=\"blue\" x=\"0\" "
            "y=\"0\" z=\"500\" radius=\"30\"/>"), 0, 1, &f)) {
        const float *p = st_px(&f, 100, 50);
        CHECK_NEAR(t, p[0], 128.0 / 255.0, 2e-3);
        CHECK_NEAR(t, p[2], 1.0 - 128.0 / 255.0, 2e-3);
    }
    sr_frame_free(&f);
}

/* A card splits translucent objects into two flushes. The first batch
 * must blend before the card and must not be blended again at the second
 * flush, with or without supersampling. */
static void test_translucent_object_batches(sr_test_ctx *t)
{
    for (int samples = 1; samples <= 4; samples *= 2) {
        char xml[4096], project[64];
        snprintf(project, sizeof project, " antialias3d=\"%d\"", samples);
        SrFrame f = {0};
        if (render_xml(t, "alpha-batches.xml", scene(xml, sizeof xml, project,
                true, 0, 0, 0,
                "<object3D id=\"far\" primitive=\"box\" material=\"alpha-red\" "
                "z=\"1000\" radius=\"100\"/>"
                "<shape id=\"middle\" shape=\"rect\" x=\"0\" y=\"0\" width=\"200\" "
                "height=\"100\" fill=\"#00FF0040\" depth=\"500\"/>"
                "<object3D id=\"near\" primitive=\"box\" material=\"alpha-blue\" "
                "z=\"0\" radius=\"100\"/>"), 0, 1, &f)) {
            const float *p = st_px(&f, 100, 50);
            double a = 128.0 / 255.0, green = 64.0 / 255.0;
            CHECK_NEAR(t, p[0], a * (1.0 - green) * (1.0 - a), 1e-6);
            CHECK_NEAR(t, p[1], green * (1.0 - a), 1e-6);
            CHECK_NEAR(t, p[2], a, 1e-6);
        }
        sr_frame_free(&f);
    }
}

/* The sphere's center is farther than the box, but its front surface
 * crosses the box. Deferred translucent samples must test the final
 * opaque depth, even when that opaque object was submitted later. */
static void test_relative_inactive_card_sort(sr_test_ctx *t)
{
    char xml[4096];
    scene(xml, sizeof(xml), " antialias3d=\"2\"", true, 0, 45, 0,
        "<object3D id=\"far\" primitive=\"sphere\" material=\"alpha-red\" "
        "x=\"10\" z=\"10\" radius=\"100\"/>"
        "<shape id=\"front\" shape=\"rect\" x=\"-200\" y=\"35\" width=\"20\" "
        "height=\"20\" fill=\"#00FF0040\" depth=\"-300\"/>"
        "<shape id=\"inactive\" shape=\"rect\" x=\"110\" y=\"40\" width=\"20\" "
        "height=\"20\" fill=\"#FFFFFF40\" depth=\"0\"/>"
        "<shape id=\"back\" shape=\"rect\" x=\"400\" y=\"50\" width=\"20\" "
        "height=\"20\" fill=\"#FFFF0040\" depth=\"300\"/>"
        "<object3D id=\"near\" primitive=\"box\" material=\"alpha-blue\" "
        "z=\"0\" radius=\"20\"/>");
    const double times[] = {.75, .25, 1, 0, .75};
    const float black[4] = {0, 0, 0, 1};
    for (int inactive = 0; inactive < 3; ++inactive) {
        SrScene scenes[2];
        bool a = st_load(t, "sort-length-a.xml", xml, &scenes[0], NULL) == SR_OK;
        bool b = st_load(t, "sort-length-b.xml", xml, &scenes[1], NULL) == SR_OK;
        CHECK(t, a && b);
        if (!a || !b) {
            if (a) sr_scene_free(&scenes[0]);
            if (b) sr_scene_free(&scenes[1]);
            continue;
        }
        for (int i = 0; i < 2; ++i) {
            scenes[i].cameras[0].orthographic = true;
            scenes[i].cameras[0].z.base = 0;
            SrNode *node = sr_scene_find_node(&scenes[i], "inactive");
            if (inactive == 0) node->visible = false;
            if (inactive == 1) node->end_time = .5;
            if (inactive == 2) node->opacity.base = 0;
        }
        scenes[1].has_relative_lengths = true;
        for (size_t i = 0; i < scenes[1].root->child_count; ++i) {
            SrNode *node = scenes[1].root->children[i];
            node->transform.x.base /= 2;
            node->transform.x.unit = SR_LENGTH_PERCENT;
            node->shape_width /= 2;
            node->shape_width_unit = SR_LENGTH_VW;
            node->shape_height_unit = SR_LENGTH_VH;
        }
        SrCompositor compositors[3];
        SrFrame frames[3] = {{0}};
        for (size_t i = 0; i < 3; ++i) {
            sr_compositor_init(&compositors[i], i == 2 ? 4 : 1);
            CHECK_INT(t, sr_frame_init(&frames[i], W, H), SR_OK);
        }
        for (size_t k = 0; k < sizeof(times) / sizeof(times[0]); ++k) {
            for (size_t i = 0; i < 3; ++i) {
                sr_frame_clear(&frames[i], black, 1);
                CHECK_INT(t, sr_compositor_render_scene(&compositors[i],
                              &scenes[i ? 1 : 0], times[k], &frames[i], NULL), SR_OK);
            }
            CHECK(t, !memcmp(frames[0].px, frames[1].px, (size_t)W * H * 4 * sizeof(float)));
            CHECK(t, !memcmp(frames[1].px, frames[2].px, (size_t)W * H * 4 * sizeof(float)));
        }
        /* Prove that substituting an inactive sort key changes this fixture:
         * the card splits crossing translucent objects into separate flushes. */
        SrNode *node = sr_scene_find_node(&scenes[0], "inactive");
        node->transform.x.base = 0;
        sr_frame_clear(&frames[0], black, 1);
        CHECK_INT(t, sr_compositor_render_scene(&compositors[0], &scenes[0], .75,
                      &frames[0], NULL), SR_OK);
        CHECK(t, memcmp(frames[0].px, frames[1].px, (size_t)W * H * 4 * sizeof(float)) != 0);
        for (size_t i = 0; i < 3; ++i) {
            sr_frame_free(&frames[i]);
            sr_compositor_free(&compositors[i]);
        }
        sr_scene_free(&scenes[0]); sr_scene_free(&scenes[1]);
    }
}

static void test_translucent_sphere_crosses_box(sr_test_ctx *t)
{
    for (int samples = 1; samples <= 4; samples *= 2) {
        char xml[4096], project[64];
        snprintf(project, sizeof project, " antialias3d=\"%d\"", samples);
        SrFrame f = {0};
        if (render_xml(t, "alpha-opaque.xml", scene(xml, sizeof xml, project,
                true, 0, 0, 0,
                "<shape id=\"back\" shape=\"rect\" width=\"200\" height=\"100\" "
                "fill=\"#000000\" depth=\"1000\"/>"
                "<object3D id=\"ball\" primitive=\"sphere\" material=\"alpha-red\" "
                "z=\"200\" radius=\"100\"/>"
                "<object3D id=\"box\" primitive=\"box\" material=\"blue\" "
                "z=\"150\" radius=\"100\"/>"), 0, 1, &f)) {
            const float *p = st_px(&f, 100, 50);
            CHECK_NEAR(t, p[0], 128.0 / 255.0, 1e-6);
            CHECK_NEAR(t, p[2], 1.0 - 128.0 / 255.0, 1e-6);
            CHECK(t, BLUE(&f, 25, 50));
        }
        sr_frame_free(&f);
    }
}

static void test_tilt_trapezoid(sr_test_ctx *t)
{
    char xml[4096];
    SrFrame f = {0};
    lens = 150.0;
    /* Edges 20 cos 45 = 14.1 nearer / farther than the pivot at 150: the
     * left edge scales by 150 / 135.9 (44 px), the right by 150 / 164.1
     * (37 px). */
    if (render_xml(t, "tilt.xml", scene(xml, sizeof xml, NULL, true, 0, 0, 0,
            "<shape id=\"r\" shape=\"rect\" x=\"100\" y=\"50\" anchorX=\"20\" "
            "anchorY=\"20\" width=\"40\" height=\"40\" fill=\"#FF0000\" "
            "rotationY=\"45\"/>"), 0, 1, &f)) {
        int first = 0, run = red_run_row(&f, 50, &first);
        CHECK(t, run > 26 && run < 34);                   /* about 40 cos 45 */
        int left = red_run_col(&f, first), right = red_run_col(&f, first + run - 1);
        CHECK(t, left >= 42 && left <= 46);
        CHECK(t, right >= 35 && right <= 39);
    }
    lens = 1000.0;
    sr_frame_free(&f);
}

/* A yawed camera takes the projective path; edges land where the
 * analytic projection puts them. */
static void test_projective_matches_analytic(sr_test_ctx *t)
{
    char xml[4096];
    SrFrame f = {0};
    SrScene s;
    const char *body =
        "<shape id=\"r\" shape=\"rect\" x=\"60\" y=\"30\" width=\"80\" height=\"40\" "
        "fill=\"#FF0000\" depth=\"40\"/>";
    lens = 150.0;
    scene(xml, sizeof xml, NULL, true, 0, 12, 0, body);
    lens = 1000.0;
    if (st_load(t, "proj-an.xml", xml, &s, NULL) != SR_OK) return;
    SrCardView view = sr_card_view(&s, 0.0);
    SrCardPose pose = sr_card_pose(&view, 60, 30, 40, 0, 0);
    CHECK(t, !pose.is_affine);
    double lx, ly, rx, ry;
    CHECK(t, sr_card_to_screen(&pose, 60, 50, &lx, &ly));
    CHECK(t, sr_card_to_screen(&pose, 140, 50, &rx, &ry));
    sr_scene_free(&s);
    if (render_xml(t, "proj.xml", xml, 0, 1, &f)) {
        int row = (int)floor((ly + ry) * 0.5);
        int first = 0, run = red_run_row(&f, row, &first);
        CHECK(t, fabs(first - lx) <= 1.0);
        CHECK(t, fabs(first + run - rx) <= 1.0);
    }
    sr_frame_free(&f);
}

/* A 3D sphere between two root cards: the near card hides it, it hides
 * the far card (objects join the first root card run). */
static void test_sphere_between_cards(sr_test_ctx *t)
{
    const char *body =
        "<shape id=\"far\" shape=\"rect\" x=\"-200\" y=\"-100\" width=\"600\" "
        "height=\"300\" fill=\"#0000FF\" depth=\"2000\"/>"
        "<shape id=\"near\" shape=\"rect\" x=\"0\" y=\"0\" width=\"100\" height=\"100\" "
        "fill=\"#FF0000\" depth=\"0\"/>"
        "<object3D id=\"ball\" primitive=\"sphere\" material=\"white\" x=\"0\" y=\"0\" "
        "z=\"500\" radius=\"30\"/>";
    const char *aa[2] = {NULL, " antialias3d=\"2\""};
    for (int k = 0; k < 2; ++k) {
        char xml[4096];
        SrFrame f = {0};
        if (render_xml(t, k ? "ball-aa.xml" : "ball.xml",
                       scene(xml, sizeof xml, aa[k], true, 0, 0, 0, body), 0, 1, &f)) {
            CHECK(t, RED(&f, 95, 50));      /* near card over the sphere */
            CHECK(t, WHITE(&f, 105, 50));   /* sphere over the far card */
            CHECK(t, BLUE(&f, 180, 50));
            CHECK(t, RED(&f, 20, 50));
        }
        sr_frame_free(&f);
    }
}

/* Cards inside a group miss the root run and draw after the 3D pass: the
 * per-sample depth test still keeps a nearer sphere in front. */
static void test_depth_test_against_3d(sr_test_ctx *t)
{
    char xml[4096];
    SrFrame f = {0};
    if (render_xml(t, "dt.xml", scene(xml, sizeof xml, NULL, true, 0, 0, 0,
            "<object3D id=\"ball\" primitive=\"sphere\" material=\"white\" x=\"0\" "
            "y=\"0\" z=\"0\" radius=\"20\"/>"
            "<group id=\"g\" z=\"5\"><shape id=\"far\" shape=\"rect\" x=\"-100\" "
            "y=\"-50\" width=\"400\" height=\"200\" fill=\"#0000FF\" depth=\"1000\"/>"
            "</group>"),
            0, 1, &f)) {
        CHECK(t, WHITE(&f, 100, 50));
        CHECK(t, BLUE(&f, 20, 50));
    }
    sr_frame_free(&f);
}

/* Crossing tilted cards: each is visible where it is nearer, in either
 * XML order (the second drawn is depth tested per sample). */
static void test_crossing_cards(sr_test_ctx *t)
{
#define FMT2 \
        "<shape id=\"%s\" shape=\"rect\" x=\"100\" y=\"50\" anchorX=\"60\" anchorY=\"30\" " \
        "width=\"120\" height=\"60\" fill=\"%s\" rotationY=\"%s\"/>"
    for (int order = 0; order < 2; ++order) {
        char red[512], blue[512], body[1024], xml[4096];
        snprintf(red, sizeof red, FMT2, "r", "#FF0000", "30");
        snprintf(blue, sizeof blue, FMT2, "b", "#0000FF", "-30");
        snprintf(body, sizeof body, "%s%s", order ? blue : red, order ? red : blue);
        SrFrame f = {0};
        if (render_xml(t, "cross.xml", scene(xml, sizeof xml, NULL, true, 0, 0, 0, body),
                       0, 1, &f)) {
            CHECK(t, RED(&f, 70, 50));     /* red's left edge is nearer */
            CHECK(t, BLUE(&f, 130, 50));
        }
        sr_frame_free(&f);
    }
}

/* Depth is written only where the composited card alpha reaches 0.5, and
 * never by a card inside an isolated group. */
static void test_depth_write_rules(sr_test_ctx *t)
{
#define FMT3 \
        "<group id=\"g1\" z=\"0\"%s><shape id=\"near\" shape=\"rect\" x=\"0\" y=\"0\" " \
        "width=\"200\" height=\"100\" fill=\"%s\" depth=\"0\"/></group>" \
        "<group id=\"g2\" z=\"1\"><shape id=\"far\" shape=\"rect\" x=\"0\" y=\"0\" " \
        "width=\"200\" height=\"100\" fill=\"#0000FF\" depth=\"1000\"/></group>"
    struct { const char *group, *fill; bool far_visible; } cases[] = {
        {"", "#FF0000FF", false},             /* opaque: writes, far hidden */
        {"", "#FF000040", true},              /* alpha 0.25: no write */
        {" opacity=\"0.5\"", "#FF0000FF", true}, /* isolated ancestor: no write */
    };
    for (size_t i = 0; i < sizeof cases / sizeof cases[0]; ++i) {
        char body[1024], xml[4096];
        snprintf(body, sizeof body, FMT3, cases[i].group, cases[i].fill);
        SrFrame f = {0};
        if (render_xml(t, "write.xml", scene(xml, sizeof xml, NULL, true, 0, 0, 0, body),
                       0, 1, &f)) {
            bool far = st_px(&f, 100, 50)[2] > 0.99f;
            if (far != cases[i].far_visible) SR_FAIL(t, "case %zu", i);
        }
        sr_frame_free(&f);
    }
}

/* Depth of field: a card at the focus distance stays sharp, a far one
 * spreads across its edge. */
static void test_depth_of_field(sr_test_ctx *t)
{
    char xml[4096];
    SrFrame sharp = {0}, soft = {0};
#define FMT4 \
        "<shape id=\"r\" shape=\"rect\" x=\"0\" y=\"0\" width=\"100\" height=\"100\" " \
        "fill=\"#FF0000\" depth=\"%s\"/>"
    char body[512];
    snprintf(body, sizeof body, FMT4, "0");
    bool ok = render_xml(t, "dof-sharp.xml", scene(xml, sizeof xml, NULL, true, 0, 0, 20,
                                                   body), 0, 1, &sharp);
    snprintf(body, sizeof body, FMT4, "1000");     /* r = 20 * 1000 / 2000 = 10 */
    ok = render_xml(t, "dof-soft.xml", scene(xml, sizeof xml, NULL, true, 0, 0, 20, body),
                    0, 1, &soft) && ok;
    if (ok) {
        CHECK(t, RED(&sharp, 99, 50));
        CHECK(t, BLACK(&sharp, 100, 50));
        /* At half size the card spans x 50..100; sigma 5 px spreads both
         * edges while the middle stays solid. */
        CHECK(t, st_px(&soft, 46, 50)[0] > 0.1f);
        CHECK(t, st_px(&soft, 53, 50)[0] < 0.9f);
        CHECK(t, st_px(&soft, 103, 50)[0] > 0.1f);
        CHECK(t, st_px(&soft, 75, 50)[0] > 0.99f);
    }
    sr_frame_free(&sharp); sr_frame_free(&soft);
}

/* A card behind the camera draws nothing; a node without depth ignores
 * the camera entirely. */
static void test_clipping_and_legacy_nodes(sr_test_ctx *t)
{
    char xml[4096];
    SrFrame f = {0};
    if (render_xml(t, "behind.xml", scene(xml, sizeof xml, NULL, true, 0, 0, 0,
            "<shape id=\"r\" shape=\"rect\" x=\"0\" y=\"0\" width=\"200\" height=\"100\" "
            "fill=\"#FF0000\" depth=\"-1500\"/>"), 0, 1, &f)) {
        bool empty = true;
        for (size_t i = 0; i < (size_t)W * H; ++i) empty = empty && f.px[i * 4] == 0.0f;
        CHECK(t, empty);
    }
    sr_frame_free(&f);
    if (render_xml(t, "legacy.xml", scene(xml, sizeof xml, NULL, true, 500, 20, 0,
            "<shape id=\"r\" shape=\"rect\" x=\"10\" y=\"10\" width=\"20\" height=\"20\" "
            "fill=\"#FF0000\"/>"), 0, 1, &f)) {
        CHECK(t, RED(&f, 20, 20));
        CHECK(t, BLACK(&f, 40, 20));
    }
    sr_frame_free(&f);
}

/* Particles inside a card scale with it. */
static void test_particles_scale(sr_test_ctx *t)
{
#define FMT5 \
        "<particleEmitter id=\"p\" x=\"100\" y=\"50\" rate=\"20\" lifetime=\"5\" " \
        "speed=\"0\" spread=\"0\" size=\"12\" color=\"#FF0000\" depth=\"%s\"/>"
    int runs[2];
    const char *depths[2] = {"0", "1000"};
    for (int i = 0; i < 2; ++i) {
        char body[512], xml[4096];
        snprintf(body, sizeof body, FMT5, depths[i]);
        SrFrame f = {0};
        runs[i] = 0;
        if (render_xml(t, "part.xml", scene(xml, sizeof xml, NULL, true, 0, 0, 0, body),
                       1.0, 1, &f))
            runs[i] = red_run_row(&f, 50, NULL);
        sr_frame_free(&f);
    }
    CHECK(t, runs[0] >= 22);
    CHECK(t, runs[1] * 2 >= runs[0] - 2 && runs[1] * 2 <= runs[0] + 2);
}

/* With a camera a sphere's depth is its camera-facing surface: a large
 * sphere in front hides a small one centred just inside its back half. */
static void test_sphere_surface_depth(sr_test_ctx *t)
{
    char xml[4096];
    SrFrame f = {0};
    if (render_xml(t, "spheres.xml", scene(xml, sizeof xml, NULL, true, 0, 0, 0,
            "<object3D id=\"big\" primitive=\"sphere\" material=\"red\" x=\"0\" y=\"0\" "
            "z=\"0\" radius=\"50\"/>"
            "<object3D id=\"small\" primitive=\"sphere\" material=\"blue\" x=\"0\" y=\"0\" "
            "z=\"40\" radius=\"5\"/>"
            "<shape id=\"c\" shape=\"rect\" x=\"0\" y=\"0\" width=\"1\" height=\"1\" "
            "fill=\"#000000\" depth=\"5000\"/>"), 0, 1, &f))
        CHECK(t, RED(&f, 100, 50));
    sr_frame_free(&f);
}

/* The sphere-depth fix applies without cards too (the legacy path). */
static void test_sphere_surface_depth_without_cards(sr_test_ctx *t)
{
    char xml[4096];
    SrFrame f = {0};
    if (render_xml(t, "spheres-legacy.xml", scene(xml, sizeof xml, NULL, true, 0, 0, 0,
            "<object3D id=\"big\" primitive=\"sphere\" material=\"red\" x=\"0\" y=\"0\" "
            "z=\"0\" radius=\"50\"/>"
            "<object3D id=\"small\" primitive=\"sphere\" material=\"blue\" x=\"0\" y=\"0\" "
            "z=\"40\" radius=\"5\"/>"), 0, 1, &f))
        CHECK(t, RED(&f, 100, 50));
    sr_frame_free(&f);
}

/* One compositor reused across frame sizes and antialias3d factors whose
 * depth buffers have equal totals (200x100 at 4 and 400x200 at 2) must
 * resize its depth geometry: same output as a fresh compositor. */
static void test_compositor_reuse(sr_test_ctx *t)
{
    const char *small =
        "<scene version=\"1.0\"><project width=\"200\" height=\"100\" fps=\"1\" "
        "duration=\"1\" linearLight=\"false\" antialias3d=\"4\"/><composition>"
        "<shape id=\"r\" shape=\"rect\" width=\"50\" height=\"50\" fill=\"#FF0000\" "
        "depth=\"0\"/></composition></scene>";
    const char *large =
        "<scene version=\"1.0\"><project width=\"400\" height=\"200\" fps=\"1\" "
        "duration=\"1\" linearLight=\"false\" antialias3d=\"2\"/><composition>"
        "<shape id=\"r\" shape=\"rect\" x=\"0\" y=\"150\" width=\"400\" "
        "height=\"50\" fill=\"#FF0000\" depth=\"0\"/></composition></scene>";
    SrScene a, b;
    if (st_load(t, "reuse-a.xml", small, &a, NULL) != SR_OK) return;
    if (st_load(t, "reuse-b.xml", large, &b, NULL) != SR_OK) { sr_scene_free(&a); return; }
    const float black[4] = {0, 0, 0, 1};
    SrFrame fa = {0}, fb = {0}, fresh = {0};
    SrCompositor shared, alone;
    sr_compositor_init(&shared, 1);
    sr_compositor_init(&alone, 1);
    bool ok = sr_frame_init(&fa, 200, 100) == SR_OK &&
              sr_frame_init(&fb, 400, 200) == SR_OK &&
              sr_frame_init(&fresh, 400, 200) == SR_OK;
    CHECK(t, ok);
    if (ok) {
        sr_frame_clear(&fa, black, 1);
        sr_frame_clear(&fb, black, 1);
        sr_frame_clear(&fresh, black, 1);
        CHECK(t, sr_compositor_render_scene(&shared, &a, 0, &fa, NULL) == SR_OK);
        CHECK(t, sr_compositor_render_scene(&shared, &b, 0, &fb, NULL) == SR_OK);
        CHECK(t, sr_compositor_render_scene(&alone, &b, 0, &fresh, NULL) == SR_OK);
        CHECK(t, shared.depth_store.samples == 2);
        CHECK(t, st_frames_equal(&fb, &fresh));
        CHECK(t, RED(&fb, 10, 190));
    }
    sr_compositor_free(&shared); sr_compositor_free(&alone);
    sr_frame_free(&fa); sr_frame_free(&fb); sr_frame_free(&fresh);
    sr_scene_free(&a); sr_scene_free(&b);
}

/* A drop shadow of a subgroup inside a perspective (tilted) card reaches
 * past the subgroup's shapes: the plane region is not cropped to them. */
static void test_projective_card_keeps_effect_reach(sr_test_ctx *t)
{
    char xml[4096];
    SrFrame f = {0};
    snprintf(xml, sizeof xml,
        "<scene version=\"1.0\"><project width=\"200\" height=\"100\" fps=\"1\" "
        "duration=\"1\" linearLight=\"false\"/><composition>"
        "<camera id=\"cam\" z=\"-1000\" zoom=\"1000\"/>"
        "<group id=\"card\" rotationY=\"1\" x=\"100\" y=\"50\" anchorX=\"100\" "
        "anchorY=\"50\"><group id=\"sub\" effects=\"ds\"><shape id=\"r\" "
        "shape=\"rect\" x=\"40\" y=\"30\" width=\"40\" height=\"40\" "
        "fill=\"#FF0000\"/></group></group></composition>"
        "<effects><effect id=\"ds\" type=\"drop-shadow\" offsetX=\"60\" offsetY=\"0\" "
        "radius=\"0\" intensity=\"1\" color=\"#00FF00\"/></effects></scene>");
    if (render_xml(t, "reach.xml", xml, 0, 1, &f)) {
        CHECK(t, RED(&f, 60, 50));
        CHECK(t, st_px(&f, 125, 50)[1] > 0.5f);        /* the shadow, 60 px right */
    }
    sr_frame_free(&f);
}

/* Depth-of-field blur starts continuously from zero radius. */
static void test_depth_of_field_continuity(sr_test_ctx *t)
{
    const char *body =
        "<shape id=\"r\" shape=\"rect\" x=\"0\" y=\"0\" width=\"100\" height=\"100\" "
        "fill=\"#FF0000\" depth=\"1000\"/>";
    double apertures[3] = {0.0, 0.8, 1.6};       /* radius 0, 0.4, 0.8 */
    float outside[3] = {0, 0, 0};
    for (int i = 0; i < 3; ++i) {
        char xml[4096];
        SrFrame f = {0};
        if (render_xml(t, "dof-c.xml", scene(xml, sizeof xml, NULL, true, 0, 0,
                                             apertures[i], body), 0, 1, &f))
            outside[i] = st_px(&f, 49, 50)[0];
        sr_frame_free(&f);
    }
    CHECK(t, outside[0] == 0.0f);
    CHECK(t, outside[1] > 0.0f && outside[1] < 0.15f);
    CHECK(t, outside[2] > outside[1]);
}

/* One complex scene (tilt, yawed camera, DOF, sphere, supersampling)
 * renders byte-identically on 1 and 4 threads. */
static void test_thread_invariance(sr_test_ctx *t)
{
    char xml[4096];
    const char *body =
        "<shape id=\"far\" shape=\"rect\" x=\"0\" y=\"0\" width=\"200\" height=\"100\" "
        "fill=\"#0000FF\" depth=\"1500\"/>"
        "<shape id=\"tilt\" shape=\"ellipse\" x=\"100\" y=\"50\" anchorX=\"50\" "
        "anchorY=\"30\" width=\"100\" height=\"60\" fill=\"#FF8000\" rotationX=\"35\" "
        "rotationY=\"-20\" depth=\"300\"/>"
        "<object3D id=\"ball\" primitive=\"sphere\" material=\"white\" x=\"30\" y=\"0\" "
        "z=\"200\" radius=\"25\"/>";
    scene(xml, sizeof xml, " antialias3d=\"2\"", true, 20, 6, 8, body);
    SrFrame one = {0}, four = {0};
    if (render_xml(t, "thr1.xml", xml, 0, 1, &one) &&
        render_xml(t, "thr4.xml", xml, 0, 4, &four))
        CHECK(t, st_frames_equal(&one, &four));
    sr_frame_free(&one); sr_frame_free(&four);
}

/* depth / rotationX / rotationY attributes and animate tracks make cards;
 * invalid nesting, zoom and aperture are rejected; cards need standard
 * mode (checked by sr_render, so --validate reports it too). */
static void test_xml(sr_test_ctx *t)
{
    char xml[4096];
    SrScene s;
    if (st_load(t, "x-ok.xml", scene(xml, sizeof xml, NULL, true, 0, 0, 0,
            "<shape id=\"a\" shape=\"rect\" width=\"1\" height=\"1\" rotationX=\"3\"/>"
            "<group id=\"g\"><animate property=\"depth\">"
            "<key time=\"0\" value=\"0\"/><key time=\"1\" value=\"50\"/></animate></group>"
            "<shape id=\"c\" shape=\"rect\" width=\"1\" height=\"1\"/>"),
            &s, NULL) == SR_OK) {
        CHECK(t, s.has_cards);
        CHECK(t, sr_scene_find_node(&s, "a")->card);
        CHECK(t, sr_scene_find_node(&s, "g")->card);
        CHECK(t, !sr_scene_find_node(&s, "c")->card);
        CHECK_NEAR(t, sr_anim_eval(&sr_scene_find_node(&s, "g")->transform.z, 0.5), 25, 1e-9);
        CHECK(t, s.cameras[0].zoom_set);
        SrRenderOptions options = {.validate_only = true};
        SrRenderMetrics metrics;
        FILE *sink;
        SrDiagnostics diag;
        st_diag(&diag, &sink);
        CHECK(t, sr_render(&s, &options, &metrics, &diag) == SR_OK);
        s.project.mode = SR_MODE_EQUIRECTANGULAR;
        CHECK(t, sr_render(&s, &options, &metrics, &diag) == SR_ERR_ARGUMENT);
        if (sink) fclose(sink);
        sr_scene_free(&s);
    }
    char *message = NULL;
    CHECK(t, st_load(t, "x-nest.xml", scene(xml, sizeof xml, NULL, false, 0, 0, 0,
            "<group id=\"g\" depth=\"10\"><shape id=\"a\" shape=\"rect\" width=\"1\" "
            "height=\"1\" depth=\"5\"/></group>"), &s, &message) != SR_OK);
    CHECK_CONTAINS(t, message, "inside another depth card");
    free(message);
    message = NULL;
    CHECK(t, st_load(t, "x-zoom.xml",
        "<scene version=\"1.0\"><project width=\"10\" height=\"10\" fps=\"1\" duration=\"1\"/>"
        "<composition><camera id=\"c\" zoom=\"0\"/></composition></scene>",
        &s, &message) != SR_OK);
    CHECK_CONTAINS(t, message, "zoom");
    free(message);
    message = NULL;
    CHECK(t, st_load(t, "x-ap.xml",
        "<scene version=\"1.0\"><project width=\"10\" height=\"10\" fps=\"1\" duration=\"1\"/>"
        "<composition><camera id=\"c\" aperture=\"-1\"/></composition></scene>",
        &s, &message) != SR_OK);
    CHECK_CONTAINS(t, message, "aperture");
    free(message);
}

const sr_test_case sr_tests_depth[] = {
    {"identity", test_identity},
    {"parallax_and_scale", test_parallax_and_scale},
    {"run_sorting", test_run_sorting},
    {"run_sorting_translucent", test_run_sorting_translucent},
    {"sphere_behind_translucent_card", test_sphere_behind_translucent_card},
    {"translucent_object_batches", test_translucent_object_batches},
    {"relative_inactive_card_sort", test_relative_inactive_card_sort},
    {"translucent_sphere_crosses_box", test_translucent_sphere_crosses_box},
    {"tilt_trapezoid", test_tilt_trapezoid},
    {"projective_matches_analytic", test_projective_matches_analytic},
    {"sphere_between_cards", test_sphere_between_cards},
    {"depth_test_against_3d", test_depth_test_against_3d},
    {"crossing_cards", test_crossing_cards},
    {"depth_write_rules", test_depth_write_rules},
    {"depth_of_field", test_depth_of_field},
    {"clipping_and_legacy_nodes", test_clipping_and_legacy_nodes},
    {"particles_scale", test_particles_scale},
    {"sphere_surface_depth", test_sphere_surface_depth},
    {"sphere_surface_depth_without_cards", test_sphere_surface_depth_without_cards},
    {"compositor_reuse", test_compositor_reuse},
    {"projective_card_keeps_effect_reach", test_projective_card_keeps_effect_reach},
    {"depth_of_field_continuity", test_depth_of_field_continuity},
    {"thread_invariance", test_thread_invariance},
    {"xml", test_xml},
    {NULL, NULL},
};
