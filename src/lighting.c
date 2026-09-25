#include "scene_render/lighting.h"
#include "scene_render/color.h"
#include "scene_render/effects.h"

#include <float.h>
#include <math.h>
#include <stdlib.h>
#include <string.h>

/* The 3D pass. Spheres, boxes and planes are camera-facing sprites shaded
 * per pixel; meshes are triangle-rasterized; everything shares one depth
 * buffer. Directional and spot lights with castShadow render a depth map
 * of every caster from the light (see ShadowMap) that receivers sample
 * with 3x3 percentage-closer filtering; point lights keep the bounding-
 * volume approximation. project.antialias3d = N renders the pass at N x N
 * samples per pixel and box-filters the result over the frame. */

typedef struct { double x, y, z; } Vec3;

static const SrCamera *scene_camera(const SrScene *scene) {
    for (size_t i = 0; i < scene->camera_count; ++i)
        if (scene->cameras[i].active) return &scene->cameras[i];
    return NULL;
}

static Vec3 rotate_x(Vec3 p,double a){double c=cos(a),s=sin(a);return(Vec3){p.x,p.y*c-p.z*s,p.y*s+p.z*c};}
static Vec3 rotate_y(Vec3 p,double a){double c=cos(a),s=sin(a);return(Vec3){p.x*c+p.z*s,p.y,-p.x*s+p.z*c};}
static Vec3 rotate_z(Vec3 p,double a){double c=cos(a),s=sin(a);return(Vec3){p.x*c-p.y*s,p.x*s+p.y*c,p.z};}

static Vec3 v_add(Vec3 a, Vec3 b) { return (Vec3){a.x + b.x, a.y + b.y, a.z + b.z}; }
static Vec3 v_sub(Vec3 a, Vec3 b) { return (Vec3){a.x - b.x, a.y - b.y, a.z - b.z}; }
static Vec3 v_scale(Vec3 a, double s) { return (Vec3){a.x * s, a.y * s, a.z * s}; }
static Vec3 v_cross(Vec3 a, Vec3 b) {
    return (Vec3){a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x};
}

static bool project(const SrScene *scene,const SrObject3D *object,double time,
                    double *x,double *y,double *scale){
    double ox=sr_anim_eval(&object->transform.x,time),oy=sr_anim_eval(&object->transform.y,time),oz=sr_anim_eval(&object->transform.z,time);
    const SrCamera *camera=scene_camera(scene);if(!camera){*x=ox;*y=oy;*scale=1;return true;}
    Vec3 p={ox-sr_anim_eval(&camera->x,time),oy-sr_anim_eval(&camera->y,time),oz-sr_anim_eval(&camera->z,time)};
    p=rotate_y(p,-sr_anim_eval(&camera->yaw,time)*SR_PI/180.0);p=rotate_x(p,-sr_anim_eval(&camera->pitch,time)*SR_PI/180.0);p=rotate_z(p,-sr_anim_eval(&camera->roll,time)*SR_PI/180.0);
    if(camera->orthographic){*scale=1;*x=scene->project.width*.5+p.x;*y=scene->project.height*.5-p.y;return true;}
    if (p.z < camera->near_plane || p.z > camera->far_plane) return false;
    double focal = scene->project.height * .5 /
                   tan(sr_anim_eval(&camera->fov,time) * SR_PI / 360.0);
    *scale = focal / p.z;
    *x = scene->project.width * .5 + p.x * *scale;
    *y = scene->project.height * .5 - p.y * *scale;
    return true;
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

static double object_bound(const SrObject3D *object,double time){
    double scale=fmax(fabs(sr_anim_eval(&object->transform.scale_x,time)),
        fmax(fabs(sr_anim_eval(&object->transform.scale_y,time)),
             fabs(sr_anim_eval(&object->transform.scale_z,time))));
    return object->radius*scale;
}

/* Point lights: a caster's bounding sphere on the segment toward the light
 * darkens the receiver (the pre-shadow-map approximation). */
static bool shadowed(const SrScene *scene,const SrObject3D *receiver,
                     Vec3 point,Vec3 direction,double maximum,double time){
    if(!receiver->receive_shadow)return false;
    for(size_t i=0;i<scene->object3d_count;++i){const SrObject3D *caster=&scene->objects3d[i];
        if(caster==receiver||!caster->cast_shadow||object_alpha(caster)==0.0)continue;
        Vec3 center={sr_anim_eval(&caster->transform.x,time),
                     sr_anim_eval(&caster->transform.y,time),
                     sr_anim_eval(&caster->transform.z,time)};
        Vec3 delta={center.x-point.x,center.y-point.y,center.z-point.z};
        double along=dot(delta,direction);
        if(along<=1e-5||along>=maximum)continue;
        double distance2=dot(delta,delta)-along*along;
        double radius=object_bound(caster,time);
        if(distance2<radius*radius)return true;
    }
    return false;
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
} View;

static View view_init(const SrScene *scene, double time) {
    View view = {.camera = scene_camera(scene),
                 .width = scene->project.width, .height = scene->project.height};
    if (view.camera) {
        const SrCamera *camera = view.camera;
        view.eye = (Vec3){sr_anim_eval(&camera->x, time), sr_anim_eval(&camera->y, time),
                          sr_anim_eval(&camera->z, time)};
        view.yaw = sr_anim_eval(&camera->yaw, time) * SR_PI / 180.0;
        view.pitch = sr_anim_eval(&camera->pitch, time) * SR_PI / 180.0;
        view.roll = sr_anim_eval(&camera->roll, time) * SR_PI / 180.0;
        view.focal = view.height * .5 / tan(sr_anim_eval(&camera->fov, time) * SR_PI / 360.0);
    }
    return view;
}

static Vec3 to_camera(const View *view, Vec3 world) {
    Vec3 p = v_sub(world, view->eye);
    p = rotate_y(p, -view->yaw); p = rotate_x(p, -view->pitch);
    return rotate_z(p, -view->roll);
}

static Vec3 camera_dir_to_world(const View *view, Vec3 v) {
    v = rotate_z(v, view->roll); v = rotate_x(v, view->pitch);
    return rotate_y(v, view->yaw);
}

/* Unit vector from the scene toward the viewer. */
static Vec3 toward_viewer(const View *view) {
    if (!view->camera) return (Vec3){0, 0, 1};
    return camera_dir_to_world(view, (Vec3){0, 0, -1});
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

/* World point and outward normal of a sprite pixel sample at screen
 * (sx, sy); (nx, ny) are its normalized (rotated) sprite coordinates. */
static void sprite_surface(const View *view, const SrObject3D *object,
                           const SpriteFrame *frame, double cx, double cy,
                           double scale, double sx, double sy, double nx,
                           double ny, Vec3 *point, Vec3 *normal) {
    double bulge = object->primitive == SR_OBJECT_SPHERE
        ? sqrt(fmax(0.0, 1.0 - nx * nx - ny * ny)) : 0.0;
    double depth = bulge * object->radius * frame->sz;
    if (!view->camera) {
        *point = (Vec3){sx, sy, frame->center.z + depth};
    } else {
        Vec3 c = to_camera(view, frame->center);
        Vec3 local = {c.x + (sx - cx) / scale, c.y - (sy - cy) / scale, c.z - depth};
        *point = v_add(view->eye, camera_dir_to_world(view, local));
    }
    *normal = object->primitive == SR_OBJECT_SPHERE
        ? normalize(v_sub(*point, frame->center)) : toward_viewer(view);
}

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

static void map_store(ShadowMap *map, int i, int j, double t) {
    float *slot = &map->depth[(size_t)j * map->size + i];
    if (t > 0.0 && t < *slot) *slot = (float)t;
}

/* Nearest ray parameter hitting the camera-aligned ellipsoid or the
 * camera-facing sprite quad of `object`, or -1. */
static double sprite_hit(const View *view, const SrObject3D *object,
                         const SpriteFrame *frame, Vec3 origin, Vec3 dir) {
    double ax = object->radius * frame->sx, ay = object->radius * frame->sy;
    double az = object->radius * frame->sz;
    if (ax <= 0.0 || ay <= 0.0) return -1.0;
    if (object->primitive == SR_OBJECT_SPHERE) {
        Vec3 o = v_sub(origin, frame->center), d = dir;
        if (view->camera) {
            o = to_camera(view, origin);
            Vec3 c = to_camera(view, frame->center);
            o = v_sub(o, c);
            d = rotate_z(rotate_x(rotate_y(dir, -view->yaw), -view->pitch), -view->roll);
        }
        if (az <= 0.0) az = fmax(ax, ay);
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
    Vec3 normal = toward_viewer(view);
    double denom = dot(dir, normal);
    if (fabs(denom) < 1e-9) return -1.0;
    double t = dot(v_sub(frame->center, origin), normal) / denom;
    if (t <= 0.0) return -1.0;
    Vec3 hit = v_add(origin, v_scale(dir, t));
    double nx, ny;
    if (view->camera) {
        Vec3 h = to_camera(view, hit), c = to_camera(view, frame->center);
        nx = (h.x - c.x) / ax; ny = -(h.y - c.y) / ay;
    } else {
        nx = (hit.x - frame->center.x) / ax; ny = (hit.y - frame->center.y) / ay;
    }
    /* The sprite loop spans the unrotated box, so a rotated quad is drawn
     * clipped to it; cast the same footprint. */
    if (fabs(nx) > 1.0 || fabs(ny) > 1.0) return -1.0;
    double rx = nx * cos(frame->angle) - ny * sin(frame->angle);
    double ry = nx * sin(frame->angle) + ny * cos(frame->angle);
    return fabs(rx) <= 1.0 && fabs(ry) <= 1.0 ? t : -1.0;
}

static Vec3 object_rotate(const SrObject3D *object,Vec3 value,double time){
    value=rotate_x(value,sr_anim_eval(&object->transform.rotation_x,time)*SR_PI/180.0);
    value=rotate_y(value,sr_anim_eval(&object->transform.rotation_y,time)*SR_PI/180.0);
    return rotate_z(value,sr_anim_eval(&object->transform.rotation,time)*SR_PI/180.0);
}

static Vec3 mesh_world(const SrObject3D *object, const double position[3], double time) {
    double radius=object->radius;
    Vec3 local={position[0]*radius*sr_anim_eval(&object->transform.scale_x,time),
                position[1]*radius*sr_anim_eval(&object->transform.scale_y,time),
                position[2]*radius*sr_anim_eval(&object->transform.scale_z,time)};
    local=object_rotate(object,local,time);
    return (Vec3){local.x+sr_anim_eval(&object->transform.x,time),
                  local.y+sr_anim_eval(&object->transform.y,time),
                  local.z+sr_anim_eval(&object->transform.z,time)};
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

static void map_mesh(ShadowMap *map, const SrObject3D *object, double time) {
    if (!object->mesh_asset || !object->mesh_asset->mesh) return;
    const SrMesh *mesh = object->mesh_asset->mesh;
    for (size_t index = 0; index < mesh->triangle_count; ++index) {
        Vec3 v[3];
        double lo_i = DBL_MAX, hi_i = -DBL_MAX, lo_j = DBL_MAX, hi_j = -DBL_MAX;
        bool whole = false;
        for (int k = 0; k < 3; ++k) {
            v[k] = mesh_world(object, mesh->triangles[index].position[k], time);
            double fi, fj, depth, texel;
            if (!map_coords(map, v[k], &fi, &fj, &depth, &texel)) { whole = true; continue; }
            lo_i = fmin(lo_i, fi); hi_i = fmax(hi_i, fi);
            lo_j = fmin(lo_j, fj); hi_j = fmax(hi_j, fj);
        }
        int i0 = whole ? 0 : sr_clamp_int(floor(lo_i) - 1.0, 0, map->size);
        int i1 = whole ? map->size : sr_clamp_int(ceil(hi_i) + 1.0, 0, map->size);
        int j0 = whole ? 0 : sr_clamp_int(floor(lo_j) - 1.0, 0, map->size);
        int j1 = whole ? map->size : sr_clamp_int(ceil(hi_j) + 1.0, 0, map->size);
        for (int j = j0; j < j1; ++j) for (int i = i0; i < i1; ++i) {
            Vec3 origin, dir;
            texel_ray(map, i, j, &origin, &dir);
            map_store(map, i, j, triangle_hit(origin, dir, v[0], v[1], v[2]));
        }
    }
}

/* Bounding sphere of an object in world space. */
static double object_extent(const SrObject3D *object, double time) {
    double bound = object_bound(object, time);
    if (object->primitive == SR_OBJECT_MESH && object->mesh_asset &&
        object->mesh_asset->mesh) {
        const SrMesh *mesh = object->mesh_asset->mesh;
        double reach = 0.0;
        for (size_t i = 0; i < mesh->triangle_count; ++i) for (int k = 0; k < 3; ++k) {
            const double *p = mesh->triangles[i].position[k];
            reach = fmax(reach, sqrt(p[0] * p[0] + p[1] * p[1] + p[2] * p[2]));
        }
        return bound * reach;
    }
    return object->primitive == SR_OBJECT_SPHERE ? bound : bound * sqrt(2.0);
}

static SrStatus map_build(const SrScene *scene, const View *view,
                          const SrLight *light, double time, ShadowMap *map) {
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
            const SrObject3D *object = &scene->objects3d[i];
            Vec3 c = sprite_frame(object, time).center;
            double r = object_extent(object, time);
            low = (Vec3){fmin(low.x, c.x - r), fmin(low.y, c.y - r), fmin(low.z, c.z - r)};
            high = (Vec3){fmax(high.x, c.x + r), fmax(high.y, c.y + r), fmax(high.z, c.z + r)};
        }
        map->center = v_scale(v_add(low, high), 0.5);
        double half = 0.0;
        for (size_t i = 0; i < scene->object3d_count; ++i) {
            const SrObject3D *object = &scene->objects3d[i];
            Vec3 d = v_sub(sprite_frame(object, time).center, map->center);
            half = fmax(half, sqrt(dot(d, d)) + object_extent(object, time));
        }
        map->half = fmax(half, 1e-6);
        map->texel = 2.0 * map->half / map->size;
    }
    size_t texels = (size_t)map->size * map->size;
    map->depth = malloc(texels * sizeof(*map->depth));
    if (!map->depth) return SR_ERR_MEMORY;
    for (size_t i = 0; i < texels; ++i) map->depth[i] = INFINITY;
    for (size_t index = 0; index < scene->object3d_count; ++index) {
        const SrObject3D *object = &scene->objects3d[index];
        if (!object->cast_shadow || object_alpha(object) == 0.0) continue;
        if (object->primitive == SR_OBJECT_MESH) { map_mesh(map, object, time); continue; }
        SpriteFrame frame = sprite_frame(object, time);
        int i0, j0, i1, j1;
        sphere_texels(map, frame.center, object_extent(object, time), &i0, &j0, &i1, &j1);
        for (int j = j0; j < j1; ++j) for (int i = i0; i < i1; ++i) {
            Vec3 origin, dir;
            texel_ray(map, i, j, &origin, &dir);
            map_store(map, i, j, sprite_hit(view, object, &frame, origin, dir));
        }
    }
    return SR_OK;
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

/* ---- shading ------------------------------------------------------------- */

typedef struct {
    size_t pixel, order;
    double depth;
    float color[4];
} Fragment;

typedef struct {
    const SrScene *scene;
    double time;
    View view;
    SrColor *light_color;       /* per light, evaluated at time */
    ShadowMap *maps;            /* per light; depth NULL when unused */
    bool blob;                  /* point-light screen-space shadow blobs */
    int samples;                /* antialias3d */
    SrFrame *target;            /* frame, or the supersampled buffer */
    double *depth;
    bool translucent;
    Fragment *fragments;
    size_t fragment_count, fragment_capacity;
    SrStatus status;
} Pass;

typedef struct { double x,y,depth; Vec3 world,normal,camera; } MeshVertex;

static void over(const SrProject *project,SrFrame *frame,int x,int y,SrColor color,double alpha);

/* Opaque geometry establishes visibility first. Translucent fragments keep
 * their own depths and blend far-to-near afterwards, including intersecting
 * meshes; they never hide subsequently visited geometry by writing depth. */
static void store_sample(Pass *pass, size_t pixel, double depth, SrColor color) {
    if (!(color.a > 0.0) || !isfinite(depth) || depth >= pass->depth[pixel]) return;
    float source[4];
    sr_color_to_blend(&pass->scene->project, color, source);
    if (!pass->translucent) {
        pass->depth[pixel] = depth;
        sr_blend_px(SR_BLEND_NORMAL, pass->target->px + pixel * 4, source);
        return;
    }
    if (pass->fragment_count == pass->fragment_capacity) {
        size_t capacity = pass->fragment_capacity ? pass->fragment_capacity * 2 : 1024;
        if (capacity < pass->fragment_capacity || capacity > SIZE_MAX / sizeof(Fragment)) {
            pass->status = SR_ERR_MEMORY;
            return;
        }
        Fragment *grown = sr_realloc(pass->fragments, capacity * sizeof(*grown));
        if (!grown) { pass->status = SR_ERR_MEMORY; return; }
        pass->fragments = grown;
        pass->fragment_capacity = capacity;
    }
    Fragment *fragment = &pass->fragments[pass->fragment_count];
    *fragment = (Fragment){.pixel = pixel, .order = pass->fragment_count, .depth = depth};
    memcpy(fragment->color, source, sizeof source);
    ++pass->fragment_count;
}

static int compare_fragments(const void *a, const void *b) {
    const Fragment *fa = a, *fb = b;
    if (fa->pixel != fb->pixel) return fa->pixel < fb->pixel ? -1 : 1;
    if (fa->depth != fb->depth) return fa->depth > fb->depth ? -1 : 1;
    return (fa->order > fb->order) - (fa->order < fb->order);
}

static void blend_fragments(Pass *pass) {
    if (!pass->fragment_count) return;
    qsort(pass->fragments, pass->fragment_count, sizeof(*pass->fragments), compare_fragments);
    for (size_t i = 0; i < pass->fragment_count; ++i) {
        const Fragment *fragment = &pass->fragments[i];
        sr_blend_px(SR_BLEND_NORMAL, pass->target->px + fragment->pixel * 4,
                    fragment->color);
    }
}

/* `geometry` is the receiver's consistent world point and normal for
 * shadow-map lookups (NULL: unshadowed by maps). */
static SrColor shade(const Pass *pass, const SrObject3D *object,
                     Vec3 point, Vec3 normal, const Vec3 geometry[2]) {
    const SrScene *scene = pass->scene;
    double time = pass->time;
    SrMaterial fallback={.base_color={0.7,0.7,0.7,1},.roughness=.5};
    const SrMaterial *material=object->material?object->material:&fallback;
    double r=material->emissive.r,g=material->emissive.g,b=material->emissive.b;
    Vec3 view={0,0,1};
    for(size_t i=0;i<scene->light_count;++i){const SrLight *light=&scene->lights[i];
        if (light->used_2d) continue;
        SrColor color = pass->light_color[i];
        double intensity=fmax(0.0,sr_anim_eval(&light->intensity,time));
        Vec3 direction={0,0,1};double attenuation=1.0,maximum=INFINITY;
        if(light->type==SR_LIGHT_AMBIENT){r+=color.r*intensity;g+=color.g*intensity;b+=color.b*intensity;continue;}
        if(light->type==SR_LIGHT_DIRECTIONAL){double yaw=sr_anim_eval(&light->yaw,time)*SR_PI/180.0;
            double pitch=sr_anim_eval(&light->pitch,time)*SR_PI/180.0;
            direction=normalize((Vec3){-sin(yaw)*cos(pitch),sin(pitch),cos(yaw)*cos(pitch)});
        }else{Vec3 delta={sr_anim_eval(&light->x,time)-point.x,sr_anim_eval(&light->y,time)-point.y,sr_anim_eval(&light->z,time)-point.z};
            double distance=sqrt(dot(delta,delta));maximum=distance;direction=normalize(delta);
            attenuation=pow(clamp01(1.0-distance/fmax(light->range,1e-9)),fmax(light->falloff,0.0));
            if(light->type==SR_LIGHT_SPOT){double yaw=sr_anim_eval(&light->yaw,time)*SR_PI/180.0,pitch=sr_anim_eval(&light->pitch,time)*SR_PI/180.0;
                Vec3 aim=normalize((Vec3){sin(yaw)*cos(pitch),-sin(pitch),-cos(yaw)*cos(pitch)});
                Vec3 from_light={-direction.x,-direction.y,-direction.z};double cone=cos(light->spot_angle*SR_PI/360.0);
                attenuation*=clamp01((dot(from_light,aim)-cone)/fmax(1.0-cone,1e-9));}}
        if (light->cast_shadow) {
            if (pass->maps[i].depth) {
                if (geometry && object->receive_shadow && attenuation > 0.0)
                    attenuation *= map_visibility(&pass->maps[i], geometry[0], geometry[1]);
            } else if (light->type == SR_LIGHT_POINT &&
                       shadowed(scene,object,point,direction,maximum,time)) {
                attenuation*=0.2;
            }
        }
        double diffuse=fmax(0.0,dot(normal,direction));
        Vec3 halfv=normalize((Vec3){direction.x+view.x,direction.y+view.y,direction.z+view.z});
        double exponent=2.0+126.0*(1.0-material->roughness);
        double spec=pow(fmax(0.0,dot(normal,halfv)),exponent)*(0.04+0.96*material->metallic);
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

static MeshVertex mesh_vertex(const Pass *pass, const SrObject3D *object,
                               const SrMeshTriangle *triangle, size_t corner) {
    MeshVertex vertex = {0};
    vertex.world = mesh_world(object, triangle->position[corner], pass->time);
    vertex.normal = normalize(object_rotate(object,
        (Vec3){triangle->normal[corner][0], triangle->normal[corner][1],
               triangle->normal[corner][2]}, pass->time));
    vertex.camera = vertex.world;
    if (pass->view.camera) {
        vertex.camera = v_sub(vertex.world, pass->view.eye);
        vertex.camera = rotate_y(vertex.camera, -pass->view.yaw);
        vertex.camera = rotate_x(vertex.camera, -pass->view.pitch);
        vertex.camera = rotate_z(vertex.camera, -pass->view.roll);
    }
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

static void render_triangle(Pass *pass, const SrObject3D *object,
                            MeshVertex v[3], Vec3 face) {
    double area = edge(v[0].x, v[0].y, v[1].x, v[1].y, v[2].x, v[2].y);
    if (!isfinite(area) || fabs(area) < 1e-12) return;
    if (area < 0.0) {
        MeshVertex swap = v[1]; v[1] = v[2]; v[2] = swap;
        area = -area;
    }
    SrFrame *frame = pass->target;
    int n = pass->samples, fw = (int)frame->width, fh = (int)frame->height;
    int min_x = sr_clamp_int(floor(fmin(v[0].x, fmin(v[1].x, v[2].x)) * n), 0, fw);
    int max_x = sr_clamp_int(ceil(fmax(v[0].x, fmax(v[1].x, v[2].x)) * n), -1, fw - 1);
    int min_y = sr_clamp_int(floor(fmin(v[0].y, fmin(v[1].y, v[2].y)) * n), 0, fh);
    int max_y = sr_clamp_int(ceil(fmax(v[0].y, fmax(v[1].y, v[2].y)) * n), -1, fh - 1);
    bool perspective = pass->view.camera && !pass->view.camera->orthographic;
    bool owns_a = owns_edge(&v[1], &v[2]), owns_b = owns_edge(&v[2], &v[0]);
    bool owns_c = owns_edge(&v[0], &v[1]);
    for (int y = min_y; y <= max_y && pass->status == SR_OK; ++y) {
        for (int x = min_x; x <= max_x && pass->status == SR_OK; ++x) {
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
            Vec3 geometry[2] = {point, face};
            store_sample(pass, at, z, shade(pass, object, point, normal, geometry));
        }
    }
}

static void render_mesh(Pass *pass, const SrObject3D *object) {
    if (!object->mesh_asset || !object->mesh_asset->mesh) return;
    const SrMesh *mesh = object->mesh_asset->mesh;
    for (size_t index = 0; index < mesh->triangle_count && pass->status == SR_OK; ++index) {
        MeshVertex polygon[8], scratch[8];
        bool finite = true;
        for (size_t k = 0; k < 3; ++k) {
            polygon[k] = mesh_vertex(pass, object, &mesh->triangles[index], k);
            finite = finite && finite_vec(polygon[k].world) && finite_vec(polygon[k].camera);
        }
        if (!finite) continue;
        Vec3 face = normalize(v_cross(v_sub(polygon[1].world, polygon[0].world),
                                      v_sub(polygon[2].world, polygon[0].world)));
        size_t count = 3;
        if (pass->view.camera) {
            count = clip_mesh_plane(polygon, count, scratch, pass->view.camera->near_plane, true);
            count = clip_mesh_plane(scratch, count, polygon, pass->view.camera->far_plane, false);
        }
        for (size_t k = 0; k < count; ++k)
            finite = project_mesh_vertex(&pass->view, &polygon[k]) && finite;
        if (!finite) continue;
        for (size_t k = 1; k + 1 < count && pass->status == SR_OK; ++k) {
            MeshVertex triangle[3] = {polygon[0], polygon[k], polygon[k + 1]};
            render_triangle(pass, object, triangle, face);
        }
    }
}

/* Premultiplied source-over of a straight working-space color. */
static void over(const SrProject *project, SrFrame *frame, int x, int y,
                 SrColor color, double alpha) {
    if (x < 0 || y < 0 || x >= (int)frame->width || y >= (int)frame->height) return;
    color.a = clamp01(alpha * color.a);
    float source[4];
    sr_color_to_blend(project, color, source);
    sr_blend_px(SR_BLEND_NORMAL, &frame->px[((size_t)y*frame->width+(size_t)x)*4],
                source);
}

/* Point lights keep the screen-space blob under shadow casters. */
static void shadow_blob(const Pass *pass,const SrObject3D *object){
    const SrScene *scene=pass->scene;double time=pass->time;int n=pass->samples;
    if(!pass->blob||!object->cast_shadow||object_alpha(object)==0.0)return;
    double x,y,projection_scale;if(!project(scene,object,time,&x,&y,&projection_scale))return;x+=25*projection_scale;y+=30*projection_scale;
    double radius=object->radius*fabs(sr_anim_eval(&object->transform.scale_x,time))*projection_scale;
    /* Pixels beyond the target are skipped by over(), so the loop bounds
     * clamp to one pixel outside it. */
    int bw=(int)pass->target->width,bh=(int)pass->target->height;
    int py0=sr_clamp_int((y-radius*.3)*n,-1,bh),py1=sr_clamp_int((y+radius*.3)*n,-1,bh);
    int px0=sr_clamp_int((x-radius)*n,-1,bw),px1=sr_clamp_int((x+radius)*n,-1,bw);
    for(int py=py0;py<=py1;++py)for(int px=px0;px<=px1;++px){
        double dx=(px/(double)n-x)/fmax(radius,1.0),dy=(py/(double)n-y)/fmax(radius*.3,1.0);
        if(dx*dx+dy*dy<=1.0)over(&scene->project,pass->target,px,py,(SrColor){0,0,0,1},.3);}
}

static void render_sprite(Pass *pass, const SrObject3D *object) {
    const SrScene *scene = pass->scene;
    double time = pass->time;
    SrFrame *frame = pass->target;
    int n = pass->samples;
    double world_x=sr_anim_eval(&object->transform.x,time),world_y=sr_anim_eval(&object->transform.y,time),cz=sr_anim_eval(&object->transform.z,time);
    double cx,cy,projection_scale;if(!project(scene,object,time,&cx,&cy,&projection_scale))return;
    double sx=fabs(sr_anim_eval(&object->transform.scale_x,time)),sy=fabs(sr_anim_eval(&object->transform.scale_y,time));
    double rx=fmax(1.0,object->radius*sx*projection_scale),ry=fmax(1.0,object->radius*sy*projection_scale);
    bool maps = false;
    for (size_t i = 0; i < scene->light_count; ++i) maps = maps || pass->maps[i].depth;
    SpriteFrame sprite = sprite_frame(object, time);
    int fw=(int)frame->width,fh=(int)frame->height;
    int y0=sr_clamp_int(floor((cy-ry)*n),-1,fh),y1=sr_clamp_int(ceil((cy+ry)*n),-1,fh);
    int x0=sr_clamp_int(floor((cx-rx)*n),-1,fw),x1=sr_clamp_int(ceil((cx+rx)*n),-1,fw);
    for(int y=y0;y<=y1&&pass->status==SR_OK;++y)for(int x=x0;x<=x1&&pass->status==SR_OK;++x){
        double px=sample_at(x,n),py=sample_at(y,n);
        double nx=(px-cx)/rx,ny=(py-cy)/ry;
        double angle=-sr_anim_eval(&object->transform.rotation,time)*SR_PI/180.0;
        double rotated_x=nx*cos(angle)-ny*sin(angle);
        double rotated_y=nx*sin(angle)+ny*cos(angle);
        nx=rotated_x;ny=rotated_y;
        bool inside=fabs(nx)<=1&&fabs(ny)<=1;
        Vec3 normal={0,0,1};
        if(object->primitive==SR_OBJECT_SPHERE){inside=nx*nx+ny*ny<=1;normal=(Vec3){nx,-ny,sqrt(fmax(0.0,1-nx*nx-ny*ny))};}
        if (!inside||x<0||y<0||x>=(int)frame->width||y>=(int)frame->height)continue;
        Vec3 point = {world_x + normal.x * object->radius,
                      world_y - normal.y * object->radius,
                      cz + normal.z * object->radius};
        size_t at=(size_t)y*frame->width+(size_t)x;
        double object_depth=view_depth(scene,point,time);
        if(object_depth>=pass->depth[at])continue;
        Vec3 geometry[2];
        if (maps)
            sprite_surface(&pass->view, object, &sprite, cx, cy, projection_scale,
                           px, py, nx, ny, &geometry[0], &geometry[1]);
        SrColor color = shade(pass, object, point, normal, maps ? geometry : NULL);
        store_sample(pass, at, object_depth, color);
    }
}

/* Box filter of the supersampled buffer over the frame. */
static void resolve(const Pass *pass, SrFrame *frame) {
    int n = pass->samples;
    double norm = 1.0 / (double)(n * n);
    for (uint32_t y = 0; y < frame->height; ++y) for (uint32_t x = 0; x < frame->width; ++x) {
        double sum[4] = {0, 0, 0, 0};
        for (int j = 0; j < n; ++j) for (int i = 0; i < n; ++i) {
            const float *s = &pass->target->px[(((size_t)y * n + j) * pass->target->width +
                                                (size_t)x * n + i) * 4];
            for (int c = 0; c < 4; ++c) sum[c] += s[c];
        }
        if (!(sum[3] > 0.0)) continue;
        float source[4] = {(float)(sum[0] * norm), (float)(sum[1] * norm),
                           (float)(sum[2] * norm), (float)(sum[3] * norm)};
        sr_blend_px(SR_BLEND_NORMAL, &frame->px[((size_t)y * frame->width + x) * 4], source);
    }
}

SrStatus sr_lighting_render(SrScene *scene,double time,SrFrame *frame,SrDiagnostics *diag){
    const SrCamera *camera=scene_camera(scene);
    if(camera&&!camera->orthographic){double fov=sr_anim_eval(&camera->fov,time);
        if(fov<=1.0||fov>=179.0){sr_diag_error(diag,camera->source_line,"camera","fov",
            "animated field of view must remain between 1 and 179 degrees");return SR_ERR_RENDER;}}
    if(!scene->object3d_count)return SR_OK;
    int n = scene->project.antialias3d >= 1 && scene->project.antialias3d <= 4
          ? (int)scene->project.antialias3d : 1;
    Pass pass = {.scene = scene, .time = time, .view = view_init(scene, time),
                 .samples = n, .target = frame};
    SrFrame supersampled = {0};
    SrStatus status = SR_OK;
    pass.light_color = sr_alloc((scene->light_count + 1) * sizeof(*pass.light_color));
    pass.maps = sr_alloc((scene->light_count + 1) * sizeof(*pass.maps));
    if (!pass.light_color || !pass.maps) { status = SR_ERR_MEMORY; goto done; }
    for (size_t i = 0; i < scene->light_count; ++i) {
        const SrLight *light = &scene->lights[i];
        pass.light_color[i] = sr_anim_color_eval(&light->color, time);
        if (!light->cast_shadow || light->used_2d) continue;
        if (light->type == SR_LIGHT_POINT) { pass.blob = true; continue; }
        if (light->type != SR_LIGHT_DIRECTIONAL && light->type != SR_LIGHT_SPOT) continue;
        status = map_build(scene, &pass.view, light, time, &pass.maps[i]);
        if (status != SR_OK) goto done;
    }
    if (n > 1) {
        status = sr_frame_init(&supersampled, frame->width * (uint32_t)n,
                               frame->height * (uint32_t)n);
        if (status != SR_OK) goto done;
        pass.target = &supersampled;
    }
    size_t pixels=(size_t)pass.target->width*pass.target->height;
    pass.depth=sr_alloc(pixels*sizeof(*pass.depth));
    if(!pass.depth){status=SR_ERR_MEMORY;goto done;}
    for(size_t i=0;i<pixels;++i)pass.depth[i]=INFINITY;
    for(size_t i=0;i<scene->object3d_count;++i)shadow_blob(&pass,&scene->objects3d[i]);
    for (int phase = 0; phase < 2; ++phase) {
        pass.translucent = phase == 1;
        for(size_t i=0;i<scene->object3d_count;++i){SrObject3D *object=&scene->objects3d[i];
            double alpha = object_alpha(object);
            if (alpha == 0.0 || (alpha < 1.0) != pass.translucent) continue;
            if(object->primitive==SR_OBJECT_MESH)render_mesh(&pass,object);
            else render_sprite(&pass,object);
            if (pass.status != SR_OK) { status = pass.status; goto done; }
        }
    }
    blend_fragments(&pass);
    if (n > 1) resolve(&pass, frame);
done:
    if (status == SR_ERR_MEMORY)
        sr_diag_error(diag,0,NULL,NULL,"cannot allocate 3D pass buffers");
    for (size_t i = 0; pass.maps && i < scene->light_count; ++i) free(pass.maps[i].depth);
    free(pass.maps); free(pass.light_color); free(pass.depth); free(pass.fragments);
    sr_frame_free(&supersampled);
    return status;
}
