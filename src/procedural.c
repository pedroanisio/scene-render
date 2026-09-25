#include "scene_render/procedural.h"
#include "scene_render/vector_path.h"

#include <math.h>
#include <stdlib.h>
#include <string.h>

static uint8_t channel(double value) {
    value = fmax(0.0, fmin(1.0, value));
    return (uint8_t)lrint(value * 255.0);
}

static SrStatus allocate_image(SrAsset *asset) {
    size_t pixels = (size_t)asset->width * asset->height;
    if (!asset->width || !asset->height || pixels / asset->height != asset->width ||
        pixels > SIZE_MAX / 4) return SR_ERR_MEMORY;
    asset->decoded = sr_alloc(sizeof(*asset->decoded));
    if (!asset->decoded) return SR_ERR_MEMORY;
    asset->decoded->rgba = sr_alloc(pixels * 4);
    if (!asset->decoded->rgba) { free(asset->decoded); asset->decoded = NULL; return SR_ERR_MEMORY; }
    asset->decoded->width = asset->width;
    asset->decoded->height = asset->height;
    return SR_OK;
}

static SrStatus render_vector(SrAsset *asset, SrDiagnostics *diag) {
    if (asset->vector_shape == SR_SHAPE_PATH)
        return sr_vector_path_render(asset->vector_path, asset->decoded,
                                     asset->color, asset->source_line, diag);
    uint8_t rgba[4] = {channel(asset->color.r), channel(asset->color.g),
                       channel(asset->color.b), channel(asset->color.a)};
    for (uint32_t y = 0; y < asset->height; ++y) for (uint32_t x = 0; x < asset->width; ++x) {
        bool inside = true;
        if (asset->vector_shape == SR_SHAPE_ELLIPSE) {
            double nx = (x + 0.5) / asset->width * 2.0 - 1.0;
            double ny = (y + 0.5) / asset->height * 2.0 - 1.0;
            inside = nx * nx + ny * ny <= 1.0;
        }
        if (inside) memcpy(&asset->decoded->rgba[((size_t)y * asset->width + x) * 4], rgba, 4);
    }
    return SR_OK;
}

SrStatus sr_procedural_asset(SrAsset *asset, SrDiagnostics *diag) {
    SrStatus status = allocate_image(asset);
    if (status != SR_OK) return status;
    if (asset->type == SR_ASSET_VECTOR) return render_vector(asset, diag);
    else {
        sr_diag_error(diag, asset->source_line, "asset", NULL,
                      "unsupported procedural asset type");
        return SR_ERR_ASSET;
    }
    return SR_OK;
}
