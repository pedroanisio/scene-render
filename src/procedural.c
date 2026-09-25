#include "scene_render/procedural.h"

#include <ctype.h>
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

static const uint8_t *glyph(char input) {
    static const uint8_t letters[36][7] = {
        {14,17,17,31,17,17,17},{30,17,17,30,17,17,30},
        {14,17,16,16,16,17,14},{30,17,17,17,17,17,30},
        {31,16,16,30,16,16,31},{31,16,16,30,16,16,16},
        {14,17,16,23,17,17,14},{17,17,17,31,17,17,17},
        {31,4,4,4,4,4,31},{7,2,2,2,18,18,12},
        {17,18,20,24,20,18,17},{16,16,16,16,16,16,31},
        {17,27,21,21,17,17,17},{17,25,21,19,17,17,17},
        {14,17,17,17,17,17,14},{30,17,17,30,16,16,16},
        {14,17,17,17,21,18,13},{30,17,17,30,20,18,17},
        {15,16,16,14,1,1,30},{31,4,4,4,4,4,4},
        {17,17,17,17,17,17,14},{17,17,17,17,17,10,4},
        {17,17,17,21,21,21,10},{17,17,10,4,10,17,17},
        {17,17,10,4,4,4,4},{31,1,2,4,8,16,31},
        {14,17,19,21,25,17,14},{4,12,4,4,4,4,14},
        {14,17,1,2,4,8,31},{30,1,1,14,1,1,30},
        {2,6,10,18,31,2,2},{31,16,16,30,1,1,30},
        {14,16,16,30,17,17,14},{31,1,2,4,8,8,8},
        {14,17,17,14,17,17,14},{14,17,17,15,1,1,14}
    };
    static const uint8_t unknown[7] = {31,17,5,4,4,0,4};
    unsigned char c = (unsigned char)toupper((unsigned char)input);
    if (c >= 'A' && c <= 'Z') return letters[c - 'A'];
    if (c >= '0' && c <= '9') return letters[26 + c - '0'];
    return unknown;
}

static void put(SrAsset *asset, int x, int y) {
    if (x < 0 || y < 0 || x >= (int)asset->width || y >= (int)asset->height) return;
    size_t at = ((size_t)y * asset->width + (size_t)x) * 4;
    asset->decoded->rgba[at] = channel(asset->color.r);
    asset->decoded->rgba[at + 1] = channel(asset->color.g);
    asset->decoded->rgba[at + 2] = channel(asset->color.b);
    asset->decoded->rgba[at + 3] = channel(asset->color.a);
}

static void render_text(SrAsset *asset) {
    int scale = (int)fmax(1.0, floor(asset->text_size / 7.0));
    int cursor = 0;
    for (const char *text = asset->text; *text; ++text) {
        if (*text == ' ') { cursor += 4 * scale; continue; }
        const uint8_t *rows = glyph(*text);
        for (int y = 0; y < 7; ++y) for (int x = 0; x < 5; ++x)
            if (rows[y] & (1U << (4 - x)))
                for (int sy = 0; sy < scale; ++sy)
                    for (int sx = 0; sx < scale; ++sx)
                        put(asset, cursor + x * scale + sx, y * scale + sy);
        cursor += 6 * scale;
        if (cursor >= (int)asset->width) break;
    }
}

static void render_vector(SrAsset *asset) {
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
}

SrStatus sr_procedural_asset(SrAsset *asset, SrDiagnostics *diag) {
    SrStatus status = allocate_image(asset);
    if (status != SR_OK) return status;
    if (asset->type == SR_ASSET_TEXT) render_text(asset);
    else if (asset->type == SR_ASSET_VECTOR) render_vector(asset);
    else {
        sr_diag_error(diag, asset->source_line, "asset", NULL,
                      "unsupported procedural asset type");
        return SR_ERR_ASSET;
    }
    return SR_OK;
}
