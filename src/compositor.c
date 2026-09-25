#include "scene_render/compositor.h"
#include "scene_render/assets.h"
#include "scene_render/color.h"
#include "scene_render/parallel.h"
#include "scene_render/physics.h"

#include <float.h>
#include <math.h>
#include <stdlib.h>
#include <string.h>

/* Draws smaller than this many pixels run on the calling thread: spawning
 * workers costs more than it saves. Pixels are independent, so the split
 * never changes results. */
#define SR_PARALLEL_MIN_PIXELS 16384

typedef struct {
    int x0, y0, x1, y1;
} SrClip;

/* Mask geometry evaluated at the current time. */
typedef struct {
    SrMaskType type;
    bool invert;
    double x, y, width, height, radius;
} SrMaskEval;

/* The masks of one node, evaluated in that node's inverse world transform.
 * Links chain outward so coverage is the product over the whole chain. */
typedef struct SrMaskLink {
    const SrMaskEval *masks;
    size_t count;
    SrMat3 inverse;
    double aa;
    const struct SrMaskLink *parent;
} SrMaskLink;

typedef struct {
    float *px;
    uint32_t width, height;
    SrGroupBuffer *buffer;  /* non-NULL when drawing into an isolated group */
} SrTarget;

typedef enum { SR_OP_IMAGE, SR_OP_SHAPE, SR_OP_BUFFER } SrOpKind;

typedef struct {
    SrOpKind kind;
    const SrTarget *target;
    SrBlendMode blend;
    float opacity;
    SrMat3 inverse;
    double aa;
    const SrMaskLink *masks;
    const SrNode *node;       /* deformation source, may be NULL */
    double time;
    double deform_width, deform_height;
    const SrImage *image;
    SrMaskType shape;
    double width, height;
    float fill[4];
    float stroke[4];
    double half_stroke;
    const SrGroupBuffer *buffer;
    SrClip bounds;
} SrDrawOp;

typedef struct {
    SrCompositor *compositor;
    SrScene *scene;
    SrDiagnostics *diag;
    double time;
    SrStatus status;
} SrDrawContext;

static double sr_clamp(double value, double low, double high) {
    return value < low ? low : value > high ? high : value;
}

/* ---- frames and buffers ------------------------------------------------ */

SrStatus sr_frame_init(SrFrame *frame, uint32_t width, uint32_t height) {
    if (!frame || width == 0 || height == 0) {
        return SR_ERR_ARGUMENT;
    }
    size_t pixels = (size_t)width * height;
    if (pixels / height != width || pixels > SIZE_MAX / (4 * sizeof(float))) {
        return SR_ERR_MEMORY;
    }
    frame->px = sr_alloc(pixels * 4 * sizeof(float));
    if (!frame->px) {
        return SR_ERR_MEMORY;
    }
    frame->width = width;
    frame->height = height;
    return SR_OK;
}

void sr_frame_free(SrFrame *frame) {
    if (frame) {
        free(frame->px);
        *frame = (SrFrame){0};
    }
}

typedef struct {
    SrFrame *frame;
    float color[4];
} SrClearContext;

static void sr_clear_rows(void *opaque, size_t begin, size_t end) {
    const SrClearContext *context = opaque;
    size_t width = context->frame->width;
    for (size_t y = begin; y < end; ++y) {
        float *row = context->frame->px + y * width * 4;
        for (size_t x = 0; x < width; ++x)
            memcpy(row + x * 4, context->color, sizeof(context->color));
    }
}

void sr_frame_clear(SrFrame *frame, const float color[4], unsigned threads) {
    if (!frame || !frame->px) return;
    SrClearContext context = {frame, {color[0], color[1], color[2], color[3]}};
    sr_parallel_for(frame->height, threads, sr_clear_rows, &context);
}

void sr_compositor_init(SrCompositor *compositor, unsigned threads) {
    *compositor = (SrCompositor){.threads = threads};
}

void sr_compositor_free(SrCompositor *compositor) {
    if (!compositor) return;
    for (size_t i = 0; i < compositor->pool_count; ++i) {
        if (compositor->pool[i]) sr_frame_free(&compositor->pool[i]->frame);
        free(compositor->pool[i]);
    }
    free(compositor->pool);
    *compositor = (SrCompositor){0};
}

static void sr_buffer_mark(SrGroupBuffer *buffer, SrClip clip) {
    if (clip.x1 <= clip.x0 || clip.y1 <= clip.y0) return;
    if (buffer->x1 <= buffer->x0 || buffer->y1 <= buffer->y0) {
        buffer->x0 = clip.x0; buffer->y0 = clip.y0;
        buffer->x1 = clip.x1; buffer->y1 = clip.y1;
        return;
    }
    if (clip.x0 < buffer->x0) buffer->x0 = clip.x0;
    if (clip.y0 < buffer->y0) buffer->y0 = clip.y0;
    if (clip.x1 > buffer->x1) buffer->x1 = clip.x1;
    if (clip.y1 > buffer->y1) buffer->y1 = clip.y1;
}

/* Returns the cleared buffer for `depth`, allocating it on first use. Only
 * the dirty rectangle of the previous use needs clearing. */
static SrStatus sr_pool_get(SrCompositor *compositor, size_t depth,
                            uint32_t width, uint32_t height,
                            SrGroupBuffer **out) {
    if (depth >= compositor->pool_count) {
        SrGroupBuffer **pool = sr_realloc(compositor->pool,
                                          (depth + 1) * sizeof(*pool));
        if (!pool) return SR_ERR_MEMORY;
        for (size_t i = compositor->pool_count; i <= depth; ++i) pool[i] = NULL;
        compositor->pool = pool;
        compositor->pool_count = depth + 1;
    }
    SrGroupBuffer *buffer = compositor->pool[depth];
    if (buffer && (buffer->frame.width != width ||
                   buffer->frame.height != height)) {
        sr_frame_free(&buffer->frame);
        free(buffer);
        buffer = compositor->pool[depth] = NULL;
    }
    if (!buffer) {
        buffer = sr_alloc(sizeof(*buffer));
        if (!buffer) return SR_ERR_MEMORY;
        if (sr_frame_init(&buffer->frame, width, height) != SR_OK) {
            free(buffer);
            return SR_ERR_MEMORY;
        }
        compositor->pool[depth] = buffer;
    } else if (buffer->x1 > buffer->x0 && buffer->y1 > buffer->y0) {
        size_t row = (size_t)(buffer->x1 - buffer->x0) * 4 * sizeof(float);
        for (int y = buffer->y0; y < buffer->y1; ++y)
            memset(buffer->frame.px + ((size_t)y * width + (size_t)buffer->x0) * 4,
                   0, row);
    }
    buffer->x0 = buffer->y0 = buffer->x1 = buffer->y1 = 0;
    *out = buffer;
    return SR_OK;
}

/* ---- geometry ----------------------------------------------------------- */

static SrMat3 sr_node_matrix(const SrScene *scene, const SrNode *node, double time) {
    double x = sr_anim_eval(&node->transform.x, time);
    double y = sr_anim_eval(&node->transform.y, time);
    double rotation = sr_anim_eval(&node->transform.rotation, time) * SR_PI / 180.0;
    double physics_rotation;
    if (sr_physics_pose(scene, node, time, &x, &y, &physics_rotation))
        rotation = physics_rotation * SR_PI / 180.0;
    double sx = sr_anim_eval(&node->transform.scale_x, time);
    double sy = sr_anim_eval(&node->transform.scale_y, time);
    double ax = sr_anim_eval(&node->transform.anchor_x, time);
    double ay = sr_anim_eval(&node->transform.anchor_y, time);
    SrMat3 matrix = sr_mat_translate(x, y);
    matrix = sr_mat_multiply(matrix, sr_mat_rotate(rotation));
    matrix = sr_mat_multiply(matrix, sr_mat_scale(sx, sy));
    return sr_mat_multiply(matrix, sr_mat_translate(-ax, -ay));
}

static SrVec2 sr_deform_inverse(const SrNode *node, SrVec2 point,
                                double width, double height, double time) {
    double cx = width * 0.5, cy = height * 0.5;
    for (size_t index = node->modifier_count; index > 0; --index) {
        const SrModifier *modifier = &node->modifiers[index - 1];
        double amount = sr_anim_eval(&modifier->amount, time);
        double frequency = sr_anim_eval(&modifier->frequency, time);
        double phase = sr_anim_eval(&modifier->phase, time);
        if (modifier->type == SR_MOD_WAVE) {
            if (modifier->axis == 'x')
                point.x -= amount * sin(point.y / fmax(height, 1.0) *
                                        2.0 * SR_PI * frequency + phase);
            else
                point.y -= amount * sin(point.x / fmax(width, 1.0) *
                                        2.0 * SR_PI * frequency + phase);
        } else if (modifier->type == SR_MOD_BEND) {
            if (modifier->axis == 'x') {
                double n = (point.y - cy) / fmax(height, 1.0);
                point.x -= amount * n * n;
            } else {
                double n = (point.x - cx) / fmax(width, 1.0);
                point.y -= amount * n * n;
            }
        } else if (modifier->type == SR_MOD_TWIST) {
            double dx = point.x - cx, dy = point.y - cy;
            double radius = hypot(dx / fmax(width, 1.0),
                                  dy / fmax(height, 1.0));
            double angle = -amount * radius * SR_PI / 180.0;
            point.x = cx + dx * cos(angle) - dy * sin(angle);
            point.y = cy + dx * sin(angle) + dy * cos(angle);
        } else if (modifier->type == SR_MOD_SQUASH) {
            double factor = fmax(0.05, 1.0 - amount);
            point.y = cy + (point.y - cy) / factor;
            point.x = cx + (point.x - cx) * factor;
        } else if (modifier->type == SR_MOD_STRETCH) {
            double factor = fmax(0.05, 1.0 + amount);
            point.y = cy + (point.y - cy) / factor;
            point.x = cx + (point.x - cx) * factor;
        }
    }
    if (node->soft_body.enabled) {
        double oscillation = sin(time * sqrt(node->soft_body.stiffness)) *
                             exp(-node->soft_body.damping * time);
        point.x -= (node->soft_body.pressure + oscillation * 3.0) *
                   sin(point.y / fmax(height, 1.0) * SR_PI);
    }
    return point;
}

static bool sr_node_deforms(const SrNode *node) {
    return node && (node->modifier_count > 0 || node->soft_body.enabled);
}

static SrClip sr_clip_intersect(SrClip a, SrClip b) {
    SrClip r = {a.x0 > b.x0 ? a.x0 : b.x0, a.y0 > b.y0 ? a.y0 : b.y0,
                a.x1 < b.x1 ? a.x1 : b.x1, a.y1 < b.y1 ? a.y1 : b.y1};
    if (r.x1 < r.x0) r.x1 = r.x0;
    if (r.y1 < r.y0) r.y1 = r.y0;
    return r;
}

/* Canvas bounds of the local box [x, x+w] x [y, y+h] under `matrix`, padded
 * by one pixel for anti-aliasing and bilinear support, clipped to the
 * target. */
static SrClip sr_bounds(SrMat3 matrix, double x, double y, double width,
                        double height, const SrTarget *target) {
    SrVec2 points[4] = {{x, y}, {x + width, y},
                        {x + width, y + height}, {x, y + height}};
    double min_x = DBL_MAX, min_y = DBL_MAX;
    double max_x = -DBL_MAX, max_y = -DBL_MAX;
    for (size_t i = 0; i < 4; ++i) {
        SrVec2 p = sr_mat_point(matrix, points[i]);
        min_x = fmin(min_x, p.x);
        min_y = fmin(min_y, p.y);
        max_x = fmax(max_x, p.x);
        max_y = fmax(max_y, p.y);
    }
    SrClip clip = {(int)fmax(0.0, floor(min_x - 1.0)),
                   (int)fmax(0.0, floor(min_y - 1.0)),
                   (int)fmin((double)target->width, ceil(max_x + 1.0)),
                   (int)fmin((double)target->height, ceil(max_y + 1.0))};
    if (clip.x1 < clip.x0) clip.x1 = clip.x0;
    if (clip.y1 < clip.y0) clip.y1 = clip.y0;
    return clip;
}

/* Local size of one canvas pixel: sqrt(|det|) of the inverse transform. */
static double sr_pixel_footprint(SrMat3 inverse) {
    return sqrt(fabs(inverse.m00 * inverse.m11 - inverse.m01 * inverse.m10));
}

/* ---- masks -------------------------------------------------------------- */

static void sr_masks_eval(const SrNode *node, double time, SrMaskEval *out) {
    for (size_t i = 0; i < node->mask_count; ++i) {
        const SrMask *mask = &node->masks[i];
        out[i] = (SrMaskEval){
            .type = mask->type,
            .invert = mask->invert,
            .x = sr_anim_eval(&mask->x, time),
            .y = sr_anim_eval(&mask->y, time),
            .width = fmax(0.0, sr_anim_eval(&mask->width, time)),
            .height = fmax(0.0, sr_anim_eval(&mask->height, time)),
            .radius = fmax(0.0, sr_anim_eval(&mask->radius, time))};
    }
}

static float sr_link_coverage(const SrMaskLink *link, double cx, double cy) {
    float coverage = 1.0f;
    for (; link && coverage > 0.0f; link = link->parent) {
        SrVec2 local = sr_mat_point(link->inverse, (SrVec2){cx, cy});
        for (size_t i = 0; i < link->count && coverage > 0.0f; ++i) {
            const SrMaskEval *m = &link->masks[i];
            float c = sr_shape_coverage(m->type, m->x, m->y, m->width,
                                        m->height, m->radius, local.x,
                                        local.y, link->aa);
            coverage *= m->invert ? 1.0f - c : c;
        }
    }
    return coverage;
}

/* Restricts `clip` to the canvas bounds of every non-inverted mask. */
static SrClip sr_mask_clip(SrClip clip, const SrMaskEval *masks, size_t count,
                           SrMat3 world, const SrTarget *target) {
    for (size_t i = 0; i < count; ++i) {
        if (masks[i].invert) continue;
        clip = sr_clip_intersect(clip, sr_bounds(world, masks[i].x, masks[i].y,
                                                 masks[i].width,
                                                 masks[i].height, target));
    }
    return clip;
}

/* ---- draw ops ----------------------------------------------------------- */

static void sr_texel(const SrImage *image, int x, int y, float weight,
                     float acc[4]) {
    if (x < 0 || y < 0 || x >= (int)image->width || y >= (int)image->height ||
        weight == 0.0f)
        return;
    const float *p = image->px + ((size_t)y * image->width + (size_t)x) * 4;
    acc[0] += p[0] * weight;
    acc[1] += p[1] * weight;
    acc[2] += p[2] * weight;
    acc[3] += p[3] * weight;
}

/* Bilinear filter on premultiplied texels with a transparent border;
 * texel (i, j) is centred at (i + 0.5, j + 0.5). */
static void sr_image_sample(const SrImage *image, double lx, double ly,
                            float out[4]) {
    double sx = lx - 0.5, sy = ly - 0.5;
    double fx = floor(sx), fy = floor(sy);
    out[0] = out[1] = out[2] = out[3] = 0.0f;
    if (fx < -2.0 || fy < -2.0 || fx > (double)image->width + 1.0 ||
        fy > (double)image->height + 1.0)
        return;
    int ix = (int)fx, iy = (int)fy;
    float tx = (float)(sx - fx), ty = (float)(sy - fy);
    sr_texel(image, ix, iy, (1.0f - tx) * (1.0f - ty), out);
    sr_texel(image, ix + 1, iy, tx * (1.0f - ty), out);
    sr_texel(image, ix, iy + 1, (1.0f - tx) * ty, out);
    sr_texel(image, ix + 1, iy + 1, tx * ty, out);
}

/* Rows are offsets from bounds.y0 so sr_parallel_for can split [0, rows). */
static void sr_op_rows(void *opaque, size_t begin, size_t end) {
    const SrDrawOp *op = opaque;
    const SrTarget *target = op->target;
    bool deform = sr_node_deforms(op->node);
    for (size_t y = (size_t)op->bounds.y0 + begin;
         y < (size_t)op->bounds.y0 + end; ++y) {
        double cy = (double)y + 0.5;
        float *d = target->px +
                   ((size_t)y * target->width + (size_t)op->bounds.x0) * 4;
        for (int x = op->bounds.x0; x < op->bounds.x1; ++x, d += 4) {
            double cx = (double)x + 0.5;
            float coverage = op->opacity * sr_link_coverage(op->masks, cx, cy);
            if (!(coverage > 0.0f)) continue;
            float s[4];
            if (op->kind == SR_OP_BUFFER) {
                memcpy(s, op->buffer->frame.px + (d - target->px), sizeof(s));
            } else {
                SrVec2 local = sr_mat_point(op->inverse, (SrVec2){cx, cy});
                if (deform)
                    local = sr_deform_inverse(op->node, local, op->deform_width,
                                              op->deform_height, op->time);
                if (op->kind == SR_OP_IMAGE) {
                    sr_image_sample(op->image, local.x, local.y, s);
                } else {
                    double sd = sr_shape_distance(op->shape, 0.0, 0.0,
                                                  op->width, op->height, 0.0,
                                                  local.x, local.y);
                    float cf = sr_distance_coverage(sd, op->aa);
                    float cs = op->half_stroke > 0.0
                        ? sr_distance_coverage(fabs(sd) - op->half_stroke,
                                               op->aa)
                        : 0.0f;
                    float keep = 1.0f - cs * op->stroke[3];
                    for (int c = 0; c < 4; ++c)
                        s[c] = op->stroke[c] * cs + op->fill[c] * cf * keep;
                }
            }
            s[0] *= coverage; s[1] *= coverage;
            s[2] *= coverage; s[3] *= coverage;
            sr_blend_px(op->blend, d, s);
        }
    }
}

static unsigned sr_op_threads(const SrCompositor *compositor, SrClip bounds) {
    size_t area = (size_t)(bounds.x1 - bounds.x0) * (size_t)(bounds.y1 - bounds.y0);
    return area < SR_PARALLEL_MIN_PIXELS ? 1U : compositor->threads;
}

static SrStatus sr_op_execute(const SrCompositor *compositor,
                              const SrDrawOp *op) {
    if (op->bounds.x1 <= op->bounds.x0 || op->bounds.y1 <= op->bounds.y0)
        return SR_OK;
    if (op->target->buffer) sr_buffer_mark(op->target->buffer, op->bounds);
    return sr_parallel_for((size_t)(op->bounds.y1 - op->bounds.y0),
                           sr_op_threads(compositor, op->bounds), sr_op_rows,
                           (void *)op);
}

/* ---- nodes -------------------------------------------------------------- */

static double sr_media_time(const SrNode *node, double time) {
    if (node->source_time.track.count) return sr_anim_eval(&node->source_time, time);
    double local = (time - node->start_time) * node->speed /
                   fmax(node->time_stretch, 1e-12);
    double end = node->clip_out >= 0.0 ? node->clip_out :
                 (node->asset ? node->asset->duration : 0.0);
    double span = fmax(0.0, end - node->clip_in);
    if (span <= 0.0) return node->clip_in;
    int64_t plays = node->loop_count > 0 ? node->loop_count : 1;
    double maximum = span * (double)plays;
    local = sr_clamp(local, 0.0, fmax(0.0, maximum - 1e-12));
    local = fmod(local, span);
    return node->reverse ? end - local - 1e-12 : node->clip_in + local;
}

static uint64_t sr_hash64(uint64_t value) {
    value += UINT64_C(0x9e3779b97f4a7c15);
    value = (value ^ (value >> 30)) * UINT64_C(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)) * UINT64_C(0x94d049bb133111eb);
    return value ^ (value >> 31);
}

static double sr_random_signed(uint64_t seed) {
    return (double)(sr_hash64(seed) >> 11) / 4503599627370495.5 - 1.0;
}

static SrDrawOp sr_op_base(const SrNode *node, const SrTarget *target,
                           SrMat3 inverse, double opacity, double time,
                           const SrMaskLink *masks) {
    return (SrDrawOp){.target = target, .blend = node->blend,
                      .opacity = (float)opacity, .inverse = inverse,
                      .aa = sr_pixel_footprint(inverse), .masks = masks,
                      .node = node, .time = time};
}

static SrStatus sr_draw_image(SrDrawContext *context, const SrNode *node,
                              SrMat3 world, SrMat3 inverse, double opacity,
                              SrClip clip, const SrTarget *target,
                              const SrMaskLink *masks) {
    const SrImage *image = sr_asset_get_frame(
        context->scene, node->asset, sr_media_time(node, context->time),
        context->diag);
    if (!image) return SR_OK;  /* reported through diagnostics */
    SrDrawOp op = sr_op_base(node, target, inverse, opacity, context->time,
                             masks);
    op.kind = SR_OP_IMAGE;
    op.image = image;
    op.deform_width = image->width;
    op.deform_height = image->height;
    op.bounds = sr_clip_intersect(
        sr_bounds(world, 0.0, 0.0, image->width, image->height, target), clip);
    return sr_op_execute(context->compositor, &op);
}

static SrStatus sr_draw_shape(SrDrawContext *context, const SrNode *node,
                              SrMat3 world, SrMat3 inverse, double opacity,
                              SrClip clip, const SrTarget *target,
                              const SrMaskLink *masks) {
    SrDrawOp op = sr_op_base(node, target, inverse, opacity, context->time,
                             masks);
    op.kind = SR_OP_SHAPE;
    op.shape = node->shape == SR_SHAPE_ELLIPSE ? SR_MASK_ELLIPSE : SR_MASK_RECT;
    op.width = node->shape_width;
    op.height = node->shape_height;
    op.deform_width = node->shape_width;
    op.deform_height = node->shape_height;
    op.half_stroke = node->stroke_width * 0.5;
    sr_color_to_blend(&context->scene->project, node->fill, op.fill);
    sr_color_to_blend(&context->scene->project, node->stroke, op.stroke);
    double pad = op.half_stroke;
    op.bounds = sr_clip_intersect(
        sr_bounds(world, -pad, -pad, node->shape_width + 2.0 * pad,
                  node->shape_height + 2.0 * pad, target), clip);
    return sr_op_execute(context->compositor, &op);
}

/* One anti-aliased disc per particle, drawn in order on this thread (discs
 * are small and may overlap, so order matters). */
static void sr_draw_disc(const SrTarget *target, SrBlendMode blend,
                         float opacity, SrVec2 center, double radius,
                         const float color[4], SrClip clip,
                         const SrMaskLink *masks, SrGroupBuffer *buffer) {
    SrClip bounds = {(int)floor(center.x - radius - 1.0),
                     (int)floor(center.y - radius - 1.0),
                     (int)ceil(center.x + radius + 1.0),
                     (int)ceil(center.y + radius + 1.0)};
    bounds = sr_clip_intersect(bounds, clip);
    if (bounds.x1 <= bounds.x0 || bounds.y1 <= bounds.y0) return;
    if (buffer) sr_buffer_mark(buffer, bounds);
    for (int y = bounds.y0; y < bounds.y1; ++y) {
        float *d = target->px + ((size_t)y * target->width + (size_t)bounds.x0) * 4;
        for (int x = bounds.x0; x < bounds.x1; ++x, d += 4) {
            double dx = x + 0.5 - center.x, dy = y + 0.5 - center.y;
            float coverage = sr_distance_coverage(sqrt(dx * dx + dy * dy) - radius,
                                                  1.0);
            if (!(coverage > 0.0f)) continue;
            coverage *= opacity * sr_link_coverage(masks, x + 0.5, y + 0.5);
            if (!(coverage > 0.0f)) continue;
            float s[4] = {color[0] * coverage, color[1] * coverage,
                          color[2] * coverage, color[3] * coverage};
            sr_blend_px(blend, d, s);
        }
    }
}

static void sr_draw_particles(SrDrawContext *context, const SrNode *node,
                              SrMat3 world, double opacity, SrClip clip,
                              const SrTarget *target,
                              const SrMaskLink *masks) {
    const SrScene *scene = context->scene;
    double time = context->time;
    double local_time = time - node->start_time;
    double rate = fmax(0.0, sr_anim_eval(&node->particle_rate, time));
    double lifetime = fmax(1e-9, sr_anim_eval(&node->particle_lifetime, time));
    double speed_value = sr_anim_eval(&node->particle_speed, time);
    double spread = sr_anim_eval(&node->particle_spread, time);
    double size = fmax(0.0, sr_anim_eval(&node->particle_size, time));
    double born_value = local_time > 0.0 ? floor(local_time * rate) : 0.0;
    uint64_t born = !isfinite(born_value) || born_value >= (double)(UINT64_MAX-1)
        ? UINT64_MAX-1 : (uint64_t)born_value;
    double active_value = ceil(lifetime * rate) + 1.0;
    uint64_t active = !isfinite(active_value) || active_value > 1000001.0
        ? 1000001 : (uint64_t)active_value;
    uint64_t first = born > active ? born - active : 0;
    if (born - first > 1000000) first = born - 1000000;
    for (uint64_t i = first; i <= born; ++i) {
        double birth = rate > 0.0 ? i / rate : 0.0;
        double age = local_time - birth;
        if (age < 0.0 || age > lifetime) continue;
        uint64_t base = scene->project.seed ^ ((uint64_t)node->order << 32) ^ i;
        double jitter = sr_random_signed(base);
        double angle = (spread * jitter - 90.0) * SR_PI / 180.0;
        double speed = speed_value * (0.75 +
                       0.25 * sr_random_signed(base + 1));
        double px = cos(angle) * speed * age;
        double py = sin(angle) * speed * age;
        if (strcmp(node->particle_preset, "rain") == 0) {
            px = jitter * 40.0; py = speed * age;
        } else if (strcmp(node->particle_preset, "smoke") == 0) {
            px += sin(age * 3.0 + jitter) * 15.0; py = -fabs(speed) * age;
        } else if (strcmp(node->particle_preset, "dust") == 0) {
            px *= 0.25; py *= 0.25;
        } else {
            py += 200.0 * age * age;
        }
        SrVec2 center = sr_mat_point(world, (SrVec2){px, py});
        double radius = size *
                        (strcmp(node->particle_preset, "smoke") == 0
                             ? 1.0 + age : 1.0);
        double fade = 1.0 - age / lifetime;
        SrColor color = node->particle_color;
        color.a *= fade;
        float premultiplied[4];
        sr_color_to_blend(&scene->project, color, premultiplied);
        sr_draw_disc(target, node->blend, (float)opacity, center, radius,
                     premultiplied, clip, masks, target->buffer);
    }
}

static SrStatus sr_draw_node(SrDrawContext *context, const SrNode *node,
                             SrMat3 parent, SrClip clip,
                             const SrTarget *target, size_t depth);

static SrStatus sr_draw_children(SrDrawContext *context, const SrNode *node,
                                 SrMat3 world, SrClip clip,
                                 const SrTarget *target, size_t depth) {
    for (size_t i = 0; i < node->child_count; ++i) {
        SrStatus status = sr_draw_node(context, node->children[i], world, clip,
                                       target, depth);
        if (status != SR_OK) return status;
    }
    return SR_OK;
}

/* A group renders into an isolated buffer iff it has a non-normal blend,
 * opacity below one, or any mask; otherwise its children draw straight into
 * the parent target. The isolated buffer is composited once with the
 * group's blend, opacity and masks. */
static SrStatus sr_draw_group(SrDrawContext *context, const SrNode *node,
                              SrMat3 world, double opacity, SrClip clip,
                              const SrTarget *target, size_t depth) {
    bool isolated = node->blend != SR_BLEND_NORMAL || opacity < 1.0 ||
                    node->mask_count > 0;
    if (!isolated)
        return sr_draw_children(context, node, world, clip, target, depth);
    SrMat3 inverse;
    if (!sr_mat_inverse(world, &inverse)) return SR_OK;
    SrMaskEval local_masks[8];
    SrMaskEval *masks = node->mask_count <= 8 ? local_masks
        : sr_alloc(node->mask_count * sizeof(*masks));
    if (!masks) return SR_ERR_MEMORY;
    sr_masks_eval(node, context->time, masks);
    SrMaskLink link = {masks, node->mask_count, inverse,
                       sr_pixel_footprint(inverse), NULL};
    SrClip inner = sr_mask_clip(clip, masks, node->mask_count, world, target);
    SrGroupBuffer *buffer = NULL;
    SrStatus status = sr_pool_get(context->compositor, depth, target->width,
                                  target->height, &buffer);
    if (status == SR_OK) {
        SrTarget group = {buffer->frame.px, target->width, target->height,
                          buffer};
        status = sr_draw_children(context, node, world, inner, &group,
                                  depth + 1);
    }
    if (status == SR_OK) {
        SrDrawOp op = {.kind = SR_OP_BUFFER, .target = target,
                       .blend = node->blend, .opacity = (float)opacity,
                       .inverse = inverse, .aa = link.aa,
                       .masks = node->mask_count ? &link : NULL,
                       .buffer = buffer};
        op.bounds = sr_clip_intersect(inner, (SrClip){buffer->x0, buffer->y0,
                                                      buffer->x1, buffer->y1});
        status = sr_op_execute(context->compositor, &op);
    }
    if (masks != local_masks) free(masks);
    return status;
}

static SrStatus sr_draw_node(SrDrawContext *context, const SrNode *node,
                             SrMat3 parent, SrClip clip,
                             const SrTarget *target, size_t depth) {
    double time = context->time;
    if (!node->visible || time < node->start_time || time >= node->end_time)
        return SR_OK;
    SrMat3 world = sr_mat_multiply(parent, sr_node_matrix(context->scene, node,
                                                          time));
    double opacity = sr_clamp(sr_anim_eval(&node->opacity, time), 0.0, 1.0);
    if (opacity <= 0.0) return SR_OK;
    if (node->type == SR_NODE_GROUP)
        return sr_draw_group(context, node, world, opacity, clip, target, depth);
    SrMat3 inverse;
    if (!sr_mat_inverse(world, &inverse)) return SR_OK;
    SrMaskEval local_masks[8];
    SrMaskEval *masks = node->mask_count <= 8 ? local_masks
        : sr_alloc(node->mask_count * sizeof(*masks));
    if (!masks) return SR_ERR_MEMORY;
    sr_masks_eval(node, time, masks);
    SrMaskLink link = {masks, node->mask_count, inverse,
                       sr_pixel_footprint(inverse), NULL};
    const SrMaskLink *chain = node->mask_count ? &link : NULL;
    clip = sr_mask_clip(clip, masks, node->mask_count, world, target);
    SrStatus status = SR_OK;
    if (node->type == SR_NODE_MEDIA)
        status = sr_draw_image(context, node, world, inverse, opacity, clip,
                               target, chain);
    else if (node->type == SR_NODE_SHAPE)
        status = sr_draw_shape(context, node, world, inverse, opacity, clip,
                               target, chain);
    else if (node->type == SR_NODE_PARTICLES)
        sr_draw_particles(context, node, world, opacity, clip, target, chain);
    if (masks != local_masks) free(masks);
    return status;
}

SrStatus sr_compositor_render(SrCompositor *compositor, SrScene *scene,
                              double time, SrFrame *frame,
                              SrDiagnostics *diag) {
    if (!compositor || !scene || !scene->root || !frame || !frame->px) {
        return SR_ERR_ARGUMENT;
    }
    SrTarget target = {frame->px, frame->width, frame->height, NULL};
    SrClip clip = {0, 0, (int)frame->width, (int)frame->height};
    size_t errors = diag ? diag->errors : 0;
    SrDrawContext context = {compositor, scene, diag, time, SR_OK};
    SrStatus status = sr_draw_node(&context, scene->root, sr_mat_identity(),
                                   clip, &target, 0);
    if (status != SR_OK) return status;
    return diag && diag->errors > errors ? SR_ERR_ASSET : SR_OK;
}

SrStatus sr_composite_scene(SrScene *scene, double time, SrFrame *frame,
                            SrDiagnostics *diag) {
    SrCompositor compositor;
    sr_compositor_init(&compositor, 1);
    SrStatus status = sr_compositor_render(&compositor, scene, time, frame, diag);
    sr_compositor_free(&compositor);
    return status;
}
