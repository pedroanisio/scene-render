#include "scene_render/procedural.h"
#include "scene_render/color.h"
#include "scene_render/raster.h"
#include "scene_render/vector_path.h"
#include "scene_render/paint.h"
#include "vector_shape_internal.h"

#include <math.h>
#include <stdlib.h>
#include <string.h>

/* Rect and ellipse vector assets fill the asset box; coverage is the
 * signed-distance estimate at one pixel of anti-aliasing and the stroke is
 * centred on the outline (its outer half is clipped by the asset bounds). */
static void shape_coverage(const SrAsset *asset, float *fill, float *stroke) {
    SrMaskType type = asset->vector_shape == SR_SHAPE_ELLIPSE
                          ? SR_MASK_ELLIPSE : SR_MASK_RECT;
    double half = asset->vector_stroke_width * 0.5;
    for (uint32_t y = 0; y < asset->height; ++y) {
        for (uint32_t x = 0; x < asset->width; ++x) {
            double sd = sr_shape_distance(type, 0.0, 0.0, asset->width,
                                          asset->height, 0.0, x + 0.5, y + 0.5);
            size_t at = (size_t)y * asset->width + x;
            fill[at] = sr_distance_coverage(sd, 1.0);
            if (stroke) stroke[at] = sr_distance_coverage(fabs(sd) - half, 1.0);
        }
    }
}

/* B1-4 vector assets: the shape renderer's geometry, stroke styles and
 * paints, rasterized once in asset pixels (identity map, scale 1). Paints
 * are static (checked at load) and dithered in asset pixel coordinates. */
static SrStatus extended_asset(const SrProject *project, const SrAsset *asset,
                               float *px, SrDiagnostics *diag) {
    const SrVectorExtension *ext = asset->vector_ext;
    uint32_t w = asset->width, h = asset->height;
    size_t pixels = (size_t)w * h;
    SrPreparedPath path = {0};
    SrShapeParams shape = {.type = asset->vector_shape, .width = w, .height = h,
                           .points = ext->points, .path = &path, .scale = 1.0};
    for (int i = 0; i < 4; ++i) shape.radii[i] = ext->radius;
    shape.outer_radius = 0.5 * (w < h ? w : h);
    shape.inner_radius = ext->inner_radius_set ? ext->inner_radius : 0.5 * shape.outer_radius;
    if (shape.inner_radius > shape.outer_radius) shape.inner_radius = shape.outer_radius;
    SrStatus status = SR_OK;
    if (asset->vector_shape == SR_SHAPE_PATH) {
        SrPathParseInfo info;
        status = sr_prepared_mask_path_parse(asset->vector_path, UINT64_MAX, &path, &info);
        if (status == SR_ERR_ASSET)
            sr_diag_error(diag, asset->source_line, "vector", "path",
                          "invalid or oversized path at byte %zu", info.byte_offset);
        if (status != SR_OK) return status;
    }
    SrPaintEval *evals = sr_alloc(2 * sizeof(*evals));
    float *fill = sr_alloc(pixels * sizeof(float));
    float *stroke = sr_alloc(pixels * sizeof(float));
    float *cells = sr_alloc(((size_t)w + 2) * h * sizeof(float));
    SrPolySet fill_set, stroke_set;
    sr_polyset_init(&fill_set, NULL, SR_MAX_SHAPE_VERTICES);
    sr_polyset_init(&stroke_set, NULL, SR_MAX_SHAPE_VERTICES);
    if (!evals || !fill || !stroke || !cells) status = SR_ERR_MEMORY;
    const SrPaint *fill_paint = ext->fill_paint.paint, *stroke_paint = ext->stroke_paint.paint;
    if (status == SR_OK && fill_paint)
        status = sr_paint_eval(fill_paint, project, 0.0, w, h, &evals[0], diag);
    if (status == SR_OK && stroke_paint)
        status = sr_paint_eval(stroke_paint, project, 0.0, w, h, &evals[1], diag);
    double width = asset->vector_stroke_width;
    if (ext->stroke.position != SR_STROKE_CENTER) width *= 2.0;
    SrStrokeParams params = {.width = width, .cap = ext->stroke.cap,
                             .join = ext->stroke.join,
                             .miter_limit = ext->stroke.miter_limit,
                             .dash = ext->stroke.dash,
                             .dash_count = ext->stroke.dash_count,
                             .dash_offset = ext->stroke.dash_offset.base,
                             .scale = 1.0};
    bool ok = status == SR_OK && sr_shape_build(&fill_set, &shape) &&
              sr_polyset_seal(&fill_set) &&
              sr_stroke_build(&fill_set, &params, &stroke_set) &&
              sr_polyset_seal(&stroke_set);
    if (status == SR_OK && !ok) {
        const SrPolySet *bad = fill_set.status != SR_OK ? &fill_set : &stroke_set;
        status = bad->status == SR_ERR_MEMORY ? SR_ERR_MEMORY : SR_ERR_ASSET;
        if (status == SR_ERR_ASSET)
            sr_diag_error(diag, asset->source_line, "vector", NULL, "%s",
                          bad->error ? bad->error : "invalid vector geometry");
    }
    const SrPathMap identity = {{1, 0, 0, 0, 1, 0}};
    if (status == SR_OK)
        status = sr_path_rasterize(fill_set.contours, fill_set.range_count, &identity,
                                   asset->vector_fill_rule, w, h, cells, fill);
    if (status == SR_OK && stroke_set.range_count)
        status = sr_path_rasterize(stroke_set.contours, stroke_set.range_count,
                                   &identity, SR_FILL_NONZERO, w, h, cells, stroke);
    if (status == SR_OK && !stroke_set.range_count)
        memset(stroke, 0, pixels * sizeof(float));
    if (status == SR_OK && ext->stroke.position != SR_STROKE_CENTER)
        sr_coverage_clip(stroke, fill, pixels, ext->stroke.position == SR_STROKE_INSIDE);
    if (status == SR_OK) {
        float fill_px[4], stroke_px[4];
        sr_color_to_blend(project, asset->color, fill_px);
        sr_color_to_blend(project, asset->vector_stroke, stroke_px);
        bool line = asset->vector_shape == SR_SHAPE_LINE;
        for (uint32_t y = 0; y < h; ++y)
            for (uint32_t x = 0; x < w; ++x) {
                size_t i = (size_t)y * w + x;
                float cf = line ? 0.0f : fill[i], cs = stroke[i];
                float f[4], s[4];
                if (fill_paint) sr_paint_sample(&evals[0], x + 0.5, y + 0.5, (int)x, (int)y, f);
                else memcpy(f, fill_px, sizeof(f));
                if (stroke_paint) sr_paint_sample(&evals[1], x + 0.5, y + 0.5, (int)x, (int)y, s);
                else memcpy(s, stroke_px, sizeof(s));
                float *out = &px[i * 4];
                if (ext->stroke.order == SR_PAINT_FILL_STROKE) {
                    float keep = 1.0f - s[3] * cs;
                    for (int c = 0; c < 4; ++c) out[c] = s[c] * cs + f[c] * cf * keep;
                } else {
                    float keep = 1.0f - f[3] * cf;
                    for (int c = 0; c < 4; ++c) out[c] = f[c] * cf + s[c] * cs * keep;
                }
            }
    }
    sr_polyset_free(&stroke_set);
    sr_polyset_free(&fill_set);
    sr_prepared_path_free(&path);
    free(evals);
    free(fill);
    free(stroke);
    free(cells);
    return status;
}

SrStatus sr_procedural_asset(const SrProject *project, SrAsset *asset,
                             SrDiagnostics *diag) {
    if (asset->type != SR_ASSET_VECTOR) {
        sr_diag_error(diag, asset->source_line, "asset", NULL,
                      "unsupported procedural asset type");
        return SR_ERR_ASSET;
    }
    size_t pixels = (size_t)asset->width * asset->height;
    if (!asset->width || !asset->height || pixels / asset->height != asset->width ||
        pixels > SIZE_MAX / (4 * sizeof(float)))
        return SR_ERR_MEMORY;
    SrVectorStyle style = {asset->vector_fill_rule, asset->color,
                           asset->vector_stroke, asset->vector_stroke_width};
    bool stroked = style.stroke_width > 0.0;
    float *fill = sr_alloc(pixels * sizeof(float));
    float *stroke = stroked ? sr_alloc(pixels * sizeof(float)) : NULL;
    float *px = sr_alloc(pixels * 4 * sizeof(float));
    SrImage *image = sr_alloc(sizeof(*image));
    SrStatus status = px && fill && image && (!stroked || stroke)
                          ? SR_OK : SR_ERR_MEMORY;
    /* Vector colors are working-space values, like every XML color; they are
     * converted once and the stroke is composed over the fill in blend
     * space, so linear-light projects blend vector edges in linear light. */
    float fill_px[4], stroke_px[4];
    sr_color_to_blend(project, style.fill, fill_px);
    sr_color_to_blend(project, style.stroke, stroke_px);
    if (status == SR_OK && asset->vector_ext && asset->vector_ext->extended) {
        status = extended_asset(project, asset, px, diag);
    } else if (status == SR_OK) {
        if (asset->vector_shape == SR_SHAPE_PATH) {
            status = sr_vector_path_render(asset->vector_path, &style, fill_px,
                                           stroke_px, asset->width,
                                           asset->height, px,
                                           asset->source_line, diag);
        } else {
            shape_coverage(asset, fill, stroke);
            sr_vector_compose(fill, stroke, fill_px, stroke_px, pixels, px);
        }
    }
    free(fill);
    free(stroke);
    if (status != SR_OK) {
        free(px);
        free(image);
        return status;
    }
    image->width = asset->width;
    image->height = asset->height;
    image->px = px;
    asset->decoded = image;
    return SR_OK;
}
