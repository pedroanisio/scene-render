/* SPDX-License-Identifier: Apache-2.0 */
/* The mesh-warp modifier and the grid sampler it shares with soft bodies. */
#include "scene_render/deform.h"

#include "scene_text.h"

static bool render(sr_test_ctx *t, SrScene *scene, double time, SrFrame *frame)
{
    const float clear[4] = {0, 0, 0, 0};
    if (sr_frame_init(frame, scene->project.width, scene->project.height) != SR_OK) {
        SR_FAIL(t, "frame allocation failed");
        return false;
    }
    sr_frame_clear(frame, clear, 1);
    SrStatus status = sr_composite_scene(scene, time, frame, NULL);
    CHECK(t, status == SR_OK);
    return status == SR_OK;
}

#define HEAD "<scene version=\"1.0\"><project width=\"96\" height=\"80\" fps=\"10\" " \
             "duration=\"1\" linearLight=\"false\"/><composition>"

/* An all-zero control grid reproduces the undeformed node bit for bit
 * (rotated and scaled, with a vector image and a shape). */
static void test_identity_grid_bit_exact(sr_test_ctx *t)
{
    const char *plain = HEAD
        "<shape id=\"s\" shape=\"ellipse\" width=\"40\" height=\"30\" x=\"30\" y=\"20\" "
        "rotation=\"17\" scaleX=\"1.3\" fill=\"#3080FF\" stroke=\"#FFFFFF\" strokeWidth=\"3\"/>"
        "</composition></scene>";
    const char *warped = HEAD
        "<shape id=\"s\" shape=\"ellipse\" width=\"40\" height=\"30\" x=\"30\" y=\"20\" "
        "rotation=\"17\" scaleX=\"1.3\" fill=\"#3080FF\" stroke=\"#FFFFFF\" strokeWidth=\"3\">"
        "<deform><modifier type=\"mesh-warp\" rows=\"3\" cols=\"5\">"
        "<point row=\"1\" col=\"2\" x=\"0\" y=\"0\"/></modifier></deform>"
        "</shape></composition></scene>";
    SrScene a, b;
    if (st_load(t, "warp-plain.xml", plain, &a, NULL) != SR_OK) { SR_FAIL(t, "load"); return; }
    if (st_load(t, "warp-identity.xml", warped, &b, NULL) != SR_OK) {
        SR_FAIL(t, "load"); sr_scene_free(&a); return;
    }
    CHECK_INT(t, b.root->children[0]->modifier_count, 1);
    SrFrame fa = {0}, fb = {0};
    if (render(t, &a, 0.0, &fa) && render(t, &b, 0.0, &fb))
        CHECK(t, st_frames_equal(&fa, &fb));
    sr_frame_free(&fa); sr_frame_free(&fb);
    sr_scene_free(&a); sr_scene_free(&b);
}

/* Moving the right-middle control point 10 px outward bulges the right
 * edge there (beyond the node's own box), and leaves the fixed corners. */
static void test_moved_point_displaces(sr_test_ctx *t)
{
    const char *xml = HEAD
        "<shape id=\"s\" shape=\"rect\" width=\"40\" height=\"40\" x=\"20\" y=\"20\" fill=\"#FFFFFF\">"
        "<deform><modifier type=\"mesh-warp\" rows=\"3\" cols=\"3\">"
        "<point row=\"1\" col=\"2\" x=\"0\" y=\"0\">"
        "<animate property=\"x\"><key time=\"0\" value=\"0\"/><key time=\"1\" value=\"10\"/></animate>"
        "</point></modifier></deform></shape></composition></scene>";
    SrScene scene;
    if (st_load(t, "warp-move.xml", xml, &scene, NULL) != SR_OK) { SR_FAIL(t, "load"); return; }
    SrFrame rest = {0}, moved = {0};
    if (render(t, &scene, 0.0, &rest) && render(t, &scene, 1.0, &moved)) {
        CHECK_NEAR(t, st_px(&rest, 65, 40)[3], 0.0, 0.0);
        CHECK_NEAR(t, st_px(&moved, 65, 40)[3], 1.0, 1e-6);   /* bulge */
        CHECK_NEAR(t, st_px(&moved, 68, 40)[3], 1.0, 1e-6);
        CHECK_NEAR(t, st_px(&moved, 71, 40)[3], 0.0, 0.0);    /* past the tip */
        CHECK_NEAR(t, st_px(&moved, 61, 21)[3], 0.0, 0.0);    /* corner fixed */
        CHECK_NEAR(t, st_px(&moved, 30, 40)[3], 1.0, 1e-6);   /* left untouched */
    }
    sr_frame_free(&rest); sr_frame_free(&moved);
    sr_scene_free(&scene);
}

/* The sampler inverts the bilinear warp: forward-mapping the returned
 * point lands on the query. */
static void test_sampler_inverts_warp(sr_test_ctx *t)
{
    double offsets[2 * 9] = {0};
    offsets[2 * 4] = 6.0;       /* center moves (+6, -3) */
    offsets[2 * 4 + 1] = -3.0;
    offsets[2 * 8] = -4.0;      /* bottom-right moves (-4, 0) */
    for (int i = 0; i <= 20; ++i) {
        SrVec2 q = {i * 2.0 + 0.3, 40.0 - i * 1.7};
        SrVec2 p;
        CHECK(t, sr_grid_warp_inverse(offsets, 3, 3, 40.0, 40.0, q, &p));
        /* Forward map: bilinear displacement at p. */
        double u = fmin(fmax(p.x / 20.0, 0.0), 2.0), v = fmin(fmax(p.y / 20.0, 0.0), 2.0);
        int c = u >= 1.0 ? 1 : 0, r = v >= 1.0 ? 1 : 0;
        double fu = u - c, fv = v - r;
        const double *d00 = offsets + 2 * (r * 3 + c), *d01 = d00 + 2;
        const double *d10 = d00 + 6, *d11 = d10 + 2;
        double dx = d00[0]*(1-fu)*(1-fv) + d01[0]*fu*(1-fv) + d10[0]*(1-fu)*fv + d11[0]*fu*fv;
        double dy = d00[1]*(1-fu)*(1-fv) + d01[1]*fu*(1-fv) + d10[1]*(1-fu)*fv + d11[1]*fu*fv;
        CHECK_NEAR(t, p.x + dx, q.x, 1e-6);
        CHECK_NEAR(t, p.y + dy, q.y, 1e-6);
    }
    double zero[8] = {0};
    SrVec2 q = {12.345, 6.789};
    SrVec2 p;
    CHECK(t, sr_grid_warp_inverse(zero, 2, 2, 10.0, 10.0, q, &p));
    CHECK(t, p.x == q.x && p.y == q.y);
    CHECK_NEAR(t, sr_grid_warp_extent(offsets, 9), 6.0, 0.0);
}

static void test_point_validation(sr_test_ctx *t)
{
    const char *xml = HEAD
        "<shape id=\"s\" shape=\"rect\" width=\"40\" height=\"40\">"
        "<deform><modifier type=\"mesh-warp\" rows=\"2\" cols=\"2\">"
        "<point row=\"2\" col=\"0\"/></modifier></deform></shape></composition></scene>";
    SrScene scene;
    char *message = NULL;
    CHECK(t, st_load(t, "warp-bad.xml", xml, &scene, &message) == SR_ERR_XML);
    CHECK_CONTAINS(t, message, "<point> @row: row is outside the control grid");
    free(message);
}


/* Forward residual of p against q under a rows x cols grid over w x h. */
static double forward_residual(const double *offsets, uint32_t rows, uint32_t cols,
                               double w, double h, SrVec2 p, SrVec2 q)
{
    double u = fmin(fmax(p.x / w * (cols - 1), 0.0), cols - 1.0);
    double v = fmin(fmax(p.y / h * (rows - 1), 0.0), rows - 1.0);
    uint32_t c = (uint32_t)fmin(floor(u), cols - 2.0), r = (uint32_t)fmin(floor(v), rows - 2.0);
    double fu = u - c, fv = v - r;
    const double *d00 = offsets + 2 * (r * cols + c), *d01 = d00 + 2;
    const double *d10 = d00 + 2 * cols, *d11 = d10 + 2;
    double dx = d00[0]*(1-fu)*(1-fv) + d01[0]*fu*(1-fv) + d10[0]*(1-fu)*fv + d11[0]*fu*fv;
    double dy = d00[1]*(1-fu)*(1-fv) + d01[1]*fu*(1-fv) + d10[1]*(1-fu)*fv + d11[1]*fu*fv;
    return fmax(fabs(p.x + dx - q.x), fabs(p.y + dy - q.y));
}

/* The review's cycling case: a width-2, three-column grid with x offsets
 * (-2, -0.3, -0.5) on both rows. Newton from q = -0.3 alternates between
 * 1.7 and -0.25 (residual 1.56); the fallback solves the first cell
 * analytically: -2 + 2.7 fu = -0.3, p.x = 1.7 / 2.7. */
static void test_inverse_newton_cycle_falls_back(sr_test_ctx *t)
{
    double offsets[2 * 6] = {-2, 0, -0.3, 0, -0.5, 0, -2, 0, -0.3, 0, -0.5, 0};
    SrVec2 q = {-0.3, 1.0}, p = {0, 0};
    CHECK(t, sr_grid_warp_inverse(offsets, 2, 3, 2.0, 2.0, q, &p));
    CHECK_NEAR(t, p.x, 1.7 / 2.7, 1e-9);
    CHECK_NEAR(t, p.y, 1.0, 1e-9);
    CHECK(t, forward_residual(offsets, 2, 3, 2.0, 2.0, p, q) < 1e-4);
    /* Every query along the row inverts with a small residual. */
    for (int i = 0; i <= 40; ++i) {
        SrVec2 row = {-3.0 + i * 0.125, 0.5};
        if (!sr_grid_warp_inverse(offsets, 2, 3, 2.0, 2.0, row, &p) ||
            !(forward_residual(offsets, 2, 3, 2.0, 2.0, p, row) < 1e-4))
            SR_FAIL(t, "query x=%g not inverted", row.x);
    }
    /* No source at all (a non-finite query) is reported, not guessed. */
    SrVec2 bad = {NAN, 0.0};
    CHECK(t, !sr_grid_warp_inverse(offsets, 2, 3, 2.0, 2.0, bad, &p));
}

/* Two uniform +10 px mesh-warps compose to +20 px: the node's bounds are
 * padded by the sum of the grids' displacements, so the shifted right
 * edge is not clipped (a single-grid 10 px pad cut it at x = 41). */
static void test_composed_grids_pad_by_sum(sr_test_ctx *t)
{
    const char *xml = HEAD
        "<shape id=\"s\" shape=\"rect\" width=\"20\" height=\"20\" x=\"10\" y=\"10\" fill=\"#FFFFFF\">"
        "<deform>"
        "<modifier type=\"mesh-warp\" rows=\"2\" cols=\"2\">"
        "<point row=\"0\" col=\"0\" x=\"10\" y=\"0\"/><point row=\"0\" col=\"1\" x=\"10\" y=\"0\"/>"
        "<point row=\"1\" col=\"0\" x=\"10\" y=\"0\"/><point row=\"1\" col=\"1\" x=\"10\" y=\"0\"/>"
        "</modifier>"
        "<modifier type=\"mesh-warp\" rows=\"2\" cols=\"2\">"
        "<point row=\"0\" col=\"0\" x=\"10\" y=\"0\"/><point row=\"0\" col=\"1\" x=\"10\" y=\"0\"/>"
        "<point row=\"1\" col=\"0\" x=\"10\" y=\"0\"/><point row=\"1\" col=\"1\" x=\"10\" y=\"0\"/>"
        "</modifier></deform></shape></composition></scene>";
    SrScene scene;
    if (st_load(t, "warp-compose.xml", xml, &scene, NULL) != SR_OK) { SR_FAIL(t, "load"); return; }
    SrFrame frame = {0};
    if (render(t, &scene, 0.0, &frame)) {
        CHECK_NEAR(t, st_px(&frame, 28, 20)[3], 0.0, 0.0);    /* moved away */
        CHECK_NEAR(t, st_px(&frame, 31, 20)[3], 1.0, 1e-6);
        CHECK_NEAR(t, st_px(&frame, 45, 20)[3], 1.0, 1e-6);   /* was clipped */
        CHECK_NEAR(t, st_px(&frame, 48, 20)[3], 1.0, 1e-6);
        CHECK_NEAR(t, st_px(&frame, 51, 20)[3], 0.0, 0.0);
    }
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

/* A malformed mesh-warp (invalid axis after rows/cols) fails cleanly; the
 * control array is allocated only once every attribute is valid, so a
 * leak checker sees nothing. */
static void test_malformed_mesh_warp_fails_cleanly(sr_test_ctx *t)
{
    static const char *const bad[] = {"axis=\"z\"", "amount=\"nan\"", "phase=\"x\""};
    for (size_t i = 0; i < 3; ++i) {
        char xml[768];
        snprintf(xml, sizeof xml, HEAD
                 "<shape id=\"s\" shape=\"rect\" width=\"40\" height=\"40\">"
                 "<deform><modifier type=\"mesh-warp\" rows=\"3\" cols=\"3\" %s/></deform>"
                 "</shape></composition></scene>", bad[i]);
        SrScene scene;
        char *message = NULL;
        CHECK(t, st_load(t, "warp-malformed.xml", xml, &scene, &message) == SR_ERR_XML);
        CHECK_CONTAINS(t, message, "<modifier>");
        free(message);
    }
}

const sr_test_case sr_tests_deform[] = {
    {"identity_grid_bit_exact", test_identity_grid_bit_exact},
    {"moved_point_displaces", test_moved_point_displaces},
    {"sampler_inverts_warp", test_sampler_inverts_warp},
    {"point_validation", test_point_validation},
    {"inverse_newton_cycle_falls_back", test_inverse_newton_cycle_falls_back},
    {"composed_grids_pad_by_sum", test_composed_grids_pad_by_sum},
    {"malformed_mesh_warp_fails_cleanly", test_malformed_mesh_warp_fails_cleanly},
    {NULL, NULL},
};
