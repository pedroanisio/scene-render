#include "scene_render/effects.h"
#include "scene_render/parallel.h"

#include <math.h>
#include <stdlib.h>
#include <string.h>

static uint8_t byte_value(double value) {
    return (uint8_t)lrint(fmax(0.0, fmin(1.0, value)) * 255.0);
}

typedef struct {
    const uint8_t *source;
    uint8_t *target;
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

static void blur_worker(void *opaque, size_t begin, size_t end) {
    BlurContext *context = opaque;
    size_t inner_count = context->horizontal ? context->width : context->height;
    int radius = context->radius;
    int count = radius * 2 + 1;
    for (size_t outer = begin; outer < end; ++outer) {
        int sum[4] = {0};
        for (int position = -radius; position <= radius; ++position) {
            size_t sample = position < 0 ? 0U :
                position >= (int)inner_count ? inner_count - 1U :
                (size_t)position;
            size_t at = blur_offset(context, outer, sample);
            for (size_t channel = 0; channel < 4; ++channel)
                sum[channel] += context->source[at + channel];
        }
        for (size_t inner = 0; inner < inner_count; ++inner) {
            size_t out = blur_offset(context, outer, inner);
            for (size_t channel = 0; channel < 4; ++channel)
                context->target[out + channel] =
                    (uint8_t)(sum[channel] / count);
            int remove_position = (int)inner - radius;
            int add_position = (int)inner + radius + 1;
            size_t remove = remove_position < 0 ? 0U :
                remove_position >= (int)inner_count ? inner_count - 1U :
                (size_t)remove_position;
            size_t add = add_position < 0 ? 0U :
                add_position >= (int)inner_count ? inner_count - 1U :
                (size_t)add_position;
            size_t remove_at = blur_offset(context, outer, remove);
            size_t add_at = blur_offset(context, outer, add);
            for (size_t channel = 0; channel < 4; ++channel) {
                sum[channel] -= context->source[remove_at + channel];
                sum[channel] += context->source[add_at + channel];
            }
        }
    }
}

static SrStatus blur_image(SrFrame *frame, int radius, unsigned threads,
                           uint8_t **result) {
    size_t bytes = (size_t)frame->width * frame->height * 4;
    uint8_t *first = sr_alloc(bytes);
    uint8_t *second = sr_alloc(bytes);
    if (!first || !second) {
        free(first); free(second);
        return SR_ERR_MEMORY;
    }
    memcpy(first, frame->rgba, bytes);
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
} PixelContext;

static void grade_worker(void *opaque, size_t begin, size_t end) {
    PixelContext *context = opaque;
    SrFrame *frame = context->frame;
    const SrEffect *effect = context->effect;
    for (size_t i = begin; i < end; ++i) {
        double original[3] = {frame->rgba[i*4]/255.0,
                              frame->rgba[i*4+1]/255.0,
                              frame->rgba[i*4+2]/255.0};
        double luminance = .2126*original[0] + .7152*original[1] +
                           .0722*original[2];
        for (size_t channel = 0; channel < 3; ++channel) {
            double value = luminance + (original[channel] - luminance) *
                           effect->saturation;
            value = (value - .5) * effect->contrast + .5 + effect->brightness;
            frame->rgba[i*4+channel] = byte_value(original[channel] +
                (value - original[channel]) * context->intensity);
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
            size_t at = (y * frame->width + x) * 4;
            for (size_t channel = 0; channel < 3; ++channel)
                frame->rgba[at+channel] =
                    (uint8_t)(frame->rgba[at+channel] * factor);
        }
    }
}

typedef struct {
    PixelContext pixels;
    double radius;
} FlareContext;

static void flare_worker(void *opaque, size_t begin, size_t end) {
    FlareContext *context = opaque;
    SrFrame *frame = context->pixels.frame;
    const SrEffect *effect = context->pixels.effect;
    double source_x = frame->width * .72;
    double source_y = frame->height * .28;
    for (size_t y = begin; y < end; ++y) {
        for (uint32_t x = 0; x < frame->width; ++x) {
            size_t at = (y * frame->width + x) * 4;
            for (int ghost = 0; ghost < 4; ++ghost) {
                double t = (ghost + 1) / 5.0;
                double center_x = source_x + (frame->width*.5-source_x)*t*1.7;
                double center_y = source_y + (frame->height*.5-source_y)*t*1.7;
                double radius = fmax(2.0, context->radius*(1+.35*ghost));
                double dx = x + .5 - center_x, dy = y + .5 - center_y;
                double distance = sqrt(dx*dx + dy*dy) / radius;
                if (distance > 1.0) continue;
                double alpha = (1.0-distance) * context->pixels.intensity * .18;
                frame->rgba[at] = byte_value(frame->rgba[at]/255.0 +
                                             effect->color.r*alpha);
                frame->rgba[at+1] = byte_value(frame->rgba[at+1]/255.0 +
                                               effect->color.g*alpha);
                frame->rgba[at+2] = byte_value(frame->rgba[at+2]/255.0 +
                                               effect->color.b*alpha);
            }
        }
    }
}

typedef struct {
    SrFrame *frame;
    const uint8_t *blurred;
    const SrEffect *effect;
    double intensity;
} MergeContext;

static void merge_worker(void *opaque, size_t begin, size_t end) {
    MergeContext *context = opaque;
    for (size_t i = begin; i < end; ++i) {
        if (context->effect->type == SR_EFFECT_BLUR) {
            for (size_t channel = 0; channel < 4; ++channel) {
                size_t at = i * 4 + channel;
                context->frame->rgba[at] = (uint8_t)lrint(
                    context->frame->rgba[at] +
                    (context->blurred[at] - context->frame->rgba[at]) *
                    fmin(1.0, context->intensity));
            }
            continue;
        }
        double luminance = (context->blurred[i*4] +
                            context->blurred[i*4+1] +
                            context->blurred[i*4+2]) / (3.0 * 255.0);
        if (luminance < context->effect->threshold) continue;
        for (size_t channel = 0; channel < 3; ++channel)
            context->frame->rgba[i*4+channel] = byte_value(
                context->frame->rgba[i*4+channel]/255.0 +
                context->blurred[i*4+channel]/255.0 * context->intensity * .5);
    }
}

SrStatus sr_effects_apply(const SrScene *scene, double time, SrFrame *frame,
                          unsigned threads, SrDiagnostics *diag) {
    (void)diag;
    size_t pixels = (size_t)frame->width * frame->height;
    for (size_t index = 0; index < scene->effect_count; ++index) {
        const SrEffect *effect = &scene->effects[index];
        if (!effect->enabled) continue;
        double intensity = fmax(0.0, sr_anim_eval(&effect->intensity, time));
        /* Every effect is an exact byte-for-byte no-op at zero intensity. */
        if (intensity <= 0.0) continue;
        int radius = (int)fmin(64.0, fmax(1.0,
                         sr_anim_eval(&effect->radius, time)));
        PixelContext context = {frame, effect, intensity};
        SrStatus status = SR_OK;
        if (effect->type == SR_EFFECT_COLOR_GRADE) {
            status = sr_parallel_for(pixels, threads, grade_worker, &context);
        } else if (effect->type == SR_EFFECT_VIGNETTE) {
            status = sr_parallel_for(frame->height, threads, vignette_worker,
                                     &context);
        } else if (effect->type == SR_EFFECT_LENS_FLARE) {
            FlareContext flare = {context, radius};
            status = sr_parallel_for(frame->height, threads, flare_worker,
                                     &flare);
        } else {
            uint8_t *blurred = NULL;
            status = blur_image(frame, radius, threads, &blurred);
            if (status == SR_OK) {
                MergeContext merge = {frame, blurred, effect, intensity};
                status = sr_parallel_for(pixels, threads, merge_worker, &merge);
            }
            free(blurred);
        }
        if (status != SR_OK) return status;
    }
    return SR_OK;
}
