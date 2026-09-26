/* SPDX-License-Identifier: Apache-2.0 */
/* B1-4 geometry: arc length, coverage clipping, mapped rasterization, shape
 * constructors, trim, dash and the stroker, against closed-form areas and
 * positions (docs/design/b1-4-shapes-paints.md). */
#include "vector_shape_internal.h"

#include <stdlib.h>

#include "harness.h"

#define GRID 96

static float cells[(GRID + 2) * GRID], coverage[GRID * GRID];

static double sum(const float *values, size_t count) {
    double total = 0.0;
    for (size_t i = 0; i < count; ++i) total += values[i];
    return total;
}

static const SrPathMap identity = {{1, 0, 0, 0, 1, 0}};

/* Area of a sealed polygon set, nonzero, on the GRID x GRID raster. */
static double area(const SrPolySet *set, SrFillRule rule) {
    if (!set->range_count) return 0.0;
    if (sr_path_rasterize(set->contours, set->range_count, &identity, rule,
                          GRID, GRID, cells, coverage) != SR_OK)
        return -1.0;
    return sum(coverage, GRID * GRID);
}

static SrStrokeParams params(double width) {
    return (SrStrokeParams){.width = width, .cap = SR_LINE_CAP_BUTT,
                            .join = SR_LINE_JOIN_MITER, .miter_limit = 4.0,
                            .scale = 1.0};
}

static double stroke_area(const SrPathPoint *points, size_t count, bool closed,
                          const SrStrokeParams *p) {
    SrPolySet out;
    sr_polyset_init(&out, NULL, SR_MAX_SHAPE_VERTICES);
    double result = -1.0;
    if (sr_stroke_polyline(&out, points, count, closed, (SrPathPoint){1, 0}, p) &&
        sr_polyset_seal(&out))
        result = area(&out, SR_FILL_NONZERO);
    sr_polyset_free(&out);
    return result;
}

static void arc_length_and_extraction(sr_test_ctx *t) {
    SrPathPoint square[4] = {{0, 0}, {10, 0}, {10, 20}, {0, 20}};
    SrPathContour contour = {square, 4, 4, true};
    double cumulative[5];
    CHECK_INT(t, sr_path_contour_segments(&contour), 4);
    CHECK_NEAR(t, sr_path_contour_measure(&contour, cumulative), 60.0, 0.0);
    CHECK_NEAR(t, cumulative[1], 10.0, 0.0);
    CHECK_NEAR(t, cumulative[2], 30.0, 0.0);
    SrPathPoint out[6], tangent;
    size_t count;
    sr_path_contour_extract(&contour, cumulative, 5.0, 25.0, out, &count, &tangent);
    CHECK_INT(t, count, 3);
    CHECK_NEAR(t, out[0].x, 5.0, 0.0);
    CHECK_NEAR(t, out[1].x, 10.0, 0.0);
    CHECK_NEAR(t, out[2].y, 15.0, 0.0);
    CHECK_NEAR(t, tangent.x, 1.0, 0.0);
    /* The closing segment ends at the first point. */
    sr_path_contour_extract(&contour, cumulative, 55.0, 60.0, out, &count, &tangent);
    CHECK_INT(t, count, 2);
    CHECK_NEAR(t, out[1].x, 0.0, 0.0);
    CHECK_NEAR(t, out[1].y, 0.0, 0.0);
    CHECK_NEAR(t, tangent.y, -1.0, 0.0);
    /* An exact vertex boundary ends on that vertex. */
    sr_path_contour_extract(&contour, cumulative, 0.0, 10.0, out, &count, &tangent);
    CHECK_INT(t, count, 2);
    CHECK_NEAR(t, out[1].x, 10.0, 0.0);
    /* Open contour and clamping. */
    SrPathContour open = {square, 3, 3, false};
    CHECK_NEAR(t, sr_path_contour_measure(&open, cumulative), 30.0, 0.0);
    sr_path_contour_extract(&open, cumulative, -5.0, 99.0, out, &count, &tangent);
    CHECK_INT(t, count, 3);
    CHECK_NEAR(t, out[2].y, 20.0, 0.0);
    /* Zero-length contour: one point, no tangent. */
    SrPathPoint same[2] = {{3, 3}, {3, 3}};
    SrPathContour dot = {same, 2, 2, false};
    CHECK_NEAR(t, sr_path_contour_measure(&dot, cumulative), 0.0, 0.0);
    sr_path_contour_extract(&dot, cumulative, 0.0, 0.0, out, &count, &tangent);
    CHECK(t, tangent.x == 0.0 && tangent.y == 0.0);
}

static void coverage_clip_and_mapped_raster(sr_test_ctx *t) {
    float stroke[4] = {1.0f, 0.5f, 1.0f, 0.25f};
    const float fill[4] = {1.0f, 1.0f, 0.0f, 0.5f};
    float inside[4], outside[4];
    memcpy(inside, stroke, sizeof(stroke));
    memcpy(outside, stroke, sizeof(stroke));
    sr_coverage_clip(inside, fill, 4, true);
    sr_coverage_clip(outside, fill, 4, false);
    CHECK_NEAR(t, inside[0], 1.0, 0.0);
    CHECK_NEAR(t, inside[2], 0.0, 0.0);
    CHECK_NEAR(t, outside[2], 1.0, 0.0);
    CHECK_NEAR(t, inside[3] + outside[3], stroke[3], 1e-7);
    /* A 5 x 2 rectangle at quarter-pixel offsets, then scaled by 2 and
     * rotated 90 degrees: the exact-area rasterizer keeps the area. */
    SrPathPoint rect[4] = {{2.25, 1.5}, {7.25, 1.5}, {7.25, 3.5}, {2.25, 3.5}};
    SrPathContour contour = {rect, 4, 4, true};
    CHECK_INT(t, sr_path_rasterize(&contour, 1, &identity, SR_FILL_NONZERO, GRID,
                                   GRID, cells, coverage), SR_OK);
    CHECK_NEAR(t, sum(coverage, GRID * GRID), 10.0, 1e-4);
    SrPathMap map = {{0, -2, 40, 2, 0, 10}};
    CHECK_INT(t, sr_path_rasterize(&contour, 1, &map, SR_FILL_EVENODD, GRID, GRID,
                                   cells, coverage), SR_OK);
    CHECK_NEAR(t, sum(coverage, GRID * GRID), 40.0, 1e-3);
    CHECK(t, sr_path_raster_work(&contour, 1, &map, GRID, GRID) >= 4 * 8);
    SrPathMap bad = {{NAN, 0, 0, 0, 1, 0}};
    CHECK_INT(t, sr_path_rasterize(&contour, 1, &bad, SR_FILL_NONZERO, GRID, GRID,
                                   cells, coverage), SR_ERR_ARGUMENT);
}

static void caps_and_joins_match_closed_forms(sr_test_ctx *t) {
    SrPathPoint segment[2] = {{10, 20}, {50, 20}};
    SrStrokeParams p = params(8.0);
    CHECK_NEAR(t, stroke_area(segment, 2, false, &p), 320.0, 1e-2);
    p.cap = SR_LINE_CAP_SQUARE;
    CHECK_NEAR(t, stroke_area(segment, 2, false, &p), 384.0, 1e-2);
    p.cap = SR_LINE_CAP_ROUND;
    CHECK_NEAR(t, stroke_area(segment, 2, false, &p), 320.0 + 16.0 * SR_PI, 0.2);
    SrPathPoint corner[3] = {{10, 10}, {40, 10}, {40, 40}};
    p = params(8.0);
    CHECK_NEAR(t, stroke_area(corner, 3, false, &p), 480.0, 1e-2);
    p.join = SR_LINE_JOIN_BEVEL;
    CHECK_NEAR(t, stroke_area(corner, 3, false, &p), 472.0, 1e-2);
    p.join = SR_LINE_JOIN_ROUND;
    CHECK_NEAR(t, stroke_area(corner, 3, false, &p), 464.0 + 4.0 * SR_PI, 0.1);
    /* A right angle has miter ratio sqrt(2). */
    p.join = SR_LINE_JOIN_MITER;
    p.miter_limit = 1.4;
    CHECK_NEAR(t, stroke_area(corner, 3, false, &p), 472.0, 1e-2);
    p.miter_limit = 1.42;
    CHECK_NEAR(t, stroke_area(corner, 3, false, &p), 480.0, 1e-2);
    /* The same corner traversed backwards (inner side swapped). */
    SrPathPoint back[3] = {{40, 40}, {40, 10}, {10, 10}};
    CHECK_NEAR(t, stroke_area(back, 3, false, &p), 480.0, 1e-2);
    /* Closed square ring: outer 44^2 minus inner 36^2. */
    SrPathPoint square[4] = {{10, 10}, {50, 10}, {50, 50}, {10, 50}};
    p = params(4.0);
    CHECK_NEAR(t, stroke_area(square, 4, true, &p), 1936.0 - 1296.0, 1e-2);
    p.join = SR_LINE_JOIN_BEVEL;
    CHECK_NEAR(t, stroke_area(square, 4, true, &p), 640.0 - 4 * 2.0, 1e-2);
    /* Degenerate pieces: round dot, square dot along the tangent, butt none. */
    SrPathPoint dot[2] = {{30, 30}, {30, 30}};
    p = params(6.0);
    CHECK_NEAR(t, stroke_area(dot, 2, false, &p), 0.0, 0.0);
    p.cap = SR_LINE_CAP_SQUARE;
    CHECK_NEAR(t, stroke_area(dot, 2, false, &p), 36.0, 1e-3);
    p.cap = SR_LINE_CAP_ROUND;
    /* Inscribed flattening loses at most (2/3) tau per unit of arc. */
    CHECK_NEAR(t, stroke_area(dot, 2, false, &p), 9.0 * SR_PI, 0.15);
    /* A 180-degree turn retraces the outline: with a bevel on pixel edges
     * the area is exact; a round tip may double-count its partially covered
     * boundary pixels (the documented accumulator limitation), bounded by
     * the tip's perimeter. */
    SrPathPoint hairpin[3] = {{20, 30}, {60, 30}, {20, 30}};
    p = params(6.0);
    p.join = SR_LINE_JOIN_BEVEL;
    CHECK_NEAR(t, stroke_area(hairpin, 3, false, &p), 240.0, 1e-3);
    p.join = SR_LINE_JOIN_ROUND;
    double tip = stroke_area(hairpin, 3, false, &p);
    CHECK(t, tip >= 240.0 + 4.5 * SR_PI - 0.2 && tip <= 240.0 + 4.5 * SR_PI + 3.0 * SR_PI);
}

static void trim_intervals(sr_test_ctx *t) {
    const double lengths[2] = {100.0, 50.0};
    const bool closed[2] = {true, false};
    SrStrokePiece pieces[6];
    SrStrokeParams p = params(1.0);
    CHECK_INT(t, sr_stroke_trim(lengths, closed, 2, &p, pieces), 2);
    CHECK(t, pieces[0].whole && !pieces[1].whole);
    p.trimmed = true;
    p.trim_start = 0.2;
    p.trim_end = 0.6;
    CHECK_INT(t, sr_stroke_trim(lengths, closed, 2, &p, pieces), 2);
    CHECK_NEAR(t, pieces[0].start, 20.0, 1e-12);
    CHECK_NEAR(t, pieces[0].end, 60.0, 1e-12);
    CHECK_NEAR(t, pieces[1].end, 30.0, 1e-12);
    /* Swapped ends and an offset that wraps: closed joins, open splits. */
    p.trim_start = 0.6;
    p.trim_end = 0.2;
    p.trim_offset = 0.7;
    CHECK_INT(t, sr_stroke_trim(lengths, closed, 2, &p, pieces), 3);
    CHECK(t, pieces[0].wrap);
    CHECK_NEAR(t, pieces[0].start, 90.0, 1e-9);
    CHECK_NEAR(t, pieces[0].wrap_end, 30.0, 1e-9);
    CHECK_NEAR(t, pieces[1].start, 45.0, 1e-9);
    CHECK_NEAR(t, pieces[2].end, 15.0, 1e-9);
    /* Negative offsets use floor-based fractions. */
    p.trim_start = 0.0;
    p.trim_end = 0.5;
    p.trim_offset = -0.25;
    CHECK_INT(t, sr_stroke_trim(lengths, closed, 1, &p, pieces), 1);
    CHECK_NEAR(t, pieces[0].start, 75.0, 1e-9);
    CHECK(t, pieces[0].wrap);
    /* Sequential: one parameter over 150 units. */
    p.trim_mode = SR_TRIM_SEQUENTIAL;
    p.trim_start = 0.5;
    p.trim_end = 0.9;
    p.trim_offset = 0.0;
    CHECK_INT(t, sr_stroke_trim(lengths, closed, 2, &p, pieces), 2);
    CHECK_NEAR(t, pieces[0].start, 75.0, 1e-9);
    CHECK_NEAR(t, pieces[0].end, 100.0, 1e-9);
    CHECK_NEAR(t, pieces[1].start, 0.0, 1e-9);
    CHECK_NEAR(t, pieces[1].end, 35.0, 1e-9);
    /* Empty and full spans. */
    p.trim_start = p.trim_end = 0.3;
    CHECK_INT(t, sr_stroke_trim(lengths, closed, 2, &p, pieces), 0);
    p.trim_start = 0.0;
    p.trim_end = 1.0;
    CHECK_INT(t, sr_stroke_trim(lengths, closed, 2, &p, pieces), 2);
    CHECK(t, pieces[0].whole);
}

static size_t build_stroke(const SrPolySet *source, const SrStrokeParams *p,
                           double *out_area) {
    SrPolySet out;
    sr_polyset_init(&out, NULL, SR_MAX_SHAPE_VERTICES);
    size_t ranges = SIZE_MAX;
    if (sr_stroke_build(source, p, &out) && sr_polyset_seal(&out)) {
        ranges = out.range_count;
        *out_area = area(&out, SR_FILL_NONZERO);
    }
    sr_polyset_free(&out);
    return ranges;
}

static void dash_and_trim_strokes(sr_test_ctx *t) {
    SrPolySet line;
    sr_polyset_init(&line, NULL, 64);
    CHECK(t, sr_polyset_begin(&line) && sr_polyset_add(&line, (SrPathPoint){10, 40}) &&
             sr_polyset_add(&line, (SrPathPoint){50, 40}) && sr_polyset_end(&line, false) &&
             sr_polyset_seal(&line));
    const double dash[2] = {10.0, 10.0};
    SrStrokeParams p = params(2.0);
    p.dash = dash;
    p.dash_count = 2;
    double a = 0.0;
    CHECK_INT(t, build_stroke(&line, &p, &a), 2);
    CHECK_NEAR(t, a, 40.0, 1e-3);
    p.dash_offset = 5.0;              /* [0,5], [15,25], [35,40] */
    CHECK_INT(t, build_stroke(&line, &p, &a), 3);
    CHECK_NEAR(t, a, 40.0, 1e-3);
    p.dash_offset = -15.0;            /* same phase, floor-based */
    CHECK_INT(t, build_stroke(&line, &p, &a), 3);
    /* Dashes stay fixed along the contour while the trim moves. */
    p.dash_offset = 0.0;
    p.trimmed = true;
    p.trim_start = 0.125;             /* [5, 40]: dashes [5,10], [20,30] */
    p.trim_end = 1.0;
    CHECK_INT(t, build_stroke(&line, &p, &a), 2);
    CHECK_NEAR(t, a, 2.0 * 15.0, 1e-3);
    /* Zero-length dashes are round dots. */
    const double dots[2] = {0.0, 10.0};
    p = params(4.0);
    p.cap = SR_LINE_CAP_ROUND;
    p.dash = dots;
    p.dash_count = 2;
    CHECK_INT(t, build_stroke(&line, &p, &a), 4);
    CHECK_NEAR(t, a, 4.0 * 4.0 * SR_PI, 0.5);
    /* Excessive dash pieces fail with a limit error. */
    const double tiny[2] = {1e-6, 1e-6};
    p.dash = tiny;
    SrPolySet out;
    sr_polyset_init(&out, NULL, SR_MAX_SHAPE_VERTICES);
    CHECK(t, !sr_stroke_build(&line, &p, &out));
    CHECK_INT(t, out.status, SR_ERR_RENDER);
    CHECK_CONTAINS(t, out.error, "1048576");
    sr_polyset_free(&out);
    sr_polyset_free(&line);
    /* A wrapped trim of a closed square is one continuous piece whose join
     * through the start vertex is mitred: a 40 x 40 square, width 2, from
     * 7/8 to 1/8 of the perimeter: two 20-unit edges around a corner. */
    SrPolySet square;
    sr_polyset_init(&square, NULL, 64);
    CHECK(t, sr_polyset_begin(&square));
    const SrPathPoint corners[4] = {{10, 10}, {50, 10}, {50, 50}, {10, 50}};
    for (int i = 0; i < 4; ++i) CHECK(t, sr_polyset_add(&square, corners[i]));
    CHECK(t, sr_polyset_end(&square, true) && sr_polyset_seal(&square));
    p = params(2.0);
    p.trimmed = true;
    p.trim_start = 0.0;
    p.trim_end = 0.25;
    p.trim_offset = 0.875;
    CHECK_INT(t, build_stroke(&square, &p, &a), 1);
    /* Two 20 x 2 bands overlapping in one unit, plus the unit miter corner. */
    CHECK_NEAR(t, a, 80.0, 1e-3);
    sr_polyset_free(&square);
}

/* Review regressions: dots and dashes share one winding; dashes crossing a
 * closed seam stay joined; a dash that cannot advance fails explicitly. */
static void dash_seams_winding_and_progress(sr_test_ctx *t) {
    SrPolySet line;
    sr_polyset_init(&line, NULL, 64);
    CHECK(t, sr_polyset_begin(&line) && sr_polyset_add(&line, (SrPathPoint){10, 40}) &&
             sr_polyset_add(&line, (SrPathPoint){60, 40}) && sr_polyset_end(&line, false) &&
             sr_polyset_seal(&line));
    const double mixed[4] = {0.0, 1.0, 10.0, 1.0};
    SrStrokeParams p = params(4.0);
    p.cap = SR_LINE_CAP_ROUND;
    p.dash = mixed;
    p.dash_count = 4;
    SrPolySet out;
    sr_polyset_init(&out, NULL, SR_MAX_SHAPE_VERTICES);
    CHECK(t, sr_stroke_build(&line, &p, &out) && sr_polyset_seal(&out));
    area(&out, SR_FILL_NONZERO);
    for (int x = 11; x < 58; ++x)   /* the centre row stays fully covered */
        CHECK_NEAR(t, coverage[39 * GRID + x], 1.0, 1e-5);
    sr_polyset_free(&out);
    /* No progress in double precision fails instead of drawing nothing. */
    SrPolySet far;
    sr_polyset_init(&far, NULL, 64);
    CHECK(t, sr_polyset_begin(&far) && sr_polyset_add(&far, (SrPathPoint){0, 0}) &&
             sr_polyset_add(&far, (SrPathPoint){1e9, 0}) && sr_polyset_end(&far, false) &&
             sr_polyset_seal(&far));
    const double tiny[2] = {4e-8, 4e-8};
    p = params(1.0);
    p.dash = tiny;
    p.dash_count = 2;
    p.trimmed = true;
    p.trim_start = 0.9999999999999999;
    p.trim_end = 1.0;
    sr_polyset_init(&out, NULL, SR_MAX_SHAPE_VERTICES);
    CHECK(t, !sr_stroke_build(&far, &p, &out));
    CHECK_INT(t, out.status, SR_ERR_RENDER);
    sr_polyset_free(&out);
    sr_polyset_free(&far);
    sr_polyset_free(&line);
    /* One dash longer than a closed square is the closed stroke. */
    const SrPathPoint corners[4] = {{10, 10}, {50, 10}, {50, 50}, {10, 50}};
    SrPolySet square;
    sr_polyset_init(&square, NULL, 64);
    CHECK(t, sr_polyset_begin(&square));
    for (int i = 0; i < 4; ++i) CHECK(t, sr_polyset_add(&square, corners[i]));
    CHECK(t, sr_polyset_end(&square, true) && sr_polyset_seal(&square));
    const double long_dash[2] = {1000.0, 1.0};
    p = params(4.0);
    p.dash = long_dash;
    p.dash_count = 2;
    double a = 0.0;
    CHECK_INT(t, build_stroke(&square, &p, &a), 2);
    CHECK_NEAR(t, a, 1936.0 - 1296.0, 1e-2);
    /* A dash crossing the seam equals the same dash on a contour that starts
     * at another corner (pattern shifted by that corner's arc length; the
     * 32-unit period divides the 160-unit perimeter, so the pattern itself is
     * continuous across both seams). */
    const double pattern[2] = {20.0, 12.0};
    p.dash = pattern;
    p.dash_offset = 10.0;
    double seam_area = 0.0, rotated_area = 0.0;
    size_t seam_pieces = build_stroke(&square, &p, &seam_area);
    SrPolySet rotated;
    sr_polyset_init(&rotated, NULL, 64);
    CHECK(t, sr_polyset_begin(&rotated));
    for (int i = 0; i < 4; ++i) CHECK(t, sr_polyset_add(&rotated, corners[(i + 1) % 4]));
    CHECK(t, sr_polyset_end(&rotated, true) && sr_polyset_seal(&rotated));
    p.dash_offset = 50.0;
    size_t rotated_pieces = build_stroke(&rotated, &p, &rotated_area);
    CHECK_INT(t, seam_pieces, rotated_pieces);
    CHECK_NEAR(t, seam_area, rotated_area, 1e-3);
    sr_polyset_free(&rotated);
    sr_polyset_free(&square);
}

static double polyset_area(const SrShapeParams *shape, SrFillRule rule) {
    SrPolySet set;
    sr_polyset_init(&set, NULL, SR_MAX_SHAPE_VERTICES);
    double result = -1.0;
    if (sr_shape_build(&set, shape) && sr_polyset_seal(&set)) result = area(&set, rule);
    sr_polyset_free(&set);
    return result;
}

static void constructors_match_closed_forms(sr_test_ctx *t) {
    SrShapeParams shape = {.type = SR_SHAPE_RECT, .width = 60, .height = 40,
                           .scale = 1.0};
    CHECK_NEAR(t, polyset_area(&shape, SR_FILL_NONZERO), 2400.0, 1e-2);
    shape.type = SR_SHAPE_ROUNDED_RECT;
    for (int i = 0; i < 4; ++i) shape.radii[i] = 10.0;
    CHECK_NEAR(t, polyset_area(&shape, SR_FILL_NONZERO),
               2400.0 - (4.0 - SR_PI) * 100.0, 0.5);
    double radii[4] = {40, 40, 40, 40};
    sr_shape_corner_radii(100, 50, radii);
    CHECK_NEAR(t, radii[0], 25.0, 1e-12);
    double mixed[4] = {-5, 30, 80, 10};
    sr_shape_corner_radii(100, 50, mixed);
    CHECK_NEAR(t, mixed[0], 0.0, 0.0);
    CHECK_NEAR(t, mixed[2], 80.0 * (50.0 / 110.0), 1e-9);
    shape.type = SR_SHAPE_ELLIPSE;
    CHECK_NEAR(t, polyset_area(&shape, SR_FILL_NONZERO), SR_PI * 30 * 20, 1.0);
    /* Regular polygon: n/2 R^2 sin(2 pi / n). */
    shape = (SrShapeParams){.type = SR_SHAPE_POLYGON, .width = 80, .height = 80,
                            .points = 6, .outer_radius = 30, .scale = 1.0};
    CHECK_NEAR(t, polyset_area(&shape, SR_FILL_NONZERO),
               3.0 * 900.0 * sin(2.0 * SR_PI / 6.0), 0.05);
    SrPathPoint top = sr_shape_vertex(40, 40, 30, 0, 6);
    CHECK_NEAR(t, top.x, 40.0, 1e-9);
    CHECK_NEAR(t, top.y, 10.0, 1e-9);
    SrPathPoint second = sr_shape_vertex(40, 40, 30, 1, 6);
    CHECK(t, second.x > 40.0 && second.y < 40.0);   /* clockwise, y-down */
    /* Star: 2n triangles of sides R, r and angle pi/n. */
    shape.type = SR_SHAPE_STAR;
    shape.points = 5;
    shape.inner_radius = 12;
    CHECK_NEAR(t, polyset_area(&shape, SR_FILL_NONZERO),
               10.0 * 0.5 * 30.0 * 12.0 * sin(SR_PI / 5.0), 0.05);
    /* Full outer roundness of a square-like polygon bulges outward; the
     * curve still passes through every vertex. */
    shape.type = SR_SHAPE_POLYGON;
    shape.points = 4;
    double sharp = polyset_area(&shape, SR_FILL_NONZERO);
    shape.outer_roundness = 1.0;
    SrPolySet set;
    sr_polyset_init(&set, NULL, SR_MAX_SHAPE_VERTICES);
    CHECK(t, sr_shape_build(&set, &shape) && sr_polyset_seal(&set));
    CHECK_NEAR(t, set.points[0].x, 40.0, 1e-9);
    CHECK_NEAR(t, set.points[0].y, 10.0, 1e-9);
    bool through = false;
    for (size_t i = 0; i < set.point_count; ++i)
        through |= fabs(set.points[i].x - 70.0) < 1e-9 && fabs(set.points[i].y - 40.0) < 1e-9;
    CHECK(t, through);
    sr_polyset_free(&set);
    CHECK(t, polyset_area(&shape, SR_FILL_NONZERO) > sharp + 100.0);
    /* Line: an open contour with no area. */
    shape = (SrShapeParams){.type = SR_SHAPE_LINE, .width = 50, .height = 10,
                            .scale = 1.0};
    CHECK_NEAR(t, polyset_area(&shape, SR_FILL_NONZERO), 0.0, 1e-9);
    /* Limits and invalid sizes. */
    shape = (SrShapeParams){.type = SR_SHAPE_POLYGON, .width = 10, .height = 10,
                            .points = 2, .outer_radius = 5, .scale = 1.0};
    CHECK_NEAR(t, polyset_area(&shape, SR_FILL_NONZERO), -1.0, 0.0);
    shape.points = 5;
    shape.width = NAN;
    CHECK_NEAR(t, polyset_area(&shape, SR_FILL_NONZERO), -1.0, 0.0);
    sr_polyset_init(&set, NULL, 3);
    CHECK(t, sr_polyset_begin(&set));
    for (int i = 0; i < 3; ++i) CHECK(t, sr_polyset_add(&set, (SrPathPoint){i, 0}));
    CHECK(t, !sr_polyset_add(&set, (SrPathPoint){9, 9}));
    CHECK_CONTAINS(t, set.error, "4194304");
    sr_polyset_free(&set);
}

static void flattening_counts(sr_test_ctx *t) {
    /* 2 acos(1 - tau / r) per piece; larger on-screen radii need more. */
    size_t small = sr_shape_arc_pieces(5.0, SR_PI, 1.0);
    size_t large = sr_shape_arc_pieces(5.0, SR_PI, 100.0);
    CHECK(t, small >= 3 && large > small);
    CHECK_INT(t, sr_shape_arc_pieces(1e12, 2.0 * SR_PI, 1.0), 4 * SR_MAX_CURVE_PIECES);
    CHECK_INT(t, sr_shape_arc_pieces(0.0, SR_PI, 1.0), 1);
    const SrPathPoint straight[4] = {{0, 0}, {1, 0}, {2, 0}, {3, 0}};
    CHECK_INT(t, sr_shape_cubic_pieces(straight, 1.0), 1);
    const SrPathPoint bent[4] = {{0, 0}, {0, 100}, {100, 100}, {100, 0}};
    size_t n = sr_shape_cubic_pieces(bent, 1.0);
    CHECK_INT(t, n, (size_t)ceil(sqrt(0.75 * hypot(100, 100) / SR_SHAPE_TOLERANCE)));
}

static void ledger_accounting(sr_test_ctx *t) {
    SrCompositeResources resources;
    sr_composite_resources_init(&resources, NULL);
    SrPolySet set;
    sr_polyset_init(&set, &resources, SR_MAX_SHAPE_VERTICES);
    SrShapeParams shape = {.type = SR_SHAPE_ELLIPSE, .width = 80, .height = 60,
                           .scale = 4.0};
    CHECK(t, sr_shape_build(&set, &shape) && sr_polyset_seal(&set));
    CHECK(t, resources.bytes > 0 && resources.work >= 16 * set.point_count);
    sr_polyset_free(&set);
    CHECK_INT(t, resources.bytes, 0);
    /* A short work quota fails with a render status, not memory. */
    SrCompositeLimits limits = {SR_MAX_COMPOSITE_BYTES, SR_MAX_COMPOSITE_PIXELS, 100};
    sr_composite_resources_init(&resources, &limits);
    sr_polyset_init(&set, &resources, SR_MAX_SHAPE_VERTICES);
    CHECK(t, !sr_shape_build(&set, &shape));
    CHECK_INT(t, set.status, SR_ERR_RENDER);
    sr_polyset_free(&set);
    CHECK_INT(t, resources.bytes, 0);
}

const sr_test_case sr_tests_shapes[] = {
    {"arc_length_and_extraction", arc_length_and_extraction},
    {"coverage_clip_and_mapped_raster", coverage_clip_and_mapped_raster},
    {"caps_and_joins_match_closed_forms", caps_and_joins_match_closed_forms},
    {"trim_intervals", trim_intervals},
    {"dash_and_trim_strokes", dash_and_trim_strokes},
    {"constructors_match_closed_forms", constructors_match_closed_forms},
    {"flattening_counts", flattening_counts},
    {"dash_seams_winding_and_progress", dash_seams_winding_and_progress},
    {"ledger_accounting", ledger_accounting},
    {NULL, NULL},
};
