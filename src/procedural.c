#include "scene_render/procedural.h"
#include "scene_render/color.h"
#include "scene_render/raster.h"
#include "scene_render/vector_path.h"

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
    if (status == SR_OK) {
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
