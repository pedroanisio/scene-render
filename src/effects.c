#include "scene_render/effects.h"
#include "scene_render/color.h"
#include "scene_render/parallel.h"

#include <float.h>
#include <math.h>
#include <stdlib.h>
#include <string.h>

/* Frame and group effects on float premultiplied blend-space pixels. Every
 * worker owns whole rows or column blocks of the processed rectangle and runs a
 * fixed arithmetic order, so the result is identical for any thread count
 * and for any rectangle that contains everything the effect can touch. */

#define TRANSFER_STEPS 4096

/* Small per-pixel helpers and the blur kernels are forced inline: the
 * helpers run several times per pixel, and the kernels are instantiated per
 * channel count and block width so the compiler sees constant trip counts
 * (and vectorizes them). */
#if defined(__GNUC__)
#define FX_INLINE static inline __attribute__((always_inline))
#else
#define FX_INLINE static inline
#endif
#define MAX_RADIUS 64

/* Piecewise-linear transfer tables for linear-light projects, so effects
 * whose parameters are perceptual (grade, vignette, threshold) act on
 * transfer-encoded values as they did before blend space was linear. */
typedef struct {
    bool linear;
    float encode[TRANSFER_STEPS + 1];
    float decode[TRANSFER_STEPS + 1];
} Transfer;

static void transfer_init(Transfer *transfer, const SrProject *project) {
    transfer->linear = project->linear_light;
    if (!transfer->linear) return;
    for (int i = 0; i <= TRANSFER_STEPS; ++i) {
        double v = (double)i / TRANSFER_STEPS;
        transfer->encode[i] = (float)sr_color_encode(v, project->working_color_space);
        transfer->decode[i] = (float)sr_color_decode(v, project->working_color_space);
    }
}

/* Per-thread state reused across effect calls and frames: the transfer
 * tables (a pure function of linear_light and the working color space) and
 * grow-only scratch buffers. Effect calls on one thread are sequential and
 * never nest, and every scratch element an effect reads it first writes, so
 * reuse cannot change a result. Thread-local, so concurrent renders on
 * different threads never share it. */
static _Thread_local Transfer *cached_transfer;
static _Thread_local SrColorSpace cached_space;

#define SCRATCH_SLOTS 2
#define SCRATCH_ALIGN 64

static _Thread_local float *scratch_data[SCRATCH_SLOTS];
static _Thread_local size_t scratch_capacity[SCRATCH_SLOTS];  /* floats */

static const Transfer *transfer_get(const SrProject *project) {
    Transfer *transfer = cached_transfer;
    if (transfer && transfer->linear == project->linear_light &&
        (!transfer->linear || cached_space == project->working_color_space))
        return transfer;
    if (!transfer) {
        transfer = malloc(sizeof(*transfer));
        if (!transfer) return NULL;
        cached_transfer = transfer;
    }
    transfer_init(transfer, project);
    cached_space = project->working_color_space;
    return transfer;
}

/* A buffer of at least `floats` floats in `slot`, contents unspecified. */
static float *scratch_get(size_t slot, size_t floats) {
    if (scratch_capacity[slot] >= floats && scratch_data[slot])
        return scratch_data[slot];
    free(scratch_data[slot]);
    scratch_data[slot] = NULL;
    scratch_capacity[slot] = 0;
    if (floats > (SIZE_MAX - SCRATCH_ALIGN) / sizeof(float)) return NULL;
    size_t bytes = (floats * sizeof(float) + SCRATCH_ALIGN - 1) /
                   SCRATCH_ALIGN * SCRATCH_ALIGN;
    float *data = aligned_alloc(SCRATCH_ALIGN, bytes ? bytes : SCRATCH_ALIGN);
    if (!data) return NULL;
    scratch_data[slot] = data;
    scratch_capacity[slot] = floats;
    return data;
}

void sr_effects_release(void) {
    for (size_t slot = 0; slot < SCRATCH_SLOTS; ++slot) {
        free(scratch_data[slot]);
        scratch_data[slot] = NULL;
        scratch_capacity[slot] = 0;
    }
    free(cached_transfer);
    cached_transfer = NULL;
}

FX_INLINE float lookup(const float *table, float value) {
    if (!(value > 0.0f)) return table[0];
    if (value >= 1.0f) return table[TRANSFER_STEPS];
    float position = value * TRANSFER_STEPS;
    int index = (int)position;
    float t = position - (float)index;
    return table[index] + (table[index + 1] - table[index]) * t;
}

/* Blend value -> perceptual (transfer-encoded) value, clamped to [0,1]. */
FX_INLINE float to_display(const Transfer *transfer, float value) {
    if (transfer->linear) return lookup(transfer->encode, value);
    return value < 0.0f ? 0.0f : (value > 1.0f ? 1.0f : value);
}

FX_INLINE float from_display(const Transfer *transfer, float value) {
    if (transfer->linear) return lookup(transfer->decode, value);
    return value < 0.0f ? 0.0f : (value > 1.0f ? 1.0f : value);
}

/* Effect parameters evaluated at one time. */
typedef struct {
    double intensity, radius, threshold, saturation, contrast, brightness;
    double offset_x, offset_y, relief;
    SrColor color;
} Params;

static Params params_eval(const SrEffect *effect, double time) {
    return (Params){
        .intensity = fmax(0.0, sr_anim_eval(&effect->intensity, time)),
        .radius = sr_anim_eval(&effect->radius, time),
        .threshold = sr_anim_eval(&effect->threshold, time),
        .saturation = sr_anim_eval(&effect->saturation, time),
        .contrast = sr_anim_eval(&effect->contrast, time),
        .brightness = sr_anim_eval(&effect->brightness, time),
        .offset_x = sr_anim_eval(&effect->offset_x, time),
        .offset_y = sr_anim_eval(&effect->offset_y, time),
        .relief = fmax(0.0, sr_anim_eval(&effect->relief, time)),
        .color = sr_anim_color_eval(&effect->color, time)};
}

static int blur_radius(double radius) {
    return sr_clamp_int(radius, 1, MAX_RADIUS);
}

static int shadow_radius(double radius) {
    return sr_clamp_int(floor(radius + 0.5), 0, MAX_RADIUS);
}

int sr_effect_reach(const SrEffect *effect, double time) {
    if (!effect->enabled) return 0;
    Params p = params_eval(effect, time);
    switch (effect->type) {
    case SR_EFFECT_GLOW:
    case SR_EFFECT_BLOOM:
    case SR_EFFECT_BLUR:
        return blur_radius(p.radius);
    case SR_EFFECT_DROP_SHADOW: {
        double shift = fmax(fabs(p.offset_x), fabs(p.offset_y));
        return sr_clamp_int(ceil(shift), 0, 1000000) + shadow_radius(p.radius) + 1;
    }
    default:
        return 0;
    }
}

double sr_light_falloff(SrFalloff falloff, double q) {
    if (!(q < 1.0)) return 0.0;
    if (q < 0.0) q = 0.0;
    switch (falloff) {
    case SR_FALLOFF_LINEAR: return 1.0 - q;
    case SR_FALLOFF_QUADRATIC: return (1.0 - q) * (1.0 - q);
    case SR_FALLOFF_NONE: return 1.0;
    case SR_FALLOFF_SMOOTH:
    default: return (1.0 - q * q) * (1.0 - q * q);
    }
}

/* ---- region helpers ------------------------------------------------------ */

typedef struct {
    SrFrame *frame;
    SrEffectRect rect;
} Region;

static size_t region_width(const Region *region) {
    return (size_t)(region->rect.x1 - region->rect.x0);
}

static size_t region_height(const Region *region) {
    return (size_t)(region->rect.y1 - region->rect.y0);
}

static float *region_px(const Region *region, size_t row, size_t column) {
    return region->frame->px +
           (((size_t)region->rect.y0 + row) * region->frame->width +
            (size_t)region->rect.x0 + column) * 4;
}

/* Effects never leave non-finite color in a frame (a huge light over black
 * computes 0 x inf): such channels become 0; alpha is left as written.
 * Every effect runs this over each pixel of its region after its own last
 * write to that pixel (fused into its final pass). */
FX_INLINE void sanitize_pixels(float *p, size_t count) {
    for (size_t x = 0; x < count; ++x, p += 4)
        for (int c = 0; c < 3; ++c)
            if (!isfinite(p[c])) p[c] = 0.0f;
}

/* ---- blur ---------------------------------------------------------------- */

/* Separable box blur, clamped to the edges, summing in double in a fixed
 * order per output value: the initial window from -radius to +radius, then
 * per step subtract the leaving tap and add the entering one. The
 * horizontal pass writes a scratch buffer; the vertical pass walks blocks
 * of adjacent columns (one running sum per column and channel, each in the
 * same order as a lone column walk) and hands each finished row segment to
 * a sink that merges it into the frame. */
#define BLOCK_COLUMNS 32


typedef struct BlurPass BlurPass;
typedef void (*BlurSink)(const BlurPass *pass, size_t row, size_t column,
                         size_t columns, const float *line);

struct BlurPass {
    const float *source;        /* horizontal: rows `stride` floats apart */
    size_t stride;
    float *target;              /* horizontal output */
    size_t target_stride;       /* floats between target rows */
    size_t width, height, channels;
    int radius;
    BlurSink sink;              /* vertical output */
    const void *sink_context;
    float *output;              /* the sink's frame pixels (prefetch only) */
    size_t output_stride;       /* floats between output rows */
};

static size_t clamp_index(int position, size_t count) {
    return position < 0 ? 0U : position >= (int)count ? count - 1U
                                                      : (size_t)position;
}

FX_INLINE void blur_line(const float *source, float *target, size_t count,
                         int radius, size_t channels) {
    double divisor = radius * 2 + 1;
    double sum[4] = {0.0, 0.0, 0.0, 0.0};
    for (int position = -radius; position <= radius; ++position) {
        const float *s = source + clamp_index(position, count) * channels;
        for (size_t c = 0; c < channels; ++c) sum[c] += s[c];
    }
    ptrdiff_t n = (ptrdiff_t)count, r = radius;
    const float *last = source + (count - 1U) * channels;
    /* i < head: the leaving tap clamps to 0; i < body: the entering tap
     * i + r + 1 is inside; from body on it clamps to the last pixel. */
    ptrdiff_t head = r < n ? r : n;
    ptrdiff_t body = n - r - 1 > head ? n - r - 1 : head;
    ptrdiff_t i = 0;
    for (; i < head; ++i) {
        float *out = target + (size_t)i * channels;
        for (size_t c = 0; c < channels; ++c) out[c] = (float)(sum[c] / divisor);
        const float *add = i + r + 1 < n ? source + (size_t)(i + r + 1) * channels
                                         : last;
        for (size_t c = 0; c < channels; ++c) {
            sum[c] -= source[c];
            sum[c] += add[c];
        }
    }
    for (; i < body; ++i) {
        float *out = target + (size_t)i * channels;
        for (size_t c = 0; c < channels; ++c) out[c] = (float)(sum[c] / divisor);
        const float *remove = source + (size_t)(i - r) * channels;
        const float *add = source + (size_t)(i + r + 1) * channels;
        for (size_t c = 0; c < channels; ++c) {
            sum[c] -= remove[c];
            sum[c] += add[c];
        }
    }
    for (; i < n; ++i) {
        float *out = target + (size_t)i * channels;
        for (size_t c = 0; c < channels; ++c) out[c] = (float)(sum[c] / divisor);
        const float *remove = source + (size_t)(i - r) * channels;
        for (size_t c = 0; c < channels; ++c) {
            sum[c] -= remove[c];
            sum[c] += last[c];
        }
    }
}

static void blur_rows4(void *opaque, size_t begin, size_t end) {
    const BlurPass *pass = opaque;
    for (size_t row = begin; row < end; ++row)
        blur_line(pass->source + row * pass->stride,
                  pass->target + row * pass->target_stride, pass->width,
                  pass->radius, 4);
}

static void blur_rows1(void *opaque, size_t begin, size_t end) {
    const BlurPass *pass = opaque;
    for (size_t row = begin; row < end; ++row)
        blur_line(pass->source + row * pass->stride,
                  pass->target + row * pass->target_stride, pass->width,
                  pass->radius, 1);
}

/* Rows of the vertical pass lie far apart, beyond the hardware
 * prefetchers' reach: the rows needed this many steps ahead are fetched. */
#define BLUR_PREFETCH 6

#if defined(__GNUC__)
#define blur_prefetch(address, write) __builtin_prefetch((address), (write))
#else
#define blur_prefetch(address, write) ((void)(address), (void)(write))
#endif

/* One block of `values` adjacent floats (columns x channels) of the
 * horizontal output, blurred down its rows. */
FX_INLINE void blur_block(const BlurPass *pass, size_t column,
                          size_t columns, size_t values) {
    size_t row_floats = pass->target_stride;
    size_t height = pass->height;
    int radius = pass->radius;
    double divisor = radius * 2 + 1;
    const float *base = pass->target + column * pass->channels;
    double sum[BLOCK_COLUMNS * 4];
    float line[BLOCK_COLUMNS * 4];
    for (size_t j = 0; j < values; ++j) sum[j] = 0.0;
    for (int position = -radius; position <= radius; ++position) {
        const float *s = base + clamp_index(position, height) * row_floats;
        for (size_t j = 0; j < values; ++j) sum[j] += s[j];
    }
    for (size_t row = 0; row < height; ++row) {
        /* Emit the row, then slide the window (on the last row the slide
         * is unused). */
        const float *remove =
            base + clamp_index((int)row - radius, height) * row_floats;
        const float *add =
            base + clamp_index((int)row + radius + 1, height) * row_floats;
        for (size_t j = 0; j < values; ++j) {
            double value = sum[j];
            line[j] = (float)(value / divisor);
            value -= remove[j];
            value += add[j];
            sum[j] = value;
        }
        pass->sink(pass, row, column, columns, line);
        if (row + BLUR_PREFETCH < height) {
            const float *next = base + clamp_index((int)(row + BLUR_PREFETCH) +
                                                   radius + 1, height) * row_floats;
            const float *out = pass->output + (row + BLUR_PREFETCH) *
                               pass->output_stride + column * 4;
            for (size_t j = 0; j < values; j += 16) blur_prefetch(next + j, 0);
            for (size_t j = 0; j < columns * 4; j += 16) blur_prefetch(out + j, 1);
        }
    }
}

static void blur_columns(void *opaque, size_t begin, size_t end) {
    const BlurPass *pass = opaque;
    for (size_t block = begin; block < end; ++block) {
        size_t column = block * BLOCK_COLUMNS;
        size_t columns = pass->width - column;
        if (columns >= BLOCK_COLUMNS) {
            if (pass->channels == 4)
                blur_block(pass, column, BLOCK_COLUMNS, BLOCK_COLUMNS * 4);
            else
                blur_block(pass, column, BLOCK_COLUMNS, BLOCK_COLUMNS);
        } else {
            blur_block(pass, column, columns, columns * pass->channels);
        }
    }
}

/* Floats between rows of the horizontal output: whole cache lines, never
 * a multiple of 4 KiB, so the vertical pass's rows do not all compete for
 * the same cache sets. */
static size_t blur_stride(size_t width, size_t channels) {
    return (width * channels + 15U) / 16U * 16U + 16U;
}

/* Blurs a width x height image with `channels` (1 or 4) floats per pixel
 * whose rows lie `stride` floats apart in `source`, feeding the result row
 * segments to `sink`. `scratch` holds height x blur_stride() floats;
 * `output` is the sink's frame (used only to prefetch). */
static SrStatus blur_run(const float *source, size_t stride, size_t width,
                         size_t height, size_t channels, int radius,
                         float *scratch, BlurSink sink, const void *sink_context,
                         float *output, size_t output_stride, unsigned threads) {
    BlurPass pass = {source, stride, scratch, blur_stride(width, channels),
                     width, height, channels, radius, sink, sink_context,
                     output, output_stride};
    SrStatus status = sr_parallel_for(height, threads,
                                      channels == 4 ? blur_rows4 : blur_rows1,
                                      &pass);
    if (status != SR_OK) return status;
    return sr_parallel_for((width + BLOCK_COLUMNS - 1) / BLOCK_COLUMNS, threads,
                           blur_columns, &pass);
}

typedef struct {
    float *buffer;              /* compact width x height float4 image */
    size_t width;
} StoreContext;

/* Store the blurred row segment back into the compact buffer (the
 * vertical pass reads only the horizontal scratch, so this is safe). */
static void store_sink(const BlurPass *pass, size_t row, size_t column,
                       size_t columns, const float *line) {
    const StoreContext *context = pass->sink_context;
    memcpy(context->buffer + (row * context->width + column) * 4, line,
           columns * 4 * sizeof(float));
}

/* Separable box blur of a compact width x height float4 buffer, clamped to
 * its edges, in place. `buffer` is consumed on failure; on success the
 * result replaces it (*result), as before the column-blocked rewrite. */
static SrStatus blur_compact(float *buffer, uint32_t width, uint32_t height,
                             int radius, unsigned threads, float **result) {
    size_t stride = blur_stride(width, 4);
    float *scratch = NULL;
    if ((size_t)height <= SIZE_MAX / sizeof(float) / stride)
        scratch = aligned_alloc(SCRATCH_ALIGN,
                                ((size_t)height * stride * sizeof(float) +
                                 SCRATCH_ALIGN - 1) / SCRATCH_ALIGN * SCRATCH_ALIGN);
    if (!scratch) {
        free(buffer);
        return SR_ERR_MEMORY;
    }
    StoreContext context = {buffer, width};
    SrStatus status = blur_run(buffer, (size_t)width * 4, width, height, 4,
                               radius, scratch, store_sink, &context, buffer,
                               (size_t)width * 4, threads);
    free(scratch);
    if (status != SR_OK) {
        free(buffer);
        return status;
    }
    *result = buffer;
    return SR_OK;
}

/* Three box passes of radius q: an approximately Gaussian kernel of
 * standard deviation about q. Consumes `buffer`. */
static SrStatus blur_gauss(float *buffer, uint32_t width, uint32_t height,
                           int q, unsigned threads, float **result) {
    SrStatus status = SR_OK;
    for (int pass = 0; pass < 3 && status == SR_OK; ++pass)
        status = blur_compact(buffer, width, height, q, threads, &buffer);
    *result = status == SR_OK ? buffer : NULL;
    return status;
}

SrStatus sr_effects_blur_rect(SrFrame *frame, SrEffectRect *rect,
                              double radius, unsigned threads) {
    double sigma = fmin((double)MAX_RADIUS, fmax(0.0, radius * 0.5));
    if (!(sigma > 0.0)) return SR_OK;
    int lo = (int)floor(sigma), hi = lo + 1;
    if (hi > MAX_RADIUS) hi = MAX_RADIUS;
    float t = (float)(sigma - lo);
    int reach = 3 * hi;
    SrEffectRect grown = {rect->x0 - reach, rect->y0 - reach, rect->x1 + reach,
                          rect->y1 + reach};
    if (grown.x0 < 0) grown.x0 = 0;
    if (grown.y0 < 0) grown.y0 = 0;
    if (grown.x1 > (int)frame->width) grown.x1 = (int)frame->width;
    if (grown.y1 > (int)frame->height) grown.y1 = (int)frame->height;
    if (grown.x1 <= grown.x0 || grown.y1 <= grown.y0) return SR_OK;
    Region region = {frame, grown};
    size_t width = region_width(&region), height = region_height(&region);
    size_t bytes = width * height * 4 * sizeof(float);
    float *low = malloc(bytes), *high = malloc(bytes);
    if (!low || !high) { free(low); free(high); return SR_ERR_MEMORY; }
    for (size_t row = 0; row < height; ++row) {
        memcpy(low + row * width * 4, region_px(&region, row, 0), width * 4 * sizeof(float));
        memcpy(high + row * width * 4, region_px(&region, row, 0), width * 4 * sizeof(float));
    }
    SrStatus status = SR_OK;
    if (lo > 0) status = blur_gauss(low, (uint32_t)width, (uint32_t)height, lo, threads, &low);
    if (status == SR_OK && t > 0.0f && hi > lo)
        status = blur_gauss(high, (uint32_t)width, (uint32_t)height, hi, threads, &high);
    else if (status == SR_OK)
        t = 0.0f;
    if (status != SR_OK) { free(low); free(high); return status; }
    for (size_t row = 0; row < height; ++row) {
        float *d = region_px(&region, row, 0);
        const float *a = low + row * width * 4, *b = high + row * width * 4;
        for (size_t i = 0; i < width * 4; ++i) d[i] = a[i] + (b[i] - a[i]) * t;
    }
    free(low);
    free(high);
    *rect = grown;
    return SR_OK;
}

/* ---- per-pixel effects --------------------------------------------------- */

/* Grade and vignette change each pixel from its own value and position
 * only, so consecutive ones run as one pass: each row gets every effect in
 * order, each followed by its sanitize, which is exactly the sequence of
 * operations separate passes give each pixel. */
#define CHAIN_MAX 8

typedef struct {
    SrEffectType type;
    Params params;
} ChainEffect;

typedef struct {
    const Region *region;
    const Transfer *transfer;
    const double *nx2;          /* vignette: per region column, nx * nx */
    ChainEffect effects[CHAIN_MAX];
    size_t count;
} ChainContext;

static void grade_row(const Transfer *transfer, const Params *params, float *p,
                      size_t width) {
    /* The grade is a pure function of the pixel: a run of identical pixels
     * reuses the previous result. */
    uint32_t last_in[4] = {0, 0, 0, 0};
    float last_out[3] = {0.0f, 0.0f, 0.0f};
    bool have_last = false;
    for (size_t x = 0; x < width; ++x, p += 4) {
        uint32_t in[4];
        memcpy(in, p, sizeof(in));
        if (have_last && memcmp(in, last_in, sizeof(in)) == 0) {
            p[0] = last_out[0]; p[1] = last_out[1]; p[2] = last_out[2];
            continue;
        }
        memcpy(last_in, in, sizeof(in));
        have_last = true;
        float alpha = p[3];
        if (alpha > 0.0f) {
            double original[3];
            for (size_t c = 0; c < 3; ++c)
                original[c] = to_display(transfer, p[c] / alpha);
            double luminance = .2126*original[0] + .7152*original[1] +
                               .0722*original[2];
            for (size_t channel = 0; channel < 3; ++channel) {
                double value = luminance + (original[channel] - luminance) *
                               params->saturation;
                value = (value - .5) * params->contrast + .5 + params->brightness;
                value = original[channel] +
                        (value - original[channel]) * params->intensity;
                p[channel] = from_display(transfer, (float)value) * alpha;
            }
        }
        last_out[0] = p[0]; last_out[1] = p[1]; last_out[2] = p[2];
    }
}

static void vignette_row(const ChainContext *context, const Params *params,
                         size_t y, float *p, size_t width) {
    const SrFrame *frame = context->region->frame;
    double ny = 2.0 * (y + .5) / frame->height - 1.0;
    double ny2 = ny * ny;
    for (size_t x = 0; x < width; ++x, p += 4) {
        double factor = fmax(0.0, 1.0 - params->intensity * .55 *
                             (context->nx2[x] + ny2));
        /* The factor darkens perceptually; in linear light apply its
         * decoded equivalent. */
        float scale = context->transfer->linear
            ? lookup(context->transfer->decode, (float)factor)
            : (float)factor;
        for (size_t channel = 0; channel < 3; ++channel) p[channel] *= scale;
    }
}

static void chain_worker(void *opaque, size_t begin, size_t end) {
    const ChainContext *context = opaque;
    const Region *region = context->region;
    size_t width = region_width(region);
    for (size_t row = begin; row < end; ++row) {
        float *p = region_px(region, row, 0);
        for (size_t k = 0; k < context->count; ++k) {
            const ChainEffect *effect = &context->effects[k];
            if (effect->type == SR_EFFECT_COLOR_GRADE)
                grade_row(context->transfer, &effect->params, p, width);
            else
                vignette_row(context, &effect->params,
                             (size_t)region->rect.y0 + row, p, width);
            sanitize_pixels(p, width);
        }
    }
}

static bool chain_type(SrEffectType type) {
    return type == SR_EFFECT_COLOR_GRADE || type == SR_EFFECT_VIGNETTE;
}

static SrStatus run_chain(ChainContext *context, unsigned threads) {
    const Region *region = context->region;
    size_t width = region_width(region);
    bool vignette = false;
    for (size_t k = 0; k < context->count; ++k)
        vignette = vignette || context->effects[k].type == SR_EFFECT_VIGNETTE;
    if (vignette) {
        double *nx2 = (double *)scratch_get(0, width * 2);
        if (!nx2) return SR_ERR_MEMORY;
        for (size_t x = 0; x < width; ++x) {
            double nx = 2.0 * ((region->rect.x0 + (int)x) + .5) /
                        region->frame->width - 1.0;
            nx2[x] = nx * nx;
        }
        context->nx2 = nx2;
    }
    return sr_parallel_for(region_height(region), threads, chain_worker, context);
}

/* Adds `amount` of a display-referred color to a premultiplied pixel in the
 * transfer-encoded domain, as additive glows were designed. */
FX_INLINE void add_display(const Transfer *transfer, float *p, const float add[3]) {
    float alpha = p[3];
    if (!(alpha > 0.0f)) return;
    for (size_t c = 0; c < 3; ++c)
        p[c] = from_display(transfer, to_display(transfer, p[c] / alpha) + add[c]) *
               alpha;
}

#define FLARE_GHOSTS 4

typedef struct {
    double center_x, center_y, radius;
    int x0, x1, y0, y1;         /* region pixels the ghost may reach */
} FlareGhost;

typedef struct {
    const Region *region;
    const Params *params;
    const Transfer *transfer;
    FlareGhost ghosts[FLARE_GHOSTS];
    int y0;                     /* first processed row */
    bool sanitize_all;          /* false: the region is known finite */
} FlareContext;

/* Each pixel receives the ghosts' contributions in ghost order; a ghost is
 * skipped only outside its bounds, where its distance test fails anyway. */
static void flare_worker(void *opaque, size_t begin, size_t end) {
    FlareContext *context = opaque;
    const Region *region = context->region;
    SrFrame *frame = region->frame;
    const Params *params = context->params;
    for (size_t index = begin; index < end; ++index) {
        int y = context->y0 + (int)index;
        int from = region->rect.x1, to = region->rect.x0;
        bool rows[FLARE_GHOSTS];
        for (int ghost = 0; ghost < FLARE_GHOSTS; ++ghost) {
            const FlareGhost *g = &context->ghosts[ghost];
            rows[ghost] = y >= g->y0 && y < g->y1 && g->x0 < g->x1;
            if (!rows[ghost]) continue;
            if (g->x0 < from) from = g->x0;
            if (g->x1 > to) to = g->x1;
        }
        for (int x = from; x < to; ++x) {
            float add[3] = {0.0f, 0.0f, 0.0f};
            bool touched = false;
            for (int ghost = 0; ghost < FLARE_GHOSTS; ++ghost) {
                const FlareGhost *g = &context->ghosts[ghost];
                if (!rows[ghost] || x < g->x0 || x >= g->x1) continue;
                double dx = x + .5 - g->center_x, dy = y + .5 - g->center_y;
                double distance = sqrt(dx*dx + dy*dy) / g->radius;
                if (distance > 1.0) continue;
                double alpha = (1.0-distance) * params->intensity * .18;
                add[0] += (float)(params->color.r * alpha);
                add[1] += (float)(params->color.g * alpha);
                add[2] += (float)(params->color.b * alpha);
                touched = true;
            }
            if (touched) {
                float *p = &frame->px[((size_t)y * frame->width + (size_t)x) * 4];
                add_display(context->transfer, p, add);
                if (!context->sanitize_all) sanitize_pixels(p, 1);
            }
        }
        if (context->sanitize_all)
            sanitize_pixels(region_px(region, (size_t)(y - region->rect.y0), 0),
                            region_width(region));
    }
}

/* A half-open pixel span [*lo, *hi) of [low, high) holding every pixel
 * whose centre lies within `radius` of `center` (with a margin of two
 * pixels, far beyond any rounding in the distance test). */
static void flare_span(double center, double radius, int low, int high,
                       int *lo, int *hi) {
    *lo = sr_clamp_int(floor(center - radius - 2.0), low, high);
    *hi = sr_clamp_int(ceil(center + radius + 2.0) + 1.0, low, high);
}

static SrStatus lens_flare(const Region *region, const Params *params,
                           const Transfer *transfer, bool finite,
                           unsigned threads) {
    SrFrame *frame = region->frame;
    FlareContext context = {.region = region, .params = params,
                            .transfer = transfer, .y0 = region->rect.y0,
                            .sanitize_all = !finite};
    double flare_radius = blur_radius(params->radius);
    double source_x = frame->width * .72;
    double source_y = frame->height * .28;
    int y0 = region->rect.y1, y1 = region->rect.y0;
    for (int ghost = 0; ghost < FLARE_GHOSTS; ++ghost) {
        FlareGhost *g = &context.ghosts[ghost];
        double t = (ghost + 1) / 5.0;
        g->center_x = source_x + (frame->width*.5-source_x)*t*1.7;
        g->center_y = source_y + (frame->height*.5-source_y)*t*1.7;
        g->radius = fmax(2.0, flare_radius*(1+.35*ghost));
        flare_span(g->center_x, g->radius, region->rect.x0, region->rect.x1,
                   &g->x0, &g->x1);
        flare_span(g->center_y, g->radius, region->rect.y0, region->rect.y1,
                   &g->y0, &g->y1);
        if (g->x0 >= g->x1 || g->y0 >= g->y1) continue;
        if (g->y0 < y0) y0 = g->y0;
        if (g->y1 > y1) y1 = g->y1;
    }
    if (!finite)
        return sr_parallel_for(region_height(region), threads, flare_worker,
                               &context);
    if (y0 >= y1) return SR_OK;
    context.y0 = y0;
    return sr_parallel_for((size_t)(y1 - y0), threads, flare_worker, &context);
}

typedef struct {
    const Region *region;
    const Params *params;
    const Transfer *transfer;
    float mix;                  /* blur */
    float gain;                 /* glow, bloom */
    float color[4];             /* drop shadow: premultiplied color */
    float opacity;
} SinkContext;

/* Blur: mix the blurred row segment into the frame. */
static void blur_sink(const BlurPass *pass, size_t row, size_t column,
                      size_t columns, const float *line) {
    const SinkContext *context = pass->sink_context;
    float *px = region_px(context->region, row, column);
    float mix = context->mix;
    for (size_t i = 0; i < columns * 4; ++i)
        px[i] = px[i] + (line[i] - px[i]) * mix;
    sanitize_pixels(px, columns);
}

/* Glow and bloom threshold and add in the transfer-encoded domain. */
FX_INLINE void glow_pixel(const SinkContext *context, const float *b, float *p,
                          float gain) {
    if (!(b[3] > 0.0f)) return;
    float add[3];
    for (size_t c = 0; c < 3; ++c)
        add[c] = to_display(context->transfer, b[c] / b[3]) * b[3];
    float luminance = (add[0] + add[1] + add[2]) / 3.0f;
    if (luminance < context->params->threshold) return;
    for (size_t c = 0; c < 3; ++c) add[c] *= gain;
    add_display(context->transfer, p, add);
}

static void glow_sink(const BlurPass *pass, size_t row, size_t column,
                      size_t columns, const float *line) {
    const SinkContext *context = pass->sink_context;
    float *px = region_px(context->region, row, column);
    float gain = context->gain;
    /* A pixel's result is a pure function of its blurred and current
     * values: a run of identical pairs reuses the previous result. */
    uint32_t last_in[8] = {0};
    float last_out[3] = {0.0f, 0.0f, 0.0f};
    bool have_last = false;
    for (size_t i = 0; i < columns; ++i) {
        const float *b = &line[i * 4];
        float *p = &px[i * 4];
        uint32_t in[8];
        memcpy(in, b, 4 * sizeof(float));
        memcpy(in + 4, p, 4 * sizeof(float));
        if (have_last && memcmp(in, last_in, sizeof(in)) == 0) {
            p[0] = last_out[0]; p[1] = last_out[1]; p[2] = last_out[2];
            continue;
        }
        memcpy(last_in, in, sizeof(in));
        have_last = true;
        glow_pixel(context, b, p, gain);
        last_out[0] = p[0]; last_out[1] = p[1]; last_out[2] = p[2];
    }
    sanitize_pixels(px, columns);
}

/* ---- drop shadow --------------------------------------------------------- */

typedef struct {
    const Region *region;
    float *shadow;              /* compact, one alpha per pixel */
    double dx, dy;
} ShiftContext;

static float frame_alpha(const SrFrame *frame, int x, int y) {
    if (x < 0 || y < 0 || x >= (int)frame->width || y >= (int)frame->height)
        return 0.0f;
    return frame->px[((size_t)y * frame->width + (size_t)x) * 4 + 3];
}

/* The content's alpha displaced by (dx, dy), bilinear at subpixel offsets,
 * transparent beyond the frame. Only alpha is kept: the shadow's color
 * channels are zero and never read, and each channel blurs on its own. */
static void shift_worker(void *opaque, size_t begin, size_t end) {
    ShiftContext *context = opaque;
    const Region *region = context->region;
    size_t width = region_width(region);
    for (size_t row = begin; row < end; ++row) {
        for (size_t column = 0; column < width; ++column) {
            double sx = (double)region->rect.x0 + (double)column - context->dx;
            double sy = (double)region->rect.y0 + (double)row - context->dy;
            double fx = floor(sx), fy = floor(sy);
            /* Any tap beyond the frame reads 0, so clamping just outside
             * it keeps huge offsets defined without changing the result. */
            int ix = sr_clamp_int(fx, -2, (int)region->frame->width + 1);
            int iy = sr_clamp_int(fy, -2, (int)region->frame->height + 1);
            float tx = (float)(sx - fx), ty = (float)(sy - fy);
            float a = frame_alpha(region->frame, ix, iy) * (1.0f - tx) * (1.0f - ty) +
                      frame_alpha(region->frame, ix + 1, iy) * tx * (1.0f - ty) +
                      frame_alpha(region->frame, ix, iy + 1) * (1.0f - tx) * ty +
                      frame_alpha(region->frame, ix + 1, iy + 1) * tx * ty;
            context->shadow[row * width + column] = a;
        }
    }
}

/* Content over its shadow: out = content + shadow * (1 - content alpha). */
static void under_pixels(const SinkContext *context, float *p,
                         const float *shadow, size_t count) {
    for (size_t column = 0; column < count; ++column, p += 4) {
        float sa = shadow[column] * context->opacity;
        if (!(sa > 0.0f)) continue;
        float k = 1.0f - p[3];
        if (!(k > 0.0f)) continue;
        for (int c = 0; c < 4; ++c) p[c] += context->color[c] * sa * k;
    }
}

static void under_sink(const BlurPass *pass, size_t row, size_t column,
                       size_t columns, const float *line) {
    const SinkContext *context = pass->sink_context;
    float *px = region_px(context->region, row, column);
    under_pixels(context, px, line, columns);
    sanitize_pixels(px, columns);
}

typedef struct {
    const SinkContext *sink;
    const float *shadow;
} UnderContext;

static void under_worker(void *opaque, size_t begin, size_t end) {
    const UnderContext *context = opaque;
    const Region *region = context->sink->region;
    size_t width = region_width(region);
    for (size_t row = begin; row < end; ++row) {
        float *px = region_px(region, row, 0);
        under_pixels(context->sink, px, context->shadow + row * width, width);
        sanitize_pixels(px, width);
    }
}

static SrStatus drop_shadow(const SrScene *scene, const Region *region,
                            const Params *params, unsigned threads) {
    size_t width = region_width(region), height = region_height(region);
    int radius = shadow_radius(params->radius);
    float *shadow = scratch_get(0, width * height);
    float *second = radius > 0 ? scratch_get(1, height * blur_stride(width, 1))
                               : NULL;
    if (!shadow || (radius > 0 && !second)) return SR_ERR_MEMORY;
    SinkContext sink = {.region = region, .params = params,
                        .opacity = (float)fmin(1.0, params->intensity)};
    sr_color_to_blend(&scene->project, params->color, sink.color);
    ShiftContext shift = {region, shadow, params->offset_x, params->offset_y};
    SrStatus status = sr_parallel_for(height, threads, shift_worker, &shift);
    if (status != SR_OK) return status;
    if (radius > 0)
        return blur_run(shadow, width, width, height, 1, radius, second,
                        under_sink, &sink, region_px(region, 0, 0),
                        (size_t)region->frame->width * 4, threads);
    UnderContext under = {&sink, shadow};
    return sr_parallel_for(height, threads, under_worker, &under);
}

/* ---- 2D lighting --------------------------------------------------------- */

typedef struct {
    SrLightType type;
    double x, y, range, height;
    double heading_x, heading_y;          /* spot heading, unit */
    double cos_inner, cos_outer;          /* spot cone */
    double lx, ly, lz;                    /* directional: unit toward light */
    float rgb[3];                         /* blend-space color x intensity */
} Light2D;

typedef struct {
    const Region *region;
    const Light2D *lights;
    size_t count;
    SrFalloff falloff;
    double relief;
    float mix;
} LightContext;

static double smoothstep(double e0, double e1, double x) {
    if (e1 <= e0) return x >= e1 ? 1.0 : 0.0;
    double u = (x - e0) / (e1 - e0);
    u = u < 0.0 ? 0.0 : (u > 1.0 ? 1.0 : u);
    return u * u * (3.0 - 2.0 * u);
}

static float clamped_alpha(const SrFrame *frame, int x, int y) {
    x = x < 0 ? 0 : (x >= (int)frame->width ? (int)frame->width - 1 : x);
    y = y < 0 ? 0 : (y >= (int)frame->height ? (int)frame->height - 1 : y);
    return frame->px[((size_t)y * frame->width + (size_t)x) * 4 + 3];
}

static void light_worker(void *opaque, size_t begin, size_t end) {
    LightContext *context = opaque;
    const Region *region = context->region;
    const SrFrame *frame = region->frame;
    for (size_t row = begin; row < end; ++row) {
        int y = region->rect.y0 + (int)row;
        for (int x = region->rect.x0; x < region->rect.x1; ++x) {
            float *p = &frame->px[((size_t)y * frame->width + (size_t)x) * 4];
            if (!(p[3] > 0.0f)) continue;
            double nx = 0.0, ny = 0.0, nz = 1.0;
            if (context->relief > 0.0) {
                nx = -context->relief * 0.5 * (clamped_alpha(frame, x + 1, y) -
                                               clamped_alpha(frame, x - 1, y));
                ny = -context->relief * 0.5 * (clamped_alpha(frame, x, y + 1) -
                                               clamped_alpha(frame, x, y - 1));
                double n = sqrt(nx * nx + ny * ny + 1.0);
                nx /= n; ny /= n; nz = 1.0 / n;
            }
            double m[3] = {0.0, 0.0, 0.0};
            for (size_t k = 0; k < context->count; ++k) {
                const Light2D *l = &context->lights[k];
                double f = 1.0;
                if (l->type == SR_LIGHT_DIRECTIONAL) {
                    if (context->relief > 0.0)
                        f = fmax(0.0, nx * l->lx + ny * l->ly + nz * l->lz);
                } else if (l->type != SR_LIGHT_AMBIENT) {
                    double dx = x + 0.5 - l->x, dy = y + 0.5 - l->y;
                    double d = sqrt(dx * dx + dy * dy);
                    f = sr_light_falloff(context->falloff, d / l->range);
                    if (f > 0.0 && l->type == SR_LIGHT_SPOT)
                        f *= d > 1e-9
                            ? smoothstep(l->cos_outer, l->cos_inner,
                                         (dx * l->heading_x + dy * l->heading_y) / d)
                            : 1.0;
                    if (f > 0.0 && context->relief > 0.0) {
                        double length = sqrt(d * d + l->height * l->height);
                        if (length > 1e-9)
                            f *= fmax(0.0, (-dx * nx - dy * ny + l->height * nz) / length);
                    }
                }
                m[0] += l->rgb[0] * f;
                m[1] += l->rgb[1] * f;
                m[2] += l->rgb[2] * f;
            }
            for (int c = 0; c < 3; ++c)
                p[c] *= (float)fmin(1.0 + (m[c] - 1.0) * context->mix, FLT_MAX);
        }
        /* Sanitizing leaves alpha, the only channel read across rows. */
        sanitize_pixels(region_px(region, row, 0), region_width(region));
    }
}

static SrStatus lighting(const SrScene *scene, const SrEffect *effect,
                         const Region *region, const Params *params,
                         SrMat3 to_canvas, double time, unsigned threads) {
    Light2D *lights = effect->light_count
        ? sr_alloc(effect->light_count * sizeof(*lights)) : NULL;
    if (effect->light_count && !lights) return SR_ERR_MEMORY;
    double scale = sqrt(fabs(to_canvas.m00 * to_canvas.m11 -
                             to_canvas.m01 * to_canvas.m10));
    double turn = atan2(to_canvas.m10, to_canvas.m00);
    for (size_t k = 0; k < effect->light_count; ++k) {
        const SrLight *light = effect->lights[k];
        Light2D *l = &lights[k];
        l->type = light->type;
        SrVec2 at = sr_mat_point(to_canvas, (SrVec2){sr_anim_eval(&light->x, time),
                                                     sr_anim_eval(&light->y, time)});
        l->x = at.x; l->y = at.y;
        l->range = fmax(1e-9, light->range * scale);
        l->height = sr_anim_eval(&light->z, time) * scale;
        double yaw = sr_anim_eval(&light->yaw, time) * SR_PI / 180.0 + turn;
        double pitch = sr_anim_eval(&light->pitch, time) * SR_PI / 180.0;
        l->heading_x = cos(yaw);
        l->heading_y = sin(yaw);
        double half = fmin(179.0, light->spot_angle) * 0.5 * SR_PI / 180.0;
        double soft = fmin(half, 5.0 * SR_PI / 180.0);
        l->cos_inner = cos(half - soft);
        l->cos_outer = cos(half + soft);
        /* Toward the light: against the heading, raised by the pitch. */
        l->lx = -cos(pitch) * l->heading_x;
        l->ly = -cos(pitch) * l->heading_y;
        l->lz = sin(pitch);
        float color[4];
        sr_color_to_blend(&scene->project,
                          sr_anim_color_eval(&light->color, time), color);
        double intensity = fmax(0.0, sr_anim_eval(&light->intensity, time));
        for (int c = 0; c < 3; ++c)
            l->rgb[c] = (float)fmin(color[c] * intensity, FLT_MAX);
    }
    LightContext context = {region, lights, effect->light_count, effect->falloff,
                            params->relief, (float)fmin(1.0, params->intensity)};
    SrStatus status = sr_parallel_for(region_height(region), threads,
                                      light_worker, &context);
    free(lights);
    return status;
}

/* ---- dispatch ------------------------------------------------------------ */

/* Whether `effect` changes anything over `region` at `time` (*params gets
 * its parameters): every effect is an exact no-op when disabled, at zero
 * intensity or over an empty region. */
static bool effect_active(const SrEffect *effect, double time,
                          const Region *region, Params *params) {
    if (!effect->enabled) return false;
    *params = params_eval(effect, time);
    if (params->intensity <= 0.0) return false;
    return region->rect.x1 > region->rect.x0 && region->rect.y1 > region->rect.y0;
}

/* Runs one active effect over the region, sanitizing it (see
 * sanitize_pixels). `finite` says every color channel in the region is
 * already finite. */
static SrStatus run_effect(const SrScene *scene, const SrEffect *effect,
                           double time, const Region *region, SrMat3 to_canvas,
                           const Params *params_in, const Transfer *transfer,
                           bool finite, unsigned threads) {
    Params params = *params_in;
    size_t rows = region_height(region);
    switch (effect->type) {
    case SR_EFFECT_COLOR_GRADE:
    case SR_EFFECT_VIGNETTE: {
        ChainContext chain = {.region = region, .transfer = transfer,
                              .effects = {{effect->type, params}}, .count = 1};
        return run_chain(&chain, threads);
    }
    case SR_EFFECT_LENS_FLARE:
        return lens_flare(region, &params, transfer, finite, threads);
    case SR_EFFECT_DROP_SHADOW:
        return drop_shadow(scene, region, &params, threads);
    case SR_EFFECT_LIGHTING:
        return lighting(scene, effect, region, &params, to_canvas, time, threads);
    default: {
        size_t width = region_width(region);
        float *scratch = scratch_get(0, rows * blur_stride(width, 4));
        if (!scratch) return SR_ERR_MEMORY;
        SinkContext sink = {.region = region, .params = &params,
                            .transfer = transfer,
                            .mix = (float)fmin(1.0, params.intensity),
                            .gain = (float)(params.intensity * .5)};
        return blur_run(region_px(region, 0, 0), (size_t)region->frame->width * 4,
                        width, rows, 4, blur_radius(params.radius), scratch,
                        effect->type == SR_EFFECT_BLUR ? blur_sink : glow_sink,
                        &sink, region_px(region, 0, 0),
                        (size_t)region->frame->width * 4, threads);
    }
    }
}

SrStatus sr_effects_apply(const SrScene *scene, double time, SrFrame *frame,
                          unsigned threads, SrDiagnostics *diag) {
    (void)diag;
    Region region = {frame, {0, 0, (int)frame->width, (int)frame->height}};
    /* After any effect has run, the frame holds only finite color. */
    bool finite = false;
    size_t index = 0;
    while (index < scene->effect_count) {
        const SrEffect *effect = &scene->effects[index++];
        Params params;
        if (effect->referenced || !effect_active(effect, time, &region, &params))
            continue;
        const Transfer *transfer = transfer_get(&scene->project);
        if (!transfer) return SR_ERR_MEMORY;
        SrStatus status;
        if (chain_type(effect->type)) {
            /* Run it with the per-pixel effects that follow it (skipping
             * no-ops) as one pass. */
            ChainContext chain = {.region = &region, .transfer = transfer,
                                  .effects = {{effect->type, params}},
                                  .count = 1};
            while (index < scene->effect_count && chain.count < CHAIN_MAX) {
                const SrEffect *next = &scene->effects[index];
                Params next_params;
                if (next->referenced ||
                    !effect_active(next, time, &region, &next_params)) {
                    ++index;
                    continue;
                }
                if (!chain_type(next->type)) break;
                chain.effects[chain.count++] = (ChainEffect){next->type,
                                                             next_params};
                ++index;
            }
            status = run_chain(&chain, threads);
        } else {
            status = run_effect(scene, effect, time, &region, sr_mat_identity(),
                                &params, transfer, finite, threads);
        }
        if (status != SR_OK) return status;
        finite = true;
    }
    return SR_OK;
}

SrStatus sr_effects_apply_group(const SrScene *scene, SrEffect *const *effects,
                                size_t count, double time, SrFrame *frame,
                                SrMat3 to_canvas, SrEffectRect *rect,
                                unsigned threads) {
    for (size_t index = 0; index < count; ++index) {
        if (rect->x1 <= rect->x0 || rect->y1 <= rect->y0) break;
        int reach = sr_effect_reach(effects[index], time);
        SrEffectRect grown = {rect->x0 - reach, rect->y0 - reach,
                              rect->x1 + reach, rect->y1 + reach};
        if (grown.x0 < 0) grown.x0 = 0;
        if (grown.y0 < 0) grown.y0 = 0;
        if (grown.x1 > (int)frame->width) grown.x1 = (int)frame->width;
        if (grown.y1 > (int)frame->height) grown.y1 = (int)frame->height;
        Region region = {frame, grown};
        Params params;
        if (effect_active(effects[index], time, &region, &params)) {
            const Transfer *transfer = transfer_get(&scene->project);
            if (!transfer) return SR_ERR_MEMORY;
            SrStatus status = run_effect(scene, effects[index], time, &region,
                                         to_canvas, &params, transfer, false,
                                         threads);
            if (status != SR_OK) return status;
        }
        *rect = grown;
    }
    return SR_OK;
}
