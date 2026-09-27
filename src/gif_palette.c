/* SPDX-License-Identifier: Apache-2.0 */
/* Deterministic per-frame median-cut palette for GIF output. Every order is
 * a function of colour values (radix sort, value tie breaks); nothing
 * depends on hash order, threads or earlier frames. */
#include "gif_palette_internal.h"

#include <stdlib.h>
#include <string.h>

struct SrGifPalette {
    size_t pixels;
    uint32_t width, height;
    uint32_t *keys, *scratch;   /* pixels entries: 0xRRGGBB */
    uint32_t *colors;           /* distinct keys, ascending */
    uint32_t *counts;           /* pixels per distinct key */
    uint32_t *order;            /* distinct-key indices, grouped by box */
    uint8_t *box_of;            /* palette index per distinct key */
};

typedef struct {
    size_t lo, hi;              /* range in order[] */
    uint64_t weight;            /* pixels in the box */
    int range;                  /* largest channel extent */
    int channel;                /* 0 r, 1 g, 2 b: the widest (ties: r<g<b) */
} Box;

SrStatus sr_gif_palette_create(SrGifPalette **out, uint32_t width,
                               uint32_t height) {
    if (!out) return SR_ERR_ARGUMENT;
    *out = NULL;
    if (!width || !height || (uint64_t)width * height > UINT32_MAX)
        return SR_ERR_ARGUMENT;
    size_t pixels = (size_t)width * height;
    if (pixels > SIZE_MAX / sizeof(uint32_t)) return SR_ERR_ARGUMENT;
    SrGifPalette *p = calloc(1, sizeof(*p));
    if (!p) return SR_ERR_MEMORY;
    p->pixels = pixels;
    p->width = width;
    p->height = height;
    p->keys = sr_alloc(pixels * sizeof(uint32_t));
    p->scratch = sr_alloc(pixels * sizeof(uint32_t));
    p->colors = sr_alloc(pixels * sizeof(uint32_t));
    p->counts = sr_alloc(pixels * sizeof(uint32_t));
    p->order = sr_alloc(pixels * sizeof(uint32_t));
    p->box_of = sr_alloc(pixels);
    if (!p->keys || !p->scratch || !p->colors || !p->counts || !p->order ||
        !p->box_of) {
        sr_gif_palette_free(p);
        return SR_ERR_MEMORY;
    }
    *out = p;
    return SR_OK;
}

void sr_gif_palette_free(SrGifPalette *p) {
    if (!p) return;
    free(p->keys);
    free(p->scratch);
    free(p->colors);
    free(p->counts);
    free(p->order);
    free(p->box_of);
    free(p);
}

/* LSD radix sort of 24-bit keys, three byte passes; stable. */
static void radix_sort(uint32_t *keys, uint32_t *scratch, size_t count) {
    for (unsigned shift = 0; shift < 24; shift += 8) {
        size_t histogram[257] = {0};
        for (size_t i = 0; i < count; ++i) ++histogram[((keys[i] >> shift) & 255) + 1];
        for (size_t b = 1; b < 257; ++b) histogram[b] += histogram[b - 1];
        for (size_t i = 0; i < count; ++i)
            scratch[histogram[(keys[i] >> shift) & 255]++] = keys[i];
        memcpy(keys, scratch, count * sizeof(*keys));
    }
}

static int channel_of(uint32_t key, int channel) {
    return (int)((key >> (16 - 8 * channel)) & 255);
}

static void measure(const SrGifPalette *p, Box *box) {
    int low[3] = {255, 255, 255}, high[3] = {0, 0, 0};
    box->weight = 0;
    for (size_t i = box->lo; i < box->hi; ++i) {
        uint32_t key = p->colors[p->order[i]];
        box->weight += p->counts[p->order[i]];
        for (int c = 0; c < 3; ++c) {
            int v = channel_of(key, c);
            if (v < low[c]) low[c] = v;
            if (v > high[c]) high[c] = v;
        }
    }
    box->range = -1;
    for (int c = 0; c < 3; ++c) {
        if (high[c] - low[c] > box->range) {
            box->range = high[c] - low[c];
            box->channel = c;
        }
    }
}

/* Sorts order[lo, hi) by one channel value, then by the whole key (the
 * keys are distinct), with a counting sort over the channel: stable over
 * an order that is already ascending by key within equal channel values. */
static void sort_box(SrGifPalette *p, const Box *box) {
    size_t histogram[257] = {0};
    size_t count = box->hi - box->lo;
    uint32_t *in = p->order + box->lo, *out = p->scratch;
    /* Restore ascending key order first so the tie break is the key. */
    for (size_t i = 0; i < count; ++i) out[i] = in[i];
    /* Indices ascend with their keys, and there are at most 2^24 distinct
     * 24-bit colours, so the 24-bit radix sort of indices sorts by key. */
    radix_sort(out, in, count);
    for (size_t i = 0; i < count; ++i)
        ++histogram[channel_of(p->colors[out[i]], box->channel) + 1];
    for (size_t b = 1; b < 257; ++b) histogram[b] += histogram[b - 1];
    for (size_t i = 0; i < count; ++i)
        in[histogram[channel_of(p->colors[out[i]], box->channel)]++] = out[i];
}

unsigned sr_gif_palette_quantize(SrGifPalette *p, const uint8_t *rgba,
                                 uint8_t *indices, int stride,
                                 uint32_t entries[256]) {
    size_t n = p->pixels;
    for (size_t i = 0; i < n; ++i) {
        const uint8_t *s = rgba + i * 4;
        p->keys[i] = (uint32_t)s[0] << 16 | (uint32_t)s[1] << 8 | s[2];
    }
    radix_sort(p->keys, p->scratch, n);
    size_t distinct = 0;
    for (size_t i = 0; i < n; ++i) {
        if (distinct && p->colors[distinct - 1] == p->keys[i]) {
            ++p->counts[distinct - 1];
        } else {
            p->colors[distinct] = p->keys[i];
            p->counts[distinct++] = 1;
        }
    }
    for (size_t i = 0; i < 256; ++i) entries[i] = 0xFF000000u;
    unsigned used;
    if (distinct <= 256) {
        for (size_t i = 0; i < distinct; ++i) {
            p->box_of[i] = (uint8_t)i;
            entries[i] = 0xFF000000u | p->colors[i];
        }
        used = (unsigned)distinct;
    } else {
        Box boxes[256];
        for (size_t i = 0; i < distinct; ++i) p->order[i] = (uint32_t)i;
        boxes[0] = (Box){0, distinct, 0, 0, 0};
        measure(p, &boxes[0]);
        used = 1;
        while (used < 256) {
            /* The widest splittable box; ties go to the lowest index. */
            int pick = -1;
            for (unsigned b = 0; b < used; ++b)
                if (boxes[b].hi - boxes[b].lo > 1 &&
                    (pick < 0 || boxes[b].range > boxes[pick].range))
                    pick = (int)b;
            if (pick < 0 || boxes[pick].range == 0) break;
            Box *box = &boxes[pick];
            sort_box(p, box);
            /* Weighted median: first split where the lower part holds at
             * least half the pixels, kept inside (lo, hi). */
            uint64_t half = (box->weight + 1) / 2, sum = 0;
            size_t split = box->lo + 1;
            for (size_t i = box->lo; i < box->hi - 1; ++i) {
                sum += p->counts[p->order[i]];
                split = i + 1;
                if (sum >= half) break;
            }
            Box upper = {split, box->hi, 0, 0, 0};
            box->hi = split;
            measure(p, box);
            measure(p, &upper);
            boxes[used++] = upper;
        }
        for (unsigned b = 0; b < used; ++b) {
            uint64_t sum[3] = {0, 0, 0}, weight = 0;
            for (size_t i = boxes[b].lo; i < boxes[b].hi; ++i) {
                uint32_t index = p->order[i];
                uint64_t count = p->counts[index];
                p->box_of[index] = (uint8_t)b;
                weight += count;
                for (int c = 0; c < 3; ++c)
                    sum[c] += count * (uint64_t)channel_of(p->colors[index], c);
            }
            uint32_t mean[3];
            for (int c = 0; c < 3; ++c)
                mean[c] = (uint32_t)((sum[c] + weight / 2) / weight);
            entries[b] = 0xFF000000u | mean[0] << 16 | mean[1] << 8 | mean[2];
        }
    }
    for (uint32_t y = 0; y < p->height; ++y) {
        for (uint32_t x = 0; x < p->width; ++x) {
            const uint8_t *s = rgba + ((size_t)y * p->width + x) * 4;
            uint32_t key = (uint32_t)s[0] << 16 | (uint32_t)s[1] << 8 | s[2];
            size_t lo = 0, hi = distinct;
            while (hi - lo > 1) {
                size_t mid = lo + (hi - lo) / 2;
                if (p->colors[mid] <= key) lo = mid;
                else hi = mid;
            }
            indices[(size_t)y * (size_t)stride + x] = p->box_of[lo];
        }
    }
    return used;
}
