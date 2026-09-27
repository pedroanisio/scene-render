/* SPDX-License-Identifier: Apache-2.0 */
/* B1-4 shape constructors and the shared contour arena. Geometry rules are
 * in docs/design/b1-4-shapes-paints.md ("Geometry"). */
#include "vector_shape_internal.h"

#include <math.h>
#include <stdlib.h>
#include <string.h>


typedef SrPathPoint Point;

void sr_polyset_init(SrPolySet *set, SrCompositeResources *resources,
                     size_t limit) {
    *set = (SrPolySet){.resources = resources, .limit = limit};
}

void sr_polyset_free(SrPolySet *set) {
    if (!set) return;
    sr_composite_free(set->resources, set->points);
    sr_composite_free(set->resources, set->ranges);
    sr_composite_free(set->resources, set->contours);
    SrCompositeResources *resources = set->resources;
    size_t limit = set->limit;
    *set = (SrPolySet){.resources = resources, .limit = limit};
}

bool sr_polyset_fail(SrPolySet *set, SrStatus status, const char *error) {
    if (set->status == SR_OK) {
        set->status = status;
        set->error = error;
    }
    return false;
}

static bool allocation_failed(SrPolySet *set) {
    return sr_polyset_fail(set, set->resources
        ? sr_composite_resource_status(set->resources) : SR_ERR_MEMORY,
        NULL);
}

bool sr_polyset_begin(SrPolySet *set) {
    if (set->status != SR_OK) return false;
    if (set->open) return sr_polyset_fail(set, SR_ERR_ARGUMENT, "contour already open");
    if (set->range_count == set->range_capacity) {
        size_t capacity = set->range_capacity ? set->range_capacity * 2 : 8;
        if (capacity > SR_MAX_SHAPE_VERTICES)
            return sr_polyset_fail(set, SR_ERR_RENDER,
                                   "shape geometry exceeds 4194304 contours");
        SrPolyRange *ranges = sr_composite_realloc(set->resources, set->ranges,
            capacity, sizeof(*ranges), 0);
        if (!ranges) return allocation_failed(set);
        set->ranges = ranges;
        set->range_capacity = capacity;
    }
    set->ranges[set->range_count] = (SrPolyRange){set->point_count, 0, false};
    set->open = ++set->range_count;
    return true;
}

bool sr_polyset_add(SrPolySet *set, Point point) {
    if (set->status != SR_OK) return false;
    if (!set->open) return sr_polyset_fail(set, SR_ERR_ARGUMENT, "no open contour");
    if (!isfinite(point.x) || !isfinite(point.y) ||
        fabs(point.x) > SR_MAX_SHAPE_COORDINATE ||
        fabs(point.y) > SR_MAX_SHAPE_COORDINATE)
        return sr_polyset_fail(set, SR_ERR_RENDER,
                               "shape geometry must be finite and within 1e9");
    SrPolyRange *range = &set->ranges[set->open - 1];
    if (range->count) {
        Point last = set->points[range->offset + range->count - 1];
        if (last.x == point.x && last.y == point.y) return true;
    }
    if (set->point_count >= set->limit)
        return sr_polyset_fail(set, SR_ERR_RENDER,
                               "shape geometry exceeds 4194304 vertices");
    if (set->point_count == set->point_capacity) {
        size_t capacity = set->point_capacity ? set->point_capacity * 2 : 64;
        if (capacity > set->limit) capacity = set->limit;
        /* 16 units per new slot: every constructor, trim, dash and stroker
         * step that fills it is constant work per point. */
        if (!sr_composite_work(set->resources, capacity - set->point_capacity, 16))
            return allocation_failed(set);
        Point *points = sr_composite_realloc(set->resources, set->points,
            capacity, sizeof(*points), 0);
        if (!points) return allocation_failed(set);
        set->points = points;
        set->point_capacity = capacity;
    }
    set->points[set->point_count++] = point;
    ++range->count;
    return true;
}

bool sr_polyset_end(SrPolySet *set, bool closed) {
    if (set->status != SR_OK) return false;
    if (!set->open) return sr_polyset_fail(set, SR_ERR_ARGUMENT, "no open contour");
    SrPolyRange *range = &set->ranges[set->open - 1];
    set->open = 0;
    if (closed && range->count > 1) {
        Point first = set->points[range->offset];
        Point last = set->points[range->offset + range->count - 1];
        if (first.x == last.x && first.y == last.y) {
            --range->count;
            --set->point_count;
        }
    }
    range->closed = closed;
    if (!range->count) --set->range_count;
    return true;
}

bool sr_polyset_seal(SrPolySet *set) {
    if (set->status != SR_OK) return false;
    if (set->open) return sr_polyset_fail(set, SR_ERR_ARGUMENT, "contour left open");
    if (!set->range_count) return true;
    SrPathContour *contours = sr_composite_realloc(set->resources, set->contours,
        set->range_count, sizeof(*contours), 0);
    if (!contours) return allocation_failed(set);
    set->contours = contours;
    for (size_t i = 0; i < set->range_count; ++i)
        contours[i] = (SrPathContour){set->points + set->ranges[i].offset,
                                      set->ranges[i].count, set->ranges[i].count,
                                      set->ranges[i].closed};
    return true;
}

/* ---- flattening --------------------------------------------------------- */

size_t sr_shape_arc_pieces(double radius, double sweep, double scale) {
    double turn = fabs(sweep);
    size_t quarters = (size_t)ceil(turn / (SR_PI * 0.5) - 1e-12);
    if (quarters < 1) quarters = 1;
    size_t maximum = quarters * SR_MAX_CURVE_PIECES;
    double pixels = radius * scale;
    if (!(pixels > 0.0) || !(turn > 0.0)) return 1;
    double ratio = SR_SHAPE_TOLERANCE / pixels;
    double step = ratio >= 2.0 ? 2.0 * SR_PI : 2.0 * acos(1.0 - ratio);
    double pieces = ceil(turn / step);
    if (!(pieces >= 1.0)) return 1;
    return pieces > (double)maximum ? maximum : (size_t)pieces;
}

size_t sr_shape_cubic_pieces(const Point p[4], double scale) {
    double m = 0.0;
    for (int i = 0; i < 2; ++i) {
        double x = p[i].x - 2.0 * p[i + 1].x + p[i + 2].x;
        double y = p[i].y - 2.0 * p[i + 1].y + p[i + 2].y;
        double length = hypot(x, y);
        if (length > m) m = length;
    }
    double pieces = ceil(sqrt(0.75 * m * scale / SR_SHAPE_TOLERANCE));
    if (!(pieces >= 1.0)) return 1;
    return pieces > (double)SR_MAX_CURVE_PIECES ? SR_MAX_CURVE_PIECES : (size_t)pieces;
}

bool sr_shape_arc(SrPolySet *set, Point centre, double radius, double a0,
                  double sweep, double scale) {
    size_t pieces = sr_shape_arc_pieces(radius, sweep, scale);
    for (size_t i = 1; i <= pieces; ++i) {
        double a = a0 + sweep * ((double)i / (double)pieces);
        if (!sr_polyset_add(set, (Point){centre.x + radius * cos(a),
                                         centre.y + radius * sin(a)}))
            return false;
    }
    return true;
}

bool sr_shape_cubic(SrPolySet *set, const Point p[4], double scale) {
    size_t pieces = sr_shape_cubic_pieces(p, scale);
    for (size_t i = 1; i <= pieces; ++i) {
        double t = (double)i / (double)pieces, u = 1.0 - t;
        Point q = {u * u * u * p[0].x + 3 * u * u * t * p[1].x +
                       3 * u * t * t * p[2].x + t * t * t * p[3].x,
                   u * u * u * p[0].y + 3 * u * u * t * p[1].y +
                       3 * u * t * t * p[2].y + t * t * t * p[3].y};
        if (i == pieces) q = p[3];
        if (!sr_polyset_add(set, q)) return false;
    }
    return true;
}

/* ---- constructors ------------------------------------------------------- */

void sr_shape_corner_radii(double width, double height, double radii[4]) {
    for (int i = 0; i < 4; ++i)
        radii[i] = radii[i] > 0.0 && isfinite(radii[i]) ? radii[i] : 0.0;
    double f = 1.0;
    const double sums[4] = {radii[0] + radii[1], radii[3] + radii[2],
                            radii[0] + radii[3], radii[1] + radii[2]};
    const double sides[4] = {width, width, height, height};
    for (int i = 0; i < 4; ++i)
        if (sums[i] > 0.0 && sides[i] / sums[i] < f) f = sides[i] / sums[i];
    if (f < 1.0)
        for (int i = 0; i < 4; ++i) radii[i] *= f;
}

static bool corner(SrPolySet *set, Point centre, double radius, double a0,
                   double scale) {
    if (!(radius > 0.0))
        return sr_polyset_add(set, (Point){centre.x, centre.y});
    if (!sr_polyset_add(set, (Point){centre.x + radius * cos(a0),
                                     centre.y + radius * sin(a0)}))
        return false;
    return sr_shape_arc(set, centre, radius, a0, SR_PI * 0.5, scale);
}

static bool build_rect(SrPolySet *set, const SrShapeParams *p) {
    double w = p->width, h = p->height, r[4];
    memcpy(r, p->radii, sizeof(r));
    sr_shape_corner_radii(w, h, r);
    if (!sr_polyset_begin(set)) return false;
    /* Clockwise in y-down from the top edge: TR, BR, BL, TL corners. */
    bool ok = sr_polyset_add(set, (Point){r[0], 0.0}) &&
        corner(set, (Point){w - r[1], r[1]}, r[1], -SR_PI * 0.5, p->scale) &&
        corner(set, (Point){w - r[2], h - r[2]}, r[2], 0.0, p->scale) &&
        corner(set, (Point){r[3], h - r[3]}, r[3], SR_PI * 0.5, p->scale) &&
        corner(set, (Point){r[0], r[0]}, r[0], SR_PI, p->scale);
    return ok && sr_polyset_end(set, true);
}

static bool build_ellipse(SrPolySet *set, const SrShapeParams *p) {
    double rx = p->width * 0.5, ry = p->height * 0.5;
    size_t quarter = sr_shape_arc_pieces(rx > ry ? rx : ry, SR_PI * 0.5, p->scale);
    size_t n = 4 * quarter;
    if (!sr_polyset_begin(set)) return false;
    for (size_t k = 0; k < n; ++k) {
        double a = -SR_PI * 0.5 + 2.0 * SR_PI * ((double)k / (double)n);
        if (!sr_polyset_add(set, (Point){rx + rx * cos(a), ry + ry * sin(a)}))
            return false;
    }
    return sr_polyset_end(set, true);
}

Point sr_shape_vertex(double cx, double cy, double radius, size_t index,
                      size_t count) {
    double a = -SR_PI * 0.5 + 2.0 * SR_PI * ((double)index / (double)count);
    return (Point){cx + radius * cos(a), cy + radius * sin(a)};
}

static bool build_star(SrPolySet *set, const SrShapeParams *p, bool star) {
    size_t points = p->points, n = star ? 2 * points : points;
    double cx = p->width * 0.5, cy = p->height * 0.5;
    double outer = p->outer_radius, inner = p->inner_radius;
    double outer_round = p->outer_roundness, inner_round = star ? p->inner_roundness : 0.0;
    bool round = outer_round > 0.0 || inner_round > 0.0;
    if (!sr_polyset_begin(set)) return false;
    Point first = {0.0, 0.0};
    Point previous_handle = {0.0, 0.0};
    Point previous = {0.0, 0.0};
    for (size_t k = 0; k <= n; ++k) {
        size_t index = k % n;
        bool is_outer = !star || index % 2 == 0;
        double radius = is_outer ? outer : inner;
        Point v = sr_shape_vertex(cx, cy, radius, index, n);
        if (!round) {
            if (k < n && !sr_polyset_add(set, v)) return false;
            continue;
        }
        /* lottie-web star/polygon roundness: tangent handles perpendicular
         * to the radius, length roundness * 2 pi rho / (4 points). */
        double a = -SR_PI * 0.5 + 2.0 * SR_PI * ((double)index / (double)n);
        double length = (is_outer ? outer_round : inner_round) *
                        (2.0 * SR_PI * radius) / (4.0 * (double)points);
        Point tangent = {-sin(a), cos(a)};
        Point in = {v.x - tangent.x * length, v.y - tangent.y * length};
        Point out = {v.x + tangent.x * length, v.y + tangent.y * length};
        if (k == 0) {
            first = v;
            if (!sr_polyset_add(set, v)) return false;
        } else {
            Point curve[4] = {previous, previous_handle, in, k == n ? first : v};
            if (!sr_shape_cubic(set, curve, p->scale)) return false;
        }
        previous = v;
        previous_handle = out;
    }
    return sr_polyset_end(set, true);
}

static bool build_path(SrPolySet *set, const SrShapeParams *p) {
    if (!p->path || !p->path->count)
        return sr_polyset_fail(set, SR_ERR_ARGUMENT, "shape path is not prepared");
    for (size_t c = 0; c < p->path->count; ++c) {
        const SrPathContour *contour = &p->path->items[c];
        if (!sr_polyset_begin(set)) return false;
        for (size_t i = 0; i < contour->count; ++i)
            if (!sr_polyset_add(set, contour->points[i])) return false;
        if (!sr_polyset_end(set, contour->closed)) return false;
    }
    return true;
}

bool sr_shape_build(SrPolySet *set, const SrShapeParams *p) {
    if (!(p->width > 0.0) || !(p->height > 0.0) || !isfinite(p->width) ||
        !isfinite(p->height) || p->width > SR_MAX_SHAPE_COORDINATE ||
        p->height > SR_MAX_SHAPE_COORDINATE || !(p->scale > 0.0) ||
        !isfinite(p->scale))
        return sr_polyset_fail(set, SR_ERR_RENDER,
                               "shape size must be positive and within 1e9");
    switch (p->type) {
    case SR_SHAPE_RECT:
    case SR_SHAPE_ROUNDED_RECT:
        return build_rect(set, p);
    case SR_SHAPE_ELLIPSE:
        return build_ellipse(set, p);
    case SR_SHAPE_POLYGON:
    case SR_SHAPE_STAR:
        if (p->points < 3 || p->points > SR_MAX_SHAPE_POINTS)
            return sr_polyset_fail(set, SR_ERR_RENDER,
                                   "points must be in [3, 4096]");
        return build_star(set, p, p->type == SR_SHAPE_STAR);
    case SR_SHAPE_LINE:
        return sr_polyset_begin(set) &&
            sr_polyset_add(set, (Point){0.0, p->height * 0.5}) &&
            sr_polyset_add(set, (Point){p->width, p->height * 0.5}) &&
            sr_polyset_end(set, false);
    case SR_SHAPE_PATH:
        return build_path(set, p);
    }
    return sr_polyset_fail(set, SR_ERR_ARGUMENT, "unknown shape type");
}
