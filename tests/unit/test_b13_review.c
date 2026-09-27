/* SPDX-License-Identifier: Apache-2.0 */
/* Regression tests for the Codex review of the B1-3 completion branch
 * (docs/reviews/b1-3-mattes-masks.md, "Diff review"). One case per finding. */
#include "b13_fixture.h"
#include "fixture.h"
#include "compositor_resources_internal.h"
#include "scene_render/effects.h"

#define ARRAY_COUNT(a) (sizeof(a) / sizeof((a)[0]))

static SrStatus scoped_render(SrScene *scene, SrFrame *frame,
                              const SrCompositeLimits *limits,
                              SrCompositeResources *result) {
    float background[4];
    sr_color_to_blend(&scene->project, scene->project.background, background);
    sr_frame_clear(frame, background, 1);
    SrCompositor compositor;
    sr_compositor_init(&compositor, 1);
    SrCompositeScope scope;
    SrStatus status = sr_composite_scope_begin(&scope, &compositor, scene, frame, limits);
    if (status == SR_OK)
        status = sr_compositor_render_scene(&compositor, scene, 0, frame, NULL);
    /* After scope end every frame allocation is released: any remaining
     * balance is a leaked reservation (debug builds also assert on it). */
    status = sr_composite_scope_end(&scope, &compositor, status, NULL);
    *result = scope.resources;
    sr_compositor_free(&compositor);
    return status;
}

/* 1: a work rejection after the effect's memory reservation leaves no
 * outstanding reservation. */
static void work_rejection_releases_reservation(sr_test_ctx *t) {
    const char *xml = "<scene version=\"1.1\">\n"
        "<project width=\"256\" height=\"256\" fps=\"2\" duration=\"1\"/>\n"
        "<composition><shape id=\"r\" shape=\"rect\" width=\"128\" height=\"256\" "
        "fill=\"#FF0000\"/><adjustment id=\"a\" effects=\"b\"/></composition>"
        "<effects><effect id=\"b\" type=\"blur\" radius=\"8\"/></effects></scene>";
    SrScene scene;
    if (b13_load(t, xml, &scene, NULL) != SR_OK) { SR_FAIL(t, "load"); return; }
    SrFrame frame;
    CHECK_INT(t, sr_frame_init(&frame, 256, 256), SR_OK);
    SrCompositeResources full;
    CHECK_INT(t, scoped_render(&scene, &frame, NULL, &full), SR_OK);
    uint64_t effect = 256 * 256 * 128;
    bool owned = false;
    for (unsigned k = 1; k < 8; ++k) {
        SrCompositeLimits limits = {UINT64_MAX, UINT64_MAX, full.work - k * effect / 8};
        SrCompositeResources result;
        CHECK_INT(t, scoped_render(&scene, &frame, &limits, &result), SR_ERR_RENDER);
        CHECK_INT(t, result.bytes, 0);
        CHECK_INT(t, result.pixels, 0);
        owned |= result.failure_owner.attribute &&
                 !strcmp(result.failure_owner.attribute, "effects");
    }
    CHECK(t, owned);
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

/* 2: shallow path edges crossing the whole width are charged per column. */
static void path_edges_charge_columns(sr_test_ctx *t) {
    enum { EDGES = 400, WIDTH = 4096 };
    size_t size = (size_t)EDGES * 32 + 1024;
    char *path = malloc(size), *xml = malloc(size + 1024);
    CHECK(t, path && xml);
    if (!path || !xml) { free(path); free(xml); return; }
    size_t used = (size_t)snprintf(path, size, "M 0 0.1");
    for (unsigned i = 1; i < EDGES; ++i)
        used += (size_t)snprintf(path + used, size - used, " L %d %s",
                                 i % 2 ? WIDTH : 0, i % 2 ? "0.9" : "0.1");
    snprintf(path + used, size - used, " Z");
    snprintf(xml, size + 1024, "<scene version=\"1.1\">\n"
             "<project width=\"%d\" height=\"8\" fps=\"2\" duration=\"1\"/>\n"
             "<composition><shape id=\"s\" shape=\"rect\" width=\"%d\" height=\"8\" "
             "fill=\"#FFFFFF\"><mask type=\"path\" path=\"%s\"/></shape>"
             "</composition></scene>", WIDTH, WIDTH, path);
    SrScene scene;
    if (b13_load(t, xml, &scene, NULL) == SR_OK) {
        SrFrame frame;
        CHECK_INT(t, sr_frame_init(&frame, WIDTH, 8), SR_OK);
        SrCompositeResources result;
        CHECK_INT(t, scoped_render(&scene, &frame, NULL, &result), SR_OK);
        /* The scanline accumulation touches every crossed column. */
        CHECK(t, result.work >= (uint64_t)EDGES * WIDTH * 4);
        sr_frame_free(&frame);
        sr_scene_free(&scene);
    } else {
        SR_FAIL(t, "load");
    }
    free(path);
    free(xml);
}

/* 3: bounded effects use private scratch sized for their own rectangle,
 * never the calling thread's warmed legacy capacity, and leave that cache
 * in place; the legacy result is unchanged. */
static void effects_use_private_scratch(sr_test_ctx *t) {
    SrScene legacy;
    fx_scene(&legacy, 1024, 1024);
    SrEffect blur = {.type = SR_EFFECT_BLUR, .enabled = true, .referenced = true,
                     .intensity = {.base = 1}, .radius = {.base = 6}};
    SrEffect *refs[1] = {&blur};
    SrNode *group = fx_add(&legacy, NULL, SR_NODE_GROUP);
    CHECK(t, group && fx_rect(&legacy, group, 0, 0, 1024, 1024,
                              (SrColor){1, .5, 0, 1}, 1));
    if (!group) { sr_scene_free(&legacy); return; }
    group->effect_refs = refs;
    group->effect_ref_count = 1;
    SrFrame big, again;
    const float clear[4] = {0, 0, 0, 0};
    CHECK(t, fx_render(t, &legacy, 0, clear, &big));   /* warms TLS scratch */
    SrEffectsPrivate saved;
    sr_effects_private_begin(&saved);
    CHECK(t, saved.capacity[0] >= 1024u * 1024u * 4u);
    CHECK_INT(t, sr_effects_private_end(&saved), 0);
    /* A small bounded adjustment borrows none of it. */
    const char *xml = "<scene version=\"1.1\">\n"
        "<project width=\"32\" height=\"32\" fps=\"2\" duration=\"1\"/>\n"
        "<composition><shape id=\"r\" shape=\"rect\" width=\"16\" height=\"32\" "
        "fill=\"#FF0000\"/><adjustment id=\"a\" effects=\"b\"/></composition>"
        "<effects><effect id=\"b\" type=\"blur\" radius=\"3\"/></effects></scene>";
    SrScene scene;
    if (b13_load(t, xml, &scene, NULL) == SR_OK) {
        SrFrame cold, warm;
        SrCompositeResources first, second;
        CHECK_INT(t, sr_frame_init(&cold, 32, 32), SR_OK);
        CHECK_INT(t, sr_frame_init(&warm, 32, 32), SR_OK);
        CHECK_INT(t, scoped_render(&scene, &warm, NULL, &second), SR_OK);
        sr_effects_release();
        CHECK_INT(t, scoped_render(&scene, &cold, NULL, &first), SR_OK);
        CHECK(t, st_frames_equal(&cold, &warm));
        CHECK_INT(t, first.peak_bytes, second.peak_bytes);
        /* The retained bytes of a private scope fit the 32x32 reservation. */
        CHECK(t, first.peak_bytes < 4u * 1024u * 1024u);
        sr_frame_free(&cold);
        sr_frame_free(&warm);
        sr_scene_free(&scene);
    }
    /* Legacy output is identical after the private scopes. */
    CHECK(t, fx_render(t, &legacy, 0, clear, &again));
    CHECK(t, st_frames_equal(&big, &again));
    sr_frame_free(&big);
    sr_frame_free(&again);
    group->effect_refs = NULL;
    group->effect_ref_count = 0;
    sr_scene_free(&legacy);
}

/* 4: an absent adjustment matte source has empty coverage. */
static void absent_adjustment_source_is_empty(sr_test_ctx *t) {
    static const char *const states[] = {"opacity=\"0\"", "visible=\"false\"",
                                         "start=\"0.5\""};
    for (size_t i = 0; i < ARRAY_COUNT(states); ++i) {
        char xml[1024];
        snprintf(xml, sizeof(xml), "<scene version=\"1.1\">\n"
                 "<project width=\"16\" height=\"16\" fps=\"2\" duration=\"1\" "
                 "background=\"#FFFFFF\"/>\n<composition>"
                 "<adjustment id=\"a\" effects=\"g\" %s/>"
                 "<shape id=\"r\" shape=\"rect\" width=\"16\" height=\"16\" "
                 "fill=\"#FF0000\" matte=\"a\"/></composition>"
                 "<effects><effect id=\"g\" type=\"color-grade\" brightness=\"0.1\"/>"
                 "</effects></scene>", states[i]);
        SrFrame frame;
        CHECK_INT(t, b13_render_xml(t, xml, 0, 1, &frame, NULL), SR_OK);
        const float *px = st_px(&frame, 8, 8);
        CHECK_NEAR(t, px[1], 1, 1e-6);   /* background, not red */
        sr_frame_free(&frame);
    }
}

/* 5: every effect of a masked adjustment chain processes the input that
 * later effects read. */
static void masked_chain_uses_complete_inputs(sr_test_ctx *t) {
    const char *xml = "<scene version=\"1.1\">\n"
        "<project width=\"64\" height=\"64\" fps=\"2\" duration=\"1\" "
        "background=\"#C02020\"/>\n<composition>"
        "<adjustment id=\"a\" effects=\"grey blur\"><mask type=\"rect\" x=\"27\" "
        "y=\"27\" width=\"10\" height=\"10\"/></adjustment></composition>"
        "<effects><effect id=\"grey\" type=\"color-grade\" saturation=\"0\"/>"
        "<effect id=\"blur\" type=\"blur\" radius=\"12\"/></effects></scene>";
    SrFrame frame;
    CHECK_INT(t, b13_render_xml(t, xml, 0, 1, &frame, NULL), SR_OK);
    const float *px = st_px(&frame, 32, 32);
    CHECK_NEAR(t, px[0], px[1], 1e-4);
    CHECK_NEAR(t, px[1], px[2], 1e-4);
    sr_frame_free(&frame);
}

/* 6: ancestor opacity scales the finished capture once, after the source's
 * own (dissolve) rendering. */
static void ancestor_opacity_after_capture(sr_test_ctx *t) {
    const char *xml = "<scene version=\"1.1\">\n"
        "<project width=\"32\" height=\"32\" fps=\"2\" duration=\"1\" seed=\"4\" "
        "background=\"#00000000\"/>\n<composition>"
        "<group id=\"g\" opacity=\".5\"><shape id=\"d\" shape=\"rect\" width=\"32\" "
        "height=\"32\" fill=\"#FFFFFF\" blend=\"dissolve\"/></group>"
        "<shape id=\"c\" shape=\"rect\" width=\"32\" height=\"32\" fill=\"#00FF00\" "
        "matte=\"d\"/></composition></scene>";
    SrFrame frame;
    CHECK_INT(t, b13_render_xml(t, xml, 0, 1, &frame, NULL), SR_OK);
    for (uint32_t i = 0; i < 32; ++i)
        CHECK_NEAR(t, st_px(&frame, i, (i * 7) % 32)[3], .5, 1e-6);
    sr_frame_free(&frame);
}

static size_t differing(const SrFrame *a, const SrFrame *b) {
    size_t count = 0;
    for (size_t i = 0; i < (size_t)a->width * a->height; ++i)
        count += memcmp(a->px + i * 4, b->px + i * 4, 4 * sizeof(float)) != 0;
    return count;
}

/* 7: an opaque flattened (dissolve) card keeps the card depth test and
 * depth write of the equivalent normal card. */
static void flattened_card_writes_depth(sr_test_ctx *t) {
    static const char *const blends[] = {"normal", "dissolve"};
    SrFrame frames[2];
    for (size_t i = 0; i < 2; ++i) {
        char xml[2048];
        snprintf(xml, sizeof(xml), "<scene version=\"1.1\">\n"
                 "<project width=\"96\" height=\"64\" fps=\"2\" duration=\"1\"/>\n"
                 "<composition><camera id=\"cam\" z=\"-300\" zoom=\"300\" active=\"true\"/>"
                 "<shape id=\"a\" shape=\"rect\" x=\"48\" y=\"32\" width=\"60\" height=\"40\" "
                 "anchorX=\"30\" anchorY=\"20\" threeD=\"true\" rotationY=\"40\" "
                 "fill=\"#FF0000\" blend=\"%s\"/>"
                 "<shape id=\"b\" shape=\"rect\" x=\"48\" y=\"32\" width=\"60\" height=\"40\" "
                 "anchorX=\"30\" anchorY=\"20\" threeD=\"true\" rotationY=\"-40\" "
                 "fill=\"#0000FF\"/></composition></scene>", blends[i]);
        CHECK_INT(t, b13_render_xml(t, xml, 0, 1, &frames[i], NULL), SR_OK);
    }
    /* Away from antialiased edges (binarized by dissolve) the occlusion
     * matches: red in front on the left, blue on the right. */
    size_t checked = 0;
    for (uint32_t y = 22; y <= 42; y += 4) {
        for (uint32_t x = 24; x <= 72; x += 4) {
            if (x > 44 && x < 52) continue;  /* intersection line */
            const float *n = st_px(&frames[0], x, y), *d = st_px(&frames[1], x, y);
            CHECK(t, (n[0] > n[2]) == (d[0] > d[2]));
            ++checked;
        }
    }
    CHECK(t, checked > 50);
    CHECK(t, differing(&frames[0], &frames[1]) < 96 * 64 / 4);
    sr_frame_free(&frames[0]);
    sr_frame_free(&frames[1]);
}

/* 8: an adjustment source's replayed prefix keeps card-run depth order. */
static void adjustment_prefix_sorts_cards(sr_test_ctx *t) {
    const char *xml = "<scene version=\"1.1\">\n"
        "<project width=\"64\" height=\"64\" fps=\"2\" duration=\"1\" "
        "background=\"#000000\"/>\n<composition>"
        "<camera id=\"cam\" z=\"-300\" zoom=\"300\" active=\"true\"/>"
        "<shape id=\"near\" shape=\"rect\" width=\"64\" height=\"64\" threeD=\"true\" "
        "fill=\"#FFFFFF\"/>"
        "<shape id=\"far\" shape=\"rect\" width=\"64\" height=\"64\" threeD=\"true\" "
        "zDepth=\"200\" fill=\"#000000\"/>"
        "<adjustment id=\"a\" effects=\"id\"/>"
        "<shape id=\"c\" shape=\"rect\" width=\"64\" height=\"64\" fill=\"#00FF00\" "
        "matte=\"a\" matteMode=\"luma\"/></composition>"
        "<effects><effect id=\"id\" type=\"color-grade\"/></effects></scene>";
    SrFrame frame;
    CHECK_INT(t, b13_render_xml(t, xml, 0, 1, &frame, NULL), SR_OK);
    /* Normal drawing puts near white over far black: full luma. */
    CHECK_NEAR(t, st_px(&frame, 32, 32)[1], 1, 1e-4);
    CHECK_NEAR(t, st_px(&frame, 32, 32)[0], 0, 1e-4);
    sr_frame_free(&frame);
}

/* 9: a masked parent keeps the backdrop its adjustment child reads. */
static void parent_mask_keeps_adjustment_input(sr_test_ctx *t) {
    const char *xml = "<scene version=\"1.1\">\n"
        "<project width=\"48\" height=\"16\" fps=\"2\" duration=\"1\" "
        "background=\"#00000000\"/>\n<composition>"
        "<group id=\"g\"><mask type=\"rect\" x=\"16\" width=\"16\" height=\"16\"/>"
        "<shape id=\"w\" shape=\"rect\" x=\"8\" width=\"4\" height=\"16\" fill=\"#FFFFFF\"/>"
        "<adjustment id=\"a\" effects=\"b\"/></group></composition>"
        "<effects><effect id=\"b\" type=\"blur\" radius=\"12\"/></effects></scene>";
    SrFrame frame;
    CHECK_INT(t, b13_render_xml(t, xml, 0, 1, &frame, NULL), SR_OK);
    CHECK(t, st_px(&frame, 17, 8)[3] > .01f);
    CHECK_NEAR(t, st_px(&frame, 10, 8)[3], 0, 0);  /* outside the mask */
    sr_frame_free(&frame);
}

/* 10: an additive innerRadius track adds to half the static outer radius. */
static void additive_inner_radius_base(sr_test_ctx *t) {
    static const char *const masks[] = {
        "<mask type=\"star\" x=\"24\" y=\"24\" radius=\"20\">"
        "<animate property=\"innerRadius\" additive=\"true\"><key time=\"0\" value=\"2\"/>"
        "</animate></mask>",
        "<mask type=\"star\" x=\"24\" y=\"24\" radius=\"20\" innerRadius=\"12\"/>"};
    SrFrame frames[2];
    for (size_t i = 0; i < 2; ++i) {
        char xml[1024];
        snprintf(xml, sizeof(xml), "<scene version=\"1.1\">\n"
                 "<project width=\"48\" height=\"48\" fps=\"2\" duration=\"1\"/>\n"
                 "<composition><shape id=\"s\" shape=\"rect\" width=\"48\" height=\"48\" "
                 "fill=\"#FFFFFF\">%s</shape></composition></scene>", masks[i]);
        CHECK_INT(t, b13_render_xml(t, xml, 0, 1, &frames[i], NULL), SR_OK);
    }
    CHECK(t, st_frames_equal(&frames[0], &frames[1]));
    sr_frame_free(&frames[0]);
    sr_frame_free(&frames[1]);
}

/* B1-5 merge: sequences and re-timed groups are matte sources; the
 * source's own clock drives its captured image. */
static void sequence_and_clock_matte_sources(sr_test_ctx *t) {
    const char *xml = "<scene version=\"1.1\">\n"
        "<project width=\"32\" height=\"16\" fps=\"4\" duration=\"2\" "
        "background=\"#00000000\"/>\n<composition>"
        "<sequence id=\"seq\"><shape id=\"a\" shape=\"rect\" width=\"8\" height=\"16\" "
        "end=\"1\" fill=\"#FFFFFF\"/><shape id=\"b\" shape=\"rect\" x=\"8\" width=\"8\" "
        "height=\"16\" end=\"1\" fill=\"#FFFFFF\"/></sequence>"
        "<group id=\"fast\" timeOffset=\"0.5\" timeScale=\"2\">"
        "<shape id=\"m\" shape=\"rect\" x=\"16\" width=\"8\" height=\"16\" "
        "fill=\"#FFFFFF\"><animate property=\"position.x\"><key time=\"0\" value=\"16\"/>"
        "<key time=\"2.5\" value=\"24\"/></animate></shape></group>"
        "<shape id=\"c1\" shape=\"rect\" width=\"16\" height=\"16\" fill=\"#FF0000\" "
        "matte=\"seq\"/>"
        "<shape id=\"c2\" shape=\"rect\" x=\"16\" width=\"16\" height=\"16\" "
        "fill=\"#00FF00\" matte=\"fast\"/></composition></scene>";
    SrScene scene;
    char *message = NULL;
    SrStatus status = b13_load(t, xml, &scene, &message);
    CHECK_INT(t, status, SR_OK);
    if (status != SR_OK) { fprintf(stderr, "%s\n", message ? message : ""); free(message); return; }
    free(message);
    SrFrame early, late;
    CHECK_INT(t, b13_render(&scene, .5, 1, &early, NULL), SR_OK);
    CHECK_INT(t, b13_render(&scene, 1.5, 4, &late, NULL), SR_OK);
    /* Sequence item a plays first, item b in the second slot. */
    CHECK_NEAR(t, st_px(&early, 4, 8)[3], 1, 1e-6);
    CHECK_NEAR(t, st_px(&early, 12, 8)[3], 0, 0);
    CHECK_NEAR(t, st_px(&late, 4, 8)[3], 0, 0);
    CHECK_NEAR(t, st_px(&late, 12, 8)[3], 1, 1e-6);
    /* The re-timed source is at local time 0.5 + 2*1.5 = 3.5 (hold): x=24. */
    CHECK_NEAR(t, st_px(&late, 20, 8)[3], 0, 0);
    CHECK_NEAR(t, st_px(&late, 28, 8)[3], 1, 1e-6);
    sr_frame_free(&early);
    sr_frame_free(&late);
    sr_scene_free(&scene);
}

/* B1-5 merge: adjustments take name/tags/markers, and key markers resolve
 * on adjustment and advanced-mask tracks. */
static void adjustment_timeline_attributes(sr_test_ctx *t) {
    const char *xml = "<scene version=\"1.1\">\n"
        "<project width=\"16\" height=\"16\" fps=\"4\" duration=\"2\"/>\n"
        "<markers><marker id=\"go\" time=\"1\"/></markers><composition>"
        "<shape id=\"s\" shape=\"rect\" width=\"16\" height=\"16\" fill=\"#FF8000\"/>"
        "<adjustment id=\"a\" effects=\"g\" name=\"Grade\" tags=\"look\" "
        "startMarker=\"go\"><animate property=\"opacity\"><key time=\"0\" value=\"0\"/>"
        "<key time=\"0.9\" marker=\"go\" value=\"1\"/></animate>"
        "<mask type=\"rect\" width=\"16\" height=\"16\"><animate property=\"feather\">"
        "<key time=\"0\" value=\"0\"/><key time=\"0.9\" marker=\"go\" value=\"2\"/></animate></mask>"
        "</adjustment></composition>"
        "<effects><effect id=\"g\" type=\"color-grade\" saturation=\"0\"/></effects></scene>";
    SrScene scene;
    char *message = NULL;
    SrStatus status = b13_load(t, xml, &scene, &message);
    CHECK_INT(t, status, SR_OK);
    if (status != SR_OK) { fprintf(stderr, "%s\n", message ? message : ""); free(message); return; }
    free(message);
    SrNode *adjustment = sr_scene_find_node(&scene, "a");
    CHECK(t, adjustment && adjustment->timeline);
    CHECK_NEAR(t, adjustment->start_time, 1, 0);
    SrFrame before, after;
    CHECK_INT(t, b13_render(&scene, .5, 1, &before, NULL), SR_OK);
    CHECK_INT(t, b13_render(&scene, 1.5, 1, &after, NULL), SR_OK);
    const float *b = st_px(&before, 8, 8), *a = st_px(&after, 8, 8);
    CHECK(t, b[0] > b[2] + .5f);                 /* not active yet: orange */
    CHECK(t, a[0] - a[2] < .75f * (b[0] - b[2])); /* active: greyer */
    sr_frame_free(&before);
    sr_frame_free(&after);
    sr_scene_free(&scene);
}

/* Follow-up 9: a card group keeps its adjustment child's backdrop too. */
static void card_parent_mask_keeps_adjustment_input(sr_test_ctx *t) {
    const char *xml = "<scene version=\"1.1\">\n"
        "<project width=\"48\" height=\"16\" fps=\"2\" duration=\"1\" "
        "background=\"#00000000\"/>\n<composition>"
        "<camera id=\"cam\" z=\"-300\" zoom=\"300\" active=\"true\"/>"
        "<group id=\"g\" threeD=\"true\"><mask type=\"rect\" x=\"16\" width=\"16\" "
        "height=\"16\"/><shape id=\"w\" shape=\"rect\" x=\"8\" width=\"4\" height=\"16\" "
        "fill=\"#FFFFFF\"/><adjustment id=\"a\" effects=\"b\"/></group></composition>"
        "<effects><effect id=\"b\" type=\"blur\" radius=\"12\"/></effects></scene>";
    SrFrame frame;
    CHECK_INT(t, b13_render_xml(t, xml, 0, 1, &frame, NULL), SR_OK);
    CHECK(t, st_px(&frame, 17, 8)[3] > .01f);
    CHECK_NEAR(t, st_px(&frame, 10, 8)[3], 0, 1e-6);
    sr_frame_free(&frame);
}

static SrStatus adjustment_render(sr_test_ctx *t, uint32_t size, SrScene *scene) {
    char xml[1024];
    snprintf(xml, sizeof(xml), "<scene version=\"1.1\">\n"
             "<project width=\"%u\" height=\"%u\" fps=\"2\" duration=\"1\"/>\n"
             "<composition><shape id=\"r\" shape=\"rect\" width=\"8\" height=\"%u\" "
             "fill=\"#FF0000\"/><adjustment id=\"a\" effects=\"b\"/></composition>"
             "<effects><effect id=\"b\" type=\"blur\" radius=\"3\"/></effects></scene>",
             size, size, size);
    return b13_load(t, xml, scene, NULL);
}

static SrStatus legacy_blur(sr_test_ctx *t, uint32_t size) {
    char xml[1024];
    snprintf(xml, sizeof(xml), "<scene version=\"1.0\">\n"
             "<project width=\"%u\" height=\"%u\" fps=\"2\" duration=\"1\"/>\n"
             "<composition><group id=\"g\" effects=\"b\"><shape id=\"r\" shape=\"rect\" "
             "width=\"%u\" height=\"%u\" fill=\"#FF0000\"/></group></composition>"
             "<effects><effect id=\"b\" type=\"blur\" radius=\"3\"/></effects></scene>",
             size, size, size, size);
    SrFrame frame;
    SrStatus status = b13_render_xml(t, xml, 0, 1, &frame, NULL);
    if (status == SR_OK) sr_frame_free(&frame);
    return status;
}

/* Follow-up 3: a real bounded effect call allocates its own scratch (even
 * with a large warmed cache on the thread), leaves a small warmed cache
 * exactly in place when it needs more, and nests. */
static void bounded_effects_allocate_privately(sr_test_ctx *t) {
    sr_effects_release();
    CHECK_INT(t, legacy_blur(t, 512), SR_OK);        /* warm: large cache */
    SrScene scene;
    if (adjustment_render(t, 32, &scene) != SR_OK) { SR_FAIL(t, "load"); return; }
    unsigned long calls = 0;
    size_t largest = 0;
    SrFrame frame;
    sr_test_aligned_stats(true, NULL, NULL);
    CHECK_INT(t, b13_render(&scene, 0, 1, &frame, NULL), SR_OK);
    sr_test_aligned_stats(true, &calls, &largest);
    CHECK(t, calls > 0);                 /* private scratch, not the cache */
    CHECK(t, largest < 64u * 1024u);     /* sized for the 32x32 call */
    sr_frame_free(&frame);
    sr_scene_free(&scene);
    /* A small warmed cache survives a larger bounded call unchanged. */
    sr_effects_release();
    CHECK_INT(t, legacy_blur(t, 16), SR_OK);
    SrEffectsPrivate before, after;
    sr_effects_private_begin(&before);
    sr_effects_private_end(&before);
    if (adjustment_render(t, 256, &scene) == SR_OK) {
        CHECK_INT(t, b13_render(&scene, 0, 1, &frame, NULL), SR_OK);
        sr_frame_free(&frame);
        sr_scene_free(&scene);
    }
    sr_effects_private_begin(&after);
    CHECK(t, after.data[0] == before.data[0]);
    CHECK_INT(t, after.capacity[0], before.capacity[0]);
    CHECK(t, before.capacity[0] > 0);
    /* Nesting: an inner scope starts empty and restores the outer one. */
    SrEffectsPrivate inner;
    SrFrame small = {0};
    CHECK_INT(t, sr_frame_init(&small, 32, 32), SR_OK);
    SrEffect blur = {.type = SR_EFFECT_BLUR, .enabled = true,
                     .intensity = {.base = 1}, .radius = {.base = 3}};
    SrEffect *refs[1] = {&blur};
    SrEffectRect rect = {0, 0, 32, 32};
    SrScene empty;
    fx_scene(&empty, 32, 32);
    CHECK_INT(t, sr_effects_apply_group(&empty, refs, 1, 0, &small,
                                        sr_mat_identity(), &rect, 1), SR_OK);
    sr_effects_private_begin(&inner);
    CHECK(t, inner.capacity[0] > 0);     /* the outer scope's allocation */
    rect = (SrEffectRect){0, 0, 32, 32};
    CHECK_INT(t, sr_effects_apply_group(&empty, refs, 1, 0, &small,
                                        sr_mat_identity(), &rect, 1), SR_OK);
    CHECK(t, sr_effects_private_end(&inner) > 0);
    CHECK(t, sr_effects_private_end(&after) > 0);
    SrEffectsPrivate check;
    sr_effects_private_begin(&check);
    CHECK(t, check.data[0] == before.data[0]);
    sr_effects_private_end(&check);
    sr_frame_free(&small);
    sr_scene_free(&empty);
    sr_effects_release();
}

/* Follow-up A: the adjustment effect list is bounded by a named limit. */
static void adjustment_effect_reference_limit(sr_test_ctx *t) {
    size_t size = 257 * 3 + 2048;
    char *xml = malloc(size);
    CHECK(t, xml != NULL);
    if (!xml) return;
    for (unsigned count = 256; count <= 257; ++count) {
        size_t used = (size_t)snprintf(xml, size, "<scene version=\"1.1\">\n"
            "<project width=\"8\" height=\"8\" fps=\"2\" duration=\"1\"/>\n"
            "<composition><adjustment id=\"a\" effects=\"");
        for (unsigned i = 0; i < count; ++i)
            used += (size_t)snprintf(xml + used, size - used, "%se", i ? " " : "");
        snprintf(xml + used, size - used, "\"/></composition><effects><effect id=\"e\" "
                 "type=\"blur\" radius=\"1\" enabled=\"false\"/></effects></scene>");
        SrScene scene;
        char *message = NULL;
        SrStatus status = b13_load(t, xml, &scene, &message);
        if (count == 256) {
            CHECK_INT(t, status, SR_OK);
            SrFrame frame;
            if (status == SR_OK) {
                CHECK_INT(t, b13_render(&scene, 0, 1, &frame, NULL), SR_OK);
                sr_frame_free(&frame);
            }
        } else {
            CHECK(t, status != SR_OK);
            CHECK_CONTAINS(t, message, "effect list limit is 256");
        }
        if (status == SR_OK) sr_scene_free(&scene);
        free(message);
    }
    free(xml);
}

#define STENCIL_CARD "<scene version=\"1.1\">\n" \
    "<project width=\"32\" height=\"32\" fps=\"2\" duration=\"1\" " \
    "background=\"#FF8000\"/>\n<composition>" \
    "<camera id=\"cam\" z=\"-300\" zoom=\"300\" active=\"true\"/>" \
    "<shape id=\"src\" shape=\"rect\" width=\"8\" height=\"8\" visible=\"false\" " \
    "fill=\"#FFFFFF\"/><shape id=\"s\" shape=\"rect\" x=\"8\" y=\"8\" width=\"16\" " \
    "height=\"16\" threeD=\"true\" fill=\"#FFFFFF\" blend=\"stencil-alpha\" " \
    "matte=\"src\"/></composition></scene>"

/* Follow-up B: an active stencil card with an empty matte still clears. */
static void empty_matte_stencil_card_clears(sr_test_ctx *t) {
    SrFrame frame;
    CHECK_INT(t, b13_render_xml(t, STENCIL_CARD, 0, 1, &frame, NULL), SR_OK);
    CHECK_NEAR(t, st_px(&frame, 16, 16)[3], 0, 0);
    CHECK_NEAR(t, st_px(&frame, 2, 2)[3], 0, 0);
    sr_frame_free(&frame);
}

/* Follow-up C: a fully visible card operator copies its result exactly,
 * even over an HDR backdrop. */
static void full_coverage_card_operator_is_exact(sr_test_ctx *t) {
    const char *xml = "<scene version=\"1.1\">\n"
        "<project width=\"32\" height=\"32\" fps=\"2\" duration=\"1\" seed=\"1\"/>\n"
        "<composition><camera id=\"cam\" z=\"-300\" zoom=\"300\" active=\"true\"/>"
        "<shape id=\"d\" shape=\"rect\" width=\"32\" height=\"32\" threeD=\"true\" "
        "fill=\"#FFFFFF\" blend=\"dissolve\"/></composition></scene>";
    SrScene scene;
    if (b13_load(t, xml, &scene, NULL) != SR_OK) { SR_FAIL(t, "load"); return; }
    SrFrame frame = {0};
    CHECK_INT(t, sr_frame_init(&frame, 32, 32), SR_OK);
    const float hdr[4] = {1e8f, 1e8f, 1e8f, 1};
    sr_frame_clear(&frame, hdr, 1);
    SrCompositor compositor;
    sr_compositor_init(&compositor, 1);
    CHECK_INT(t, sr_compositor_render_scene(&compositor, &scene, 0, &frame, NULL), SR_OK);
    sr_compositor_free(&compositor);
    const float white[4] = {1, 1, 1, 1};
    CHECK(t, !memcmp(st_px(&frame, 16, 16), white, sizeof(white)));
    sr_frame_free(&frame);
    sr_scene_free(&scene);
}

const sr_test_case sr_tests_b13_review[] = {
    {"card_parent_mask_keeps_adjustment_input", card_parent_mask_keeps_adjustment_input},
    {"bounded_effects_allocate_privately", bounded_effects_allocate_privately},
    {"adjustment_effect_reference_limit", adjustment_effect_reference_limit},
    {"empty_matte_stencil_card_clears", empty_matte_stencil_card_clears},
    {"full_coverage_card_operator_is_exact", full_coverage_card_operator_is_exact},
    {"sequence_and_clock_matte_sources", sequence_and_clock_matte_sources},
    {"adjustment_timeline_attributes", adjustment_timeline_attributes},
    {"work_rejection_releases_reservation", work_rejection_releases_reservation},
    {"path_edges_charge_columns", path_edges_charge_columns},
    {"effects_use_private_scratch", effects_use_private_scratch},
    {"absent_adjustment_source_is_empty", absent_adjustment_source_is_empty},
    {"masked_chain_uses_complete_inputs", masked_chain_uses_complete_inputs},
    {"ancestor_opacity_after_capture", ancestor_opacity_after_capture},
    {"flattened_card_writes_depth", flattened_card_writes_depth},
    {"adjustment_prefix_sorts_cards", adjustment_prefix_sorts_cards},
    {"parent_mask_keeps_adjustment_input", parent_mask_keeps_adjustment_input},
    {"additive_inner_radius_base", additive_inner_radius_base},
    {NULL, NULL}
};
