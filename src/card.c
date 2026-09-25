#include "scene_render/card.h"

#include <float.h>
#include <math.h>

/* Rotations match src/lighting.c so cards and 3D objects share one world. */
static void rotate_x(double p[3], double a) {
    double c = cos(a), s = sin(a), y = p[1], z = p[2];
    p[1] = y * c - z * s;
    p[2] = y * s + z * c;
}

static void rotate_y(double p[3], double a) {
    double c = cos(a), s = sin(a), x = p[0], z = p[2];
    p[0] = x * c + z * s;
    p[2] = -x * s + z * c;
}

static void rotate_z(double p[3], double a) {
    double c = cos(a), s = sin(a), x = p[0], y = p[1];
    p[0] = x * c - y * s;
    p[1] = x * s + y * c;
}

/* World direction to camera space (no translation). */
static void to_camera_dir(const SrCardView *view, double p[3]) {
    rotate_y(p, -view->yaw);
    rotate_x(p, -view->pitch);
    rotate_z(p, -view->roll);
}

static void to_camera(const SrCardView *view, const double world[3],
                      double p[3]) {
    p[0] = world[0] - view->eye[0];
    p[1] = world[1] - view->eye[1];
    p[2] = world[2] - view->eye[2];
    to_camera_dir(view, p);
}

const SrCamera *sr_active_camera(const SrScene *scene) {
    for (size_t i = 0; scene && i < scene->camera_count; ++i)
        if (scene->cameras[i].active) return &scene->cameras[i];
    return NULL;
}

double sr_camera_focal(const SrScene *scene, const SrCamera *camera,
                       double time) {
    if (camera->zoom_set) return sr_anim_eval(&camera->zoom, time);
    return scene->project.height * .5 /
           tan(sr_anim_eval(&camera->fov, time) * SR_PI / 360.0);
}

SrCardView sr_card_view(const SrScene *scene, double time) {
    SrCardView view = {.orthographic = true,
                       .width = scene->project.width,
                       .height = scene->project.height,
                       .near_plane = -DBL_MAX, .far_plane = DBL_MAX,
                       .focus_distance = 1000.0};
    const SrCamera *camera = sr_active_camera(scene);
    if (!camera) return view;
    view.camera = true;
    view.orthographic = camera->orthographic;
    view.eye[0] = sr_anim_eval(&camera->x, time);
    view.eye[1] = sr_anim_eval(&camera->y, time);
    view.eye[2] = sr_anim_eval(&camera->z, time);
    view.yaw = sr_anim_eval(&camera->yaw, time) * SR_PI / 180.0;
    view.pitch = sr_anim_eval(&camera->pitch, time) * SR_PI / 180.0;
    view.roll = sr_anim_eval(&camera->roll, time) * SR_PI / 180.0;
    if (!camera->orthographic) view.focal = sr_camera_focal(scene, camera, time);
    view.near_plane = camera->near_plane;
    view.far_plane = camera->far_plane;
    view.focus_distance = sr_anim_eval(&camera->focus_distance, time);
    view.aperture = fmax(0.0, sr_anim_eval(&camera->aperture, time));
    return view;
}

double sr_card_view_depth(const SrCardView *view, const double world[3]) {
    double p[3];
    to_camera(view, world, p);
    return p[2];
}

bool sr_card_project(const SrCardView *view, const double world[3],
                     double *sx, double *sy, double *depth) {
    double p[3];
    to_camera(view, world, p);
    *depth = p[2];
    if (p[2] < view->near_plane || p[2] > view->far_plane) return false;
    if (view->orthographic) {
        *sx = view->width * .5 + p[0];
        *sy = view->height * .5 - p[1];
    } else {
        *sx = view->width * .5 + p[0] * view->focal / p[2];
        *sy = view->height * .5 - p[1] * view->focal / p[2];
    }
    return true;
}

static bool invert(const SrHomography *a, SrHomography *out) {
    const double (*m)[3] = a->m;
    double c00 = m[1][1] * m[2][2] - m[1][2] * m[2][1];
    double c01 = m[1][2] * m[2][0] - m[1][0] * m[2][2];
    double c02 = m[1][0] * m[2][1] - m[1][1] * m[2][0];
    double det = m[0][0] * c00 + m[0][1] * c01 + m[0][2] * c02;
    if (!(fabs(det) > 1e-300) || !isfinite(det)) return false;
    double k = 1.0 / det;
    out->m[0][0] = c00 * k;
    out->m[0][1] = (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * k;
    out->m[0][2] = (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * k;
    out->m[1][0] = c01 * k;
    out->m[1][1] = (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * k;
    out->m[1][2] = (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * k;
    out->m[2][0] = c02 * k;
    out->m[2][1] = (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * k;
    out->m[2][2] = (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * k;
    return true;
}

/* Rotates a plane-space offset by the card tilt. */
static void tilt(double p[3], double rotation_x, double rotation_y) {
    rotate_x(p, rotation_x * SR_PI / 180.0);
    rotate_y(p, -rotation_y * SR_PI / 180.0);
}

SrCardPose sr_card_pose(const SrCardView *view, double pu, double pv,
                        double depth, double rotation_x, double rotation_y) {
    SrCardPose pose = {0};
    double w = view->width, h = view->height;
    double pivot[3] = {pu - w * .5, h * .5 - pv, depth};
    /* World plane: Q'(u, v) = origin + u * eu + v * ev. */
    double eu[3] = {1, 0, 0}, ev[3] = {0, -1, 0};
    double origin[3] = {-w * .5 - pivot[0], h * .5 - pivot[1], 0};
    tilt(eu, rotation_x, rotation_y);
    tilt(ev, rotation_x, rotation_y);
    tilt(origin, rotation_x, rotation_y);
    for (int i = 0; i < 3; ++i) origin[i] += pivot[i];
    double co[3], cu[3] = {eu[0], eu[1], eu[2]}, cv[3] = {ev[0], ev[1], ev[2]};
    to_camera(view, origin, co);
    to_camera_dir(view, cu);
    to_camera_dir(view, cv);
    double (*m)[3] = pose.to_screen.m;
    if (view->orthographic) {
        m[0][0] = cu[0]; m[0][1] = cv[0]; m[0][2] = co[0] + w * .5;
        m[1][0] = -cu[1]; m[1][1] = -cv[1]; m[1][2] = h * .5 - co[1];
        m[2][0] = 0; m[2][1] = 0; m[2][2] = 1;
    } else {
        double f = view->focal;
        m[0][0] = f * cu[0] + w * .5 * cu[2];
        m[0][1] = f * cv[0] + w * .5 * cv[2];
        m[0][2] = f * co[0] + w * .5 * co[2];
        m[1][0] = -f * cu[1] + h * .5 * cu[2];
        m[1][1] = -f * cv[1] + h * .5 * cv[2];
        m[1][2] = -f * co[1] + h * .5 * co[2];
        m[2][0] = cu[2]; m[2][1] = cv[2]; m[2][2] = co[2];
    }
    /* Camera-space plane through the origin point with normal cu x cv. */
    pose.normal[0] = cu[1] * cv[2] - cu[2] * cv[1];
    pose.normal[1] = cu[2] * cv[0] - cu[0] * cv[2];
    pose.normal[2] = cu[0] * cv[1] - cu[1] * cv[0];
    pose.offset = pose.normal[0] * co[0] + pose.normal[1] * co[1] +
                  pose.normal[2] * co[2];
    pose.pivot_depth = sr_card_view_depth(view, pivot);
    pose.invertible = invert(&pose.to_screen, &pose.to_plane);
    /* Exactly affine when the bottom row has no u, v terms: always for an
     * orthographic view, and for an untilted card under a camera without
     * yaw or pitch (the rotations by zero are exact). */
    pose.is_affine = m[2][0] == 0.0 && m[2][1] == 0.0 && m[2][2] > 0.0;
    if (pose.is_affine) {
        double k = 1.0 / m[2][2];
        pose.affine = (SrMat3){m[0][0] * k, m[0][1] * k, m[0][2] * k,
                               m[1][0] * k, m[1][1] * k, m[1][2] * k};
    }
    return pose;
}

bool sr_card_depth_at(const SrCardView *view, const SrCardPose *pose,
                      double sx, double sy, double *depth) {
    const double *n = pose->normal;
    if (view->orthographic) {
        if (n[2] == 0.0) return false;
        double ox = sx - view->width * .5, oy = view->height * .5 - sy;
        *depth = (pose->offset - n[0] * ox - n[1] * oy) / n[2];
        return true;
    }
    double rx = (sx - view->width * .5) / view->focal;
    double ry = (view->height * .5 - sy) / view->focal;
    double denom = n[0] * rx + n[1] * ry + n[2];
    if (denom == 0.0) return false;
    double t = pose->offset / denom;
    if (!(t > 0.0)) return false;
    *depth = t;
    return true;
}

static bool apply(const SrHomography *h, double x, double y, double *ox,
                  double *oy) {
    const double (*m)[3] = h->m;
    double w = m[2][0] * x + m[2][1] * y + m[2][2];
    if (!(w > 0.0)) return false;
    *ox = (m[0][0] * x + m[0][1] * y + m[0][2]) / w;
    *oy = (m[1][0] * x + m[1][1] * y + m[1][2]) / w;
    return true;
}

bool sr_card_to_plane(const SrCardPose *pose, double sx, double sy,
                      double *u, double *v) {
    return pose->invertible && apply(&pose->to_plane, sx, sy, u, v);
}

bool sr_card_to_screen(const SrCardPose *pose, double u, double v,
                       double *sx, double *sy) {
    return apply(&pose->to_screen, u, v, sx, sy);
}

double sr_card_blur_radius(const SrCardView *view, double depth) {
    if (!(view->aperture > 0.0)) return 0.0;
    double z = fmax(depth, view->orthographic ? 1e-9 : view->near_plane);
    if (!(z > 0.0)) z = 1e-9;
    return view->aperture * fabs(depth - view->focus_distance) / z;
}
