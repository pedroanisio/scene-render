#include "scene_render/effects.h"
#include "scene_render/color.h"
#include "scene_render/parallel.h"

#include <float.h>
#include <math.h>
#include <stdlib.h>
#include <string.h>

/* Frame and group effects on float premultiplied blend-space pixels. Every
 * worker owns whole rows or columns of the processed rectangle and runs a
 * fixed arithmetic order, so the result is identical for any thread count
 * and for any rectangle that contains everything the effect can touch. */

#define TRANSFER_STEPS 4096
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

/* ---- blur ---------------------------------------------------------------- */

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

/* Separable box blur of a compact width x height float4 buffer, clamped to
 * its edges. `buffer` is consumed: the result replaces it (*result). */
static SrStatus blur_compact(float *buffer, uint32_t width, uint32_t height,
                             int radius, unsigned threads, float **result) {
    size_t bytes = (size_t)width * height * 4 * sizeof(float);
    float *second = malloc(bytes);
    if (!second) {
        free(buffer);
        return SR_ERR_MEMORY;
    }
    BlurContext context = {buffer, second, width, height, radius, true};
    SrStatus status = sr_parallel_for(height, threads, blur_worker, &context);
    if (status == SR_OK) {
        context = (BlurContext){second, buffer, width, height, radius, false};
        status = sr_parallel_for(width, threads, blur_worker, &context);
    }
    free(second);
    if (status != SR_OK) {
        free(buffer);
        return status;
    }
    *result = buffer;
    return SR_OK;
}

/* Box blur of the region's pixels into a compact buffer. */
static SrStatus blur_region(const Region *region, int radius, unsigned threads,
                            float **result) {
    size_t width = region_width(region), height = region_height(region);
    float *first = malloc(width * height * 4 * sizeof(float));
    if (!first) return SR_ERR_MEMORY;
    for (size_t row = 0; row < height; ++row)
        memcpy(first + row * width * 4, region_px(region, row, 0),
               width * 4 * sizeof(float));
    return blur_compact(first, (uint32_t)width, (uint32_t)height, radius,
                        threads, result);
}

/* ---- per-pixel effects --------------------------------------------------- */

typedef struct {
    const Region *region;
    const Params *params;
    const Transfer *transfer;
} PixelContext;

static void grade_worker(void *opaque, size_t begin, size_t end) {
    PixelContext *context = opaque;
    const Params *params = context->params;
    size_t width = region_width(context->region);
    for (size_t row = begin; row < end; ++row) {
        float *p = region_px(context->region, row, 0);
        for (size_t x = 0; x < width; ++x, p += 4) {
            float alpha = p[3];
            if (!(alpha > 0.0f)) continue;
            double original[3];
            for (size_t c = 0; c < 3; ++c)
                original[c] = to_display(context->transfer, p[c] / alpha);
            double luminance = .2126*original[0] + .7152*original[1] +
                               .0722*original[2];
            for (size_t channel = 0; channel < 3; ++channel) {
                double value = luminance + (original[channel] - luminance) *
                               params->saturation;
                value = (value - .5) * params->contrast + .5 + params->brightness;
                value = original[channel] +
                        (value - original[channel]) * params->intensity;
                p[channel] = from_display(context->transfer, (float)value) * alpha;
            }
        }
    }
}

static void vignette_worker(void *opaque, size_t begin, size_t end) {
    PixelContext *context = opaque;
    const Region *region = context->region;
    SrFrame *frame = region->frame;
    for (size_t row = begin; row < end; ++row) {
        size_t y = (size_t)region->rect.y0 + row;
        for (int x = region->rect.x0; x < region->rect.x1; ++x) {
            double nx = 2.0 * (x + .5) / frame->width - 1.0;
            double ny = 2.0 * (y + .5) / frame->height - 1.0;
            double factor = fmax(0.0, 1.0 - context->params->intensity * .55 *
                                 (nx * nx + ny * ny));
            /* The factor darkens perceptually; in linear light apply its
             * decoded equivalent. */
            float scale = context->transfer->linear
                ? lookup(context->transfer->decode, (float)factor)
                : (float)factor;
            float *p = &frame->px[(y * frame->width + (size_t)x) * 4];
            for (size_t channel = 0; channel < 3; ++channel) p[channel] *= scale;
        }
    }
}

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
    PixelContext *context = opaque;
    const Region *region = context->region;
    SrFrame *frame = region->frame;
    const Params *params = context->params;
    double flare_radius = blur_radius(params->radius);
    double source_x = frame->width * .72;
    double source_y = frame->height * .28;
    for (size_t row = begin; row < end; ++row) {
        size_t y = (size_t)region->rect.y0 + row;
        for (int x = region->rect.x0; x < region->rect.x1; ++x) {
            float add[3] = {0.0f, 0.0f, 0.0f};
            bool touched = false;
            for (int ghost = 0; ghost < 4; ++ghost) {
                double t = (ghost + 1) / 5.0;
                double center_x = source_x + (frame->width*.5-source_x)*t*1.7;
                double center_y = source_y + (frame->height*.5-source_y)*t*1.7;
                double radius = fmax(2.0, flare_radius*(1+.35*ghost));
                double dx = x + .5 - center_x, dy = y + .5 - center_y;
                double distance = sqrt(dx*dx + dy*dy) / radius;
                if (distance > 1.0) continue;
                double alpha = (1.0-distance) * params->intensity * .18;
                add[0] += (float)(params->color.r * alpha);
                add[1] += (float)(params->color.g * alpha);
                add[2] += (float)(params->color.b * alpha);
                touched = true;
            }
            if (touched)
                add_display(context->transfer,
                            &frame->px[(y * frame->width + (size_t)x) * 4], add);
        }
    }
}

typedef struct {
    const Region *region;
    const float *blurred;       /* compact region buffer */
    const SrEffect *effect;
    const Params *params;
    const Transfer *transfer;
} MergeContext;

static void merge_worker(void *opaque, size_t begin, size_t end) {
    MergeContext *context = opaque;
    size_t width = region_width(context->region);
    const Params *params = context->params;
    for (size_t row = begin; row < end; ++row) {
        float *px = region_px(context->region, row, 0);
        const float *blurred = context->blurred + row * width * 4;
        if (context->effect->type == SR_EFFECT_BLUR) {
            float mix = (float)fmin(1.0, params->intensity);
            for (size_t i = 0; i < width * 4; ++i)
                px[i] = px[i] + (blurred[i] - px[i]) * mix;
            continue;
        }
        /* Glow and bloom threshold and add in the transfer-encoded domain. */
        float gain = (float)(params->intensity * .5);
        for (size_t i = 0; i < width; ++i) {
            const float *b = &blurred[i * 4];
            if (!(b[3] > 0.0f)) continue;
            float add[3];
            for (size_t c = 0; c < 3; ++c)
                add[c] = to_display(context->transfer, b[c] / b[3]) * b[3];
            float luminance = (add[0] + add[1] + add[2]) / 3.0f;
            if (luminance < params->threshold) continue;
            for (size_t c = 0; c < 3; ++c) add[c] *= gain;
            add_display(context->transfer, &px[i * 4], add);
        }
    }
}

/* ---- drop shadow --------------------------------------------------------- */

typedef struct {
    const Region *region;
    float *shadow;              /* compact: alpha in channel 3 */
    double dx, dy;
    float color[4];             /* premultiplied shadow color */
    float opacity;
} ShadowContext;

static float frame_alpha(const SrFrame *frame, int x, int y) {
    if (x < 0 || y < 0 || x >= (int)frame->width || y >= (int)frame->height)
        return 0.0f;
    return frame->px[((size_t)y * frame->width + (size_t)x) * 4 + 3];
}

/* The content's alpha displaced by (dx, dy), bilinear at subpixel offsets,
 * transparent beyond the frame. */
static void shift_worker(void *opaque, size_t begin, size_t end) {
    ShadowContext *context = opaque;
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
            float *d = context->shadow + (row * width + column) * 4;
            d[0] = d[1] = d[2] = 0.0f;
            d[3] = a;
        }
    }
}

/* Content over its shadow: out = content + shadow * (1 - content alpha). */
static void under_worker(void *opaque, size_t begin, size_t end) {
    ShadowContext *context = opaque;
    size_t width = region_width(context->region);
    for (size_t row = begin; row < end; ++row) {
        float *p = region_px(context->region, row, 0);
        const float *s = context->shadow + row * width * 4;
        for (size_t column = 0; column < width; ++column, p += 4, s += 4) {
            float sa = s[3] * context->opacity;
            if (!(sa > 0.0f)) continue;
            float k = 1.0f - p[3];
            if (!(k > 0.0f)) continue;
            for (int c = 0; c < 4; ++c) p[c] += context->color[c] * sa * k;
        }
    }
}

static SrStatus drop_shadow(const SrScene *scene, const Region *region,
                            const Params *params, unsigned threads) {
    size_t width = region_width(region), height = region_height(region);
    ShadowContext context = {region, malloc(width * height * 4 * sizeof(float)),
                             params->offset_x, params->offset_y, {0},
                             (float)fmin(1.0, params->intensity)};
    if (!context.shadow) return SR_ERR_MEMORY;
    sr_color_to_blend(&scene->project, params->color, context.color);
    SrStatus status = sr_parallel_for(height, threads, shift_worker, &context);
    int radius = shadow_radius(params->radius);
    if (status == SR_OK && radius > 0) {
        float *blurred = NULL;
        status = blur_compact(context.shadow, (uint32_t)width, (uint32_t)height,
                              radius, threads, &blurred);
        context.shadow = status == SR_OK ? blurred : NULL;
    }
    if (status == SR_OK)
        status = sr_parallel_for(height, threads, under_worker, &context);
    free(context.shadow);
    return status;
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

/* Effects never leave non-finite color in a frame (a huge light over black
 * computes 0 x inf): such channels become 0; alpha is left as written. */
static void sanitize_worker(void *opaque, size_t begin, size_t end) {
    const Region *region = opaque;
    size_t width = region_width(region);
    for (size_t row = begin; row < end; ++row) {
        float *p = region_px(region, row, 0);
        for (size_t x = 0; x < width; ++x, p += 4)
            for (int c = 0; c < 3; ++c)
                if (!isfinite(p[c])) p[c] = 0.0f;
    }
}

static SrStatus run_one(const SrScene *scene, const SrEffect *effect,
                        double time, const Region *region, SrMat3 to_canvas,
                        const Params *params, Transfer *transfer,
                        unsigned threads);

static SrStatus apply_one(const SrScene *scene, const SrEffect *effect,
                          double time, const Region *region, SrMat3 to_canvas,
                          Transfer **transfer, unsigned threads) {
    if (!effect->enabled) return SR_OK;
    Params params = params_eval(effect, time);
    /* Every effect is an exact no-op at zero intensity. */
    if (params.intensity <= 0.0) return SR_OK;
    if (region->rect.x1 <= region->rect.x0 || region->rect.y1 <= region->rect.y0)
        return SR_OK;
    if (!*transfer) {
        *transfer = malloc(sizeof(**transfer));
        if (!*transfer) return SR_ERR_MEMORY;
        transfer_init(*transfer, &scene->project);
    }
    SrStatus status = run_one(scene, effect, time, region, to_canvas, &params,
                              *transfer, threads);
    if (status != SR_OK) return status;
    return sr_parallel_for(region_height(region), threads, sanitize_worker,
                           (void *)region);
}

static SrStatus run_one(const SrScene *scene, const SrEffect *effect,
                        double time, const Region *region, SrMat3 to_canvas,
                        const Params *params_in, Transfer *transfer,
                        unsigned threads) {
    Params params = *params_in;
    PixelContext context = {region, &params, transfer};
    size_t rows = region_height(region);
    switch (effect->type) {
    case SR_EFFECT_COLOR_GRADE:
        return sr_parallel_for(rows, threads, grade_worker, &context);
    case SR_EFFECT_VIGNETTE:
        return sr_parallel_for(rows, threads, vignette_worker, &context);
    case SR_EFFECT_LENS_FLARE:
        return sr_parallel_for(rows, threads, flare_worker, &context);
    case SR_EFFECT_DROP_SHADOW:
        return drop_shadow(scene, region, &params, threads);
    case SR_EFFECT_LIGHTING:
        return lighting(scene, effect, region, &params, to_canvas, time, threads);
    default: {
        float *blurred = NULL;
        SrStatus status = blur_region(region, blur_radius(params.radius),
                                      threads, &blurred);
        if (status == SR_OK) {
            MergeContext merge = {region, blurred, effect, &params, transfer};
            status = sr_parallel_for(rows, threads, merge_worker, &merge);
        }
        free(blurred);
        return status;
    }
    }
}

SrStatus sr_effects_apply(const SrScene *scene, double time, SrFrame *frame,
                          unsigned threads, SrDiagnostics *diag) {
    (void)diag;
    Transfer *transfer = NULL;
    Region region = {frame, {0, 0, (int)frame->width, (int)frame->height}};
    for (size_t index = 0; index < scene->effect_count; ++index) {
        const SrEffect *effect = &scene->effects[index];
        if (effect->referenced) continue;
        SrStatus status = apply_one(scene, effect, time, &region,
                                    sr_mat_identity(), &transfer, threads);
        if (status != SR_OK) {
            free(transfer);
            return status;
        }
    }
    free(transfer);
    return SR_OK;
}

SrStatus sr_effects_apply_group(const SrScene *scene, SrEffect *const *effects,
                                size_t count, double time, SrFrame *frame,
                                SrMat3 to_canvas, SrEffectRect *rect,
                                unsigned threads) {
    Transfer *transfer = NULL;
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
        SrStatus status = apply_one(scene, effects[index], time, &region,
                                    to_canvas, &transfer, threads);
        if (status != SR_OK) {
            free(transfer);
            return status;
        }
        *rect = grown;
    }
    free(transfer);
    return SR_OK;
}
