#include "scene_render/camera.h"

#include <math.h>
#include <pthread.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

typedef struct { double x, y, z; } Vec3;

static Vec3 rotate_x(Vec3 p, double a) {
    double c = cos(a), s = sin(a);
    return (Vec3){p.x, p.y * c - p.z * s, p.y * s + p.z * c};
}
static Vec3 rotate_y(Vec3 p, double a) {
    double c = cos(a), s = sin(a);
    return (Vec3){p.x * c + p.z * s, p.y, -p.x * s + p.z * c};
}
static Vec3 rotate_z(Vec3 p, double a) {
    double c = cos(a), s = sin(a);
    return (Vec3){p.x * c - p.y * s, p.x * s + p.y * c, p.z};
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
    uint32_t y0, y1;
    double yaw, pitch, roll, tan_half, aspect;
} ViewportJob;

static void *render_rows(void *argument) {
    ViewportJob *job = argument;
    for (uint32_t y = job->y0; y < job->y1; ++y) {
        SrFrame *viewport = job->viewport;
        const SrFrame *panorama = job->panorama;
        double ny = 1.0 - 2.0 * (y + 0.5) / viewport->height;
        for (uint32_t x = 0; x < viewport->width; ++x) {
            double nx = 2.0 * (x + 0.5) / viewport->width - 1.0;
            Vec3 ray = {nx * job->aspect * job->tan_half,
                        ny * job->tan_half, 1.0};
            ray = rotate_z(ray, job->roll);
            ray = rotate_x(ray, job->pitch);
            ray = rotate_y(ray, job->yaw);
            double length = sqrt(ray.x * ray.x + ray.y * ray.y + ray.z * ray.z);
            double longitude = atan2(ray.x, ray.z);
            double latitude = asin(ray.y / length);
            double px = (longitude / (2.0 * SR_PI) + 0.5) * panorama->width;
            double py = (0.5 - latitude / SR_PI) * panorama->height;
            sample_wrap(panorama, px, py,
                        &viewport->px[((size_t)y * viewport->width + x) * 4]);
        }
    }
    return NULL;
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
    if (!threads) {
        long detected = sysconf(_SC_NPROCESSORS_ONLN);
        threads = detected > 0 ? (unsigned)detected : 1;
    }
    if (threads > 32) threads = 32;
    if (threads > viewport->height) threads = viewport->height;
    if (!threads) threads = 1;
    ViewportJob *jobs = sr_alloc(threads * sizeof(*jobs));
    pthread_t *workers = sr_alloc(threads * sizeof(*workers));
    bool *started = sr_alloc(threads * sizeof(*started));
    if (!jobs || !workers || !started) {
        free(jobs); free(workers); free(started);
        return SR_ERR_MEMORY;
    }
    ViewportJob base = {.panorama=panorama, .viewport=viewport,
        .yaw=sr_anim_eval(&camera->yaw,time)*SR_PI/180.0,
        .pitch=sr_anim_eval(&camera->pitch,time)*SR_PI/180.0,
        .roll=sr_anim_eval(&camera->roll,time)*SR_PI/180.0,
        .tan_half=tan(fov*SR_PI/360.0),
        .aspect=(double)viewport->width/viewport->height};
    for (unsigned i = 0; i < threads; ++i) {
        jobs[i] = base;
        jobs[i].y0 = (uint32_t)((uint64_t)viewport->height * i / threads);
        jobs[i].y1 = (uint32_t)((uint64_t)viewport->height * (i + 1) / threads);
        if (i + 1 < threads)
            started[i] = pthread_create(&workers[i], NULL, render_rows,
                                        &jobs[i]) == 0;
        if (i + 1 == threads || !started[i]) render_rows(&jobs[i]);
    }
    for (unsigned i = 0; i + 1 < threads; ++i)
        if (started[i]) pthread_join(workers[i], NULL);
    free(started); free(workers); free(jobs);
    return SR_OK;
}
