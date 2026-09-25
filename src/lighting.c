#include "scene_render/lighting.h"
#include "scene_render/card.h"
#include "scene_render/color.h"
#include "scene_render/effects.h"
#include "scene_render/parallel.h"

#include <float.h>
#include <math.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

/* The 3D pass. Spheres, boxes and planes are camera-facing sprites shaded
 * per pixel; meshes are triangle-rasterized; everything shares one depth
 * buffer. Directional and spot lights with castShadow render a depth map
 * of every caster from the light (see ShadowMap) that receivers sample
 * with 3x3 percentage-closer filtering; point lights keep the bounding-
 * volume approximation. project.antialias3d = N renders the pass at N x N
 * samples per pixel and box-filters the result over the frame.
 *
 * Performance structure (all of it output-invariant):
 *  - Everything that depends only on the frame time (camera and object
 *    transforms and their sines and cosines, light directions, projected
 *    and clipped mesh triangles, object extents) is evaluated once in
 *    sr_lighting_begin with the same expressions the per-sample code used,
 *    so every value is bit-identical.
 *  - Each draw (one object, or all shadow blobs), each flush and each
 *    shadow map runs in parallel over disjoint row bands. A band replays
 *    the draw's primitives in order restricted to its rows, so every sample
 *    (texel) sees exactly the serial sequence of depth tests, shading and
 *    blends. Translucent fragments are kept per fixed tile of rows in
 *    generation order, so the sort at flush orders each pixel's fragments
 *    exactly as before.
 *  - The supersampled buffer, the pass-owned depth buffer and the shadow
 *    maps persist between passes (sr_lighting_release frees them). The
 *    supersampled buffer is kept all zero and the depth buffer all
 *    INFINITY between passes by clearing exactly the rectangles a pass
 *    touched, which is what freshly initialized buffers held. */

typedef struct { double x, y, z; } Vec3;
typedef struct { double c, s; } Trig;

static const SrCamera *scene_camera(const SrScene *scene) {
    for (size_t i = 0; i < scene->camera_count; ++i)
        if (scene->cameras[i].active) return &scene->cameras[i];
    return NULL;
}

static Trig trig(double a) { return (Trig){cos(a), sin(a)}; }

/* Rotations by a precomputed cos/sin: the expressions of rotate_*. */
static Vec3 rot_x(Vec3 p,Trig t){double c=t.c,s=t.s;return(Vec3){p.x,p.y*c-p.z*s,p.y*s+p.z*c};}
static Vec3 rot_y(Vec3 p,Trig t){double c=t.c,s=t.s;return(Vec3){p.x*c+p.z*s,p.y,-p.x*s+p.z*c};}
static Vec3 rot_z(Vec3 p,Trig t){double c=t.c,s=t.s;return(Vec3){p.x*c-p.y*s,p.x*s+p.y*c,p.z};}

static Vec3 rotate_x(Vec3 p,double a){return rot_x(p,trig(a));}
static Vec3 rotate_y(Vec3 p,double a){return rot_y(p,trig(a));}
static Vec3 rotate_z(Vec3 p,double a){return rot_z(p,trig(a));}

static Vec3 v_add(Vec3 a, Vec3 b) { return (Vec3){a.x + b.x, a.y + b.y, a.z + b.z}; }
static Vec3 v_sub(Vec3 a, Vec3 b) { return (Vec3){a.x - b.x, a.y - b.y, a.z - b.z}; }
static Vec3 v_scale(Vec3 a, double s) { return (Vec3){a.x * s, a.y * s, a.z * s}; }
static Vec3 v_cross(Vec3 a, Vec3 b) {
    return (Vec3){a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x};
}

static double clamp01(double value) {
    return fmax(0.0, fmin(1.0, value));
}
static Vec3 normalize(Vec3 value) {
    double length = sqrt(value.x * value.x + value.y * value.y + value.z * value.z);
    return length > 1e-12 ? (Vec3){value.x/length,value.y/length,value.z/length}
                          : (Vec3){0,0,1};
}
static double dot(Vec3 a, Vec3 b) { return a.x*b.x+a.y*b.y+a.z*b.z; }

static double object_alpha(const SrObject3D *object) {
    return object->material ? clamp01(object->material->base_color.a) : 1.0;
}

static double view_depth(const SrScene *scene,Vec3 point,double time){
    const SrCamera *camera=scene_camera(scene);if(!camera)return-point.z;
    point.x-=sr_anim_eval(&camera->x,time);point.y-=sr_anim_eval(&camera->y,time);
    point.z-=sr_anim_eval(&camera->z,time);
    point=rotate_y(point,-sr_anim_eval(&camera->yaw,time)*SR_PI/180.0);
    point=rotate_x(point,-sr_anim_eval(&camera->pitch,time)*SR_PI/180.0);
    point=rotate_z(point,-sr_anim_eval(&camera->roll,time)*SR_PI/180.0);
    return point.z;
}

/* ---- geometry frame -------------------------------------------------------
 * Shadows need consistent world geometry. Without a camera, world x and y
 * are screen pixels (y down) and the viewer looks down -z. With a camera,
 * world space is the one its transform maps to the screen. */

typedef struct {
    const SrCamera *camera;
    Vec3 eye;
    double yaw, pitch, roll;    /* radians */
    double focal;
    double width, height;
    /* cos/sin of -yaw, -pitch, -roll (world to camera) and of yaw, pitch,
     * roll (camera to world). The per-sample code negated the evaluated
     * degrees before scaling to radians; IEEE multiplication and division
     * are sign-symmetric, so those angles equal -yaw etc. bit for bit. */
    Trig to_yaw, to_pitch, to_roll;
    Trig from_yaw, from_pitch, from_roll;
    Vec3 toward;                /* unit vector toward the viewer */
} View;

static Vec3 to_camera(const View *view, Vec3 world) {
    Vec3 p = v_sub(world, view->eye);
    p = rot_y(p, view->to_yaw); p = rot_x(p, view->to_pitch);
    return rot_z(p, view->to_roll);
}

static Vec3 camera_dir_to_world(const View *view, Vec3 v) {
    v = rot_z(v, view->from_roll); v = rot_x(v, view->from_pitch);
    return rot_y(v, view->from_yaw);
}

static View view_init(const SrScene *scene, double time) {
    View view = {.camera = scene_camera(scene),
                 .width = scene->project.width, .height = scene->project.height};
    view.toward = (Vec3){0, 0, 1};
    if (view.camera) {
        const SrCamera *camera = view.camera;
        view.eye = (Vec3){sr_anim_eval(&camera->x, time), sr_anim_eval(&camera->y, time),
                          sr_anim_eval(&camera->z, time)};
        view.yaw = sr_anim_eval(&camera->yaw, time) * SR_PI / 180.0;
        view.pitch = sr_anim_eval(&camera->pitch, time) * SR_PI / 180.0;
        view.roll = sr_anim_eval(&camera->roll, time) * SR_PI / 180.0;
        view.focal = sr_camera_focal(scene, camera, time);
        view.to_yaw = trig(-view.yaw); view.to_pitch = trig(-view.pitch);
        view.to_roll = trig(-view.roll);
        view.from_yaw = trig(view.yaw); view.from_pitch = trig(view.pitch);
        view.from_roll = trig(view.roll);
        view.toward = camera_dir_to_world(&view, (Vec3){0, 0, -1});
    }
    return view;
}

/* The shading frame (x right, y up on screen, z toward the viewer) to the
 * geometry frame. */
static Vec3 shading_to_world(const View *view, Vec3 v) {
    if (!view->camera) return (Vec3){v.x, -v.y, v.z};
    return camera_dir_to_world(view, (Vec3){v.x, v.y, -v.z});
}

typedef struct {
    Vec3 center;                /* world */
    double sx, sy, sz;          /* |scale| */
    double angle;               /* -rotation, radians (sprite in-plane) */
} SpriteFrame;

static SpriteFrame sprite_frame(const SrObject3D *object, double time) {
    return (SpriteFrame){
        {sr_anim_eval(&object->transform.x, time), sr_anim_eval(&object->transform.y, time),
         sr_anim_eval(&object->transform.z, time)},
        fabs(sr_anim_eval(&object->transform.scale_x, time)),
        fabs(sr_anim_eval(&object->transform.scale_y, time)),
        fabs(sr_anim_eval(&object->transform.scale_z, time)),
        -sr_anim_eval(&object->transform.rotation, time) * SR_PI / 180.0};
}

/* ---- per-pass object state ----------------------------------------------- */

typedef struct { double x,y,depth; Vec3 world,normal,camera; } MeshVertex;

/* A fan triangle of a clipped mesh triangle, as render_triangle prepared
 * it: winding normalized, bounds clamped to the target. */
typedef struct {
    MeshVertex v[3];
    double area;
    Vec3 face;
    int min_x, max_x, min_y, max_y;
    bool owns_a, owns_b, owns_c;
} RasterTri;

typedef struct { Vec3 v[3]; } WorldTri;

typedef struct { int x0, y0, x1, y1; } Rect;   /* mark() arguments */

typedef struct {
    const SrObject3D *object;
    const SrMaterial *material;
    double alpha;
    SpriteFrame sprite;
    Trig angle;                 /* cos/sin of sprite.angle */
    double bound;               /* object_bound */
    double extent;              /* bounding sphere radius (object_extent) */
    Vec3 cam_center;            /* to_camera(center) with a camera */
    /* shade() material invariants. */
    double exponent, spec_scale;
    bool zero_safe;
    /* project() of the center: sprite and blob. */
    bool projected;
    double cx, cy, pscale;
    double rx, ry;              /* sprite radii */
    int sx0, sx1, sy0, sy1;     /* sprite sample loop bounds (may be -1..size) */
    double bx, by, bdx, bdy;    /* blob center and fmax'd radii */
    int bx0, bx1, by0, by1;     /* blob loop bounds (may be -1..size) */
    /* Mesh. */
    const WorldTri *world;      /* every triangle (shadow casters) */
    size_t world_count;
    const RasterTri *tris;      /* drawn fan triangles in order */
    size_t tri_count;
    bool mesh_marked;
    Rect mesh_mark;             /* union of the fan triangles' marks */
} ObjectFrame;

static const SrMaterial fallback_material = {.base_color = {0.7, 0.7, 0.7, 1},
                                             .roughness = .5};

/* ---- shadow maps --------------------------------------------------------- */

typedef struct {
    bool spot;
    int size;
    float *depth;               /* distance along each texel's ray */
    Vec3 dir, u, v;             /* travel direction and map axes */
    Vec3 center;                /* directional: scene bound center */
    double half, texel;         /* directional: half extent, texel size */
    Vec3 origin;                /* spot: light position */
    double tan_half;            /* spot: tangent of the map half angle */
} ShadowMap;

static void basis(Vec3 dir, Vec3 *u, Vec3 *v) {
    Vec3 up = fabs(dir.y) < 0.99 ? (Vec3){0, 1, 0} : (Vec3){1, 0, 0};
    *u = normalize(v_cross(up, dir));
    *v = v_cross(dir, *u);
}

static void texel_ray(const ShadowMap *map, int i, int j, Vec3 *origin, Vec3 *dir) {
    if (map->spot) {
        double a = (2.0 * (i + 0.5) / map->size - 1.0) * map->tan_half;
        double b = (1.0 - 2.0 * (j + 0.5) / map->size) * map->tan_half;
        *origin = map->origin;
        *dir = normalize(v_add(map->dir, v_add(v_scale(map->u, a), v_scale(map->v, b))));
    } else {
        double a = -map->half + (i + 0.5) * map->texel;
        double b = map->half - (j + 0.5) * map->texel;
        *origin = v_add(map->center, v_add(v_add(v_scale(map->u, a), v_scale(map->v, b)),
                                           v_scale(map->dir, -2.0 * map->half)));
        *dir = map->dir;
    }
}

/* Map coordinates (texel units) and ray depth of world point p. */
static bool map_coords(const ShadowMap *map, Vec3 p, double *fi, double *fj,
                       double *depth, double *texel_world) {
    if (map->spot) {
        Vec3 q = v_sub(p, map->origin);
        double z = dot(q, map->dir);
        if (z <= 1e-9) return false;
        *fi = (dot(q, map->u) / (z * map->tan_half) + 1.0) * 0.5 * map->size;
        *fj = (1.0 - dot(q, map->v) / (z * map->tan_half)) * 0.5 * map->size;
        *depth = sqrt(dot(q, q));
        *texel_world = 2.0 * z * map->tan_half / map->size;
    } else {
        Vec3 q = v_sub(p, map->center);
        *fi = (dot(q, map->u) + map->half) / map->texel;
        *fj = (map->half - dot(q, map->v)) / map->texel;
        *depth = dot(q, map->dir) + 2.0 * map->half;
        *texel_world = map->texel;
    }
    return true;
}

bool sr_light_cone_slopes(double a, double z, double r, double *low,
                          double *high) {
    double rho = hypot(a, z);
    if (!(z > r) || !(r >= 0.0) || !(rho > r)) return false;
    double beta = atan2(a, z), delta = asin(r / rho);
    *low = tan(beta - delta);
    *high = tan(beta + delta);
    return isfinite(*low) && isfinite(*high);
}

/* Texel rectangle covering a bounding sphere, clamped to the map. A spot
 * map uses the exact tangent cone of the sphere (sr_light_cone_slopes)
 * per axis, plus one texel of padding. */
static void sphere_texels(const ShadowMap *map, Vec3 c, double r, int *i0,
                          int *j0, int *i1, int *j1) {
    *i0 = 0; *j0 = 0; *i1 = map->size; *j1 = map->size;
    double u0, u1, v0, v1;
    if (map->spot) {
        Vec3 q = v_sub(c, map->origin);
        double z = dot(q, map->dir);
        if (z - r <= 1e-6) return;
        double lo_u, hi_u, lo_v, hi_v;
        if (!sr_light_cone_slopes(dot(q, map->u), z, r, &lo_u, &hi_u) ||
            !sr_light_cone_slopes(dot(q, map->v), z, r, &lo_v, &hi_v)) return;
        u0 = (lo_u / map->tan_half + 1.0) * 0.5 * map->size;
        u1 = (hi_u / map->tan_half + 1.0) * 0.5 * map->size;
        v0 = (1.0 - hi_v / map->tan_half) * 0.5 * map->size;
        v1 = (1.0 - lo_v / map->tan_half) * 0.5 * map->size;
    } else {
        Vec3 q = v_sub(c, map->center);
        double u = dot(q, map->u), v = dot(q, map->v);
        u0 = (u - r + map->half) / map->texel; u1 = (u + r + map->half) / map->texel;
        v0 = (map->half - v - r) / map->texel; v1 = (map->half - v + r) / map->texel;
    }
    *i0 = sr_clamp_int(floor(u0) - 1.0, 0, map->size);
    *i1 = sr_clamp_int(ceil(u1) + 1.0, 0, map->size);
    *j0 = sr_clamp_int(floor(v0) - 1.0, 0, map->size);
    *j1 = sr_clamp_int(ceil(v1) + 1.0, 0, map->size);
}

static void map_store(const ShadowMap *map, int i, int j, double t) {
    float *slot = &map->depth[(size_t)j * map->size + i];
    if (t > 0.0 && t < *slot) *slot = (float)t;
}

/* Nearest ray parameter hitting the camera-aligned ellipsoid or the
 * camera-facing sprite quad of `of`, or -1. The semi-axes come
 * precomputed, `az` already defaulted for spheres, and ax, ay > 0. */
static double sprite_hit(const View *view, const ObjectFrame *of, double ax,
                         double ay, double az, Vec3 origin, Vec3 dir) {
    const SpriteFrame *frame = &of->sprite;
    if (of->object->primitive == SR_OBJECT_SPHERE) {
        Vec3 o = v_sub(origin, frame->center), d = dir;
        if (view->camera) {
            o = to_camera(view, origin);
            o = v_sub(o, of->cam_center);
            d = rot_z(rot_x(rot_y(dir, view->to_yaw), view->to_pitch), view->to_roll);
        }
        o = (Vec3){o.x / ax, o.y / ay, o.z / az};
        d = (Vec3){d.x / ax, d.y / ay, d.z / az};
        double a = dot(d, d), b = dot(o, d), c = dot(o, o) - 1.0;
        double disc = b * b - a * c;
        if (disc < 0.0 || a <= 0.0) return -1.0;
        double root = sqrt(disc);
        double t = (-b - root) / a;
        if (t <= 0.0) t = (-b + root) / a;
        return t > 0.0 ? t : -1.0;
    }
    Vec3 normal = view->toward;
    double denom = dot(dir, normal);
    if (fabs(denom) < 1e-9) return -1.0;
    double t = dot(v_sub(frame->center, origin), normal) / denom;
    if (t <= 0.0) return -1.0;
    Vec3 hit = v_add(origin, v_scale(dir, t));
    double nx, ny;
    if (view->camera) {
        Vec3 h = to_camera(view, hit), c = of->cam_center;
        nx = (h.x - c.x) / ax; ny = -(h.y - c.y) / ay;
    } else {
        nx = (hit.x - frame->center.x) / ax; ny = (hit.y - frame->center.y) / ay;
    }
    /* The sprite loop spans the unrotated box, so a rotated quad is drawn
     * clipped to it; cast the same footprint. */
    if (fabs(nx) > 1.0 || fabs(ny) > 1.0) return -1.0;
    double rx = nx * of->angle.c - ny * of->angle.s;
    double ry = nx * of->angle.s + ny * of->angle.c;
    return fabs(rx) <= 1.0 && fabs(ry) <= 1.0 ? t : -1.0;
}

static double triangle_hit(Vec3 origin, Vec3 dir, Vec3 a, Vec3 b, Vec3 c) {
    Vec3 e1 = v_sub(b, a), e2 = v_sub(c, a), p = v_cross(dir, e2);
    double det = dot(e1, p);
    if (fabs(det) < 1e-12) return -1.0;
    Vec3 s = v_sub(origin, a);
    double u = dot(s, p) / det;
    if (u < 0.0 || u > 1.0) return -1.0;
    Vec3 q = v_cross(s, e1);
    double v = dot(dir, q) / det;
    if (v < 0.0 || u + v > 1.0) return -1.0;
    double t = dot(e2, q) / det;
    return t > 0.0 ? t : -1.0;
}

/* ---- row bands ------------------------------------------------------------
 * A parallel loop over units [lo, lo + count) (rows or row tiles) uses
 * workers x per bands. sr_parallel_for hands each worker a contiguous item
 * range; item k runs band (k % per) * workers + k / per, spreading each
 * worker's bands over the whole range for load balance. */

typedef struct {
    int lo, count;
    unsigned workers;
    int per, bands;
} BandPlan;

static BandPlan band_plan(int lo, int count, unsigned threads) {
    BandPlan plan = {.lo = lo, .count = count};
    plan.workers = sr_parallel_thread_count(threads, (size_t)(count > 0 ? count : 0));
    if (plan.workers < 1) plan.workers = 1;
    int per = count / (int)plan.workers;
    if (per > 8) per = 8;
    if (per < 1) per = 1;
    plan.per = per;
    plan.bands = (int)plan.workers * per;
    if (plan.bands > count) plan.bands = count;
    return plan;
}

static void band_units(const BandPlan *plan, size_t k, int *u0, int *u1) {
    size_t band = plan->bands == (int)plan->workers * plan->per
        ? (k % (size_t)plan->per) * plan->workers + k / (size_t)plan->per : k;
    *u0 = plan->lo + (int)((long long)plan->count * (long long)band / plan->bands);
    *u1 = plan->lo + (int)((long long)plan->count * (long long)(band + 1) / plan->bands);
}

/* ---- shadow-map construction --------------------------------------------- */

typedef struct {
    const ObjectFrame *of;
    const WorldTri *tri;        /* NULL: sprite caster */
    double ax, ay, az;
    int i0, i1, j0, j1;         /* texel rectangle, exclusive ends */
} Caster;

typedef struct {
    const View *view;
    const ShadowMap *map;
    const Caster *casters;
    size_t count;
    BandPlan plan;
} MapJob;

static void map_rows(const MapJob *job, int lo, int hi) {
    const ShadowMap *map = job->map;
    float *row = map->depth + (size_t)lo * map->size;
    for (size_t i = 0, n = (size_t)(hi - lo) * map->size; i < n; ++i) row[i] = INFINITY;
    for (size_t e = 0; e < job->count; ++e) {
        const Caster *caster = &job->casters[e];
        int j0 = caster->j0 > lo ? caster->j0 : lo;
        int j1 = caster->j1 < hi ? caster->j1 : hi;
        for (int j = j0; j < j1; ++j) for (int i = caster->i0; i < caster->i1; ++i) {
            Vec3 origin, dir;
            texel_ray(map, i, j, &origin, &dir);
            double t = caster->tri
                ? triangle_hit(origin, dir, caster->tri->v[0], caster->tri->v[1],
                               caster->tri->v[2])
                : sprite_hit(job->view, caster->of, caster->ax, caster->ay,
                             caster->az, origin, dir);
            map_store(map, i, j, t);
        }
    }
}

static void map_worker(void *opaque, size_t begin, size_t end) {
    const MapJob *job = opaque;
    for (size_t k = begin; k < end; ++k) {
        int lo, hi;
        band_units(&job->plan, k, &lo, &hi);
        map_rows(job, lo, hi);
    }
}

/* Grow-only float storage; contents are not preserved. */
static bool reserve_floats(float **buffer, size_t *capacity, size_t count) {
    if (*buffer && count <= *capacity) return true;
    free(*buffer);
    *buffer = NULL;
    *capacity = 0;
    if (count > SIZE_MAX / sizeof(float)) return false;
    *buffer = malloc((count ? count : 1) * sizeof(float));
    if (!*buffer) return false;
    *capacity = count;
    return true;
}

static SrStatus map_build(const SrScene *scene, const View *view,
                          const ObjectFrame *objects, const SrLight *light,
                          double time, unsigned threads, float **storage,
                          size_t *storage_cap, ShadowMap *map) {
    *map = (ShadowMap){.spot = light->type == SR_LIGHT_SPOT,
                       .size = (int)light->shadow_map_size};
    double yaw = sr_anim_eval(&light->yaw, time) * SR_PI / 180.0;
    double pitch = sr_anim_eval(&light->pitch, time) * SR_PI / 180.0;
    /* The shading model's direction toward the light, in the shading
     * frame; light travels the opposite way. */
    Vec3 toward = normalize((Vec3){-sin(yaw)*cos(pitch), sin(pitch), cos(yaw)*cos(pitch)});
    map->dir = normalize(v_scale(shading_to_world(view, toward), -1.0));
    basis(map->dir, &map->u, &map->v);
    if (map->spot) {
        map->origin = (Vec3){sr_anim_eval(&light->x, time), sr_anim_eval(&light->y, time),
                             sr_anim_eval(&light->z, time)};
        map->tan_half = tan(fmin(170.0, light->spot_angle * 1.1) * 0.5 * SR_PI / 180.0);
    } else {
        Vec3 low = {DBL_MAX, DBL_MAX, DBL_MAX}, high = {-DBL_MAX, -DBL_MAX, -DBL_MAX};
        for (size_t i = 0; i < scene->object3d_count; ++i) {
            Vec3 c = objects[i].sprite.center;
            double r = objects[i].extent;
            low = (Vec3){fmin(low.x, c.x - r), fmin(low.y, c.y - r), fmin(low.z, c.z - r)};
            high = (Vec3){fmax(high.x, c.x + r), fmax(high.y, c.y + r), fmax(high.z, c.z + r)};
        }
        map->center = v_scale(v_add(low, high), 0.5);
        double half = 0.0;
        for (size_t i = 0; i < scene->object3d_count; ++i) {
            Vec3 d = v_sub(objects[i].sprite.center, map->center);
            half = fmax(half, sqrt(dot(d, d)) + objects[i].extent);
        }
        map->half = fmax(half, 1e-6);
        map->texel = 2.0 * map->half / map->size;
    }
    size_t texels = (size_t)map->size * map->size;
    if (!reserve_floats(storage, storage_cap, texels)) return SR_ERR_MEMORY;
    map->depth = *storage;
    if (texels == 0) return SR_OK;

    /* Casters in scene order: sprites, and each mesh triangle. */
    size_t count = 0;
    for (size_t index = 0; index < scene->object3d_count; ++index) {
        const ObjectFrame *of = &objects[index];
        if (!of->object->cast_shadow || of->alpha == 0.0) continue;
        count += of->object->primitive == SR_OBJECT_MESH ? of->world_count : 1;
    }
    Caster *casters = count ? sr_alloc(count * sizeof(*casters)) : NULL;
    if (count && !casters) return SR_ERR_MEMORY;
    size_t c = 0;
    for (size_t index = 0; index < scene->object3d_count; ++index) {
        const ObjectFrame *of = &objects[index];
        const SrObject3D *object = of->object;
        if (!object->cast_shadow || of->alpha == 0.0) continue;
        if (object->primitive == SR_OBJECT_MESH) {
            for (size_t t = 0; t < of->world_count; ++t) {
                const WorldTri *tri = &of->world[t];
                double lo_i = DBL_MAX, hi_i = -DBL_MAX, lo_j = DBL_MAX, hi_j = -DBL_MAX;
                bool whole = false;
                for (int k = 0; k < 3; ++k) {
                    double fi, fj, depth, texel;
                    if (!map_coords(map, tri->v[k], &fi, &fj, &depth, &texel)) {
                        whole = true; continue;
                    }
                    lo_i = fmin(lo_i, fi); hi_i = fmax(hi_i, fi);
                    lo_j = fmin(lo_j, fj); hi_j = fmax(hi_j, fj);
                }
                Caster *out = &casters[c++];
                *out = (Caster){.of = of, .tri = tri};
                out->i0 = whole ? 0 : sr_clamp_int(floor(lo_i) - 1.0, 0, map->size);
                out->i1 = whole ? map->size : sr_clamp_int(ceil(hi_i) + 1.0, 0, map->size);
                out->j0 = whole ? 0 : sr_clamp_int(floor(lo_j) - 1.0, 0, map->size);
                out->j1 = whole ? map->size : sr_clamp_int(ceil(hi_j) + 1.0, 0, map->size);
            }
            continue;
        }
        Caster *out = &casters[c];
        *out = (Caster){.of = of};
        out->ax = object->radius * of->sprite.sx;
        out->ay = object->radius * of->sprite.sy;
        out->az = object->radius * of->sprite.sz;
        if (object->primitive == SR_OBJECT_SPHERE && out->az <= 0.0)
            out->az = fmax(out->ax, out->ay);
        sphere_texels(map, of->sprite.center, of->extent, &out->i0, &out->j0,
                      &out->i1, &out->j1);
        /* Degenerate semi-axes never hit: every texel would store -1. */
        if (out->ax <= 0.0 || out->ay <= 0.0) continue;
        ++c;
    }
    MapJob job = {.view = view, .map = map, .casters = casters, .count = c,
                  .plan = band_plan(0, map->size, threads)};
    SrStatus status = sr_parallel_for((size_t)job.plan.bands, threads, map_worker, &job);
    free(casters);
    return status;
}

/* Fraction of the light reaching world point p (unit normal n): 3 x 3
 * percentage-closer filter with a slope-scaled bias of 1.5 texels plus 2
 * per unit of surface tangent (at most 10). Outside the map: lit. */
static double map_visibility(const ShadowMap *map, Vec3 p, Vec3 n) {
    double fi, fj, depth, texel;
    if (!map_coords(map, p, &fi, &fj, &depth, &texel)) return 1.0;
    Vec3 dir = map->spot ? normalize(v_sub(p, map->origin)) : map->dir;
    double ndl = fmax(1e-3, fabs(dot(n, dir)));
    double tangent = sqrt(fmax(0.0, 1.0 - ndl * ndl)) / ndl;
    double biased = depth - texel * (1.5 + 2.0 * fmin(tangent, 10.0));
    /* Taps beyond the map are lit, so clamping just outside it is exact. */
    int ci = sr_clamp_int(floor(fi), -2, map->size + 1);
    int cj = sr_clamp_int(floor(fj), -2, map->size + 1), lit = 0;
    for (int dj = -1; dj <= 1; ++dj) for (int di = -1; di <= 1; ++di) {
        int i = ci + di, j = cj + dj;
        if (i < 0 || j < 0 || i >= map->size || j >= map->size ||
            biased <= map->depth[(size_t)j * map->size + i])
            ++lit;
    }
    return lit / 9.0;
}

/* ---- persistent buffers --------------------------------------------------- */

typedef struct {
    size_t pixel, order;
    double depth;
    float color[4];
} Fragment;

/* Translucent fragments of one tile of TILE_ROWS target rows, in
 * generation order (`order` counts within the tile). */
typedef struct {
    Fragment *data;
    size_t count, capacity;
} FragmentTile;

enum { TILE_ROWS = 4 };

typedef struct {
    float *samples; size_t samples_cap;     /* all zero between passes */
    double *depth; size_t depth_cap;        /* all INFINITY between passes */
    float **maps; size_t *map_caps; size_t map_slots;
    FragmentTile *tiles; size_t tile_count; /* counts zero between passes */
} Scratch;

static Scratch shared_scratch;
static pthread_mutex_t shared_scratch_lock = PTHREAD_MUTEX_INITIALIZER;

static void scratch_free(Scratch *s) {
    free(s->samples); free(s->depth);
    for (size_t i = 0; i < s->map_slots; ++i) free(s->maps[i]);
    free(s->maps); free(s->map_caps);
    for (size_t i = 0; i < s->tile_count; ++i) free(s->tiles[i].data);
    free(s->tiles);
    *s = (Scratch){0};
}

void sr_lighting_release(void) {
    pthread_mutex_lock(&shared_scratch_lock);
    scratch_free(&shared_scratch);
    pthread_mutex_unlock(&shared_scratch_lock);
}

/* ---- shading ------------------------------------------------------------- */

/* Per-light values shade() evaluated per sample, with the same
 * expressions. */
typedef struct {
    SrColor color;
    double intensity;
    Vec3 direction, halfv;      /* directional */
    Vec3 position;              /* point, spot */
    double range, falloff;      /* fmax(range, 1e-9), fmax(falloff, 0) */
    Vec3 aim;                   /* spot */
    double cone, cone_span;     /* spot: cos, fmax(1 - cos, 1e-9) */
    bool zero_safe;             /* intensity and color finite */
} LightFrame;

/* A point-shadow caster (cast_shadow, visible): center and bound. */
typedef struct {
    const SrObject3D *object;
    Vec3 center;
    double bound;
} PointCaster;

typedef struct {
    const SrScene *scene;
    double time;
    unsigned threads;
    View view;
    LightFrame *lights;         /* per light */
    ShadowMap *maps;            /* per light; depth NULL when unused */
    bool any_map;
    PointCaster *casters;
    size_t caster_count;
    ObjectFrame *objects;
    bool blob;                  /* point-light screen-space shadow blobs */
    float blob_source[4];       /* the blob's blend-space color */
    int samples;                /* antialias3d */
    SrFrame *target;            /* frame, or the supersampled buffer */
    double *depth;
    int *dirty;                 /* [x0, y0, x1, y1) touched target samples */
    int touched[4];             /* the same since begin (depth reset) */
    bool translucent;
    FragmentTile *tiles;
    size_t tile_count;
    bool fragments;             /* some tile may hold fragments */
    SrStatus status;
} Pass;

static void mark_rect(int *d, int w, int h, int x0, int y0, int x1, int y1) {
    x0 = x0 < 0 ? 0 : x0; y0 = y0 < 0 ? 0 : y0;
    x1 = x1 > w ? w : x1; y1 = y1 > h ? h : y1;
    if (x1 <= x0 || y1 <= y0) return;
    if (d[2] <= d[0] || d[3] <= d[1]) { d[0] = x0; d[1] = y0; d[2] = x1; d[3] = y1; return; }
    if (x0 < d[0]) d[0] = x0;
    if (y0 < d[1]) d[1] = y0;
    if (x1 > d[2]) d[2] = x1;
    if (y1 > d[3]) d[3] = y1;
}

static void mark(Pass *pass, int x0, int y0, int x1, int y1) {
    int w = (int)pass->target->width, h = (int)pass->target->height;
    mark_rect(pass->dirty, w, h, x0, y0, x1, y1);
    mark_rect(pass->touched, w, h, x0, y0, x1, y1);
}

/* Point lights: a caster's bounding sphere on the segment toward the light
 * darkens the receiver (the pre-shadow-map approximation). */
static bool shadowed(const Pass *pass,const SrObject3D *receiver,
                     Vec3 point,Vec3 direction,double maximum){
    if(!receiver->receive_shadow)return false;
    for(size_t i=0;i<pass->caster_count;++i){const PointCaster *caster=&pass->casters[i];
        if(caster->object==receiver)continue;
        Vec3 center=caster->center;
        Vec3 delta={center.x-point.x,center.y-point.y,center.z-point.z};
        double along=dot(delta,direction);
        if(along<=1e-5||along>=maximum)continue;
        double distance2=dot(delta,delta)-along*along;
        double radius=caster->bound;
        if(distance2<radius*radius)return true;
    }
    return false;
}

/* `geometry` is the receiver's consistent world point and normal for
 * shadow-map lookups (NULL: unshadowed by maps).
 *
 * A light whose energy (intensity x attenuation) is zero adds
 * color * 0 * (diffuse + specular), a zero, when all those terms are
 * finite (zero_safe on the light and the material; attenuation always lies
 * in [0, 1] and diffuse and the specular base in [0, 1]); r, g and b start
 * at a finite emissive that is not -0 and so never are -0, and adding a
 * zero leaves them unchanged. Such lights skip the shadow test and the
 * specular power. pow(+0, e) is +0 for every e > 0. */
static SrColor shade(const Pass *pass, const ObjectFrame *of,
                     Vec3 point, Vec3 normal, const Vec3 geometry[2]) {
    const SrScene *scene = pass->scene;
    const SrObject3D *object = of->object;
    const SrMaterial *material = of->material;
    double r=material->emissive.r,g=material->emissive.g,b=material->emissive.b;
    Vec3 view={0,0,1};
    for(size_t i=0;i<scene->light_count;++i){const SrLight *light=&scene->lights[i];
        if (light->used_2d) continue;
        const LightFrame *lf = &pass->lights[i];
        bool zero_skip = lf->zero_safe && of->zero_safe;
        if (lf->intensity == 0.0 && zero_skip) continue;
        SrColor color = lf->color;
        double intensity=lf->intensity;
        Vec3 direction={0,0,1};double attenuation=1.0,maximum=INFINITY;
        if(light->type==SR_LIGHT_AMBIENT){r+=color.r*intensity;g+=color.g*intensity;b+=color.b*intensity;continue;}
        if(light->type==SR_LIGHT_DIRECTIONAL){direction=lf->direction;
        }else{Vec3 delta={lf->position.x-point.x,lf->position.y-point.y,lf->position.z-point.z};
            double distance=sqrt(dot(delta,delta));maximum=distance;direction=normalize(delta);
            attenuation=pow(clamp01(1.0-distance/lf->range),lf->falloff);
            if(light->type==SR_LIGHT_SPOT){
                Vec3 from_light={-direction.x,-direction.y,-direction.z};
                attenuation*=clamp01((dot(from_light,lf->aim)-lf->cone)/lf->cone_span);}}
        if (attenuation == 0.0 && zero_skip) continue;
        if (light->cast_shadow) {
            if (pass->maps[i].depth) {
                if (geometry && object->receive_shadow && attenuation > 0.0)
                    attenuation *= map_visibility(&pass->maps[i], geometry[0], geometry[1]);
            } else if (light->type == SR_LIGHT_POINT &&
                       shadowed(pass,object,point,direction,maximum)) {
                attenuation*=0.2;
            }
        }
        double diffuse=fmax(0.0,dot(normal,direction));
        Vec3 halfv=light->type==SR_LIGHT_DIRECTIONAL ? lf->halfv
            : normalize((Vec3){direction.x+view.x,direction.y+view.y,direction.z+view.z});
        double facing=fmax(0.0,dot(normal,halfv));
        double power=facing==0.0&&!signbit(facing)&&of->exponent>0.0
            ? 0.0 : pow(facing,of->exponent);
        double spec=power*of->spec_scale;
        double energy=intensity*attenuation;
        r+=color.r*energy*(material->base_color.r*diffuse+spec);
        g+=color.g*energy*(material->base_color.g*diffuse+spec);
        b+=color.b*energy*(material->base_color.b*diffuse+spec);
    }
    return (SrColor){clamp01(r),clamp01(g),clamp01(b),material->base_color.a};
}

static bool finite_vec(Vec3 v) {
    return isfinite(v.x) && isfinite(v.y) && isfinite(v.z);
}

/* ---- sample stores -------------------------------------------------------- */

/* The last color converted by sr_color_to_blend (a pure function) in one
 * band of one draw: a bit-identical input reuses its output. */
typedef struct {
    bool valid;
    SrColor in;
    float out[4];
} BlendMemo;

static const float *to_blend(const SrProject *project, SrColor color, BlendMemo *memo) {
    if (!memo->valid || memcmp(&memo->in, &color, sizeof(color)) != 0) {
        sr_color_to_blend(project, color, memo->out);
        memo->in = color;
        memo->valid = true;
    }
    return memo->out;
}

/* One band of one draw. */
typedef struct {
    const Pass *pass;
    BlendMemo memo;
    bool failed;
} Band;

/* Opaque geometry establishes visibility first. Translucent fragments keep
 * their own depths and blend far-to-near afterwards, including intersecting
 * meshes; they never hide subsequently visited geometry by writing depth. */
static void store_sample(Band *band, int y, size_t pixel, double depth, SrColor color) {
    const Pass *pass = band->pass;
    if (!(color.a > 0.0) || !isfinite(depth) || depth >= pass->depth[pixel]) return;
    const float *source = to_blend(&pass->scene->project, color, &band->memo);
    if (!pass->translucent) {
        pass->depth[pixel] = depth;
        sr_blend_px(SR_BLEND_NORMAL, pass->target->px + pixel * 4, source);
        return;
    }
    FragmentTile *tile = &pass->tiles[y / TILE_ROWS];
    if (tile->count == tile->capacity) {
        size_t capacity = tile->capacity ? tile->capacity * 2 : 256;
        if (capacity < tile->capacity || capacity > SIZE_MAX / sizeof(Fragment)) {
            band->failed = true;
            return;
        }
        Fragment *grown = sr_realloc(tile->data, capacity * sizeof(*grown));
        if (!grown) { band->failed = true; return; }
        tile->data = grown;
        tile->capacity = capacity;
    }
    Fragment *fragment = &tile->data[tile->count];
    *fragment = (Fragment){.pixel = pixel, .order = tile->count, .depth = depth};
    memcpy(fragment->color, source, 4 * sizeof(float));
    ++tile->count;
}

static int compare_fragments(const void *a, const void *b) {
    const Fragment *fa = a, *fb = b;
    if (fa->pixel != fb->pixel) return fa->pixel < fb->pixel ? -1 : 1;
    if (fa->depth != fb->depth) return fa->depth > fb->depth ? -1 : 1;
    return (fa->order > fb->order) - (fa->order < fb->order);
}

static double edge(double ax,double ay,double bx,double by,double px,double py){
    return(px-ax)*(by-ay)-(py-ay)*(bx-ax);
}

/* Screen coordinate of supersample index X (pixel centers at N = 1). */
static double sample_at(int index, int samples) {
    return (index + .5) / samples;
}

/* Half-open edge ownership prevents a translucent mesh (or a clipped fan)
 * from blending twice at a shared edge. Winding is normalized beforehand. */
static bool owns_edge(const MeshVertex *a, const MeshVertex *b) {
    return b->y > a->y || (b->y == a->y && b->x < a->x);
}

/* Rows [y0, y1] of a prepared fan triangle. */
static void draw_triangle(Band *band, const ObjectFrame *of, const RasterTri *tri,
                          int y0, int y1) {
    const Pass *pass = band->pass;
    const MeshVertex *v = tri->v;
    double area = tri->area;
    SrFrame *frame = pass->target;
    int n = pass->samples;
    bool perspective = pass->view.camera && !pass->view.camera->orthographic;
    bool owns_a = tri->owns_a, owns_b = tri->owns_b, owns_c = tri->owns_c;
    for (int y = y0; y <= y1 && !band->failed; ++y) {
        for (int x = tri->min_x; x <= tri->max_x && !band->failed; ++x) {
            double sx = sample_at(x, n), sy = sample_at(y, n);
            double ea = edge(v[1].x, v[1].y, v[2].x, v[2].y, sx, sy);
            double eb = edge(v[2].x, v[2].y, v[0].x, v[0].y, sx, sy);
            double ec = edge(v[0].x, v[0].y, v[1].x, v[1].y, sx, sy);
            if (ea < 0.0 || (ea == 0.0 && !owns_a) ||
                eb < 0.0 || (eb == 0.0 && !owns_b) ||
                ec < 0.0 || (ec == 0.0 && !owns_c)) continue;
            double a = ea / area, b = eb / area, c = ec / area;
            double z;
            if (perspective) {
                a /= v[0].depth; b /= v[1].depth; c /= v[2].depth;
                z = 1.0 / (a + b + c);
                a *= z; b *= z; c *= z;
            } else {
                z = a * v[0].depth + b * v[1].depth + c * v[2].depth;
            }
            size_t at = (size_t)y * frame->width + (size_t)x;
            if (!isfinite(z) || z >= pass->depth[at]) continue;
            Vec3 point = {a*v[0].world.x+b*v[1].world.x+c*v[2].world.x,
                          a*v[0].world.y+b*v[1].world.y+c*v[2].world.y,
                          a*v[0].world.z+b*v[1].world.z+c*v[2].world.z};
            Vec3 normal = normalize((Vec3){a*v[0].normal.x+b*v[1].normal.x+c*v[2].normal.x,
                a*v[0].normal.y+b*v[1].normal.y+c*v[2].normal.y,
                a*v[0].normal.z+b*v[1].normal.z+c*v[2].normal.z});
            Vec3 geometry[2] = {point, tri->face};
            store_sample(band, y, at, z, shade(pass, of, point, normal, geometry));
        }
    }
}

/* Rows [y0, y1] (within the target) of the point-light screen blob under
 * a caster. Pixels beyond the target were skipped by over(), so the
 * columns are clamped to it. */
static void draw_blob(const Pass *pass, const ObjectFrame *of, int y0, int y1) {
    SrFrame *target = pass->target;
    int n = pass->samples;
    double x = of->bx, y = of->by;
    int x0 = of->bx0 < 0 ? 0 : of->bx0;
    int x1 = of->bx1 > (int)target->width - 1 ? (int)target->width - 1 : of->bx1;
    for(int py=y0;py<=y1;++py)for(int px=x0;px<=x1;++px){
        double dx=(px/(double)n-x)/of->bdx,dy=(py/(double)n-y)/of->bdy;
        if(dx*dx+dy*dy<=1.0)
            sr_blend_px(SR_BLEND_NORMAL,
                        &target->px[((size_t)py*target->width+(size_t)px)*4],
                        pass->blob_source);
    }
}

/* World point and outward normal of a sprite pixel sample at screen
 * (sx, sy); (nx, ny) are its normalized (rotated) sprite coordinates. */
static void sprite_surface(const View *view, const ObjectFrame *of, double sx,
                           double sy, double nx, double ny, Vec3 *point,
                           Vec3 *normal) {
    const SrObject3D *object = of->object;
    const SpriteFrame *frame = &of->sprite;
    double bulge = object->primitive == SR_OBJECT_SPHERE
        ? sqrt(fmax(0.0, 1.0 - nx * nx - ny * ny)) : 0.0;
    double depth = bulge * object->radius * frame->sz;
    if (!view->camera) {
        *point = (Vec3){sx, sy, frame->center.z + depth};
    } else {
        Vec3 c = of->cam_center;
        Vec3 local = {c.x + (sx - of->cx) / of->pscale, c.y - (sy - of->cy) / of->pscale,
                      c.z - depth};
        *point = v_add(view->eye, camera_dir_to_world(view, local));
    }
    *normal = object->primitive == SR_OBJECT_SPHERE
        ? normalize(v_sub(*point, frame->center)) : view->toward;
}

/* Rows [y0, y1] (within the target) of a camera-facing sprite. */
static void draw_sprite(Band *band, const ObjectFrame *of, int y0, int y1) {
    const Pass *pass = band->pass;
    const SrObject3D *object = of->object;
    SrFrame *frame = pass->target;
    int n = pass->samples;
    double world_x=of->sprite.center.x,world_y=of->sprite.center.y,cz=of->sprite.center.z;
    double cx=of->cx,cy=of->cy,rx=of->rx,ry=of->ry;
    double ca=of->angle.c,sa=of->angle.s;
    bool maps = pass->any_map;
    int x0 = of->sx0 < 0 ? 0 : of->sx0;
    int x1 = of->sx1 > (int)frame->width - 1 ? (int)frame->width - 1 : of->sx1;
    /* With a camera, depth is the view depth of the camera-facing surface:
     * the center's minus the sphere bulge (see sprite_surface). */
    double center_depth = pass->view.camera ? of->cam_center.z : 0.0;
    for(int y=y0;y<=y1&&!band->failed;++y)for(int x=x0;x<=x1&&!band->failed;++x){
        double px=sample_at(x,n),py=sample_at(y,n);
        double nx=(px-cx)/rx,ny=(py-cy)/ry;
        double rotated_x=nx*ca-ny*sa;
        double rotated_y=nx*sa+ny*ca;
        nx=rotated_x;ny=rotated_y;
        bool inside=fabs(nx)<=1&&fabs(ny)<=1;
        Vec3 normal={0,0,1};
        if(object->primitive==SR_OBJECT_SPHERE){inside=nx*nx+ny*ny<=1;normal=(Vec3){nx,-ny,sqrt(fmax(0.0,1-nx*nx-ny*ny))};}
        if (!inside)continue;
        Vec3 point = {world_x + normal.x * object->radius,
                      world_y - normal.y * object->radius,
                      cz + normal.z * object->radius};
        size_t at=(size_t)y*frame->width+(size_t)x;
        double object_depth = pass->view.camera
            ? center_depth - (object->primitive == SR_OBJECT_SPHERE ? normal.z : 0.0) *
                             object->radius * of->sprite.sz
            : -point.z;
        if(object_depth>=pass->depth[at])continue;
        Vec3 geometry[2];
        if (maps)
            sprite_surface(&pass->view, of, px, py, nx, ny, &geometry[0], &geometry[1]);
        SrColor color = shade(pass, of, point, normal, maps ? geometry : NULL);
        store_sample(band, y, at, object_depth, color);
    }
}

/* Target rows [lo, hi] of a sprite or blob span clamped to the target. */
static bool clip_rows(int y0, int y1, int h, int *lo, int *hi) {
    *lo = y0 < 0 ? 0 : y0;
    *hi = y1 > h - 1 ? h - 1 : y1;
    return *lo <= *hi;
}

/* ---- draws ----------------------------------------------------------------
 * A draw is all shadow blobs, or one object with or without its blob.
 * Bands cover whole fragment tiles, so each tile is appended to by a single
 * band. */

typedef struct {
    const Pass *pass;
    const ObjectFrame *of;      /* NULL: every object's blob */
    bool blob;
    BandPlan plan;              /* over fragment tiles */
    int lo, hi;                 /* rows of the draw */
    atomic_bool failed;
} DrawJob;

static void draw_rows(const DrawJob *job, int lo, int hi, Band *band) {
    const Pass *pass = job->pass;
    int h = (int)pass->target->height, a, b;
    if (!job->of) {
        for (size_t i = 0; i < pass->scene->object3d_count; ++i) {
            const ObjectFrame *of = &pass->objects[i];
            if (!of->projected || !of->object->cast_shadow || of->alpha == 0.0) continue;
            if (!clip_rows(of->by0, of->by1, h, &a, &b)) continue;
            if (a < lo) a = lo;
            if (b > hi) b = hi;
            if (a <= b) draw_blob(pass, of, a, b);
        }
        return;
    }
    const ObjectFrame *of = job->of;
    if (job->blob && clip_rows(of->by0, of->by1, h, &a, &b)) {
        if (a < lo) a = lo;
        if (b > hi) b = hi;
        if (a <= b) draw_blob(pass, of, a, b);
    }
    if (of->object->primitive == SR_OBJECT_MESH) {
        for (size_t t = 0; t < of->tri_count && !band->failed; ++t) {
            const RasterTri *tri = &of->tris[t];
            a = tri->min_y > lo ? tri->min_y : lo;
            b = tri->max_y < hi ? tri->max_y : hi;
            if (a <= b) draw_triangle(band, of, tri, a, b);
        }
    } else if (of->projected && clip_rows(of->sy0, of->sy1, h, &a, &b)) {
        if (a < lo) a = lo;
        if (b > hi) b = hi;
        if (a <= b) draw_sprite(band, of, a, b);
    }
}

static void draw_worker(void *opaque, size_t begin, size_t end) {
    DrawJob *job = opaque;
    Band band = {.pass = job->pass};
    for (size_t k = begin; k < end && !band.failed; ++k) {
        int t0, t1;
        band_units(&job->plan, k, &t0, &t1);
        int lo = t0 * TILE_ROWS, hi = t1 * TILE_ROWS - 1;
        if (lo < job->lo) lo = job->lo;
        if (hi > job->hi) hi = job->hi;
        band.memo.valid = false;
        if (lo <= hi) draw_rows(job, lo, hi, &band);
    }
    if (band.failed) atomic_store(&job->failed, true);
}

/* Runs a draw over target rows [lo, hi]; small draws run inline. */
static void run_draw(Pass *pass, const ObjectFrame *of, bool blob, int lo, int hi,
                     long long samples) {
    if (lo > hi) return;
    DrawJob job = {.pass = pass, .of = of, .blob = blob, .lo = lo, .hi = hi};
    atomic_init(&job.failed, false);
    int t0 = lo / TILE_ROWS, t1 = hi / TILE_ROWS;
    job.plan = band_plan(t0, t1 - t0 + 1, samples < 4096 ? 1U : pass->threads);
    if (job.plan.bands <= 1 || job.plan.workers <= 1) {
        Band band = {.pass = pass};
        draw_rows(&job, lo, hi, &band);
        if (band.failed) pass->status = SR_ERR_MEMORY;
        return;
    }
    sr_parallel_for((size_t)job.plan.bands, pass->threads, draw_worker, &job);
    if (atomic_load(&job.failed)) pass->status = SR_ERR_MEMORY;
}

/* ---- per-pass setup ------------------------------------------------------- */

/* World position of a mesh-local point: the transform's scale, rotation
 * (x, then y, then z) and translation, evaluated once per object. */
typedef struct {
    double radius, sx, sy, sz;
    Trig rx, ry, rz;
    Vec3 offset;
} MeshTransform;

static MeshTransform mesh_transform(const SrObject3D *object, double time) {
    return (MeshTransform){
        object->radius,
        sr_anim_eval(&object->transform.scale_x, time),
        sr_anim_eval(&object->transform.scale_y, time),
        sr_anim_eval(&object->transform.scale_z, time),
        trig(sr_anim_eval(&object->transform.rotation_x, time) * SR_PI / 180.0),
        trig(sr_anim_eval(&object->transform.rotation_y, time) * SR_PI / 180.0),
        trig(sr_anim_eval(&object->transform.rotation, time) * SR_PI / 180.0),
        {sr_anim_eval(&object->transform.x, time), sr_anim_eval(&object->transform.y, time),
         sr_anim_eval(&object->transform.z, time)}};
}

static Vec3 mesh_rotate(const MeshTransform *m, Vec3 value) {
    return rot_z(rot_y(rot_x(value, m->rx), m->ry), m->rz);
}

static Vec3 mesh_world(const MeshTransform *m, const double position[3]) {
    double radius = m->radius;
    Vec3 local = {position[0]*radius*m->sx, position[1]*radius*m->sy,
                  position[2]*radius*m->sz};
    local = mesh_rotate(m, local);
    return (Vec3){local.x+m->offset.x, local.y+m->offset.y, local.z+m->offset.z};
}

static MeshVertex mesh_vertex(const View *view, const MeshTransform *m,
                              const SrMeshTriangle *triangle, size_t corner, Vec3 world) {
    MeshVertex vertex = {0};
    vertex.world = world;
    vertex.normal = normalize(mesh_rotate(m,
        (Vec3){triangle->normal[corner][0], triangle->normal[corner][1],
               triangle->normal[corner][2]}));
    vertex.camera = vertex.world;
    if (view->camera) vertex.camera = to_camera(view, vertex.world);
    return vertex;
}

/* Clip in camera space before dividing by z. Attributes at a cut edge
 * are interpolated in world space, then perspective corrected at samples. */
static size_t clip_mesh_plane(const MeshVertex *in, size_t count,
                               MeshVertex *out, double plane, bool near_plane) {
    size_t used = 0;
    for (size_t i = 0; i < count; ++i) {
        const MeshVertex *a = &in[(i + count - 1) % count], *b = &in[i];
        bool a_in = near_plane ? a->camera.z >= plane : a->camera.z <= plane;
        bool b_in = near_plane ? b->camera.z >= plane : b->camera.z <= plane;
        if (a_in != b_in) {
            double t = (plane - a->camera.z) / (b->camera.z - a->camera.z);
            MeshVertex cut = {0};
            cut.world = v_add(v_scale(a->world, 1.0 - t), v_scale(b->world, t));
            cut.normal = v_add(v_scale(a->normal, 1.0 - t), v_scale(b->normal, t));
            cut.camera = v_add(v_scale(a->camera, 1.0 - t), v_scale(b->camera, t));
            cut.camera.z = plane;
            out[used++] = cut;
        }
        if (b_in) out[used++] = *b;
    }
    return used;
}

static bool project_mesh_vertex(const View *view, MeshVertex *vertex) {
    if (!finite_vec(vertex->world) || !finite_vec(vertex->camera)) return false;
    if (!view->camera) {
        vertex->x = vertex->world.x;
        vertex->y = vertex->world.y;
        vertex->depth = -vertex->world.z;
    } else {
        double scale = view->camera->orthographic ? 1.0 : view->focal / vertex->camera.z;
        vertex->x = view->width * .5 + vertex->camera.x * scale;
        vertex->y = view->height * .5 - vertex->camera.y * scale;
        vertex->depth = vertex->camera.z;
    }
    return isfinite(vertex->x) && isfinite(vertex->y) && isfinite(vertex->depth);
}

/* render_triangle's setup; false when it drew nothing and marked nothing. */
static bool prepare_triangle(const MeshVertex in[3], Vec3 face, int n, int fw, int fh,
                             RasterTri *out) {
    MeshVertex v[3] = {in[0], in[1], in[2]};
    double area = edge(v[0].x, v[0].y, v[1].x, v[1].y, v[2].x, v[2].y);
    if (!isfinite(area) || fabs(area) < 1e-12) return false;
    if (area < 0.0) {
        MeshVertex swap = v[1]; v[1] = v[2]; v[2] = swap;
        area = -area;
    }
    memcpy(out->v, v, sizeof v);
    out->area = area;
    out->face = face;
    out->min_x = sr_clamp_int(floor(fmin(v[0].x, fmin(v[1].x, v[2].x)) * n), 0, fw);
    out->max_x = sr_clamp_int(ceil(fmax(v[0].x, fmax(v[1].x, v[2].x)) * n), -1, fw - 1);
    out->min_y = sr_clamp_int(floor(fmin(v[0].y, fmin(v[1].y, v[2].y)) * n), 0, fh);
    out->max_y = sr_clamp_int(ceil(fmax(v[0].y, fmax(v[1].y, v[2].y)) * n), -1, fh - 1);
    out->owns_a = owns_edge(&v[1], &v[2]);
    out->owns_b = owns_edge(&v[2], &v[0]);
    out->owns_c = owns_edge(&v[0], &v[1]);
    return true;
}

/* project() of the object's center through the pass view. */
static void object_project(const SrScene *scene, const View *view, ObjectFrame *of) {
    const SrCamera *camera = view->camera;
    Vec3 c = of->sprite.center;
    if (!camera) {
        of->projected = true; of->cx = c.x; of->cy = c.y; of->pscale = 1;
        return;
    }
    Vec3 p = of->cam_center;
    if (camera->orthographic) {
        of->projected = true; of->pscale = 1;
        of->cx = scene->project.width*.5+p.x; of->cy = scene->project.height*.5-p.y;
    } else if (!(p.z < camera->near_plane || p.z > camera->far_plane)) {
        of->projected = true;
        of->pscale = view->focal / p.z;
        of->cx = scene->project.width * .5 + p.x * of->pscale;
        of->cy = scene->project.height * .5 - p.y * of->pscale;
    }
}

static void object_frame_init(const SrScene *scene, const View *view,
                              const SrObject3D *object, double time, int n,
                              int fw, int fh, ObjectFrame *of) {
    *of = (ObjectFrame){.object = object};
    of->material = object->material ? object->material : &fallback_material;
    of->alpha = object_alpha(object);
    of->sprite = sprite_frame(object, time);
    of->angle = trig(of->sprite.angle);
    of->bound = object->radius * fmax(of->sprite.sx, fmax(of->sprite.sy, of->sprite.sz));
    const SrMaterial *material = of->material;
    of->exponent = 2.0+126.0*(1.0-material->roughness);
    of->spec_scale = 0.04+0.96*material->metallic;
    bool emissive_ok = true;
    const double emissive[3] = {material->emissive.r, material->emissive.g,
                                material->emissive.b};
    for (int c = 0; c < 3; ++c)
        emissive_ok = emissive_ok && isfinite(emissive[c]) &&
                      !(emissive[c] == 0.0 && signbit(emissive[c]));
    of->zero_safe = emissive_ok && isfinite(material->base_color.r) &&
                    isfinite(material->base_color.g) && isfinite(material->base_color.b) &&
                    isfinite(of->spec_scale) && of->exponent >= 0.0 &&
                    of->exponent <= 1000.0;
    if (view->camera) of->cam_center = to_camera(view, of->sprite.center);
    if (object->primitive == SR_OBJECT_MESH && object->mesh_asset &&
        object->mesh_asset->mesh) {
        const SrMesh *mesh = object->mesh_asset->mesh;
        double reach = 0.0;
        for (size_t i = 0; i < mesh->triangle_count; ++i) for (int k = 0; k < 3; ++k) {
            const double *p = mesh->triangles[i].position[k];
            reach = fmax(reach, sqrt(p[0] * p[0] + p[1] * p[1] + p[2] * p[2]));
        }
        of->extent = of->bound * reach;
    } else {
        of->extent = object->primitive == SR_OBJECT_SPHERE ? of->bound
                                                           : of->bound * sqrt(2.0);
    }
    object_project(scene, view, of);
    if (of->projected) {
        double cx = of->cx, cy = of->cy, ps = of->pscale;
        of->rx=fmax(1.0,object->radius*of->sprite.sx*ps);
        of->ry=fmax(1.0,object->radius*of->sprite.sy*ps);
        of->sy0=sr_clamp_int(floor((cy-of->ry)*n),-1,fh);
        of->sy1=sr_clamp_int(ceil((cy+of->ry)*n),-1,fh);
        of->sx0=sr_clamp_int(floor((cx-of->rx)*n),-1,fw);
        of->sx1=sr_clamp_int(ceil((cx+of->rx)*n),-1,fw);
        double x = cx + 25*ps, y = cy + 30*ps;
        double radius = object->radius*of->sprite.sx*ps;
        of->bx = x; of->by = y;
        of->bdx = fmax(radius,1.0); of->bdy = fmax(radius*.3,1.0);
        of->by0=sr_clamp_int((y-radius*.3)*n,-1,fh);
        of->by1=sr_clamp_int((y+radius*.3)*n,-1,fh);
        of->bx0=sr_clamp_int((x-radius)*n,-1,fw);
        of->bx1=sr_clamp_int((x+radius)*n,-1,fw);
    }
}

/* World triangles and drawn fan triangles of a visible mesh object. */
static SrStatus object_mesh_init(const View *view, double time, int n, int fw, int fh,
                                 ObjectFrame *of, WorldTri **world_out,
                                 RasterTri **tris_out) {
    const SrObject3D *object = of->object;
    const SrMesh *mesh = object->mesh_asset->mesh;
    size_t count = mesh->triangle_count;
    if (count > SIZE_MAX / (3 * sizeof(RasterTri))) return SR_ERR_MEMORY;
    WorldTri *world = sr_alloc(count * sizeof(*world));
    RasterTri *tris = sr_alloc(count * 3 * sizeof(*tris));
    *world_out = world;
    *tris_out = tris;
    if (!world || !tris) return SR_ERR_MEMORY;
    MeshTransform m = mesh_transform(object, time);
    int dirty[4] = {0, 0, 0, 0};
    size_t used = 0;
    for (size_t index = 0; index < count; ++index) {
        const SrMeshTriangle *triangle = &mesh->triangles[index];
        for (int k = 0; k < 3; ++k) world[index].v[k] = mesh_world(&m, triangle->position[k]);
        MeshVertex polygon[8], scratch[8];
        bool finite = true;
        for (size_t k = 0; k < 3; ++k) {
            polygon[k] = mesh_vertex(view, &m, triangle, k, world[index].v[k]);
            finite = finite && finite_vec(polygon[k].world) && finite_vec(polygon[k].camera);
        }
        if (!finite) continue;
        Vec3 face = normalize(v_cross(v_sub(polygon[1].world, polygon[0].world),
                                      v_sub(polygon[2].world, polygon[0].world)));
        size_t corners = 3;
        if (view->camera) {
            corners = clip_mesh_plane(polygon, corners, scratch, view->camera->near_plane, true);
            corners = clip_mesh_plane(scratch, corners, polygon, view->camera->far_plane, false);
        }
        for (size_t k = 0; k < corners; ++k)
            finite = project_mesh_vertex(view, &polygon[k]) && finite;
        if (!finite) continue;
        for (size_t k = 1; k + 1 < corners; ++k) {
            MeshVertex triangle3[3] = {polygon[0], polygon[k], polygon[k + 1]};
            RasterTri *tri = &tris[used];
            if (!prepare_triangle(triangle3, face, n, fw, fh, tri)) continue;
            mark_rect(dirty, fw, fh, tri->min_x, tri->min_y, tri->max_x + 1, tri->max_y + 1);
            ++used;
        }
    }
    of->world = world;
    of->world_count = count;
    of->tris = tris;
    of->tri_count = used;
    of->mesh_marked = dirty[2] > dirty[0] && dirty[3] > dirty[1];
    of->mesh_mark = (Rect){dirty[0], dirty[1], dirty[2], dirty[3]};
    return SR_OK;
}

/* ---- resolve -------------------------------------------------------------- */

typedef struct {
    const SrFrame *source;      /* supersampled */
    SrFrame *frame;
    int samples;
    uint32_t x0, x1;
    BandPlan plan;              /* over frame rows */
} ResolveJob;

/* Box filter of the supersampled buffer over frame pixels [x0,x1) of rows
 * [y0,y1), then clears exactly those samples. */
static void resolve_rows(const ResolveJob *job, uint32_t y0, uint32_t y1) {
    const SrFrame *target = job->source;
    SrFrame *frame = job->frame;
    int n = job->samples;
    double norm = 1.0 / (double)(n * n);
    uint32_t x0 = job->x0, x1 = job->x1;
    for (uint32_t y = y0; y < y1; ++y) {
        for (uint32_t x = x0; x < x1; ++x) {
            double sum[4] = {0, 0, 0, 0};
            for (int j = 0; j < n; ++j) for (int i = 0; i < n; ++i) {
                const float *s = &target->px[(((size_t)y * n + j) * target->width +
                                              (size_t)x * n + i) * 4];
                for (int c = 0; c < 4; ++c) sum[c] += s[c];
            }
            if (!(sum[3] > 0.0)) continue;
            float source[4] = {(float)(sum[0] * norm), (float)(sum[1] * norm),
                               (float)(sum[2] * norm), (float)(sum[3] * norm)};
            sr_blend_px(SR_BLEND_NORMAL, &frame->px[((size_t)y * frame->width + x) * 4],
                        source);
        }
        size_t row = (size_t)(x1 - x0) * (size_t)n * 4 * sizeof(float);
        for (uint32_t s = y * (uint32_t)n; s < (y + 1) * (uint32_t)n; ++s)
            memset(target->px + ((size_t)s * target->width + (size_t)x0 * (size_t)n) * 4,
                   0, row);
    }
}

static void resolve_worker(void *opaque, size_t begin, size_t end) {
    const ResolveJob *job = opaque;
    for (size_t k = begin; k < end; ++k) {
        int lo, hi;
        band_units(&job->plan, k, &lo, &hi);
        resolve_rows(job, (uint32_t)lo, (uint32_t)hi);
    }
}

/* ---- rectangle clears ----------------------------------------------------- */

typedef struct {
    float *samples;             /* zero these, or */
    double *depth;              /* set these to INFINITY */
    size_t width;
    int x0, x1;
    BandPlan plan;
} ClearJob;

static void clear_worker(void *opaque, size_t begin, size_t end) {
    const ClearJob *job = opaque;
    for (size_t k = begin; k < end; ++k) {
        int lo, hi;
        band_units(&job->plan, k, &lo, &hi);
        for (int y = lo; y < hi; ++y) {
            size_t first = (size_t)y * job->width + (size_t)job->x0;
            size_t count = (size_t)(job->x1 - job->x0);
            if (job->samples) {
                memset(job->samples + first * 4, 0, count * 4 * sizeof(float));
            } else {
                for (size_t i = 0; i < count; ++i) job->depth[first + i] = INFINITY;
            }
        }
    }
}

static void clear_rect(float *samples, double *depth, size_t width, const int *d,
                       unsigned threads) {
    if (d[2] <= d[0] || d[3] <= d[1]) return;
    ClearJob job = {.samples = samples, .depth = depth, .width = width,
                    .x0 = d[0], .x1 = d[2], .plan = band_plan(d[1], d[3] - d[1], threads)};
    sr_parallel_for((size_t)job.plan.bands, threads, clear_worker, &job);
}

/* ---- the pass ------------------------------------------------------------- */

int sr_lighting_samples(const SrScene *scene) {
    return scene->project.antialias3d >= 1 && scene->project.antialias3d <= 4
         ? (int)scene->project.antialias3d : 1;
}

struct SrLightingPass {
    Pass pass;
    SrFrame *frame;
    SrDiagnostics *diag;
    SrFrame supersampled;       /* over scratch->samples */
    bool own_depth;
    int dirty[4];
    Scratch *scratch;
    Scratch local;              /* when the shared scratch is busy */
    bool shared;
    WorldTri **mesh_world;      /* per object, freed at end */
    RasterTri **mesh_tris;
};

void sr_lighting_end(SrLightingPass *lp) {
    if (!lp) return;
    Pass *pass = &lp->pass;
    const SrScene *scene = pass->scene;
    Scratch *s = lp->scratch;
    if (s) {
        /* Restore the between-pass invariants of the persistent buffers. */
        if (lp->supersampled.px)
            clear_rect(lp->supersampled.px, NULL, lp->supersampled.width, lp->dirty,
                       pass->threads);
        if (lp->own_depth && pass->depth)
            clear_rect(NULL, pass->depth, pass->target->width, pass->touched,
                       pass->threads);
        for (size_t i = 0; i < s->tile_count; ++i) s->tiles[i].count = 0;
    }
    for (size_t i = 0; i < scene->object3d_count; ++i) {
        if (lp->mesh_world) free(lp->mesh_world[i]);
        if (lp->mesh_tris) free(lp->mesh_tris[i]);
    }
    free(lp->mesh_world);
    free(lp->mesh_tris);
    free(pass->objects);
    free(pass->casters);
    free(pass->maps);
    free(pass->lights);
    if (lp->shared) pthread_mutex_unlock(&shared_scratch_lock);
    else scratch_free(&lp->local);
    free(lp);
}

SrStatus sr_lighting_begin(SrScene *scene, double time, SrFrame *frame,
                           SrDepthBuffer *shared, unsigned threads,
                           SrDiagnostics *diag, SrLightingPass **out) {
    *out = NULL;
    const SrCamera *camera=scene_camera(scene);
    if (camera && !camera->orthographic && camera->zoom_set) {
        if (!(sr_anim_eval(&camera->zoom, time) > 0.0)) {
            sr_diag_error(diag, camera->source_line, "camera", "zoom",
                          "animated zoom must remain positive");
            return SR_ERR_RENDER;
        }
    } else if(camera&&!camera->orthographic){double fov=sr_anim_eval(&camera->fov,time);
        if(fov<=1.0||fov>=179.0){sr_diag_error(diag,camera->source_line,"camera","fov",
            "animated field of view must remain between 1 and 179 degrees");return SR_ERR_RENDER;}}
    if(!scene->object3d_count)return SR_OK;
    SrLightingPass *lp = sr_alloc(sizeof(*lp));
    if (!lp) {
        sr_diag_error(diag,0,NULL,NULL,"cannot allocate 3D pass buffers");
        return SR_ERR_MEMORY;
    }
    *lp = (SrLightingPass){0};
    lp->shared = pthread_mutex_trylock(&shared_scratch_lock) == 0;
    lp->scratch = lp->shared ? &shared_scratch : &lp->local;
    Scratch *s = lp->scratch;
    int n = sr_lighting_samples(scene);
    lp->frame = frame;
    lp->diag = diag;
    lp->pass = (Pass){.scene = scene, .time = time, .threads = threads,
                      .view = view_init(scene, time), .samples = n, .target = frame,
                      .dirty = lp->dirty};
    Pass *pass = &lp->pass;
    SrStatus status = SR_OK;
    size_t objects = scene->object3d_count, lights = scene->light_count;
    pass->lights = sr_alloc((lights + 1) * sizeof(*pass->lights));
    pass->maps = sr_alloc((lights + 1) * sizeof(*pass->maps));
    pass->objects = sr_alloc(objects * sizeof(*pass->objects));
    pass->casters = sr_alloc(objects * sizeof(*pass->casters));
    lp->mesh_world = sr_alloc(objects * sizeof(*lp->mesh_world));
    lp->mesh_tris = sr_alloc(objects * sizeof(*lp->mesh_tris));
    if (!pass->lights || !pass->maps || !pass->objects || !pass->casters ||
        !lp->mesh_world || !lp->mesh_tris) {
        status = SR_ERR_MEMORY; goto fail;
    }
    uint32_t tw = frame->width, th = frame->height;
    if (n > 1) {
        if (tw == 0 || th == 0) { status = SR_ERR_ARGUMENT; goto fail; }
        tw = frame->width * (uint32_t)n;
        th = frame->height * (uint32_t)n;
    }
    int fw = (int)tw, fh = (int)th;

    /* Objects. */
    for (size_t i = 0; i < objects; ++i) {
        ObjectFrame *of = &pass->objects[i];
        object_frame_init(scene, &pass->view, &scene->objects3d[i], time, n, fw, fh, of);
        const SrObject3D *object = of->object;
        if (of->alpha != 0.0 && object->primitive == SR_OBJECT_MESH &&
            object->mesh_asset && object->mesh_asset->mesh) {
            status = object_mesh_init(&pass->view, time, n, fw, fh, of,
                                      &lp->mesh_world[i], &lp->mesh_tris[i]);
            if (status != SR_OK) goto fail;
        }
        if (object->cast_shadow && of->alpha != 0.0)
            pass->casters[pass->caster_count++] =
                (PointCaster){object, of->sprite.center, of->bound};
    }

    /* Lights and shadow maps. */
    if (lights + 1 > s->map_slots) {
        float **maps = sr_realloc(s->maps, (lights + 1) * sizeof(*maps));
        if (!maps) { status = SR_ERR_MEMORY; goto fail; }
        s->maps = maps;
        size_t *caps = sr_realloc(s->map_caps, (lights + 1) * sizeof(*caps));
        if (!caps) { status = SR_ERR_MEMORY; goto fail; }
        s->map_caps = caps;
        for (size_t i = s->map_slots; i < lights + 1; ++i) { maps[i] = NULL; caps[i] = 0; }
        s->map_slots = lights + 1;
    }
    for (size_t i = 0; i < lights; ++i) {
        const SrLight *light = &scene->lights[i];
        LightFrame *lf = &pass->lights[i];
        lf->color = sr_anim_color_eval(&light->color, time);
        lf->intensity = fmax(0.0, sr_anim_eval(&light->intensity, time));
        lf->zero_safe = isfinite(lf->intensity) && isfinite(lf->color.r) &&
                        isfinite(lf->color.g) && isfinite(lf->color.b);
        if (light->type == SR_LIGHT_DIRECTIONAL) {
            double yaw=sr_anim_eval(&light->yaw,time)*SR_PI/180.0;
            double pitch=sr_anim_eval(&light->pitch,time)*SR_PI/180.0;
            Vec3 direction=normalize((Vec3){-sin(yaw)*cos(pitch),sin(pitch),cos(yaw)*cos(pitch)});
            Vec3 view={0,0,1};
            lf->direction = direction;
            lf->halfv = normalize((Vec3){direction.x+view.x,direction.y+view.y,direction.z+view.z});
        } else if (light->type != SR_LIGHT_AMBIENT) {
            lf->position = (Vec3){sr_anim_eval(&light->x,time),sr_anim_eval(&light->y,time),
                                  sr_anim_eval(&light->z,time)};
            lf->range = fmax(light->range,1e-9);
            lf->falloff = fmax(light->falloff,0.0);
            if (light->type == SR_LIGHT_SPOT) {
                double yaw=sr_anim_eval(&light->yaw,time)*SR_PI/180.0,pitch=sr_anim_eval(&light->pitch,time)*SR_PI/180.0;
                lf->aim=normalize((Vec3){sin(yaw)*cos(pitch),-sin(pitch),-cos(yaw)*cos(pitch)});
                lf->cone=cos(light->spot_angle*SR_PI/360.0);
                lf->cone_span=fmax(1.0-lf->cone,1e-9);
            }
        }
        if (!light->cast_shadow || light->used_2d) continue;
        if (light->type == SR_LIGHT_POINT) { pass->blob = true; continue; }
        if (light->type != SR_LIGHT_DIRECTIONAL && light->type != SR_LIGHT_SPOT) continue;
        status = map_build(scene, &pass->view, pass->objects, light, time, threads,
                           &s->maps[i], &s->map_caps[i], &pass->maps[i]);
        if (status != SR_OK) goto fail;
        pass->any_map = pass->any_map || pass->maps[i].depth;
    }
    {
        SrColor blob = {0, 0, 0, 1};
        blob.a = clamp01(.3 * blob.a);
        sr_color_to_blend(&scene->project, blob, pass->blob_source);
    }

    /* Supersampled target: kept all zero between passes. */
    size_t pixels = (size_t)tw * th;
    if (n > 1) {
        if (pixels / th != tw || pixels > SIZE_MAX / (4 * sizeof(float))) {
            status = SR_ERR_MEMORY; goto fail;
        }
        if (!s->samples || s->samples_cap < pixels * 4) {
            free(s->samples);
            s->samples_cap = 0;
            s->samples = sr_alloc(pixels * 4 * sizeof(float));
            if (!s->samples) { status = SR_ERR_MEMORY; goto fail; }
            s->samples_cap = pixels * 4;
        }
        lp->supersampled = (SrFrame){tw, th, s->samples};
        pass->target = &lp->supersampled;
    }
    if (shared) {
        if (shared->width != pass->target->width || shared->height != pass->target->height) {
            sr_lighting_end(lp);
            return SR_ERR_ARGUMENT;
        }
        pass->depth = shared->z;
    } else {
        /* Kept all INFINITY between passes. */
        if (!s->depth || s->depth_cap < pixels) {
            free(s->depth);
            s->depth_cap = 0;
            if (pixels > SIZE_MAX / sizeof(double)) { status = SR_ERR_MEMORY; goto fail; }
            s->depth = sr_alloc(pixels * sizeof(*s->depth));
            if (!s->depth) { status = SR_ERR_MEMORY; goto fail; }
            s->depth_cap = pixels;
            for (size_t i = 0; i < pixels; ++i) s->depth[i] = INFINITY;
        }
        pass->depth = s->depth;
        lp->own_depth = true;
    }

    /* Fragment tiles. */
    size_t tiles = ((size_t)pass->target->height + TILE_ROWS - 1) / TILE_ROWS;
    if (tiles > s->tile_count) {
        if (tiles > SIZE_MAX / sizeof(FragmentTile)) { status = SR_ERR_MEMORY; goto fail; }
        FragmentTile *grown = sr_realloc(s->tiles, tiles * sizeof(*grown));
        if (!grown) { status = SR_ERR_MEMORY; goto fail; }
        for (size_t i = s->tile_count; i < tiles; ++i) grown[i] = (FragmentTile){0};
        s->tiles = grown;
        s->tile_count = tiles;
    }
    pass->tiles = s->tiles;
    pass->tile_count = tiles;
    *out = lp;
    return SR_OK;
fail:
    if (status == SR_ERR_MEMORY)
        sr_diag_error(diag,0,NULL,NULL,"cannot allocate 3D pass buffers");
    sr_lighting_end(lp);
    return status;
}

/* The target rows of an object's blob, or false. */
static bool blob_rows(const Pass *pass, const ObjectFrame *of, int *lo, int *hi) {
    if (!pass->blob || !of->object->cast_shadow || of->alpha == 0.0 || !of->projected)
        return false;
    return clip_rows(of->by0, of->by1, (int)pass->target->height, lo, hi);
}

static long long blob_samples(const ObjectFrame *of) {
    return (long long)(of->bx1 - of->bx0 + 1) * (of->by1 - of->by0 + 1);
}

void sr_lighting_draw_blobs(SrLightingPass *lp) {
    Pass *pass = &lp->pass;
    const SrScene *scene = pass->scene;
    int lo = INT32_MAX, hi = -1;
    long long samples = 0;
    for (size_t i = 0; i < scene->object3d_count; ++i) {
        const ObjectFrame *of = &pass->objects[i];
        if (!pass->blob || !of->object->cast_shadow || of->alpha == 0.0 || !of->projected)
            continue;
        mark(pass, of->bx0, of->by0, of->bx1 + 1, of->by1 + 1);
        int a, b;
        if (!blob_rows(pass, of, &a, &b)) continue;
        if (a < lo) lo = a;
        if (b > hi) hi = b;
        samples += blob_samples(of);
    }
    if (lo <= hi) run_draw(pass, NULL, false, lo, hi, samples);
}

SrStatus sr_lighting_draw_object(SrLightingPass *lp, size_t index, bool blob) {
    Pass *pass = &lp->pass;
    if (pass->status != SR_OK) return pass->status;
    const ObjectFrame *of = &pass->objects[index];
    if (of->alpha == 0.0) return SR_OK;
    pass->translucent = of->alpha < 1.0;
    int lo = INT32_MAX, hi = -1, a, b;
    long long samples = 0;
    blob = blob && pass->blob && of->object->cast_shadow && of->projected;
    if (blob) {
        mark(pass, of->bx0, of->by0, of->bx1 + 1, of->by1 + 1);
        if (blob_rows(pass, of, &a, &b)) {
            lo = a; hi = b;
            samples += blob_samples(of);
        }
    }
    if (of->object->primitive == SR_OBJECT_MESH) {
        if (of->mesh_marked)
            mark(pass, of->mesh_mark.x0, of->mesh_mark.y0, of->mesh_mark.x1,
                 of->mesh_mark.y1);
        for (size_t t = 0; t < of->tri_count; ++t) {
            const RasterTri *tri = &of->tris[t];
            if (tri->min_y > tri->max_y || tri->min_x > tri->max_x) continue;
            if (tri->min_y < lo) lo = tri->min_y;
            if (tri->max_y > hi) hi = tri->max_y;
            samples += (long long)(tri->max_x - tri->min_x + 1) *
                       (tri->max_y - tri->min_y + 1);
        }
    } else if (of->projected) {
        mark(pass, of->sx0, of->sy0, of->sx1 + 1, of->sy1 + 1);
        if (clip_rows(of->sy0, of->sy1, (int)pass->target->height, &a, &b)) {
            if (a < lo) lo = a;
            if (b > hi) hi = b;
            samples += (long long)(of->sx1 - of->sx0 + 1) * (of->sy1 - of->sy0 + 1);
        }
    }
    if (pass->translucent) pass->fragments = true;
    if (lo <= hi) run_draw(pass, of, blob, lo, hi, samples);
    if (pass->status == SR_ERR_MEMORY)
        sr_diag_error(lp->diag, 0, NULL, NULL, "cannot allocate 3D pass buffers");
    return pass->status;
}

typedef struct {
    const Pass *pass;
    BandPlan plan;              /* over fragment tiles */
} BlendJob;

static void blend_worker(void *opaque, size_t begin, size_t end) {
    const BlendJob *job = opaque;
    const Pass *pass = job->pass;
    for (size_t k = begin; k < end; ++k) {
        int t0, t1;
        band_units(&job->plan, k, &t0, &t1);
        for (int t = t0; t < t1; ++t) {
            FragmentTile *tile = &pass->tiles[t];
            if (!tile->count) continue;
            qsort(tile->data, tile->count, sizeof(*tile->data), compare_fragments);
            for (size_t i = 0; i < tile->count; ++i) {
                const Fragment *fragment = &tile->data[i];
                /* An opaque object may have been drawn after this fragment was
                 * collected, including within a batch interleaved with cards. */
                if (fragment->depth >= pass->depth[fragment->pixel]) continue;
                sr_blend_px(SR_BLEND_NORMAL, pass->target->px + fragment->pixel * 4,
                            fragment->color);
            }
            tile->count = 0;
        }
    }
}

void sr_lighting_flush(SrLightingPass *lp, bool whole_frame) {
    Pass *pass = &lp->pass;
    if (pass->fragments) {
        BlendJob job = {.pass = pass, .plan = band_plan(0, (int)pass->tile_count,
                                                        pass->threads)};
        sr_parallel_for((size_t)job.plan.bands, pass->threads, blend_worker, &job);
        pass->fragments = false;
    }
    int n = pass->samples;
    if (n <= 1) return;
    SrFrame *frame = lp->frame;
    int *d = lp->dirty;
    /* Outside the dirty rectangle the supersampled buffer is all zero,
     * which resolves to nothing, so a whole-frame resolve covers only the
     * dirty rectangle; the resolved samples are cleared either way. */
    (void)whole_frame;
    if (d[2] > d[0] && d[3] > d[1]) {
        uint32_t x0 = (uint32_t)(d[0] / n), y0 = (uint32_t)(d[1] / n);
        uint32_t x1 = (uint32_t)((d[2] + n - 1) / n), y1 = (uint32_t)((d[3] + n - 1) / n);
        if (x1 > frame->width) x1 = frame->width;
        if (y1 > frame->height) y1 = frame->height;
        ResolveJob job = {.source = &lp->supersampled, .frame = frame, .samples = n,
                          .x0 = x0, .x1 = x1,
                          .plan = band_plan((int)y0, (int)(y1 - y0), pass->threads)};
        if (y1 > y0 && x1 > x0)
            sr_parallel_for((size_t)job.plan.bands, pass->threads, resolve_worker, &job);
    }
    d[0] = d[1] = d[2] = d[3] = 0;
}

double sr_lighting_object_depth(const SrScene *scene, size_t index, double time) {
    const SrObject3D *object = &scene->objects3d[index];
    Vec3 center = {sr_anim_eval(&object->transform.x, time),
                   sr_anim_eval(&object->transform.y, time),
                   sr_anim_eval(&object->transform.z, time)};
    return view_depth(scene, center, time);
}

static SrStatus render_depth(SrScene *scene, double time, SrFrame *frame,
                             SrDepthBuffer *shared, unsigned threads,
                             SrDiagnostics *diag) {
    SrLightingPass *lp = NULL;
    SrStatus status = sr_lighting_begin(scene, time, frame, shared, threads, diag, &lp);
    if (status != SR_OK || !lp) return status;
    sr_lighting_draw_blobs(lp);
    for (size_t i = 0; i < scene->object3d_count && status == SR_OK; ++i)
        status = sr_lighting_draw_object(lp, i, false);
    if (status == SR_OK) sr_lighting_flush(lp, true);
    sr_lighting_end(lp);
    return status;
}

SrStatus sr_lighting_render_threads(SrScene *scene, double time, SrFrame *frame,
                                    unsigned threads, SrDiagnostics *diag) {
    return render_depth(scene, time, frame, NULL, threads, diag);
}

SrStatus sr_lighting_render(SrScene *scene,double time,SrFrame *frame,SrDiagnostics *diag){
    return render_depth(scene, time, frame, NULL, 0, diag);
}

SrStatus sr_lighting_render_depth(SrScene *scene, double time, SrFrame *frame,
                                  SrDepthBuffer *shared, SrDiagnostics *diag) {
    return render_depth(scene, time, frame, shared, 0, diag);
}
