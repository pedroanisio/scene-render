#include "scene_render/deform.h"

#include <math.h>

typedef struct {
    double dx, dy;              /* displacement */
    double dxdx, dxdy, dydx, dydy;  /* its Jacobian */
} Field;

static Field field_at(const double *offsets, uint32_t rows, uint32_t cols,
                      double width, double height, SrVec2 p) {
    double w = width > 0.0 ? width : 1.0, h = height > 0.0 ? height : 1.0;
    double u = p.x / w * (double)(cols - 1), v = p.y / h * (double)(rows - 1);
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

SrVec2 sr_grid_warp_inverse(const double *offsets, uint32_t rows, uint32_t cols,
                            double width, double height, SrVec2 q) {
    if (!offsets || rows < 2 || cols < 2) return q;
    Field start = field_at(offsets, rows, cols, width, height, q);
    SrVec2 p = {q.x - start.dx, q.y - start.dy};
    for (int iteration = 0; iteration < 12; ++iteration) {
        Field f = field_at(offsets, rows, cols, width, height, p);
        double rx = p.x + f.dx - q.x, ry = p.y + f.dy - q.y;
        if (fabs(rx) < 1e-7 && fabs(ry) < 1e-7) break;
        double a = 1.0 + f.dxdx, b = f.dxdy, c = f.dydx, d = 1.0 + f.dydy;
        double det = a * d - b * c;
        if (fabs(det) < 1e-6) {
            p.x -= rx;           /* folded cell: plain fixed-point step */
            p.y -= ry;
        } else {
            p.x -= (d * rx - b * ry) / det;
            p.y -= (a * ry - c * rx) / det;
        }
    }
    return p;
}

double sr_grid_warp_extent(const double *offsets, size_t count) {
    double extent = 0.0;
    for (size_t i = 0; i < count * 2; ++i)
        if (fabs(offsets[i]) > extent) extent = fabs(offsets[i]);
    return extent;
}
