#include "scene_render/deform.h"

#include <math.h>
#include <stdint.h>

typedef struct {
    double dx, dy;              /* displacement */
    double dxdx, dxdy, dydx, dydy;  /* its Jacobian */
} Field;

static Field field_at(const double *offsets, uint32_t rows, uint32_t cols,
                      double width, double height, SrVec2 p) {
    double w = width > 0.0 ? width : 1.0, h = height > 0.0 ? height : 1.0;
    double u = p.x / w * (double)(cols - 1), v = p.y / h * (double)(rows - 1);
    /* Newton steps and finite but extreme grids can overflow. Never turn
     * a non-finite coordinate into an array index. */
    if (!isfinite(u) || !isfinite(v))
        return (Field){NAN, NAN, NAN, NAN, NAN, NAN};
    bool clamp_u = false, clamp_v = false;
    if (u < 0.0) { u = 0.0; clamp_u = true; }
    if (u > (double)(cols - 1)) { u = (double)(cols - 1); clamp_u = true; }
    if (v < 0.0) { v = 0.0; clamp_v = true; }
    if (v > (double)(rows - 1)) { v = (double)(rows - 1); clamp_v = true; }
    uint32_t c = (uint32_t)floor(u), r = (uint32_t)floor(v);
    if (c > cols - 2) c = cols - 2;
    if (r > rows - 2) r = rows - 2;
    double fu = u - c, fv = v - r;
    const double *d00 = offsets + 2 * ((size_t)r * cols + c);
    const double *d01 = d00 + 2;
    const double *d10 = d00 + 2 * (size_t)cols;
    const double *d11 = d10 + 2;
    Field field;
    double w00 = (1.0 - fu) * (1.0 - fv), w01 = fu * (1.0 - fv);
    double w10 = (1.0 - fu) * fv, w11 = fu * fv;
    field.dx = d00[0] * w00 + d01[0] * w01 + d10[0] * w10 + d11[0] * w11;
    field.dy = d00[1] * w00 + d01[1] * w01 + d10[1] * w10 + d11[1] * w11;
    double su = clamp_u ? 0.0 : (double)(cols - 1) / w;
    double sv = clamp_v ? 0.0 : (double)(rows - 1) / h;
    field.dxdx = ((d01[0] - d00[0]) * (1.0 - fv) + (d11[0] - d10[0]) * fv) * su;
    field.dydx = ((d01[1] - d00[1]) * (1.0 - fv) + (d11[1] - d10[1]) * fv) * su;
    field.dxdy = ((d10[0] - d00[0]) * (1.0 - fu) + (d11[0] - d01[0]) * fu) * sv;
    field.dydy = ((d10[1] - d00[1]) * (1.0 - fu) + (d11[1] - d01[1]) * fu) * sv;
    return field;
}

/* Forward warp of p (constant displacement beyond the box). */
static SrVec2 forward(const double *offsets, uint32_t rows, uint32_t cols,
                      double width, double height, SrVec2 p) {
    Field f = field_at(offsets, rows, cols, width, height, p);
    return (SrVec2){p.x + f.dx, p.y + f.dy};
}

static double residual(const double *offsets, uint32_t rows, uint32_t cols,
                       double width, double height, SrVec2 p, SrVec2 q) {
    SrVec2 f = forward(offsets, rows, cols, width, height, p);
    if (!isfinite(f.x) || !isfinite(f.y)) return INFINITY;
    return fmax(fabs(f.x - q.x), fabs(f.y - q.y));
}

/* One piece of the forward map: F(s, t) = a + b s + c t + d s t over
 * s in [s0, s1], t in [t0, t1] (infinite ends for the strips beyond the
 * box), with p(s, t) = origin + ps * s + pt * t the source point. */
typedef struct {
    SrVec2 a, b, c, d;
    double s0, s1, t0, t1;
    SrVec2 origin, ps, pt;
} Patch;

static double cross(SrVec2 u, SrVec2 v) { return u.x * v.y - u.y * v.x; }

static const double *node_offset(const double *offsets, uint32_t cols,
                                 uint32_t r, uint32_t c) {
    return offsets + 2 * ((size_t)r * cols + c);
}

/* Considers parameter (s, t) of `patch` as a solution for q. */
static void consider(const Patch *patch, double s, double t, SrVec2 q,
                     SrVec2 start, const double *offsets, uint32_t rows,
                     uint32_t cols, double width, double height, bool *found,
                     SrVec2 *best, double *best_distance) {
    const double eps = 1e-9;
    if (!isfinite(s) || !isfinite(t)) return;
    if (s < patch->s0 - eps || s > patch->s1 + eps ||
        t < patch->t0 - eps || t > patch->t1 + eps) return;
    s = fmax(patch->s0, fmin(patch->s1, s));
    t = fmax(patch->t0, fmin(patch->t1, t));
    SrVec2 p = {patch->origin.x + patch->ps.x * s + patch->pt.x * t,
                patch->origin.y + patch->ps.y * s + patch->pt.y * t};
    if (!(residual(offsets, rows, cols, width, height, p, q) < 1e-4)) return;
    double distance = hypot(p.x - start.x, p.y - start.y);
    if (!*found || distance < *best_distance) {
        *found = true;
        *best = p;
        *best_distance = distance;
    }
}

/* Analytic inverse of one bilinear patch: E = b s + c t + d s t with
 * E = q - a eliminates t to cross(b,d) s^2 + (cross(b,c) - cross(E,d)) s
 * - cross(E,c) = 0; each root gives t by least squares along c + d s. */
static void solve_patch(const Patch *patch, SrVec2 q, SrVec2 start,
                        const double *offsets, uint32_t rows, uint32_t cols,
                        double width, double height, bool *found, SrVec2 *best,
                        double *best_distance) {
    SrVec2 e = {q.x - patch->a.x, q.y - patch->a.y};
    double k2 = cross(patch->b, patch->d);
    double k1 = cross(patch->b, patch->c) - cross(e, patch->d);
    double k0 = -cross(e, patch->c);
    double roots[2];
    int count = 0;
    double scale = fmax(fabs(k1), fmax(fabs(k0), 1e-300));
    if (fabs(k2) <= 1e-12 * scale) {
        if (fabs(k1) > 0.0) roots[count++] = -k0 / k1;
    } else {
        double disc = k1 * k1 - 4.0 * k2 * k0;
        if (disc >= 0.0) {
            double root = sqrt(disc);
            /* Numerically stable pair. */
            double qv = -0.5 * (k1 + (k1 >= 0.0 ? root : -root));
            roots[count++] = qv / k2;
            if (qv != 0.0) roots[count++] = k0 / qv;
        }
    }
    for (int i = 0; i < count; ++i) {
        double s = roots[i];
        SrVec2 axis = {patch->c.x + patch->d.x * s, patch->c.y + patch->d.y * s};
        double length = axis.x * axis.x + axis.y * axis.y;
        if (!(length > 0.0)) continue;
        SrVec2 rest = {e.x - patch->b.x * s, e.y - patch->b.y * s};
        double t = (rest.x * axis.x + rest.y * axis.y) / length;
        consider(patch, s, t, q, start, offsets, rows, cols, width, height,
                 found, best, best_distance);
    }
}

/* Robust fallback: every piece of the forward map (the interior cells and
 * the constant-extension strips and corners beyond the box) whose image
 * can contain q is inverted analytically; the solution nearest `start`
 * wins. False when no piece maps onto q. */
static bool solve_cells(const double *offsets, uint32_t rows, uint32_t cols,
                        double width, double height, SrVec2 q, SrVec2 start,
                        SrVec2 *out) {
    double w = width > 0.0 ? width : 1.0, h = height > 0.0 ? height : 1.0;
    double cw = w / (double)(cols - 1), ch = h / (double)(rows - 1);
    bool found = false;
    double best_distance = 0.0;
    SrVec2 best = q;
    /* Columns / rows index -1 and cols-1 / rows-1 stand for the regions
     * beyond the box on each side. */
    for (int64_t r = -1; r < (int64_t)rows; ++r) {
        for (int64_t c = -1; c < (int64_t)cols; ++c) {
            bool out_u = c < 0 || c >= (int64_t)cols - 1;
            bool out_v = r < 0 || r >= (int64_t)rows - 1;
            uint32_t c0 = c < 0 ? 0 : (uint32_t)(c >= (int64_t)cols - 1 ? cols - 1 : c);
            uint32_t r0 = r < 0 ? 0 : (uint32_t)(r >= (int64_t)rows - 1 ? rows - 1 : r);
            uint32_t c1 = out_u ? c0 : c0 + 1, r1 = out_v ? r0 : r0 + 1;
            const double *d00 = node_offset(offsets, cols, r0, c0);
            const double *d01 = node_offset(offsets, cols, r0, c1);
            const double *d10 = node_offset(offsets, cols, r1, c0);
            const double *d11 = node_offset(offsets, cols, r1, c1);
            Patch patch;
            /* s runs along x: the cell parameter fu in [0,1] inside, or the
             * plain local x offset from the box edge outside; t likewise. */
            double x0 = out_u ? (c < 0 ? 0.0 : w) : (double)c0 * cw;
            double y0 = out_v ? (r < 0 ? 0.0 : h) : (double)r0 * ch;
            double sx = out_u ? 1.0 : cw, ty = out_v ? 1.0 : ch;
            patch.s0 = out_u ? (c < 0 ? -INFINITY : 0.0) : 0.0;
            patch.s1 = out_u ? (c < 0 ? 0.0 : INFINITY) : 1.0;
            patch.t0 = out_v ? (r < 0 ? -INFINITY : 0.0) : 0.0;
            patch.t1 = out_v ? (r < 0 ? 0.0 : INFINITY) : 1.0;
            patch.origin = (SrVec2){x0, y0};
            patch.ps = (SrVec2){sx, 0.0};
            patch.pt = (SrVec2){0.0, ty};
            /* Displacement is bilinear in (s, t) inside and constant along
             * an outside axis (d01 == d00 there, so those terms vanish). */
            double us = out_u ? 0.0 : 1.0, vt = out_v ? 0.0 : 1.0;
            patch.a = (SrVec2){x0 + d00[0], y0 + d00[1]};
            patch.b = (SrVec2){sx + (d01[0] - d00[0]) * us, (d01[1] - d00[1]) * us};
            patch.c = (SrVec2){(d10[0] - d00[0]) * vt, ty + (d10[1] - d00[1]) * vt};
            patch.d = (SrVec2){(d11[0] - d01[0] - d10[0] + d00[0]) * us * vt,
                               (d11[1] - d01[1] - d10[1] + d00[1]) * us * vt};
            if (!out_u && !out_v) {
                /* A bilinear cell lies in the hull of its corners. */
                double lo_x = INFINITY, hi_x = -INFINITY, lo_y = INFINITY, hi_y = -INFINITY;
                const double corner[4][2] = {{0, 0}, {1, 0}, {0, 1}, {1, 1}};
                for (int k = 0; k < 4; ++k) {
                    double s = corner[k][0], t = corner[k][1];
                    double fx = patch.a.x + patch.b.x * s + patch.c.x * t + patch.d.x * s * t;
                    double fy = patch.a.y + patch.b.y * s + patch.c.y * t + patch.d.y * s * t;
                    lo_x = fmin(lo_x, fx); hi_x = fmax(hi_x, fx);
                    lo_y = fmin(lo_y, fy); hi_y = fmax(hi_y, fy);
                }
                double slack = 1e-6 * (1.0 + fabs(q.x) + fabs(q.y));
                if (q.x < lo_x - slack || q.x > hi_x + slack ||
                    q.y < lo_y - slack || q.y > hi_y + slack) continue;
            }
            solve_patch(&patch, q, start, offsets, rows, cols, width, height,
                        &found, &best, &best_distance);
        }
    }
    if (found) *out = best;
    return found;
}

bool sr_grid_warp_inverse(const double *offsets, uint32_t rows, uint32_t cols,
                          double width, double height, SrVec2 q, SrVec2 *out) {
    if (!out || !isfinite(q.x) || !isfinite(q.y) ||
        !isfinite(width) || !isfinite(height)) return false;
    if (!offsets || rows < 2 || cols < 2) { *out = q; return true; }
    Field start = field_at(offsets, rows, cols, width, height, q);
    SrVec2 initial = {q.x - start.dx, q.y - start.dy};
    if (!isfinite(initial.x) || !isfinite(initial.y)) return false;
    SrVec2 p = initial;
    for (int iteration = 0; iteration < 12; ++iteration) {
        if (!isfinite(p.x) || !isfinite(p.y)) break;
        Field f = field_at(offsets, rows, cols, width, height, p);
        double rx = p.x + f.dx - q.x, ry = p.y + f.dy - q.y;
        if (fabs(rx) < 1e-7 && fabs(ry) < 1e-7) break;
        double a = 1.0 + f.dxdx, b = f.dxdy, c = f.dydx, d = 1.0 + f.dydy;
        double det = a * d - b * c;
        if (!isfinite(rx) || !isfinite(ry) || !isfinite(det)) break;
        if (fabs(det) < 1e-6) {
            p.x -= rx;           /* folded cell: plain fixed-point step */
            p.y -= ry;
        } else {
            p.x -= (d * rx - b * ry) / det;
            p.y -= (a * ry - c * rx) / det;
        }
    }
    if (residual(offsets, rows, cols, width, height, p, q) < 1e-4) {
        *out = p;
        return true;
    }
    return solve_cells(offsets, rows, cols, width, height, q, initial, out);
}

double sr_grid_warp_extent(const double *offsets, size_t count) {
    double extent = 0.0;
    for (size_t i = 0; i < count * 2; ++i)
        if (fabs(offsets[i]) > extent) extent = fabs(offsets[i]);
    return extent;
}
