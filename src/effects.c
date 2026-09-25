#include "scene_render/effects.h"
#include "scene_render/color.h"
#include "scene_render/parallel.h"

#include <math.h>
#include <stdlib.h>
#include <string.h>

/* Whole-frame effects on float premultiplied blend-space pixels. Every
 * worker owns whole rows or columns and runs a fixed arithmetic order, so
 * the result is identical for any thread count. */

#define TRANSFER_STEPS 4096

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

static float lookup(const float *table, float value) {
    if (!(value > 0.0f)) return table[0];
    if (value >= 1.0f) return table[TRANSFER_STEPS];
    float position = value * TRANSFER_STEPS;
    int index = (int)position;
    float t = position - (float)index;
    return table[index] + (table[index + 1] - table[index]) * t;
}

/* Blend value -> perceptual (transfer-encoded) value, clamped to [0,1]. */
static float to_display(const Transfer *transfer, float value) {
    if (transfer->linear) return lookup(transfer->encode, value);
    return value < 0.0f ? 0.0f : (value > 1.0f ? 1.0f : value);
}

static float from_display(const Transfer *transfer, float value) {
    if (transfer->linear) return lookup(transfer->decode, value);
    return value < 0.0f ? 0.0f : (value > 1.0f ? 1.0f : value);
}

typedef struct {
    const float *source;
    float *target;
    uint32_t width;
    uint32_t height;
    int radius;
    bool horizontal;
} BlurContext;

static size_t blur_offset(const BlurContext *context, size_t outer,
                          size_t inner) {
    return context->horizontal
        ? (outer * context->width + inner) * 4
        : (inner * context->width + outer) * 4;
}

static size_t clamp_index(int position, size_t count) {
    return position < 0 ? 0U : position >= (int)count ? count - 1U
                                                      : (size_t)position;
}

static void blur_worker(void *opaque, size_t begin, size_t end) {
    BlurContext *context = opaque;
    size_t inner_count = context->horizontal ? context->width : context->height;
    int radius = context->radius;
    double count = radius * 2 + 1;
    for (size_t outer = begin; outer < end; ++outer) {
        double sum[4] = {0};
        for (int position = -radius; position <= radius; ++position) {
            size_t at = blur_offset(context, outer,
                                    clamp_index(position, inner_count));
            for (size_t channel = 0; channel < 4; ++channel)
                sum[channel] += context->source[at + channel];
        }
        for (size_t inner = 0; inner < inner_count; ++inner) {
            size_t out = blur_offset(context, outer, inner);
            for (size_t channel = 0; channel < 4; ++channel)
                context->target[out + channel] = (float)(sum[channel] / count);
            size_t remove_at = blur_offset(context, outer,
                clamp_index((int)inner - radius, inner_count));
            size_t add_at = blur_offset(context, outer,
                clamp_index((int)inner + radius + 1, inner_count));
            for (size_t channel = 0; channel < 4; ++channel) {
                sum[channel] -= context->source[remove_at + channel];
                sum[channel] += context->source[add_at + channel];
            }
        }
    }
}

static SrStatus blur_image(SrFrame *frame, int radius, unsigned threads,
                           float **result) {
    size_t bytes = (size_t)frame->width * frame->height * 4 * sizeof(float);
    float *first = malloc(bytes);
    float *second = malloc(bytes);
    if (!first || !second) {
        free(first); free(second);
        return SR_ERR_MEMORY;
    }
    memcpy(first, frame->px, bytes);
    BlurContext context = {first, second, frame->width, frame->height,
                           radius, true};
    SrStatus status = sr_parallel_for(frame->height, threads, blur_worker,
                                      &context);
    if (status == SR_OK) {
        context = (BlurContext){second, first, frame->width, frame->height,
                                radius, false};
        status = sr_parallel_for(frame->width, threads, blur_worker, &context);
    }
    free(second);
    if (status != SR_OK) {
        free(first);
        return status;
    }
    *result = first;
    return SR_OK;
}

typedef struct {
    SrFrame *frame;
    const SrEffect *effect;
    double intensity;
    const Transfer *transfer;
} PixelContext;

static void grade_worker(void *opaque, size_t begin, size_t end) {
    PixelContext *context = opaque;
    const SrEffect *effect = context->effect;
    for (size_t i = begin; i < end; ++i) {
        float *p = &context->frame->px[i * 4];
        float alpha = p[3];
        if (!(alpha > 0.0f)) continue;
        double original[3];
        for (size_t c = 0; c < 3; ++c)
            original[c] = to_display(context->transfer, p[c] / alpha);
        double luminance = .2126*original[0] + .7152*original[1] +
                           .0722*original[2];
        for (size_t channel = 0; channel < 3; ++channel) {
            double value = luminance + (original[channel] - luminance) *
                           effect->saturation;
            value = (value - .5) * effect->contrast + .5 + effect->brightness;
            value = original[channel] +
                    (value - original[channel]) * context->intensity;
            p[channel] = from_display(context->transfer, (float)value) * alpha;
        }
    }
}

static void vignette_worker(void *opaque, size_t begin, size_t end) {
    PixelContext *context = opaque;
    SrFrame *frame = context->frame;
    for (size_t y = begin; y < end; ++y) {
        for (uint32_t x = 0; x < frame->width; ++x) {
            double nx = 2.0 * (x + .5) / frame->width - 1.0;
            double ny = 2.0 * (y + .5) / frame->height - 1.0;
            double factor = fmax(0.0, 1.0 - context->intensity * .55 *
                                 (nx * nx + ny * ny));
            /* The factor darkens perceptually; in linear light apply its
             * decoded equivalent. */
            float scale = context->transfer->linear
                ? lookup(context->transfer->decode, (float)factor)
                : (float)factor;
            float *p = &frame->px[(y * frame->width + x) * 4];
            for (size_t channel = 0; channel < 3; ++channel) p[channel] *= scale;
        }
    }
}

typedef struct {
    PixelContext pixels;
    double radius;
} FlareContext;

/* Adds `amount` of a display-referred color to a premultiplied pixel in the
 * transfer-encoded domain, as additive glows were designed. */
static void add_display(const Transfer *transfer, float *p, const float add[3]) {
    float alpha = p[3];
    if (!(alpha > 0.0f)) return;
    for (size_t c = 0; c < 3; ++c)
        p[c] = from_display(transfer, to_display(transfer, p[c] / alpha) + add[c]) *
               alpha;
}

static void flare_worker(void *opaque, size_t begin, size_t end) {
    FlareContext *context = opaque;
    SrFrame *frame = context->pixels.frame;
    const SrEffect *effect = context->pixels.effect;
    double source_x = frame->width * .72;
    double source_y = frame->height * .28;
    for (size_t y = begin; y < end; ++y) {
        for (uint32_t x = 0; x < frame->width; ++x) {
            float add[3] = {0.0f, 0.0f, 0.0f};
            bool touched = false;
            for (int ghost = 0; ghost < 4; ++ghost) {
                double t = (ghost + 1) / 5.0;
                double center_x = source_x + (frame->width*.5-source_x)*t*1.7;
                double center_y = source_y + (frame->height*.5-source_y)*t*1.7;
                double radius = fmax(2.0, context->radius*(1+.35*ghost));
                double dx = x + .5 - center_x, dy = y + .5 - center_y;
                double distance = sqrt(dx*dx + dy*dy) / radius;
                if (distance > 1.0) continue;
                double alpha = (1.0-distance) * context->pixels.intensity * .18;
                add[0] += (float)(effect->color.r * alpha);
                add[1] += (float)(effect->color.g * alpha);
                add[2] += (float)(effect->color.b * alpha);
                touched = true;
            }
            if (touched)
                add_display(context->pixels.transfer,
                            &frame->px[(y * frame->width + x) * 4], add);
        }
    }
}

typedef struct {
    SrFrame *frame;
    const float *blurred;
    const SrEffect *effect;
    double intensity;
    const Transfer *transfer;
} MergeContext;

static void merge_worker(void *opaque, size_t begin, size_t end) {
    MergeContext *context = opaque;
    float *px = context->frame->px;
    const float *blurred = context->blurred;
    if (context->effect->type == SR_EFFECT_BLUR) {
        float mix = (float)fmin(1.0, context->intensity);
        for (size_t i = begin * 4; i < end * 4; ++i)
            px[i] = px[i] + (blurred[i] - px[i]) * mix;
        return;
    }
    /* Glow and bloom threshold and add in the transfer-encoded domain. */
    float gain = (float)(context->intensity * .5);
    for (size_t i = begin; i < end; ++i) {
        const float *b = &blurred[i * 4];
        if (!(b[3] > 0.0f)) continue;
        float add[3];
        for (size_t c = 0; c < 3; ++c)
            add[c] = to_display(context->transfer, b[c] / b[3]) * b[3];
        float luminance = (add[0] + add[1] + add[2]) / 3.0f;
        if (luminance < context->effect->threshold) continue;
        for (size_t c = 0; c < 3; ++c) add[c] *= gain;
        add_display(context->transfer, &px[i * 4], add);
    }
}

SrStatus sr_effects_apply(const SrScene *scene, double time, SrFrame *frame,
                          unsigned threads, SrDiagnostics *diag) {
    (void)diag;
    size_t pixels = (size_t)frame->width * frame->height;
    Transfer *transfer = NULL;
    for (size_t index = 0; index < scene->effect_count; ++index) {
        const SrEffect *effect = &scene->effects[index];
        if (!effect->enabled) continue;
        double intensity = fmax(0.0, sr_anim_eval(&effect->intensity, time));
        /* Every effect is an exact no-op at zero intensity. */
        if (intensity <= 0.0) continue;
        if (!transfer) {
            transfer = malloc(sizeof(*transfer));
            if (!transfer) return SR_ERR_MEMORY;
            transfer_init(transfer, &scene->project);
        }
        int radius = (int)fmin(64.0, fmax(1.0,
                         sr_anim_eval(&effect->radius, time)));
        PixelContext context = {frame, effect, intensity, transfer};
        SrStatus status = SR_OK;
        if (effect->type == SR_EFFECT_COLOR_GRADE) {
            status = sr_parallel_for(pixels, threads, grade_worker, &context);
        } else if (effect->type == SR_EFFECT_VIGNETTE) {
            status = sr_parallel_for(frame->height, threads, vignette_worker,
                                     &context);
        } else if (effect->type == SR_EFFECT_LENS_FLARE) {
            FlareContext flare_context = {context, radius};
            status = sr_parallel_for(frame->height, threads, flare_worker,
                                     &flare_context);
        } else {
            float *blurred = NULL;
            status = blur_image(frame, radius, threads, &blurred);
            if (status == SR_OK) {
                MergeContext merge = {frame, blurred, effect, intensity,
                                      transfer};
                status = sr_parallel_for(pixels, threads, merge_worker, &merge);
            }
            free(blurred);
        }
        if (status != SR_OK) {
            free(transfer);
            return status;
        }
    }
    free(transfer);
    return SR_OK;
}
