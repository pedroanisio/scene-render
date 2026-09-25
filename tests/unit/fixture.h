/* SPDX-License-Identifier: Apache-2.0 */
/* Small scene-graph fixtures shared by the compositor-level suites. */
#ifndef SR_TEST_FIXTURE_H
#define SR_TEST_FIXTURE_H

#include "scene_render/compositor.h"
#include "scene_render/scene.h"

#include "harness.h"

/* A width x height non-linear sRGB project, so blend values equal the XML
 * color values. */
static inline void fx_scene(SrScene *scene, uint32_t width, uint32_t height)
{
    sr_scene_init(scene);
    scene->project.width = width;
    scene->project.height = height;
    scene->project.linear_light = false;
    scene->project.working_color_space = SR_COLOR_SRGB;
}

static inline SrNode *fx_add(SrScene *scene, SrNode *parent, SrNodeType type)
{
    SrNode *node = sr_node_create(scene, type);
    if (node && sr_node_add_child(parent ? parent : scene->root, node) != SR_OK) {
        sr_node_free(node);
        node = NULL;
    }
    return node;
}

static inline SrNode *fx_rect(SrScene *scene, SrNode *parent, double x,
                              double y, double width, double height,
                              SrColor fill, double opacity)
{
    SrNode *node = fx_add(scene, parent, SR_NODE_SHAPE);
    if (!node) return NULL;
    node->shape = SR_SHAPE_RECT;
    node->transform.x.base = x;
    node->transform.y.base = y;
    node->shape_width = width;
    node->shape_height = height;
    node->fill = fill;
    node->opacity.base = opacity;
    return node;
}

static inline SrMask fx_mask(SrMaskType type, double x, double y,
                             double width, double height, bool invert)
{
    SrMask mask = {0};
    mask.type = type;
    mask.invert = invert;
    mask.x.base = x;
    mask.y.base = y;
    mask.width.base = width;
    mask.height.base = height;
    return mask;
}

/* Renders the scene at `time` over a frame cleared to `background`
 * (premultiplied). The caller frees the frame. */
static inline bool fx_render(sr_test_ctx *t, SrScene *scene, double time,
                             const float background[4], SrFrame *frame)
{
    if (sr_frame_init(frame, scene->project.width, scene->project.height) != SR_OK) {
        SR_FAIL(t, "frame allocation failed");
        return false;
    }
    sr_frame_clear(frame, background, 1);
    SrStatus status = sr_composite_scene(scene, time, frame, NULL);
    CHECK(t, status == SR_OK);
    return status == SR_OK;
}

static inline const float *fx_px(const SrFrame *frame, uint32_t x, uint32_t y)
{
    return &frame->px[((size_t)y * frame->width + x) * 4];
}

#endif
