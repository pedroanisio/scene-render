#include "scene_render/compositor.h"
#include "scene_render/assets.h"
#include "scene_render/physics.h"

#include <float.h>
#include <math.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    int x0, y0, x1, y1;
} SrClip;

static double sr_clamp(double value, double low, double high) {
    return value < low ? low : value > high ? high : value;
}

static uint8_t sr_byte(double value) {
    return (uint8_t)lrint(sr_clamp(value, 0.0, 1.0) * 255.0);
}

static double sr_to_linear(double value) {
    return value <= 0.04045 ? value / 12.92
                            : pow((value + 0.055) / 1.055, 2.4);
}

static double sr_from_linear(double value) {
    value = sr_clamp(value, 0.0, 1.0);
    return value <= 0.0031308 ? value * 12.92
                              : 1.055 * pow(value, 1.0 / 2.4) - 0.055;
}

static double sr_blend_channel(double backdrop, double source,
                               SrBlendMode mode) {
    switch (mode) {
    case SR_BLEND_ADD:
        return fmin(1.0, backdrop + source);
    case SR_BLEND_MULTIPLY:
        return backdrop * source;
    case SR_BLEND_SCREEN:
        return backdrop + source - backdrop * source;
    case SR_BLEND_OVERLAY:
        return backdrop <= 0.5 ? 2.0 * backdrop * source
                               : 1.0 - 2.0 * (1.0 - backdrop) * (1.0 - source);
    case SR_BLEND_DIFFERENCE:
        return fabs(backdrop - source);
    case SR_BLEND_NORMAL:
    default:
        return source;
    }
}

SrColor sr_blend_pixel(SrColor backdrop, SrColor source, double opacity,
                       SrBlendMode mode, bool linear_light) {
    double ab = sr_clamp(backdrop.a, 0.0, 1.0);
    double as = sr_clamp(source.a * opacity, 0.0, 1.0);
    double ao = as + ab * (1.0 - as);
    SrColor out = {0.0, 0.0, 0.0, ao};
    if (ao <= 0.0) {
        return out;
    }
    double cb[3] = {backdrop.r, backdrop.g, backdrop.b};
    double cs[3] = {source.r, source.g, source.b};
    double *co[3] = {&out.r, &out.g, &out.b};
    for (size_t i = 0; i < 3; ++i) {
        if (linear_light) {
            cb[i] = sr_to_linear(sr_clamp(cb[i], 0.0, 1.0));
            cs[i] = sr_to_linear(sr_clamp(cs[i], 0.0, 1.0));
        }
        double blended = sr_blend_channel(cb[i], cs[i], mode);
        double premultiplied = as * (1.0 - ab) * cs[i] +
                               ab * (1.0 - as) * cb[i] + as * ab * blended;
        double straight = premultiplied / ao;
        *co[i] = linear_light ? sr_from_linear(straight) : straight;
    }
    return out;
}

SrStatus sr_frame_init(SrFrame *frame, uint32_t width, uint32_t height) {
    if (!frame || width == 0 || height == 0) {
        return SR_ERR_ARGUMENT;
    }
    size_t pixels = (size_t)width * height;
    if (pixels / height != width || pixels > SIZE_MAX / 4) {
        return SR_ERR_MEMORY;
    }
    frame->rgba = sr_alloc(pixels * 4);
    if (!frame->rgba) {
        return SR_ERR_MEMORY;
    }
    frame->width = width;
    frame->height = height;
    return SR_OK;
}

void sr_frame_free(SrFrame *frame) {
    if (frame) {
        free(frame->rgba);
        *frame = (SrFrame){0};
    }
}

void sr_frame_clear(SrFrame *frame, SrColor color) {
    uint8_t values[4] = {sr_byte(color.r), sr_byte(color.g), sr_byte(color.b),
                         sr_byte(color.a)};
    size_t pixels = (size_t)frame->width * frame->height;
    for (size_t i = 0; i < pixels; ++i) {
        frame->rgba[i * 4] = values[0];
        frame->rgba[i * 4 + 1] = values[1];
        frame->rgba[i * 4 + 2] = values[2];
        frame->rgba[i * 4 + 3] = values[3];
    }
}

static SrColor sr_image_sample(const SrImage *image, double x, double y) {
    x = sr_clamp(x - 0.5, 0.0, (double)image->width - 1.0);
    y = sr_clamp(y - 0.5, 0.0, (double)image->height - 1.0);
    uint32_t x0 = (uint32_t)floor(x);
    uint32_t y0 = (uint32_t)floor(y);
    uint32_t x1 = x0 + 1 < image->width ? x0 + 1 : x0;
    uint32_t y1 = y0 + 1 < image->height ? y0 + 1 : y0;
    double tx = x - x0;
    double ty = y - y0;
    SrColor result = {0};
    double *channels[4] = {&result.r, &result.g, &result.b, &result.a};
    for (size_t c = 0; c < 4; ++c) {
        double a = image->rgba[((size_t)y0 * image->width + x0) * 4 + c] / 255.0;
        double b = image->rgba[((size_t)y0 * image->width + x1) * 4 + c] / 255.0;
        double d = image->rgba[((size_t)y1 * image->width + x0) * 4 + c] / 255.0;
        double e = image->rgba[((size_t)y1 * image->width + x1) * 4 + c] / 255.0;
        *channels[c] = (a + (b - a) * tx) * (1.0 - ty) +
                       (d + (e - d) * tx) * ty;
    }
    return result;
}

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

static SrClip sr_clip_intersect(SrClip a, SrClip b) {
    return (SrClip){a.x0 > b.x0 ? a.x0 : b.x0,
                    a.y0 > b.y0 ? a.y0 : b.y0,
                    a.x1 < b.x1 ? a.x1 : b.x1,
                    a.y1 < b.y1 ? a.y1 : b.y1};
}

static SrClip sr_bounds(SrMat3 matrix, double width, double height,
                        const SrFrame *frame) {
    SrVec2 points[4] = {{0.0, 0.0}, {width, 0.0},
                        {width, height}, {0.0, height}};
    double min_x = DBL_MAX, min_y = DBL_MAX;
    double max_x = -DBL_MAX, max_y = -DBL_MAX;
    for (size_t i = 0; i < 4; ++i) {
        SrVec2 p = sr_mat_point(matrix, points[i]);
        min_x = fmin(min_x, p.x);
        min_y = fmin(min_y, p.y);
        max_x = fmax(max_x, p.x);
        max_y = fmax(max_y, p.y);
    }
    return (SrClip){(int)fmax(0.0, floor(min_x)),
                    (int)fmax(0.0, floor(min_y)),
                    (int)fmin((double)frame->width, ceil(max_x)),
                    (int)fmin((double)frame->height, ceil(max_y))};
}

static bool sr_in_mask(const SrNode *node, SrVec2 local) {
    if (!node->mask.enabled) {
        return true;
    }
    bool inside = local.x >= node->mask.x && local.y >= node->mask.y &&
                  local.x < node->mask.x + node->mask.width &&
                  local.y < node->mask.y + node->mask.height;
    return node->mask.invert ? !inside : inside;
}

static void sr_put_pixel(const SrScene *scene, const SrNode *node,
                         SrFrame *frame, int x, int y, SrColor source,
                         double opacity) {
    if (x < 0 || y < 0 || x >= (int)frame->width || y >= (int)frame->height)
        return;
    size_t offset = ((size_t)y * frame->width + (size_t)x) * 4;
    SrColor backdrop = {frame->rgba[offset] / 255.0,
                        frame->rgba[offset + 1] / 255.0,
                        frame->rgba[offset + 2] / 255.0,
                        frame->rgba[offset + 3] / 255.0};
    SrColor out = sr_blend_pixel(backdrop, source, opacity, node->blend,
                                 scene->project.linear_light);
    frame->rgba[offset] = sr_byte(out.r);
    frame->rgba[offset + 1] = sr_byte(out.g);
    frame->rgba[offset + 2] = sr_byte(out.b);
    frame->rgba[offset + 3] = sr_byte(out.a);
}

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

static void sr_draw_image(SrScene *scene, const SrNode *node,
                          SrMat3 world, double opacity, SrClip clip,
                          double time, SrFrame *frame, SrDiagnostics *diag) {
    const SrImage *image = sr_asset_get_frame(scene, node->asset,
                                               sr_media_time(node, time), diag);
    if (!image) {
        return;
    }
    SrMat3 inverse;
    if (!sr_mat_inverse(world, &inverse)) {
        return;
    }
    SrClip bounds = sr_bounds(world, image->width, image->height, frame);
    bounds = sr_clip_intersect(bounds, clip);
    for (int y = bounds.y0; y < bounds.y1; ++y) {
        for (int x = bounds.x0; x < bounds.x1; ++x) {
            SrVec2 local = sr_mat_point(inverse, (SrVec2){x + 0.5, y + 0.5});
            local = sr_deform_inverse(node, local, image->width, image->height,
                                      time);
            if (local.x < 0.0 || local.y < 0.0 ||
                local.x >= image->width || local.y >= image->height ||
                !sr_in_mask(node, local)) {
                continue;
            }
            SrColor source = sr_image_sample(image, local.x, local.y);
            sr_put_pixel(scene, node, frame, x, y, source, opacity);
        }
    }
}

static void sr_draw_shape(const SrScene *scene, const SrNode *node,
                          SrMat3 world, double opacity, SrClip clip,
                          double time, SrFrame *frame) {
    SrMat3 inverse;
    if (!sr_mat_inverse(world, &inverse)) return;
    SrClip bounds = sr_clip_intersect(
        sr_bounds(world, node->shape_width, node->shape_height, frame), clip);
    double half_stroke = node->stroke_width * 0.5;
    for (int y = bounds.y0; y < bounds.y1; ++y) {
        for (int x = bounds.x0; x < bounds.x1; ++x) {
            SrVec2 p = sr_mat_point(inverse, (SrVec2){x + 0.5, y + 0.5});
            p = sr_deform_inverse(node, p, node->shape_width,
                                  node->shape_height, time);
            if (!sr_in_mask(node, p)) continue;
            bool inside = p.x >= 0.0 && p.y >= 0.0 &&
                          p.x < node->shape_width && p.y < node->shape_height;
            bool edge = inside && (p.x < node->stroke_width ||
                p.y < node->stroke_width ||
                p.x >= node->shape_width - node->stroke_width ||
                p.y >= node->shape_height - node->stroke_width);
            if (node->shape == SR_SHAPE_ELLIPSE) {
                double nx = (p.x - node->shape_width * 0.5) /
                            (node->shape_width * 0.5);
                double ny = (p.y - node->shape_height * 0.5) /
                            (node->shape_height * 0.5);
                double radius = sqrt(nx * nx + ny * ny);
                inside = radius <= 1.0;
                double edge_width = fmax(half_stroke /
                    fmax(node->shape_width, node->shape_height), 0.0);
                edge = inside && radius >= 1.0 - edge_width * 4.0;
            }
            if (!inside) continue;
            SrColor color = edge && node->stroke_width > 0.0
                ? node->stroke : node->fill;
            sr_put_pixel(scene, node, frame, x, y, color, opacity);
        }
    }
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

static void sr_draw_particles(const SrScene *scene, const SrNode *node,
                              SrMat3 world, double opacity, double time,
                              SrClip clip, SrFrame *frame) {
    double local_time = time - node->start_time;
    uint64_t born = local_time > 0.0
        ? (uint64_t)floor(local_time * node->particle_rate) : 0;
    uint64_t active = (uint64_t)ceil(node->particle_lifetime *
                                     node->particle_rate) + 1;
    uint64_t first = born > active ? born - active : 0;
    if (born - first > 1000000) first = born - 1000000;
    for (uint64_t i = first; i <= born; ++i) {
        double birth = node->particle_rate > 0.0 ? i / node->particle_rate : 0.0;
        double age = local_time - birth;
        if (age < 0.0 || age > node->particle_lifetime) continue;
        uint64_t base = scene->project.seed ^ ((uint64_t)node->order << 32) ^ i;
        double jitter = sr_random_signed(base);
        double angle = (node->particle_spread * jitter - 90.0) * SR_PI / 180.0;
        double speed = node->particle_speed * (0.75 +
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
        double radius = node->particle_size *
                        (strcmp(node->particle_preset, "smoke") == 0
                             ? 1.0 + age : 1.0);
        int x0 = (int)floor(center.x - radius), x1 = (int)ceil(center.x + radius);
        int y0 = (int)floor(center.y - radius), y1 = (int)ceil(center.y + radius);
        double fade = 1.0 - age / node->particle_lifetime;
        SrColor color = node->particle_color;
        color.a *= fade;
        for (int y = y0; y <= y1; ++y) for (int x = x0; x <= x1; ++x) {
            if (x < clip.x0 || x >= clip.x1 || y < clip.y0 || y >= clip.y1)
                continue;
            double dx = x + 0.5 - center.x, dy = y + 0.5 - center.y;
            if (dx * dx + dy * dy <= radius * radius)
                sr_put_pixel(scene, node, frame, x, y, color, opacity);
        }
    }
}

static void sr_draw_node(SrScene *scene, const SrNode *node,
                         SrMat3 parent, double parent_opacity, double time,
                         SrClip clip, SrFrame *frame, SrDiagnostics *diag) {
    if (!node->visible || time < node->start_time || time >= node->end_time) {
        return;
    }
    SrMat3 world = sr_mat_multiply(parent, sr_node_matrix(scene, node, time));
    double opacity = sr_clamp(parent_opacity * sr_anim_eval(&node->opacity, time),
                              0.0, 1.0);
    if (opacity <= 0.0) {
        return;
    }
    if (node->type == SR_NODE_MEDIA) {
        sr_draw_image(scene, node, world, opacity, clip, time, frame, diag);
        return;
    }
    if (node->type == SR_NODE_SHAPE) {
        sr_draw_shape(scene, node, world, opacity, clip, time, frame);
        return;
    }
    if (node->type == SR_NODE_PARTICLES) {
        sr_draw_particles(scene, node, world, opacity, time, clip, frame);
        return;
    }
    if (node->mask.enabled && !node->mask.invert) {
        SrMat3 mask_matrix = sr_mat_multiply(
            world, sr_mat_translate(node->mask.x, node->mask.y));
        SrClip mask_clip = sr_bounds(mask_matrix, node->mask.width,
                                     node->mask.height, frame);
        clip = sr_clip_intersect(clip, mask_clip);
    }
    for (size_t i = 0; i < node->child_count; ++i) {
        sr_draw_node(scene, node->children[i], world, opacity, time, clip, frame,
                     diag);
    }
}

SrStatus sr_composite_scene(SrScene *scene, double time, SrFrame *frame,
                            SrDiagnostics *diag) {
    if (!scene || !scene->root || !frame || !frame->rgba) {
        return SR_ERR_ARGUMENT;
    }
    SrClip clip = {0, 0, (int)frame->width, (int)frame->height};
    size_t errors = diag ? diag->errors : 0;
    sr_draw_node(scene, scene->root, sr_mat_identity(), 1.0, time, clip, frame,
                 diag);
    return diag && diag->errors > errors ? SR_ERR_ASSET : SR_OK;
}
