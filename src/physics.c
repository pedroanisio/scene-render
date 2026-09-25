#define _POSIX_C_SOURCE 200809L
#include "scene_render/physics.h"

#include <errno.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

typedef struct {
    SrNode *node;
    double x, y, angle, vx, vy, angular_velocity;
    double inverse_mass;
} BodyState;

typedef struct {
    size_t a, b;
    double rest;
} Spring;

/* A soft body: a rows x cols grid of point masses in physics space joined
 * by structural and shear springs. */
typedef struct {
    SrNode *node;
    BodyState *rigid;           /* the node's own rigid body, or NULL */
    uint32_t rows, cols;
    size_t count;
    double width, height;       /* local box spanned by the grid */
    double *rest;               /* 2 * count local rest coordinates */
    double *px, *py, *vx, *vy;  /* physics-space state */
    bool *pinned;
    bool *bounded;              /* started inside the frame: collides with it */
    double node_mass;
    Spring *springs;
    size_t spring_count;
    size_t *ring;               /* boundary loop, for pressure */
    size_t ring_count;
    double area0;
} SoftState;

/* Bump whenever the integrator changes so caches written by an older
 * simulation are re-simulated instead of replayed (2: exponential damping;
 * 3: soft-body grids, OBB contacts, pins, vortex and animated fields). */
#define PHYSICS_CACHE_VERSION 3u

typedef struct {
    char magic[8];
    uint32_t version;
    uint32_t body_count;
    uint32_t soft_count;
    uint32_t reserved;
    uint64_t sample_count;
    uint64_t signature;
    double fixed_step;
} CacheHeader;

static bool collect(SrNode *node, BodyState **states, size_t *count,
                    size_t *capacity) {
    if (node->body.type != SR_BODY_NONE) {
        if (*count == *capacity) {
            size_t next = *capacity ? *capacity * 2 : 8;
            BodyState *grown = sr_realloc(*states, next * sizeof(*grown));
            if (!grown) return false;
            *states = grown; *capacity = next;
        }
        SrRigidBody *body = &node->body;
        (*states)[*count] = (BodyState){node, node->transform.x.base,
            node->transform.y.base, node->transform.rotation.base,
            body->velocity_x, body->velocity_y, body->angular_velocity,
            body->type == SR_BODY_DYNAMIC && body->mass > 0.0 ?
                1.0 / body->mass : 0.0};
        ++*count;
    }
    for (size_t i = 0; i < node->child_count; ++i)
        if (!collect(node->children[i], states, count, capacity)) return false;
    return true;
}

/* Local box of a node, or false when it has none. */
static bool local_box(const SrNode *node, double *width, double *height) {
    if (node->type == SR_NODE_MEDIA && node->asset) {
        *width = node->asset->width;
        *height = node->asset->height;
    } else if (node->type == SR_NODE_SHAPE) {
        *width = node->shape_width;
        *height = node->shape_height;
    } else {
        return false;
    }
    return *width > 0.0 && *height > 0.0;
}

static bool collect_soft(SrNode *node, SrNode ***nodes, size_t *count,
                         size_t *capacity) {
    double width, height;
    if (node->soft_body.enabled && local_box(node, &width, &height)) {
        if (*count == *capacity) {
            size_t next = *capacity ? *capacity * 2 : 4;
            SrNode **grown = sr_realloc(*nodes, next * sizeof(*grown));
            if (!grown) return false;
            *nodes = grown; *capacity = next;
        }
        (*nodes)[(*count)++] = node;
    }
    for (size_t i = 0; i < node->child_count; ++i)
        if (!collect_soft(node->children[i], nodes, count, capacity)) return false;
    return true;
}

/* ---- signature ------------------------------------------------------------ */

static uint64_t hash_bytes(uint64_t hash, const void *data, size_t length) {
    const uint8_t *bytes = data;
    for (size_t i = 0; i < length; ++i) {
        hash ^= bytes[i]; hash *= UINT64_C(1099511628211);
    }
    return hash;
}

static uint64_t hash_double(uint64_t hash, double value) {
    return hash_bytes(hash, &value, sizeof(value));
}

static uint64_t hash_string(uint64_t hash, const char *text) {
    if (!text) return hash_bytes(hash, "\xff", 1);
    return hash_bytes(hash, text, strlen(text) + 1);
}

static uint64_t hash_anim(uint64_t hash, const SrAnimValue *value) {
    hash = hash_double(hash, value->base);
    uint64_t count = value->track.count;
    hash = hash_bytes(hash, &count, sizeof(count));
    for (size_t i = 0; i < value->track.count; ++i) {
        const SrKeyframe *key = &value->track.keys[i];
        int32_t curve = (int32_t)key->curve;
        hash = hash_double(hash, key->time);
        hash = hash_double(hash, key->value);
        hash = hash_bytes(hash, &curve, sizeof(curve));
        hash = hash_double(hash, key->x1); hash = hash_double(hash, key->y1);
        hash = hash_double(hash, key->x2); hash = hash_double(hash, key->y2);
    }
    return hash;
}

static uint64_t hash_transform(uint64_t hash, const SrNode *node) {
    hash = hash_double(hash, node->transform.x.base);
    hash = hash_double(hash, node->transform.y.base);
    hash = hash_double(hash, node->transform.rotation.base);
    hash = hash_double(hash, node->transform.scale_x.base);
    hash = hash_double(hash, node->transform.scale_y.base);
    hash = hash_double(hash, node->transform.anchor_x.base);
    return hash_double(hash, node->transform.anchor_y.base);
}

static uint64_t signature(const SrScene *scene, const BodyState *states,
                          size_t count, SrNode *const *softs, size_t soft_count) {
    uint64_t hash = UINT64_C(1469598103934665603);
    uint32_t version = PHYSICS_CACHE_VERSION;
    hash = hash_bytes(hash, SR_VERSION, strlen(SR_VERSION));
    hash = hash_bytes(hash, &version, sizeof(version));
    hash = hash_bytes(hash, &scene->project.seed, sizeof(scene->project.seed));
    hash = hash_double(hash, scene->project.duration);
    hash = hash_bytes(hash, &scene->project.width, sizeof(uint32_t));
    hash = hash_bytes(hash, &scene->project.height, sizeof(uint32_t));
    hash = hash_double(hash, scene->physics.fixed_step);
    hash = hash_double(hash, scene->physics.gravity_x);
    hash = hash_double(hash, scene->physics.gravity_y);
    hash = hash_bytes(hash, &scene->physics.enabled, sizeof(bool));
    for (size_t i = 0; i < count; ++i) {
        const SrNode *node = states[i].node;
        const SrRigidBody *body = &node->body;
        int32_t kinds[2] = {(int32_t)body->type, (int32_t)body->collider};
        hash = hash_string(hash, node->id);
        hash = hash_bytes(hash, kinds, sizeof(kinds));
        const double values[] = {body->mass, body->friction, body->restitution,
            body->linear_damping, body->angular_damping, body->velocity_x,
            body->velocity_y, body->angular_velocity, body->radius,
            node->shape_width, node->shape_height};
        hash = hash_bytes(hash, values, sizeof(values));
        hash = hash_transform(hash, node);
        if (node->asset) {
            hash = hash_bytes(hash, &node->asset->width, sizeof(uint32_t));
            hash = hash_bytes(hash, &node->asset->height, sizeof(uint32_t));
        }
    }
    uint64_t fields = scene->physics.field_count;
    hash = hash_bytes(hash, &fields, sizeof(fields));
    for (size_t i = 0; i < scene->physics.field_count; ++i) {
        const SrForceField *field = &scene->physics.fields[i];
        int32_t type = (int32_t)field->type;
        hash = hash_string(hash, field->id);
        hash = hash_bytes(hash, &type, sizeof(type));
        hash = hash_anim(hash, &field->x);
        hash = hash_anim(hash, &field->y);
        hash = hash_anim(hash, &field->force_x);
        hash = hash_anim(hash, &field->force_y);
        hash = hash_anim(hash, &field->strength);
        hash = hash_double(hash, field->falloff);
    }
    uint64_t constraints = scene->physics.constraint_count;
    hash = hash_bytes(hash, &constraints, sizeof(constraints));
    for (size_t i = 0; i < scene->physics.constraint_count; ++i) {
        const SrConstraint *constraint = &scene->physics.constraints[i];
        int32_t type = (int32_t)constraint->type;
        hash = hash_string(hash, constraint->id);
        hash = hash_bytes(hash, &type, sizeof(type));
        hash = hash_string(hash, constraint->a_id);
        hash = hash_string(hash, constraint->b_id);
        const double values[] = {constraint->rest_length, constraint->stiffness,
                                 constraint->damping, constraint->x, constraint->y,
                                 constraint->rigid ? 1.0 : 0.0};
        hash = hash_bytes(hash, values, sizeof(values));
    }
    uint64_t softs_count = soft_count;
    hash = hash_bytes(hash, &softs_count, sizeof(softs_count));
    for (size_t i = 0; i < soft_count; ++i) {
        const SrNode *node = softs[i];
        const SrSoftBody *soft = &node->soft_body;
        double width = 0.0, height = 0.0;
        local_box(node, &width, &height);
        const double values[] = {soft->mass, soft->stiffness, soft->damping,
                                 soft->pressure, (double)soft->rows,
                                 (double)soft->cols, (double)soft->pin,
                                 width, height,
                                 node->body.type != SR_BODY_NONE ? 1.0 : 0.0};
        hash = hash_string(hash, node->id);
        hash = hash_bytes(hash, values, sizeof(values));
        hash = hash_transform(hash, node);
    }
    return hash;
}

/* ---- cache ---------------------------------------------------------------- */

static char *cache_path(const SrScene *scene) {
    return scene->physics.cache_path ?
        sr_path_join(scene->base_dir, scene->physics.cache_path) : NULL;
}

static void release_samples(BodyState *states, size_t count,
                            SrNode *const *softs, size_t soft_count) {
    for (size_t i = 0; i < count; ++i) {
        free(states[i].node->physics_samples);
        states[i].node->physics_samples = NULL;
        states[i].node->physics_sample_count = 0;
    }
    for (size_t i = 0; i < soft_count; ++i) {
        free(softs[i]->soft_body.offsets);
        softs[i]->soft_body.offsets = NULL;
        softs[i]->soft_body.sample_count = 0;
    }
}

static bool allocate_samples(BodyState *states, size_t count,
                             SrNode *const *softs, size_t soft_count,
                             uint64_t samples) {
    for (size_t i = 0; i < count; ++i) {
        SrNode *node = states[i].node;
        free(node->physics_samples);
        node->physics_samples = sr_alloc(samples * sizeof(SrPhysicsSample));
        node->physics_sample_count = node->physics_samples ? samples : 0;
        if (!node->physics_samples) return false;
    }
    for (size_t i = 0; i < soft_count; ++i) {
        SrSoftBody *soft = &softs[i]->soft_body;
        size_t values = (size_t)soft->rows * soft->cols * 2;
        free(soft->offsets);
        soft->offsets = NULL;
        soft->sample_count = 0;
        if (samples > SIZE_MAX / sizeof(double) / values) return false;
        soft->offsets = sr_alloc(samples * values * sizeof(double));
        if (!soft->offsets) return false;
        soft->sample_count = samples;
    }
    return true;
}

static bool load_cache(SrScene *scene, BodyState *states, size_t count,
                       SrNode *const *softs, size_t soft_count,
                       uint64_t samples, uint64_t expected) {
    char *path = cache_path(scene);
    if (!path) return false;
    FILE *file = fopen(path, "rb"); free(path);
    if (!file) return false;
    CacheHeader header;
    bool ok = fread(&header, sizeof(header), 1, file) == 1 &&
        memcmp(header.magic, "SRPHYS1", 8) == 0 &&
        header.version == PHYSICS_CACHE_VERSION &&
        header.body_count == count && header.soft_count == soft_count &&
        header.sample_count == samples && header.signature == expected &&
        header.fixed_step == scene->physics.fixed_step;
    if (ok) ok = allocate_samples(states, count, softs, soft_count, samples);
    for (size_t i = 0; ok && i < count; ++i) {
        for (uint64_t s = 0; s < samples; ++s) {
            double pose[3];
            if (fread(pose, sizeof(pose), 1, file) != 1) { ok = false; break; }
            SrPhysicsSample *sample = &states[i].node->physics_samples[s];
            sample->enabled = true;
            sample->x.base = pose[0];
            sample->y.base = pose[1];
            sample->rotation.base = pose[2];
        }
    }
    for (size_t i = 0; ok && i < soft_count; ++i) {
        SrSoftBody *soft = &softs[i]->soft_body;
        size_t values = (size_t)soft->rows * soft->cols * 2 * samples;
        if (fread(soft->offsets, sizeof(double), values, file) != values) ok = false;
    }
    fclose(file);
    if (!ok) release_samples(states, count, softs, soft_count);
    return ok;
}

static void save_cache(const SrScene *scene, BodyState *states, size_t count,
                       SrNode *const *softs, size_t soft_count,
                       uint64_t samples, uint64_t hash, SrDiagnostics *diag) {
    char *path = cache_path(scene); if (!path) return;
    size_t length = strlen(path) + 16;
    char *temporary = sr_alloc(length);
    if (!temporary) { free(path); return; }
    snprintf(temporary, length, "%s.tmp-XXXXXX", path);
    int fd = mkstemp(temporary);
    FILE *file = fd >= 0 ? fdopen(fd, "wb") : NULL;
    if (!file) { if (fd >= 0) close(fd); free(temporary); free(path); return; }
    CacheHeader header = {{'S','R','P','H','Y','S','1','\0'}, PHYSICS_CACHE_VERSION,
                          (uint32_t)count, (uint32_t)soft_count, 0, samples, hash,
                          scene->physics.fixed_step};
    bool ok = fwrite(&header, sizeof(header), 1, file) == 1;
    for (size_t i = 0; ok && i < count; ++i) for (uint64_t s = 0; s < samples; ++s) {
        SrPhysicsSample *sample = &states[i].node->physics_samples[s];
        double pose[3] = {sample->x.base, sample->y.base, sample->rotation.base};
        if (fwrite(pose, sizeof(pose), 1, file) != 1) { ok = false; break; }
    }
    for (size_t i = 0; ok && i < soft_count; ++i) {
        const SrSoftBody *soft = &softs[i]->soft_body;
        size_t values = (size_t)soft->rows * soft->cols * 2 * samples;
        if (fwrite(soft->offsets, sizeof(double), values, file) != values) ok = false;
    }
    if (fclose(file) != 0) ok = false;
    if (ok && rename(temporary, path) != 0) ok = false;
    if (!ok) {
        unlink(temporary);
        sr_diag_warning(diag, 0, NULL, NULL,
                        "could not atomically write physics cache '%s': %s",
                        path, strerror(errno));
    }
    free(temporary); free(path);
}

/* ---- rigid bodies ---------------------------------------------------------- */

static void dimensions(const SrNode *node, double *width, double *height) {
    if (node->type == SR_NODE_MEDIA && node->asset) {
        *width = node->asset->width * fabs(node->transform.scale_x.base);
        *height = node->asset->height * fabs(node->transform.scale_y.base);
    } else {
        *width = node->shape_width * fabs(node->transform.scale_x.base);
        *height = node->shape_height * fabs(node->transform.scale_y.base);
    }
    if (*width <= 0.0) *width = node->body.radius > 0 ? node->body.radius * 2 : 1;
    if (*height <= 0.0) *height = node->body.radius > 0 ? node->body.radius * 2 : 1;
}

static BodyState *state_for(BodyState *states, size_t count, const SrNode *node) {
    for (size_t i = 0; i < count; ++i) if (states[i].node == node) return &states[i];
    return NULL;
}

static void apply_constraints(SrScene *scene, BodyState *states, size_t count) {
    double dt = scene->physics.fixed_step;
    for (size_t i = 0; i < scene->physics.constraint_count; ++i) {
        SrConstraint *constraint = &scene->physics.constraints[i];
        if (constraint->type == SR_CONSTRAINT_PIN && constraint->rigid) continue;
        BodyState *a = state_for(states, count, constraint->a);
        if (!a) continue;
        BodyState *b = NULL;
        double bx, by, bvx = 0.0, bvy = 0.0;
        if (constraint->type == SR_CONSTRAINT_PIN) {
            bx = constraint->x; by = constraint->y;
        } else {
            b = state_for(states, count, constraint->b);
            if (!b) continue;
            bx = b->x; by = b->y; bvx = b->vx; bvy = b->vy;
        }
        double dx = bx - a->x, dy = by - a->y;
        double length = hypot(dx, dy); if (length < 1e-9) continue;
        double nx = dx / length, ny = dy / length;
        double relative = (bvx-a->vx)*nx + (bvy-a->vy)*ny;
        double force = (length-constraint->rest_length)*constraint->stiffness +
                       relative*constraint->damping;
        if (a->inverse_mass) { a->vx += force*nx*a->inverse_mass*dt;
                               a->vy += force*ny*a->inverse_mass*dt; }
        if (b && b->inverse_mass) { b->vx -= force*nx*b->inverse_mass*dt;
                                    b->vy -= force*ny*b->inverse_mass*dt; }
    }
}

/* Rigid pins: the body is projected onto the circle of radius restLength
 * around the anchor (onto the anchor when it is 0) and loses its velocity
 * along the pin. */
static void project_pins(SrScene *scene, BodyState *states, size_t count) {
    for (size_t i = 0; i < scene->physics.constraint_count; ++i) {
        SrConstraint *constraint = &scene->physics.constraints[i];
        if (constraint->type != SR_CONSTRAINT_PIN || !constraint->rigid) continue;
        BodyState *a = state_for(states, count, constraint->a);
        if (!a || !a->inverse_mass) continue;
        double dx = a->x - constraint->x, dy = a->y - constraint->y;
        double length = hypot(dx, dy);
        if (constraint->rest_length <= 0.0 || length < 1e-12) {
            a->x = constraint->x; a->y = constraint->y;
            a->vx = a->vy = 0.0;
            continue;
        }
        double nx = dx / length, ny = dy / length;
        a->x = constraint->x + nx * constraint->rest_length;
        a->y = constraint->y + ny * constraint->rest_length;
        double radial = a->vx * nx + a->vy * ny;
        a->vx -= radial * nx;
        a->vy -= radial * ny;
    }
}

static double circle_radius(const BodyState *state, double width, double height) {
    return state->node->body.radius > 0 ? state->node->body.radius
                                        : fmax(width, height) / 2;
}

/* Contact normal (from box toward circle) and penetration of a circle
 * against an oriented box, or false when they do not touch. */
static bool circle_box(const BodyState *box, double bw, double bh,
                       const BodyState *circle, double radius,
                       double *nx, double *ny, double *penetration) {
    double angle = box->angle * SR_PI / 180.0, c = cos(angle), s = sin(angle);
    double wx = circle->x - box->x, wy = circle->y - box->y;
    double lx = c * wx + s * wy, ly = -s * wx + c * wy;
    double hw = bw / 2, hh = bh / 2;
    double qx = fmax(-hw, fmin(hw, lx)), qy = fmax(-hh, fmin(hh, ly));
    double mx, my, depth;
    if (fabs(lx) <= hw && fabs(ly) <= hh) {
        /* Center inside the box: leave through the nearest face. */
        double px = hw - fabs(lx), py = hh - fabs(ly);
        if (px < py) { mx = lx < 0 ? -1 : 1; my = 0; depth = px + radius; }
        else { mx = 0; my = ly < 0 ? -1 : 1; depth = py + radius; }
    } else {
        double ex = lx - qx, ey = ly - qy, distance = hypot(ex, ey);
        if (distance >= radius) return false;
        mx = ex / distance; my = ey / distance; depth = radius - distance;
    }
    *nx = c * mx - s * my;
    *ny = s * mx + c * my;
    *penetration = depth;
    return true;
}

/* Separating-axis test of two oriented boxes: the normal (from a to b) and
 * depth of the axis with the least overlap. For axis-aligned boxes this is
 * exactly the previous per-axis overlap test, ties going to the y axis. */
static bool box_box(const BodyState *a, double aw, double ah,
                    const BodyState *b, double bw, double bh,
                    double *nx, double *ny, double *penetration) {
    double ta = a->angle * SR_PI / 180.0, tb = b->angle * SR_PI / 180.0;
    double axes[4][2] = {{cos(ta), sin(ta)}, {-sin(ta), cos(ta)},
                         {cos(tb), sin(tb)}, {-sin(tb), cos(tb)}};
    double dx = b->x - a->x, dy = b->y - a->y;
    double best = INFINITY;
    for (int k = 0; k < 4; ++k) {
        double lx = axes[k][0], ly = axes[k][1];
        double ra = aw * fabs(axes[0][0] * lx + axes[0][1] * ly) +
                    ah * fabs(axes[1][0] * lx + axes[1][1] * ly);
        double rb = bw * fabs(axes[2][0] * lx + axes[2][1] * ly) +
                    bh * fabs(axes[3][0] * lx + axes[3][1] * ly);
        double along = dx * lx + dy * ly;
        double overlap = (ra + rb) / 2 - fabs(along);
        if (overlap <= 0) return false;
        if (overlap < best || (k % 2 == 1 && overlap <= best)) {
            best = overlap;
            double sign = along < 0 ? -1 : 1;
            *nx = lx * sign; *ny = ly * sign;
        }
    }
    *penetration = best;
    return true;
}

static void collide(BodyState *a, BodyState *b) {
    if (!a->inverse_mass && !b->inverse_mass) return;
    double aw, ah, bw, bh; dimensions(a->node,&aw,&ah); dimensions(b->node,&bw,&bh);
    double nx = 0, ny = 0, penetration = 0;
    bool a_circle = a->node->body.collider == SR_COLLIDER_CIRCLE;
    bool b_circle = b->node->body.collider == SR_COLLIDER_CIRCLE;
    if (a_circle && b_circle) {
        double ar = circle_radius(a, aw, ah), br = circle_radius(b, bw, bh);
        double dx = b->x - a->x, dy = b->y - a->y;
        double distance = hypot(dx, dy); if (distance >= ar + br) return;
        if (distance < 1e-9) { nx = 1; ny = 0; } else { nx = dx/distance; ny = dy/distance; }
        penetration = ar + br - distance;
    } else if (a_circle || b_circle) {
        bool a_box = !a_circle;
        const BodyState *box = a_box ? a : b, *circle = a_box ? b : a;
        double w = a_box ? aw : bw, h = a_box ? ah : bh;
        double cr = a_box ? circle_radius(b, bw, bh) : circle_radius(a, aw, ah);
        if (!circle_box(box, w, h, circle, cr, &nx, &ny, &penetration)) return;
        if (!a_box) { nx = -nx; ny = -ny; }
    } else if (!box_box(a, aw, ah, b, bw, bh, &nx, &ny, &penetration)) {
        return;
    }
    double inv=a->inverse_mass+b->inverse_mass;if(inv<=0)return;
    if(a->inverse_mass){a->x-=nx*penetration*a->inverse_mass/inv;a->y-=ny*penetration*a->inverse_mass/inv;}
    if(b->inverse_mass){b->x+=nx*penetration*b->inverse_mass/inv;b->y+=ny*penetration*b->inverse_mass/inv;}
    double relative=(b->vx-a->vx)*nx+(b->vy-a->vy)*ny;if(relative>=0)return;
    double restitution=fmin(a->node->body.restitution,b->node->body.restitution);
    double impulse=-(1+restitution)*relative/inv;
    if(a->inverse_mass){a->vx-=impulse*nx*a->inverse_mass;a->vy-=impulse*ny*a->inverse_mass;}
    if(b->inverse_mass){b->vx+=impulse*nx*b->inverse_mass;b->vy+=impulse*ny*b->inverse_mass;}
    double tx=-ny,ty=nx,tangent=(b->vx-a->vx)*tx+(b->vy-a->vy)*ty;
    double friction=sqrt(a->node->body.friction*b->node->body.friction);
    double fi=fmax(-impulse*friction,fmin(impulse*friction,-tangent/inv));
    if(a->inverse_mass){a->vx-=fi*tx*a->inverse_mass;a->vy-=fi*ty*a->inverse_mass;}
    if(b->inverse_mass){b->vx+=fi*tx*b->inverse_mass;b->vy+=fi*ty*b->inverse_mass;}
}

/* World gravity plus every force field at (x, y), time t. */
static void acceleration(const SrScene *scene, double x, double y, double t,
                         double *ax, double *ay) {
    *ax = scene->physics.gravity_x;
    *ay = scene->physics.gravity_y;
    for (size_t f = 0; f < scene->physics.field_count; ++f) {
        const SrForceField *field = &scene->physics.fields[f];
        if (field->type == SR_FIELD_DIRECTIONAL) {
            *ax += sr_anim_eval(&field->force_x, t);
            *ay += sr_anim_eval(&field->force_y, t);
            continue;
        }
        double fx = sr_anim_eval(&field->x, t), fy = sr_anim_eval(&field->y, t);
        double dx = fx - x, dy = fy - y, d = hypot(dx, dy);
        if (d <= 1e-9) continue;
        double scale = sr_anim_eval(&field->strength, t) /
                       pow(1 + d, fmax(0.0, field->falloff));
        if (field->type == SR_FIELD_RADIAL) {
            *ax += dx / d * scale;
            *ay += dy / d * scale;
        } else {
            /* Vortex: tangential, clockwise on screen (y down). */
            *ax += dy / d * scale;
            *ay += -dx / d * scale;
        }
    }
}

static void simulate_step(SrScene *scene, BodyState *states, size_t count,
                          double t) {
    double dt=scene->physics.fixed_step;
    apply_constraints(scene,states,count);
    for(size_t i=0;i<count;++i){BodyState *state=&states[i];
        if (state->node->body.type == SR_BODY_KINEMATIC) {
            state->x += state->vx * dt;
            state->y += state->vy * dt;
            state->angle += state->angular_velocity * dt;
            continue;
        }
        if(!state->inverse_mass)continue;
        double ax, ay;
        acceleration(scene, state->x, state->y, t, &ax, &ay);
        /* Exact solution of dv/dt = -damping*v over one step: the factor
         * stays in (0,1] for any damping*dt >= 0, so velocity decays but
         * never flips sign or snaps to zero. */
        double linear=exp(-state->node->body.linear_damping*dt);
        state->vx=(state->vx+ax*dt)*linear;
        state->vy=(state->vy+ay*dt)*linear;
        state->angular_velocity*=exp(-state->node->body.angular_damping*dt);
        state->x+=state->vx*dt;state->y+=state->vy*dt;state->angle+=state->angular_velocity*dt;}
    for(size_t iteration=0;iteration<3;++iteration)for(size_t i=0;i<count;++i)for(size_t j=i+1;j<count;++j)collide(&states[i],&states[j]);
    for (int pass = 0; pass < 4; ++pass) project_pins(scene, states, count);
}

/* ---- soft bodies ----------------------------------------------------------- */

/* Node pose used for the grid's rest frame: the rigid body when the node
 * has one, else the static transform. */
static SrMat3 soft_pose(const SoftState *soft) {
    const SrNode *node = soft->node;
    double x = node->transform.x.base, y = node->transform.y.base;
    double angle = node->transform.rotation.base;
    if (soft->rigid) { x = soft->rigid->x; y = soft->rigid->y; angle = soft->rigid->angle; }
    SrMat3 matrix = sr_mat_translate(x, y);
    matrix = sr_mat_multiply(matrix, sr_mat_rotate(angle * SR_PI / 180.0));
    matrix = sr_mat_multiply(matrix, sr_mat_scale(node->transform.scale_x.base,
                                                  node->transform.scale_y.base));
    return sr_mat_multiply(matrix, sr_mat_translate(-node->transform.anchor_x.base,
                                                    -node->transform.anchor_y.base));
}

static bool pinned_node(SrPinMode pin, uint32_t r, uint32_t c, uint32_t rows,
                        uint32_t cols) {
    switch (pin) {
    case SR_PIN_TOP: return r == 0;
    case SR_PIN_BOTTOM: return r == rows - 1;
    case SR_PIN_LEFT: return c == 0;
    case SR_PIN_RIGHT: return c == cols - 1;
    case SR_PIN_CORNERS: return (r == 0 || r == rows - 1) && (c == 0 || c == cols - 1);
    default: return false;
    }
}

static void soft_free(SoftState *soft) {
    free(soft->rest); free(soft->px); free(soft->py); free(soft->vx);
    free(soft->vy); free(soft->pinned); free(soft->bounded); free(soft->springs);
    free(soft->ring);
}

static double ring_area(const SoftState *soft) {
    double area = 0.0;
    for (size_t i = 0; i < soft->ring_count; ++i) {
        size_t a = soft->ring[i], b = soft->ring[(i + 1) % soft->ring_count];
        area += soft->px[a] * soft->py[b] - soft->px[b] * soft->py[a];
    }
    return area * 0.5;
}

static bool soft_init(const SrScene *scene, SoftState *soft, SrNode *node,
                      BodyState *rigid) {
    *soft = (SoftState){.node = node, .rigid = rigid,
                        .rows = node->soft_body.rows, .cols = node->soft_body.cols};
    local_box(node, &soft->width, &soft->height);
    size_t n = soft->count = (size_t)soft->rows * soft->cols;
    soft->rest = sr_alloc(2 * n * sizeof(double));
    soft->px = sr_alloc(n * sizeof(double)); soft->py = sr_alloc(n * sizeof(double));
    soft->vx = sr_alloc(n * sizeof(double)); soft->vy = sr_alloc(n * sizeof(double));
    soft->pinned = sr_alloc(n * sizeof(bool));
    soft->bounded = sr_alloc(n * sizeof(bool));
    size_t springs = 2 * n + 2 * (size_t)(soft->rows - 1) * (soft->cols - 1);
    soft->springs = sr_alloc(springs * sizeof(Spring));
    soft->ring = sr_alloc(2 * (soft->rows + soft->cols) * sizeof(size_t));
    if (!soft->rest || !soft->px || !soft->py || !soft->vx || !soft->vy ||
        !soft->pinned || !soft->bounded || !soft->springs || !soft->ring) return false;
    soft->node_mass = node->soft_body.mass / (double)n;
    SrMat3 pose = soft_pose(soft);
    for (uint32_t r = 0; r < soft->rows; ++r) for (uint32_t c = 0; c < soft->cols; ++c) {
        size_t k = (size_t)r * soft->cols + c;
        soft->rest[2 * k] = soft->width * c / (soft->cols - 1);
        soft->rest[2 * k + 1] = soft->height * r / (soft->rows - 1);
        SrVec2 p = sr_mat_point(pose, (SrVec2){soft->rest[2 * k], soft->rest[2 * k + 1]});
        soft->px[k] = p.x; soft->py[k] = p.y;
        soft->vx[k] = rigid ? rigid->vx : 0.0;
        soft->vy[k] = rigid ? rigid->vy : 0.0;
        soft->pinned[k] = pinned_node(node->soft_body.pin, r, c, soft->rows, soft->cols);
        soft->bounded[k] = p.x >= 0.0 && p.y >= 0.0 && p.x <= scene->project.width &&
                           p.y <= scene->project.height;
    }
    /* Structural springs along rows and columns, shear springs across cells. */
    for (uint32_t r = 0; r < soft->rows; ++r) for (uint32_t c = 0; c < soft->cols; ++c) {
        size_t k = (size_t)r * soft->cols + c;
        size_t pairs[4][2] = {{k, k + 1}, {k, k + soft->cols},
                              {k, k + soft->cols + 1}, {k + 1, k + soft->cols}};
        bool valid[4] = {c + 1 < soft->cols, r + 1 < soft->rows,
                         c + 1 < soft->cols && r + 1 < soft->rows,
                         c + 1 < soft->cols && r + 1 < soft->rows};
        for (int s = 0; s < 4; ++s) {
            if (!valid[s]) continue;
            size_t a = pairs[s][0], b = pairs[s][1];
            soft->springs[soft->spring_count++] = (Spring){a, b,
                hypot(soft->px[b] - soft->px[a], soft->py[b] - soft->py[a])};
        }
    }
    /* Boundary loop: top row left to right, right column down, bottom row
     * right to left, left column up. */
    for (uint32_t c = 0; c < soft->cols; ++c) soft->ring[soft->ring_count++] = c;
    for (uint32_t r = 1; r < soft->rows; ++r)
        soft->ring[soft->ring_count++] = (size_t)r * soft->cols + soft->cols - 1;
    for (uint32_t c = soft->cols - 1; c-- > 0;)
        soft->ring[soft->ring_count++] = (size_t)(soft->rows - 1) * soft->cols + c;
    for (uint32_t r = soft->rows - 1; r-- > 1;)
        soft->ring[soft->ring_count++] = (size_t)r * soft->cols;
    soft->area0 = ring_area(soft);
    return true;
}

/* Local-space offsets of the grid from its rest positions. */
static void soft_record(const SoftState *soft, double *out) {
    SrMat3 inverse;
    if (!sr_mat_inverse(soft_pose(soft), &inverse)) inverse = sr_mat_identity();
    for (size_t k = 0; k < soft->count; ++k) {
        SrVec2 local = sr_mat_point(inverse, (SrVec2){soft->px[k], soft->py[k]});
        out[2 * k] = local.x - soft->rest[2 * k];
        out[2 * k + 1] = local.y - soft->rest[2 * k + 1];
    }
}

/* One fixed step of the grid: gravity and fields, springs with damping
 * ratio `damping`, area-preserving pressure, anchor springs to the node's
 * rigid pose (when it has a rigid body), pins, and frame-bound collisions;
 * semi-implicit Euler with enough substeps to stay stable. */
static void soft_step(const SrScene *scene, SoftState *soft, double t) {
    const SrSoftBody *body = &soft->node->soft_body;
    double dt = scene->physics.fixed_step, m = soft->node_mass;
    double k = body->stiffness, zeta = body->damping;
    double c = 2.0 * zeta * sqrt(k * m);
    double springs_per_node = soft->rigid ? 9.0 : 8.0;
    double omega = sqrt(springs_per_node * k / m);
    double substeps = fmax(ceil(omega * dt / 0.25),
                           ceil(springs_per_node * c * dt / (m * 0.5)));
    int steps = (int)fmin(256.0, fmax(1.0, substeps));
    double h = dt / steps, decay = exp(-body->damping * h);
    double width = scene->project.width, height = scene->project.height;
    double *fx = sr_alloc(soft->count * sizeof(double));
    double *fy = sr_alloc(soft->count * sizeof(double));
    if (!fx || !fy) { free(fx); free(fy); return; }
    SrMat3 pose = soft_pose(soft);
    for (int step = 0; step < steps; ++step) {
        double time = t + step * h;
        for (size_t i = 0; i < soft->count; ++i) {
            double ax, ay;
            acceleration(scene, soft->px[i], soft->py[i], time, &ax, &ay);
            fx[i] = ax * m; fy[i] = ay * m;
        }
        for (size_t s = 0; s < soft->spring_count; ++s) {
            const Spring *spring = &soft->springs[s];
            size_t a = spring->a, b = spring->b;
            double dx = soft->px[b] - soft->px[a], dy = soft->py[b] - soft->py[a];
            double length = hypot(dx, dy);
            if (length < 1e-9) continue;
            double nx = dx / length, ny = dy / length;
            double relative = (soft->vx[b] - soft->vx[a]) * nx +
                              (soft->vy[b] - soft->vy[a]) * ny;
            double force = k * (length - spring->rest) + c * relative;
            fx[a] += force * nx; fy[a] += force * ny;
            fx[b] -= force * nx; fy[b] -= force * ny;
        }
        if (body->pressure != 0.0 && soft->ring_count >= 3 &&
            fabs(soft->area0) > 1e-9) {
            double area = ring_area(soft);
            double ratio = area * soft->area0 > 1e-12 ? soft->area0 / area : 4.0;
            double pressure = body->pressure * (fmin(ratio, 4.0) - 1.0);
            double sign = soft->area0 > 0.0 ? 1.0 : -1.0;
            for (size_t i = 0; i < soft->ring_count; ++i) {
                size_t a = soft->ring[i], b = soft->ring[(i + 1) % soft->ring_count];
                double ex = soft->px[b] - soft->px[a], ey = soft->py[b] - soft->py[a];
                double ox = ey * sign * pressure * 0.5, oy = -ex * sign * pressure * 0.5;
                fx[a] += ox; fy[a] += oy; fx[b] += ox; fy[b] += oy;
            }
        }
        if (soft->rigid) {
            for (size_t i = 0; i < soft->count; ++i) {
                SrVec2 anchor = sr_mat_point(pose, (SrVec2){soft->rest[2 * i],
                                                            soft->rest[2 * i + 1]});
                fx[i] += k * (anchor.x - soft->px[i]) + c * (soft->rigid->vx - soft->vx[i]);
                fy[i] += k * (anchor.y - soft->py[i]) + c * (soft->rigid->vy - soft->vy[i]);
            }
        }
        for (size_t i = 0; i < soft->count; ++i) {
            if (soft->pinned[i]) {
                SrVec2 anchor = sr_mat_point(pose, (SrVec2){soft->rest[2 * i],
                                                            soft->rest[2 * i + 1]});
                soft->px[i] = anchor.x; soft->py[i] = anchor.y;
                soft->vx[i] = soft->rigid ? soft->rigid->vx : 0.0;
                soft->vy[i] = soft->rigid ? soft->rigid->vy : 0.0;
                continue;
            }
            soft->vx[i] = (soft->vx[i] + fx[i] / m * h) * decay;
            soft->vy[i] = (soft->vy[i] + fy[i] / m * h) * decay;
            soft->px[i] += soft->vx[i] * h;
            soft->py[i] += soft->vy[i] * h;
            /* Frame bounds: restitution 0.2, friction 0.5. */
            if (!soft->bounded[i]) continue;
            if (soft->px[i] < 0.0 || soft->px[i] > width) {
                soft->px[i] = soft->px[i] < 0.0 ? 0.0 : width;
                soft->vx[i] *= -0.2; soft->vy[i] *= 0.5;
            }
            if (soft->py[i] < 0.0 || soft->py[i] > height) {
                soft->py[i] = soft->py[i] < 0.0 ? 0.0 : height;
                soft->vy[i] *= -0.2; soft->vx[i] *= 0.5;
            }
        }
    }
    free(fx); free(fy);
}

/* ---- driver ---------------------------------------------------------------- */

SrStatus sr_physics_prepare(SrScene *scene, SrDiagnostics *diag) {
    BodyState *states = NULL;
    size_t count = 0, capacity = 0;
    SrNode **softs = NULL;
    size_t soft_count = 0, soft_capacity = 0;
    if (scene->physics.enabled &&
        !collect(scene->root, &states, &count, &capacity)) {
        free(states); return SR_ERR_MEMORY;
    }
    if (!collect_soft(scene->root, &softs, &soft_count, &soft_capacity)) {
        free(states); free(softs); return SR_ERR_MEMORY;
    }
    if (!count && !soft_count) { free(states); free(softs); return SR_OK; }
    double exact_samples = ceil(scene->project.duration / scene->physics.fixed_step) + 1.0;
    if (!isfinite(exact_samples) ||
        exact_samples > (double)(SIZE_MAX / sizeof(SrPhysicsSample) / 512)) {
        sr_diag_error(diag, 0, "physics", "fixedStep", "physics sample cache is too large");
        free(states); free(softs); return SR_ERR_MEMORY;
    }
    uint64_t samples = (uint64_t)exact_samples;
    uint64_t hash = signature(scene, states, count, softs, soft_count);
    if (load_cache(scene, states, count, softs, soft_count, samples, hash)) {
        sr_diag_info(diag, "loaded physics cache");
        free(states); free(softs); return SR_OK;
    }
    SoftState *soft_states = soft_count ? sr_alloc(soft_count * sizeof(*soft_states)) : NULL;
    bool ok = !soft_count || soft_states;
    for (size_t i = 0; ok && i < soft_count; ++i)
        ok = soft_init(scene, &soft_states[i], softs[i],
                       state_for(states, count, softs[i]));
    if (ok) ok = allocate_samples(states, count, softs, soft_count, samples);
    if (!ok) {
        release_samples(states, count, softs, soft_count);
        for (size_t i = 0; soft_states && i < soft_count; ++i) soft_free(&soft_states[i]);
        free(soft_states); free(states); free(softs);
        return SR_ERR_MEMORY;
    }
    double dt = scene->physics.fixed_step;
    for (uint64_t s = 0; s < samples; ++s) {
        for (size_t i = 0; i < count; ++i) {
            SrPhysicsSample *sample = &states[i].node->physics_samples[s];
            sample->enabled = true;
            sample->x.base = states[i].x;
            sample->y.base = states[i].y;
            sample->rotation.base = states[i].angle;
        }
        for (size_t i = 0; i < soft_count; ++i) {
            SrSoftBody *body = &softs[i]->soft_body;
            soft_record(&soft_states[i],
                        body->offsets + s * (size_t)body->rows * body->cols * 2);
        }
        if (s + 1 < samples) {
            double t = (double)s * dt;
            if (count) simulate_step(scene, states, count, t);
            for (size_t i = 0; i < soft_count; ++i) soft_step(scene, &soft_states[i], t);
        }
    }
    save_cache(scene, states, count, softs, soft_count, samples, hash, diag);
    sr_diag_info(diag, "simulated %llu fixed physics steps",
                 (unsigned long long)(samples - 1));
    for (size_t i = 0; i < soft_count; ++i) soft_free(&soft_states[i]);
    free(soft_states); free(states); free(softs);
    return SR_OK;
}

static void sample_span(const SrScene *scene, size_t sample_count, double time,
                        size_t *first, size_t *second, double *t) {
    double exact = fmax(0.0, time) / scene->physics.fixed_step;
    double floor_exact = floor(exact);
    *first = floor_exact >= (double)(sample_count - 1) ? sample_count - 1
                                                        : (size_t)floor_exact;
    *second = *first + 1 < sample_count ? *first + 1 : *first;
    *t = *second == *first ? 0 : exact - (double)*first;
}

bool sr_physics_pose(const SrScene *scene, const SrNode *node, double time,
                     double *x, double *y, double *rotation) {
    if (!scene->physics.enabled || !node->physics_sample_count) return false;
    size_t first, second;
    double t;
    sample_span(scene, node->physics_sample_count, time, &first, &second, &t);
    const SrPhysicsSample *a = &node->physics_samples[first];
    const SrPhysicsSample *b = &node->physics_samples[second];
    *x = a->x.base + (b->x.base - a->x.base) * t;
    *y = a->y.base + (b->y.base - a->y.base) * t;
    *rotation = a->rotation.base + (b->rotation.base - a->rotation.base) * t;
    return true;
}

bool sr_physics_soft_offsets(const SrScene *scene, const SrNode *node,
                             double time, double *out) {
    const SrSoftBody *soft = &node->soft_body;
    if (!soft->enabled || !soft->sample_count || !soft->offsets) return false;
    size_t first, second;
    double t;
    sample_span(scene, soft->sample_count, time, &first, &second, &t);
    size_t values = (size_t)soft->rows * soft->cols * 2;
    const double *a = soft->offsets + first * values;
    const double *b = soft->offsets + second * values;
    for (size_t i = 0; i < values; ++i) out[i] = a[i] + (b[i] - a[i]) * t;
    return true;
}
