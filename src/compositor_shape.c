/* SPDX-License-Identifier: Apache-2.0 */
/* B1-4 extended shape rendering: per-frame evaluation, geometry, coverage
 * grids and per-pixel paint composition. The contract is in
 * docs/design/b1-4-shapes-paints.md ("Rendering path", "Resource
 * accounting"). Everything here runs on the calling thread except
 * sr_shape_raster_sample, which only reads the finished raster. */
#include "compositor_shape_internal.h"
#include "compositor_evaluation_internal.h"
#include "vector_shape_internal.h"
#include "scene_render/color.h"

#include <float.h>
#include <math.h>
#include <stdlib.h>
#include <string.h>

static double unit(double value) {
    return value > 0.0 ? (value < 1.0 ? value : 1.0) : 0.0;
}

static double max_scale(SrMat3 m) {
    double a = m.m00, b = m.m01, c = m.m10, d = m.m11;
    double s = a * a + b * b + c * c + d * d, det = a * d - b * c;
    double disc = s * s - 4.0 * det * det;
    return sqrt(0.5 * (s + sqrt(disc > 0.0 ? disc : 0.0)));
}

static SrStatus fail(const SrShapeInput *in, const char *attribute,
                     const char *message) {
    if (in->diag)
        sr_diag_error(in->diag, in->node->source_line, "shape", attribute,
                      "shape '%s': %s", in->node->id ? in->node->id : "", message);
    if (in->resources)
        sr_composite_resource_fail(in->resources, SR_ERR_RENDER, message);
    return SR_ERR_RENDER;
}

static SrStatus ledger_status(const SrShapeInput *in) {
    return in->resources ? sr_composite_resource_status(in->resources)
                         : SR_ERR_MEMORY;
}

static SrStatus set_status(const SrShapeInput *in, const SrPolySet *set,
                           const char *attribute) {
    if (set->status == SR_ERR_RENDER && set->error)
        return fail(in, attribute, set->error);
    if (set->status == SR_ERR_ARGUMENT)
        return fail(in, attribute, set->error ? set->error : "invalid geometry");
    return set->status == SR_OK ? ledger_status(in) : set->status;
}

static bool values_work(SrCompositeResources *resources,
                        const SrAnimValue *const *values, size_t count) {
    for (size_t i = 0; i < count; ++i)
        if (!sr_composite_anim_work(resources, values[i], false)) return false;
    return true;
}

static bool style_work(SrCompositeResources *resources, const SrShapeStyle *style) {
    const SrAnimValue *values[] = {&style->radius, &style->inner_radius,
        &style->outer_radius, &style->inner_roundness, &style->outer_roundness,
        &style->trim_start, &style->trim_end, &style->trim_offset,
        &style->stroke.dash_offset};
    return values_work(resources, values, sizeof(values) / sizeof(values[0]));
}

static bool paint_work(SrCompositeResources *resources, const SrPaint *paint) {
    const SrAnimValue *values[] = {&paint->rotation, &paint->x1, &paint->y1,
        &paint->x2, &paint->y2, &paint->cx, &paint->cy, &paint->r, &paint->fx,
        &paint->fy, &paint->fr, &paint->aspect, &paint->angle};
    if (!values_work(resources, values, sizeof(values) / sizeof(values[0]))) return false;
    if (paint->stop_count > SR_MAX_GRADIENT_STOPS || (paint->stop_count && !paint->stops))
        return sr_composite_resource_fail(resources, SR_ERR_RENDER,
                                          "invalid gradient stop storage");
    for (size_t i = 0; i < paint->stop_count; ++i) {
        const SrGradientStop *stop = &paint->stops[i];
        const SrAnimValue *stop_values[] = {&stop->offset, &stop->opacity, &stop->midpoint};
        if (!values_work(resources, stop_values, 3) ||
            !sr_composite_color_work(resources, &stop->color))
            return false;
    }
    return true;
}

static uint64_t paint_cost(const SrPaint *paint) {
    if (!paint) return 4;
    uint64_t steps = 0;
    for (size_t n = paint->stop_count + 1; n > 1; n = (n + 1) / 2) ++steps;
    return 32 + 8 * steps;
}

/* Geometry parameters of the shape at the render time. */
static SrStatus shape_params(const SrShapeInput *in, SrShapeParams *p,
                             double scale) {
    const SrNode *node = in->node;
    const SrShapeStyle *style = &node->shape_style;
    double t = in->time;
    *p = (SrShapeParams){.type = node->shape, .width = in->width,
                         .height = in->height, .points = style->points,
                         .path = style->path, .scale = scale};
    if (node->shape == SR_SHAPE_RECT || node->shape == SR_SHAPE_ROUNDED_RECT) {
        if (style->corner_radii_set) {
            memcpy(p->radii, style->corner_radii, sizeof(p->radii));
        } else {
            double radius = sr_anim_eval(&style->radius, t);
            if (!isfinite(radius) || radius > SR_MAX_SHAPE_COORDINATE)
                return fail(in, "radius", "radius must be finite and within 1e9");
            for (int i = 0; i < 4; ++i) p->radii[i] = radius;
        }
    }
    if (node->shape == SR_SHAPE_POLYGON || node->shape == SR_SHAPE_STAR) {
        double outer = style->outer_radius_set ? sr_anim_eval(&style->outer_radius, t)
                                               : 0.5 * fmin(in->width, in->height);
        double inner = style->inner_radius_set ? sr_anim_eval(&style->inner_radius, t)
                                               : 0.5 * outer;
        if (!isfinite(outer) || !isfinite(inner) ||
            fabs(outer) > SR_MAX_SHAPE_COORDINATE || fabs(inner) > SR_MAX_SHAPE_COORDINATE)
            return fail(in, "outerRadius/innerRadius",
                        "radii must be finite and within 1e9");
        outer = outer > 0.0 ? outer : 0.0;
        inner = inner > 0.0 ? (inner < outer ? inner : outer) : 0.0;
        p->outer_radius = outer;
        p->inner_radius = inner;
        p->outer_roundness = unit(sr_anim_eval(&style->outer_roundness, t));
        p->inner_roundness = unit(sr_anim_eval(&style->inner_roundness, t));
    }
    if (node->shape == SR_SHAPE_PATH && (!style->path || !style->path->count))
        return fail(in, "path", "shape=path needs a prepared path");
    return SR_OK;
}

static SrStatus stroke_params(const SrShapeInput *in, SrStrokeParams *p,
                              double scale) {
    const SrShapeStyle *style = &in->node->shape_style;
    double t = in->time;
    double width = in->node->stroke_width;
    if (style->stroke.position != SR_STROKE_CENTER) width *= 2.0;
    *p = (SrStrokeParams){.width = width, .cap = style->stroke.cap,
                          .join = style->stroke.join,
                          .miter_limit = style->stroke.miter_limit,
                          .dash = style->stroke.dash,
                          .dash_count = style->stroke.dash_count,
                          .trim_mode = style->trim_mode, .scale = scale};
    if (style->stroke.dash_count > 2 * SR_MAX_DASH_ENTRIES ||
        (style->stroke.dash_count && !style->stroke.dash) ||
        style->stroke.dash_count % 2)
        return fail(in, "dash", "invalid dash storage");
    for (size_t i = 0; i < style->stroke.dash_count; ++i)
        if (!(style->stroke.dash[i] >= 0.0) ||
            style->stroke.dash[i] > SR_MAX_SHAPE_COORDINATE)
            return fail(in, "dash", "dash lengths must be in [0, 1e9]");
    if (!(p->miter_limit >= 1.0) || p->miter_limit > SR_MAX_MITER_LIMIT)
        return fail(in, "miterLimit", "miter limit must be in [1, 1e6]");
    double offset = sr_anim_eval(&style->stroke.dash_offset, t);
    if (!isfinite(offset) || fabs(offset) > SR_MAX_SHAPE_COORDINATE)
        return fail(in, "dashOffset", "dash offset must be finite and within 1e9");
    p->dash_offset = offset;
    double a = sr_anim_eval(&style->trim_start, t);
    double b = sr_anim_eval(&style->trim_end, t);
    double o = sr_anim_eval(&style->trim_offset, t);
    if (!isfinite(a) || !isfinite(b) || !isfinite(o) || fabs(o) > SR_MAX_TRIM_OFFSET)
        return fail(in, "trimStart/trimEnd/trimOffset",
                    "trim values must be finite; trimOffset within +-1e6");
    p->trim_start = unit(a);
    p->trim_end = unit(b);
    p->trim_offset = o;
    p->trimmed = !(p->trim_start == 0.0 && p->trim_end == 1.0);
    return SR_OK;
}

static void colour(const SrShapeInput *in, const SrAnimColor *color, float out[4]) {
    sr_color_to_blend(&in->scene->project, sr_anim_color_eval(color, in->time), out);
}

static void extend(double box[4], const SrPolySet *set) {
    for (size_t i = 0; i < set->point_count; ++i) {
        SrPathPoint p = set->points[i];
        box[0] = fmin(box[0], p.x); box[1] = fmin(box[1], p.y);
        box[2] = fmax(box[2], p.x); box[3] = fmax(box[3], p.y);
    }
}

static void mapped_extend(double box[4], const SrPolySet *set, SrMat3 m) {
    for (size_t i = 0; i < set->point_count; ++i) {
        SrPathPoint p = set->points[i];
        double x = m.m00 * p.x + m.m01 * p.y + m.m02;
        double y = m.m10 * p.x + m.m11 * p.y + m.m12;
        box[0] = fmin(box[0], x); box[1] = fmin(box[1], y);
        box[2] = fmax(box[2], x); box[3] = fmax(box[3], y);
    }
}

static SrStatus rasterize(const SrShapeInput *in, SrShapeRaster *out,
                          const SrPolySet *set, SrFillRule rule,
                          const SrPathMap *map, float *cells, float **grid) {
    size_t samples = (size_t)out->grid_w * out->grid_h;
    *grid = sr_composite_alloc(in->resources, samples, sizeof(float), samples);
    if (!*grid) return ledger_status(in);
    if (!set->range_count) return SR_OK;   /* already zero */
    uint64_t work = sr_path_raster_work(set->contours, set->range_count, map,
                                        out->grid_w, out->grid_h);
    if (!sr_composite_work(in->resources, 1, work) ||
        !sr_composite_work(in->resources, samples, 1))
        return ledger_status(in);
    SrStatus status = sr_path_rasterize(set->contours, set->range_count, map, rule,
                                        out->grid_w, out->grid_h, cells, *grid);
    if (status != SR_OK)
        return fail(in, "shape", "transformed shape geometry is not finite");
    return SR_OK;
}

SrStatus sr_shape_raster_build(const SrShapeInput *in, SrShapeRaster *out) {
    const SrNode *node = in->node;
    const SrShapeStyle *style = &node->shape_style;
    SrCompositeResources *resources = in->resources;
    *out = (SrShapeRaster){.resources = resources, .empty = true,
                           .order = style->stroke.order,
                           .local_grid = in->local_grid};
    const SrPaint *fill_paint = style->fill_paint.paint;
    const SrPaint *stroke_paint = style->stroke_paint.paint;
    if ((style->fill_paint.id && !fill_paint) || (style->stroke_paint.id && !stroke_paint))
        return fail(in, "fill/stroke", "paint reference is not resolved");
    if (!style_work(resources, style) ||
        (!fill_paint && !sr_composite_color_work(resources, &node->fill)) ||
        (!stroke_paint && !sr_composite_color_work(resources, &node->stroke)) ||
        (fill_paint && !paint_work(resources, fill_paint)) ||
        (stroke_paint && !paint_work(resources, stroke_paint)))
        return ledger_status(in);
    if (!(in->width > 0.0) || !(in->height > 0.0) || !isfinite(in->width) ||
        !isfinite(in->height))
        return SR_OK;                     /* nothing to draw, as legacy shapes */
    double scale = in->local_grid ? 1.0 : max_scale(in->world);
    if (!(scale > 0.0) || !isfinite(scale)) return SR_OK;
    /* Colours and paints. */
    if (fill_paint || stroke_paint) {
        out->evals = sr_composite_alloc(resources, 2, sizeof(SrPaintEval), 0);
        if (!out->evals) return ledger_status(in);
    }
    SrStatus status;
    if (fill_paint) {
        status = sr_paint_eval(fill_paint, &in->scene->project, in->time, in->width,
                               in->height, &out->evals[0], in->diag);
        if (status != SR_OK) return fail(in, "fill", "invalid evaluated fill paint");
        out->fill_paint = &out->evals[0];
    } else {
        colour(in, &node->fill, out->fill_px);
    }
    if (stroke_paint) {
        status = sr_paint_eval(stroke_paint, &in->scene->project, in->time, in->width,
                               in->height, &out->evals[1], in->diag);
        if (status != SR_OK) return fail(in, "stroke", "invalid evaluated stroke paint");
        out->stroke_paint = &out->evals[1];
    } else {
        colour(in, &node->stroke, out->stroke_px);
    }
    out->draw_fill = node->shape != SR_SHAPE_LINE &&
                     (fill_paint || out->fill_px[3] > 0.0f);
    bool draw_stroke = node->stroke_width > 0.0 &&
                       (stroke_paint || out->stroke_px[3] > 0.0f);
    bool clip_stroke = draw_stroke && style->stroke.position != SR_STROKE_CENTER;
    if (!out->draw_fill && !draw_stroke) return SR_OK;
    /* Geometry. */
    SrShapeParams shape;
    SrStrokeParams stroke;
    status = shape_params(in, &shape, scale);
    if (status == SR_OK && draw_stroke) status = stroke_params(in, &stroke, scale);
    if (status != SR_OK) return status;
    SrPolySet fill_set, stroke_set;
    sr_polyset_init(&fill_set, resources, SR_MAX_SHAPE_VERTICES);
    sr_polyset_init(&stroke_set, resources, SR_MAX_SHAPE_VERTICES);
    float *cells = NULL;
    if (!sr_shape_build(&fill_set, &shape) || !sr_polyset_seal(&fill_set)) {
        status = set_status(in, &fill_set, "shape");
        goto done;
    }
    if (draw_stroke) {
        stroke_set.limit = SR_MAX_SHAPE_VERTICES - fill_set.point_count;
        if (!sr_stroke_build(&fill_set, &stroke, &stroke_set) ||
            !sr_polyset_seal(&stroke_set)) {
            status = set_status(in, &stroke_set, "strokeWidth/dash/trim");
            goto done;
        }
    }
    bool need_fill = out->draw_fill || clip_stroke;
    double box[4] = {DBL_MAX, DBL_MAX, -DBL_MAX, -DBL_MAX};
    if (need_fill) extend(box, &fill_set);
    if (draw_stroke) extend(box, &stroke_set);
    memcpy(out->extent, box, sizeof(box));
    if (!(box[0] <= box[2]) || !(box[1] <= box[3])) goto done;   /* no points */
    /* Raster domain and local -> grid map. */
    SrPathMap map;
    if (!in->local_grid) {
        size_t points = (need_fill ? fill_set.point_count : 0) +
                        (draw_stroke ? stroke_set.point_count : 0);
        if (!sr_composite_work(resources, points, 4)) {
            status = ledger_status(in);
            goto done;
        }
        double screen[4] = {DBL_MAX, DBL_MAX, -DBL_MAX, -DBL_MAX};
        if (need_fill) mapped_extend(screen, &fill_set, in->world);
        if (draw_stroke) mapped_extend(screen, &stroke_set, in->world);
        for (int i = 0; i < 4; ++i)
            if (!isfinite(screen[i]) || fabs(screen[i]) > SR_MAX_SHAPE_COORDINATE) {
                status = fail(in, "shape", "transformed shape exceeds the 1e9 "
                                           "raster coordinate bound");
                goto done;
            }
        SrClip bounds = {sr_clamp_int(floor(screen[0]) - 1.0, in->clip.x0, in->clip.x1),
                         sr_clamp_int(floor(screen[1]) - 1.0, in->clip.y0, in->clip.y1),
                         sr_clamp_int(ceil(screen[2]) + 1.0, in->clip.x0, in->clip.x1),
                         sr_clamp_int(ceil(screen[3]) + 1.0, in->clip.y0, in->clip.y1)};
        if (bounds.x1 <= bounds.x0 || bounds.y1 <= bounds.y0) goto done;
        out->bounds = bounds;
        out->grid_x0 = bounds.x0;
        out->grid_y0 = bounds.y0;
        out->grid_w = (uint32_t)(bounds.x1 - bounds.x0);
        out->grid_h = (uint32_t)(bounds.y1 - bounds.y0);
        map = (SrPathMap){{in->world.m00, in->world.m01, in->world.m02 - bounds.x0,
                           in->world.m10, in->world.m11, in->world.m12 - bounds.y0}};
    } else {
        double x0 = floor(box[0]) - 1.0, y0 = floor(box[1]) - 1.0;
        double x1 = ceil(box[2]) + 1.0, y1 = ceil(box[3]) + 1.0;
        if (!(x1 - x0 <= SR_MAX_COVERAGE_DIMENSION) ||
            !(y1 - y0 <= SR_MAX_COVERAGE_DIMENSION)) {
            status = fail(in, "deform", "deformed shape grid exceeds 16384 local "
                                        "units per side");
            goto done;
        }
        out->grid_x0 = (int)x0;
        out->grid_y0 = (int)y0;
        out->grid_w = (uint32_t)(x1 - x0);
        out->grid_h = (uint32_t)(y1 - y0);
        map = (SrPathMap){{1.0, 0.0, -x0, 0.0, 1.0, -y0}};
    }
    size_t cell_count = ((size_t)out->grid_w + 2) * out->grid_h;
    cells = sr_composite_alloc(resources, cell_count, sizeof(float), cell_count);
    if (!cells) {
        status = ledger_status(in);
        goto done;
    }
    if (need_fill) {
        status = rasterize(in, out, &fill_set, style->fill_rule, &map, cells, &out->fill);
        if (status != SR_OK) goto done;
    }
    if (draw_stroke) {
        status = rasterize(in, out, &stroke_set, SR_FILL_NONZERO, &map, cells,
                           &out->stroke);
        if (status != SR_OK) goto done;
        if (clip_stroke) {
            size_t samples = (size_t)out->grid_w * out->grid_h;
            if (!sr_composite_work(resources, samples, 1)) {
                status = ledger_status(in);
                goto done;
            }
            sr_coverage_clip(out->stroke, out->fill, samples,
                             style->stroke.position == SR_STROKE_INSIDE);
        }
    }
    out->pixel_cost = 8 + (out->draw_fill ? paint_cost(fill_paint) : 0) +
                      (draw_stroke ? paint_cost(stroke_paint) : 0);
    out->empty = false;
done:
    sr_composite_free(resources, cells);
    sr_polyset_free(&stroke_set);
    sr_polyset_free(&fill_set);
    return status;
}

void sr_shape_raster_free(SrShapeRaster *raster) {
    if (!raster) return;
    sr_composite_free(raster->resources, raster->fill);
    sr_composite_free(raster->resources, raster->stroke);
    sr_composite_free(raster->resources, raster->evals);
    raster->fill = raster->stroke = NULL;
    raster->evals = NULL;
    raster->empty = true;
}

static float bilinear(const SrShapeRaster *r, const float *grid, double x, double y) {
    double gx = x - r->grid_x0 - 0.5, gy = y - r->grid_y0 - 0.5;
    if (!(gx > -1.0) || !(gy > -1.0) || !(gx < (double)r->grid_w) ||
        !(gy < (double)r->grid_h))
        return 0.0f;
    double fx = floor(gx), fy = floor(gy);
    int ix = (int)fx, iy = (int)fy;
    float tx = (float)(gx - fx), ty = (float)(gy - fy);
    float v[4];
    for (int k = 0; k < 4; ++k) {
        int sx = ix + (k & 1), sy = iy + (k >> 1);
        v[k] = sx < 0 || sy < 0 || sx >= (int)r->grid_w || sy >= (int)r->grid_h
            ? 0.0f : grid[(size_t)sy * r->grid_w + (size_t)sx];
    }
    float top = v[0] + (v[1] - v[0]) * tx, bottom = v[2] + (v[3] - v[2]) * tx;
    return top + (bottom - top) * ty;
}

bool sr_shape_raster_sample(const SrShapeRaster *r, int px, int py, double x,
                            double y, float out[4]) {
    float cf = 0.0f, cs = 0.0f;
    if (!r->local_grid) {
        int gx = px - r->grid_x0, gy = py - r->grid_y0;
        if (gx < 0 || gy < 0 || gx >= (int)r->grid_w || gy >= (int)r->grid_h)
            return false;
        size_t i = (size_t)gy * r->grid_w + (size_t)gx;
        if (r->draw_fill && r->fill) cf = r->fill[i];
        if (r->stroke) cs = r->stroke[i];
    } else {
        if (r->draw_fill && r->fill) cf = bilinear(r, r->fill, x, y);
        if (r->stroke) cs = bilinear(r, r->stroke, x, y);
    }
    if (!(cf > 0.0f) && !(cs > 0.0f)) return false;
    float f[4] = {0, 0, 0, 0}, s[4] = {0, 0, 0, 0};
    if (cf > 0.0f) {
        if (r->fill_paint) sr_paint_sample(r->fill_paint, x, y, px, py, f);
        else memcpy(f, r->fill_px, sizeof(f));
    }
    if (cs > 0.0f) {
        if (r->stroke_paint) sr_paint_sample(r->stroke_paint, x, y, px, py, s);
        else memcpy(s, r->stroke_px, sizeof(s));
    }
    if (r->order == SR_PAINT_FILL_STROKE) {
        float keep = 1.0f - s[3] * cs;
        for (int c = 0; c < 4; ++c) out[c] = s[c] * cs + f[c] * cf * keep;
    } else {
        float keep = 1.0f - f[3] * cf;
        for (int c = 0; c < 4; ++c) out[c] = f[c] * cf + s[c] * cs * keep;
    }
    return true;
}

bool sr_shape_local_bounds(SrCompositeResources *resources, const SrNode *node,
                           double time, double width, double height,
                           double out[4]) {
    const SrShapeStyle *style = &node->shape_style;
    if (!style_work(resources, style)) return false;
    double x0 = 0.0, y0 = 0.0, x1 = width, y1 = height;
    if (node->shape == SR_SHAPE_POLYGON || node->shape == SR_SHAPE_STAR) {
        double outer = style->outer_radius_set ? sr_anim_eval(&style->outer_radius, time)
                                               : 0.5 * fmin(width, height);
        double inner = style->inner_radius_set ? sr_anim_eval(&style->inner_radius, time)
                                               : 0.5 * outer;
        double rho = fmax(fabs(outer), fabs(inner));
        double round = fmax(unit(sr_anim_eval(&style->outer_roundness, time)),
                            unit(sr_anim_eval(&style->inner_roundness, time)));
        double points = style->points >= 3 ? (double)style->points : 3.0;
        rho += round * SR_PI * rho / (2.0 * points);   /* handle length bound */
        x0 = fmin(x0, 0.5 * width - rho); x1 = fmax(x1, 0.5 * width + rho);
        y0 = fmin(y0, 0.5 * height - rho); y1 = fmax(y1, 0.5 * height + rho);
    }
    if (node->shape == SR_SHAPE_PATH && style->path && style->path->count) {
        x0 = fmin(x0, style->path_bounds[0]); y0 = fmin(y0, style->path_bounds[1]);
        x1 = fmax(x1, style->path_bounds[2]); y1 = fmax(y1, style->path_bounds[3]);
    }
    double half = node->stroke_width * 0.5;
    if (style->stroke.position != SR_STROKE_CENTER) half *= 2.0;
    double factor = 1.0;
    if (style->stroke.join == SR_LINE_JOIN_MITER && style->stroke.miter_limit > factor)
        factor = style->stroke.miter_limit;
    if (style->stroke.cap == SR_LINE_CAP_SQUARE && factor < 1.5) factor = 1.5;
    double pad = half * factor + 1.0;
    out[0] = x0 - pad;
    out[1] = y0 - pad;
    out[2] = x1 + pad;
    out[3] = y1 + pad;
    for (int i = 0; i < 4; ++i)
        if (!isfinite(out[i])) return false;
    return true;
}

static bool paint_valid(const SrPaint *paint) {
    return paint->stop_count && paint->stop_count <= SR_MAX_GRADIENT_STOPS &&
           paint->stops && paint->type <= SR_PAINT_CONIC &&
           paint->spread <= SR_SPREAD_REPEAT && paint->units <= SR_PAINT_UNITS_USER &&
           paint->space <= SR_INTERP_OKLCH;
}

bool sr_shape_style_prepare(const SrNode *node, uint64_t *bytes,
                            const char **attribute, const char **message) {
    const SrShapeStyle *style = &node->shape_style;
    *bytes = 0;
    *attribute = NULL;
    *message = NULL;
    if (!style->extended) return true;
    if (node->shape > SR_SHAPE_LINE) {
        *attribute = "shape";
        *message = "unknown shape type";
    } else if ((node->shape == SR_SHAPE_POLYGON || node->shape == SR_SHAPE_STAR) &&
               (style->points < 3 || style->points > SR_MAX_SHAPE_POINTS)) {
        *attribute = "points";
        *message = "points must be in [3, 4096]";
    } else if (style->stroke.dash_count > 2 * SR_MAX_DASH_ENTRIES ||
               style->stroke.dash_count % 2 ||
               (style->stroke.dash_count && !style->stroke.dash)) {
        *attribute = "dash";
        *message = "dash storage must hold an even count of at most 128 lengths";
    } else if (node->shape == SR_SHAPE_PATH && (!style->path || !style->path->count ||
                                                !style->path->items)) {
        *attribute = "path";
        *message = "shape=path needs a prepared path";
    } else if ((style->fill_paint.paint && !paint_valid(style->fill_paint.paint)) ||
               (style->stroke_paint.paint && !paint_valid(style->stroke_paint.paint))) {
        *attribute = "fill/stroke";
        *message = "invalid gradient paint storage";
    } else if (style->stroke.cap > SR_LINE_CAP_SQUARE ||
               style->stroke.join > SR_LINE_JOIN_BEVEL ||
               style->stroke.position > SR_STROKE_OUTSIDE ||
               style->stroke.order > SR_PAINT_STROKE_FILL ||
               style->trim_mode > SR_TRIM_SEQUENTIAL ||
               style->fill_rule > SR_FILL_NONZERO) {
        *attribute = "strokeCap/strokeJoin/strokePosition/paintOrder/trimMode";
        *message = "unknown stroke style value";
    }
    if (*message) return false;
    if (style->path) {
        uint64_t total = sizeof(*style->path) +
                         (uint64_t)style->path->capacity * sizeof(*style->path->items);
        for (size_t i = 0; i < style->path->count; ++i)
            total += (uint64_t)style->path->items[i].capacity *
                     sizeof(*style->path->items[i].points);
        *bytes = total;
    }
    *bytes += (uint64_t)style->stroke.dash_count * sizeof(double);
    return true;
}
