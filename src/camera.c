#include "scene_render/camera.h"

#include "scene_render/parallel.h"

#include <math.h>
#include <stdlib.h>
#include <string.h>

typedef struct { double x, y, z; } Vec3;

/* Ray rotations with cos/sin evaluated once per frame instead of per pixel;
 * the arithmetic and its order match the former per-pixel rotate_x/y/z. */
typedef struct { double c, s; } SinCos;
static SinCos sincos_of(double a) { return (SinCos){cos(a), sin(a)}; }
static Vec3 rotate_x_cs(Vec3 p, SinCos r) {
    return (Vec3){p.x, p.y * r.c - p.z * r.s, p.y * r.s + p.z * r.c};
}
static Vec3 rotate_y_cs(Vec3 p, SinCos r) {
    return (Vec3){p.x * r.c + p.z * r.s, p.y, -p.x * r.s + p.z * r.c};
}
static Vec3 rotate_z_cs(Vec3 p, SinCos r) {
    return (Vec3){p.x * r.c - p.y * r.s, p.x * r.s + p.y * r.c, p.z};
}

static const SrCamera *active_camera(const SrScene *scene) {
    if (scene->scene360.viewport_camera_id) {
        for (size_t i = 0; i < scene->camera_count; ++i)
            if (strcmp(scene->cameras[i].id,
                       scene->scene360.viewport_camera_id) == 0)
                return &scene->cameras[i];
    }
    for (size_t i = 0; i < scene->camera_count; ++i)
        if (scene->cameras[i].active) return &scene->cameras[i];
    return NULL;
}

/* Bilinear lookup in continuous panorama coordinates where pixel (i, j)
 * covers [i, i+1) x [j, j+1), so its center sits at (i + 0.5, j + 0.5).
 * Shift by half a pixel so integer coordinates address centers, then wrap
 * x around the 360-degree seam and clamp y at the poles. */
static void sample_wrap(const SrFrame *image, double x, double y, float out[4]) {
    x -= 0.5;
    y -= 0.5;
    x = fmod(x, image->width);
    if (x < 0.0) x += image->width;
    y = fmax(0.0, fmin((double)image->height - 1.0, y));
    uint32_t x0 = (uint32_t)floor(x), y0 = (uint32_t)floor(y);
    if (x0 >= image->width) x0 = image->width - 1;
    uint32_t x1 = (x0 + 1) % image->width;
    uint32_t y1 = y0 + 1 < image->height ? y0 + 1 : y0;
    float tx = (float)(x - x0), ty = (float)(y - y0);
    const float *a = &image->px[((size_t)y0 * image->width + x0) * 4];
    const float *b = &image->px[((size_t)y0 * image->width + x1) * 4];
    const float *d = &image->px[((size_t)y1 * image->width + x0) * 4];
    const float *e = &image->px[((size_t)y1 * image->width + x1) * 4];
    for (size_t c = 0; c < 4; ++c)
        out[c] = (a[c] + (b[c] - a[c]) * tx) * (1.0f - ty) +
                 (d[c] + (e[c] - d[c]) * tx) * ty;
}

typedef struct {
    const SrFrame *panorama;
    SrFrame *viewport;
    SinCos yaw, pitch, roll;
    double tan_half, aspect;
} ViewportJob;

/* Rows are independent, so any row partition yields identical pixels. */
static void render_rows(void *argument, size_t begin, size_t end) {
    ViewportJob *job = argument;
    for (uint32_t y = (uint32_t)begin; y < (uint32_t)end; ++y) {
        SrFrame *viewport = job->viewport;
        const SrFrame *panorama = job->panorama;
        double ny = 1.0 - 2.0 * (y + 0.5) / viewport->height;
        for (uint32_t x = 0; x < viewport->width; ++x) {
            double nx = 2.0 * (x + 0.5) / viewport->width - 1.0;
            Vec3 ray = {nx * job->aspect * job->tan_half,
                        ny * job->tan_half, 1.0};
            ray = rotate_z_cs(ray, job->roll);
            ray = rotate_x_cs(ray, job->pitch);
            ray = rotate_y_cs(ray, job->yaw);
            double length = sqrt(ray.x * ray.x + ray.y * ray.y + ray.z * ray.z);
            double longitude = atan2(ray.x, ray.z);
            double latitude = asin(ray.y / length);
            double px = (longitude / (2.0 * SR_PI) + 0.5) * panorama->width;
            double py = (0.5 - latitude / SR_PI) * panorama->height;
            sample_wrap(panorama, px, py,
                        &viewport->px[((size_t)y * viewport->width + x) * 4]);
        }
    }
}

SrStatus sr_camera_extract_viewport(const SrScene *scene, double time,
                                    const SrFrame *panorama, SrFrame *viewport,
                                    unsigned threads, SrDiagnostics *diag) {
    const SrCamera *camera = active_camera(scene);
    if (!camera) {
        sr_diag_error(diag, 0, "scene360", "viewportCamera",
                      "viewport mode requires a valid active camera");
        return SR_ERR_XML;
    }
    double fov = sr_anim_eval(&camera->fov, time);
    if (fov <= 1.0 || fov >= 179.0) {
        sr_diag_error(diag, camera->source_line, "camera", "fov",
                      "animated field of view must remain between 1 and 179 degrees");
        return SR_ERR_ARGUMENT;
    }
    ViewportJob job = {.panorama=panorama, .viewport=viewport,
        .yaw=sincos_of(sr_anim_eval(&camera->yaw,time)*SR_PI/180.0),
        .pitch=sincos_of(sr_anim_eval(&camera->pitch,time)*SR_PI/180.0),
        .roll=sincos_of(sr_anim_eval(&camera->roll,time)*SR_PI/180.0),
        .tan_half=tan(fov*SR_PI/360.0),
        .aspect=(double)viewport->width/viewport->height};
    return sr_parallel_for(viewport->height, threads, render_rows, &job);
}
