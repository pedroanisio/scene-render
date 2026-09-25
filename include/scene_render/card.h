#ifndef SCENE_RENDER_CARD_H
#define SCENE_RENDER_CARD_H

#include "scene_render/scene.h"

/* Depth cards: 2D nodes placed in the 3D world and projected by the active
 * camera (docs/xml-reference.md, "Depth cards").
 *
 * A card's plane coordinates (u, v) are composition pixels: the point its
 * ancestors' and its own 2D transforms map to. At depth d the untilted
 * plane point is the world point (u - W/2, H/2 - v, d) (y up, z away from
 * the camera), the same world space 3D objects use. The tilt rotates the
 * plane about the card's pivot: rotationX first (positive tips the top edge
 * away), then rotationY (positive turns the right edge away). Without an
 * active camera the view is an orthographic camera at the origin, so an
 * untilted card keeps its authored pixels and depth only orders. */

/* The camera state one frame projects with. */
typedef struct {
    bool camera;                /* an active camera exists */
    bool orthographic;          /* also true without a camera */
    double eye[3];
    double yaw, pitch, roll;    /* radians */
    double focal;               /* px; perspective only */
    double width, height;       /* project size */
    double near_plane, far_plane;
    double focus_distance, aperture;
} SrCardView;

/* A projective 3x3 matrix acting on column vectors (x, y, 1). */
typedef struct {
    double m[3][3];
} SrHomography;

/* One card's projection at one time. */
typedef struct {
    SrHomography to_screen;     /* plane (u, v, 1) -> screen, homogeneous */
    SrHomography to_plane;      /* inverse */
    /* True when the mapping is exactly affine (untilted card, camera with
     * zero yaw and pitch): `affine` is then the plane-to-screen transform. */
    bool is_affine;
    SrMat3 affine;
    bool invertible;            /* false when the plane is seen edge-on */
    double pivot_depth;         /* view depth of the pivot */
    /* The card plane in camera space: dot(normal, p) = offset. */
    double normal[3];
    double offset;
} SrCardPose;

/* The first active camera, or NULL. */
const SrCamera *sr_active_camera(const SrScene *scene);
/* Focal length in px of `camera` at `time`: its zoom when set, else
 * height / 2 / tan(fov / 2). */
double sr_camera_focal(const SrScene *scene, const SrCamera *camera,
                       double time);
SrCardView sr_card_view(const SrScene *scene, double time);

/* View depth of the world point `world`. */
double sr_card_view_depth(const SrCardView *view, const double world[3]);
/* Projects world point `world`; false when it is outside [near, far]. */
bool sr_card_project(const SrCardView *view, const double world[3],
                     double *sx, double *sy, double *depth);

/* Pose of a card whose pivot has plane coordinates (pu, pv), at `depth`,
 * tilted by rotation_x / rotation_y degrees. */
SrCardPose sr_card_pose(const SrCardView *view, double pu, double pv,
                        double depth, double rotation_x, double rotation_y);

/* View depth of the card plane seen through screen point (sx, sy); false
 * when the ray misses the plane or meets it behind the camera. */
bool sr_card_depth_at(const SrCardView *view, const SrCardPose *pose,
                      double sx, double sy, double *depth);

/* Maps screen (sx, sy) to plane coordinates; false when the point is on or
 * beyond the plane's horizon (homogeneous w <= 0). */
bool sr_card_to_plane(const SrCardPose *pose, double sx, double sy,
                      double *u, double *v);
/* Maps plane (u, v) to the screen; false as sr_card_to_plane. */
bool sr_card_to_screen(const SrCardPose *pose, double u, double v,
                       double *sx, double *sy);

/* Depth-of-field blur radius in px for a card at view depth `depth`. */
double sr_card_blur_radius(const SrCardView *view, double depth);

#endif
