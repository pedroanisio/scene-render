/* SPDX-License-Identifier: Apache-2.0 */
/* Advanced mask coverage (B1-3): node-local scalar rasters combined in
 * authored order. See docs/design/b1-3-compositing.md, "Masks".
 *
 * For every participating mask (mode != none): antialiased shape coverage
 * at one sample per local pixel, then expansion (grayscale disk
 * morphology), then feather (three box passes approximating a Gaussian of
 * sigma = feather/2), then inversion, then opacity. The accumulator starts
 * at one for intersect/subtract/darken and zero for add/lighten/difference
 * and combines each mask B into A as:
 *   intersect A*B, add A+B-A*B, subtract A*(1-B), lighten max, darken min,
 *   difference |A-B|.
 * Outside every mask's shape-plus-filter extent each mask is constant, so
 * the combination is a constant exterior value and only the part of the
 * receiving clip inside that union is stored. */
#include "compositor_coverage_internal.h"
#include "compositing_internal.h"
#include "compositor_evaluation_internal.h"
#include "mask_outline_internal.h"
#include "scene_render/raster.h"

#include <float.h>
#include <math.h>
#include <stdlib.h>
#include <string.h>

void sr_mask_path_free(SrMaskPath *path) {
    if (!path) return;
    sr_prepared_path_free(&path->path);
    free(path);
}

SrStatus sr_mask_path_prepare(SrMask *mask, uint64_t available_bytes,
                               SrPathParseInfo *info) {
    sr_mask_path_free(mask->prepared);
    mask->prepared = NULL;
    if (info) *info = (SrPathParseInfo){SR_PATH_PARSE_ARGUMENT, 0, 0};
    if (!mask->path) return SR_ERR_ASSET;
    if (available_bytes < sizeof(SrMaskPath)) {
        if (info) info->error = SR_PATH_PARSE_STORAGE;
        return SR_ERR_ASSET;
    }
    SrMaskPath *prepared = sr_alloc(sizeof(*prepared));
    if (!prepared) {
        if (info) info->error = SR_PATH_PARSE_MEMORY;
        return SR_ERR_MEMORY;
    }
    SrPathParseInfo local;
    SrStatus status = sr_prepared_mask_path_parse(mask->path,
        available_bytes - sizeof(SrMaskPath), &prepared->path, &local);
    if (info) *info = local;
    if (status != SR_OK) {
        sr_mask_path_free(prepared);
        return status;
    }
    prepared->owned_bytes = local.owned_bytes + sizeof(SrMaskPath);
    if (info) info->owned_bytes = prepared->owned_bytes;
    mask->prepared = prepared;
    return SR_OK;
}

static bool animated_or(const SrAnimValue *value, double base) {
    return value->track.count || value->base != base;
}

bool sr_mask_advanced(const SrMask *mask) {
    return mask->type > SR_MASK_ROUNDED_RECT ||
        mask->mode != SR_MASK_MODE_INTERSECT ||
        (mask->opacity_set && animated_or(&mask->opacity, 1.0)) ||
        (!mask->opacity_set && mask->opacity.track.count) ||
        animated_or(&mask->feather, 0.0) || animated_or(&mask->expansion, 0.0);
}

bool sr_node_masks_advanced(const SrNode *node) {
    for (size_t i = 0; i < node->mask_count; ++i)
        if (sr_mask_advanced(&node->masks[i])) return true;
    return false;
}

const char *sr_mask_validate(const SrMask *mask, const char **message) {
    *message = NULL;
    bool polygonal = mask->type == SR_MASK_POLYGON || mask->type == SR_MASK_STAR;
    bool simple = mask->type <= SR_MASK_ROUNDED_RECT;
    if (mask->type > SR_MASK_STAR) {
        *message = "unknown mask type";
        return "type";
    }
    if ((unsigned)mask->mode > SR_MASK_MODE_NONE) {
        *message = "unknown mask mode";
        return "mode";
    }
    if (mask->fill_rule != SR_FILL_EVENODD && mask->fill_rule != SR_FILL_NONZERO) {
        *message = "unknown fill rule";
        return "fillRule";
    }
    if (mask->opacity_set && !(mask->opacity.base >= 0.0 && mask->opacity.base <= 1.0)) {
        *message = "mask opacity must be in [0,1]";
        return "opacity";
    }
    if (!(mask->feather.base >= 0.0 && mask->feather.base <= SR_MAX_MASK_FILTER_RADIUS)) {
        *message = "mask feather must be in [0,4096] local px";
        return "feather";
    }
    if (!(fabs(mask->expansion.base) <= SR_MAX_MASK_FILTER_RADIUS)) {
        *message = "mask expansion must be within +-4096 local px";
        return "expansion";
    }
    if (!(fabs(mask->x.base) <= SR_MAX_MASK_COORDINATE) ||
        !(fabs(mask->y.base) <= SR_MAX_MASK_COORDINATE)) {
        *message = "mask coordinates must be within +-1e9";
        return "x/y";
    }
    if (mask->type != SR_MASK_PATH && mask->path) {
        *message = "path is valid only for type=\"path\"";
        return "path";
    }
    if (!polygonal && mask->points && mask->points != 5) {
        *message = "points is valid only for polygon and star masks";
        return "points";
    }
    if (mask->type != SR_MASK_STAR && mask->inner_radius_set) {
        *message = "innerRadius is valid only for star masks";
        return "innerRadius";
    }
    /* Simple kinds keep their legacy dimension policy (the loader requires
     * positive sizes; a direct-C empty rect covers nothing). */
    if (simple) return NULL;
    if (mask->box_set && (!(mask->width.base > 0.0) || !(mask->height.base > 0.0))) {
        *message = "width and height must be positive and given together";
        return "width/height";
    }
    if (mask->type == SR_MASK_PATH) {
        if (!mask->path || !*mask->path) {
            *message = "a path mask requires a nonempty path";
            return "path";
        }
        return NULL;
    }
    if (mask->points < 3 || mask->points > SR_MAX_MASK_VERTICES) {
        *message = "points must be in [3,4096]";
        return "points";
    }
    if (!(mask->radius.base > 0.0 || mask->radius.track.count) ||
        !(mask->radius.base <= SR_MAX_MASK_COORDINATE)) {
        *message = "polygon and star masks require a positive radius";
        return "radius";
    }
    if (mask->type == SR_MASK_STAR && mask->inner_radius_set &&
        !mask->inner_radius.track.count &&
        !(mask->inner_radius.base >= 0.0 &&
          mask->inner_radius.base <= mask->radius.base)) {
        *message = "innerRadius must be in [0, radius]";
        return "innerRadius";
    }
    return NULL;
}

/* ---- evaluation ---------------------------------------------------------- */

typedef struct {
    const SrMask *mask;
    double x, y, width, height, radius, inner;
    double opacity, feather, expansion;
    double box[4];              /* local shape extent before filtering */
    int halo;                   /* filter reach in local px */
} MaskValue;

static bool fail_mask(SrCompositeResources *resources, const SrMask *mask,
                       const char *attribute, const char *reason) {
    sr_composite_owner(resources, (SrCompositeOwner){mask->source_line, "mask",
                                                      attribute});
    return sr_composite_resource_fail(resources, SR_ERR_RENDER, reason);
}

static bool path_bounds(SrCompositeResources *resources, const SrPreparedPath *path,
                         double box[4]) {
    box[0] = box[1] = DBL_MAX;
    box[2] = box[3] = -DBL_MAX;
    for (size_t c = 0; c < path->count; ++c) {
        if (!sr_composite_work(resources, path->items[c].count + 1, 2)) return false;
        for (size_t i = 0; i < path->items[c].count; ++i) {
            const SrPathPoint *p = &path->items[c].points[i];
            box[0] = fmin(box[0], p->x); box[1] = fmin(box[1], p->y);
            box[2] = fmax(box[2], p->x); box[3] = fmax(box[3], p->y);
        }
    }
    return box[0] <= box[2];
}

static bool evaluate(SrCompositeResources *resources, const SrMask *mask,
                     const SrMaskGeometry *geometry, double time, MaskValue *out) {
    *out = (MaskValue){.mask = mask};
    SrCompositeOwner previous = sr_composite_owner(resources,
        (SrCompositeOwner){mask->source_line, "mask",
                           "opacity/feather/expansion/innerRadius"});
    const SrAnimValue *values[] = {&mask->opacity, &mask->feather,
                                   &mask->expansion, &mask->inner_radius,
                                   &mask->radius};
    bool valid = sr_composite_work(resources, 1, 32);
    for (size_t i = 0; valid && i < sizeof(values) / sizeof(values[0]); ++i)
        valid = sr_composite_anim_work(resources, values[i], false);
    if (valid && !geometry) {
        const SrAnimValue *coordinates[] = {&mask->x, &mask->y, &mask->width,
                                            &mask->height};
        for (size_t i = 0; valid && i < 4; ++i)
            valid = sr_composite_anim_work(resources, coordinates[i], false);
    }
    sr_composite_owner(resources, previous);
    if (!valid) return false;
    out->x = geometry ? geometry->x : sr_anim_eval(&mask->x, time);
    out->y = geometry ? geometry->y : sr_anim_eval(&mask->y, time);
    out->width = geometry ? geometry->width : fmax(0.0, sr_anim_eval(&mask->width, time));
    out->height = geometry ? geometry->height : fmax(0.0, sr_anim_eval(&mask->height, time));
    out->radius = fmax(0.0, sr_anim_eval(&mask->radius, time));
    out->opacity = mask->opacity_set || mask->opacity.track.count
        ? sr_anim_eval(&mask->opacity, time) : 1.0;
    out->feather = sr_anim_eval(&mask->feather, time);
    out->expansion = sr_anim_eval(&mask->expansion, time);
    if (!isfinite(out->opacity) || out->opacity < 0.0 || out->opacity > 1.0)
        return fail_mask(resources, mask, "opacity",
                         "evaluated mask opacity must be in [0,1]");
    if (!isfinite(out->feather) || out->feather < 0.0 ||
        out->feather > SR_MAX_MASK_FILTER_RADIUS)
        return fail_mask(resources, mask, "feather",
                         "evaluated mask feather must be in [0,4096]");
    if (!isfinite(out->expansion) ||
        fabs(out->expansion) > SR_MAX_MASK_FILTER_RADIUS)
        return fail_mask(resources, mask, "expansion",
                         "evaluated mask expansion must be within +-4096");
    double limit = SR_MAX_MASK_COORDINATE;
    if (!(fabs(out->x) <= limit) || !(fabs(out->y) <= limit) ||
        !(out->width <= limit) || !(out->height <= limit) ||
        !(out->radius <= limit))
        return fail_mask(resources, mask, "x/y/width/height/radius",
                         "evaluated mask geometry must be within +-1e9");
    bool polygonal = mask->type == SR_MASK_POLYGON || mask->type == SR_MASK_STAR;
    if (polygonal) {
        if (!(out->radius > 0.0))
            return fail_mask(resources, mask, "radius",
                             "evaluated polygon/star radius must be positive");
        out->inner = mask->inner_radius_set || mask->inner_radius.track.count
            ? sr_anim_eval(&mask->inner_radius, time) : 0.5 * out->radius;
        if (mask->type == SR_MASK_STAR &&
            (!isfinite(out->inner) || out->inner < 0.0 || out->inner > out->radius))
            return fail_mask(resources, mask, "innerRadius",
                             "evaluated innerRadius must be in [0, radius]");
        out->box[0] = out->x - out->radius; out->box[1] = out->y - out->radius;
        out->box[2] = out->x + out->radius; out->box[3] = out->y + out->radius;
    } else if (mask->type == SR_MASK_PATH) {
        const SrMaskPath *prepared = mask->prepared;
        if (!prepared || !prepared->path.count)
            return fail_mask(resources, mask, "path",
                             "path mask requires compositing preparation");
        double bounds[4];
        if (!path_bounds(resources, &prepared->path, bounds)) {
            if (resources->status != SR_OK) return false;
            bounds[0] = bounds[1] = bounds[2] = bounds[3] = 0.0;
        }
        out->box[0] = out->x + bounds[0]; out->box[1] = out->y + bounds[1];
        out->box[2] = out->x + bounds[2]; out->box[3] = out->y + bounds[3];
    } else {
        out->box[0] = out->x; out->box[1] = out->y;
        out->box[2] = out->x + out->width; out->box[3] = out->y + out->height;
    }
    if (mask->type > SR_MASK_ROUNDED_RECT && mask->box_set) {
        out->box[0] = fmax(out->box[0], out->x);
        out->box[1] = fmax(out->box[1], out->y);
        out->box[2] = fmin(out->box[2], out->x + out->width);
        out->box[3] = fmin(out->box[3], out->y + out->height);
    }
    /* Antialiasing reaches one local pixel beyond the outline. */
    out->box[0] -= 1.0; out->box[1] -= 1.0;
    out->box[2] += 1.0; out->box[3] += 1.0;
    double reach = ceil(fabs(out->expansion)) + 3.0 * ceil(out->feather * 0.5) + 1.0;
    out->halo = (int)reach;
    return true;
}

/* ---- combination ---------------------------------------------------------- */

static double combine(SrMaskMode mode, double a, double b) {
    switch (mode) {
    case SR_MASK_MODE_ADD: return a + b - a * b;
    case SR_MASK_MODE_SUBTRACT: return a * (1.0 - b);
    case SR_MASK_MODE_LIGHTEN: return fmax(a, b);
    case SR_MASK_MODE_DARKEN: return fmin(a, b);
    case SR_MASK_MODE_DIFFERENCE: return fabs(a - b);
    case SR_MASK_MODE_INTERSECT:
    default: return a * b;
    }
}

static double initial(SrMaskMode mode) {
    return mode == SR_MASK_MODE_ADD || mode == SR_MASK_MODE_LIGHTEN ||
           mode == SR_MASK_MODE_DIFFERENCE ? 0.0 : 1.0;
}

static double unit(double value) {
    return value < 0.0 ? 0.0 : value > 1.0 ? 1.0 : value;
}

/* ---- local rasters -------------------------------------------------------- */

typedef struct {
    int64_t x0, y0, x1, y1;     /* local pixel rectangle */
} Rect;

static uint64_t rect_width(Rect r) { return r.x1 > r.x0 ? (uint64_t)(r.x1 - r.x0) : 0; }
static uint64_t rect_height(Rect r) { return r.y1 > r.y0 ? (uint64_t)(r.y1 - r.y0) : 0; }

static Rect rect_intersect(Rect a, Rect b) {
    Rect r = {a.x0 > b.x0 ? a.x0 : b.x0, a.y0 > b.y0 ? a.y0 : b.y0,
              a.x1 < b.x1 ? a.x1 : b.x1, a.y1 < b.y1 ? a.y1 : b.y1};
    if (r.x1 < r.x0) r.x1 = r.x0;
    if (r.y1 < r.y0) r.y1 = r.y0;
    return r;
}

static Rect rect_grow(Rect r, int64_t margin) {
    return (Rect){r.x0 - margin, r.y0 - margin, r.x1 + margin, r.y1 + margin};
}

/* Local extents are within 1e9 plus bounded filter reach, far inside int64. */
static Rect rect_from(const double box[4]) {
    return (Rect){(int64_t)floor(box[0]), (int64_t)floor(box[1]),
                  (int64_t)ceil(box[2]), (int64_t)ceil(box[3])};
}

static void *scratch(SrCompositeResources *resources, uint64_t count) {
    if (count > SIZE_MAX / sizeof(float)) {
        sr_composite_resource_fail(resources, SR_ERR_RENDER,
                                    "mask coverage allocation overflow");
        return NULL;
    }
    return sr_composite_alloc(resources, (size_t)count, sizeof(float), count);
}

static double clamp_to(double value, double high) {
    return value < 0.0 ? 0.0 : value > high ? high : value;
}

/* Conservative accumulation work of every edge of `path` translated by
 * (dx, dy) on a w x h grid: accumulate_edge visits at most ceil(|dy|) + 2
 * clipped rows and, across them, at most |dx| + 3 columns per row plus the
 * columns between; charge 16 units per row and 4 per column step. */
static uint64_t edge_work(const SrPreparedPath *path, double dx, double dy,
                          uint64_t w, uint64_t h) {
    uint64_t total = 0;
    for (size_t c = 0; c < path->count; ++c) {
        const SrPathContour *contour = &path->items[c];
        for (size_t i = 0; i < contour->count; ++i) {
            const SrPathPoint *a = &contour->points[i];
            const SrPathPoint *b = &contour->points[(i + 1) % contour->count];
            double ya = clamp_to(a->y + dy, (double)h), yb = clamp_to(b->y + dy, (double)h);
            double xa = clamp_to(a->x + dx, (double)w), xb = clamp_to(b->x + dx, (double)w);
            uint64_t rows = (uint64_t)ceil(fabs(yb - ya)) + 2;
            if (rows > h + 1) rows = h + 1;
            uint64_t columns = (uint64_t)ceil(fabs(xb - xa)) + 3 * rows;
            total += 16 * rows + 4 * columns + 48;  /* up to 3 clipped pieces */
        }
    }
    return total;
}

static bool raster_shape(SrCompositeResources *resources, const MaskValue *value,
                          Rect q, float *out) {
    const SrMask *mask = value->mask;
    uint64_t w = rect_width(q), h = rect_height(q);
    if (mask->type <= SR_MASK_ROUNDED_RECT) {
        if (!sr_composite_work(resources, w * h, 16)) return false;
        for (uint64_t j = 0; j < h; ++j) {
            double ly = (double)(q.y0 + (int64_t)j) + 0.5;
            for (uint64_t i = 0; i < w; ++i) {
                double lx = (double)(q.x0 + (int64_t)i) + 0.5;
                out[j * w + i] = sr_distance_coverage_inline(
                    sr_shape_distance_inline(mask->type, value->x, value->y,
                        value->width, value->height, value->radius, lx, ly), 1.0);
            }
        }
        return true;
    }
    SrPreparedPath local = {0};
    SrPathContour contour = {0};
    const SrPreparedPath *path = NULL;
    SrPathPoint *vertices = NULL;
    double dx = value->x - (double)q.x0, dy = value->y - (double)q.y0;
    uint64_t points = 0;
    if (mask->type == SR_MASK_PATH) {
        path = &mask->prepared->path;
        for (size_t c = 0; c < path->count; ++c) points += path->items[c].count;
    } else {
        bool star = mask->type == SR_MASK_STAR;
        size_t count = sr_mask_outline_count(star, mask->points);
        vertices = sr_composite_alloc(resources, count, sizeof(*vertices), 0);
        if (!vertices) return false;
        if (!sr_composite_work(resources, count, 16)) {
            sr_composite_free(resources, vertices);
            return false;
        }
        sr_mask_outline(star, mask->points, 0.0, 0.0, value->radius,
                        value->inner, vertices);
        contour = (SrPathContour){vertices, count, count, true};
        local = (SrPreparedPath){&contour, 1, 1};
        path = &local;
        points = count;
    }
    /* Resolve scans every cell; the edge charge covers each edge's clipped
     * row span and the columns it crosses in those rows. */
    float *cells = NULL;
    bool ok = sr_composite_work(resources, points, 8) &&
              sr_composite_work(resources, edge_work(path, dx, dy, w, h), 1) &&
              sr_composite_work(resources, (w + 2) * h, 4) &&
              (cells = scratch(resources, (w + 2) * h)) != NULL;
    if (ok)
        ok = sr_prepared_path_fill_offset(path, mask->fill_rule, dx, dy,
                                          (uint32_t)w, (uint32_t)h, cells,
                                          out) == SR_OK;
    if (!ok && resources->status == SR_OK)
        sr_composite_resource_fail(resources, SR_ERR_RENDER,
                                    "mask path rasterization failed");
    sr_composite_free(resources, cells);
    sr_composite_free(resources, vertices);
    if (ok && mask->box_set) {
        if (!sr_composite_work(resources, w * h, 16)) return false;
        for (uint64_t j = 0; j < h; ++j) {
            double ly = (double)(q.y0 + (int64_t)j) + 0.5;
            for (uint64_t i = 0; i < w; ++i) {
                double lx = (double)(q.x0 + (int64_t)i) + 0.5;
                out[j * w + i] *= sr_distance_coverage_inline(
                    sr_shape_distance_inline(SR_MASK_RECT, value->x, value->y,
                        value->width, value->height, 0.0, lx, ly), 1.0);
            }
        }
    }
    return ok;
}

/* Sliding extremum over [x - half, x + half] of one row, reading the
 * transparent exterior (zero) outside it: van Herk / Gil-Werman blocks of
 * size k = 2 * half + 1. pad/g/s hold width + 2 * half floats. */
static void window_extreme(const float *in, size_t width, size_t half, bool dilate,
                           float *pad, float *g, float *s, float *out) {
    size_t k = 2 * half + 1, n = width + 2 * half;
    for (size_t j = 0; j < n; ++j)
        pad[j] = j >= half && j < half + width ? in[j - half] : 0.0f;
    for (size_t j = 0; j < n; ++j) {
        if (j % k == 0) g[j] = pad[j];
        else g[j] = dilate ? fmaxf(g[j - 1], pad[j]) : fminf(g[j - 1], pad[j]);
    }
    for (size_t j = n; j-- > 0;) {
        if (j % k == k - 1 || j == n - 1) s[j] = pad[j];
        else s[j] = dilate ? fmaxf(s[j + 1], pad[j]) : fminf(s[j + 1], pad[j]);
    }
    for (size_t x = 0; x < width; ++x)
        out[x] = dilate ? fmaxf(s[x], g[x + k - 1]) : fminf(s[x], g[x + k - 1]);
}

/* Grayscale morphology with the integer Euclidean disk dx^2 + dy^2 <= r^2:
 * each disk row is a horizontal sliding extremum; rows combine in fixed
 * dy order. Radius zero is the identity. */
static bool morphology(SrCompositeResources *resources, const float *in, float *out,
                        uint64_t w, uint64_t h, uint64_t r, bool dilate) {
    if (r == 0) {
        memcpy(out, in, (size_t)(w * h) * sizeof(float));
        return true;
    }
    uint64_t n = w + 2 * r;
    if (!sr_composite_work(resources, w * h, (2 * r + 1) * 12)) return false;
    float *rows = scratch(resources, 4 * n);
    if (!rows) return false;
    float *pad = rows, *g = rows + n, *s = rows + 2 * n, *line = rows + 3 * n;
    for (uint64_t y = 0; y < h; ++y) {
        float *target = out + y * w;
        for (uint64_t x = 0; x < w; ++x) target[x] = dilate ? 0.0f : 1.0f;
        for (int64_t dy = -(int64_t)r; dy <= (int64_t)r; ++dy) {
            int64_t source = (int64_t)y + dy;
            uint64_t remaining = r * r - (uint64_t)(dy * dy);
            uint64_t half = (uint64_t)sqrt((double)remaining);
            while (half * half > remaining) --half;
            while ((half + 1) * (half + 1) <= remaining) ++half;
            if (source < 0 || source >= (int64_t)h) {
                if (!dilate) for (uint64_t x = 0; x < w; ++x) target[x] = 0.0f;
                continue;
            }
            window_extreme(in + (uint64_t)source * w, (size_t)w, (size_t)half,
                           dilate, pad, g, s, line);
            for (uint64_t x = 0; x < w; ++x)
                target[x] = dilate ? fmaxf(target[x], line[x])
                                   : fminf(target[x], line[x]);
        }
    }
    sr_composite_free(resources, rows);
    return true;
}

/* One box pass of radius q along rows then columns, zero exterior, with
 * double running sums. */
static void box_pass(float *buffer, float *tmp, uint64_t w, uint64_t h, uint64_t q) {
    double scale = 1.0 / (double)(2 * q + 1);
    for (uint64_t y = 0; y < h; ++y) {
        const float *row = buffer + y * w;
        float *target = tmp + y * w;
        double sum = 0.0;
        for (uint64_t x = 0; x < q && x < w; ++x) sum += row[x];
        for (uint64_t x = 0; x < w; ++x) {
            if (x + q < w) sum += row[x + q];
            if (x > q) sum -= row[x - q - 1];
            target[x] = (float)(sum * scale);
        }
    }
    for (uint64_t x = 0; x < w; ++x) {
        double sum = 0.0;
        for (uint64_t y = 0; y < q && y < h; ++y) sum += tmp[y * w + x];
        for (uint64_t y = 0; y < h; ++y) {
            if (y + q < h) sum += tmp[(y + q) * w + x];
            if (y > q) sum -= tmp[(y - q - 1) * w + x];
            buffer[y * w + x] = (float)(sum * scale);
        }
    }
}

static void box_gauss(float *buffer, float *tmp, uint64_t w, uint64_t h, uint64_t q) {
    for (int pass = 0; pass < 3; ++pass) box_pass(buffer, tmp, w, h, q);
}

/* Three box passes approximating a Gaussian of sigma = feather / 2; the
 * fractional part blends the neighbouring integer radii, as the group blur
 * does. Zero feather is the identity. */
static bool feather(SrCompositeResources *resources, float *values, uint64_t w,
                    uint64_t h, double amount) {
    double sigma = amount * 0.5;
    if (!(sigma > 0.0)) return true;
    uint64_t lo = (uint64_t)floor(sigma), hi = lo + 1;
    double t = sigma - (double)lo;
    uint64_t passes = (lo > 0) + (t > 0.0);
    if (!sr_composite_work(resources, w * h, 24 * passes + 2)) return false;
    float *tmp = scratch(resources, w * h);
    float *high = t > 0.0 ? scratch(resources, w * h) : NULL;
    if (!tmp || (t > 0.0 && !high)) {
        sr_composite_free(resources, tmp);
        sr_composite_free(resources, high);
        return false;
    }
    if (high) memcpy(high, values, (size_t)(w * h) * sizeof(float));
    if (lo > 0) box_gauss(values, tmp, w, h, lo);
    if (high) {
        box_gauss(high, tmp, w, h, hi);
        float blend = (float)t;
        for (uint64_t i = 0; i < w * h; ++i)
            values[i] = values[i] + (high[i] - values[i]) * blend;
    }
    sr_composite_free(resources, tmp);
    sr_composite_free(resources, high);
    return true;
}

/* Shape coverage, then expansion, then feather, over the local rect q. */
static bool mask_raster(SrCompositeResources *resources, const MaskValue *value,
                         Rect q, float *out) {
    uint64_t w = rect_width(q), h = rect_height(q);
    if (!raster_shape(resources, value, q, out)) return false;
    double radius = fabs(value->expansion);
    if (radius > 0.0) {
        bool dilate = value->expansion > 0.0;
        uint64_t r0 = (uint64_t)floor(radius), r1 = (uint64_t)ceil(radius);
        double t = radius - (double)r0;
        float *big = scratch(resources, w * h);
        float *small = r0 != r1 ? scratch(resources, w * h) : NULL;
        bool ok = big && (r0 == r1 || small) &&
            morphology(resources, out, big, w, h, r1, dilate) &&
            (r0 == r1 || morphology(resources, out, small, w, h, r0, dilate));
        if (ok) {
            if (r0 == r1) {
                memcpy(out, big, (size_t)(w * h) * sizeof(float));
            } else {
                float blend = (float)t;
                for (uint64_t i = 0; i < w * h; ++i)
                    out[i] = small[i] + (big[i] - small[i]) * blend;
            }
        }
        sr_composite_free(resources, big);
        sr_composite_free(resources, small);
        if (!ok) return false;
    }
    return feather(resources, out, w, h, value->feather);
}

/* ---- grid ------------------------------------------------------------------ */

void sr_coverage_free(SrCompositeResources *resources, SrCoverageGrid *grid) {
    if (!grid) return;
    sr_composite_free(resources, grid->values);
    *grid = (SrCoverageGrid){0};
}

static bool dimension_ok(SrCompositeResources *resources, Rect r) {
    if (rect_width(r) > SR_MAX_COVERAGE_DIMENSION ||
        rect_height(r) > SR_MAX_COVERAGE_DIMENSION)
        return sr_composite_resource_fail(resources, SR_ERR_RENDER,
            "mask coverage dimension limit is 16384 local px including halo");
    return true;
}

SrStatus sr_coverage_build(SrCompositeResources *resources, const SrScene *scene,
                           const SrNode *node, const SrLengthFrame *lengths,
                           double time, SrMat3 inverse, SrClip clip,
                           SrDiagnostics *diag, SrCoverageGrid *grid) {
    (void)scene;
    (void)diag;
    *grid = (SrCoverageGrid){.exterior = 1.0f};
    const SrNodeGeometry *geometry = sr_length_node(lengths, node);
    MaskValue *values = sr_composite_alloc(resources, node->mask_count,
                                            sizeof(*values), 0);
    if (!values) return sr_composite_resource_status(resources);
    SrStatus status = SR_OK;
    bool any = false;
    double exterior = 1.0;
    double union_box[4] = {DBL_MAX, DBL_MAX, -DBL_MAX, -DBL_MAX};
    for (size_t i = 0; i < node->mask_count; ++i) {
        const SrMask *mask = &node->masks[i];
        const SrMaskGeometry *evaluated = geometry
            ? &lengths->masks[geometry->mask_offset + i] : NULL;
        if (!evaluate(resources, mask, evaluated, time, &values[i])) {
            status = SR_ERR_RENDER;
            goto done;
        }
        if (mask->mode == SR_MASK_MODE_NONE) continue;
        if (!any) exterior = initial(mask->mode);
        any = true;
        double e = mask->invert ? values[i].opacity : 0.0;
        exterior = unit(combine(mask->mode, exterior, e));
        const MaskValue *v = &values[i];
        union_box[0] = fmin(union_box[0], v->box[0] - v->halo);
        union_box[1] = fmin(union_box[1], v->box[1] - v->halo);
        union_box[2] = fmax(union_box[2], v->box[2] + v->halo);
        union_box[3] = fmax(union_box[3], v->box[3] + v->halo);
    }
    grid->exterior = (float)exterior;
    if (!any) goto done;
    /* Receiving clip pixel edges in local space, plus bilinear support. */
    double corners[4][2] = {{clip.x0, clip.y0}, {clip.x1, clip.y0},
                            {clip.x0, clip.y1}, {clip.x1, clip.y1}};
    double local[4] = {DBL_MAX, DBL_MAX, -DBL_MAX, -DBL_MAX};
    for (size_t i = 0; i < 4; ++i) {
        SrVec2 p = sr_mat_point(inverse, (SrVec2){corners[i][0], corners[i][1]});
        if (!isfinite(p.x) || !isfinite(p.y)) {
            sr_composite_resource_fail(resources, SR_ERR_RENDER,
                                        "nonfinite mask coverage mapping");
            status = SR_ERR_RENDER;
            goto done;
        }
        local[0] = fmin(local[0], p.x); local[1] = fmin(local[1], p.y);
        local[2] = fmax(local[2], p.x); local[3] = fmax(local[3], p.y);
    }
    local[0] = fmax(local[0] - 1.0, union_box[0]);
    local[1] = fmax(local[1] - 1.0, union_box[1]);
    local[2] = fmin(local[2] + 1.0, union_box[2]);
    local[3] = fmin(local[3] + 1.0, union_box[3]);
    if (!(local[0] < local[2]) || !(local[1] < local[3])) goto done;
    Rect g = rect_from(local);
    if (!dimension_ok(resources, g)) {
        status = SR_ERR_RENDER;
        goto done;
    }
    uint64_t w = rect_width(g), h = rect_height(g);
    float *accumulator = scratch(resources, w * h);
    if (!accumulator) {
        status = sr_composite_resource_status(resources);
        goto done;
    }
    *grid = (SrCoverageGrid){accumulator, (int)g.x0, (int)g.y0, (uint32_t)w,
                             (uint32_t)h, (float)exterior};
    bool first = true;
    for (size_t i = 0; i < node->mask_count; ++i) {
        const MaskValue *v = &values[i];
        const SrMask *mask = v->mask;
        if (mask->mode == SR_MASK_MODE_NONE) continue;
        sr_composite_owner(resources, (SrCompositeOwner){mask->source_line, "mask",
            "type/feather/expansion"});
        Rect q = rect_intersect(rect_grow(g, v->halo),
                                rect_grow(rect_from(v->box), v->halo));
        if (!dimension_ok(resources, q)) {
            status = SR_ERR_RENDER;
            goto done;
        }
        uint64_t qw = rect_width(q), qh = rect_height(q);
        float *shape = NULL;
        if (qw && qh) {
            shape = scratch(resources, qw * qh);
            if (!shape || !mask_raster(resources, v, q, shape)) {
                sr_composite_free(resources, shape);
                status = sr_composite_resource_status(resources);
                goto done;
            }
        }
        if (!sr_composite_work(resources, w * h, 8)) {
            sr_composite_free(resources, shape);
            status = SR_ERR_RENDER;
            goto done;
        }
        double start = first ? initial(mask->mode) : 0.0;
        for (uint64_t j = 0; j < h; ++j) {
            int64_t ly = g.y0 + (int64_t)j;
            for (uint64_t i2 = 0; i2 < w; ++i2) {
                int64_t lx = g.x0 + (int64_t)i2;
                double c = 0.0;
                if (shape && lx >= q.x0 && lx < q.x1 && ly >= q.y0 && ly < q.y1)
                    c = shape[(uint64_t)(ly - q.y0) * qw + (uint64_t)(lx - q.x0)];
                c = unit(c);
                if (mask->invert) c = 1.0 - c;
                c = unit(c * v->opacity);
                float *a = &accumulator[j * w + i2];
                double previous = first ? start : *a;
                *a = (float)unit(combine(mask->mode, previous, c));
            }
        }
        first = false;
        sr_composite_free(resources, shape);
    }
done:
    sr_composite_free(resources, values);
    if (status != SR_OK) sr_coverage_free(resources, grid);
    return status;
}

static float grid_at(const SrCoverageGrid *grid, int64_t i, int64_t j) {
    if (i < 0 || j < 0 || i >= (int64_t)grid->width || j >= (int64_t)grid->height)
        return grid->exterior;
    return grid->values[(size_t)j * grid->width + (size_t)i];
}

float sr_coverage_sample(const SrCoverageGrid *grid, double lx, double ly) {
    if (!grid->width || !grid->height) return grid->exterior;
    double gx = lx - (double)grid->x0 - 0.5, gy = ly - (double)grid->y0 - 0.5;
    if (!(gx > -1.0 && gx < (double)grid->width &&
          gy > -1.0 && gy < (double)grid->height))
        return grid->exterior;
    double fx = floor(gx), fy = floor(gy);
    int64_t ix = (int64_t)fx, iy = (int64_t)fy;
    double tx = gx - fx, ty = gy - fy;
    double top = grid_at(grid, ix, iy) * (1.0 - tx) + grid_at(grid, ix + 1, iy) * tx;
    double bottom = grid_at(grid, ix, iy + 1) * (1.0 - tx) +
                    grid_at(grid, ix + 1, iy + 1) * tx;
    return (float)(top * (1.0 - ty) + bottom * ty);
}

void sr_coverage_extent(const SrCoverageGrid *grid, double box[4]) {
    box[0] = grid->x0;
    box[1] = grid->y0;
    box[2] = (double)grid->x0 + grid->width;
    box[3] = (double)grid->y0 + grid->height;
}
