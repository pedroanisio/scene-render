/* SPDX-License-Identifier: Apache-2.0 */
/* Loads scenes written inline as XML text (feature-parity suites). */
#ifndef SR_TEST_SCENE_TEXT_H
#define SR_TEST_SCENE_TEXT_H

#include "scene_render/compositor.h"
#include "scene_render/diagnostics.h"
#include "scene_render/xml.h"

#include <stdlib.h>
#include <unistd.h>

#include "harness.h"

/* Writes `xml` to the scratch file `name` and loads it. Diagnostics go to
 * a temporary sink; *message (when non-NULL) receives its text (caller
 * frees). Returns the load status; on SR_OK the caller frees the scene. */
static inline SrStatus st_load(sr_test_ctx *t, const char *name, const char *xml,
                               SrScene *scene, char **message)
{
    const char *path = sr_test_tmp_path(name);
    FILE *file = fopen(path, "w");
    CHECK(t, file != NULL);
    if (!file) return SR_ERR_IO;
    fputs(xml, file);
    CHECK(t, fclose(file) == 0);
    FILE *sink = tmpfile();
    CHECK(t, sink != NULL);
    if (!sink) return SR_ERR_IO;
    SrDiagnostics diag;
    sr_diag_init(&diag, path, sink);
    SrStatus status = sr_scene_load_xml(path, scene, &diag);
    if (message) {
        fflush(sink);
        long size = ftell(sink);
        rewind(sink);
        *message = calloc(1, (size_t)(size > 0 ? size : 0) + 1);
        if (*message && size > 0 && fread(*message, 1, (size_t)size, sink) == 0)
            (*message)[0] = '\0';
    }
    fclose(sink);
    return status;
}

/* A quiet diagnostics sink for render calls; the caller closes *sink. */
static inline void st_diag(SrDiagnostics *diag, FILE **sink)
{
    *sink = tmpfile();
    sr_diag_init(diag, "test", *sink ? *sink : stderr);
}

static inline bool st_frames_equal(const SrFrame *a, const SrFrame *b)
{
    return a->width == b->width && a->height == b->height &&
           memcmp(a->px, b->px, (size_t)a->width * a->height * 4 * sizeof(float)) == 0;
}

static inline const float *st_px(const SrFrame *frame, uint32_t x, uint32_t y)
{
    return &frame->px[((size_t)y * frame->width + x) * 4];
}

#endif
