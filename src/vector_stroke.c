/* SPDX-License-Identifier: Apache-2.0 */
/* B1-4 trim paths, dashes and the polygon stroker. The rules are in
 * docs/design/b1-4-shapes-paints.md ("Arc length, trim and dash",
 * "Stroker"). The stroker follows the construction of Skia's SkStroke and
 * FreeType's FT_Stroker: offset outlines with inner joins through the pivot,
 * filled with the nonzero rule. */
#include "vector_shape_internal.h"

#include <math.h>
#include <stdlib.h>
#include <string.h>

#define SR_STROKE_MERGE 1e-9

typedef SrPathPoint Point;

/* ---- trim ---------------------------------------------------------------- */

static double fraction(double value) {
    return value - floor(value);
}

static size_t add_piece(SrStrokePiece *pieces, size_t n, size_t contour,
                        double start, double end) {
    if (end > start)
        pieces[n++] = (SrStrokePiece){contour, start, end, false, 0.0, false};
    return n;
}

size_t sr_stroke_trim(const double *lengths, const bool *closed, size_t count,
                      const SrStrokeParams *params, SrStrokePiece *pieces) {
    size_t n = 0;
    double a = params->trim_start, b = params->trim_end;
    if (!params->trimmed || !(fabs(b - a) < 1.0)) {
        for (size_t c = 0; c < count; ++c)
            pieces[n++] = (SrStrokePiece){c, 0.0, lengths[c], false, 0.0,
                                          closed[c]};
        return n;
    }
    if (a > b) {
        double t = a;
        a = b;
        b = t;
    }
    double span = b - a;
    if (!(span > 0.0)) return 0;
    double u = fraction(a + params->trim_offset);
    double v = u + span;             /* in (u, u + 1) */
    if (params->trim_mode == SR_TRIM_SIMULTANEOUS || count == 1) {
        for (size_t c = 0; c < count; ++c) {
            double length = lengths[c];
            if (v <= 1.0) {
                n = add_piece(pieces, n, c, u * length, v * length);
            } else if (closed[c]) {
                pieces[n++] = (SrStrokePiece){c, u * length, length, true,
                                              (v - 1.0) * length, false};
            } else {
                n = add_piece(pieces, n, c, u * length, length);
                n = add_piece(pieces, n, c, 0.0, (v - 1.0) * length);
            }
        }
        return n;
    }
    /* Sequential: one parameter over the concatenated outline. */
    double total = 0.0;
    for (size_t c = 0; c < count; ++c) total += lengths[c];
    double ranges[2][2] = {{u, v <= 1.0 ? v : 1.0}, {0.0, v - 1.0}};
    size_t range_count = v <= 1.0 ? 1 : 2;
    for (size_t r = 0; r < range_count; ++r) {
        double from = ranges[r][0] * total, to = ranges[r][1] * total;
        double base = 0.0;
        for (size_t c = 0; c < count; ++c) {
            double start = from - base, end = to - base;
            if (start < 0.0) start = 0.0;
            if (end > lengths[c]) end = lengths[c];
            if (end > start) n = add_piece(pieces, n, c, start, end);
            base += lengths[c];
        }
    }
    return n;
}

/* ---- stroker ------------------------------------------------------------- */

static Point direction(Point a, Point b) {
    double length = hypot(b.x - a.x, b.y - a.y);
    return length > 0.0 ? (Point){(b.x - a.x) / length, (b.y - a.y) / length}
                        : (Point){0.0, 0.0};
}

static bool add(SrPolySet *out, Point p) {
    return sr_polyset_add(out, p);
}

static Point offset(Point v, Point normal, double h) {
    return (Point){v.x + normal.x * h, v.y + normal.y * h};
}

/* Join at v between incoming unit direction din and outgoing dout on the
 * left side (normal n = (-d.y, d.x)). */
static bool join(SrPolySet *out, Point v, Point din, Point dout,
                 const SrStrokeParams *params, double h) {
    Point nin = {-din.y, din.x}, nout = {-dout.y, dout.x};
    Point a = offset(v, nin, h), b = offset(v, nout, h);
    double cross = din.x * dout.y - din.y * dout.x;
    double dot = din.x * dout.x + din.y * dout.y;
    if (fabs(cross) <= 1e-12 && dot > 0.0) return add(out, a) && add(out, b);
    if (cross > 0.0)                       /* inner side: through the pivot */
        return add(out, a) && add(out, v) && add(out, b);
    if (!add(out, a)) return false;
    if (params->join == SR_LINE_JOIN_ROUND) {
        double sweep = cross == 0.0 ? -SR_PI : atan2(cross, dot);
        return sr_shape_arc(out, v, h, atan2(nin.y, nin.x), sweep, params->scale) &&
               add(out, b);
    }
    if (params->join == SR_LINE_JOIN_MITER) {
        Point m = {nin.x + nout.x, nin.y + nout.y};
        double length = hypot(m.x, m.y);
        if (length > 0.0) {
            m.x /= length;
            m.y /= length;
            double cosine = m.x * nin.x + m.y * nin.y;
            if (cosine > 0.0 && 1.0 / cosine <= params->miter_limit &&
                !add(out, offset(v, m, h / cosine)))
                return false;
        }
    }
    return add(out, b);
}

/* End cap at e with unit direction d, from the left offset to the right. */
static bool cap(SrPolySet *out, Point e, Point d, const SrStrokeParams *params,
                double h) {
    Point n = {-d.y, d.x};
    if (params->cap == SR_LINE_CAP_SQUARE)
        return add(out, (Point){e.x + (n.x + d.x) * h, e.y + (n.y + d.y) * h}) &&
               add(out, (Point){e.x + (-n.x + d.x) * h, e.y + (-n.y + d.y) * h});
    if (params->cap == SR_LINE_CAP_ROUND) {
        size_t pieces = sr_shape_arc_pieces(h, SR_PI, params->scale);
        double a0 = atan2(n.y, n.x);
        for (size_t i = 1; i < pieces; ++i) {
            double a = a0 - SR_PI * ((double)i / (double)pieces);
            if (!add(out, (Point){e.x + h * cos(a), e.y + h * sin(a)})) return false;
        }
    }
    return true;
}

static bool dot_piece(SrPolySet *out, Point p, Point tangent,
                      const SrStrokeParams *params, double h) {
    if (params->cap == SR_LINE_CAP_BUTT) return true;
    if (!sr_polyset_begin(out)) return false;
    if (params->cap == SR_LINE_CAP_ROUND) {
        size_t pieces = sr_shape_arc_pieces(h, 2.0 * SR_PI, params->scale);
        if (pieces < 4) pieces = 4;
        /* Decreasing angles: the winding sign of every other stroke
         * polygon, so overlapping pieces add instead of cancelling. */
        for (size_t i = 0; i < pieces; ++i) {
            double a = -2.0 * SR_PI * ((double)i / (double)pieces);
            if (!add(out, (Point){p.x + h * cos(a), p.y + h * sin(a)})) return false;
        }
    } else {
        Point d = tangent.x || tangent.y ? tangent : (Point){1.0, 0.0};
        Point n = {-d.y, d.x};
        const double corners[4][2] = {{1, 1}, {1, -1}, {-1, -1}, {-1, 1}};
        for (int i = 0; i < 4; ++i)
            if (!add(out, (Point){p.x + (d.x * corners[i][0] + n.x * corners[i][1]) * h,
                                  p.y + (d.y * corners[i][0] + n.y * corners[i][1]) * h}))
                return false;
    }
    return sr_polyset_end(out, true);
}

/* Left offsets of an open polyline, with interior joins. */
static bool open_side(SrPolySet *out, const Point *p, size_t n, bool reverse,
                      const SrStrokeParams *params, double h) {
#define AT(i) (reverse ? p[n - 1 - (i)] : p[(i)])
    Point d0 = direction(AT(0), AT(1));
    if (!add(out, offset(AT(0), (Point){-d0.y, d0.x}, h))) return false;
    for (size_t i = 1; i + 1 < n; ++i)
        if (!join(out, AT(i), direction(AT(i - 1), AT(i)),
                  direction(AT(i), AT(i + 1)), params, h))
            return false;
    Point dl = direction(AT(n - 2), AT(n - 1));
    if (!add(out, offset(AT(n - 1), (Point){-dl.y, dl.x}, h))) return false;
    return cap(out, AT(n - 1), dl, params, h);
#undef AT
}

static bool closed_side(SrPolySet *out, const Point *p, size_t n, bool reverse,
                        const SrStrokeParams *params, double h) {
#define AT(i) (reverse ? p[(n - (i) % n) % n] : p[(i) % n])
    if (!sr_polyset_begin(out)) return false;
    for (size_t i = 0; i < n; ++i)
        if (!join(out, AT(i), direction(AT(i + n - 1), AT(i)),
                  direction(AT(i), AT(i + 1)), params, h))
            return false;
    return sr_polyset_end(out, true);
#undef AT
}

/* Strokes the polyline in `p`, merging close points in place first. */
static bool stroke_buffer(SrPolySet *out, Point *p, size_t count, bool closed,
                          Point tangent, const SrStrokeParams *params) {
    double h = params->width * 0.5;
    if (!(h > 0.0) || !count) return true;
    size_t n = 1;
    for (size_t i = 1; i < count; ++i)
        if (hypot(p[i].x - p[n - 1].x, p[i].y - p[n - 1].y) > SR_STROKE_MERGE)
            p[n++] = p[i];
    if (closed && n > 1 &&
        hypot(p[n - 1].x - p[0].x, p[n - 1].y - p[0].y) <= SR_STROKE_MERGE)
        --n;
    if (n == 1) return dot_piece(out, p[0], tangent, params, h);
    if (closed && n > 2)
        return closed_side(out, p, n, false, params, h) &&
               closed_side(out, p, n, true, params, h);
    return sr_polyset_begin(out) && open_side(out, p, n, false, params, h) &&
           open_side(out, p, n, true, params, h) && sr_polyset_end(out, true);
}

bool sr_stroke_polyline(SrPolySet *out, const Point *input, size_t count,
                        bool closed, Point tangent, const SrStrokeParams *params) {
    if (!count) return true;
    Point *p = sr_composite_alloc(out->resources, count, sizeof(*p), 0);
    if (!p) return sr_polyset_fail(out, out->resources
        ? sr_composite_resource_status(out->resources) : SR_ERR_MEMORY, NULL);
    memcpy(p, input, count * sizeof(*p));
    bool ok = stroke_buffer(out, p, count, closed, tangent, params);
    sr_composite_free(out->resources, p);
    return ok;
}

/* ---- dash and assembly --------------------------------------------------- */

typedef struct {
    const SrPathContour *contour;
    const double *cumulative;
    Point *scratch;             /* segments + 2 points */
    Point *joined;              /* 2 * (segments + 2) points */
} ContourScratch;

static bool stroke_range(SrPolySet *out, const ContourScratch *s, double from,
                         double to, const SrStrokeParams *params) {
    size_t n;
    Point tangent;
    sr_path_contour_extract(s->contour, s->cumulative, from, to, s->scratch,
                            &n, &tangent);
    return stroke_buffer(out, s->scratch, n, false, tangent, params);
}

/* A stretch of a closed contour from `start` through its end and start to
 * `wrap_end`, stroked as one open polyline (a join at the start vertex). */
static bool stroke_wrapped(SrPolySet *out, const ContourScratch *s, double start,
                           double wrap_end, const SrStrokeParams *params) {
    size_t first, second;
    Point tangent, ignored;
    size_t segments = sr_path_contour_segments(s->contour);
    double total = s->cumulative[segments];
    sr_path_contour_extract(s->contour, s->cumulative, start, total, s->joined,
                            &first, &tangent);
    sr_path_contour_extract(s->contour, s->cumulative, 0.0, wrap_end,
                            s->joined + first, &second, &ignored);
    /* The second part starts at the contour start, which the first ends at. */
    memmove(s->joined + first, s->joined + first + 1,
            (second - 1) * sizeof(*s->joined));
    return stroke_buffer(out, s->joined, first + second - 1, false, tangent, params);
}

/* On-intervals of a dashed range that touch a closed contour's seam are
 * held back so the caller can join them across it. */
typedef struct {
    bool hold_first, hold_last;   /* at `from` / at `to` */
    bool has_first, has_last;
    double first[2], last[2];
} DashSeam;

/* Dashes of [from, to] with the phase measured along the contour. */
static bool dash_range(SrPolySet *out, const ContourScratch *s, double from,
                       double to, const SrStrokeParams *params, double period,
                       size_t *emitted, DashSeam *seam) {
    size_t n = params->dash_count;
    double length = to - from;
    double estimate = (floor(length / period) + 2.0) * (double)n;
    if (!(estimate <= (double)SR_MAX_DASH_PIECES) ||
        *emitted + (size_t)estimate > SR_MAX_DASH_PIECES)
        return sr_polyset_fail(out, SR_ERR_RENDER,
                               "dash pattern exceeds 1048576 pieces");
    double phase = from + params->dash_offset;
    phase -= floor(phase / period) * period;
    size_t index = 0;
    while (index + 1 < n && phase >= params->dash[index]) {
        /* A zero-length dash exactly at the start is still drawn. */
        if (params->dash[index] == 0.0 && phase == 0.0) break;
        phase -= params->dash[index];
        ++index;
    }
    double position = from - phase;        /* start of the current entry */
    size_t guard = (size_t)estimate + n;
    size_t step = 0;
    for (; position < to && step <= guard; ++step) {
        double end = position + params->dash[index];
        /* At extreme magnitudes a dash may not advance the position. */
        if (params->dash[index] > 0.0 && !(end > position))
            return sr_polyset_fail(out, SR_ERR_RENDER,
                                   "dash lengths are too small for the outline "
                                   "length (no progress in double precision)");
        if (index % 2 == 0) {
            double a = position > from ? position : from;
            double b = end < to ? end : to;
            if (b > a || (b == a && params->dash[index] == 0.0 && a >= from)) {
                ++*emitted;
                bool at_from = seam && seam->hold_first && a == from && b > a;
                bool at_to = seam && seam->hold_last && b == to && b > a;
                if (at_from) {
                    seam->has_first = true;
                    seam->first[0] = a;
                    seam->first[1] = b;
                }
                if (at_to) {
                    seam->has_last = true;
                    seam->last[0] = a;
                    seam->last[1] = b;
                }
                if (!at_from && !at_to && !stroke_range(out, s, a, b, params))
                    return false;
            }
        }
        position = end;
        index = (index + 1) % n;
    }
    if (position < to)
        return sr_polyset_fail(out, SR_ERR_RENDER,
                               "dash pattern traversal exceeded its bound");
    return true;
}

static bool stroke_piece(SrPolySet *out, const ContourScratch *s,
                         const SrStrokePiece *piece, const SrStrokeParams *params,
                         double period, size_t *emitted) {
    const SrPathContour *contour = s->contour;
    size_t segments = sr_path_contour_segments(contour);
    double total = s->cumulative[segments];
    if (period > 0.0 && (piece->whole || piece->wrap)) {
        /* Closed seams: a dash crossing the start vertex stays one piece. */
        DashSeam seam = {0};
        if (piece->whole) {
            seam.hold_first = seam.hold_last = true;
            if (!dash_range(out, s, 0.0, total, params, period, emitted, &seam))
                return false;
            if (seam.has_first && seam.has_last && seam.first[0] == seam.last[0]) {
                /* One dash covers the whole closed contour. */
                memcpy(s->joined, contour->points, contour->count * sizeof(*s->joined));
                return stroke_buffer(out, s->joined, contour->count, true,
                                     (Point){0.0, 0.0}, params);
            }
        } else {
            DashSeam tail = {.hold_last = true}, head = {.hold_first = true};
            if (!dash_range(out, s, piece->start, total, params, period, emitted,
                            &tail) ||
                !dash_range(out, s, 0.0, piece->wrap_end, params, period, emitted,
                            &head))
                return false;
            seam = (DashSeam){.has_first = head.has_first, .has_last = tail.has_last,
                              .first = {head.first[0], head.first[1]},
                              .last = {tail.last[0], tail.last[1]}};
        }
        if (seam.has_first && seam.has_last)
            return stroke_wrapped(out, s, seam.last[0], seam.first[1], params);
        if (seam.has_first &&
            !stroke_range(out, s, seam.first[0], seam.first[1], params))
            return false;
        return !seam.has_last ||
               stroke_range(out, s, seam.last[0], seam.last[1], params);
    }
    if (period > 0.0)
        return dash_range(out, s, piece->start, piece->end, params, period,
                          emitted, NULL);
    if (piece->whole) {
        memcpy(s->joined, contour->points, contour->count * sizeof(*s->joined));
        return stroke_buffer(out, s->joined, contour->count, true,
                             (Point){0.0, 0.0}, params);
    }
    if (!piece->wrap) return stroke_range(out, s, piece->start, piece->end, params);
    return stroke_wrapped(out, s, piece->start, piece->wrap_end, params);
}

bool sr_stroke_build(const SrPolySet *source, const SrStrokeParams *params,
                     SrPolySet *out) {
    size_t count = source->range_count;
    if (!count || !(params->width > 0.0)) return true;
    if (!isfinite(params->width) || params->width > SR_MAX_SHAPE_COORDINATE)
        return sr_polyset_fail(out, SR_ERR_RENDER,
                               "stroke width must be finite and within 1e9");
    double period = 0.0;
    for (size_t i = 0; i < params->dash_count; ++i) period += params->dash[i];
    if (!params->dash || !(period > 0.0)) period = 0.0;
    SrCompositeResources *resources = out->resources;
    size_t segments = 0;
    for (size_t c = 0; c < count; ++c)
        segments += sr_path_contour_segments(&source->contours[c]) + 1;
    /* Arc lengths, per-contour lengths/flags, pieces and one scratch
     * polyline sized for the longest contour (two for wrapped joins). */
    size_t longest = 0;
    for (size_t c = 0; c < count; ++c) {
        size_t s = sr_path_contour_segments(&source->contours[c]);
        if (s > longest) longest = s;
    }
    double *cumulative = sr_composite_alloc(resources, segments, sizeof(double), 0);
    double *lengths = cumulative ? sr_composite_alloc(resources, count, sizeof(double), 0) : NULL;
    bool *closed = lengths ? sr_composite_alloc(resources, count, sizeof(bool), 0) : NULL;
    SrStrokePiece *pieces = closed ? sr_composite_alloc(resources, 2 * count + 2,
                                                        sizeof(*pieces), 0) : NULL;
    Point *scratch = pieces ? sr_composite_alloc(resources, 3 * (longest + 2),
                                                 sizeof(Point), 0) : NULL;
    bool ok = scratch != NULL;
    if (!ok) sr_polyset_fail(out, resources
        ? sr_composite_resource_status(resources) : SR_ERR_MEMORY, NULL);
    size_t *offsets = NULL;
    if (ok) {
        offsets = sr_composite_alloc(resources, count, sizeof(size_t), 0);
        ok = offsets != NULL;
        if (!ok) sr_polyset_fail(out, resources
            ? sr_composite_resource_status(resources) : SR_ERR_MEMORY, NULL);
    }
    if (ok) {
        size_t at = 0;
        for (size_t c = 0; c < count; ++c) {
            offsets[c] = at;
            lengths[c] = sr_path_contour_measure(&source->contours[c], cumulative + at);
            closed[c] = source->contours[c].closed;
            at += sr_path_contour_segments(&source->contours[c]) + 1;
        }
        size_t piece_count = sr_stroke_trim(lengths, closed, count, params, pieces);
        size_t emitted = 0;
        for (size_t i = 0; ok && i < piece_count; ++i) {
            ContourScratch s = {&source->contours[pieces[i].contour],
                                cumulative + offsets[pieces[i].contour],
                                scratch, scratch + (longest + 2)};
            ok = stroke_piece(out, &s, &pieces[i], params, period, &emitted);
        }
    }
    sr_composite_free(resources, offsets);
    sr_composite_free(resources, scratch);
    sr_composite_free(resources, pieces);
    sr_composite_free(resources, closed);
    sr_composite_free(resources, lengths);
    sr_composite_free(resources, cumulative);
    return ok;
}
