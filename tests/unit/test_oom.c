/* SPDX-License-Identifier: Apache-2.0 */
/* Allocation-failure injection. The unit binary is linked with
 * -Wl,--wrap=malloc,--wrap=calloc,--wrap=realloc, so every malloc, calloc
 * and realloc call made by code linked statically into it (the whole
 * scene-render core and the tests) goes through the wrappers below. Calls
 * made inside shared libraries (libxml2, Expat, libav*, FreeType, HarfBuzz,
 * FriBidi, Fontconfig, libc's own strdup/fopen) are resolved inside those
 * libraries and are not intercepted; their failure paths are exercised
 * by the libav fault wrappers of test_encode_faults.c where the core reacts
 * to them.
 *
 * Each operation is replayed with allocation n failing, for n = 0, 1, 2,
 * ... until a replay completes without reaching an injected failure. Every
 * injected replay must fail cleanly with SR_ERR_MEMORY (the exceptions are
 * listed at each operation) and must not crash or leak. Leaks are found
 * by the wrappers themselves (free is wrapped too): every block allocated
 * through them during a replay must have been freed once the replay's
 * results are released, so the check also works where LeakSanitizer
 * cannot run (it needs ptrace, which the Flatpak SDK sandbox denies). The
 * sanitizer build (SR_SANITIZE=ON) turns invalid accesses into failures.
 * SR_OOM_BACKTRACE=1 prints a backtrace of every injected failure. */
#include <execinfo.h>
#include <inttypes.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdlib.h>
#include <string.h>

#include "harness.h"
#include "scene_render/assets.h"
#include "scene_render/encoder.h"
#include "scene_render/renderer.h"
#include "scene_render/xml.h"

void *__real_malloc(size_t size);
void *__real_calloc(size_t count, size_t size);
void *__real_realloc(void *ptr, size_t size);
void *__wrap_malloc(size_t size);
void *__wrap_calloc(size_t count, size_t size);
void *__wrap_realloc(void *ptr, size_t size);
void __real_free(void *ptr);
void __wrap_free(void *ptr);

/* Allocations left before the injected failure; -1 disarms. Atomic because
 * rendering may allocate on worker threads. */
static atomic_long g_countdown = -1;
static atomic_long g_injected = 0;
static const char *g_label = "";   /* operation being replayed, for traces */

static bool oom_should_fail(void) {
    long left = atomic_load(&g_countdown);
    while (left >= 0) {
        if (atomic_compare_exchange_weak(&g_countdown, &left, left - 1)) {
            if (left == 0) {
                atomic_fetch_add(&g_injected, 1);   /* exactly one failure */
                if (getenv("SR_OOM_BACKTRACE")) {   /* debugging aid */
                    fprintf(stderr, "[%s]\n", g_label);
                    void *frames[32];
                    backtrace_symbols_fd(frames, backtrace(frames, 32), 2);
                    fputs("----\n", stderr);
                }
                return true;
            }
            return false;
        }
    }
    return false;
}

/* Live blocks allocated through the wrappers while tracking is on: an
 * open-addressing set of pointers (0 = empty, 1 = deleted). */
enum { OOM_SLOTS = 1 << 15 };
static uintptr_t g_live[OOM_SLOTS];
static atomic_size_t g_live_count;   /* written under g_live_lock */
static atomic_bool g_tracking;        /* written under g_live_lock */
static bool g_live_overflow;
static pthread_mutex_t g_live_lock = PTHREAD_MUTEX_INITIALIZER;

static size_t slot_of(uintptr_t key) {
    return (size_t)((key >> 4) * UINT64_C(0x9E3779B97F4A7C15) >> 46) & (OOM_SLOTS - 1);
}

static void live_add(void *ptr) {
    if (!ptr || !atomic_load(&g_tracking)) return;   /* fast path */
    pthread_mutex_lock(&g_live_lock);
    if (g_tracking) {
        if (g_live_count >= OOM_SLOTS / 2) {
            g_live_overflow = true;
        } else {
            size_t i = slot_of((uintptr_t)ptr);
            while (g_live[i] > 1) i = (i + 1) & (OOM_SLOTS - 1);
            g_live[i] = (uintptr_t)ptr;
            ++g_live_count;
        }
    }
    pthread_mutex_unlock(&g_live_lock);
}

static void live_remove(void *ptr) {
    if (!ptr || !atomic_load(&g_live_count)) return;   /* fast path */
    pthread_mutex_lock(&g_live_lock);
    if (g_live_count) {
        for (size_t i = slot_of((uintptr_t)ptr); g_live[i]; i = (i + 1) & (OOM_SLOTS - 1)) {
            if (g_live[i] == (uintptr_t)ptr) {
                g_live[i] = 1;
                --g_live_count;
                break;
            }
        }
    }
    pthread_mutex_unlock(&g_live_lock);
}

static void live_start(void) {
    pthread_mutex_lock(&g_live_lock);
    memset(g_live, 0, sizeof(g_live));
    g_live_count = 0;
    g_live_overflow = false;
    g_tracking = true;
    pthread_mutex_unlock(&g_live_lock);
}

static void live_set_tracking(bool on) {
    pthread_mutex_lock(&g_live_lock);
    g_tracking = on;
    pthread_mutex_unlock(&g_live_lock);
}

/* Stops tracking; returns the number of blocks still live. */
static size_t live_stop(void) {
    pthread_mutex_lock(&g_live_lock);
    g_tracking = false;
    size_t count = g_live_overflow ? SIZE_MAX : g_live_count;
    pthread_mutex_unlock(&g_live_lock);
    return count;
}

void *__wrap_malloc(size_t size) {
    void *ptr = oom_should_fail() ? NULL : __real_malloc(size);
    live_add(ptr);
    return ptr;
}
void *__wrap_calloc(size_t count, size_t size) {
    void *ptr = oom_should_fail() ? NULL : __real_calloc(count, size);
    live_add(ptr);
    return ptr;
}
void *__wrap_realloc(void *ptr, size_t size) {
    if (oom_should_fail()) return NULL;   /* ptr stays valid and live */
    void *moved = __real_realloc(ptr, size);
    if (moved) {
        live_remove(ptr);
        live_add(moved);
    }
    return moved;
}
void __wrap_free(void *ptr) {
    live_remove(ptr);
    __real_free(ptr);
}

static void oom_arm(long n) {
    atomic_store(&g_injected, 0);
    atomic_store(&g_countdown, n);
}
static void oom_disarm(void) { atomic_store(&g_countdown, -1); }
static bool oom_hit(void) { return atomic_load(&g_injected) != 0; }

/* An operation that allocates more than this many times fails the test
 * instead of looping for ever. */
enum { OOM_MAX_REPLAYS = 20000 };

static FILE *g_sink;

static SrDiagnostics quiet_diag(const char *source) {
    if (!g_sink) g_sink = fopen("/dev/null", "w");
    SrDiagnostics diag;
    sr_diag_init(&diag, source, g_sink ? g_sink : stderr);
    return diag;
}

typedef struct {
    const char *what;
    SrStatus (*op)(void *context);
    /* After every replay: `release` frees what the replay produced (the
     * leak check follows it), `prepare` sets up the next replay. Either
     * may be NULL. */
    void (*release)(void *context);
    void (*prepare)(void *context);
    /* Optional: true when an injected replay that still returned SR_OK
     * produced exactly the uninjected result (a documented fallback such
     * as sr_parallel_for running serially when it cannot allocate its
     * thread table). Without it an injected SR_OK is a failure: the
     * allocation failure was swallowed. */
    bool (*same_result)(void *context);
    SrStatus allowed[3];                     /* SR_ERR_MEMORY first; 0 ends */
} OomSpec;

/* Replays spec->op with failure n = 0, 1, ... and returns the number of
 * allocations the operation makes (the first n that does not fail). */
static long replay_until_success(sr_test_ctx *t, const OomSpec *spec, void *context) {
    long n = 0, fallbacks = 0, by_status[16] = {0};
    char label[160];
    double start = sr_monotonic_seconds();
    for (; n < OOM_MAX_REPLAYS; ++n) {
        snprintf(label, sizeof(label), "%s #%ld", spec->what, n);
        g_label = label;
        live_start();
        oom_arm(n);
        SrStatus status = spec->op(context);
        oom_disarm();
        bool injected = oom_hit();
        live_set_tracking(false);   /* the comparison's own allocations */
        bool fallback = injected && status == SR_OK && spec->same_result &&
                        spec->same_result(context);
        live_set_tracking(true);
        if (spec->release) spec->release(context);
        size_t leaked = live_stop();
        if (spec->prepare) spec->prepare(context);
        if (leaked) {
            SR_FAIL(t, "%s: replay %ld leaked %zu block(s)%s", spec->what, n,
                    leaked == SIZE_MAX ? 0 : leaked,
                    leaked == SIZE_MAX ? " (tracking table overflow)" : "");
            break;
        }
        if (!injected) {
            if (status != SR_OK)
                SR_FAIL(t, "%s: uninjected run returned %d", spec->what, (int)status);
            break;
        }
        if (fallback) {
            ++fallbacks;
            continue;
        }
        bool ok = false;
        for (size_t i = 0; i < 3 && spec->allowed[i]; ++i)
            ok = ok || status == spec->allowed[i];
        if (!ok) {
            SR_FAIL(t, "%s: allocation %ld failed but the operation returned %d%s",
                    spec->what, n, (int)status,
                    status == SR_OK ? " (failure swallowed)" : "");
            break;
        }
        ++by_status[(unsigned)status & 15u];
    }
    CHECK(t, n > 0 && n < OOM_MAX_REPLAYS);
    printf("  %s: %ld allocations; injected failures returned SR_ERR_MEMORY %ld, "
           "SR_ERR_XML %ld times; identical-result fallbacks %ld (%.1f s)\n", spec->what,
           n, by_status[SR_ERR_MEMORY], by_status[SR_ERR_XML], fallbacks,
           sr_monotonic_seconds() - start);
    return n;
}

/* ---------------------------------------------------------------- loading */

typedef struct {
    const char *path;
    SrScene scene;
    bool loaded;
} LoadContext;

/* The Expat builder callbacks cannot return a status: an allocation
 * failure inside one is reported as a diagnostic that says "out of memory"
 * and the load returns SR_ERR_XML (sr_scene_load_xml maps every builder
 * failure to SR_ERR_XML). Such a replay is accepted only when its
 * diagnostics say so; any other SR_ERR_XML is turned into SR_ERR_ARGUMENT,
 * which the replay loop rejects. */
static SrStatus load_op(void *opaque) {
    LoadContext *c = opaque;
    FILE *log = tmpfile();
    SrDiagnostics diag;
    sr_diag_init(&diag, "oom-load", log ? log : stderr);
    SrStatus status = sr_scene_load_xml(c->path, &c->scene, &diag);
    c->loaded = status == SR_OK;
    if (log) {
        char text[4096] = "";
        rewind(log);
        size_t got = fread(text, 1, sizeof(text) - 1, log);
        text[got] = '\0';
        fclose(log);
        if (status == SR_ERR_XML && !strstr(text, "out of memory")) {
            fprintf(stderr, "  SR_ERR_XML without an out-of-memory diagnostic:\n%s", text);
            status = SR_ERR_ARGUMENT;
        }
    }
    return status;
}

static void load_reset(void *opaque) {
    LoadContext *c = opaque;
    if (c->loaded) sr_scene_free(&c->scene);   /* failures free themselves */
    c->loaded = false;
}

static void check_load(sr_test_ctx *t, const char *relative, long minimum) {
    LoadContext c = {.path = sr_test_data_path(relative)};
    char path[1024];
    snprintf(path, sizeof(path), "%s", c.path);
    c.path = path;
    const OomSpec spec = {relative, load_op, load_reset, NULL, NULL,
                          {SR_ERR_MEMORY, SR_ERR_XML}};
    long n = replay_until_success(t, &spec, &c);
    CHECK(t, n > minimum);
}

static void xml_load_survives_allocation_failures(sr_test_ctx *t) {
    check_load(t, "tests/data-oom.xml", 50);
    check_load(t, "tests/data-profile.xml", 25);
    check_load(t, "examples/feature-parity.xml", 50);
}

/* ----------------------------------------------------------------- assets */

typedef struct {
    SrScene scene;
} SceneContext;

static SrStatus assets_op(void *opaque) {
    SceneContext *c = opaque;
    SrDiagnostics diag = quiet_diag("oom-assets");
    return sr_assets_load(&c->scene, &diag);
}

static void assets_reset(void *opaque) {
    sr_assets_unload(&((SceneContext *)opaque)->scene);
}

static bool load_oom_scene(sr_test_ctx *t, SrScene *scene) {
    SrDiagnostics diag = quiet_diag("oom");
    if (sr_scene_load_xml(sr_test_data_path("tests/data-oom.xml"), scene, &diag) != SR_OK) {
        SR_FAIL(t, "tests/data-oom.xml does not load");
        return false;
    }
    return true;
}

static void asset_load_survives_allocation_failures(sr_test_ctx *t) {
    SceneContext c;
    if (!load_oom_scene(t, &c.scene)) return;
    const OomSpec spec = {"asset load", assets_op, assets_reset, NULL, NULL,
                          {SR_ERR_MEMORY}};
    replay_until_success(t, &spec, &c);
    sr_scene_free(&c.scene);
}

/* ------------------------------------------------------------ frame render */

typedef struct {
    SrScene scene;
    bool loaded;
    unsigned threads;
    char path[1024];
    uint8_t *reference;     /* PPM bytes of the uninjected render */
    size_t reference_size;
} RenderContext;

static bool read_all(const char *path, uint8_t **data, size_t *size) {
    *data = NULL;
    *size = 0;
    FILE *file = fopen(path, "rb");
    if (!file) return false;
    uint8_t chunk[65536];
    size_t got;
    bool ok = true;
    while (ok && (got = fread(chunk, 1, sizeof(chunk), file)) > 0) {
        uint8_t *grown = realloc(*data, *size + got);
        if (!grown) ok = false;
        else {
            memcpy(grown + *size, chunk, got);
            *data = grown;
            *size += got;
        }
    }
    fclose(file);
    if (!ok) {
        free(*data);
        *data = NULL;
    }
    return ok && *size > 0;
}

/* sr_render in preview mode: asset load, physics preparation (rigid and
 * soft bodies), 3D pass with a shadow map, compositing, effects, color
 * conversion and the PPM write, for one frame. Every replay starts from a
 * freshly loaded scene (loaded without injection). */
static SrStatus render_op(void *opaque) {
    RenderContext *c = opaque;
    remove(c->path);
    SrDiagnostics diag = quiet_diag("oom-render");
    SrRenderOptions options = {.preview = true, .preview_frame = 6,
                               .preview_path = c->path,
                               .encoder_threads = c->threads};
    SrRenderMetrics metrics;
    return sr_render(&c->scene, &options, &metrics, &diag);
}

static void render_release(void *opaque) {
    RenderContext *c = opaque;
    if (c->loaded) sr_scene_free(&c->scene);
    c->loaded = false;
}

static void render_prepare(void *opaque) {
    RenderContext *c = opaque;
    SrDiagnostics diag = quiet_diag("oom");
    c->loaded = sr_scene_load_xml(sr_test_data_path("tests/data-oom.xml"),
                                  &c->scene, &diag) == SR_OK;
}

static bool render_same_result(void *opaque) {
    RenderContext *c = opaque;
    uint8_t *data = NULL;
    size_t size = 0;
    bool same = read_all(c->path, &data, &size) && size == c->reference_size &&
                memcmp(data, c->reference, size) == 0;
    free(data);
    return same;
}

static void frame_render_survives_allocation_failures(sr_test_ctx *t) {
    static const unsigned threads[] = {1, 3};
    for (size_t i = 0; i < 2; ++i) {
        RenderContext c = {.threads = threads[i], .loaded = true};
        snprintf(c.path, sizeof(c.path), "%s", sr_test_tmp_path("oom-frame.ppm"));
        if (!load_oom_scene(t, &c.scene)) return;
        /* The uninjected frame every surviving replay must reproduce. */
        if (render_op(&c) != SR_OK || !read_all(c.path, &c.reference, &c.reference_size)) {
            SR_FAIL(t, "reference render of tests/data-oom.xml failed");
            sr_scene_free(&c.scene);
            return;
        }
        render_release(&c);
        render_prepare(&c);
        char what[64];
        snprintf(what, sizeof(what), "one-frame render (threads=%u)", threads[i]);
        const OomSpec spec = {what, render_op, render_release, render_prepare,
                              render_same_result,
                              {SR_ERR_MEMORY}};
        long n = replay_until_success(t, &spec, &c);
        CHECK(t, n > 20);
        free(c.reference);
        if (c.loaded) sr_scene_free(&c.scene);
    }
}

/* ---------------------------------------------------------------- encoder */

typedef struct {
    SrScene scene;
    uint8_t *rgba;
    char path[1024];
} EncodeContext;

/* Open (FFV1 Matroska, with an audio stream), one frame of video and one
 * frame of audio, finish, destroy. */
static SrStatus encode_op(void *opaque) {
    EncodeContext *c = opaque;
    SrDiagnostics diag = quiet_diag("oom-encode");
    SrEncoderAudio audio = {48000, 2};
    SrEncoder *encoder = NULL;
    SrStatus status = sr_encoder_open(&encoder, &c->scene, c->path, 1, &audio, &diag);
    float pcm[4000 * 2] = {0};
    if (status == SR_OK) status = sr_encoder_write_video(encoder, c->rgba, &diag);
    if (status == SR_OK) status = sr_encoder_write_audio(encoder, pcm, 4000, &diag);
    if (status == SR_OK) status = sr_encoder_finish(encoder, &diag);
    sr_encoder_destroy(encoder);
    return status;
}

static void encoder_survives_allocation_failures(sr_test_ctx *t) {
    EncodeContext c;
    if (!load_oom_scene(t, &c.scene)) return;
    snprintf(c.path, sizeof(c.path), "%s", sr_test_tmp_path("oom-encode.mkv"));
    size_t bytes = (size_t)c.scene.project.width * c.scene.project.height * 4;
    c.rgba = malloc(bytes);
    if (!c.rgba) {
        SR_FAIL(t, "fixture allocation");
        sr_scene_free(&c.scene);
        return;
    }
    for (size_t i = 0; i < bytes; ++i) c.rgba[i] = (uint8_t)(i * 7u);
    /* libav's own allocations are not intercepted (see the top of this
     * file); the core's are few, but each must fail cleanly. */
    const OomSpec spec = {"encoder open+frame+finish", encode_op, NULL, NULL, NULL,
                          {SR_ERR_MEMORY}};
    replay_until_success(t, &spec, &c);
    free(c.rgba);
    sr_scene_free(&c.scene);
}

/* Rendering twice with one scene (as the golden suite's warm renders do)
 * reloads nothing twice and leaks nothing: sr_render loads assets and
 * prepares physics on every call. */
static void repeated_render_leaks_nothing(sr_test_ctx *t) {
    live_start();
    RenderContext c = {.threads = 2, .loaded = true};
    snprintf(c.path, sizeof(c.path), "%s", sr_test_tmp_path("oom-twice.ppm"));
    if (load_oom_scene(t, &c.scene)) {
        CHECK_INT(t, render_op(&c), SR_OK);
        CHECK_INT(t, render_op(&c), SR_OK);
        sr_scene_free(&c.scene);
    }
    CHECK_INT(t, live_stop(), 0);
}

/* The instruments themselves: the n-th allocation fails exactly once and
 * a block left allocated is reported. */
static void injection_and_leak_check_work(sr_test_ctx *t) {
    /* Called through volatile pointers so the compiler cannot pair and
     * elide the allocations. */
    void *(*volatile m)(size_t) = malloc;
    void *(*volatile z)(size_t, size_t) = calloc;
    void *(*volatile r)(void *, size_t) = realloc;
    void (*volatile f)(void *) = free;
    live_start();
    oom_arm(1);
    void *a = m(16), *b = z(1, 16), *c = r(NULL, 16);
    oom_disarm();
    CHECK(t, a != NULL && b == NULL && c != NULL && oom_hit());
    f(a);
    CHECK_INT(t, live_stop(), 1);   /* c */
    f(c);
    live_start();
    char *p = m(8);
    char *q = p ? r(p, 4096) : NULL;
    f(q ? q : p);
    CHECK_INT(t, live_stop(), 0);
}

const sr_test_case sr_tests_oom[] = {
    {"injection_and_leak_check_work", injection_and_leak_check_work},
    {"repeated_render_leaks_nothing", repeated_render_leaks_nothing},
    {"xml_load_survives_allocation_failures", xml_load_survives_allocation_failures},
    {"asset_load_survives_allocation_failures", asset_load_survives_allocation_failures},
    {"frame_render_survives_allocation_failures", frame_render_survives_allocation_failures},
    {"encoder_survives_allocation_failures", encoder_survives_allocation_failures},
    {NULL, NULL},
};
