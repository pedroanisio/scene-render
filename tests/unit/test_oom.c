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
#include "vector_path_internal.h"
#include "compositing_internal.h"
#include "compositor_resources_internal.h"

#include <execinfo.h>
#include <inttypes.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdlib.h>
#include <string.h>

#include "harness.h"
#include "fixture.h"
#include "resource_fixture.h"
#include "length_frame.h"
#include "particles_internal.h"
#include "scene_render/physics.h"
#include "scene_render/assets.h"
#include "scene_render/audio.h"
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
    g_label = "";
    return n;
}

/* ---------------------------------------------------------- color parsing */

static SrStatus color_parse_op(void *opaque) {
    const char *text = opaque;
    const SrColor sentinel = {0.2, 0.3, 0.4, 0.5};
    SrColor parsed = sentinel;
    SrStatus status = sr_parse_color_status(text, &parsed);
    if (status != SR_OK && memcmp(&parsed, &sentinel, sizeof(parsed)) != 0)
        return SR_ERR_ARGUMENT;
    return status;
}

static void color_parse_survives_allocation_failures(sr_test_ctx *t) {
    const OomSpec spec = {"decimal color", color_parse_op, NULL, NULL, NULL,
                         {SR_ERR_MEMORY}};
    char rgb[] = "0.1,0.2,0.3", rgba[] = "0.1,0.2,0.3,0.4";
    CHECK_INT(t, replay_until_success(t, &spec, rgb), 1);
    CHECK_INT(t, replay_until_success(t, &spec, rgba), 1);

    SrColor parsed = {0.2, 0.3, 0.4, 0.5}, sentinel = parsed;
    live_start();
    g_label = "boolean color wrapper";
    oom_arm(0);
    bool ok = sr_parse_color(rgb, &parsed);
    oom_disarm();
    CHECK(t, !ok && oom_hit());
    CHECK(t, memcmp(&parsed, &sentinel, sizeof(parsed)) == 0);
    CHECK_INT(t, live_stop(), 0);
}

/* The descriptor is caller-owned; successful and partial parses, raster
 * scratch and stroke indices must all unwind independently. */
static SrStatus prepared_path_op(void *opaque) {
    SrPreparedPath *path = opaque;
    SrStatus status = sr_prepared_path_parse(
        "M 1 1 C 2 8 8 2 9 9 Q 5 11 1 9 Z M 3 3 L 7 4 L 4 8 Z", path);
    float fill[144], stroke[144];
    if (status == SR_OK)
        status = sr_prepared_path_coverage(path, SR_FILL_EVENODD,
                                           1.5, 12, 12, fill, stroke);
    return status;
}

static SrStatus prepared_path_bad_op(void *opaque) {
    SrPreparedPath *path = opaque;
    SrStatus status = sr_prepared_path_parse(
        "M 1 1 C 2 8 8 2 9 9 Q 5 11 1 9 Z M 3 3 L 7 4 L 4 8 Z Q nope", path);
    if (path->items || path->count || path->capacity) return SR_ERR_ARGUMENT;
    return status == SR_ERR_ASSET ? SR_OK : status;
}

static void prepared_path_release(void *opaque) {
    sr_prepared_path_free(opaque);
}

static void prepared_paths_survive_allocation_failures(sr_test_ctx *t) {
    SrPreparedPath path = {0};
    const OomSpec valid = {"prepared path and coverage", prepared_path_op,
        prepared_path_release, NULL, NULL, {SR_ERR_MEMORY}};
    CHECK(t, replay_until_success(t, &valid, &path) > 5);
    const OomSpec invalid = {"partial prepared path", prepared_path_bad_op,
        prepared_path_release, NULL, NULL, {SR_ERR_MEMORY}};
    CHECK(t, replay_until_success(t, &invalid, &path) > 2);
}

typedef struct {
    SrPreparedPath path;
    const char *text;
    uint64_t quota;
    SrPathParseError expected;
} MaskPathOom;

static SrStatus mask_path_op(void *opaque) {
    MaskPathOom *ctx = opaque;
    SrPathParseInfo info;
    SrStatus status = sr_prepared_mask_path_parse(ctx->text, ctx->quota,
                                                  &ctx->path, &info);
    if (status != SR_OK && (ctx->path.items || ctx->path.count ||
                           ctx->path.capacity || info.owned_bytes)) return SR_ERR_ARGUMENT;
    if (status == SR_ERR_MEMORY)
        return info.error == SR_PATH_PARSE_MEMORY ? status : SR_ERR_ARGUMENT;
    if (info.error != ctx->expected) return SR_ERR_ARGUMENT;
    if (status == SR_OK) return info.owned_bytes ? SR_OK : SR_ERR_ARGUMENT;
    return status == SR_ERR_ASSET ? SR_OK : SR_ERR_ARGUMENT;
}

static void mask_path_release(void *opaque) {
    MaskPathOom *ctx = opaque;
    sr_prepared_path_free(&ctx->path);
}

static void mask_paths_survive_allocation_failures(sr_test_ctx *t) {
    const char *prefix = "M0 0C1 0 1 1 0 1Q1 2 3 4Z "
                         "M0 0H1 M0 0H1 M0 0H1 M0 0H1";
    char text[256];
    const char *tails[] = {"", " Q?", " q1000000000 0 0 0", ""};
    const SrPathParseError errors[] = {SR_PATH_PARSE_OK, SR_PATH_PARSE_SYNTAX,
                                      SR_PATH_PARSE_COORDINATE, SR_PATH_PARSE_STORAGE};
    const OomSpec spec = {"bounded mask path", mask_path_op,
        mask_path_release, NULL, NULL, {SR_ERR_MEMORY}};
    for (size_t i = 0; i < 4; ++i) {
        snprintf(text, sizeof(text), "%s%s", prefix, tails[i]);
        MaskPathOom ctx = {.text = text, .quota = i == 3 ? 1000 : SR_MAX_COMPOSITE_BYTES,
                           .expected = errors[i]};
        CHECK(t, replay_until_success(t, &spec, &ctx) > 2);
    }
}

/* ---------------------------------------------------------------- loading */

static SrStatus compositing_prepare_op(void *opaque) {
    SrScene *scene = opaque;
    SrStatus status = sr_scene_prepare_compositing(scene, NULL);
    /* Repreparation must discard the previous plan even if replacement fails. */
    if (status == SR_OK) status = sr_scene_prepare_compositing(scene, NULL);
    if (status != SR_OK && scene->compositing) return SR_ERR_ARGUMENT;
    return status;
}

static SrStatus compositing_invalid_op(void *opaque) {
    SrScene *scene = opaque;
    SrStatus status = sr_scene_prepare_compositing(scene, NULL);
    if (scene->compositing) return SR_ERR_ARGUMENT;
    return status == SR_ERR_RENDER ? SR_OK : status;
}

static void compositing_release(void *opaque) {
    sr_scene_invalidate_compositing(opaque);
}

static void compositing_preparation_survives_allocation_failures(sr_test_ctx *t) {
    SrScene scene;
    fx_scene(&scene, 16, 16);
    for (size_t i = 0; i < 128; ++i)
        CHECK(t, fx_add(&scene, NULL, SR_NODE_GROUP) != NULL);
    if (scene.root->child_count != 128) {
        sr_scene_free(&scene);
        return;
    }
    scene.root->transform.skew_x.base = 1;
    const OomSpec valid = {"compositing preparation/replacement",
        compositing_prepare_op, compositing_release, NULL, NULL, {SR_ERR_MEMORY}};
    CHECK(t, replay_until_success(t, &valid, &scene) >= 10);
    SrNode *last = scene.root->children[127];
    scene.root->children[127] = scene.root->children[0];
    const OomSpec invalid = {"partial invalid compositing ownership",
        compositing_invalid_op, compositing_release, NULL, NULL, {SR_ERR_MEMORY}};
    CHECK(t, replay_until_success(t, &invalid, &scene) >= 4);
    scene.root->children[127] = last;
    sr_scene_free(&scene);
}

static SrStatus resource_growth_op(void *opaque) {
    (void)opaque;
    SrCompositeResources resources;
    sr_composite_resources_init(&resources, NULL);
    unsigned char *p = sr_composite_alloc(&resources, 16, 1, 4);
    SrStatus status = SR_OK;
    if (!p) status = sr_composite_resource_status(&resources);
    else {
        p[0] = 71;
        uint64_t bytes = resources.bytes;
        void *replacement = sr_composite_realloc(&resources, p, 64, 1, 16);
        if (replacement) p = replacement;
        else {
            status = sr_composite_resource_status(&resources);
            if (resources.bytes != bytes || resources.pixels != 4 || p[0] != 71)
                status = SR_ERR_ARGUMENT;
        }
        sr_composite_free(&resources, p);
    }
    if (resources.bytes || resources.pixels) return SR_ERR_ARGUMENT;
    return status;
}

typedef struct {
    SrScene scene;
    SrFrame frame;
    SrCompositor compositor;
    bool lighting;
    double time;
} ResourceContext;

static SrStatus resource_render_op(void *opaque) {
    ResourceContext *c = opaque;
    SrStatus status = c->lighting
        ? sr_compositor_render_scene(&c->compositor, &c->scene, .5, &c->frame, NULL)
        : sr_compositor_render(&c->compositor, &c->scene, c->time, &c->frame, NULL);
    if (c->compositor.resources || c->compositor.pool || c->compositor.queue)
        return SR_ERR_ARGUMENT;
    return status;
}

static void resource_particle_release(void *opaque) {
    ResourceContext *c = opaque;
    sr_particles_invalidate(c->scene.root->children[0]);
}

static void resources_survive_allocation_failures(sr_test_ctx *t) {
    const OomSpec growth = {"compositing resource growth", resource_growth_op,
        NULL, NULL, NULL, {SR_ERR_MEMORY}};
    CHECK_INT(t, replay_until_success(t, &growth, NULL), 2);
    ResourceContext c = {0};
    fx_scene(&c.scene, 16, 16);
    sr_compositor_init(&c.compositor, 1);
    bool built = true;
    for (size_t g = 0; g < 2 && built; ++g) {
        SrNode *group = fx_add(&c.scene, NULL, SR_NODE_GROUP);
        if (!group) { built = false; break; }
        group->blend = SR_BLEND_COLOR_BURN;
        group->masks = sr_alloc(9 * sizeof(*group->masks));
        if (!group->masks) { built = false; break; }
        group->mask_count = 9;
        for (size_t i = 0; i < 9; ++i)
            group->masks[i] = fx_mask(SR_MASK_RECT, 0, 0, 16, 16, false);
        for (size_t i = 0; i < 70 && built; ++i)
            built = fx_rect(&c.scene, group, 0, 0, 8, 8,
                             (SrColor){0.5, 0.8, 0.2, 0.1}, 1) != NULL;
    }
    CHECK(t, built);
    CHECK_INT(t, sr_scene_prepare_compositing(&c.scene, NULL), SR_OK);
    CHECK_INT(t, sr_frame_init(&c.frame, 16, 16), SR_OK);
    const OomSpec render = {"bounded compositor queues/pools/masks",
        resource_render_op, NULL, NULL, NULL, {SR_ERR_MEMORY}};
    if (built && c.scene.compositing && c.frame.px)
        CHECK(t, replay_until_success(t, &render, &c) >= 12);
    sr_compositor_free(&c.compositor);
    sr_frame_free(&c.frame);
    sr_scene_free(&c.scene);

    c = (ResourceContext){0};
    built = resource_geometry_scene(&c.scene);
    sr_compositor_init(&c.compositor, 1);
    CHECK(t, built);
    CHECK_INT(t, sr_scene_prepare_compositing(&c.scene, NULL), SR_OK);
    CHECK_INT(t, sr_frame_init(&c.frame, 64, 64), SR_OK);
    const OomSpec geometry = {"bounded length/mesh/soft geometry",
        resource_render_op, NULL, NULL, NULL, {SR_ERR_MEMORY}};
    if (built && c.scene.compositing && c.frame.px)
        CHECK(t, replay_until_success(t, &geometry, &c) >= 10);
    CHECK_INT(t, resource_render_op(&c), SR_OK);
    sr_compositor_free(&c.compositor);
    sr_frame_free(&c.frame);
    sr_scene_free(&c.scene);

    c = (ResourceContext){.time = 12.5};
    built = resource_particle_scene(&c.scene);
    sr_compositor_init(&c.compositor, 1);
    CHECK(t, built);
    CHECK_INT(t, sr_scene_prepare_compositing(&c.scene, NULL), SR_OK);
    CHECK_INT(t, sr_frame_init(&c.frame, 96, 96), SR_OK);
    const OomSpec particles = {"bounded particle cache/walk/output/queue",
        resource_render_op, resource_particle_release, NULL, NULL, {SR_ERR_MEMORY}};
    if (built && c.scene.compositing && c.frame.px)
        CHECK(t, replay_until_success(t, &particles, &c) >= 10);
    CHECK_INT(t, resource_render_op(&c), SR_OK);
    sr_compositor_free(&c.compositor);
    sr_frame_free(&c.frame);
    sr_scene_free(&c.scene);

    c = (ResourceContext){.lighting = true};
    built = resource_card_scene(&c.scene);
    sr_compositor_init(&c.compositor, 1);
    CHECK(t, built);
    CHECK_INT(t, sr_scene_prepare_compositing(&c.scene, NULL), SR_OK);
    CHECK_INT(t, sr_frame_init(&c.frame, 64, 64), SR_OK);
    const OomSpec cards = {"bounded projective card/depth/shared mask",
        resource_render_op, NULL, NULL, NULL, {SR_ERR_MEMORY}};
    if (built && c.scene.compositing && c.frame.px)
        CHECK(t, replay_until_success(t, &cards, &c) >= 12);
    CHECK_INT(t, resource_render_op(&c), SR_OK);
    sr_compositor_free(&c.compositor);
    sr_frame_free(&c.frame);
    sr_scene_free(&c.scene);
}

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
    check_load(t, "tests/data-animation.xml", 23);
    check_load(t, "tests/data-animation-hosts.xml", 20);
    check_load(t, "tests/data-styles.xml", 60);
    check_load(t, "tests/data-metadata.xml", 50);
    check_load(t, "tests/data-lengths.xml", 50);
    check_load(t, "tests/golden/skew.xml", 40);
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
    const char *fixture;
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
    c->loaded = sr_scene_load_xml(sr_test_data_path(c->fixture),
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
    for (size_t i = 0; i < 4; ++i) {
        RenderContext c = {.threads = threads[i % 2],
            .fixture = i == 3 ? "tests/data-animation-hosts.xml" :
                i == 2 ? "tests/data-animation.xml" : "tests/data-oom.xml"};
        snprintf(c.path, sizeof(c.path), "%s", sr_test_tmp_path("oom-frame.ppm"));
        render_prepare(&c);
        if (!c.loaded) { SR_FAIL(t, "could not load render fixture"); return; }
        /* The uninjected frame every surviving replay must reproduce. */
        if (render_op(&c) != SR_OK || !read_all(c.path, &c.reference, &c.reference_size)) {
            SR_FAIL(t, "reference render of %s failed", c.fixture);
            sr_scene_free(&c.scene);
            return;
        }
        render_release(&c);
        render_prepare(&c);
        char what[64];
        snprintf(what, sizeof(what), "%s render (threads=%u)", c.fixture, c.threads);
        const OomSpec spec = {what, render_op, render_release, render_prepare,
                              render_same_result,
                              {SR_ERR_MEMORY}};
        long n = replay_until_success(t, &spec, &c);
        CHECK(t, n >= (i >= 2 ? 12 : 21));
        free(c.reference);
        if (c.loaded) sr_scene_free(&c.scene);
    }
}

typedef struct {
    SrScene scene;
    SrFrame raster;
    SrCompositor compositor;
    SrLengthFrame lengths;
    unsigned mode;
    bool ready;
} LengthOomContext;

static void length_prepare(void *opaque) {
    LengthOomContext *c = opaque;
    fx_scene(&c->scene, 64, 48);
    c->ready = false;
    c->scene.has_relative_lengths = c->scene.has_cards = true;
    c->scene.project.duration = .02;
    c->scene.physics.enabled = true;
    c->scene.physics.fixed_step = .01;
    c->scene.physics.gravity_y = 0;
    SrNode *group = fx_add(&c->scene, NULL, SR_NODE_GROUP);
    if (!group) return;
    group->card = true;
    group->transform.rotation_y.base = 25;
    SrCamera *camera = c->scene.cameras = sr_alloc(sizeof(*camera));
    if (!camera) return;
    c->scene.camera_count = c->scene.camera_capacity = 1;
    camera->active = camera->zoom_set = true;
    camera->zoom.base = 100;
    camera->z.base = -100;
    camera->near_plane = 1;
    camera->far_plane = 500;
    for (size_t i = 0; i < 70; ++i) {
        SrNode *node = fx_rect(&c->scene, group, (double)(i % 10) * 8, (double)(i / 10) * 10,
                               10, 10, (SrColor){.8, .4, .2, 1}, .9);
        if (!node) return;
        node->transform.x.unit = node->transform.y.unit = SR_LENGTH_PERCENT;
        node->shape_width_unit = node->shape_height_unit = SR_LENGTH_PERCENT;
        for (size_t k = 0; k < 9; ++k) {
            SrMask mask = fx_mask(SR_MASK_RECT, 0, 0, 90, 90, false);
            mask.width.unit = mask.height.unit = SR_LENGTH_PERCENT;
            if (sr_node_add_mask(node, mask) != SR_OK) return;
        }
        if (i < 2) node->body.type = i == 0 ? SR_BODY_DYNAMIC : SR_BODY_STATIC;
        if (i == 0)
            node->soft_body = (SrSoftBody){.enabled = true, .rows = 3, .cols = 3,
                .mass = 1, .stiffness = 2, .damping = .1, .pin = SR_PIN_TOP};
    }
    SrConstraint *constraint = c->scene.physics.constraints = sr_alloc(sizeof(*constraint));
    if (!constraint) return;
    c->scene.physics.constraint_count = c->scene.physics.constraint_capacity = 1;
    *constraint = (SrConstraint){.type = SR_CONSTRAINT_SPRING, .a = group->children[0],
        .b = group->children[1], .stiffness = 2};
    sr_compositor_init(&c->compositor, 1);
    c->ready = sr_frame_init(&c->raster, 64, 48) == SR_OK;
}

static void length_release(void *opaque) {
    LengthOomContext *c = opaque;
    sr_length_frame_free(&c->lengths);
    sr_compositor_free(&c->compositor);
    sr_frame_free(&c->raster);
    sr_scene_free(&c->scene);
    c->ready = false;
}

static SrStatus length_operation(void *opaque) {
    LengthOomContext *c = opaque;
    if (!c->ready) return SR_ERR_ARGUMENT;
    SrDiagnostics diag = quiet_diag("oom-length");
    if (c->mode == 2) return sr_physics_prepare(&c->scene, &diag);
    SrStatus status;
    if (c->mode == 0) {
        status = sr_length_frame_prepare(&c->lengths, &c->scene, .01, false, &diag);
        if (status == SR_ERR_MEMORY &&
            sr_length_frame_prepare(&c->lengths, &c->scene, 0, false, &diag) != SR_OK)
            return SR_ERR_RENDER;
    } else {
        const float black[4] = {0, 0, 0, 1};
        sr_frame_clear(&c->raster, black, 1);
        status = sr_compositor_render_scene(&c->compositor, &c->scene, .01, &c->raster, &diag);
        if (status == SR_ERR_MEMORY) {
            sr_frame_clear(&c->raster, black, 1);
            if (sr_compositor_render_scene(&c->compositor, &c->scene, 0,
                                            &c->raster, &diag) != SR_OK)
                return SR_ERR_RENDER;
        }
    }
    return status;
}

static void relative_lengths_survive_allocation_failures(sr_test_ctx *t) {
    const char *names[] = {"relative geometry + recovery", "relative card + recovery",
                           "relative rigid/soft/constraint preparation"};
    for (unsigned mode = 0; mode < 3; ++mode) {
        LengthOomContext c = {.mode = mode};
        length_prepare(&c);
        CHECK(t, c.ready);
        if (c.ready) {
            const OomSpec spec = {names[mode], length_operation, length_release,
                length_prepare, NULL, {SR_ERR_MEMORY}};
            long count = replay_until_success(t, &spec, &c);
            CHECK(t, count >= 6);
        }
        length_release(&c);
    }
}

/* ---------------------------------------------------------------- encoder */

static SrStatus mixer_op(void *opaque) {
    SceneContext *c = opaque;
    SrMixer *mixer = NULL;
    SrStatus status = sr_mixer_create(&c->scene, &mixer);
    float samples[512];
    if (status == SR_OK) sr_mixer_mix(mixer, 24000, 256, samples);
    sr_mixer_destroy(mixer);
    return status;
}

static void animated_mixer_survives_allocation_failures(sr_test_ctx *t) {
    SceneContext c;
    SrDiagnostics diag = quiet_diag("oom-mixer");
    if (sr_scene_load_xml(sr_test_data_path("tests/data-animation-hosts.xml"),
                         &c.scene, &diag) != SR_OK) {
        SR_FAIL(t, "mixer scene load");
        return;
    }
    if (sr_audio_load(&c.scene, &diag) == SR_OK) {
        const OomSpec spec = {"animated mixer", mixer_op, NULL, NULL, NULL,
                              {SR_ERR_MEMORY}};
        CHECK_INT(t, replay_until_success(t, &spec, &c), 2);
    } else {
        SR_FAIL(t, "mixer asset load");
    }
    sr_scene_free(&c.scene);
}

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
    RenderContext c = {.threads = 2, .loaded = true, .fixture = "tests/data-oom.xml"};
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
    {"prepared_paths_survive_allocation_failures",
     prepared_paths_survive_allocation_failures},
    {"mask_paths_survive_allocation_failures",
     mask_paths_survive_allocation_failures},
    {"compositing_preparation_survives_allocation_failures",
     compositing_preparation_survives_allocation_failures},
    {"resources_survive_allocation_failures",
     resources_survive_allocation_failures},
    {"color_parse_survives_allocation_failures",
     color_parse_survives_allocation_failures},
    {"repeated_render_leaks_nothing", repeated_render_leaks_nothing},
    {"xml_load_survives_allocation_failures", xml_load_survives_allocation_failures},
    {"asset_load_survives_allocation_failures", asset_load_survives_allocation_failures},
    {"animated_mixer_survives_allocation_failures", animated_mixer_survives_allocation_failures},
    {"frame_render_survives_allocation_failures", frame_render_survives_allocation_failures},
    {"relative_lengths_survive_allocation_failures",
     relative_lengths_survive_allocation_failures},
    {"encoder_survives_allocation_failures", encoder_survives_allocation_failures},
    {NULL, NULL},
};
