/* SPDX-License-Identifier: Apache-2.0 */
/* B1-5 timeline structure: markers, beat grids, generated ids, group
 * clocks, sequences, key snapping, names and tags. References are closed
 * forms computed by hand in each test (docs/design/b1-5-timeline.md). */
#include "fixture.h"
#include "scene_text.h"
#include "scene_render/markers.h"

#include <float.h>

#define HEAD(duration) "<scene version=\"1.1\"><project width=\"32\" height=\"16\" " \
    "fps=\"8\" duration=\"" duration "\"/>"
#define GRID "<markers><marker id=\"m\" time=\"0.5\"/><marker id=\"n\" time=\"1.5\"/>" \
    "<beatGrid bpm=\"120\" offset=\"0.25\" beatsPerBar=\"3\"/></markers>"

static SrStatus load(sr_test_ctx *t, const char *xml, SrScene *scene, char **message) {
    return st_load(t, "markers.xml", xml, scene, message);
}

static void expect_error(sr_test_ctx *t, const char *xml, const char *diagnostic) {
    SrScene scene;
    char *message = NULL;
    SrStatus status = load(t, xml, &scene, &message);
    CHECK_INT(t, status, SR_ERR_XML);
    CHECK_CONTAINS(t, message, diagnostic);
    if (status == SR_OK) sr_scene_free(&scene);
    free(message);
}

static bool expect_ok(sr_test_ctx *t, const char *xml, SrScene *scene) {
    char *message = NULL;
    SrStatus status = load(t, xml, scene, &message);
    CHECK_INT(t, status, SR_OK);
    if (status != SR_OK) fprintf(stderr, "    %s", message ? message : "");
    free(message);
    return status == SR_OK;
}

/* ---- beat grid and ids ---------------------------------------------------- */

static SrTimeline grid_timeline(double bpm, double offset, uint32_t beats) {
    SrTimeline timeline = {0};
    timeline.grid = (SrBeatGrid){.present = true, .bpm = bpm, .offset = offset,
                                 .beats_per_bar = beats, .source_line = 7};
    return timeline;
}

static void beat_grid_closed_form(sr_test_ctx *t) {
    SrTimeline timeline = grid_timeline(120, 0.5, 3);
    const char *problem;
    size_t line;
    CHECK_INT(t, sr_timeline_prepare(&timeline, 10.0, &problem, &line), SR_OK);
    /* 0.5 + (N - 1) * 0.5 <= 10 for N <= 20; bars start at beats 1, 4, ..., 19. */
    CHECK_INT(t, timeline.grid.beat_count, 20);
    CHECK_INT(t, timeline.grid.bar_count, 7);
    double time = -1;
    CHECK_INT(t, sr_timeline_lookup(&timeline, "beat.1", &time), SR_MARKER_LOOKUP_FOUND);
    CHECK(t, time == 0.5);
    CHECK_INT(t, sr_timeline_lookup(&timeline, "beat.20", &time), SR_MARKER_LOOKUP_FOUND);
    CHECK(t, time == 10.0);
    CHECK_INT(t, sr_timeline_lookup(&timeline, "bar.7", &time), SR_MARKER_LOOKUP_FOUND);
    CHECK(t, time == 9.5);
    CHECK(t, sr_beat_grid_bar_time(&timeline.grid, 3) ==
             sr_beat_grid_beat_time(&timeline.grid, 7));
    CHECK_INT(t, sr_timeline_lookup(&timeline, "beat.21", &time),
              SR_MARKER_LOOKUP_OUT_OF_GRID);
    CHECK_INT(t, sr_timeline_lookup(&timeline, "bar.8", &time),
              SR_MARKER_LOOKUP_OUT_OF_GRID);
    CHECK_INT(t, sr_timeline_lookup(&timeline, "beat.x", &time), SR_MARKER_LOOKUP_UNKNOWN);
    /* A non-dyadic tempo: bar and beat instants are bitwise equal. */
    SrTimeline odd = grid_timeline(97, 0.1, 7);
    CHECK_INT(t, sr_timeline_prepare(&odd, 1000.0, &problem, &line), SR_OK);
    for (uint64_t bar = 1; bar <= odd.grid.bar_count; ++bar)
        CHECK(t, sr_beat_grid_bar_time(&odd.grid, bar) ==
                 sr_beat_grid_beat_time(&odd.grid, (bar - 1) * 7 + 1));
    CHECK(t, sr_beat_grid_beat_time(&odd.grid, odd.grid.beat_count) <= 1000.0);
    CHECK(t, sr_beat_grid_beat_time(&odd.grid, odd.grid.beat_count + 1) > 1000.0);
    /* A grid starting after the project end generates nothing. */
    SrTimeline late = grid_timeline(60, 11, 4);
    CHECK_INT(t, sr_timeline_prepare(&late, 10.0, &problem, &line), SR_OK);
    CHECK_INT(t, late.grid.beat_count, 0);
    CHECK_INT(t, sr_timeline_lookup(&late, "beat.1", &time), SR_MARKER_LOOKUP_OUT_OF_GRID);
    /* A negative offset keeps beat.1 at the offset. */
    SrTimeline early = grid_timeline(60, -2.5, 4);
    CHECK_INT(t, sr_timeline_prepare(&early, 1.0, &problem, &line), SR_OK);
    CHECK_INT(t, early.grid.beat_count, 4);
    free(timeline.index);
    free(odd.index);
    free(late.index);
    free(early.index);
}

static void generated_id_syntax(sr_test_ctx *t) {
    bool bar = false;
    uint64_t number = 0;
    CHECK(t, sr_beat_id_parse("beat.1", &bar, &number) && !bar && number == 1);
    CHECK(t, sr_beat_id_parse("bar.1234567", &bar, &number) && bar && number == 1234567);
    CHECK(t, sr_beat_id_parse("beat.1234567890123456789", &bar, &number));
    static const char *const rejected[] = {"beat.0", "beat.01", "beat.+1", "beat.",
        "beat.1x", "beat.-1", "beats.1", "Beat.1", "bar.12345678901234567890",
        "bar", "", "beat.1 "};
    for (size_t i = 0; i < sizeof(rejected) / sizeof(rejected[0]); ++i)
        CHECK(t, !sr_beat_id_parse(rejected[i], &bar, &number));
    SrTimeline timeline = grid_timeline(60, 0, 4);
    const char *problem;
    size_t line;
    CHECK_INT(t, sr_timeline_prepare(&timeline, 3.0, &problem, &line), SR_OK);
    CHECK(t, sr_timeline_generates(&timeline, "beat.4"));
    CHECK(t, !sr_timeline_generates(&timeline, "beat.5"));
    CHECK(t, sr_timeline_generates(&timeline, "bar.1"));
    CHECK(t, !sr_timeline_generates(&timeline, "bar.2"));
    free(timeline.index);
}

static void generated_limit(sr_test_ctx *t) {
    const char *problem;
    size_t line;
    SrTimeline exact = grid_timeline(60, 0, 1);
    CHECK_INT(t, sr_timeline_prepare(&exact, SR_MAX_GENERATED_MARKERS - 1.0,
                                     &problem, &line), SR_OK);
    CHECK_INT(t, exact.grid.beat_count, SR_MAX_GENERATED_MARKERS);
    SrTimeline over = grid_timeline(60, 0, 1);
    CHECK_INT(t, sr_timeline_prepare(&over, (double)SR_MAX_GENERATED_MARKERS,
                                     &problem, &line), SR_ERR_XML);
    CHECK_CONTAINS(t, problem, "1048576 beats");
    CHECK_INT(t, line, 7);
    SrTimeline huge = grid_timeline(SR_MAX_BPM, -1e6, 1);
    CHECK_INT(t, sr_timeline_prepare(&huge, 1e6, &problem, &line), SR_ERR_XML);
    free(exact.index);
    expect_error(t, HEAD("1000") "<markers><beatGrid bpm=\"1000000\"/></markers>"
                 "<composition/></scene>", "more than 1048576 beats");
}

static void marker_table(sr_test_ctx *t) {
    SrScene scene;
    if (expect_ok(t, HEAD("2") "<markers><marker id=\"z\" time=\"1\" duration=\"0.5\" "
                  "kind=\"section\" label=\"Zed\" color=\"#FF0000\"/><marker time=\"0.2\"/>"
                  "<marker id=\"a\" time=\"-3\" kind=\"beat\"/></markers>"
                  "<composition/></scene>", &scene)) {
        const SrTimeline *timeline = scene.timeline;
        CHECK(t, timeline != NULL);
        if (timeline) {
            CHECK_INT(t, timeline->marker_count, 3);
            CHECK_STR(t, timeline->markers[0].label, "Zed");
            CHECK_INT(t, timeline->markers[0].kind, SR_MARKER_SECTION);
            CHECK(t, timeline->markers[0].duration == 0.5);
            CHECK(t, timeline->markers[0].color_set);
            CHECK(t, timeline->markers[1].id == NULL);
            CHECK_INT(t, timeline->markers[1].kind, SR_MARKER_CUE);
            CHECK_INT(t, timeline->index_count, 2);
            double time = 0;
            CHECK_INT(t, sr_timeline_lookup(timeline, "a", &time), SR_MARKER_LOOKUP_FOUND);
            CHECK(t, time == -3.0);
            CHECK_INT(t, sr_timeline_lookup(timeline, "zz", &time), SR_MARKER_LOOKUP_UNKNOWN);
        }
        sr_scene_free(&scene);
    }
    expect_error(t, HEAD("1") "<markers><marker id=\"x\" time=\"2000000\"/></markers>"
                 "<composition/></scene>", "at most 1e6 seconds");
    expect_error(t, HEAD("1") "<markers><marker id=\"x\" time=\"0\" duration=\"-1\"/>"
                 "</markers><composition/></scene>", "duration");
    expect_error(t, HEAD("1") "<markers><beatGrid bpm=\"60\"/><beatGrid bpm=\"90\"/>"
                 "</markers><composition/></scene>", "only one beatGrid");
    expect_error(t, HEAD("4") "<markers><marker id=\"beat.2\" time=\"0\"/>"
                 "<beatGrid bpm=\"60\"/></markers><composition/></scene>",
                 "generated by beatGrid");
    expect_error(t, HEAD("4") "<markers><beatGrid bpm=\"60\"/></markers><composition>"
                 "<shape id=\"bar.1\" shape=\"rect\" width=\"1\" height=\"1\"/>"
                 "</composition></scene>", "id 'bar.1' equals an id generated");
    /* Ids beyond the grid are ordinary ids. */
    if (expect_ok(t, HEAD("4") "<markers><beatGrid bpm=\"60\"/></markers><composition>"
                  "<shape id=\"beat.9\" shape=\"rect\" width=\"1\" height=\"1\"/>"
                  "</composition></scene>", &scene))
        sr_scene_free(&scene);
    expect_error(t, HEAD("1") "<markers><beatGrid bpm=\"2000000\"/></markers>"
                 "<composition/></scene>", "(0,1e6] bpm");
    expect_error(t, HEAD("1") "<markers><beatGrid bpm=\"60\" beatsPerBar=\"1025\"/>"
                 "</markers><composition/></scene>", "[1,1024]");
    expect_error(t, HEAD("4") "<assets><image id=\"beat.1\" src=\"x.png\" width=\"1\" "
                 "height=\"1\"/></assets><markers><beatGrid bpm=\"60\"/></markers>"
                 "<composition/></scene>", "id 'beat.1' equals an id generated");
    expect_error(t, HEAD("4") "<markers><beatGrid bpm=\"60\"/></markers><composition/>"
                 "<effects><effect id=\"bar.1\" type=\"glow\"/></effects></scene>",
                 "id 'bar.1' equals an id generated");
    expect_error(t, HEAD("1") "<markers><marker time=\"0\" kind=\"chapter\"/></markers>"
                 "<composition/></scene>", "unsupported in this build");
    expect_error(t, HEAD("1") "<markers><beatGrid bpm=\"60\" source=\"x\"/></markers>"
                 "<composition/></scene>", "unsupported in this build");
    expect_error(t, "<scene version=\"1.0\"><project width=\"32\" height=\"16\" fps=\"8\" "
                 "duration=\"1\"/><markers><marker time=\"0\"/></markers>"
                 "<composition/></scene>", "requires version=\"1.1\"");
    char label[SR_MAX_MARKER_LABEL + 2];
    memset(label, 'x', sizeof(label) - 1);
    label[sizeof(label) - 1] = '\0';
    char xml[SR_MAX_MARKER_LABEL + 512];
    snprintf(xml, sizeof(xml), HEAD("1") "<markers><marker time=\"0\" label=\"%s\"/>"
             "</markers><composition/></scene>", label);
    expect_error(t, xml, "label exceeds 4096 bytes");
}

static void marker_count_limit(sr_test_ctx *t) {
    size_t size = (SR_MAX_MARKERS + 1) * 20 + 512;
    char *xml = malloc(size);
    CHECK(t, xml != NULL);
    if (!xml) return;
    size_t used = (size_t)snprintf(xml, size, HEAD("1") "<markers>");
    for (size_t i = 0; i <= SR_MAX_MARKERS; ++i)
        used += (size_t)snprintf(xml + used, size - used, "<marker time=\"0\"/>");
    snprintf(xml + used, size - used, "</markers><composition/></scene>");
    expect_error(t, xml, "marker limit is 65536");
    free(xml);
}

/* ---- clocks --------------------------------------------------------------- */

static void nested_group_clocks(sr_test_ctx *t) {
    SrScene scene;
    /* Outer: s=1, q=2, o=0.5 -> a=2, b=-0.5. Inner (start 2 on that clock):
     * q=0.5, o=-1 -> a=1, b=(-0.5-2)*0.5+2-1=-0.25. Leaf [3,4) -> [3.25,4.25). */
    if (!expect_ok(t, HEAD("8") "<composition>"
        "<group id=\"outer\" start=\"1\" timeScale=\"2\" timeOffset=\"0.5\">"
        "<group id=\"inner\" start=\"2\" timeScale=\"0.5\" timeOffset=\"-1\">"
        "<shape id=\"leaf\" shape=\"rect\" width=\"2\" height=\"2\" start=\"3\" end=\"4\">"
        "<animate property=\"position.x\"><key time=\"3\" value=\"0\"/><key time=\"4\" value=\"100\"/></animate>"
        "<animate property=\"position.y\" timeBase=\"local\"><key time=\"0\" value=\"0\"/><key time=\"1\" value=\"10\"/></animate>"
        "<animate property=\"rotation\" timeBase=\"normalized\"><key time=\"0\" value=\"0\"/><key time=\"1\" value=\"90\"/></animate>"
        "</shape></group></group></composition></scene>", &scene)) return;
    const SrNode *outer = sr_scene_find_node(&scene, "outer");
    const SrNode *inner = sr_scene_find_node(&scene, "inner");
    const SrNode *leaf = sr_scene_find_node(&scene, "leaf");
    CHECK(t, outer && inner && leaf);
    if (outer && inner && leaf) {
        CHECK(t, outer->start_time == 1.0 && outer->clock_scale == 1.0);
        CHECK(t, inner->start_time == 1.25);
        CHECK(t, inner->clock_scale == 2.0 && inner->clock_offset == -0.5);
        CHECK(t, leaf->clock_scale == 1.0 && leaf->clock_offset == -0.25);
        CHECK(t, leaf->start_time == 3.25 && leaf->end_time == 4.25);
        const SrTrack *x = &leaf->transform.x.track;
        CHECK(t, x->clock_set && x->clock_scale == 1.0 && x->clock_offset == -0.25);
        CHECK(t, x->domain_start == 3.25 && x->domain_end == 4.25);
        CHECK(t, leaf->transform.y.track.clock_offset == -3.25);
        CHECK(t, leaf->transform.rotation.track.clock_offset == -3.25);
        CHECK(t, leaf->transform.rotation.track.seconds_per_unit == 1.0);
        CHECK_NEAR(t, sr_anim_eval(&leaf->transform.x, 3.75), 50.0, 1e-12);
        CHECK_NEAR(t, sr_anim_eval(&leaf->transform.y, 3.75), 5.0, 1e-12);
        CHECK_NEAR(t, sr_anim_eval(&leaf->transform.rotation, 4.0), 67.5, 1e-12);
    }
    sr_scene_free(&scene);
    /* The combined scale is bounded even when each factor is valid. */
    expect_error(t, HEAD("1") "<composition><group id=\"a\" timeScale=\"1000\">"
                 "<group id=\"b\" timeScale=\"10000\"/></group></composition></scene>",
                 "combined time scale");
    expect_error(t, HEAD("1") "<composition><group id=\"a\" timeScale=\"2000000\"/>"
                 "</composition></scene>", "[1e-6,1e6]");
    /* A normalized track on a node that starts after the project end. */
    expect_error(t, HEAD("1") "<composition><shape id=\"s\" shape=\"rect\" width=\"1\" "
                 "height=\"1\" start=\"2\"><animate property=\"opacity\" "
                 "timeBase=\"normalized\"><key time=\"0\" value=\"0\"/></animate>"
                 "</shape></composition></scene>", "finite positive host span");
    expect_error(t, HEAD("1") "<composition><group id=\"a\" timeOffset=\"2e6\"/>"
                 "</composition></scene>", "at most 1e6 seconds");
}

static void default_groups_keep_absolute_children(sr_test_ctx *t) {
    SrScene scene;
    /* Identity placement keeps 1.0 acceptance: no new bound on starts. */
    if (expect_ok(t, "<scene version=\"1.0\"><project width=\"32\" height=\"16\" "
                  "fps=\"8\" duration=\"1\"/><composition>"
                  "<group id=\"g\" start=\"10000000000000\"/></composition></scene>", &scene))
        sr_scene_free(&scene);
    /* A sequence shift is bounded even for a node without tracks. */
    expect_error(t, HEAD("1") "<composition><sequence id=\"s\" start=\"1000000000000\" "
                 "timeScale=\"2\"><group id=\"a\"/></sequence></composition></scene>",
                 "exceeds 1e12 seconds");
    /* A 1.0-style group start does not shift its children; local time is
     * elapsed seconds since the node's own start (B1-1 regression). */
    if (!expect_ok(t, HEAD("8") "<composition><group id=\"g\" start=\"2\">"
        "<shape id=\"s\" shape=\"rect\" width=\"2\" height=\"2\" start=\"3\">"
        "<animate property=\"position.x\" timeBase=\"local\"><key time=\"0\" value=\"0\"/>"
        "<key time=\"1\" value=\"10\"/></animate></shape></group></composition></scene>",
        &scene)) return;
    const SrNode *s = sr_scene_find_node(&scene, "s");
    CHECK(t, s && s->start_time == 3.0 && s->clock_scale == 1.0 && s->clock_offset == 0.0);
    if (s) {
        CHECK(t, sr_anim_eval(&s->transform.x, 3.0) == 0.0);
        CHECK(t, sr_anim_eval(&s->transform.x, 3.5) == 5.0);
    }
    sr_scene_free(&scene);
}

static void sequence_placement(sr_test_ctx *t) {
    SrScene scene;
    if (!expect_ok(t, HEAD("10") "<composition>"
        "<sequence id=\"seq\" start=\"1\" timeGap=\"0.5\" name=\"Main\">"
        "<shape id=\"a\" shape=\"rect\" width=\"2\" height=\"2\" start=\"0.25\" end=\"2\" z=\"5\"/>"
        "<group id=\"b\" end=\"1\"><shape id=\"b1\" shape=\"rect\" width=\"2\" height=\"2\" start=\"0.5\">"
        "<animate property=\"position.x\"><key time=\"0.5\" value=\"0\"/><key time=\"1\" value=\"8\"/></animate>"
        "</shape></group>"
        "<shape id=\"c\" shape=\"rect\" width=\"2\" height=\"2\"/>"
        "</sequence>"
        "<sequence id=\"lap\" timeGap=\"-0.25\">"
        "<shape id=\"d\" shape=\"rect\" width=\"2\" height=\"2\" end=\"1\"/>"
        "<shape id=\"e\" shape=\"rect\" width=\"2\" height=\"2\" end=\"1\"/>"
        "</sequence></composition></scene>", &scene)) return;
    const SrNode *a = sr_scene_find_node(&scene, "a"), *b = sr_scene_find_node(&scene, "b");
    const SrNode *b1 = sr_scene_find_node(&scene, "b1"), *c = sr_scene_find_node(&scene, "c");
    const SrNode *d = sr_scene_find_node(&scene, "d"), *e = sr_scene_find_node(&scene, "e");
    CHECK(t, a && b && b1 && c && d && e);
    if (a && b && b1 && c && d && e) {
        CHECK(t, a->start_time == 1.25 && a->end_time == 3.0);
        CHECK(t, b->start_time == 3.5 && b->end_time == 4.5);
        CHECK(t, b1->start_time == 4.0 && isinf(b1->end_time));
        CHECK(t, c->start_time == 5.0 && isinf(c->end_time));
        CHECK(t, d->end_time == 1.0 && e->start_time == 0.75 && e->end_time == 1.75);
        /* Slot-relative keys: 0.5 -> 4.0 and 1 -> 4.5 on the composition. */
        CHECK(t, sr_anim_eval(&b1->transform.x, 4.25) == 4.0);
        CHECK(t, b->timeline == NULL && a->timeline == NULL);
        const SrNode *seq = sr_scene_find_node(&scene, "seq");
        CHECK(t, seq && seq->timeline && seq->timeline->sequence);
        CHECK(t, seq && seq->timeline && seq->timeline->time_gap == 0.5);
    }
    sr_scene_free(&scene);
    expect_error(t, HEAD("4") "<composition><sequence id=\"s\">"
                 "<shape id=\"a\" name=\"Open card\" shape=\"rect\" width=\"2\" height=\"2\"/>"
                 "<shape id=\"b\" shape=\"rect\" width=\"2\" height=\"2\"/>"
                 "</sequence></composition></scene>",
                 "shape 'a' (\"Open card\") needs an end");
    expect_error(t, HEAD("4") GRID "<composition><sequence id=\"s\">"
                 "<shape id=\"a\" shape=\"rect\" width=\"2\" height=\"2\" startMarker=\"m\"/>"
                 "</sequence></composition></scene>", "startMarker is not allowed");
    expect_error(t, HEAD("4") "<composition><sequence id=\"s\" transition=\"fade\"/>"
                 "</composition></scene>", "unsupported in this build");
    expect_error(t, HEAD("4") "<composition><sequence id=\"s\" transitionDuration=\"1\"/>"
                 "</composition></scene>", "unsupported in this build");
    expect_error(t, "<scene version=\"1.0\"><project width=\"32\" height=\"16\" fps=\"8\" "
                 "duration=\"1\"/><composition><sequence id=\"s\"/></composition></scene>",
                 "requires version=\"1.1\"");
    /* An item's end marker is converted into its slot. */
    if (expect_ok(t, HEAD("4") GRID "<composition><sequence id=\"s\" timeGap=\"0.25\">"
                  "<shape id=\"a\" shape=\"rect\" width=\"2\" height=\"2\" endMarker=\"m\"/>"
                  "<shape id=\"b\" shape=\"rect\" width=\"2\" height=\"2\" end=\"1\"/>"
                  "</sequence></composition></scene>", &scene)) {
        const SrNode *first = sr_scene_find_node(&scene, "a");
        const SrNode *second = sr_scene_find_node(&scene, "b");
        CHECK(t, first && first->end_time == 0.5);
        CHECK(t, second && second->start_time == 0.75 && second->end_time == 1.75);
        sr_scene_free(&scene);
    }
}

static void timed_sequence_items_meet_exactly(sr_test_ctx *t) {
    SrScene scene;
    /* A marker end whose clock round trip is inexact (0.79166666666666663
     * maps back to ...674): the next item still starts at the marker, so
     * frame 19 at 24 fps shows one of them. */
    if (expect_ok(t, "<scene version=\"1.1\"><project width=\"32\" height=\"16\" "
                  "fps=\"24\" duration=\"2\"/><markers><marker id=\"m\" "
                  "time=\"0.7916666666666666\"/></markers><composition>"
                  "<sequence id=\"s\" timeScale=\"10\" timeOffset=\"0.3\">"
                  "<shape id=\"a\" shape=\"rect\" width=\"2\" height=\"2\" endMarker=\"m\"/>"
                  "<shape id=\"b\" shape=\"rect\" width=\"2\" height=\"2\"/>"
                  "</sequence></composition></scene>", &scene)) {
        const SrNode *a = sr_scene_find_node(&scene, "a");
        const SrNode *b = sr_scene_find_node(&scene, "b");
        CHECK(t, a && b && a->end_time == 0.7916666666666666 &&
                 b->start_time == a->end_time);
        sr_scene_free(&scene);
    }
    /* The same junction reaches descendants of a group item. */
    if (expect_ok(t, "<scene version=\"1.1\"><project width=\"32\" height=\"16\" "
                  "fps=\"24\" duration=\"2\"/><markers><marker id=\"m\" "
                  "time=\"0.7916666666666666\"/><marker id=\"e\" time=\"1.2\"/>"
                  "</markers><composition>"
                  "<sequence id=\"s\" timeScale=\"10\" timeOffset=\"0.3\">"
                  "<shape id=\"a\" shape=\"rect\" width=\"2\" height=\"2\" endMarker=\"m\"/>"
                  "<group id=\"b\" endMarker=\"e\"><group id=\"c\" timeOffset=\"0\">"
                  "<shape id=\"d\" shape=\"rect\" width=\"2\" height=\"2\"/></group>"
                  "<shape id=\"f\" shape=\"rect\" width=\"2\" height=\"2\" start=\"1\"/>"
                  "</group></sequence></composition></scene>", &scene)) {
        const SrNode *a = sr_scene_find_node(&scene, "a");
        const SrNode *b = sr_scene_find_node(&scene, "b");
        const SrNode *d = sr_scene_find_node(&scene, "d");
        const SrNode *f = sr_scene_find_node(&scene, "f");
        CHECK(t, a && b && d && f);
        if (a && b && d && f) {
            CHECK(t, b->start_time == a->end_time && d->start_time == a->end_time);
            CHECK(t, b->end_time == 1.2);
            CHECK(t, f->start_time > b->start_time && isinf(f->end_time));
        }
        sr_scene_free(&scene);
    }
    if (!expect_ok(t, HEAD("10") "<composition>"
        "<sequence id=\"s\" start=\"0.3\" timeScale=\"3\" timeOffset=\"0.1\">"
        "<shape id=\"a\" shape=\"rect\" width=\"2\" height=\"2\" end=\"0.7\"/>"
        "<shape id=\"b\" shape=\"rect\" width=\"2\" height=\"2\" end=\"1.1\"/>"
        "<shape id=\"c\" shape=\"rect\" width=\"2\" height=\"2\"/>"
        "</sequence></composition></scene>", &scene)) return;
    const SrNode *a = sr_scene_find_node(&scene, "a"), *b = sr_scene_find_node(&scene, "b");
    const SrNode *c = sr_scene_find_node(&scene, "c");
    CHECK(t, a && b && c);
    if (a && b && c) {
        CHECK(t, a->end_time == b->start_time);
        CHECK(t, b->end_time == c->start_time);
        /* C(t) = (t - 0.3) * 3 + 0.4: item a covers C in [0.3, 1.0). */
        CHECK_NEAR(t, a->start_time, (0.3 - 0.4) / 3 + 0.3, 1e-12);
        CHECK_NEAR(t, a->end_time, (1.0 - 0.4) / 3 + 0.3, 1e-12);
    }
    sr_scene_free(&scene);
}

/* ---- markers on nodes and keys -------------------------------------------- */

static void node_markers(sr_test_ctx *t) {
    SrScene scene;
    /* A child authored to end with its group keeps the group's exact end. */
    if (expect_ok(t, HEAD("4") "<markers><marker id=\"q\" time=\"0.7\"/></markers>"
                  "<composition><group id=\"g\" timeScale=\"10\" timeOffset=\"0.3\">"
                  "<group id=\"h\" end=\"7.3\"><shape id=\"s\" shape=\"rect\" width=\"2\" "
                  "height=\"2\" end=\"7.3\"/></group></group></composition></scene>", &scene)) {
        const SrNode *h = sr_scene_find_node(&scene, "h");
        const SrNode *s = sr_scene_find_node(&scene, "s");
        CHECK(t, h && s && s->end_time == h->end_time);
        sr_scene_free(&scene);
    }
    /* The scale 10 clock does not round-trip 0.1 exactly: endpoints must
     * still be the marker instants. */
    if (expect_ok(t, HEAD("4") "<markers><marker id=\"p\" time=\"0.1\"/>"
                  "<marker id=\"q\" time=\"0.7\"/></markers><composition>"
                  "<group id=\"g\" timeScale=\"10\" timeOffset=\"0.3\">"
                  "<shape id=\"s\" shape=\"rect\" width=\"2\" height=\"2\" "
                  "startMarker=\"p\" endMarker=\"q\"/></group>"
                  "<group id=\"h\" startMarker=\"p\"/></composition></scene>", &scene)) {
        const SrNode *s = sr_scene_find_node(&scene, "s");
        const SrNode *h = sr_scene_find_node(&scene, "h");
        CHECK(t, s && s->start_time == 0.1 && s->end_time == 0.7);
        CHECK(t, h && h->start_time == 0.1 && isinf(h->end_time));
        sr_scene_free(&scene);
    }
    expect_error(t, HEAD("4") GRID "<composition><shape id=\"s\" shape=\"rect\" "
                 "width=\"2\" height=\"2\" start=\"1\" startMarker=\"m\"/></composition></scene>",
                 "start and startMarker are mutually exclusive");
    expect_error(t, HEAD("4") GRID "<composition><shape id=\"s\" shape=\"rect\" "
                 "width=\"2\" height=\"2\" end=\"1\" endMarker=\"m\"/></composition></scene>",
                 "end and endMarker are mutually exclusive");
    expect_error(t, HEAD("4") GRID "<composition><shape id=\"s\" shape=\"rect\" "
                 "width=\"2\" height=\"2\" startMarker=\"n\" endMarker=\"m\"/></composition></scene>",
                 "must satisfy 0 <= start < end");
    expect_error(t, HEAD("4") GRID "<composition><shape id=\"s\" shape=\"rect\" "
                 "width=\"2\" height=\"2\" startMarker=\"nope\"/></composition></scene>",
                 "unknown marker id 'nope'");
    expect_error(t, HEAD("4") GRID "<composition><shape id=\"s\" shape=\"rect\" "
                 "width=\"2\" height=\"2\" startMarker=\"s\"/></composition></scene>",
                 "'s' is not a marker");
    expect_error(t, HEAD("4") GRID "<composition><shape id=\"s\" shape=\"rect\" "
                 "width=\"2\" height=\"2\" startMarker=\"beat.99\"/></composition></scene>",
                 "beat.1 to beat.8 and bar.1 to bar.3");
}

static void key_snapping(sr_test_ctx *t) {
    SrScene scene;
    if (!expect_ok(t, HEAD("4") "<materials><material id=\"mat\" roughness=\"0.5\">"
        "<animate property=\"roughness\"><key marker=\"bar.2\" time=\"0\" value=\"0.2\"/>"
        "<key marker=\"m\" time=\"0\" value=\"0.8\"/></animate></material></materials>"
        GRID "<composition>"
        "<shape id=\"s\" shape=\"rect\" width=\"2\" height=\"2\">"
        "<animate property=\"position.x\">"
        "<key marker=\"n\" time=\"-0.25\" value=\"30\"/>"
        "<key marker=\"m\" time=\"0\" value=\"10\"/>"
        "<key time=\"0.1\" value=\"0\"/></animate>"
        "<animate property=\"fill\" defaultInterpolation=\"step\">"
        "<key marker=\"beat.1\" time=\"0\" value=\"#FF0000\"/>"
        "<key marker=\"beat.3\" time=\"0\" value=\"#0000FF\"/></animate></shape>"
        "<group id=\"g\" start=\"0.5\" timeScale=\"4\">"
        "<shape id=\"u\" shape=\"rect\" width=\"2\" height=\"2\">"
        "<animate property=\"position.y\" timeBase=\"local\">"
        "<key time=\"0\" value=\"0\"/><key marker=\"n\" time=\"0\" value=\"40\"/></animate>"
        "</shape></group></composition></scene>", &scene)) return;
    const SrNode *s = sr_scene_find_node(&scene, "s");
    const SrNode *u = sr_scene_find_node(&scene, "u");
    CHECK(t, s && u);
    if (s && u) {
        const SrTrack *x = &s->transform.x.track;
        CHECK_INT(t, x->count, 3);
        if (x->count == 3) {
            CHECK(t, x->keys[0].time == 0.1 && x->keys[1].time == 0.5 &&
                     x->keys[2].time == 1.25);
        }
        CHECK(t, sr_anim_eval(&s->transform.x, 0.5) == 10.0);
        CHECK(t, sr_anim_eval(&s->transform.x, 1.25) == 30.0);
        CHECK(t, s->fill.r.keys[1].time == 1.25 && s->fill.a.keys[1].time == 1.25);
        /* Under a scale 4 clock the key sits at composition 1.5 exactly. */
        CHECK(t, sr_anim_eval(&u->transform.y, 1.5) == 40.0);
        CHECK_NEAR(t, sr_anim_eval(&u->transform.y, 1.0), 40.0 * 2.5 / 4.5, 1e-12);
    }
    CHECK(t, scene.materials[0].roughness.track.keys[0].time == 0.5);
    CHECK(t, scene.materials[0].roughness.track.keys[1].time == 1.75);
    CHECK_INT(t, scene.timeline ? scene.timeline->pending_tracks : 1, 0);
    sr_scene_free(&scene);
    expect_error(t, HEAD("4") GRID "<composition><shape id=\"s\" shape=\"rect\" "
                 "width=\"2\" height=\"2\"><animate property=\"opacity\">"
                 "<key marker=\"m\" time=\"0\" value=\"0\"/>\n"
                 "<key marker=\"beat.2\" time=\"-0.25\" value=\"1\"/></animate>"
                 "</shape></composition></scene>", "keyframe times must be unique");
    expect_error(t, HEAD("4") GRID "<composition><shape id=\"s\" shape=\"rect\" "
                 "width=\"2\" height=\"2\"><animate property=\"opacity\">"
                 "<key marker=\"m\" time=\"-1\" value=\"0\"/></animate>"
                 "</shape></composition></scene>", "must be non-negative");
    expect_error(t, HEAD("4") GRID "<composition><shape id=\"s\" shape=\"rect\" "
                 "width=\"2\" height=\"2\"><animate property=\"opacity\">"
                 "<key marker=\"bar.9\" time=\"0\" value=\"0\"/></animate>"
                 "</shape></composition></scene>", "unknown marker id 'bar.9'");
    expect_error(t, HEAD("4") GRID "<composition><shape id=\"s\" shape=\"rect\" "
                 "width=\"2\" height=\"2\"><animate property=\"opacity\">"
                 "<key marker=\"m\" time=\"2e6\" value=\"0\"/></animate>"
                 "</shape></composition></scene>", "at most 1e6 seconds");
    expect_error(t, HEAD("4") "<composition><shape id=\"s\" shape=\"rect\" "
                 "width=\"2\" height=\"2\"><animate property=\"opacity\">"
                 "<key time=\"-1\" value=\"0\"/></animate>"
                 "</shape></composition></scene>", "non-negative time");
}

/* ---- names, tags and clock consumers -------------------------------------- */

static void names_and_tags(sr_test_ctx *t) {
    SrScene scene;
    if (expect_ok(t, HEAD("1") "<composition><group id=\"g\" name=\"Lower third &amp; logo\" "
                  "tags=\" brand\tcta:primary  x.y-z_1 \"/></composition></scene>", &scene)) {
        const SrNode *g = sr_scene_find_node(&scene, "g");
        CHECK(t, g && g->timeline);
        if (g && g->timeline) {
            CHECK_STR(t, g->timeline->name, "Lower third & logo");
            CHECK_INT(t, g->timeline->tag_count, 3);
            if (g->timeline->tag_count == 3) {
                CHECK_STR(t, g->timeline->tags[0], "brand");
                CHECK_STR(t, g->timeline->tags[1], "cta:primary");
                CHECK_STR(t, g->timeline->tags[2], "x.y-z_1");
            }
        }
        sr_scene_free(&scene);
    }
    expect_error(t, HEAD("1") "<composition><group id=\"g\" tags=\"a b a\"/></composition></scene>",
                 "duplicate tag");
    char xml[8192];
    size_t used = (size_t)snprintf(xml, sizeof(xml), HEAD("1")
                                   "<composition><group id=\"g\" tags=\"");
    for (int i = 0; i <= (int)SR_MAX_NODE_TAGS; ++i)
        used += (size_t)snprintf(xml + used, sizeof(xml) - used, "t%d ", i);
    snprintf(xml + used, sizeof(xml) - used, "\"/></composition></scene>");
    expect_error(t, xml, "at most 64 tags");
    char tag[SR_MAX_TAG_BYTES + 2];
    memset(tag, 'a', sizeof(tag) - 1);
    tag[sizeof(tag) - 1] = '\0';
    snprintf(xml, sizeof(xml), HEAD("1") "<composition><group id=\"g\" tags=\"%s\"/>"
             "</composition></scene>", tag);
    expect_error(t, xml, "exceeds 128 bytes");
    char name[SR_MAX_NODE_NAME + 2];
    memset(name, 'n', sizeof(name) - 1);
    name[sizeof(name) - 1] = '\0';
    char big[SR_MAX_NODE_NAME + 512];
    snprintf(big, sizeof(big), HEAD("1") "<composition><shape id=\"s\" shape=\"rect\" "
             "width=\"1\" height=\"1\" name=\"%s\"/></composition></scene>", name);
    expect_error(t, big, "name exceeds 1024 bytes");
}

static void media_and_physics_clocks(sr_test_ctx *t) {
    SrScene scene;
    if (expect_ok(t, HEAD("2") "<assets><image id=\"i\" src=\"../../tests/data-oom-tile.png\" "
                  "width=\"4\" height=\"4\"/></assets><composition><group id=\"g\" timeScale=\"2\">"
                  "<layer id=\"l\" asset=\"i\" speed=\"1.5\"/></group>"
                  "<layer id=\"plain\" asset=\"i\" speed=\"1.5\"/></composition></scene>", &scene)) {
        const SrNode *l = sr_scene_find_node(&scene, "l");
        const SrNode *plain = sr_scene_find_node(&scene, "plain");
        CHECK(t, l && l->speed == 3.0);
        CHECK(t, plain && plain->speed == 1.5);
        sr_scene_free(&scene);
    }
    expect_error(t, HEAD("2") "<composition><group id=\"g\" timeScale=\"2\">"
                 "<shape id=\"s\" shape=\"rect\" width=\"2\" height=\"2\">"
                 "<rigidBody type=\"dynamic\"/></shape></group></composition></scene>",
                 "unsupported in this build");
    /* Offsets alone keep physics on the project clock. */
    if (expect_ok(t, HEAD("2") "<composition><group id=\"g\" timeOffset=\"1\">"
                  "<shape id=\"s\" shape=\"rect\" width=\"2\" height=\"2\">"
                  "<rigidBody type=\"dynamic\"/></shape></group></composition></scene>", &scene))
        sr_scene_free(&scene);
}

static bool render_at(sr_test_ctx *t, const char *xml, double time, SrFrame *frame) {
    SrScene scene;
    if (!expect_ok(t, xml, &scene)) return false;
    const float clear[4] = {0, 0, 0, 1};
    bool ok = fx_render(t, &scene, time, clear, frame);
    sr_scene_free(&scene);
    return ok;
}

#define EMITTER(rate) "<particleEmitter id=\"p\" x=\"16\" y=\"8\" lifetime=\"0.75\" " \
    "speed=\"12\" spread=\"180\" size=\"1\" seed=\"5\" " rate ">" \
    "<animate property=\"size\" timeBase=\"local\"><key time=\"0\" value=\"1\"/><key time=\"1\" value=\"2\"/></animate>"

static void particles_follow_clock(sr_test_ctx *t) {
    /* timeScale 2 at t equals the same emitter at 2t outside the group:
     * every conversion is a multiplication or division by two. Local
     * tracks keep both scenes on the clocked (extended) rate path. */
    const char *keyed_rate = "rate=\"10\"><animate property=\"rate\" timeBase=\"local\"><key time=\"0\" value=\"4\"/>"
        "<key time=\"0.5\" value=\"40\"/></animate";
    char scaled[2048], plain[2048];
    snprintf(scaled, sizeof(scaled), HEAD("2") "<composition><group id=\"g\" timeScale=\"2\">"
             "<particleEmitter id=\"p\" x=\"16\" y=\"8\" lifetime=\"0.75\" speed=\"12\" "
             "spread=\"180\" size=\"1\" seed=\"5\" %s></particleEmitter></group>"
             "</composition></scene>", keyed_rate);
    snprintf(plain, sizeof(plain), HEAD("4") "<composition>"
             "<particleEmitter id=\"p\" x=\"16\" y=\"8\" lifetime=\"0.75\" speed=\"12\" "
             "spread=\"180\" size=\"1\" seed=\"5\" %s></particleEmitter>"
             "</composition></scene>", keyed_rate);
    const double times[] = {0.1, 0.3, 0.45, 0.8};
    for (size_t i = 0; i < sizeof(times) / sizeof(times[0]); ++i) {
        SrFrame a = {0}, b = {0};
        if (render_at(t, scaled, times[i], &a) && render_at(t, plain, times[i] * 2, &b))
            CHECK(t, st_frames_equal(&a, &b));
        sr_frame_free(&a);
        sr_frame_free(&b);
    }
    /* An offset of 0.25 with a dyadic rate: births and conversions are exact. */
    snprintf(scaled, sizeof(scaled), HEAD("2") "<composition><group id=\"g\" timeOffset=\"0.25\">"
             EMITTER("rate=\"8\"") "</particleEmitter></group></composition></scene>");
    snprintf(plain, sizeof(plain), HEAD("4") "<composition>"
             EMITTER("rate=\"8\"") "</particleEmitter></composition></scene>");
    for (size_t i = 0; i < 3; ++i) {
        double time = 0.125 + 0.25 * (double)i;
        SrFrame a = {0}, b = {0};
        if (render_at(t, scaled, time, &a) && render_at(t, plain, time + 0.25, &b))
            CHECK(t, st_frames_equal(&a, &b));
        sr_frame_free(&a);
        sr_frame_free(&b);
    }
}

static void every_construct_fixture(sr_test_ctx *t) {
    SrScene scene;
    FILE *sink;
    SrDiagnostics diag;
    st_diag(&diag, &sink);
    SrStatus status = sr_scene_load_xml(sr_test_data_path("tests/data-timeline.xml"),
                                       &scene, &diag);
    if (sink) fclose(sink);
    CHECK_INT(t, status, SR_OK);
    if (status != SR_OK) return;
    const SrNode *picture = sr_scene_find_node(&scene, "picture");
    const SrNode *sparks = sr_scene_find_node(&scene, "sparks");
    const SrNode *dot = sr_scene_find_node(&scene, "dot");
    CHECK(t, picture && sparks && dot);
    if (picture && sparks && dot) {
        /* C(t) = 1.5 t + 0.125; picture's slot starts at C = 0.75 and it
         * ends at the marker "part" (composition 1.0); sparks follows 0.25
         * after that end, at C = 1.875. */
        CHECK(t, picture->end_time == 1.0);
        CHECK_NEAR(t, picture->start_time, (0.75 - 0.125) / 1.5, 1e-12);
        CHECK_NEAR(t, sparks->start_time, (1.875 - 0.125) / 1.5, 1e-12);
        CHECK(t, sparks->clock_scale == 1.5);
        /* slow: C(t) = (t - 0.5) * 0.5 + 0.75, so dot's start 0 is t = -1. */
        CHECK(t, dot->start_time == -1.0 && dot->clock_scale == 0.5);
        CHECK(t, dot->clock_offset == 0.5);
    }
    CHECK_INT(t, scene.timeline->pending_tracks, 0);
    CHECK(t, scene.audio.tracks[0].volume.track.keys[1].time == 1.0);
    CHECK(t, scene.lights[0].intensity.track.keys[0].time == 1.25);
    CHECK(t, scene.effects[0].intensity.track.keys[0].time == 0.25);
    const float clear[4] = {0, 0, 0, 1};
    SrFrame frame = {0};
    CHECK(t, fx_render(t, &scene, 1.1, clear, &frame));
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

const sr_test_case sr_tests_markers[] = {
    {"beat_grid_closed_form", beat_grid_closed_form},
    {"generated_id_syntax", generated_id_syntax},
    {"generated_limit", generated_limit},
    {"marker_table", marker_table},
    {"marker_count_limit", marker_count_limit},
    {"nested_group_clocks", nested_group_clocks},
    {"default_groups_keep_absolute_children", default_groups_keep_absolute_children},
    {"sequence_placement", sequence_placement},
    {"timed_sequence_items_meet_exactly", timed_sequence_items_meet_exactly},
    {"node_markers", node_markers},
    {"key_snapping", key_snapping},
    {"names_and_tags", names_and_tags},
    {"media_and_physics_clocks", media_and_physics_clocks},
    {"particles_follow_clock", particles_follow_clock},
    {"every_construct_fixture", every_construct_fixture},
    {NULL, NULL},
};
