#include "scene_render/compositor.h"
#include "scene_render/assets.h"
#include "scene_render/card.h"
#include "scene_render/color.h"
#include "scene_render/deform.h"
#include "scene_render/effects.h"
#include "scene_render/lighting.h"
#include "scene_render/parallel.h"
#include "particles_internal.h"
#include "scene_render/physics.h"
#include "length_frame.h"
#include "compositing_internal.h"
#include "compositor_internal.h"
#include "compositor_resources_internal.h"
#include "compositor_geometry_internal.h"
#include "compositor_evaluation_internal.h"

#include <float.h>
#include <limits.h>
#include <math.h>
#include <stdlib.h>
#include <string.h>

/* Draws smaller than this many pixels run on the calling thread: spawning
 * workers costs more than it saves. Pixels are independent, so the split
 * never changes results. */
#define SR_PARALLEL_MIN_PIXELS 16384

/* SR_OP_CLEAR zeroes its bounds (a pooled buffer's previous dirty rect). */
typedef enum {
    SR_OP_IMAGE, SR_OP_SHAPE, SR_OP_BUFFER, SR_OP_DISC, SR_OP_CLEAR
} SrOpKind;

/* Grid deformations evaluated once per draw: the offsets of every
 * mesh-warp modifier (NULL entries for the other modifier types) and of
 * the simulated soft body, plus every modifier's amount, frequency and
 * phase (the values the per-pixel evaluation gave). */
typedef struct {
    double **mesh;
    double *storage;            /* owns every mesh[i] grid */
    double *soft;
    double *params;             /* 3 per modifier */
    SrCompositeResources *resources; /* borrowed through immediate draw */
    uint64_t pixel_work;        /* worst-case inverse, independent of samples */
} SrDeformState;

typedef struct {
    SrOpKind kind;
    SrTarget target;          /* by value: queued ops outlive the caller */
    SrBlendMode blend;
    float opacity;
    SrMat3 inverse;
    double aa;
    const SrMaskLink *masks;
    const SrNode *node;       /* deformation source, may be NULL */
    double time;
    double deform_width, deform_height;
    const SrDeformState *deform;
    const SrImage *image;
    SrMaskType shape;
    double width, height;
    float fill[4];
    float stroke[4];
    double half_stroke;
    SrVec2 center;            /* SR_OP_DISC: one particle, color in fill */
    double radius;
    bool square;
    const SrGroupBuffer *buffer;
    bool has_card;            /* SR_OP_BUFFER: depth test with `card` */
    SrCardTest card;
    SrClip bounds;
    SrCompositeOwner owner;   /* source of queued resource reservations */
    void *owned;              /* queued copy of the mask chain, or NULL */
} SrDrawOp;

/* Recorded draw ops awaiting execution (see sr_queue_flush). */
struct SrOpQueue {
    SrDrawOp *ops;
    size_t count, capacity;
    size_t pixels;            /* total op area, for the serial threshold */
    uint32_t *order;          /* band dealing order, see sr_queue_flush */
    size_t order_capacity;
};

/* A card's own blend applies when its buffer is composited, not inside. */
static SrBlendMode sr_node_blend(const SrDrawContext *context,
                                 const SrNode *node) {
    return node == context->card_node ? SR_BLEND_NORMAL : node->blend;
}

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
    if (compositor->plane) {
        sr_compositor_free(compositor->plane);
        sr_composite_free(compositor->resources, compositor->plane);
    }
    sr_composite_free(compositor->resources, compositor->depth_store.z);
    if (compositor->lengths) {
        sr_length_frame_free(compositor->lengths);
        sr_composite_free(compositor->resources, compositor->lengths);
    }
    for (size_t i = 0; i < compositor->pool_count; ++i) {
        if (compositor->pool[i])
            sr_composite_free(compositor->resources, compositor->pool[i]->frame.px);
        sr_composite_free(compositor->resources, compositor->pool[i]);
    }
    sr_composite_free(compositor->resources, compositor->pool);
    if (compositor->queue) {
        for (size_t i = 0; i < compositor->queue->count; ++i)
            sr_composite_free(compositor->resources, compositor->queue->ops[i].owned);
        sr_composite_free(compositor->resources, compositor->queue->ops);
        sr_composite_free(compositor->resources, compositor->queue->order);
        sr_composite_free(compositor->resources, compositor->queue);
    }
    *compositor = (SrCompositor){0};
}

static SrStatus sr_queue_flush(SrCompositor *compositor);
static SrStatus sr_op_submit(SrCompositor *compositor, const SrDrawOp *op,
                             bool immediate);

/* Drops every queued op unrun (after a failure), here and in the
 * plane-buffer pool. */
static void sr_queue_discard(SrCompositor *compositor) {
    for (; compositor; compositor = compositor->plane) {
        struct SrOpQueue *queue = compositor->queue;
        if (!queue) continue;
        for (size_t i = 0; i < queue->count; ++i)
            sr_composite_free(compositor->resources, queue->ops[i].owned);
        queue->count = 0;
        queue->pixels = 0;
    }
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
 * the dirty rectangle of the previous use needs clearing. Queued ops may
 * still write or read the previous use, so the clear is queued behind them
 * as an SR_OP_CLEAR (each pixel is still zeroed after its last use and
 * before its next), and a buffer is only freed after a flush. */
static SrStatus sr_pool_get(SrCompositor *compositor, size_t depth,
                            uint32_t width, uint32_t height,
                            SrGroupBuffer **out) {
    SrCompositeResources *resources = compositor->resources;
    if (resources && (depth >= SR_MAX_COMPOSITE_DEPTH || !width || !height ||
        width > SR_MAX_COVERAGE_DIMENSION || height > SR_MAX_COVERAGE_DIMENSION)) {
        sr_composite_resource_fail(resources, SR_ERR_RENDER,
                                    "compositing buffer dimensions/depth exceeded");
        return SR_ERR_RENDER;
    }
    if (depth >= compositor->pool_count) {
        SrGroupBuffer **pool = sr_composite_realloc(resources,
            compositor->pool, depth + 1, sizeof(*pool), 0);
        if (!pool) return sr_composite_resource_status(resources);
        for (size_t i = compositor->pool_count; i <= depth; ++i) pool[i] = NULL;
        compositor->pool = pool;
        compositor->pool_count = depth + 1;
    }
    SrGroupBuffer *buffer = compositor->pool[depth];
    if (buffer && (buffer->frame.width != width ||
                   buffer->frame.height != height)) {
        SrStatus flushed = sr_queue_flush(compositor);
        if (flushed != SR_OK) return flushed;
        sr_composite_free(resources, buffer->frame.px);
        sr_composite_free(resources, buffer);
        buffer = compositor->pool[depth] = NULL;
    }
    if (!buffer) {
        buffer = sr_composite_alloc(resources, 1, sizeof(*buffer), 0);
        if (!buffer) return sr_composite_resource_status(resources);
        uint64_t pixels = (uint64_t)width * height * 4;
        buffer->frame = (SrFrame){width, height,
            sr_composite_alloc(resources, (size_t)width * height,
                                 4 * sizeof(float), pixels)};
        if (!buffer->frame.px) {
            sr_composite_free(resources, buffer);
            return sr_composite_resource_status(resources);
        }
        compositor->pool[depth] = buffer;
    } else if (buffer->x1 > buffer->x0 && buffer->y1 > buffer->y0) {
        SrDrawOp clear = {.kind = SR_OP_CLEAR,
                          .target = {buffer->frame.px, width, height, NULL},
                          .bounds = {buffer->x0, buffer->y0, buffer->x1,
                                     buffer->y1}};
        SrStatus status = sr_op_submit(compositor, &clear, false);
        if (status != SR_OK) return status;
    }
    buffer->x0 = buffer->y0 = buffer->x1 = buffer->y1 = 0;
    *out = buffer;
    return SR_OK;
}

/* ---- geometry ----------------------------------------------------------- */

static SrStatus sr_skew_error(const SrDrawContext *context, const SrNode *node,
                              const char *attribute, const char *message) {
    const char *element = node->type == SR_NODE_GROUP ? "group"
        : node->type == SR_NODE_MEDIA ? "layer"
        : node->type == SR_NODE_PARTICLES ? "particleEmitter" : "shape";
    if (context->diag)
        sr_diag_error(context->diag, node->source_line, element, attribute,
                      "%s for node '%s'", message, node->id ? node->id : "");
    return SR_ERR_RENDER;
}

static SrStatus sr_skew_matrix_valid(const SrDrawContext *context,
                                      const SrNode *node, SrMat3 matrix) {
    if (!context->skewed) return SR_OK;
    SrMat3 inverse;
    return sr_mat_checked_inverse(matrix, &inverse) ? SR_OK
        : sr_skew_error(context, node, "skewX/skewY",
                        "skewed transform must be finite and invertible");
}

/* The context is a stack copy for this node. Its skew flag follows the
 * transform chain, including descendants inside a projected card. */
static SrStatus sr_node_world(SrDrawContext *context, const SrNode *node,
                              SrMat3 parent, SrMat3 *out) {
    const SrScene *scene = context->scene;
    double time = context->time;
    const SrNodeGeometry *geometry = sr_length_node(context->lengths, node);
    SrCompositeResources *resources = context->compositor->resources;
    if (resources && !sr_composite_world_work(resources, scene, node, geometry != NULL))
        return SR_ERR_RENDER;
    double x = geometry ? geometry->x : sr_anim_eval(&node->transform.x, time);
    double y = geometry ? geometry->y : sr_anim_eval(&node->transform.y, time);
    double rotation = sr_anim_eval(&node->transform.rotation, time) * SR_PI / 180.0;
    double physics_rotation;
    if (!sr_composite_physics_ready(context->compositor->resources, scene, node, time))
        return SR_ERR_RENDER;
    if (sr_physics_pose(scene, node, time, &x, &y, &physics_rotation))
        rotation = physics_rotation * SR_PI / 180.0;
    double sx = sr_anim_eval(&node->transform.scale_x, time);
    double sy = sr_anim_eval(&node->transform.scale_y, time);
    double ax = geometry ? geometry->anchor_x : sr_anim_eval(&node->transform.anchor_x, time);
    double ay = geometry ? geometry->anchor_y : sr_anim_eval(&node->transform.anchor_y, time);
    SrMat3 matrix = sr_mat_translate(x, y);
    matrix = sr_mat_multiply(matrix, sr_mat_rotate(rotation));
    const SrAnimValue *kx = &node->transform.skew_x;
    const SrAnimValue *ky = &node->transform.skew_y;
    if (kx->base != 0.0 || ky->base != 0.0 || kx->track.count || ky->track.count) {
        double skew_x = sr_anim_eval(kx, time), skew_y = sr_anim_eval(ky, time);
        if (!isfinite(skew_x) || fabs(skew_x) > SR_MAX_SKEW_DEGREES)
            return sr_skew_error(context, node, "skewX",
                                 "evaluated skew must be within [-89,89] degrees");
        if (!isfinite(skew_y) || fabs(skew_y) > SR_MAX_SKEW_DEGREES)
            return sr_skew_error(context, node, "skewY",
                                 "evaluated skew must be within [-89,89] degrees");
        matrix = sr_mat_apply_skew(matrix, skew_x, skew_y);
        context->skewed |= skew_x != 0.0 || skew_y != 0.0;
    }
    matrix = sr_mat_multiply(matrix, sr_mat_scale(sx, sy));
    matrix = sr_mat_multiply(matrix, sr_mat_translate(-ax, -ay));
    *out = sr_mat_multiply(parent, matrix);
    return sr_skew_matrix_valid(context, node, *out);
}

/* Source point of `point` under the node's deformations; false when a
 * grid has no source there (the pixel stays empty). The modifier
 * parameters come from state->params (see sr_deform_prepare). */
static bool sr_deform_inverse(const SrNode *node, SrVec2 *io,
                              double width, double height,
                              const SrDeformState *state) {
    SrVec2 point = *io;
    double cx = width * 0.5, cy = height * 0.5;
    for (size_t index = node->modifier_count; index > 0; --index) {
        const SrModifier *modifier = &node->modifiers[index - 1];
        if (modifier->type == SR_MOD_MESH_WARP) {
            if (state->mesh && state->mesh[index - 1] &&
                !sr_grid_warp_inverse(state->mesh[index - 1], modifier->rows,
                                      modifier->cols, width, height, point,
                                      &point))
                return false;
            continue;
        }
        const double *param = state->params + (index - 1) * 3;
        double amount = param[0];
        double frequency = param[1];
        double phase = param[2];
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
            double ca = cos(angle), sa = sin(angle);
            point.x = cx + dx * ca - dy * sa;
            point.y = cy + dx * sa + dy * ca;
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
    if (state->soft &&
        !sr_grid_warp_inverse(state->soft, node->soft_body.rows,
                              node->soft_body.cols, width, height, point, &point))
        return false;
    *io = point;
    return true;
}

static bool sr_node_deforms(const SrNode *node) {
    return node && (node->modifier_count > 0 ||
                    (node->soft_body.enabled && node->soft_body.sample_count));
}

static void sr_deform_free(SrDeformState *state) {
    sr_composite_free(state->resources, state->mesh);
    sr_composite_free(state->resources, state->storage);
    sr_composite_free(state->resources, state->soft);
    sr_composite_free(state->resources, state->params);
    *state = (SrDeformState){0};
}

/* Evaluates the grid deformations of `node` at `time`. */
static SrStatus sr_deform_prepare(const SrScene *scene, const SrNode *node,
                                  double time, SrCompositeResources *resources,
                                  SrDeformState *state) {
    *state = (SrDeformState){.resources = resources};
    if (!sr_composite_deform_admit(resources, scene, node, time, &state->pixel_work))
        return sr_composite_resource_status(resources);
    if (node->modifier_count) {
        state->params = sr_composite_alloc(resources, node->modifier_count,
                                            3 * sizeof(*state->params), 0);
        if (!state->params) return sr_composite_resource_status(resources);
        for (size_t i = 0; i < node->modifier_count; ++i) {
            const SrModifier *modifier = &node->modifiers[i];
            double *param = state->params + i * 3;
            param[0] = param[1] = param[2] = 0.0;
            if (modifier->type == SR_MOD_MESH_WARP) continue;
            param[0] = sr_anim_eval(&modifier->amount, time);
            param[1] = sr_anim_eval(&modifier->frequency, time);
            param[2] = sr_anim_eval(&modifier->phase, time);
        }
    }
    size_t total = 0;
    for (size_t i = 0; i < node->modifier_count; ++i)
        if (node->modifiers[i].type == SR_MOD_MESH_WARP)
            total += (size_t)node->modifiers[i].rows * node->modifiers[i].cols * 2;
    if (total) {
        state->mesh = sr_composite_alloc(resources, node->modifier_count,
                                          sizeof(*state->mesh), 0);
        double *values = state->storage = sr_composite_alloc(resources, total,
                                                              sizeof(*values), 0);
        if (!state->mesh || !values) {
            sr_deform_free(state);
            return sr_composite_resource_status(resources);
        }
        for (size_t i = 0; i < node->modifier_count; ++i) {
            const SrModifier *modifier = &node->modifiers[i];
            if (modifier->type != SR_MOD_MESH_WARP) continue;
            size_t count = (size_t)modifier->rows * modifier->cols * 2;
            state->mesh[i] = values;
            for (size_t j = 0; j < count; ++j)
                values[j] = sr_anim_eval(&modifier->points[j], time);
            values += count;
        }
    }
    if (node->soft_body.enabled && node->soft_body.sample_count) {
        size_t count = (size_t)node->soft_body.rows * node->soft_body.cols;
        state->soft = sr_composite_alloc(resources, count, 2 * sizeof(*state->soft), 0);
        if (!state->soft) {
            sr_deform_free(state);
            return sr_composite_resource_status(resources);
        }
        if (!sr_physics_soft_offsets(scene, node, time, state->soft)) {
            sr_composite_free(resources, state->soft);
            state->soft = NULL;
        }
    }
    return SR_OK;
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
    int width_px = (int)target->width, height_px = (int)target->height;
    SrClip clip = {sr_clamp_int(floor(min_x - 1.0), 0, width_px),
                   sr_clamp_int(floor(min_y - 1.0), 0, height_px),
                   sr_clamp_int(ceil(max_x + 1.0), 0, width_px),
                   sr_clamp_int(ceil(max_y + 1.0), 0, height_px)};
    if (clip.x1 < clip.x0) clip.x1 = clip.x0;
    if (clip.y1 < clip.y0) clip.y1 = clip.y0;
    return clip;
}

/* Propagate a conservative local box through the forward modifiers in
 * application order. In particular, a bend after a wave/grid must bound
 * the already displaced box, not just the original node dimensions. */
static SrClip sr_deformed_bounds(const SrNode *node, SrMat3 world,
                                 double width, double height, double pad,
                                 double time, const SrDeformState *state,
                                 const SrTarget *target) {
    double x0 = -pad, y0 = -pad, x1 = width + pad, y1 = height + pad;
    double cx = width * .5, cy = height * .5;
    if (state->soft) {
        double extent = sr_grid_warp_extent(state->soft,
                            (size_t)node->soft_body.rows * node->soft_body.cols);
        x0 -= extent; y0 -= extent; x1 += extent; y1 += extent;
    }
    for (size_t i = 0; i < node->modifier_count; ++i) {
        const SrModifier *modifier = &node->modifiers[i];
        double amount = sr_anim_eval(&modifier->amount, time);
        double dx = 0.0, dy = 0.0;
        if (modifier->type == SR_MOD_MESH_WARP) {
            if (state->mesh && state->mesh[i])
                dx = dy = sr_grid_warp_extent(state->mesh[i],
                                  (size_t)modifier->rows * modifier->cols);
        } else if (modifier->type == SR_MOD_WAVE) {
            if (modifier->axis == 'x') dx = fabs(amount);
            else dy = fabs(amount);
        } else if (modifier->type == SR_MOD_BEND) {
            if (modifier->axis == 'x') {
                double n = fmax(fabs(y0 - cy), fabs(y1 - cy)) / fmax(height, 1.0);
                dx = fabs(amount) * n * n;
            } else {
                double n = fmax(fabs(x0 - cx), fabs(x1 - cx)) / fmax(width, 1.0);
                dy = fabs(amount) * n * n;
            }
        } else if (modifier->type == SR_MOD_TWIST && amount != 0.0) {
            /* The inverse twist preserves Euclidean distance from the
             * centre, including for non-square source boxes. */
            double radius = hypot(fmax(fabs(x0 - cx), fabs(x1 - cx)),
                                  fmax(fabs(y0 - cy), fabs(y1 - cy)));
            x0 = cx - radius; x1 = cx + radius;
            y0 = cy - radius; y1 = cy + radius;
        } else if (modifier->type == SR_MOD_SQUASH ||
                   modifier->type == SR_MOD_STRETCH) {
            double factor = fmax(.05, 1.0 +
                (modifier->type == SR_MOD_SQUASH ? -amount : amount));
            x0 = cx + (x0 - cx) / factor; x1 = cx + (x1 - cx) / factor;
            y0 = cy + (y0 - cy) * factor; y1 = cy + (y1 - cy) * factor;
        }
        x0 -= dx; x1 += dx; y0 -= dy; y1 += dy;
        if (!isfinite(x0) || !isfinite(x1) || !isfinite(y0) || !isfinite(y1))
            return (SrClip){0, 0, (int)target->width, (int)target->height};
    }
    if (!isfinite(x1 - x0) || !isfinite(y1 - y0))
        return (SrClip){0, 0, (int)target->width, (int)target->height};
    return sr_bounds(world, x0, y0, x1 - x0, y1 - y0, target);
}

/* Local size of one canvas pixel: sqrt(|det|) of the inverse transform. */
static double sr_pixel_footprint(SrMat3 inverse) {
    return sqrt(fabs(inverse.m00 * inverse.m11 - inverse.m01 * inverse.m10));
}

/* ---- masks -------------------------------------------------------------- */

static bool sr_masks_eval(const SrDrawContext *context, const SrNode *node, SrMaskEval *out) {
    double time = context->time;
    const SrNodeGeometry *geometry = sr_length_node(context->lengths, node);
    if (!sr_composite_masks_work(context->compositor->resources, node, geometry != NULL))
        return false;
    for (size_t i = 0; i < node->mask_count; ++i) {
        const SrMask *mask = &node->masks[i];
        const SrMaskGeometry *evaluated = geometry
            ? &context->lengths->masks[geometry->mask_offset + i] : NULL;
        out[i] = (SrMaskEval){
            .type = mask->type,
            .invert = mask->invert,
            .x = evaluated ? evaluated->x : sr_anim_eval(&mask->x, time),
            .y = evaluated ? evaluated->y : sr_anim_eval(&mask->y, time),
            .width = evaluated ? evaluated->width : fmax(0.0, sr_anim_eval(&mask->width, time)),
            .height = evaluated ? evaluated->height : fmax(0.0, sr_anim_eval(&mask->height, time)),
            .radius = fmax(0.0, sr_anim_eval(&mask->radius, time))};
    }
    return true;
}

/* sr_mat_point, inlined: the same products and sums in the same order. */
static inline SrVec2 sr_mat_apply(const SrMat3 *m, double x, double y) {
    return (SrVec2){m->m00 * x + m->m01 * y + m->m02,
                    m->m10 * x + m->m11 * y + m->m12};
}

static float sr_link_coverage(const SrMaskLink *link, double cx, double cy) {
    float coverage = 1.0f;
    for (; link && coverage > 0.0f; link = link->parent) {
        SrVec2 local = sr_mat_apply(&link->inverse, cx, cy);
        for (size_t i = 0; i < link->count && coverage > 0.0f; ++i) {
            const SrMaskEval *m = &link->masks[i];
            float c = sr_distance_coverage_inline(
                sr_shape_distance_inline(m->type, m->x, m->y, m->width,
                                         m->height, m->radius, local.x,
                                         local.y),
                link->aa);
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

/* Bilinear filter on premultiplied texels, clamped to the edge texels;
 * texel (i, j) is centred at (i + 0.5, j + 0.5). Edge anti-aliasing comes
 * from the geometric coverage of the image rectangle instead (see
 * sr_op_rows), so the fade is about one output pixel at any scale. */
static void sr_image_sample(const SrImage *image, double lx, double ly,
                            float out[4]) {
    double sx = sr_clamp(lx - 0.5, 0.0, (double)image->width - 1.0);
    double sy = sr_clamp(ly - 0.5, 0.0, (double)image->height - 1.0);
    double fx = floor(sx), fy = floor(sy);
    int ix = sr_clamp_int(fx, 0, (int)image->width - 1);
    int iy = sr_clamp_int(fy, 0, (int)image->height - 1);
    int ix1 = ix + 1 < (int)image->width ? ix + 1 : ix;
    int iy1 = iy + 1 < (int)image->height ? iy + 1 : iy;
    float tx = (float)(sx - fx), ty = (float)(sy - fy);
    out[0] = out[1] = out[2] = out[3] = 0.0f;
    sr_texel(image, ix, iy, (1.0f - tx) * (1.0f - ty), out);
    sr_texel(image, ix1, iy, tx * (1.0f - ty), out);
    sr_texel(image, ix, iy1, (1.0f - tx) * ty, out);
    sr_texel(image, ix1, iy1, tx * ty, out);
}

/* The row half of sr_image_sample at local y `ly`: the two texel rows and
 * the vertical weight, with the same operations sr_image_sample uses. The
 * image must be at least 1x1 (then every clamped index is in range and
 * sr_texel's bounds test always passes). */
static inline void sr_image_rows(const SrImage *image, double ly,
                                 const float **row0, const float **row1,
                                 float *ty) {
    double sy = sr_clamp(ly - 0.5, 0.0, (double)image->height - 1.0);
    double fy = floor(sy);
    int iy = sr_clamp_int(fy, 0, (int)image->height - 1);
    int iy1 = iy + 1 < (int)image->height ? iy + 1 : iy;
    *ty = (float)(sy - fy);
    *row0 = image->px + (size_t)iy * image->width * 4;
    *row1 = image->px + (size_t)iy1 * image->width * 4;
}

static inline void sr_texel_at(const float *row, int x, float weight,
                               float acc[4]) {
    if (weight == 0.0f) return;
    const float *p = row + (size_t)x * 4;
    acc[0] += p[0] * weight;
    acc[1] += p[1] * weight;
    acc[2] += p[2] * weight;
    acc[3] += p[3] * weight;
}

/* The column half of sr_image_sample: identical taps, weights and
 * accumulation order. */
static inline void sr_image_sample_rows(const float *row0, const float *row1,
                                        int width, float ty, double lx,
                                        float out[4]) {
    double sx = sr_clamp(lx - 0.5, 0.0, (double)width - 1.0);
    double fx = floor(sx);
    int ix = sr_clamp_int(fx, 0, width - 1);
    int ix1 = ix + 1 < width ? ix + 1 : ix;
    float tx = (float)(sx - fx);
    out[0] = out[1] = out[2] = out[3] = 0.0f;
    sr_texel_at(row0, ix, (1.0f - tx) * (1.0f - ty), out);
    sr_texel_at(row0, ix1, tx * (1.0f - ty), out);
    sr_texel_at(row1, ix, (1.0f - tx) * ty, out);
    sr_texel_at(row1, ix1, tx * ty, out);
}

/* Fraction of pixel (x, y)'s depth samples at which the card is visible;
 * writes the card depth into visible samples when `alpha` (the composited
 * card alpha) reaches 0.5. Samples of one pixel row belong to one depth
 * row band, so row-parallel composites stay race free and deterministic. */
static float sr_card_visibility(const SrCardTest *test, int x, int y,
                                float alpha) {
    SrDepthBuffer *depth = test->depth;
    int n = depth ? depth->samples : 1;
    int visible = 0;
    bool write = test->write && depth && alpha >= 0.5f;
    for (int j = 0; j < n; ++j) for (int i = 0; i < n; ++i) {
        double z;
        if (!sr_card_depth_at(test->view, &test->pose, x + (i + .5) / n,
                              y + (j + .5) / n, &z) ||
            z < test->view->near_plane || z > test->view->far_plane)
            continue;
        if (depth) {
            double *slot = &depth->z[((size_t)y * (size_t)n + (size_t)j) *
                                     depth->width + (size_t)x * (size_t)n +
                                     (size_t)i];
            /* Ties go to the later draw, as ordinary compositing does.
             * (az > 1.0 ? az : 1.0) is fmax(1.0, az): az is never -0.0
             * and a NaN az gives 1.0 either way. */
            double az = fabs(z);
            if (z > *slot + 1e-9 * (az > 1.0 ? az : 1.0)) continue;
            if (write && z < *slot) *slot = z;
        }
        ++visible;
    }
    return (float)visible / (float)(n * n);
}

/* The per-pixel body of every draw op but particles. The op's invariants
 * are copied into locals first (the float stores into the target could
 * otherwise alias the op's fields and force re-reads), and the flags that
 * are constant per op (kind, shape, normal blend, masks, deformation, card
 * test) are passed as compile-time constants by sr_op_rows so each
 * combination gets its own branch-free loop. Every pixel still evaluates
 * exactly the same float and double operations in the same order as the
 * generic formulation (sr_mat_point, sr_shape_coverage, sr_image_sample,
 * sr_blend_px):
 *  - without masks the coverage opacity * 1.0f is opacity itself;
 *  - the inverse transform keeps (m00*cx + m01*cy) + m02, only the
 *    row-constant product m01*cy (m11*cy) is computed once per row;
 *  - `axis` (inverse with m10 == 0 and a non-degenerate box): m10*cx is
 *    then the same signed zero for every cx > 0, so local y, and every term
 *    of the rect/ellipse distance and of the bilinear sample that depends
 *    only on it, is a row constant computed once per row with the very
 *    same operations;
 *  - a buffer pixel whose alpha is not positive is skipped before the
 *    masks: the composite of such a pixel is a no-op in sr_blend_px (the
 *    scaled alpha is not positive either) and it never reaches the 0.5
 *    alpha at which a card writes depth. */
/* Forced specialization is useful only with constant folding enabled.
 * At -O0 it creates 33 copies with unreachable flag branches, obscuring
 * both debugger stepping and coverage of the shared pixel kernel. */
#if defined(__OPTIMIZE__)
#define SR_OP_INLINE __attribute__((always_inline))
#else
#define SR_OP_INLINE
#endif
static inline SR_OP_INLINE void sr_op_rows_impl(
    const SrDrawOp *op, size_t begin, size_t end, SrOpKind kind,
    SrMaskType shape, bool normal, bool masked, bool deform, bool axis,
    bool card) {
    float *const base = op->target.px;
    const size_t stride = op->target.width;
    const int x0 = op->bounds.x0, x1 = op->bounds.x1;
    const size_t y0 = (size_t)op->bounds.y0;
    const SrBlendMode blend = normal ? SR_BLEND_NORMAL : op->blend;
    const float opacity = op->opacity;
    const SrMaskLink *const masks = masked ? op->masks : NULL;
    const SrMat3 inv = op->inverse;
    const double aa = op->aa;
    const float *const buffer_px =
        kind == SR_OP_BUFFER ? op->buffer->frame.px : NULL;
    const SrCardTest *const test = card ? &op->card : NULL;
    const SrImage *const image = kind == SR_OP_IMAGE ? op->image : NULL;
    const int image_w = kind == SR_OP_IMAGE ? (int)image->width : 0;
    /* sr_shape_distance(RECT or shape, 0, 0, w, h, 0, lx, ly) pieces; an
     * image covers the rect of its own size. */
    const double shape_w = kind == SR_OP_IMAGE ? (double)image->width
                                               : op->width;
    const double shape_h = kind == SR_OP_IMAGE ? (double)image->height
                                               : op->height;
    const SrMaskType dist_shape = kind == SR_OP_IMAGE ? SR_MASK_RECT : shape;
    const double half_stroke = op->half_stroke;
    const float fill[4] = {op->fill[0], op->fill[1], op->fill[2], op->fill[3]};
    const float stroke[4] = {op->stroke[0], op->stroke[1], op->stroke[2],
                             op->stroke[3]};
    const SrNode *const node = op->node;
    const SrDeformState *const deform_state = op->deform;
    const double deform_w = op->deform_width, deform_h = op->deform_height;
    const double hw = 0.5 * shape_w, hh = 0.5 * shape_h;
    const double center_x = 0.0 + hw, center_y = 0.0 + hh;
    const double half_x = hw - 0.0, half_y = hh - 0.0;  /* r = 0 */
    const double ellipse_min = -(hw < hh ? hw : hh);
    if (!masked && !(opacity > 0.0f)) return;
    for (size_t y = y0 + begin; y < y0 + end; ++y) {
        const double cy = (double)y + 0.5;
        const double row_x = inv.m01 * cy, row_y = inv.m11 * cy;
        float *restrict d = base + (y * stride + (size_t)x0) * 4;
        /* axis: ellipse row_a = uy*uy, row_b = gy*gy; rect row_a = qy,
         * row_b = oy*oy; images also get their texel rows. */
        double row_a = 0.0, row_b = 0.0;
        const float *img0 = NULL, *img1 = NULL;
        float img_ty = 0.0f;
        if (axis) {
            double ly = inv.m10 * 0.5 + row_y + inv.m12;
            double py = ly - center_y;
            if (dist_shape == SR_MASK_ELLIPSE) {
                double uy = py / hh;
                double gy = uy / hh;
                row_a = uy * uy;
                row_b = gy * gy;
            } else {
                double qy = fabs(py) - half_y;
                double oy = qy > 0.0 ? qy : 0.0;
                row_a = qy;
                row_b = oy * oy;
            }
            if (kind == SR_OP_IMAGE)
                sr_image_rows(image, ly, &img0, &img1, &img_ty);
        }
        for (int x = x0; x < x1; ++x, d += 4) {
            const double cx = (double)x + 0.5;
            const float *src = NULL;
            if (kind == SR_OP_BUFFER) {
                src = buffer_px + (d - base);
                if (!(src[3] > 0.0f)) continue;
            }
            float coverage =
                masked ? opacity * sr_link_coverage(masks, cx, cy) : opacity;
            if (masked && !(coverage > 0.0f)) continue;
            float s[4];
            if (kind == SR_OP_BUFFER) {
                memcpy(s, src, sizeof(s));
                if (card) {
                    coverage *= sr_card_visibility(test, x, (int)y,
                                                   coverage * s[3]);
                    if (!(coverage > 0.0f)) continue;
                }
            } else if (axis) {
                double px = inv.m00 * cx + row_x + inv.m02 - center_x;
                double sd;
                if (dist_shape == SR_MASK_ELLIPSE) {
                    double ux = px / hw;
                    double k0 = sqrt(ux * ux + row_a);
                    double gx = ux / hw;
                    double k1 = sqrt(gx * gx + row_b);
                    sd = k1 > 1e-12 ? k0 * (k0 - 1.0) / k1 : ellipse_min;
                } else {
                    double qx = fabs(px) - half_x;
                    double ox = qx > 0.0 ? qx : 0.0;
                    double inside = qx > row_a ? qx : row_a;
                    sd = sqrt(ox * ox + row_b) +
                         (inside < 0.0 ? inside : 0.0) - 0.0;
                }
                if (kind == SR_OP_IMAGE) {
                    coverage *= sr_distance_coverage_inline(sd, aa);
                    if (!(coverage > 0.0f)) continue;
                    sr_image_sample_rows(img0, img1, image_w, img_ty,
                                         inv.m00 * cx + row_x + inv.m02, s);
                } else {
                    float cf = sr_distance_coverage_inline(sd, aa);
                    float cs = half_stroke > 0.0
                        ? sr_distance_coverage_inline(fabs(sd) - half_stroke,
                                                      aa)
                        : 0.0f;
                    float keep = 1.0f - cs * stroke[3];
                    for (int c = 0; c < 4; ++c)
                        s[c] = stroke[c] * cs + fill[c] * cf * keep;
                }
            } else {
                SrVec2 local = {inv.m00 * cx + row_x + inv.m02,
                                inv.m10 * cx + row_y + inv.m12};
                if (deform &&
                    !sr_deform_inverse(node, &local, deform_w, deform_h,
                                       deform_state))
                    continue;
                double sd = sr_shape_distance_inline(dist_shape, 0.0, 0.0,
                                                     shape_w, shape_h, 0.0,
                                                     local.x, local.y);
                if (kind == SR_OP_IMAGE) {
                    coverage *= sr_distance_coverage_inline(sd, aa);
                    if (!(coverage > 0.0f)) continue;
                    const float *r0, *r1;
                    float ty;
                    sr_image_rows(image, local.y, &r0, &r1, &ty);
                    sr_image_sample_rows(r0, r1, image_w, ty, local.x, s);
                } else {
                    float cf = sr_distance_coverage_inline(sd, aa);
                    float cs = half_stroke > 0.0
                        ? sr_distance_coverage_inline(fabs(sd) - half_stroke,
                                                      aa)
                        : 0.0f;
                    float keep = 1.0f - cs * stroke[3];
                    for (int c = 0; c < 4; ++c)
                        s[c] = stroke[c] * cs + fill[c] * cf * keep;
                }
            }
            s[0] *= coverage; s[1] *= coverage;
            s[2] *= coverage; s[3] *= coverage;
            sr_blend_px_inline(blend, d, s);
        }
    }
}

#undef SR_OP_INLINE

/* Deforming draws are rare and dominated by the inverse warp: one generic
 * loop with runtime flags. */
static void sr_op_rows_deform(const SrDrawOp *op, size_t begin, size_t end) {
    sr_op_rows_impl(op, begin, end, op->kind, op->shape,
                    op->blend == SR_BLEND_NORMAL, op->masks != NULL, true,
                    false, false);
}

#define SR_OP_ROWS_VARIANTS(kind, shape, axis, card)                         \
    do {                                                                     \
        if (masked) {                                                        \
            if (normal)                                                      \
                sr_op_rows_impl(op, begin, end, kind, shape, true, true,     \
                                false, axis, card);                          \
            else                                                             \
                sr_op_rows_impl(op, begin, end, kind, shape, false, true,    \
                                false, axis, card);                          \
        } else {                                                             \
            if (normal)                                                      \
                sr_op_rows_impl(op, begin, end, kind, shape, true, false,    \
                                false, axis, card);                          \
            else                                                             \
                sr_op_rows_impl(op, begin, end, kind, shape, false, false,   \
                                false, axis, card);                          \
        }                                                                    \
    } while (0)

/* One anti-aliased particle disc (or axis-aligned square); the same
 * per-pixel arithmetic the particles always used. */
static void sr_disc_rows(const SrDrawOp *op, size_t begin, size_t end) {
    float *const base = op->target.px;
    const size_t stride = op->target.width;
    const int x0 = op->bounds.x0, x1 = op->bounds.x1;
    const SrBlendMode blend = op->blend;
    const float opacity = op->opacity;
    const SrMaskLink *const masks = op->masks;
    const SrVec2 center = op->center;
    const double radius = op->radius;
    const bool square = op->square;
    const float color[4] = {op->fill[0], op->fill[1], op->fill[2],
                            op->fill[3]};
    for (int y = op->bounds.y0 + (int)begin; y < op->bounds.y0 + (int)end;
         ++y) {
        float *restrict d = base + ((size_t)y * stride + (size_t)x0) * 4;
        double dy = y + 0.5 - center.y;
        for (int x = x0; x < x1; ++x, d += 4) {
            double dx = x + 0.5 - center.x;
            double distance = square ? fmax(fabs(dx), fabs(dy)) - radius
                                     : sqrt(dx * dx + dy * dy) - radius;
            float coverage = sr_distance_coverage_inline(distance, 1.0);
            if (!(coverage > 0.0f)) continue;
            coverage *= opacity * sr_link_coverage(masks, x + 0.5, y + 0.5);
            if (!(coverage > 0.0f)) continue;
            float s[4] = {color[0] * coverage, color[1] * coverage,
                          color[2] * coverage, color[3] * coverage};
            sr_blend_px_inline(blend, d, s);
        }
    }
}

/* Rows are offsets from bounds.y0 so sr_parallel_for can split [0, rows). */
static void sr_op_rows(void *opaque, size_t begin, size_t end) {
    const SrDrawOp *op = opaque;
    if (op->kind == SR_OP_DISC) {
        sr_disc_rows(op, begin, end);
        return;
    }
    if (op->kind == SR_OP_CLEAR) {
        size_t row = (size_t)(op->bounds.x1 - op->bounds.x0) * 4 * sizeof(float);
        for (size_t y = (size_t)op->bounds.y0 + begin;
             y < (size_t)op->bounds.y0 + end; ++y)
            memset(op->target.px + (y * op->target.width +
                                    (size_t)op->bounds.x0) * 4, 0, row);
        return;
    }
    if (op->kind != SR_OP_BUFFER && sr_node_deforms(op->node)) {
        sr_op_rows_deform(op, begin, end);
        return;
    }
    bool masked = op->masks != NULL;
    bool normal = op->blend == SR_BLEND_NORMAL;
    if (op->kind == SR_OP_BUFFER) {
        if (op->has_card)
            SR_OP_ROWS_VARIANTS(SR_OP_BUFFER, SR_MASK_RECT, false, true);
        else
            SR_OP_ROWS_VARIANTS(SR_OP_BUFFER, SR_MASK_RECT, false, false);
    } else if (op->kind == SR_OP_IMAGE) {
        if (op->inverse.m10 == 0.0 && op->image->width > 0 &&
            op->image->height > 0)
            SR_OP_ROWS_VARIANTS(SR_OP_IMAGE, SR_MASK_RECT, true, false);
        else
            SR_OP_ROWS_VARIANTS(SR_OP_IMAGE, SR_MASK_RECT, false, false);
    } else {
        bool axis = op->inverse.m10 == 0.0 && op->width > 0.0 &&
                    op->height > 0.0;
        if (op->shape == SR_MASK_ELLIPSE && axis)
            SR_OP_ROWS_VARIANTS(SR_OP_SHAPE, SR_MASK_ELLIPSE, true, false);
        else if (op->shape == SR_MASK_ELLIPSE)
            SR_OP_ROWS_VARIANTS(SR_OP_SHAPE, SR_MASK_ELLIPSE, false, false);
        else if (axis)
            SR_OP_ROWS_VARIANTS(SR_OP_SHAPE, SR_MASK_RECT, true, false);
        else
            SR_OP_ROWS_VARIANTS(SR_OP_SHAPE, SR_MASK_RECT, false, false);
    }
}

static unsigned sr_op_threads(const SrCompositor *compositor, SrClip bounds) {
    size_t area = (size_t)(bounds.x1 - bounds.x0) * (size_t)(bounds.y1 - bounds.y0);
    return area < SR_PARALLEL_MIN_PIXELS ? 1U : compositor->threads;
}

/* ---- op queue ------------------------------------------------------------
 * Draw ops are recorded and executed in batches: the covered rows are cut
 * into bands of SR_BAND_ROWS rows, and each worker runs the whole op list,
 * in submission order, over its bands. A band stays in cache across the
 * ops instead of the full frame streaming through memory once per op, and
 * there is one fork/join per batch instead of one per op.
 * Correctness: an op writes only its own pixel and reads only that same
 * pixel (the backdrop, for SR_OP_BUFFER the buffer at the same coordinates
 * and, for a card composite, that pixel's own depth samples), so every
 * pixel receives exactly the ops it received before, in the same order,
 * whatever the band split or thread count (clearing a reused pool buffer
 * is an op too). Anything that reads or writes pixels or depth samples
 * elsewhere flushes the queue first: freeing a pool buffer, group and card
 * effects, the perspective warp, every 3D object draw, and the end of the
 * render. Ops whose inputs do not outlive the
 * call (deformation state, video frames the decoder may recycle) run
 * immediately after a flush. */

#define SR_BAND_ROWS 8

typedef struct {
    const SrDrawOp *ops;
    size_t count;
    int y0, y1;
    const uint32_t *order;    /* work item -> band, or NULL for identity */
} SrBandJob;

static void sr_band_rows(void *opaque, size_t begin, size_t end) {
    const SrBandJob *job = opaque;
    for (size_t item = begin; item < end; ++item) {
        size_t band = job->order ? job->order[item] : item;
        int r0 = job->y0 + (int)band * SR_BAND_ROWS;
        int r1 = r0 + SR_BAND_ROWS < job->y1 ? r0 + SR_BAND_ROWS : job->y1;
        for (size_t i = 0; i < job->count; ++i) {
            const SrDrawOp *op = &job->ops[i];
            int a = op->bounds.y0 > r0 ? op->bounds.y0 : r0;
            int b = op->bounds.y1 < r1 ? op->bounds.y1 : r1;
            if (a < b)
                sr_op_rows((void *)op, (size_t)(a - op->bounds.y0),
                           (size_t)(b - op->bounds.y0));
        }
    }
}

static SrStatus sr_queue_flush(SrCompositor *compositor) {
    struct SrOpQueue *queue = compositor->queue;
    if (!queue || queue->count == 0) return SR_OK;
    SrCompositeResources *resources = compositor->resources;
    SrCompositeOwner previous = sr_composite_owner(resources,
                                                     queue->ops[queue->count - 1].owner);
    if (!sr_composite_work(resources, queue->count, 2)) {
        sr_composite_owner(resources, previous);
        return SR_ERR_RENDER;
    }
    int y0 = queue->ops[0].bounds.y0, y1 = queue->ops[0].bounds.y1;
    for (size_t i = 1; i < queue->count; ++i) {
        if (queue->ops[i].bounds.y0 < y0) y0 = queue->ops[i].bounds.y0;
        if (queue->ops[i].bounds.y1 > y1) y1 = queue->ops[i].bounds.y1;
    }
    SrBandJob job = {queue->ops, queue->count, y0, y1, NULL};
    size_t bands = ((size_t)(y1 - y0) + SR_BAND_ROWS - 1) / SR_BAND_ROWS;
    if (!sr_composite_work(resources, bands, queue->count + 1)) {
        sr_composite_owner(resources, previous);
        return SR_ERR_RENDER;
    }
    unsigned threads = queue->pixels < SR_PARALLEL_MIN_PIXELS
                           ? 1U : compositor->threads;
    /* sr_parallel_for hands each worker a contiguous run of work items;
     * dealing the bands round-robin instead spreads dense regions of the
     * frame over all workers. Which worker runs a band never changes its
     * pixels. */
    unsigned workers = sr_parallel_thread_count(threads, bands);
    if (workers > 1 || resources) {
        if (queue->order_capacity < bands) {
            uint32_t *order = sr_composite_realloc(resources,
                queue->order, bands, sizeof(*order), 0);
            if (!order) {
                sr_composite_owner(resources, previous);
                return sr_composite_resource_status(resources);
            }
            queue->order = order;
            queue->order_capacity = bands;
        }
    }
    if (workers > 1) {
        size_t item = 0;
        for (size_t first = 0; first < workers; ++first)
            for (size_t band = first; band < bands; band += workers)
                queue->order[item++] = (uint32_t)band;
        job.order = queue->order;
    }
    SrStatus status = sr_parallel_for(bands, threads, sr_band_rows, &job);
    for (size_t i = 0; i < queue->count; ++i) sr_composite_free(resources, queue->ops[i].owned);
    queue->count = 0;
    queue->pixels = 0;
    sr_composite_owner(resources, previous);
    return status;
}

/* Copies a mask chain (links and their evaluated masks) into one block so
 * a queued op does not point into its caller's stack frames. */
static SrStatus sr_mask_chain_copy(SrCompositeResources *resources,
                                    SrDrawOp *op) {
    size_t links = 0, masks = 0;
    for (const SrMaskLink *link = op->masks; link; link = link->parent) {
        if (!sr_composite_work(resources, 1, 1)) return SR_ERR_RENDER;
        ++links;
        masks += link->count;
    }
    /* Prepared ancestry bounds both sums before computing the copy size. */
    size_t bytes = links * sizeof(SrMaskLink) + masks * sizeof(SrMaskEval);
    /* Allocation zeroing and payload copying are distinct full passes. */
    if (!sr_composite_work(resources, bytes / 4 + (bytes % 4 != 0) + links, 1))
        return SR_ERR_RENDER;
    SrMaskLink *copy = sr_composite_alloc(resources, 1, bytes, 0);
    if (!copy) return sr_composite_resource_status(resources);
    SrMaskEval *evals = (SrMaskEval *)(copy + links);
    size_t i = 0;
    for (const SrMaskLink *link = op->masks; link; link = link->parent, ++i) {
        copy[i] = *link;
        if (link->count)
            memcpy(evals, link->masks, link->count * sizeof(*evals));
        copy[i].masks = evals;
        copy[i].parent = link->parent ? &copy[i + 1] : NULL;
        evals += link->count;
    }
    op->masks = copy;
    op->owned = copy;
    return SR_OK;
}

/* Records `op` (or, with `immediate`, flushes the queue and runs it now,
 * row-parallel on its own). `shared`, when given, lets a run of ops with
 * the same mask chain (one particle emitter) share one queued copy:
 * *shared starts NULL and receives the first copy made. */
static SrStatus sr_op_submit_shared(SrCompositor *compositor,
                                    const SrDrawOp *op, bool immediate,
                                    const SrMaskLink **shared) {
    if (op->bounds.x1 <= op->bounds.x0 || op->bounds.y1 <= op->bounds.y0)
        return SR_OK;
    SrCompositeResources *resources = compositor->resources;
    if (resources) {
        /* One raster/blend step, every inherited mask and link, and every
         * card depth sample, plus the full inverse-deformation fallback. */
        uint64_t cost = 1;
        if (op->deform) cost += op->deform->pixel_work;
        for (const SrMaskLink *link = op->masks; link; link = link->parent) {
            if (!sr_composite_work(resources, 1, 1)) return SR_ERR_RENDER;
            cost += 1 + link->count;
        }
        if (op->has_card && op->card.depth) {
            uint64_t samples = (uint64_t)op->card.depth->samples;
            cost += samples * samples;
        }
        uint64_t area = (uint64_t)(op->bounds.x1 - op->bounds.x0) *
                        (uint64_t)(op->bounds.y1 - op->bounds.y0);
        /* A clear charges the whole buffer, independent of dirty history. */
        if (op->kind == SR_OP_CLEAR) {
            area = (uint64_t)op->target.width * op->target.height;
            cost = 4;
        }
        if (!sr_composite_work(resources, area, cost)) return SR_ERR_RENDER;
    }
    if (op->target.buffer) sr_buffer_mark(op->target.buffer, op->bounds);
    if (immediate) {
        SrStatus status = sr_queue_flush(compositor);
        if (status != SR_OK) return status;
        return sr_parallel_for((size_t)(op->bounds.y1 - op->bounds.y0),
                               sr_op_threads(compositor, op->bounds),
                               sr_op_rows, (void *)op);
    }
    struct SrOpQueue *queue = compositor->queue;
    if (!queue) {
        queue = compositor->queue = sr_composite_alloc(resources, 1, sizeof(*queue), 0);
        if (!queue) return sr_composite_resource_status(resources);
        *queue = (struct SrOpQueue){0};
    }
    if (queue->count == queue->capacity) {
        size_t capacity = queue->capacity ? queue->capacity * 2 : 64;
        SrDrawOp *ops = sr_composite_realloc(resources, queue->ops,
            capacity, sizeof(*ops), 0);
        if (!ops) return sr_composite_resource_status(resources);
        queue->ops = ops;
        queue->capacity = capacity;
    }
    SrDrawOp *slot = &queue->ops[queue->count];
    *slot = *op;
    slot->owned = NULL;
    slot->owner = resources ? resources->owner : (SrCompositeOwner){0};
    if (slot->masks && shared && *shared) {
        slot->masks = *shared;
    } else if (slot->masks) {
        SrStatus status = sr_mask_chain_copy(resources, slot);
        if (status != SR_OK) return status;
        if (shared) *shared = slot->masks;
    }
    ++queue->count;
    queue->pixels += (size_t)(op->bounds.x1 - op->bounds.x0) *
                     (size_t)(op->bounds.y1 - op->bounds.y0);
    return SR_OK;
}

static SrStatus sr_op_submit(SrCompositor *compositor, const SrDrawOp *op,
                             bool immediate) {
    return sr_op_submit_shared(compositor, op, immediate, NULL);
}

/* ---- nodes -------------------------------------------------------------- */

static double sr_media_time(const SrNode *node, double time, bool *before) {
    *before = false;
    if (node->source_time.track.count) return sr_anim_eval(&node->source_time, time);
    double local = (time - node->start_time) * node->speed /
                   fmax(node->time_stretch, 1e-12);
    double end = node->clip_out >= 0.0 ? node->clip_out :
                 (node->asset ? node->asset->duration : 0.0);
    double span = fmax(0.0, end - node->clip_in);
    if (span <= 0.0) return node->clip_in;
    int64_t plays = node->loop_count > 0 ? node->loop_count : 1;
    double maximum = span * (double)plays;
    if (local >= maximum) {
        *before = !node->reverse;
        return node->reverse ? node->clip_in : end;
    }
    local = fmod(fmax(0.0, local), span);
    *before = node->reverse;
    return node->reverse ? end - local : node->clip_in + local;
}

static SrDrawOp sr_op_base(const SrDrawContext *context, const SrNode *node,
                           const SrTarget *target, SrMat3 inverse,
                           double opacity, double time,
                           const SrMaskLink *masks) {
    return (SrDrawOp){.target = *target, .blend = sr_node_blend(context, node),
                      .opacity = (float)opacity, .inverse = inverse,
                      .aa = sr_pixel_footprint(inverse), .masks = masks,
                      .node = node, .time = time};
}

static SrStatus sr_draw_image(SrDrawContext *context, const SrNode *node,
                              SrMat3 world, SrMat3 inverse, double opacity,
                              SrClip clip, const SrTarget *target,
                              const SrMaskLink *masks) {
    SrCompositeResources *resources = context->compositor->resources;
    if (resources && node->source_time.track.count) {
        SrCompositeOwner previous = sr_composite_owner(resources,
            sr_composite_node_owner(context->scene, node, "sourceTime"));
        bool valid = sr_composite_anim_work(resources, &node->source_time, false);
        sr_composite_owner(resources, previous);
        if (!valid) return SR_ERR_RENDER;
    }
    if (!sr_composite_work(resources, 1, 32)) return SR_ERR_RENDER;
    bool before;
    double source_time = sr_media_time(node, context->time, &before);
    SrStatus frame_status = SR_OK;
    const SrImage *image = sr_asset_get_frame_status(
        context->scene, node->asset, source_time, before, context->diag,
        &frame_status);
    /* Decoding errors are reported through diagnostics (the render then
     * fails with SR_ERR_ASSET); running out of memory stops it at once. */
    if (!image) return frame_status == SR_ERR_MEMORY ? SR_ERR_MEMORY : SR_OK;
    SrDeformState deform;
    SrStatus status = sr_deform_prepare(context->scene, node, context->time,
                                        context->compositor->resources, &deform);
    if (status != SR_OK) return status;
    SrDrawOp op = sr_op_base(context, node, target, inverse, opacity,
                             context->time, masks);
    op.kind = SR_OP_IMAGE;
    op.image = image;
    op.deform_width = image->width;
    op.deform_height = image->height;
    op.deform = &deform;
    op.bounds = sr_clip_intersect(
        sr_deformed_bounds(node, world, image->width, image->height,
                            sr_node_deforms(node) ? op.aa : 0.0,
                            context->time, &deform, target), clip);
    /* The deformation state is freed below and a video frame may be
     * recycled by the next decode, so those draws cannot be queued. */
    bool immediate = sr_node_deforms(node) ||
                     node->asset->type == SR_ASSET_VIDEO;
    if (!immediate) op.deform = NULL;
    status = sr_op_submit(context->compositor, &op, immediate);
    sr_deform_free(&deform);
    return status;
}

static SrStatus sr_draw_shape(SrDrawContext *context, const SrNode *node,
                              SrMat3 world, SrMat3 inverse, double opacity,
                              SrClip clip, const SrTarget *target,
                              const SrMaskLink *masks) {
    SrCompositeResources *resources = context->compositor->resources;
    if (resources) {
        SrCompositeOwner previous = sr_composite_owner(resources,
            sr_composite_node_owner(context->scene, node, "fill/stroke"));
        bool valid = sr_composite_color_work(resources, &node->fill) &&
            sr_composite_color_work(resources, &node->stroke);
        sr_composite_owner(resources, previous);
        if (!valid) return SR_ERR_RENDER;
    }
    SrDeformState deform;
    SrStatus status = sr_deform_prepare(context->scene, node, context->time,
                                        context->compositor->resources, &deform);
    if (status != SR_OK) return status;
    SrDrawOp op = sr_op_base(context, node, target, inverse, opacity,
                             context->time, masks);
    op.kind = SR_OP_SHAPE;
    op.shape = node->shape == SR_SHAPE_ELLIPSE ? SR_MASK_ELLIPSE : SR_MASK_RECT;
    const SrNodeGeometry *geometry = sr_length_node(context->lengths, node);
    op.width = geometry ? geometry->box.width : node->shape_width;
    op.height = geometry ? geometry->box.height : node->shape_height;
    op.deform_width = op.width;
    op.deform_height = op.height;
    op.deform = &deform;
    op.half_stroke = node->stroke_width * 0.5;
    sr_color_to_blend(&context->scene->project,
                      sr_anim_color_eval(&node->fill, context->time), op.fill);
    sr_color_to_blend(&context->scene->project,
                      sr_anim_color_eval(&node->stroke, context->time), op.stroke);
    op.bounds = sr_clip_intersect(
        sr_deformed_bounds(node, world, op.width, op.height,
                            op.half_stroke + (sr_node_deforms(node) ? op.aa : 0.0),
                            context->time, &deform, target), clip);
    bool immediate = sr_node_deforms(node);  /* the state is freed below */
    if (!immediate) op.deform = NULL;
    status = sr_op_submit(context->compositor, &op, immediate);
    sr_deform_free(&deform);
    return status;
}

/* One anti-aliased disc (or axis-aligned square) per particle, queued as
 * SR_OP_DISC ops in particle order (particles are small and may overlap,
 * so order matters; the queue keeps it per pixel). */
static SrStatus sr_draw_particles(SrDrawContext *context, const SrNode *node,
                                  SrMat3 world, double opacity, SrClip clip,
                                  const SrTarget *target,
                                  const SrMaskLink *masks) {
    SrParticle *particles = NULL;
    size_t count = 0;
    SrCompositeResources *resources = context->compositor->resources;
    SrStatus status = sr_particles_eval_composite(context->scene, node, context->time,
                                                  resources, &particles, &count);
    if (status == SR_ERR_RENDER && (!resources || resources->status == SR_OK))
        sr_diag_error(context->diag, node->source_line, "particleEmitter", "rate/lifetime",
                      "animation exceeds the particle sampling budget or "
                      "9e15 emission-index limit");
    if (status != SR_OK) return status;
    SrDrawOp op = {.kind = SR_OP_DISC, .target = *target,
                   .blend = sr_node_blend(context, node),
                   .opacity = (float)opacity, .masks = masks,
                   .square = node->particle_shape == SR_PARTICLE_SQUARE};
    const SrMaskLink *shared = NULL;
    for (size_t i = 0; i < count && status == SR_OK; ++i) {
        const SrParticle *particle = &particles[i];
        SrVec2 center = sr_mat_point(world, (SrVec2){particle->x, particle->y});
        double radius = particle->radius * context->particle_scale;
        sr_color_to_blend(&context->scene->project, particle->color, op.fill);
        op.center = center;
        op.radius = radius;
        SrClip bounds = {
            sr_clamp_int(floor(center.x - radius - 1.0), clip.x0, clip.x1),
            sr_clamp_int(floor(center.y - radius - 1.0), clip.y0, clip.y1),
            sr_clamp_int(ceil(center.x + radius + 1.0), clip.x0, clip.x1),
            sr_clamp_int(ceil(center.y + radius + 1.0), clip.y0, clip.y1)};
        op.bounds = sr_clip_intersect(bounds, clip);
        status = sr_op_submit_shared(context->compositor, &op, false, &shared);
    }
    sr_composite_free(resources, particles);
    return status;
}

static SrStatus sr_draw_node(SrDrawContext *context, const SrNode *node,
                             SrMat3 parent, SrClip clip,
                             const SrTarget *target, size_t depth,
                             const SrMaskLink *outer);

/* View depth of a card's pivot (its sort key). */
static SrStatus sr_card_key(const SrDrawContext *context, const SrNode *node,
                            SrMat3 parent, double *out) {
    double time = context->time;
    SrDrawContext local = *context;
    SrMat3 plane;
    SrStatus status = sr_node_world(&local, node, parent, &plane);
    if (status != SR_OK) return status;
    const SrNodeGeometry *geometry = sr_length_node(context->lengths, node);
    if (!sr_composite_pivot_work(context->compositor->resources, context->scene,
                                  node, geometry != NULL, false)) return SR_ERR_RENDER;
    SrVec2 pivot = sr_mat_point(plane, (SrVec2){
        geometry ? geometry->anchor_x : sr_anim_eval(&node->transform.anchor_x, time),
        geometry ? geometry->anchor_y : sr_anim_eval(&node->transform.anchor_y, time)});
    double world[3] = {pivot.x - context->view->width * .5,
                       context->view->height * .5 - pivot.y,
                       sr_anim_eval(&node->transform.z, time)};
    double key = sr_card_view_depth(context->view, world);
    if (local.skewed && (!isfinite(pivot.x) || !isfinite(pivot.y) || !isfinite(key)))
        return sr_skew_error(context, node, "skewX/skewY",
                             "skewed card pivot and sort depth must be finite");
    *out = isnan(key) ? INFINITY : key;
    return SR_OK;
}

/* Children draw in (z, XML order). Each maximal run of consecutive card
 * siblings is re-sorted per frame far to near by pivot view depth (the
 * After Effects model: non-card siblings break runs). The first run of the
 * composition root also takes the 3D objects, keyed by their centers. */
static SrStatus sr_draw_children(SrDrawContext *context, const SrNode *node,
                                 SrMat3 world, SrClip clip,
                                 const SrTarget *target, size_t depth,
                                 const SrMaskLink *masks) {
    SrCompositeResources *resources = context->compositor->resources;
    /* Discovery, construction, drawing and final lighting-handoff scans,
     * including copied list records. Prepared ownership bounds children. */
    if (!sr_composite_work(resources, node->child_count,
                            8 + 2 * ((sizeof(SrDrawItem) + 3) / 4))) return SR_ERR_RENDER;
    bool cards = false;
    for (size_t i = 0; i < node->child_count && !cards; ++i)
        cards = node->children[i]->card;
    if (!cards) {
        for (size_t i = 0; i < node->child_count; ++i) {
            SrStatus status = sr_draw_node(context, node->children[i], world,
                                           clip, target, depth, masks);
            if (status != SR_OK) return status;
        }
        return SR_OK;
    }
    const SrScene *scene = context->scene;
    bool merge = context->lighting && node == scene->root && !target->buffer;
    if (resources && (node->child_count > SR_MAX_COMPOSITE_NODES ||
        scene->object3d_count > SR_MAX_COMPOSITE_OBJECTS)) {
        sr_composite_resource_fail(resources, SR_ERR_RENDER, "compositing draw-list limit");
        return SR_ERR_RENDER;
    }
    if (merge && !sr_composite_work(resources, scene->object3d_count,
        8 + 2 * ((sizeof(SrDrawItem) + 3) / 4))) return SR_ERR_RENDER;
    size_t capacity = node->child_count + (merge ? scene->object3d_count : 0);
    SrDrawItem *items = sr_composite_alloc(context->compositor->resources,
        capacity, sizeof(*items), 0);
    if (!items) return sr_composite_resource_status(context->compositor->resources);
    size_t count = 0;
    for (size_t i = 0; i < node->child_count;) {
        if (!node->children[i]->card) {
            items[count++] = (SrDrawItem){node->children[i], 0, 0.0, i};
            ++i;
            continue;
        }
        size_t first = count;
        for (; i < node->child_count && node->children[i]->card; ++i) {
            double key;
            SrStatus status = sr_card_key(context, node->children[i], world, &key);
            if (status != SR_OK) {
                sr_composite_free(context->compositor->resources, items);
                return status;
            }
            items[count++] = (SrDrawItem){node->children[i], 0, key, i};
        }
        if (merge) {
            for (size_t k = 0; k < scene->object3d_count; ++k) {
                if (!sr_composite_object_key_work(resources, scene, k)) {
                    sr_composite_free(resources, items);
                    return SR_ERR_RENDER;
                }
                double key = sr_lighting_object_depth(scene, k, context->time);
                items[count++] = (SrDrawItem){NULL, k,
                    isnan(key) ? INFINITY : key, node->child_count + k};
            }
            merge = false;
        }
        if (!sr_composite_sort_items(resources, items + first, count - first)) {
            sr_composite_free(resources, items);
            return SR_ERR_RENDER;
        }
    }
    SrStatus status = SR_OK;
    for (size_t i = 0; i < count && status == SR_OK; ++i) {
        if (items[i].node) {
            status = sr_draw_node(context, items[i].node, world, clip, target,
                                  depth, masks);
        } else {
            /* Consecutive objects share one resolve, so their samples keep
             * correct coverage against each other. The object writes the
             * frame and the depth buffer anywhere, so every queued card
             * draw runs first. */
            status = sr_queue_flush(context->compositor);
            if (status == SR_OK)
                status = sr_lighting_draw_object(context->lighting,
                                                 items[i].object, true);
            if (status == SR_OK && (i + 1 == count || items[i + 1].node))
                sr_lighting_flush(context->lighting, false);
        }
    }
    if (node == scene->root && context->lighting && count > 0) {
        for (size_t i = 0; i < count; ++i)
            if (!items[i].node) { context->lighting = NULL; break; }
    }
    sr_composite_free(context->compositor->resources, items);
    return status;
}

/* A group renders into an isolated buffer iff it has a non-normal blend,
 * opacity below one or group effects; the effects run on the buffer (its
 * dirty rectangle grown by each effect's reach) and the buffer is then
 * composited once with the group's blend, opacity and masks (plus any
 * masks inherited from pass-through ancestors). With effects, the
 * children are drawn unrestricted by the group's own masks (only by the
 * ancestors' clip grown by the effects' reach), so the effects see the
 * whole content before the masks cut it.
 * Otherwise the group is a pass-through: its children draw straight into the
 * parent target against the real backdrop, and its masks join the chain
 * applied to every child draw. */
static SrStatus sr_draw_group(SrDrawContext *context, const SrNode *node,
                              SrMat3 world, double opacity, SrClip clip,
                              const SrTarget *target, size_t depth,
                              const SrMaskLink *outer) {
    bool isolated = node->blend != SR_BLEND_NORMAL || opacity < 1.0 ||
                    node->effect_ref_count > 0;
    if (!isolated && node->mask_count == 0)
        return sr_draw_children(context, node, world, clip, target, depth,
                                outer);
    SrMat3 inverse;
    if (!sr_mat_inverse(world, &inverse)) return SR_OK;
    SrMaskEval local_masks[8];
    SrMaskEval *masks = node->mask_count <= 8 ? local_masks
        : sr_composite_alloc(context->compositor->resources, node->mask_count,
                               sizeof(*masks), 0);
    if (!masks) return sr_composite_resource_status(context->compositor->resources);
    if (!sr_masks_eval(context, node, masks)) {
        if (masks != local_masks)
            sr_composite_free(context->compositor->resources, masks);
        return SR_ERR_RENDER;
    }
    SrMaskLink link = {masks, node->mask_count, inverse,
                       sr_pixel_footprint(inverse), outer};
    const SrMaskLink *chain = node->mask_count ? &link : outer;
    SrClip inner = sr_mask_clip(clip, masks, node->mask_count, world, target);
    SrStatus status = SR_OK;
    if (!isolated) {
        status = sr_draw_children(context, node, world, inner, target, depth,
                                  chain);
    } else {
        SrGroupBuffer *buffer = NULL;
        status = sr_pool_get(context->compositor, depth, target->width,
                             target->height, &buffer);
        if (status == SR_OK) {
            SrTarget group = {buffer->frame.px, target->width, target->height,
                              buffer};
            /* Group effects read content beyond the group's own masks (a
             * shadow cast into the mask from outside it, blur taps), so
             * the children fill the ancestors' clip grown by the effects'
             * reach; the masks apply when the buffer is composited. */
            SrClip fill = inner;
            if (node->effect_ref_count) {
                double reach = 0.0;
                for (size_t i = 0; i < node->effect_ref_count; ++i)
                    reach += sr_effect_reach(node->effect_refs[i], context->time);
                int w = (int)target->width, h = (int)target->height;
                fill = (SrClip){sr_clamp_int(clip.x0 - reach, 0, w),
                                sr_clamp_int(clip.y0 - reach, 0, h),
                                sr_clamp_int(clip.x1 + reach, 0, w),
                                sr_clamp_int(clip.y1 + reach, 0, h)};
            }
            status = sr_draw_children(context, node, world, fill, &group,
                                      depth + 1, NULL);
        }
        if (status == SR_OK && node->effect_ref_count)
            status = sr_queue_flush(context->compositor);  /* effects read */
        if (status == SR_OK && node->effect_ref_count) {
            SrEffectRect rect = {buffer->x0, buffer->y0, buffer->x1, buffer->y1};
            status = sr_effects_apply_group(context->scene, node->effect_refs,
                                            node->effect_ref_count,
                                            context->time, &buffer->frame,
                                            world, &rect,
                                            context->compositor->threads);
            buffer->x0 = rect.x0; buffer->y0 = rect.y0;
            buffer->x1 = rect.x1; buffer->y1 = rect.y1;
        }
        if (status == SR_OK) {
            SrDrawOp op = {.kind = SR_OP_BUFFER, .target = *target,
                           .blend = node->blend, .opacity = (float)opacity,
                           .inverse = inverse, .aa = link.aa,
                           .masks = chain, .buffer = buffer};
            op.bounds = sr_clip_intersect(inner, (SrClip){buffer->x0,
                buffer->y0, buffer->x1, buffer->y1});
            status = sr_op_submit(context->compositor, &op, false);
        }
    }
    if (masks != local_masks)
        sr_composite_free(context->compositor->resources, masks);
    return status;
}

static SrStatus sr_draw_leaf(SrDrawContext *context, const SrNode *node,
                             SrMat3 world, double opacity, SrClip clip,
                             const SrTarget *target, const SrMaskLink *outer);
static SrStatus sr_draw_card(SrDrawContext *context, const SrNode *node,
                             SrMat3 parent, double opacity, SrClip clip,
                             const SrTarget *target, size_t depth,
                             const SrMaskLink *outer);

static SrStatus sr_draw_node_impl(SrDrawContext *context, const SrNode *node,
                                  SrMat3 parent, SrClip clip,
                                  const SrTarget *target, size_t depth,
                                  const SrMaskLink *outer) {
    SrStatus ready = sr_composite_node_ready(context->scene, node, context->diag);
    if (ready != SR_OK) return ready;
    double time = context->time;
    if (!node->visible || time < node->start_time || time >= node->end_time)
        return SR_OK;
    SrCompositeResources *resources = context->compositor->resources;
    if (resources) {
        SrCompositeOwner previous = sr_composite_owner(resources,
            sr_composite_node_owner(context->scene, node, "opacity"));
        bool valid = sr_composite_anim_work(resources, &node->opacity, false);
        sr_composite_owner(resources, previous);
        if (!valid) return SR_ERR_RENDER;
    }
    double opacity = sr_clamp(sr_anim_eval(&node->opacity, time), 0.0, 1.0);
    if (opacity <= 0.0) return SR_OK;
    SrDrawContext *caller = context;
    SrDrawContext local = *context;
    context = &local;
    SrMat3 world;
    SrStatus status = sr_node_world(context, node, parent, &world);
    if (status != SR_OK) return status;
    if (node->card && context->view)
        status = sr_draw_card(context, node, world, opacity, clip, target, depth,
                            outer);
    else if (node->type == SR_NODE_GROUP)
        status = sr_draw_group(context, node, world, opacity, clip, target, depth,
                             outer);
    else status = sr_draw_leaf(context, node, world, opacity, clip, target, outer);
    /* Root card runs consume the shared 3D pass. Preserve that handoff
     * across the stack copy used only to isolate the skew flag. */
    caller->lighting = local.lighting;
    return status;
}

static SrStatus sr_draw_node(SrDrawContext *context, const SrNode *node,
                             SrMat3 parent, SrClip clip,
                             const SrTarget *target, size_t depth,
                             const SrMaskLink *outer) {
    SrCompositeResources *resources = context->compositor->resources;
    if (!resources)
        return sr_draw_node_impl(context, node, parent, clip, target, depth, outer);
    const char *attribute = node->blend > SR_BLEND_DIFFERENCE ? "blend"
        : sr_node_uses_compositing(node) ? "skewX/skewY" : NULL;
    SrCompositeOwner previous = sr_composite_owner(resources,
        sr_composite_node_owner(context->scene, node, attribute));
    SrStatus status = SR_ERR_RENDER;
    if (sr_composite_work(resources, 1 + node->mask_count, 1))
        status = sr_draw_node_impl(context, node, parent, clip, target, depth, outer);
    sr_composite_owner(resources, previous);
    return status;
}

static SrStatus sr_draw_leaf(SrDrawContext *context, const SrNode *node,
                             SrMat3 world, double opacity, SrClip clip,
                             const SrTarget *target,
                             const SrMaskLink *outer) {
    SrMat3 inverse;
    if (!sr_mat_inverse(world, &inverse)) return SR_OK;
    SrMaskEval local_masks[8];
    SrMaskEval *masks = node->mask_count <= 8 ? local_masks
        : sr_composite_alloc(context->compositor->resources, node->mask_count,
                               sizeof(*masks), 0);
    if (!masks) return sr_composite_resource_status(context->compositor->resources);
    if (!sr_masks_eval(context, node, masks)) {
        if (masks != local_masks)
            sr_composite_free(context->compositor->resources, masks);
        return SR_ERR_RENDER;
    }
    SrMaskLink link = {masks, node->mask_count, inverse,
                       sr_pixel_footprint(inverse), outer};
    const SrMaskLink *chain = node->mask_count ? &link : outer;
    clip = sr_mask_clip(clip, masks, node->mask_count, world, target);
    SrStatus status = SR_OK;
    if (node->type == SR_NODE_MEDIA)
        status = sr_draw_image(context, node, world, inverse, opacity, clip,
                               target, chain);
    else if (node->type == SR_NODE_SHAPE)
        status = sr_draw_shape(context, node, world, inverse, opacity, clip,
                               target, chain);
    else if (node->type == SR_NODE_PARTICLES)
        status = sr_draw_particles(context, node, world, opacity, clip, target,
                                   chain);
    if (masks != local_masks)
        sr_composite_free(context->compositor->resources, masks);
    return status;
}

/* ---- depth cards --------------------------------------------------------- */

/* Draws a card's content with its own masks but not its blend, opacity or
 * effects (those apply when the card buffer is composited). */
static SrStatus sr_draw_content(SrDrawContext *context, const SrNode *node,
                                SrMat3 world, SrClip clip,
                                const SrTarget *target, size_t depth) {
    SrStatus valid = sr_skew_matrix_valid(context, node, world);
    if (valid != SR_OK) return valid;
    if (node->type != SR_NODE_GROUP)
        return sr_draw_leaf(context, node, world, 1.0, clip, target, NULL);
    SrMat3 inverse;
    if (!sr_mat_inverse(world, &inverse)) return SR_OK;
    SrMaskEval local_masks[8];
    SrMaskEval *masks = node->mask_count <= 8 ? local_masks
        : sr_composite_alloc(context->compositor->resources, node->mask_count,
                               sizeof(*masks), 0);
    if (!masks) return sr_composite_resource_status(context->compositor->resources);
    if (!sr_masks_eval(context, node, masks)) {
        if (masks != local_masks)
            sr_composite_free(context->compositor->resources, masks);
        return SR_ERR_RENDER;
    }
    SrMaskLink link = {masks, node->mask_count, inverse,
                       sr_pixel_footprint(inverse), NULL};
    SrClip inner = sr_mask_clip(clip, masks, node->mask_count, world, target);
    SrStatus status = sr_draw_children(context, node, world, inner, target,
                                       depth, node->mask_count ? &link : NULL);
    if (masks != local_masks)
        sr_composite_free(context->compositor->resources, masks);
    return status;
}

/* Conservative bounds, in the coordinates `matrix` maps to, of what `node`
 * can draw at `time`; false when unknown (particles, deformers). */
static bool sr_content_bounds(const SrDrawContext *context, const SrNode *node,
                              SrMat3 matrix, bool own, double box[4], bool *any,
                              SrStatus *status);

static bool sr_content_bounds_impl(const SrDrawContext *context, const SrNode *node,
                              SrMat3 matrix, bool own, double box[4], bool *any,
                              SrStatus *status) {
    double time = context->time;
    if (!node->visible || time < node->start_time || time >= node->end_time)
        return true;
    SrDrawContext local = *context;
    SrMat3 m = matrix;
    if (!own) {
        *status = sr_node_world(&local, node, matrix, &m);
        if (*status != SR_OK) return false;
    }
    if (node->type == SR_NODE_GROUP) {
        if (node->effect_ref_count && !own) return false;  /* effect reach */
        for (size_t i = 0; i < node->child_count; ++i)
            if (!sr_content_bounds(&local, node->children[i], m, false,
                                   box, any, status))
                return false;
        return true;
    }
    if (node->type == SR_NODE_PARTICLES || sr_node_deforms(node)) return false;
    double w, h, pad = 1.0;
    if (node->type == SR_NODE_MEDIA) {
        if (!node->asset) return true;
        w = node->asset->width;
        h = node->asset->height;
    } else {
        const SrNodeGeometry *geometry = sr_length_node(context->lengths, node);
        w = geometry ? geometry->box.width : node->shape_width;
        h = geometry ? geometry->box.height : node->shape_height;
        pad += node->stroke_width * 0.5;
    }
    SrVec2 corners[4] = {{-pad, -pad}, {w + pad, -pad}, {w + pad, h + pad},
                         {-pad, h + pad}};
    for (int i = 0; i < 4; ++i) {
        SrVec2 p = sr_mat_point(m, corners[i]);
        if (local.skewed && (!isfinite(p.x) || !isfinite(p.y))) {
            *status = sr_skew_error(context, node, "skewX/skewY",
                                    "skewed card bounds must be finite");
            return false;
        }
        if (!*any) {
            box[0] = box[2] = p.x;
            box[1] = box[3] = p.y;
            *any = true;
        }
        box[0] = fmin(box[0], p.x); box[1] = fmin(box[1], p.y);
        box[2] = fmax(box[2], p.x); box[3] = fmax(box[3], p.y);
    }
    return true;
}

static bool sr_content_bounds(const SrDrawContext *context, const SrNode *node,
                              SrMat3 matrix, bool own, double box[4], bool *any,
                              SrStatus *status) {
    SrCompositeResources *resources = context->compositor->resources;
    if (!resources) return sr_content_bounds_impl(context, node, matrix, own, box, any, status);
    SrCompositeOwner previous = sr_composite_owner(resources,
        sr_composite_node_owner(context->scene, node, "bounds"));
    bool result = false;
    /* Admission plus four corners, including invisible and unknown leaves. */
    if (!sr_composite_work(resources, 1, 64) ||
        !sr_composite_work(resources, node->child_count, 1)) *status = SR_ERR_RENDER;
    else result = sr_content_bounds_impl(context, node, matrix, own, box, any, status);
    sr_composite_owner(resources, previous);
    return result;
}

/* Checked appends also bound adversarial finite clipping state. */
static bool sr_clip_vertex(SrCompositeResources *resources, double *out,
                            size_t *count, double x, double y) {
    if (resources && (*count == SR_MAX_COMPOSITE_CLIP_VERTICES ||
        !isfinite(x) || !isfinite(y)))
        return sr_composite_resource_fail(resources, SR_ERR_RENDER,
            "invalid projective card clipping vertex/capacity");
    out[2 * *count] = x;
    out[2 * *count + 1] = y;
    ++*count;
    return true;
}

/* Clips convex polygon (u, v pairs) to a * u + b * v + c >= 0. */
static size_t sr_clip_polygon(const double *in, size_t count, double a,
                              double b, double c, double *out,
                              SrCompositeResources *resources) {
    if (resources && (count > SR_MAX_COMPOSITE_CLIP_VERTICES ||
        !isfinite(a) || !isfinite(b) || !isfinite(c))) {
        sr_composite_resource_fail(resources, SR_ERR_RENDER,
            "invalid projective card clipping plane");
        return 0;
    }
    size_t n = 0;
    for (size_t i = 0; i < count; ++i) {
        const double *p = in + 2 * i, *q = in + 2 * ((i + 1) % count);
        double dp = a * p[0] + b * p[1] + c, dq = a * q[0] + b * q[1] + c;
        if (resources && (!isfinite(dp) || !isfinite(dq))) {
            sr_composite_resource_fail(resources, SR_ERR_RENDER,
                "nonfinite projective card clipping distance");
            return 0;
        }
        if (dp >= 0.0 && !sr_clip_vertex(resources, out, &n, p[0], p[1])) return 0;
        if ((dp >= 0.0) != (dq >= 0.0)) {
            if (resources && !isfinite(dp - dq)) {
                sr_composite_resource_fail(resources, SR_ERR_RENDER,
                    "nonfinite projective card clipping intersection");
                return 0;
            }
            double t = dp / (dp - dq);
            double x = p[0] + (q[0] - p[0]) * t;
            double y = p[1] + (q[1] - p[1]) * t;
            if (!sr_clip_vertex(resources, out, &n, x, y)) return 0;
        }
    }
    return n;
}

/* Linear screen-space scale of the plane at (u, v): sqrt |det J|. */
static double sr_plane_magnification(const SrCardPose *pose, double u,
                                     double v) {
    const double (*m)[3] = pose->to_screen.m;
    double det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) -
                 m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) +
                 m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    double w = m[2][0] * u + m[2][1] * v + m[2][2];
    return sqrt(fabs(det) / fmax(1e-300, fabs(w * w * w)));
}

typedef struct {
    const SrCardPose *pose;
    const SrCardView *view;
    SrImage plane;              /* the rendered plane buffer */
    double s, u0, v0;           /* buffer = (plane - (u0, v0)) * s + 1 */
    const SrTarget *target;
    SrClip bounds;
} SrWarp;

static bool sr_warp_tap(const SrWarp *warp, double sx, double sy,
                        float acc[4]) {
    double u, v;
    if (!sr_card_to_plane(warp->pose, sx, sy, &u, &v)) return false;
    float texel[4];
    sr_image_sample(&warp->plane, (u - warp->u0) * warp->s + 1.0,
                    (v - warp->v0) * warp->s + 1.0, texel);
    for (int c = 0; c < 4; ++c) acc[c] += texel[c];
    return true;
}

/* Resamples the plane buffer onto the screen: bilinear taps, k x k of them
 * across the pixel when it covers more than 1.5 buffer pixels. */
static void sr_warp_rows(void *opaque, size_t begin, size_t end) {
    const SrWarp *warp = opaque;
    const SrTarget *target = warp->target;
    for (size_t row = begin; row < end; ++row) {
        int y = warp->bounds.y0 + (int)row;
        float *d = target->px + ((size_t)y * target->width +
                                 (size_t)warp->bounds.x0) * 4;
        for (int x = warp->bounds.x0; x < warp->bounds.x1; ++x, d += 4) {
            double cx = x + .5, cy = y + .5, z, u, v, ux, vx, uy, vy;
            if (!sr_card_depth_at(warp->view, warp->pose, cx, cy, &z) ||
                z < warp->view->near_plane || z > warp->view->far_plane ||
                !sr_card_to_plane(warp->pose, cx, cy, &u, &v))
                continue;
            int k = 1;
            if (sr_card_to_plane(warp->pose, cx + 1.0, cy, &ux, &vx) &&
                sr_card_to_plane(warp->pose, cx, cy + 1.0, &uy, &vy)) {
                double footprint = warp->s *
                    sqrt(fabs((ux - u) * (vy - v) - (vx - v) * (uy - u)));
                if (footprint > 1.5) k = footprint >= 4.0 ? 4 : (int)ceil(footprint);
            }
            float acc[4] = {0, 0, 0, 0};
            for (int j = 0; j < k; ++j) for (int i = 0; i < k; ++i)
                sr_warp_tap(warp, x + (i + .5) / k, y + (j + .5) / k, acc);
            float norm = 1.0f / (float)(k * k);
            for (int c = 0; c < 4; ++c) d[c] = acc[c] * norm;
        }
    }
}

/* The plane-to-screen affine that matches the homography at (u, v). */
static SrMat3 sr_plane_tangent(const SrCardPose *pose, double u, double v) {
    double x, y, xu, yu, xv, yv;
    if (!sr_card_to_screen(pose, u, v, &x, &y) ||
        !sr_card_to_screen(pose, u + 1.0, v, &xu, &yu) ||
        !sr_card_to_screen(pose, u, v + 1.0, &xv, &yv))
        return sr_mat_identity();
    SrMat3 m = {xu - x, xv - x, 0, yu - y, yv - y, 0};
    SrVec2 p = sr_mat_point(m, (SrVec2){u, v});
    m.m02 = x - p.x;
    m.m12 = y - p.y;
    return m;
}

#define SR_PLANE_MAX_SIDE 4096.0
#define SR_PLANE_MAX_PIXELS 8.0e6
#define SR_PLANE_QUANTUM 256u

/* Projective card: render the visible part of its plane into a plane
 * buffer, then warp that onto the (cleared, screen-sized) card buffer. */
static SrStatus sr_card_projective(SrDrawContext *context, const SrNode *node,
                                   SrMat3 plane, const SrCardPose *pose,
                                   const SrTarget *card, SrClip clip) {
    const SrCardView *view = context->view;
    SrCompositeResources *resources = context->compositor->resources;
    /* Six clips of at most 12 vertices, bounded appends/intersections,
     * 12 final projections and 16 size attempts; pose work is separate. */
    if (!sr_composite_work(resources, 1, SR_COMPOSITE_PROJECTIVE_WORK)) return SR_ERR_RENDER;
    const double (*h)[3] = pose->to_screen.m;
    /* Visible plane region: content bounds (or a huge square) clipped to
     * the screen edges and the near/far planes, all half-planes in (u, v). */
    double box[4];
    bool any = false;
    SrStatus bounds_status = SR_OK;
    bool bounded = sr_content_bounds(context, node, plane, true, box, &any,
                                     &bounds_status);
    if (bounds_status != SR_OK) return bounds_status;
    if (bounded && !any) return SR_OK;
    if (!bounded) { box[0] = box[1] = -1e9; box[2] = box[3] = 1e9; }
    if (resources) {
        for (size_t i = 0; i < 4; ++i) {
            if (!isfinite(box[i])) {
                sr_composite_resource_fail(resources, SR_ERR_RENDER,
                    "nonfinite projective card content bounds");
                return SR_ERR_RENDER;
            }
        }
    }
    double a[2 * SR_MAX_COMPOSITE_CLIP_VERTICES];
    double b[2 * SR_MAX_COMPOSITE_CLIP_VERTICES];
    double *poly = a, *next = b;
    size_t count = 4;
    double start[8] = {box[0], box[1], box[2], box[1], box[2], box[3], box[0], box[3]};
    memcpy(poly, start, sizeof(start));
    double cons[6][3];
    size_t ncons = 0;
    double width = view->width, height = view->height;
    if (!view->orthographic) {
        double near = view->near_plane > 0.0 ? view->near_plane : 1e-9;
        cons[ncons][0] = h[2][0]; cons[ncons][1] = h[2][1];
        cons[ncons++][2] = h[2][2] - near;
        cons[ncons][0] = -h[2][0]; cons[ncons][1] = -h[2][1];
        cons[ncons++][2] = view->far_plane - h[2][2];
    }
    for (int axis = 0; axis < 2; ++axis) {
        double extent = axis ? height : width;
        const double *r = h[axis];
        cons[ncons][0] = r[0]; cons[ncons][1] = r[1]; cons[ncons++][2] = r[2];
        cons[ncons][0] = extent * h[2][0] - r[0];
        cons[ncons][1] = extent * h[2][1] - r[1];
        cons[ncons++][2] = extent * h[2][2] - r[2];
    }
    for (size_t i = 0; i < ncons && count >= 3; ++i) {
        count = sr_clip_polygon(poly, count, cons[i][0], cons[i][1],
                                cons[i][2], next, resources);
        if (resources && resources->status != SR_OK) return SR_ERR_RENDER;
        double *swap = poly; poly = next; next = swap;
    }
    if (count < 3) return SR_OK;
    double u0 = poly[0], v0 = poly[1], u1 = poly[0], v1 = poly[1];
    double s = 0.0;
    SrClip screen = {INT_MAX, INT_MAX, INT_MIN, INT_MIN};
    for (size_t i = 0; i < count; ++i) {
        double u = poly[2 * i], v = poly[2 * i + 1], x, y;
        u0 = fmin(u0, u); v0 = fmin(v0, v); u1 = fmax(u1, u); v1 = fmax(v1, v);
        double magnification = sr_plane_magnification(pose, u, v);
        if (resources && (!isfinite(u) || !isfinite(v) || !isfinite(magnification))) {
            sr_composite_resource_fail(resources, SR_ERR_RENDER,
                "nonfinite projective card scale");
            return SR_ERR_RENDER;
        }
        s = fmax(s, magnification);
        if (sr_card_to_screen(pose, u, v, &x, &y)) {
            if (resources) {
                if (!isfinite(x) || !isfinite(y)) {
                    sr_composite_resource_fail(resources, SR_ERR_RENDER,
                        "nonfinite projective card screen coordinate");
                    return SR_ERR_RENDER;
                }
                screen.x0 = sr_clamp_int(fmin(screen.x0, floor(x) - 1), 0, card->width);
                screen.y0 = sr_clamp_int(fmin(screen.y0, floor(y) - 1), 0, card->height);
                screen.x1 = sr_clamp_int(fmax(screen.x1, ceil(x) + 1), 0, card->width);
                screen.y1 = sr_clamp_int(fmax(screen.y1, ceil(y) + 1), 0, card->height);
            } else {
                screen.x0 = (int)fmin(screen.x0, floor(x) - 1);
                screen.y0 = (int)fmin(screen.y0, floor(y) - 1);
                screen.x1 = (int)fmax(screen.x1, ceil(x) + 1);
                screen.y1 = (int)fmax(screen.y1, ceil(y) + 1);
            }
        }
    }
    screen = sr_clip_intersect(screen, clip);
    if (screen.x1 <= screen.x0 || screen.y1 <= screen.y0) return SR_OK;
    double pw = fmax(u1 - u0, 1e-6), ph = fmax(v1 - v0, 1e-6);
    if (resources && (!isfinite(pw) || !isfinite(ph) || !isfinite(pw * ph))) {
        sr_composite_resource_fail(resources, SR_ERR_RENDER,
            "nonfinite projective card plane extent/area");
        return SR_ERR_RENDER;
    }
    s = fmin(s, fmin((SR_PLANE_MAX_SIDE - 2.0) / pw, (SR_PLANE_MAX_SIDE - 2.0) / ph));
    s = fmin(s, sqrt(SR_PLANE_MAX_PIXELS / (pw * ph)));
    if (!(s > 0.0) || !isfinite(s)) return SR_OK;
    uint32_t bw = 0, bh = 0;
    /* Quantized sizes round up; shrink s until they fit the caps too. */
    for (int attempt = 0; attempt < 16; ++attempt) {
        if (resources && (!isfinite(pw * s) || !isfinite(ph * s) ||
            ceil(pw * s) > UINT32_MAX - 2u - (SR_PLANE_QUANTUM - 1u) ||
            ceil(ph * s) > UINT32_MAX - 2u - (SR_PLANE_QUANTUM - 1u))) {
            sr_composite_resource_fail(resources, SR_ERR_RENDER,
                "projective card plane dimension overflow");
            return SR_ERR_RENDER;
        }
        bw = (uint32_t)ceil(pw * s) + 2;
        bh = (uint32_t)ceil(ph * s) + 2;
        bw = (bw + SR_PLANE_QUANTUM - 1) / SR_PLANE_QUANTUM * SR_PLANE_QUANTUM;
        bh = (bh + SR_PLANE_QUANTUM - 1) / SR_PLANE_QUANTUM * SR_PLANE_QUANTUM;
        if ((double)bw * bh <= SR_PLANE_MAX_PIXELS && bw <= SR_PLANE_MAX_SIDE &&
            bh <= SR_PLANE_MAX_SIDE)
            break;
        s *= 0.9;
    }
    if ((double)bw * bh > SR_PLANE_MAX_PIXELS || bw > SR_PLANE_MAX_SIDE ||
        bh > SR_PLANE_MAX_SIDE)
        return SR_OK;
    SrCompositor *pool = context->compositor->plane;
    if (!pool) {
        pool = context->compositor->plane = sr_composite_alloc(
            context->compositor->resources, 1, sizeof(*pool), 0);
        if (!pool)
            return sr_composite_resource_status(context->compositor->resources);
        sr_compositor_init(pool, context->compositor->threads);
        pool->resources = context->compositor->resources;
    }
    SrGroupBuffer *buffer = NULL;
    SrStatus status = sr_pool_get(pool, 0, bw, bh, &buffer);
    if (status != SR_OK) return status;
    SrTarget target = {buffer->frame.px, bw, bh, buffer};
    SrMat3 to_buffer = sr_mat_multiply(
        sr_mat_translate(1.0, 1.0),
        sr_mat_multiply(sr_mat_scale(s, s), sr_mat_translate(-u0, -v0)));
    SrDrawContext inner = *context;
    inner.compositor = pool;
    inner.card_node = node;
    inner.particle_scale = context->particle_scale * s;
    status = sr_draw_content(&inner, node, sr_mat_multiply(to_buffer, plane),
                             (SrClip){0, 0, (int)bw, (int)bh}, &target, 1);
    /* The warp reads the plane buffer and writes the card buffer directly. */
    if (status == SR_OK) status = sr_queue_flush(pool);
    if (status == SR_OK) status = sr_queue_flush(context->compositor);
    if (status != SR_OK) return status;
    SrWarp warp = {pose, view, {bw, bh, buffer->frame.px}, s, u0, v0, card,
                   screen};
    /* At most 16 taps: homography + clear + four RGBA texels + accumulation,
     * plus four outer queries, loop dispatch and final channel stores. */
    if (!sr_composite_work(context->compositor->resources,
        (uint64_t)(screen.x1 - screen.x0) * (uint64_t)(screen.y1 - screen.y0),
        SR_COMPOSITE_WARP_PIXEL_WORK)) return SR_ERR_RENDER;
    sr_buffer_mark(card->buffer, screen);
    return sr_parallel_for((size_t)(screen.y1 - screen.y0),
                           sr_op_threads(context->compositor, screen),
                           sr_warp_rows, &warp);
}

/* A depth card: its subtree renders as one unit into an isolated buffer
 * placed by the camera (directly with the projected affine when the
 * mapping is affine, else through a plane buffer and a perspective warp),
 * then gets its effects and depth-of-field blur in screen space and is
 * composited with its blend, opacity and the pass-through mask chain,
 * depth tested per sample against the shared depth buffer. */
static SrStatus sr_draw_card(SrDrawContext *context, const SrNode *node,
                             SrMat3 plane, double opacity, SrClip clip,
                             const SrTarget *target, size_t depth,
                             const SrMaskLink *outer) {
    const SrCardView *view = context->view;
    double time = context->time;
    const SrNodeGeometry *geometry = sr_length_node(context->lengths, node);
    if (!sr_composite_pivot_work(context->compositor->resources, context->scene,
                                  node, geometry != NULL, true)) return SR_ERR_RENDER;
    SrVec2 pivot = sr_mat_point(plane, (SrVec2){
        geometry ? geometry->anchor_x : sr_anim_eval(&node->transform.anchor_x, time),
        geometry ? geometry->anchor_y : sr_anim_eval(&node->transform.anchor_y, time)});
    SrCardPose pose = sr_card_pose(view, pivot.x, pivot.y,
                                   sr_anim_eval(&node->transform.z, time),
                                   sr_anim_eval(&node->transform.rotation_x, time),
                                   sr_anim_eval(&node->transform.rotation_y, time));
    if (context->skewed) {
        bool finite = isfinite(pose.pivot_depth) && isfinite(pose.offset);
        for (size_t r = 0; r < 3; ++r) {
            finite &= isfinite(pose.normal[r]);
            for (size_t c = 0; c < 3; ++c)
                finite &= isfinite(pose.to_screen.m[r][c]) &&
                          isfinite(pose.to_plane.m[r][c]);
        }
        if (!finite || !pose.invertible)
            return sr_skew_error(context, node, "skewX/skewY",
                                 "skewed card projection must be finite and invertible");
    }
    if (!pose.invertible) return SR_OK;
    bool flat = pose.normal[0] == 0.0 && pose.normal[1] == 0.0;
    if (flat && (pose.pivot_depth < view->near_plane ||
                 pose.pivot_depth > view->far_plane))
        return SR_OK;
    SrGroupBuffer *buffer = NULL;
    SrStatus status = sr_pool_get(context->compositor, depth, target->width,
                                  target->height, &buffer);
    if (status != SR_OK) return status;
    SrTarget card = {buffer->frame.px, target->width, target->height, buffer};
    SrMat3 to_canvas;
    if (pose.is_affine) {
        to_canvas = sr_mat_multiply(pose.affine, plane);
        SrDrawContext inner = *context;
        inner.card_node = node;
        inner.particle_scale = context->particle_scale *
            sqrt(fabs(pose.affine.m00 * pose.affine.m11 -
                      pose.affine.m01 * pose.affine.m10));
        status = sr_draw_content(&inner, node, to_canvas, clip, &card,
                                 depth + 1);
    } else {
        to_canvas = sr_mat_multiply(sr_plane_tangent(&pose, pivot.x, pivot.y),
                                    plane);
        status = sr_card_projective(context, node, plane, &pose, &card, clip);
    }
    /* The effects and the blur read and write the card buffer in place. */
    double blur = view->camera ? sr_card_blur_radius(view, pose.pivot_depth) : 0.0;
    if (status == SR_OK && (node->effect_ref_count || blur > 1e-3))
        status = sr_queue_flush(context->compositor);
    if (status != SR_OK) return status;
    SrEffectRect rect = {buffer->x0, buffer->y0, buffer->x1, buffer->y1};
    if (node->effect_ref_count)
        status = sr_effects_apply_group(context->scene, node->effect_refs,
                                        node->effect_ref_count, time,
                                        &buffer->frame, to_canvas, &rect,
                                        context->compositor->threads);
    if (status == SR_OK && blur > 1e-3 && rect.x1 > rect.x0 && rect.y1 > rect.y0)
        status = sr_effects_blur_rect(&buffer->frame, &rect, blur,
                                      context->compositor->threads);
    if (status != SR_OK) return status;
    buffer->x0 = rect.x0; buffer->y0 = rect.y0;
    buffer->x1 = rect.x1; buffer->y1 = rect.y1;
    SrDrawOp op = {.kind = SR_OP_BUFFER, .target = *target, .blend = node->blend,
                   .opacity = (float)opacity, .inverse = sr_mat_identity(),
                   .aa = 1.0, .masks = outer, .buffer = buffer,
                   .has_card = true,
                   .card = {context->compositor->depth, view, pose,
                            !target->buffer}};
    op.bounds = sr_clip_intersect(clip, (SrClip){buffer->x0, buffer->y0,
                                                 buffer->x1, buffer->y1});
    return sr_op_submit(context->compositor, &op, false);
}

static SrStatus sr_prepare_lengths(SrCompositor *compositor, const SrScene *scene,
                                    double time, SrDiagnostics *diag) {
    if (!scene->has_relative_lengths) return SR_OK;
    if (!compositor->lengths) {
        compositor->lengths = sr_composite_alloc(compositor->resources, 1,
                                                 sizeof(*compositor->lengths), 0);
        if (!compositor->lengths) {
            if (compositor->resources)
                return sr_composite_resource_status(compositor->resources);
            if (diag)
                sr_diag_error(diag, 0, "composition", NULL,
                              "out of memory allocating evaluated lengths");
            return SR_ERR_MEMORY;
        }
        compositor->lengths->resources = compositor->resources;
    }
    return sr_length_frame_prepare(compositor->lengths, scene, time, false, diag);
}

static SrStatus sr_compositor_draw(SrCompositor *compositor, SrScene *scene,
                                    double time, SrFrame *frame, SrDiagnostics *diag) {
    SrTarget target = {frame->px, frame->width, frame->height, NULL};
    SrClip clip = {0, 0, (int)frame->width, (int)frame->height};
    size_t errors = diag ? diag->errors : 0;
    SrCardView view = sr_card_view(scene, time);
    SrDrawContext context = {compositor, scene, diag, time, &view, NULL, 1.0,
                             NULL, scene->has_relative_lengths ? compositor->lengths : NULL,
                             false};
    SrStatus status = sr_draw_node(&context, scene->root, sr_mat_identity(),
                                   clip, &target, 0, NULL);
    /* Run whatever is still queued; on failure it is discarded unrun. */
    if (status == SR_OK) status = sr_queue_flush(compositor);
    if (status != SR_OK) {
        sr_queue_discard(compositor);
        return status;
    }
    return diag && diag->errors > errors ? SR_ERR_ASSET : SR_OK;
}

static SrStatus sr_compositor_render_impl(SrCompositor *compositor, SrScene *scene,
                              double time, SrFrame *frame, SrDiagnostics *diag) {
    if (!compositor || !scene || !scene->root || !frame || !frame->px)
        return SR_ERR_ARGUMENT;
    SrStatus ready = sr_composite_scene_ready(scene, diag);
    if (ready != SR_OK) return ready;
    if (!sr_composite_view_work(compositor->resources, scene, time)) return SR_ERR_RENDER;
    SrStatus status = sr_prepare_lengths(compositor, scene, time, diag);
    return status == SR_OK ? sr_compositor_draw(compositor, scene, time, frame, diag)
                          : status;
}

static bool sr_root_has_cards(const SrScene *scene) {
    for (size_t i = 0; scene->root && i < scene->root->child_count; ++i)
        if (scene->root->children[i]->card) return true;
    return false;
}

static SrStatus sr_compositor_render_scene_impl(SrCompositor *compositor, SrScene *scene,
                                    double time, SrFrame *frame,
                                    SrDiagnostics *diag) {
    if (!compositor || !scene || !scene->root || !frame || !frame->px)
        return SR_ERR_ARGUMENT;
    SrStatus ready = sr_composite_scene_ready(scene, diag);
    if (ready != SR_OK) return ready;
    if (!sr_composite_view_work(compositor->resources, scene, time)) return SR_ERR_RENDER;
    SrStatus prepared = sr_prepare_lengths(compositor, scene, time, diag);
    if (prepared != SR_OK) return prepared;
    if (!scene->has_cards) {
        SrStatus status = sr_lighting_render_threads(scene, time, frame,
                                                     compositor->threads, diag);
        return status == SR_OK
            ? sr_compositor_draw(compositor, scene, time, frame, diag) : status;
    }
    int n = sr_lighting_samples(scene);
    SrDepthBuffer *depth = &compositor->depth_store;
    uint32_t dw = frame->width * (uint32_t)n, dh = frame->height * (uint32_t)n;
    if (!depth->z || depth->width != dw || depth->height != dh ||
        depth->samples != n) {
        sr_composite_free(compositor->resources, depth->z);
        *depth = (SrDepthBuffer){sr_composite_alloc(compositor->resources,
            (size_t)dw * dh, sizeof(double), (uint64_t)dw * dh * 2), dw, dh, n};
        if (!depth->z) {
            if (!compositor->resources)
                sr_diag_error(diag, 0, NULL, NULL, "cannot allocate the depth buffer");
            return sr_composite_resource_status(compositor->resources);
        }
    }
    if (!sr_composite_work(compositor->resources, (uint64_t)dw * dh, 2))
        return SR_ERR_RENDER;
    for (size_t i = 0, count = (size_t)dw * dh; i < count; ++i)
        depth->z[i] = INFINITY;
    SrLightingPass *pass = NULL;
    SrStatus status = sr_lighting_begin(scene, time, frame, depth,
                                        compositor->threads, diag, &pass);
    if (status != SR_OK) return status;
    if (pass && !sr_composite_work(compositor->resources, scene->root->child_count, 1)) {
        sr_lighting_end(pass);
        return SR_ERR_RENDER;
    }
    if (pass && !sr_root_has_cards(scene)) {
        sr_lighting_draw_blobs(pass);
        for (size_t i = 0; i < scene->object3d_count && status == SR_OK; ++i)
            status = sr_lighting_draw_object(pass, i, false);
        if (status == SR_OK) sr_lighting_flush(pass, true);
        sr_lighting_end(pass);
        pass = NULL;
        if (status != SR_OK) return status;
    }
    compositor->depth = depth;
    SrTarget target = {frame->px, frame->width, frame->height, NULL};
    SrClip clip = {0, 0, (int)frame->width, (int)frame->height};
    size_t errors = diag ? diag->errors : 0;
    SrCardView view = sr_card_view(scene, time);
    SrDrawContext context = {compositor, scene, diag, time, &view, NULL, 1.0,
                             pass, scene->has_relative_lengths ? compositor->lengths : NULL,
                             false};
    status = sr_draw_node(&context, scene->root, sr_mat_identity(), clip,
                          &target, 0, NULL);
    /* Queued card composites test against the depth buffer: they run
     * before any remaining 3D object; on failure they are discarded. */
    if (status == SR_OK) status = sr_queue_flush(compositor);
    if (status != SR_OK) sr_queue_discard(compositor);
    compositor->depth = NULL;
    /* A root card run that was not visible still owes the 3D objects. */
    if (status == SR_OK && context.lighting) {
        for (size_t i = 0; i < scene->object3d_count && status == SR_OK; ++i)
            status = sr_lighting_draw_object(pass, i, true);
        if (status == SR_OK) sr_lighting_flush(pass, false);
    }
    sr_lighting_end(pass);
    if (status != SR_OK) return status;
    return diag && diag->errors > errors ? SR_ERR_ASSET : SR_OK;
}

static SrStatus sr_render_scoped(SrCompositor *compositor, SrScene *scene,
                                 double time, SrFrame *frame,
                                 SrDiagnostics *diag, bool lighting) {
    if (!compositor || !scene || !scene->root || !frame || !frame->px)
        return SR_ERR_ARGUMENT;
    SrStatus status = sr_composite_scene_ready(scene, diag);
    if (status != SR_OK) return status;
    SrCompositeScope scope;
    status = sr_composite_scope_begin(&scope, compositor, scene, frame, NULL);
    if (status == SR_OK)
        status = lighting
            ? sr_compositor_render_scene_impl(compositor, scene, time, frame, diag)
            : sr_compositor_render_impl(compositor, scene, time, frame, diag);
    return sr_composite_scope_end(&scope, compositor, status, diag);
}

SrStatus sr_compositor_render(SrCompositor *compositor, SrScene *scene,
                              double time, SrFrame *frame, SrDiagnostics *diag) {
    return sr_render_scoped(compositor, scene, time, frame, diag, false);
}

SrStatus sr_compositor_render_scene(SrCompositor *compositor, SrScene *scene,
                                    double time, SrFrame *frame,
                                    SrDiagnostics *diag) {
    return sr_render_scoped(compositor, scene, time, frame, diag, true);
}

SrStatus sr_composite_scene(SrScene *scene, double time, SrFrame *frame,
                            SrDiagnostics *diag) {
    SrCompositor compositor;
    sr_compositor_init(&compositor, 1);
    SrStatus status = sr_compositor_render(&compositor, scene, time, frame, diag);
    sr_compositor_free(&compositor);
    return status;
}
