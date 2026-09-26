/* SPDX-License-Identifier: Apache-2.0 */
/* Seeded generator for the B1-5 parsers: the markers section, generated
 * beat ids, node tags/markers, sequences and key marker references. Each
 * seed yields valid documents, single-byte mutations and truncations; every
 * input must load or fail with a diagnostic, never crash, and loading is
 * deterministic. */
#include "scene_text.h"
#include "scene_render/markers.h"
#include "scene_render/random.h"

typedef struct {
    char *text;
    size_t used, size;
} Buffer;

#include <stdarg.h>

__attribute__((format(printf, 2, 3)))
static void put(Buffer *buffer, const char *format, ...) {
    if (buffer->used + 1 >= buffer->size) return;
    va_list args;
    va_start(args, format);
    int added = vsnprintf(buffer->text + buffer->used, buffer->size - buffer->used,
                          format, args);
    va_end(args);
    if (added > 0) buffer->used += (size_t)added;
    if (buffer->used >= buffer->size) buffer->used = buffer->size - 1;
}

static const char *reference(uint64_t value) {
    static const char *const ids[] = {"m0", "m1", "m2", "beat.1", "beat.3", "bar.2",
                                      "beat.40", "bar.0", "s0", "none"};
    return ids[value % (sizeof(ids) / sizeof(ids[0]))];
}

typedef unsigned long long ull;

static void generate(Buffer *buffer, uint64_t value) {
    put(buffer, "<scene version=\"1.1\"><project width=\"16\" height=\"8\" fps=\"4\" "
        "duration=\"%llu\"/><markers>", (ull)(2 + value % 3));
    for (uint64_t i = 0; i < 3; ++i) {
        value = sr_random_mix64(value);
        put(buffer, "<marker id=\"m%llu\" time=\"%llu.25\" kind=\"%s\" label=\"L%llu\"/>",
            (ull)i, (ull)(value % 3), value % 2 ? "cue" : "section", (ull)i);
    }
    put(buffer, "<beatGrid bpm=\"%llu\" offset=\"0.%llu\"/></markers><composition>",
        (ull)(60 + value % 120), (ull)(value % 10));
    value = sr_random_mix64(value);
    put(buffer, "<sequence id=\"s0\" timeGap=\"0.%llu\" timeScale=\"%llu\" tags=\"a b\">",
        (ull)(value % 9), (ull)(1 + value % 3));
    static const char *const bases[] = {"composition", "local", "normalized"};
    for (uint64_t i = 0; i < 3; ++i) {
        value = sr_random_mix64(value);
        put(buffer, "<shape id=\"c%llu\" shape=\"rect\" width=\"2\" height=\"2\" "
            "end=\"%llu\" name=\"n%llu\" tags=\"t%llu\">", (ull)i, (ull)(1 + value % 2),
            (ull)i, (ull)(value % 5));
        put(buffer, "<animate property=\"opacity\" timeBase=\"%s\">"
            "<key time=\"0\" value=\"0\"/><key time=\"0.%llu\" value=\"1\"/>"
            "<key marker=\"%s\" time=\"0.%llu\" value=\"0.5\"/></animate></shape>",
            bases[value % 3], (ull)(1 + value % 8), reference(value >> 8),
            (ull)((value >> 16) % 9));
    }
    value = sr_random_mix64(value);
    put(buffer, "</sequence><group id=\"g\" timeOffset=\"0.%llu\" startMarker=\"%s\">"
        "<shape id=\"d\" shape=\"ellipse\" width=\"2\" height=\"2\"/></group>"
        "</composition></scene>", (ull)(value % 9), reference(value >> 12));
}

typedef struct {
    SrStatus status;
    double starts[8];
} Outcome;

static Outcome load_once(sr_test_ctx *t, const char *xml) {
    Outcome outcome = {0};
    SrScene scene;
    char *message = NULL;
    outcome.status = st_load(t, "fuzz-markers.xml", xml, &scene, &message);
    CHECK(t, outcome.status == SR_OK || outcome.status == SR_ERR_XML);
    if (outcome.status != SR_OK) CHECK(t, message && strstr(message, "error"));
    free(message);
    if (outcome.status != SR_OK) return outcome;
    static const char *const ids[] = {"s0", "c0", "c1", "c2", "g", "d"};
    for (size_t i = 0; i < sizeof(ids) / sizeof(ids[0]); ++i) {
        const SrNode *node = sr_scene_find_node(&scene, ids[i]);
        if (!node) continue;
        CHECK(t, isfinite(node->start_time));
        CHECK(t, node->end_time > node->start_time);
        outcome.starts[i] = node->start_time;
        const SrTrack *track = &node->opacity.track;
        for (size_t k = 1; k < track->count; ++k)
            CHECK(t, track->keys[k].time > track->keys[k - 1].time);
    }
    if (scene.timeline) CHECK_INT(t, scene.timeline->pending_tracks, 0);
    sr_scene_free(&scene);
    return outcome;
}

static void check_input(sr_test_ctx *t, const char *xml) {
    Outcome first = load_once(t, xml), second = load_once(t, xml);
    CHECK_INT(t, first.status, second.status);
    CHECK(t, !memcmp(first.starts, second.starts, sizeof(first.starts)));
}

static void seeded_documents(sr_test_ctx *t) {
    const uint64_t seeds[] = {0, 1, UINT64_C(0x9e3779b97f4a7c15), UINT64_MAX};
    const char mutations[] = "<>\"=/ .-0159abmr\t\xff";
    char text[8192], edited[8192];
    for (size_t seed = 0; seed < sizeof(seeds) / sizeof(seeds[0]); ++seed) {
        for (uint64_t sample = 0; sample < 12; ++sample) {
            uint64_t value = sr_random_mix64(seeds[seed] ^ (sample * 0x100000001b3ull));
            Buffer buffer = {text, 0, sizeof(text)};
            generate(&buffer, value);
            check_input(t, text);
            size_t used = strlen(text);
            memcpy(edited, text, used + 1);
            edited[(value >> 7) % used] = mutations[(value >> 29) % (sizeof(mutations) - 1)];
            check_input(t, edited);
            memcpy(edited, text, used + 1);
            edited[(value >> 37) % used] = '\0';
            check_input(t, edited);
        }
    }
}

static void generated_id_strings(sr_test_ctx *t) {
    SrTimeline timeline = {0};
    timeline.grid = (SrBeatGrid){.present = true, .bpm = 150, .offset = 0.1,
                                 .beats_per_bar = 5};
    const char *problem;
    size_t line;
    CHECK_INT(t, sr_timeline_prepare(&timeline, 30.0, &problem, &line), SR_OK);
    const char alphabet[] = "beatr.0123456789+-x";
    for (uint64_t sample = 0; sample < 4096; ++sample) {
        uint64_t value = sr_random_mix64(sample);
        char id[24];
        size_t length = 1 + value % 22;
        for (size_t i = 0; i < length; ++i) {
            value = sr_random_mix64(value);
            id[i] = alphabet[value % (sizeof(alphabet) - 1)];
        }
        if (sample % 3 == 0 && length >= 5) memcpy(id, "beat.", 5);
        id[length] = '\0';
        bool bar;
        uint64_t number;
        bool parsed = sr_beat_id_parse(id, &bar, &number);
        double time = 0;
        SrMarkerLookup found = sr_timeline_lookup(&timeline, id, &time);
        CHECK(t, found != SR_MARKER_LOOKUP_FOUND || parsed);
        if (found == SR_MARKER_LOOKUP_FOUND) {
            CHECK(t, number >= 1);
            CHECK(t, time >= 0.1 && time <= 30.0);
            char canonical[32];
            snprintf(canonical, sizeof(canonical), "%s.%llu", bar ? "bar" : "beat",
                     (unsigned long long)number);
            CHECK_STR(t, canonical, id);
        }
    }
    free(timeline.index);
}

const sr_test_case sr_tests_fuzz_markers[] = {
    {"seeded_documents", seeded_documents},
    {"generated_id_strings", generated_id_strings},
    {NULL, NULL},
};
