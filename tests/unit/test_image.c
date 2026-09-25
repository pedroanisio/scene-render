/* SPDX-License-Identifier: Apache-2.0 */
#include "fixture.h"

#include <stdlib.h>

static const float clear[4] = {0, 0, 0, 0};

/* Adds an image layer over a W x H premultiplied image owned by the scene. */
static SrNode *image_layer(SrScene *scene, uint32_t width, uint32_t height,
                           bool opaque)
{
    SrAsset *asset = sr_scene_add_asset(scene);
    SrImage *image = sr_alloc(sizeof(*image));
    float *px = sr_alloc((size_t)width * height * 4 * sizeof(float));
    SrNode *node = fx_add(scene, NULL, SR_NODE_MEDIA);
    if (!asset || !image || !px || !node) {
        free(image);
        free(px);
        return NULL;
    }
    for (size_t i = 0; i < (size_t)width * height; ++i) {
        float alpha = opaque ? 1.0f : (float)((i % 5) + 1) / 5.0f;
        px[i * 4] = (float)((i * 37) % 101) / 100.0f * alpha;
        px[i * 4 + 1] = (float)((i * 11) % 13) / 12.0f * alpha;
        px[i * 4 + 2] = (float)(i % 7) / 6.0f * alpha;
        px[i * 4 + 3] = alpha;
    }
    *image = (SrImage){width, height, px};
    asset->type = SR_ASSET_IMAGE;
    asset->width = width;
    asset->height = height;
    asset->decoded = image;
    node->asset = asset;
    return node;
}

static void test_identity_reproduces_pixels(sr_test_ctx *t)
{
    SrScene scene;
    fx_scene(&scene, 9, 6);
    SrNode *layer = image_layer(&scene, 9, 6, false);
    CHECK(t, layer != NULL);
    SrFrame frame = {0};
    if (layer && fx_render(t, &scene, 0.0, clear, &frame))
        CHECK(t, memcmp(frame.px, layer->asset->decoded->px,
                        (size_t)9 * 6 * 4 * sizeof(float)) == 0);
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

static void test_half_pixel_translation(sr_test_ctx *t)
{
    SrScene scene;
    fx_scene(&scene, 12, 4);
    SrNode *layer = image_layer(&scene, 8, 4, true);
    CHECK(t, layer != NULL);
    if (layer) layer->transform.x.base = 0.5;
    SrFrame frame = {0};
    if (layer && fx_render(t, &scene, 0.0, clear, &frame)) {
        /* The first and last columns are half covered, with coverage
         * falling off smoothly into the transparent border. */
        CHECK_NEAR(t, fx_px(&frame, 0, 1)[3], 0.5, 1e-6);
        CHECK_NEAR(t, fx_px(&frame, 4, 1)[3], 1.0, 1e-6);
        CHECK_NEAR(t, fx_px(&frame, 8, 1)[3], 0.5, 1e-6);
        CHECK_NEAR(t, fx_px(&frame, 9, 1)[3], 0.0, 1e-6);
        const float *texel = &layer->asset->decoded->px[(1 * 8 + 0) * 4];
        CHECK_NEAR(t, fx_px(&frame, 0, 1)[0], texel[0] * 0.5, 1e-6);
    }
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

const sr_test_case sr_tests_image[] = {
    {"identity_reproduces_pixels", test_identity_reproduces_pixels},
    {"half_pixel_translation", test_half_pixel_translation},
    {NULL, NULL},
};
