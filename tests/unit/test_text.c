/* SPDX-License-Identifier: Apache-2.0 */
#include "fixture.h"

#include "scene_render/assets.h"
#include "scene_render/text.h"

#include <hb.h>
#include <stdlib.h>

#define INTER "assets/third-party/inter/Inter-Regular.ttf"

static const char *const paragraph =
    "The quick brown fox jumps over the lazy dog while seven wizards quietly "
    "judge the boxing match; meanwhile a well-known jukebox plays jazz for "
    "sphinxes of black quartz who judge vows in the evening light.";

static SrTextStyle style_for(double size, uint32_t width, uint32_t height)
{
    return (SrTextStyle){.size = size, .line_height = 1.2, .width = width,
                         .height = height};
}

static SrFont *open_inter(sr_test_ctx *t)
{
    SrFont *font = NULL;
    char err[256] = "";
    SrStatus status = sr_font_open(sr_test_data_path(INTER), 0, &font, err, sizeof(err));
    if (status != SR_OK) SR_FAIL(t, "cannot open Inter: %s", err);
    return font;
}

/* Opens a Fontconfig family covering every code point, or NULL (with a
 * notice) when none is installed. */
static SrFont *open_family_covering(const char *family, const uint32_t *cps,
                                    size_t count)
{
    char *path = NULL;
    int index = 0;
    char err[256] = "";
    SrFont *font = NULL;
    if (sr_font_match_family(family, &path, &index, err, sizeof(err)) == SR_OK)
        sr_font_open(path, index, &font, err, sizeof(err));
    free(path);
    for (size_t i = 0; font && i < count; ++i)
        if (!sr_font_has_codepoint(font, cps[i])) {
            sr_font_close(font);
            font = NULL;
        }
    if (!font)
        fprintf(stderr, "  NOTICE: no '%s' font covering the test script; "
                        "skipping\n", family);
    return font;
}

static bool layout_ok(sr_test_ctx *t, SrFont *font, const char *text,
                      const SrTextStyle *style, SrTextLayout *layout)
{
    char err[256] = "";
    SrStatus status = sr_text_layout(font, text, style, layout, err, sizeof(err));
    if (status != SR_OK) SR_FAIL(t, "layout of \"%s\" failed: %s", text, err);
    return status == SR_OK;
}

static double line_width(sr_test_ctx *t, SrFont *font, const char *text,
                         SrTextStyle style)
{
    SrTextLayout layout;
    double width = -1.0;
    if (layout_ok(t, font, text, &style, &layout)) {
        CHECK_INT(t, layout.line_count, 1);
        if (layout.line_count) width = layout.lines[0].width;
    }
    sr_text_layout_free(&layout);
    return width;
}

/* x of the first glyph whose cluster starts at `byte`. */
static double cluster_x(const SrTextLayout *layout, size_t byte)
{
    for (size_t i = 0; i < layout->glyph_count; ++i)
        if (layout->glyphs[i].cluster == byte) return layout->glyphs[i].x;
    return NAN;
}

static void test_kerning(sr_test_ctx *t)
{
    SrFont *font = open_inter(t);
    if (!font) return;
    SrTextStyle style = style_for(100, 2000, 200);
    double pair = line_width(t, font, "AV", style);
    double a = line_width(t, font, "A", style);
    double v = line_width(t, font, "V", style);
    CHECK(t, pair > 0.0 && pair < a + v - 1.0);
    sr_font_close(font);
}

/* Glyph count HarfBuzz itself produces for `text` with default features. */
static unsigned hb_glyph_count(const char *path, int index, const char *text)
{
    hb_blob_t *blob = hb_blob_create_from_file(path);
    hb_face_t *face = hb_face_create(blob, (unsigned)index);
    hb_font_t *font = hb_font_create(face);
    hb_buffer_t *buffer = hb_buffer_create();
    hb_buffer_add_utf8(buffer, text, -1, 0, -1);
    hb_buffer_guess_segment_properties(buffer);
    hb_shape(font, buffer, NULL, 0);
    unsigned count = hb_buffer_get_length(buffer);
    hb_buffer_destroy(buffer);
    hb_font_destroy(font);
    hb_face_destroy(face);
    hb_blob_destroy(blob);
    return count;
}

static void test_ligature(sr_test_ctx *t)
{
    /* Inter 4 has no ffi ligature; fall back to DejaVu Sans, which does. */
    char *path = sr_strdup(sr_test_data_path(INTER));
    int index = 0;
    if (path && hb_glyph_count(path, 0, "ffi") >= 3) {
        free(path);
        path = NULL;
        char err[256] = "";
        if (sr_font_match_family("DejaVu Sans", &path, &index, err, sizeof(err)) != SR_OK ||
            hb_glyph_count(path, index, "ffi") >= 3) {
            fprintf(stderr, "  NOTICE: no font with an ffi ligature; skipping\n");
            free(path);
            return;
        }
    }
    SrFont *font = NULL;
    char err[256] = "";
    CHECK(t, path && sr_font_open(path, index, &font, err, sizeof(err)) == SR_OK);
    SrTextStyle style = style_for(40, 800, 100);
    SrTextLayout layout;
    if (font && layout_ok(t, font, "ffi", &style, &layout)) {
        CHECK(t, layout.glyph_count < 3);
        CHECK_INT(t, layout.lines[0].cluster_count, layout.glyph_count);
        sr_text_layout_free(&layout);
    }
    sr_font_close(font);
    free(path);
}

static void test_rtl_start_is_right(sr_test_ctx *t)
{
    static const uint32_t hebrew[] = {0x05E9, 0x05DC, 0x05D5, 0x05DD};
    static const uint32_t arabic[] = {0x0645, 0x0631, 0x062D, 0x0628, 0x0627};
    const struct { const uint32_t *cps; size_t count; const char *text; } cases[] = {
        {hebrew, 4, "שלום עולם"},
        {arabic, 5, "مرحبا بالعالم"},
    };
    for (size_t c = 0; c < 2; ++c) {
        SrFont *font = open_family_covering("DejaVu Sans", cases[c].cps, cases[c].count);
        if (!font) continue;
        SrTextStyle style = style_for(32, 500, 60);
        SrTextLayout layout;
        if (layout_ok(t, font, cases[c].text, &style, &layout)) {
            CHECK_INT(t, layout.line_count, 1);
            const SrTextLine *line = &layout.lines[0];
            CHECK(t, line->rtl);
            CHECK_NEAR(t, line->x + line->width, 500.0, 1e-6);
            CHECK(t, line->x > 100.0);
            /* Logical order runs right to left. */
            CHECK(t, cluster_x(&layout, 0) > cluster_x(&layout, line->byte_end - 2));
            sr_text_layout_free(&layout);
        }
        /* An explicit direction overrides the first strong character. */
        style.direction = SR_TEXT_DIR_LTR;
        if (layout_ok(t, font, cases[c].text, &style, &layout)) {
            CHECK(t, !layout.lines[0].rtl);
            CHECK_NEAR(t, layout.lines[0].x, 0.0, 1e-9);
            sr_text_layout_free(&layout);
        }
        if (c == 1) {
            /* Arabic is shaped: joining forms differ from the isolated ones. */
            style.direction = SR_TEXT_DIR_AUTO;
            SrTextLayout joined, isolated;
            if (layout_ok(t, font, "مرحبا", &style, &joined) &&
                layout_ok(t, font, "م ر ح ب ا", &style, &isolated)) {
                CHECK(t, joined.glyph_count > 0);
                CHECK(t, joined.glyphs[joined.glyph_count - 1].glyph !=
                         isolated.glyphs[isolated.glyph_count - 1].glyph);
            }
            sr_text_layout_free(&joined);
            sr_text_layout_free(&isolated);
        }
        sr_font_close(font);
    }
}

static void test_bidi_mixed_visual_order(sr_test_ctx *t)
{
    static const uint32_t hebrew[] = {0x05D0, 0x05D1, 0x05D2};
    SrFont *font = open_family_covering("DejaVu Sans", hebrew, 3);
    if (!font) return;
    /* "abc אבג 123": a b c at bytes 0-2, alef/bet/gimel at 4/6/8, digits at
     * 11-13. In an LTR paragraph the digits join the RTL run (UAX #9 W7/N1,
     * L2): visual order is "abc 123 גבא". */
    SrTextStyle style = style_for(30, 800, 60);
    SrTextLayout layout;
    if (layout_ok(t, font, "abc אבג 123", &style, &layout)) {
        CHECK(t, !layout.lines[0].rtl);
        const size_t order[] = {0, 1, 2, 11, 12, 13, 8, 6, 4};
        for (size_t i = 0; i + 1 < sizeof(order) / sizeof(order[0]); ++i) {
            double a = cluster_x(&layout, order[i]), b = cluster_x(&layout, order[i + 1]);
            if (!(a < b))
                SR_FAIL(t, "cluster %zu (x=%g) is not left of cluster %zu (x=%g)",
                        order[i], a, order[i + 1], b);
        }
        sr_text_layout_free(&layout);
    }
    sr_font_close(font);
}

static void test_wrap_and_justify(sr_test_ctx *t)
{
    SrFont *font = open_inter(t);
    if (!font) return;
    SrTextStyle style = style_for(20, 300, 400);
    SrTextLayout layout;
    if (layout_ok(t, font, paragraph, &style, &layout)) {
        CHECK(t, layout.line_count >= 3);
        CHECK(t, !layout.overflow);
        for (size_t i = 0; i < layout.line_count; ++i) {
            const SrTextLine *line = &layout.lines[i];
            CHECK(t, line->width <= 300.0 + 1e-6);
            CHECK(t, line->ink_left >= -1.0 && line->ink_right <= 301.0);
            if (i) CHECK_NEAR(t, line->baseline - layout.lines[i - 1].baseline, 24.0, 1.0);
        }
        sr_text_layout_free(&layout);
    }
    style.align = SR_TEXT_ALIGN_JUSTIFY;
    if (layout_ok(t, font, paragraph, &style, &layout)) {
        for (size_t i = 0; i < layout.line_count; ++i) {
            const SrTextLine *line = &layout.lines[i];
            const SrTextGlyph *first = &layout.glyphs[line->first_glyph];
            const SrTextGlyph *last = &layout.glyphs[line->first_glyph + line->glyph_count - 1];
            double right = last->x + last->advance;
            CHECK_NEAR(t, first->x, 0.0, 1e-9);
            if (i + 1 < layout.line_count) CHECK_NEAR(t, right, 300.0, 1.0);
            else CHECK(t, right < 299.0);
        }
        sr_text_layout_free(&layout);
    }
    /* A word wider than the box breaks between clusters. */
    style = style_for(40, 120, 400);
    if (layout_ok(t, font, "Supercalifragilistic", &style, &layout)) {
        CHECK(t, layout.line_count >= 2);
        for (size_t i = 0; i < layout.line_count; ++i)
            CHECK(t, layout.lines[i].width <= 120.0 + 1e-6);
        sr_text_layout_free(&layout);
    }
    /* Explicit newlines start paragraphs; a hyphen is a break opportunity. */
    style = style_for(20, 1000, 400);
    if (layout_ok(t, font, "one\ntwo\n\nfour", &style, &layout)) {
        CHECK_INT(t, layout.line_count, 4);
        sr_text_layout_free(&layout);
    }
    double left = line_width(t, font, "well-", style_for(20, 1000, 40));
    double right = line_width(t, font, "known", style_for(20, 1000, 40));
    style = style_for(20, (uint32_t)ceil(fmax(left, right)) + 2, 100);
    CHECK(t, style.width < line_width(t, font, "well-known", style_for(20, 1000, 40)));
    if (layout_ok(t, font, "well-known", &style, &layout)) {
        CHECK_INT(t, layout.line_count, 2);
        if (layout.line_count == 2) CHECK_INT(t, layout.lines[1].byte_start, 5);
        sr_text_layout_free(&layout);
    }
    sr_font_close(font);
}

static void test_letter_spacing(sr_test_ctx *t)
{
    SrFont *font = open_inter(t);
    if (!font) return;
    SrTextStyle style = style_for(30, 1000, 60);
    SrTextLayout layout;
    double plain = line_width(t, font, "Hello world", style);
    style.letter_spacing = 3.0;
    if (layout_ok(t, font, "Hello world", &style, &layout)) {
        const SrTextLine *line = &layout.lines[0];
        CHECK_INT(t, line->cluster_count, 11);
        CHECK_NEAR(t, line->width - plain, (line->cluster_count - 1) * 3.0, 1.0);
        const SrTextGlyph *last = &layout.glyphs[line->glyph_count - 1];
        CHECK_NEAR(t, last->x + last->advance - 3.0, line->width, 1e-9);
        sr_text_layout_free(&layout);
    }
    sr_font_close(font);
}

static void test_alignment_and_overflow(sr_test_ctx *t)
{
    SrFont *font = open_inter(t);
    if (!font) return;
    SrTextStyle style = style_for(20, 400, 100);
    double width = line_width(t, font, "Centered", style);
    SrTextLayout layout;
    style.align = SR_TEXT_ALIGN_CENTER;
    style.valign = SR_TEXT_VALIGN_MIDDLE;
    if (layout_ok(t, font, "Centered", &style, &layout)) {
        CHECK_NEAR(t, layout.lines[0].x, (400.0 - width) / 2.0, 1e-9);
        CHECK_NEAR(t, layout.block_height, 24.0, 1e-9);
        CHECK(t, layout.lines[0].baseline > 45.0 && layout.lines[0].baseline < 62.0);
        sr_text_layout_free(&layout);
    }
    style.align = SR_TEXT_ALIGN_END;
    style.height = 30;
    if (layout_ok(t, font, "one\ntwo", &style, &layout)) {
        CHECK_NEAR(t, layout.lines[0].x + layout.lines[0].width, 400.0, 1e-9);
        CHECK(t, layout.overflow);
        sr_text_layout_free(&layout);
    }
    sr_font_close(font);
}

static void test_rasterize_deterministic(sr_test_ctx *t)
{
    SrFont *font = open_inter(t);
    if (!font) return;
    SrTextStyle style = style_for(23.5, 320, 120);
    style.letter_spacing = 0.3;
    SrTextLayout layout;
    float *a = calloc(320 * 120, sizeof(float));
    float *b = calloc(320 * 120, sizeof(float));
    if (a && b && layout_ok(t, font, paragraph, &style, &layout)) {
        CHECK(t, sr_text_rasterize(font, &layout, style.size, 320, 120, a) == SR_OK);
        CHECK(t, sr_text_rasterize(font, &layout, style.size, 320, 120, b) == SR_OK);
        CHECK(t, memcmp(a, b, 320 * 120 * sizeof(float)) == 0);
        double ink = 0.0;
        float peak = 0.0f;
        for (size_t i = 0; i < 320 * 120; ++i) {
            ink += a[i];
            if (a[i] > peak) peak = a[i];
        }
        CHECK(t, ink > 500.0);
        CHECK(t, peak <= 1.0f && peak > 0.99f);
        sr_text_layout_free(&layout);
    }
    free(a);
    free(b);
    sr_font_close(font);
}

/* Builds a scene with one text layer; fontFile is absolute. */
static SrAsset *text_scene(SrScene *scene, const char *font_file, const char *family)
{
    fx_scene(scene, 200, 80);
    SrAsset *asset = sr_scene_add_asset(scene);
    SrNode *node = fx_add(scene, NULL, SR_NODE_MEDIA);
    if (!asset || !node) return NULL;
    asset->type = SR_ASSET_TEXT;
    asset->id = sr_strdup("label");
    asset->text = sr_strdup("Thread-invariant text, wrapped twice over");
    asset->width = 190;
    asset->height = 70;
    asset->text_size = 17.25;
    asset->text_line_height = 1.2;
    asset->text_align = SR_TEXT_ALIGN_JUSTIFY;
    asset->color = (SrColor){0.9, 0.6, 0.2, 0.8};
    asset->font_family = family ? sr_strdup(family) : NULL;
    asset->font_file = font_file ? sr_strdup(font_file) : NULL;
    node->asset = asset;
    node->transform.x.base = 3.25;
    node->transform.y.base = 4.5;
    return asset;
}

static void test_render_thread_invariant(sr_test_ctx *t)
{
    SrScene scene;
    SrAsset *asset = text_scene(&scene, sr_test_data_path(INTER), NULL);
    FILE *sink = tmpfile();
    SrDiagnostics diag;
    sr_diag_init(&diag, "text", sink);
    CHECK(t, asset && sr_assets_load(&scene, &diag) == SR_OK);
    CHECK(t, asset && asset->decoded);
    if (asset && asset->decoded) {
        /* Premultiplied: color * coverage, alpha at most 0.8. */
        float alpha = 0.0f;
        for (size_t i = 0; i < (size_t)190 * 70; ++i)
            if (asset->decoded->px[i * 4 + 3] > alpha) alpha = asset->decoded->px[i * 4 + 3];
        CHECK_NEAR(t, alpha, 0.8, 1e-6);
        const float background[4] = {0.1f, 0.1f, 0.1f, 1.0f};
        SrFrame frames[2] = {{0}, {0}};
        const unsigned threads[2] = {1, 7};
        for (int i = 0; i < 2; ++i) {
            CHECK(t, sr_frame_init(&frames[i], 200, 80) == SR_OK);
            if (!frames[i].px) continue;
            sr_frame_clear(&frames[i], background, threads[i]);
            SrCompositor compositor;
            sr_compositor_init(&compositor, threads[i]);
            CHECK(t, sr_compositor_render(&compositor, &scene, 0.0, &frames[i], NULL) == SR_OK);
            sr_compositor_free(&compositor);
        }
        if (frames[0].px && frames[1].px)
            CHECK(t, memcmp(frames[0].px, frames[1].px, (size_t)200 * 80 * 4 * sizeof(float)) == 0);
        sr_frame_free(&frames[0]);
        sr_frame_free(&frames[1]);
    }
    sr_assets_unload(&scene);
    CHECK(t, scene.font_cache == NULL);
    sr_scene_free(&scene);
    if (sink) fclose(sink);
}

static void expect_font_error(sr_test_ctx *t, const char *font_file,
                              const char *family, const char *needle)
{
    SrScene scene;
    SrAsset *asset = text_scene(&scene, font_file, family);
    FILE *sink = tmpfile();
    CHECK(t, sink != NULL);
    if (!sink) {
        sr_scene_free(&scene);
        return;
    }
    SrDiagnostics diag;
    sr_diag_init(&diag, "text", sink);
    CHECK(t, asset && sr_assets_load(&scene, &diag) == SR_ERR_ASSET);
    CHECK_INT(t, diag.errors, 1);
    fflush(sink);
    rewind(sink);
    char message[1024] = {0};
    CHECK(t, fread(message, 1, sizeof(message) - 1, sink) > 0);
    CHECK_CONTAINS(t, message, needle);
    sr_scene_free(&scene);
    fclose(sink);
}

static void test_missing_fonts(sr_test_ctx *t)
{
    expect_font_error(t, NULL, "No Such Family Xyzzy", "'No Such Family Xyzzy'");
    expect_font_error(t, "/nonexistent/font-file.ttf", NULL,
                      "cannot open font file '/nonexistent/font-file.ttf'");
    /* A file that exists but is not a font. */
    expect_font_error(t, sr_test_data_path("tests/data-media.xml"), NULL,
                      "not a readable font");
}

const sr_test_case sr_tests_text[] = {
    {"kerning", test_kerning},
    {"ligature", test_ligature},
    {"rtl_start_is_right", test_rtl_start_is_right},
    {"bidi_mixed_visual_order", test_bidi_mixed_visual_order},
    {"wrap_and_justify", test_wrap_and_justify},
    {"letter_spacing", test_letter_spacing},
    {"alignment_and_overflow", test_alignment_and_overflow},
    {"rasterize_deterministic", test_rasterize_deterministic},
    {"render_thread_invariant", test_render_thread_invariant},
    {"missing_fonts", test_missing_fonts},
    {NULL, NULL},
};
