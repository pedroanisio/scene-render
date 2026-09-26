/* SPDX-License-Identifier: Apache-2.0 */
/* Shared helpers for the B1-3 operator, mask, matte and adjustment suites:
 * inline XML scenes rendered through the full scene compositor. */
#ifndef SR_TEST_B13_FIXTURE_H
#define SR_TEST_B13_FIXTURE_H

#include "scene_render/assets.h"
#include "scene_render/color.h"
#include "scene_render/compositing.h"
#include "scene_text.h"

#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

/* Reads and closes a diagnostics sink into a new string (caller frees). */
static inline char *b13_sink_text(FILE *sink) {
    if (!sink) return calloc(1, 1);
    fflush(sink);
    long size = ftell(sink);
    rewind(sink);
    char *text = calloc(1, (size_t)(size > 0 ? size : 0) + 1);
    if (text && size > 0 && fread(text, 1, (size_t)size, sink) == 0) text[0] = '\0';
    fclose(sink);
    return text;
}

/* Loads `xml` and its assets. On SR_OK the caller frees the scene. */
static inline SrStatus b13_load(sr_test_ctx *t, const char *xml, SrScene *scene,
                                char **message) {
    /* CTest runs suites in parallel: keep scratch names per process. */
    char name[64];
    snprintf(name, sizeof(name), "b13-scene-%ld.xml", (long)getpid());
    SrStatus status = st_load(t, name, xml, scene, message);
    if (status != SR_OK) return status;
    FILE *sink;
    SrDiagnostics diag;
    st_diag(&diag, &sink);
    status = sr_assets_load(scene, &diag);
    char *text = b13_sink_text(sink);
    if (status != SR_OK) {
        if (message) {
            free(*message);
            *message = text;
            text = NULL;
        }
        sr_scene_free(scene);
    }
    free(text);
    return status;
}

/* Renders `scene` at `time` into a fresh frame cleared to the project
 * background, as the renderer does. The caller frees frame->px. */
static inline SrStatus b13_render(const SrScene *scene_in, double time,
                                  unsigned threads, SrFrame *frame, char **message) {
    SrScene *scene = (SrScene *)scene_in;
    *frame = (SrFrame){0};
    SrStatus status = sr_frame_init(frame, scene->project.width, scene->project.height);
    if (status != SR_OK) return status;
    float background[4];
    sr_color_to_blend(&scene->project, scene->project.background, background);
    sr_frame_clear(frame, background, threads);
    SrCompositor compositor;
    sr_compositor_init(&compositor, threads);
    FILE *sink;
    SrDiagnostics diag;
    st_diag(&diag, &sink);
    status = sr_compositor_render_scene(&compositor, scene, time, frame, &diag);
    sr_compositor_free(&compositor);
    char *text = b13_sink_text(sink);
    if (message) *message = text;
    else free(text);
    return status;
}

/* Loads and renders once; *frame is owned by the caller on SR_OK. */
static inline SrStatus b13_render_xml(sr_test_ctx *t, const char *xml, double time,
                                      unsigned threads, SrFrame *frame,
                                      char **message) {
    SrScene scene;
    *frame = (SrFrame){0};
    SrStatus status = b13_load(t, xml, &scene, message);
    if (status != SR_OK) return status;
    if (message) {
        free(*message);
        *message = NULL;
    }
    status = b13_render(&scene, time, threads, frame, message);
    if (status != SR_OK) sr_frame_free(frame);
    sr_scene_free(&scene);
    return status;
}

static inline bool b13_near4(const float *a, const float *b, double eps) {
    for (int c = 0; c < 4; ++c)
        if (!(fabs((double)a[c] - (double)b[c]) <= eps)) return false;
    return true;
}

#endif
