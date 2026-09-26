/* SPDX-License-Identifier: Apache-2.0 */
/* Golden reference images. Each scene in tests/golden/ is rendered at a
 * few chosen frames through the same path as `scene-render --frame N
 * --preview-out F.png` (sr_render in preview mode: 8-bit straight RGBA in
 * the output color space) and every byte of the decoded frame is compared
 * with tests/golden/expected/<scene>-fNNN.png. Each frame is rendered with
 * 1 thread on a fresh scene and with 4 threads on a scene that first
 * rendered another frame, so thread count and state left behind by an
 * earlier frame (asset caches, video decoders, physics samples) cannot
 * change the result. On mismatch the actual frame is kept in the build's
 * test_tmp/golden/ directory. SR_UPDATE_GOLDEN=1 rewrites the references
 * from the 1-thread render (review them before committing; see
 * tests/golden/README.md). */
#include <errno.h>
#include <inttypes.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>

#include <fontconfig/fontconfig.h>
#include <libavcodec/avcodec.h>
#include <libavutil/frame.h>

#include "harness.h"
#include "scene_render/renderer.h"
#include "scene_render/xml.h"

typedef struct {
    const char *scene;      /* file name under tests/golden/ */
    uint64_t frames[3];     /* frames to check; unused slots are UINT64_MAX */
} GoldenCase;

#define NO_FRAME UINT64_MAX

static bool read_file(const char *path, uint8_t **data, size_t *size) {
    FILE *file = fopen(path, "rb");
    if (!file) return false;
    bool ok = fseek(file, 0, SEEK_END) == 0;
    long length = ok ? ftell(file) : -1;
    ok = ok && length > 0 && fseek(file, 0, SEEK_SET) == 0;
    uint8_t *buffer = ok ? malloc((size_t)length + AV_INPUT_BUFFER_PADDING_SIZE) : NULL;
    ok = buffer && fread(buffer, 1, (size_t)length, file) == (size_t)length;
    fclose(file);
    if (!ok) {
        free(buffer);
        return false;
    }
    memset(buffer + length, 0, AV_INPUT_BUFFER_PADDING_SIZE);
    *data = buffer;
    *size = (size_t)length;
    return true;
}

static bool write_file(const char *path, const uint8_t *data, size_t size) {
    FILE *file = fopen(path, "wb");
    if (!file) return false;
    bool ok = fwrite(data, 1, size, file) == size;
    return fclose(file) == 0 && ok;
}

/* Decodes an 8-bit RGBA PNG with libavcodec into tightly packed RGBA. */
static uint8_t *decode_png(const char *path, int *width, int *height) {
    uint8_t *data = NULL, *pixels = NULL;
    size_t size = 0;
    if (!read_file(path, &data, &size)) return NULL;
    const AVCodec *codec = avcodec_find_decoder(AV_CODEC_ID_PNG);
    AVCodecContext *context = codec ? avcodec_alloc_context3(codec) : NULL;
    AVPacket *packet = av_packet_alloc();
    AVFrame *frame = av_frame_alloc();
    if (context && packet && frame && avcodec_open2(context, codec, NULL) == 0) {
        packet->data = data;
        packet->size = (int)size;
        if (avcodec_send_packet(context, packet) == 0 &&
            avcodec_receive_frame(context, frame) == 0 &&
            frame->format == AV_PIX_FMT_RGBA) {
            size_t row = (size_t)frame->width * 4;
            pixels = malloc(row * (size_t)frame->height);
            for (int y = 0; pixels && y < frame->height; ++y)
                memcpy(pixels + row * (size_t)y,
                       frame->data[0] + (ptrdiff_t)frame->linesize[0] * y, row);
            *width = frame->width;
            *height = frame->height;
        }
    }
    av_frame_free(&frame);
    packet->data = NULL;
    av_packet_free(&packet);
    avcodec_free_context(&context);
    free(data);
    return pixels;
}

static bool update_mode(void) {
    const char *value = getenv("SR_UPDATE_GOLDEN");
    return value && strcmp(value, "1") == 0;
}

static const char *tmp_dir(void) {
    const char *dir = sr_test_tmp_path("golden");
    if (mkdir(dir, 0755) != 0 && errno != EEXIST)
        fprintf(stderr, "cannot create %s\n", dir);
    return dir;
}

static SrStatus render_preview(SrScene *scene, uint64_t frame, unsigned threads,
                               const char *path, SrDiagnostics *diag) {
    SrRenderOptions options = {.preview = true, .preview_frame = frame,
                               .preview_path = path, .encoder_threads = threads};
    SrRenderMetrics metrics;
    return sr_render(scene, &options, &metrics, diag);
}

/* Renders `frame` of `name` with `threads` threads and compares it with its
 * reference. With `warm_frame` != NO_FRAME that frame is rendered first on
 * the same scene. */
static void check_frame(sr_test_ctx *t, const char *name, uint64_t frame,
                        unsigned threads, uint64_t warm_frame) {
    char scene_path[1024], stem[256], expected[1024], actual[1200], warm[1200];
    snprintf(scene_path, sizeof(scene_path), "%s/tests/golden/%s",
             SR_TEST_DATA_DIR, name);
    snprintf(stem, sizeof(stem), "%s", name);
    char *dot = strrchr(stem, '.');
    if (dot) *dot = '\0';
    snprintf(expected, sizeof(expected), "%s/tests/golden/expected/%s-f%03" PRIu64 ".png",
             SR_TEST_DATA_DIR, stem, frame);
    const char *dir = tmp_dir();
    snprintf(actual, sizeof(actual), "%s/%s-f%03" PRIu64 "-t%u.png", dir, stem,
             frame, threads);
    snprintf(warm, sizeof(warm), "%s/%s-warm.png", dir, stem);

    SrDiagnostics diag;
    sr_diag_init(&diag, name, stderr);
    SrScene scene;
    SrStatus status = sr_scene_load_xml(scene_path, &scene, &diag);
    if (status != SR_OK) {
        SR_FAIL(t, "%s: load failed with status %d", name, (int)status);
        return;
    }
    if (warm_frame != NO_FRAME)
        status = render_preview(&scene, warm_frame, threads, warm, &diag);
    if (status == SR_OK)
        status = render_preview(&scene, frame, threads, actual, &diag);
    sr_scene_free(&scene);
    if (status != SR_OK) {
        SR_FAIL(t, "%s frame %" PRIu64 " (threads=%u): render status %d", name,
                frame, threads, (int)status);
        return;
    }
    if (update_mode() && threads == 1) {
        uint8_t *data = NULL;
        size_t size = 0;
        if (!read_file(actual, &data, &size) || !write_file(expected, data, size))
            SR_FAIL(t, "cannot update %s", expected);
        else
            printf("  updated %s\n", expected);
        free(data);
    }
    int aw = 0, ah = 0, ew = 0, eh = 0;
    uint8_t *got = decode_png(actual, &aw, &ah);
    uint8_t *want = decode_png(expected, &ew, &eh);
    if (!got) {
        SR_FAIL(t, "cannot decode rendered %s", actual);
    } else if (!want) {
        SR_FAIL(t, "reference %s is missing or unreadable (SR_UPDATE_GOLDEN=1 creates it)",
                expected);
    } else if (aw != ew || ah != eh) {
        SR_FAIL(t, "%s: rendered %dx%d, reference %dx%d", expected, aw, ah, ew, eh);
    } else {
        size_t bytes = (size_t)aw * (size_t)ah * 4, differ = 0;
        int max_delta = 0;
        for (size_t i = 0; i < bytes; ++i) {
            int delta = abs((int)got[i] - (int)want[i]);
            differ += delta != 0;
            if (delta > max_delta) max_delta = delta;
        }
        if (differ) {
            SR_FAIL(t, "%s-f%03" PRIu64 " (threads=%u): %zu of %zu bytes differ, "
                    "max delta %d; actual frame kept in %s", stem, frame, threads,
                    differ, bytes, max_delta, actual);
        } else {
            remove(actual);
        }
    }
    free(got);
    free(want);
}

static void run_case(sr_test_ctx *t, const GoldenCase *c) {
    for (int i = 0; i < 3 && c->frames[i] != NO_FRAME; ++i) {
        check_frame(t, c->scene, c->frames[i], 1, NO_FRAME);
        /* Warm the 4-thread scene with the next checked frame (or the
         * first one), so frames are also reached out of order. */
        uint64_t warm = (i + 1 < 3 && c->frames[i + 1] != NO_FRAME)
                            ? c->frames[i + 1] : c->frames[0];
        if (warm == c->frames[i]) warm = NO_FRAME;
        check_frame(t, c->scene, c->frames[i], 4, warm);
    }
}

#define GOLDEN(fn, file, f0, f1, f2)                                      \
    static void fn(sr_test_ctx *t) {                                      \
        static const GoldenCase c = {file, {f0, f1, f2}};                 \
        run_case(t, &c);                                                  \
    }

GOLDEN(animation_curve_families, "curves.xml", 0, 8, 20)
GOLDEN(animation_track_options, "tracks.xml", 0, 12, 20)
GOLDEN(composite_blend_modes, "composite.xml", 0, 12, 23)
GOLDEN(groups_and_masks, "groups-masks.xml", 0, 12, 23)
GOLDEN(vector_paths, "paths.xml", 0, 12, 23)
GOLDEN(image_magnification, "images.xml", 0, 12, 23)
GOLDEN(video_time_remap, "video.xml", 0, 12, 23)
GOLDEN(text_layout_inter, "text.xml", 0, 23, NO_FRAME)
GOLDEN(equirect_canvas, "equirect.xml", 0, 12, 23)
GOLDEN(viewport_extraction, "viewport.xml", 0, 12, 23)
GOLDEN(scene3d_shadows_mesh, "scene3d.xml", 0, 12, 23)
GOLDEN(effects_stack, "fx.xml", 0, 12, 23)
GOLDEN(particle_emitters, "particles.xml", 0, 12, 23)
GOLDEN(physics_rigid_soft, "physics.xml", 0, 12, 23)
GOLDEN(deformers_mesh_warp, "deform.xml", 0, 12, 23)

/* The font file Fontconfig's "sans" resolved to when the text-scripts
 * reference was made (Freedesktop SDK 25.08). */
#define SCRIPTS_FONT "DejaVuSans.ttf"

static bool sans_is_reference_font(char *found, size_t size) {
    snprintf(found, size, "(none)");
    FcConfig *config = FcInitLoadConfigAndFonts();
    FcPattern *pattern = config ? FcNameParse((const FcChar8 *)"sans") : NULL;
    bool match = false;
    if (pattern) {
        FcConfigSubstitute(config, pattern, FcMatchPattern);
        FcDefaultSubstitute(pattern);
        FcResult result;
        FcPattern *font = FcFontMatch(config, pattern, &result);
        FcChar8 *file = NULL;
        if (font && FcPatternGetString(font, FC_FILE, 0, &file) == FcResultMatch) {
            const char *base = strrchr((const char *)file, '/');
            base = base ? base + 1 : (const char *)file;
            snprintf(found, size, "%s", (const char *)file);
            match = strcmp(base, SCRIPTS_FONT) == 0;
        }
        if (font) FcPatternDestroy(font);
        FcPatternDestroy(pattern);
    }
    if (config) FcConfigDestroy(config);
    return match;
}

/* Hebrew and Arabic need a system font: this scene is compared only where
 * Fontconfig's "sans" is the font the reference was made with. */
static void text_scripts_fontconfig(sr_test_ctx *t) {
    char found[1024];
    if (!sans_is_reference_font(found, sizeof(found))) {
        printf("  skip: Fontconfig 'sans' is %s, reference used %s\n", found,
               SCRIPTS_FONT);
        return;
    }
    static const GoldenCase c = {"text-scripts.xml", {0, 11, NO_FRAME}};
    run_case(t, &c);
}

const sr_test_case sr_tests_golden[] = {
    {"animation_curve_families", animation_curve_families},
    {"animation_track_options", animation_track_options},
    {"composite_blend_modes", composite_blend_modes},
    {"groups_and_masks", groups_and_masks},
    {"vector_paths", vector_paths},
    {"image_magnification", image_magnification},
    {"video_time_remap", video_time_remap},
    {"text_layout_inter", text_layout_inter},
    {"text_scripts_fontconfig", text_scripts_fontconfig},
    {"equirect_canvas", equirect_canvas},
    {"viewport_extraction", viewport_extraction},
    {"scene3d_shadows_mesh", scene3d_shadows_mesh},
    {"effects_stack", effects_stack},
    {"particle_emitters", particle_emitters},
    {"physics_rigid_soft", physics_rigid_soft},
    {"deformers_mesh_warp", deformers_mesh_warp},
    {NULL, NULL},
};
