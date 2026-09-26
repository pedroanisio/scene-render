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

const sr_test_case sr_tests_b13_review[] = {
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
