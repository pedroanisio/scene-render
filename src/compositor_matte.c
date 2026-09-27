/* SPDX-License-Identifier: Apache-2.0 */
/* Track-matte capture cache and sampling (B1-3). The capture traversal
 * itself is in compositor.c; the dependency order and suppression flags
 * are part of the prepared plan (compositing.c). */
#include "compositor_matte_internal.h"
#include "compositor_coverage_internal.h"
#include "scene_render/raster.h"

#include <math.h>
#include <string.h>

SrStatus sr_matte_frame_create(SrCompositeResources *resources, size_t count,
                               SrMatteFrame **out) {
    *out = NULL;
    SrMatteFrame *frame = sr_composite_alloc(resources, 1, sizeof(*frame), 0);
    if (!frame) return sr_composite_resource_status(resources);
    frame->captures = sr_composite_alloc(resources, count ? count : 1,
                                         sizeof(*frame->captures), 0);
    if (!frame->captures) {
        sr_composite_free(resources, frame);
        return sr_composite_resource_status(resources);
    }
    frame->count = count;
    for (size_t i = 0; i < count; ++i) frame->captures[i] = (SrMatteCapture){0};
    *out = frame;
    return SR_OK;
}

void sr_matte_frame_free(SrCompositeResources *resources, SrMatteFrame *frame) {
    if (!frame) return;
    for (size_t i = 0; i < frame->count; ++i)
        sr_composite_free(resources, frame->captures[i].coverage);
    sr_composite_free(resources, frame->captures);
    sr_composite_free(resources, frame);
}

SrStatus sr_matte_capture_store(SrCompositeResources *resources,
                                const SrLumaConfig *luma, const float *rgba,
                                uint32_t width, uint32_t height, SrClip dirty,
                                float scale, SrMatteCapture *capture) {
    *capture = (SrMatteCapture){.width = width, .height = height,
                                .dirty = dirty, .rendered = true};
    if (dirty.x1 <= dirty.x0 || dirty.y1 <= dirty.y0 || !(scale > 0.0f)) return SR_OK;
    uint64_t pixels = (uint64_t)width * height;
    /* Allocation zeroing plus reduction of the dirty rectangle. */
    uint64_t area = (uint64_t)(dirty.x1 - dirty.x0) * (uint64_t)(dirty.y1 - dirty.y0);
    if (!sr_composite_work(resources, area, 48)) return SR_ERR_RENDER;
    capture->coverage = sr_composite_alloc(resources, (size_t)pixels,
                                           2 * sizeof(float), pixels * 2);
    if (!capture->coverage) return sr_composite_resource_status(resources);
    for (int y = dirty.y0; y < dirty.y1; ++y) {
        for (int x = dirty.x0; x < dirty.x1; ++x) {
            size_t index = (size_t)y * width + (size_t)x;
            const float *px = rgba + index * 4;
            float alpha = px[3] < 0.0f ? 0.0f : px[3] > 1.0f ? 1.0f : px[3];
            capture->coverage[index * 2] = alpha * scale;
            capture->coverage[index * 2 + 1] =
                (float)(alpha * sr_luma_px(luma, px)) * scale;
        }
    }
    return SR_OK;
}

float sr_matte_empty_value(SrMatteMode mode) {
    return mode == SR_MATTE_ALPHA_INVERTED || mode == SR_MATTE_LUMA_INVERTED
        ? 1.0f : 0.0f;
}

static float texel(const SrMatteCapture *capture, int64_t x, int64_t y,
                   size_t channel) {
    if (x < capture->dirty.x0 || y < capture->dirty.y0 ||
        x >= capture->dirty.x1 || y >= capture->dirty.y1)
        return 0.0f;
    return capture->coverage[((size_t)y * capture->width + (size_t)x) * 2 + channel];
}

float sr_matte_sample(const SrMaskLink *link, double cx, double cy) {
    const SrMatteCapture *capture = link->matte;
    SrMatteMode mode = link->matte_mode;
    bool inverted = mode == SR_MATTE_ALPHA_INVERTED || mode == SR_MATTE_LUMA_INVERTED;
    size_t channel = mode == SR_MATTE_LUMA || mode == SR_MATTE_LUMA_INVERTED;
    double value = 0.0;
    if (capture->coverage) {
        double sx = cx, sy = cy;
        bool mapped = true;
        if (link->plane) {
            const SrPlaneMap *plane = link->plane;
            double u = (cx - 1.0) / plane->s + plane->u0;
            double v = (cy - 1.0) / plane->s + plane->v0;
            mapped = sr_card_to_screen(plane->pose, u, v, &sx, &sy) &&
                     isfinite(sx) && isfinite(sy);
        }
        double gx = sx - 0.5, gy = sy - 0.5;
        if (mapped && gx > -1.0 && gy > -1.0 && gx < (double)capture->width &&
            gy < (double)capture->height) {
            double fx = floor(gx), fy = floor(gy);
            int64_t ix = (int64_t)fx, iy = (int64_t)fy;
            double tx = gx - fx, ty = gy - fy;
            if (tx == 0.0 && ty == 0.0) {
                value = texel(capture, ix, iy, channel);
            } else {
                double top = texel(capture, ix, iy, channel) * (1.0 - tx) +
                             texel(capture, ix + 1, iy, channel) * tx;
                double bottom = texel(capture, ix, iy + 1, channel) * (1.0 - tx) +
                                texel(capture, ix + 1, iy + 1, channel) * tx;
                value = top * (1.0 - ty) + bottom * ty;
            }
        }
    }
    if (inverted) value = 1.0 - value;
    return (float)(value < 0.0 ? 0.0 : value > 1.0 ? 1.0 : value);
}

float sr_link_special_coverage(const SrMaskLink *link, double cx, double cy) {
    if (link->matte) return sr_matte_sample(link, cx, cy);
    SrVec2 local = {link->inverse.m00 * cx + link->inverse.m01 * cy + link->inverse.m02,
                    link->inverse.m10 * cx + link->inverse.m11 * cy + link->inverse.m12};
    return sr_coverage_sample(link->grid, local.x, local.y);
}
