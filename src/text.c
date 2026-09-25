#include "scene_render/text.h"
#include "scene_render/color.h"

#include <errno.h>
#include <limits.h>
#include <math.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>

#include <fontconfig/fontconfig.h>
#include <fribidi.h>
#include <ft2build.h>
#include FT_FREETYPE_H
#include FT_OUTLINE_H
#include <hb.h>

/* ------------------------------------------------------------------ fonts */

struct SrFont {
    FT_Library library;
    FT_Face face;
    hb_face_t *hb_face;
};

static void set_err(char *err, size_t errlen, const char *format, ...) {
    if (!err || !errlen) return;
    va_list args;
    va_start(args, format);
    vsnprintf(err, errlen, format, args);
    va_end(args);
}

SrStatus sr_font_open(const char *path, int face_index, SrFont **out,
                      char *err, size_t errlen) {
    *out = NULL;
    if (!path || face_index < 0) return SR_ERR_ARGUMENT;
    FILE *probe = fopen(path, "rb");
    if (!probe) {
        set_err(err, errlen, "%s", strerror(errno));
        return SR_ERR_ASSET;
    }
    fclose(probe);
    SrFont *font = calloc(1, sizeof(*font));
    if (!font) return SR_ERR_MEMORY;
    FT_Error error = FT_Init_FreeType(&font->library);
    if (!error) error = FT_New_Face(font->library, path, face_index, &font->face);
    if (error) {
        set_err(err, errlen, "not a readable font (FreeType error %d)", (int)error);
        sr_font_close(font);
        return error == FT_Err_Out_Of_Memory ? SR_ERR_MEMORY : SR_ERR_ASSET;
    }
    if (!FT_IS_SCALABLE(font->face)) {
        set_err(err, errlen, "not a scalable outline font");
        sr_font_close(font);
        return SR_ERR_ASSET;
    }
    hb_blob_t *blob = hb_blob_create_from_file_or_fail(path);
    if (!blob) {
        set_err(err, errlen, "HarfBuzz cannot read the font file");
        sr_font_close(font);
        return SR_ERR_ASSET;
    }
    font->hb_face = hb_face_create(blob, (unsigned)face_index);
    hb_blob_destroy(blob);
    if (!font->hb_face || hb_face_get_glyph_count(font->hb_face) == 0) {
        set_err(err, errlen, "HarfBuzz found no glyphs in face %d", face_index);
        sr_font_close(font);
        return SR_ERR_ASSET;
    }
    *out = font;
    return SR_OK;
}

void sr_font_close(SrFont *font) {
    if (!font) return;
    if (font->hb_face) hb_face_destroy(font->hb_face);
    if (font->face) FT_Done_Face(font->face);
    if (font->library) FT_Done_FreeType(font->library);
    free(font);
}

bool sr_font_has_codepoint(const SrFont *font, uint32_t codepoint) {
    return font && FT_Get_Char_Index(font->face, codepoint) != 0;
}

/* ------------------------------------------------------------- fontconfig */

static bool generic_family(const char *family) {
    static const char *const generic[] = {"sans", "sans-serif", "serif",
                                          "monospace", "mono", "system-ui"};
    for (size_t i = 0; i < sizeof(generic) / sizeof(generic[0]); ++i)
        if (strcasecmp(family, generic[i]) == 0) return true;
    return false;
}

/* FcNameParse treats '-', ':' and ',' as syntax; escape them so the whole
 * attribute is one literal family name. */
static char *escape_family(const char *family) {
    size_t length = 0;
    for (const char *p = family; *p; ++p)
        length += strchr("\\-:,", *p) ? 2 : 1;
    char *escaped = malloc(length + 1);
    if (!escaped) return NULL;
    char *out = escaped;
    for (const char *p = family; *p; ++p) {
        if (strchr("\\-:,", *p)) *out++ = '\\';
        *out++ = *p;
    }
    *out = '\0';
    return escaped;
}

static SrStatus match_family(FcConfig *config, const char *family, char **path,
                             int *face_index, char *err, size_t errlen) {
    *path = NULL;
    *face_index = 0;
    char *escaped = escape_family(family);
    if (!escaped) return SR_ERR_MEMORY;
    FcPattern *pattern = FcNameParse((const FcChar8 *)escaped);
    free(escaped);
    if (!pattern) return SR_ERR_MEMORY;
    FcConfigSubstitute(config, pattern, FcMatchPattern);
    FcDefaultSubstitute(pattern);
    FcResult result = FcResultNoMatch;
    FcPattern *match = FcFontMatch(config, pattern, &result);
    FcPatternDestroy(pattern);
    if (!match) {
        set_err(err, errlen, "no installed font matches family '%s'", family);
        return SR_ERR_ASSET;
    }
    bool same = generic_family(family);
    FcChar8 *name = NULL;
    const char *first = NULL;
    for (int i = 0; !same && FcPatternGetString(match, FC_FAMILY, i, &name) ==
                                 FcResultMatch; ++i) {
        if (!first) first = (const char *)name;
        same = strcasecmp((const char *)name, family) == 0;
    }
    FcChar8 *file = NULL;
    int index = 0;
    SrStatus status = SR_OK;
    if (!same) {
        set_err(err, errlen,
                "font family '%s' is not installed (Fontconfig's closest "
                "match is '%s')", family, first ? first : "unknown");
        status = SR_ERR_ASSET;
    } else if (FcPatternGetString(match, FC_FILE, 0, &file) != FcResultMatch) {
        set_err(err, errlen, "Fontconfig match for '%s' has no file", family);
        status = SR_ERR_ASSET;
    } else {
        if (FcPatternGetInteger(match, FC_INDEX, 0, &index) != FcResultMatch)
            index = 0;
        *path = sr_strdup((const char *)file);
        *face_index = index & 0xFFFF; /* upper bits select a named instance */
        if (!*path) status = SR_ERR_MEMORY;
    }
    FcPatternDestroy(match);
    return status;
}

SrStatus sr_font_match_family(const char *family, char **path, int *face_index,
                              char *err, size_t errlen) {
    *path = NULL;
    if (!family || !*family) return SR_ERR_ARGUMENT;
    FcConfig *config = FcInitLoadConfigAndFonts();
    if (!config) {
        set_err(err, errlen, "cannot load the Fontconfig configuration");
        return SR_ERR_ASSET;
    }
    SrStatus status = match_family(config, family, path, face_index, err, errlen);
    FcConfigDestroy(config);
    return status;
}

/* ------------------------------------------------------------ font cache */

typedef struct {
    char *path;
    int index;
    SrFont *font;
} FontEntry;

typedef struct {
    char *family;
    char *path;
    int index;
} FamilyEntry;

struct SrFontCache {
    FontEntry *fonts;
    size_t font_count;
    FamilyEntry *families;
    size_t family_count;
    FcConfig *config;
};

void sr_font_cache_free(SrFontCache *cache) {
    if (!cache) return;
    for (size_t i = 0; i < cache->font_count; ++i) {
        free(cache->fonts[i].path);
        sr_font_close(cache->fonts[i].font);
    }
    for (size_t i = 0; i < cache->family_count; ++i) {
        free(cache->families[i].family);
        free(cache->families[i].path);
    }
    free(cache->fonts);
    free(cache->families);
    if (cache->config) FcConfigDestroy(cache->config);
    free(cache);
}

static SrStatus cache_family(SrFontCache *cache, const char *family,
                             const char **path, int *index, char *err,
                             size_t errlen) {
    for (size_t i = 0; i < cache->family_count; ++i)
        if (strcmp(cache->families[i].family, family) == 0) {
            *path = cache->families[i].path;
            *index = cache->families[i].index;
            return SR_OK;
        }
    if (!cache->config) {
        cache->config = FcInitLoadConfigAndFonts();
        if (!cache->config) {
            set_err(err, errlen, "cannot load the Fontconfig configuration");
            return SR_ERR_ASSET;
        }
    }
    char *resolved = NULL;
    SrStatus status = match_family(cache->config, family, &resolved, index,
                                   err, errlen);
    if (status != SR_OK) return status;
    FamilyEntry *grown = sr_realloc(cache->families, (cache->family_count + 1) *
                                                         sizeof(*grown));
    char *name = sr_strdup(family);
    if (!grown || !name) {
        if (grown) cache->families = grown;
        free(name);
        free(resolved);
        return SR_ERR_MEMORY;
    }
    cache->families = grown;
    cache->families[cache->family_count++] =
        (FamilyEntry){.family = name, .path = resolved, .index = *index};
    *path = resolved;
    return SR_OK;
}

static SrStatus cache_font(SrFontCache *cache, const char *path, int index,
                           SrFont **font, char *err, size_t errlen) {
    for (size_t i = 0; i < cache->font_count; ++i)
        if (cache->fonts[i].index == index &&
            strcmp(cache->fonts[i].path, path) == 0) {
            *font = cache->fonts[i].font;
            return SR_OK;
        }
    SrStatus status = sr_font_open(path, index, font, err, errlen);
    if (status != SR_OK) return status;
    FontEntry *grown = sr_realloc(cache->fonts,
                                  (cache->font_count + 1) * sizeof(*grown));
    char *key = sr_strdup(path);
    if (!grown || !key) {
        if (grown) cache->fonts = grown;
        free(key);
        sr_font_close(*font);
        *font = NULL;
        return SR_ERR_MEMORY;
    }
    cache->fonts = grown;
    cache->fonts[cache->font_count++] =
        (FontEntry){.path = key, .index = index, .font = *font};
    return SR_OK;
}

/* ---------------------------------------------------------------- layout */

typedef struct {
    uint32_t *cp;          /* code points of one paragraph */
    size_t *byte;          /* byte offset of each code point (n + 1) */
    FriBidiLevel *levels;
    hb_script_t *scripts;
    double *advance;       /* shaped advance attributed to each code point */
    bool *cluster_start;
    size_t n;
    size_t byte_base;      /* paragraph offset in the source text */
    bool rtl;
} Paragraph;

typedef struct {
    uint32_t glyph;
    uint32_t cluster;      /* code point index in the paragraph */
    double advance;        /* px, letter spacing included */
    double x_offset;
    double y_offset;
    bool space;
    bool cluster_end;
} Shaped;

typedef struct {
    Shaped *items;
    size_t count;
    size_t capacity;
} ShapedVec;

typedef struct {
    size_t start;
    size_t end;
    FriBidiLevel level;
    hb_script_t script;
    size_t first;          /* first shaped glyph */
    size_t count;
} Run;

typedef struct {
    hb_font_t *font;
    hb_buffer_t *buffer;
    hb_language_t language;
    double spacing;
} Shaper;

static bool is_space(uint32_t c) { return c == 0x20 || c == 0x09 || c == 0x3000; }
static bool is_hyphen(uint32_t c) { return c == 0x2D || c == 0x2010; }

/* Decodes one UTF-8 sequence; malformed input decodes as U+FFFD. */
static size_t utf8_next(const unsigned char *s, size_t length, uint32_t *out) {
    unsigned c = s[0];
    size_t extra = c >= 0xF0 && c < 0xF8 ? 3 : c >= 0xE0 ? 2 : c >= 0xC2 ? 1 : 0;
    if (c < 0x80) {
        *out = c;
        return 1;
    }
    if (c >= 0xF8 || (c >= 0x80 && extra == 0) || extra >= length) {
        *out = 0xFFFD;
        return 1;
    }
    uint32_t value = c & (extra == 3 ? 0x07 : extra == 2 ? 0x0F : 0x1F);
    for (size_t i = 1; i <= extra; ++i) {
        if ((s[i] & 0xC0) != 0x80) {
            *out = 0xFFFD;
            return i;
        }
        value = (value << 6) | (s[i] & 0x3F);
    }
    static const uint32_t minimum[4] = {0, 0x80, 0x800, 0x10000};
    if (value < minimum[extra] || value > 0x10FFFF ||
        (value >= 0xD800 && value <= 0xDFFF))
        value = 0xFFFD;
    *out = value;
    return extra + 1;
}

static void paragraph_free(Paragraph *p) {
    free(p->cp);
    free(p->byte);
    free(p->levels);
    free(p->scripts);
    free(p->advance);
    free(p->cluster_start);
    *p = (Paragraph){0};
}

static SrStatus paragraph_init(Paragraph *p, const char *text, size_t begin,
                               size_t end, SrTextDirection direction) {
    *p = (Paragraph){.byte_base = begin};
    size_t length = end - begin;
    if (length > (size_t)INT_MAX - 1) return SR_ERR_ARGUMENT;
    size_t slots = length + 1;
    p->cp = calloc(slots, sizeof(*p->cp));
    p->byte = calloc(slots, sizeof(*p->byte));
    p->levels = calloc(slots, sizeof(*p->levels));
    p->scripts = calloc(slots, sizeof(*p->scripts));
    p->advance = calloc(slots, sizeof(*p->advance));
    p->cluster_start = calloc(slots, sizeof(*p->cluster_start));
    FriBidiCharType *types = calloc(slots, sizeof(*types));
    FriBidiBracketType *brackets = calloc(slots, sizeof(*brackets));
    if (!p->cp || !p->byte || !p->levels || !p->scripts || !p->advance ||
        !p->cluster_start || !types || !brackets) {
        free(types);
        free(brackets);
        paragraph_free(p);
        return SR_ERR_MEMORY;
    }
    const unsigned char *s = (const unsigned char *)text + begin;
    for (size_t at = 0; at < length;) {
        p->byte[p->n] = at;
        at += utf8_next(s + at, length - at, &p->cp[p->n]);
        ++p->n;
    }
    p->byte[p->n] = length;
    p->rtl = direction == SR_TEXT_DIR_RTL;
    if (p->n) {
        FriBidiParType base = direction == SR_TEXT_DIR_LTR   ? FRIBIDI_PAR_LTR
                              : direction == SR_TEXT_DIR_RTL ? FRIBIDI_PAR_RTL
                                                             : FRIBIDI_PAR_ON;
        FriBidiStrIndex n = (FriBidiStrIndex)p->n;
        fribidi_get_bidi_types(p->cp, n, types);
        fribidi_get_bracket_types(p->cp, n, types, brackets);
        if (!fribidi_get_par_embedding_levels_ex(types, brackets, n, &base,
                                                 p->levels)) {
            free(types);
            free(brackets);
            paragraph_free(p);
            return SR_ERR_MEMORY;
        }
        p->rtl = FRIBIDI_IS_RTL(base);
    }
    free(types);
    free(brackets);
    /* Script itemization: Common/Inherited/Unknown characters join the
     * script of the preceding character (or the first real script). */
    hb_unicode_funcs_t *unicode = hb_unicode_funcs_get_default();
    hb_script_t current = HB_SCRIPT_COMMON;
    for (size_t i = 0; i < p->n; ++i) {
        hb_script_t script = hb_unicode_script(unicode, p->cp[i]);
        if (script != HB_SCRIPT_COMMON && script != HB_SCRIPT_INHERITED &&
            script != HB_SCRIPT_UNKNOWN) {
            if (current == HB_SCRIPT_COMMON)
                for (size_t k = 0; k < i; ++k) p->scripts[k] = script;
            current = script;
        }
        p->scripts[i] = current;
    }
    if (current == HB_SCRIPT_COMMON)
        for (size_t i = 0; i < p->n; ++i) p->scripts[i] = HB_SCRIPT_LATIN;
    return SR_OK;
}

static SrStatus shaped_push(ShapedVec *vec, Shaped item) {
    if (vec->count == vec->capacity) {
        size_t capacity = vec->capacity ? vec->capacity * 2 : 64;
        Shaped *grown = sr_realloc(vec->items, capacity * sizeof(*grown));
        if (!grown) return SR_ERR_MEMORY;
        vec->items = grown;
        vec->capacity = capacity;
    }
    vec->items[vec->count++] = item;
    return SR_OK;
}

/* Shapes [start, end) of a paragraph run by run (constant bidi level and
 * script). Runs are returned in logical order; each run's glyphs are in
 * visual order, as HarfBuzz emits them. Letter spacing is added after every
 * cluster. */
static SrStatus shape_range(const Shaper *shaper, const Paragraph *p,
                            size_t start, size_t end, ShapedVec *out,
                            Run **runs_out, size_t *run_count) {
    *runs_out = NULL;
    *run_count = 0;
    out->count = 0;
    size_t count = 0;
    for (size_t i = start; i < end; ++i)
        if (i == start || p->levels[i] != p->levels[i - 1] ||
            p->scripts[i] != p->scripts[i - 1])
            ++count;
    if (!count) return SR_OK;
    Run *runs = calloc(count, sizeof(*runs));
    if (!runs) return SR_ERR_MEMORY;
    size_t r = 0;
    for (size_t i = start; i < end; ++i) {
        if (i == start || p->levels[i] != p->levels[i - 1] ||
            p->scripts[i] != p->scripts[i - 1])
            runs[r++] = (Run){.start = i, .level = p->levels[i],
                              .script = p->scripts[i]};
        runs[r - 1].end = i + 1;
    }
    for (r = 0; r < count; ++r) {
        Run *run = &runs[r];
        hb_buffer_clear_contents(shaper->buffer);
        hb_buffer_add_codepoints(shaper->buffer, p->cp, (int)p->n,
                                 (unsigned)run->start,
                                 (int)(run->end - run->start));
        hb_buffer_set_direction(shaper->buffer, run->level & 1
                                                    ? HB_DIRECTION_RTL
                                                    : HB_DIRECTION_LTR);
        hb_buffer_set_script(shaper->buffer, run->script);
        if (shaper->language != HB_LANGUAGE_INVALID)
            hb_buffer_set_language(shaper->buffer, shaper->language);
        hb_buffer_set_flags(shaper->buffer,
                            (hb_buffer_flags_t)((run->start == 0 ? HB_BUFFER_FLAG_BOT : 0) |
                                                (run->end == p->n ? HB_BUFFER_FLAG_EOT : 0)));
        hb_shape(shaper->font, shaper->buffer, NULL, 0);
        unsigned length = 0;
        const hb_glyph_info_t *info =
            hb_buffer_get_glyph_infos(shaper->buffer, &length);
        const hb_glyph_position_t *pos =
            hb_buffer_get_glyph_positions(shaper->buffer, NULL);
        run->first = out->count;
        run->count = length;
        for (unsigned g = 0; g < length; ++g) {
            bool cluster_end = g + 1 == length || info[g + 1].cluster != info[g].cluster;
            Shaped item = {
                .glyph = info[g].codepoint,
                .cluster = info[g].cluster,
                .advance = pos[g].x_advance / 64.0 +
                           (cluster_end ? shaper->spacing : 0.0),
                .x_offset = pos[g].x_offset / 64.0,
                .y_offset = pos[g].y_offset / 64.0,
                .space = info[g].cluster < p->n && is_space(p->cp[info[g].cluster]),
                .cluster_end = cluster_end,
            };
            if (shaped_push(out, item) != SR_OK) {
                free(runs);
                return SR_ERR_MEMORY;
            }
        }
    }
    *runs_out = runs;
    *run_count = count;
    return SR_OK;
}

/* UAX #9 rule L2 on whole runs: from the highest level down to the lowest
 * odd level, reverse every maximal sequence of runs at that level or above. */
static void reorder_runs(const Run *runs, size_t count, size_t *order) {
    int high = 0, low = INT_MAX;
    for (size_t i = 0; i < count; ++i) {
        order[i] = i;
        if (runs[i].level > high) high = runs[i].level;
        if (runs[i].level < low) low = runs[i].level;
    }
    if (!count) return;
    int lowest_odd = low | 1;
    for (int level = high; level >= lowest_odd; --level) {
        for (size_t i = 0; i < count;) {
            if (runs[order[i]].level < level) {
                ++i;
                continue;
            }
            size_t j = i;
            while (j < count && runs[order[j]].level >= level) ++j;
            for (size_t a = i, b = j - 1; a < b; ++a, --b) {
                size_t t = order[a];
                order[a] = order[b];
                order[b] = t;
            }
            i = j;
        }
    }
}

static SrStatus layout_push_glyph(SrTextLayout *layout, SrTextGlyph glyph) {
    if (layout->glyph_count == layout->glyph_capacity) {
        size_t capacity = layout->glyph_capacity ? layout->glyph_capacity * 2 : 64;
        SrTextGlyph *grown = sr_realloc(layout->glyphs, capacity * sizeof(*grown));
        if (!grown) return SR_ERR_MEMORY;
        layout->glyphs = grown;
        layout->glyph_capacity = capacity;
    }
    layout->glyphs[layout->glyph_count++] = glyph;
    return SR_OK;
}

static SrStatus layout_push_line(SrTextLayout *layout, SrTextLine line) {
    if (layout->line_count == layout->line_capacity) {
        size_t capacity = layout->line_capacity ? layout->line_capacity * 2 : 8;
        SrTextLine *grown = sr_realloc(layout->lines, capacity * sizeof(*grown));
        if (!grown) return SR_ERR_MEMORY;
        layout->lines = grown;
        layout->line_capacity = capacity;
    }
    layout->lines[layout->line_count++] = line;
    return SR_OK;
}

/* Tolerance for line fitting: one 26.6 unit. */
#define SR_TEXT_FIT_EPSILON (1.0 / 64.0)

/* Shapes one visible line [start, end), orders it visually, aligns it and
 * appends its glyphs (baseline filled in later). */
static SrStatus emit_line(const Shaper *shaper, const Paragraph *p,
                          const SrTextStyle *style, size_t start, size_t end,
                          bool paragraph_end, ShapedVec *scratch,
                          SrTextLayout *layout) {
    Run *runs = NULL;
    size_t run_count = 0;
    SrStatus status = shape_range(shaper, p, start, end, scratch, &runs, &run_count);
    if (status != SR_OK) return status;
    size_t *order = calloc(run_count ? run_count : 1, sizeof(*order));
    if (!order) {
        free(runs);
        return SR_ERR_MEMORY;
    }
    reorder_runs(runs, run_count, order);
    double total = 0.0;
    size_t clusters = 0, spaces = 0;
    for (size_t i = 0; i < scratch->count; ++i) {
        total += scratch->items[i].advance;
        clusters += scratch->items[i].cluster_end;
        spaces += scratch->items[i].space && scratch->items[i].cluster_end;
    }
    double width = total - (clusters ? shaper->spacing : 0.0);
    double box = style->width;
    SrTextAlign align = style->align;
    double per_space = 0.0;
    if (align == SR_TEXT_ALIGN_JUSTIFY) {
        if (!paragraph_end && spaces && width < box) {
            per_space = (box - width) / (double)spaces;
            width = box;
        }
        align = SR_TEXT_ALIGN_START;
    }
    bool right = (align == SR_TEXT_ALIGN_START && p->rtl) ||
                 (align == SR_TEXT_ALIGN_END && !p->rtl);
    double x0 = align == SR_TEXT_ALIGN_CENTER ? (box - width) / 2.0
                : right                       ? box - width
                                              : 0.0;
    SrTextLine line = {
        .first_glyph = layout->glyph_count,
        .glyph_count = scratch->count,
        .cluster_count = clusters,
        .byte_start = p->byte_base + p->byte[start],
        .byte_end = p->byte_base + p->byte[end],
        .x = x0,
        .width = width,
        .ink_left = HUGE_VAL,
        .ink_right = -HUGE_VAL,
        .rtl = p->rtl,
        .paragraph_end = paragraph_end,
    };
    double pen = x0;
    for (size_t r = 0; r < run_count && status == SR_OK; ++r) {
        const Run *run = &runs[order[r]];
        for (size_t g = run->first; g < run->first + run->count; ++g) {
            const Shaped *item = &scratch->items[g];
            SrTextGlyph glyph = {
                .glyph = item->glyph,
                .cluster = p->byte_base + p->byte[item->cluster],
                .x = pen + item->x_offset,
                .y = -item->y_offset,   /* baseline added later */
                .advance = item->advance +
                           (item->space && item->cluster_end ? per_space : 0.0),
            };
            hb_glyph_extents_t extents;
            if (hb_font_get_glyph_extents(shaper->font, item->glyph, &extents) &&
                extents.width != 0) {
                double left = glyph.x + extents.x_bearing / 64.0;
                double right_edge = left + extents.width / 64.0;
                if (left < line.ink_left) line.ink_left = left;
                if (right_edge > line.ink_right) line.ink_right = right_edge;
            }
            pen += glyph.advance;
            status = layout_push_glyph(layout, glyph);
            if (status != SR_OK) break;
        }
    }
    if (line.ink_left > line.ink_right) line.ink_left = line.ink_right = x0;
    if (status == SR_OK) status = layout_push_line(layout, line);
    free(order);
    free(runs);
    return status;
}

/* Greedy line breaking on the paragraph's logical advances: break after a
 * run of spaces or after a hyphen; a word wider than the box breaks at the
 * last HarfBuzz cluster boundary that fits. Spaces at a break hang. */
static SrStatus layout_paragraph(const Shaper *shaper, Paragraph *p,
                                 const SrTextStyle *style, ShapedVec *scratch,
                                 SrTextLayout *layout) {
    Run *runs = NULL;
    size_t run_count = 0;
    SrStatus status = shape_range(shaper, p, 0, p->n, scratch, &runs, &run_count);
    free(runs);
    if (status != SR_OK) return status;
    for (size_t g = 0; g < scratch->count; ++g) {
        const Shaped *item = &scratch->items[g];
        if (item->cluster >= p->n) continue;
        p->advance[item->cluster] += item->advance;
        p->cluster_start[item->cluster] = true;
    }
    if (!p->n) return emit_line(shaper, p, style, 0, 0, true, scratch, layout);
    double box = style->width + SR_TEXT_FIT_EPSILON;
    size_t start = 0;
    while (start < p->n && status == SR_OK) {
        double width = 0.0;
        size_t opportunity = 0, end = p->n;
        for (size_t i = start; i < p->n; ++i) {
            uint32_t c = p->cp[i];
            width += p->advance[i];
            if (is_space(c)) {
                if (i + 1 < p->n && !is_space(p->cp[i + 1])) opportunity = i + 1;
                continue;
            }
            if (i > start && width - shaper->spacing > box) {
                if (opportunity > start) {
                    end = opportunity;
                } else {
                    size_t j = i;
                    while (j > start && !p->cluster_start[j]) --j;
                    if (j == start) {
                        j = start + 1;
                        while (j < p->n && !p->cluster_start[j]) ++j;
                    }
                    end = j;
                }
                break;
            }
            if (is_hyphen(c) && i > start && i + 1 < p->n && !is_space(p->cp[i + 1]))
                opportunity = i + 1;
        }
        size_t visible = end;
        while (visible > start && is_space(p->cp[visible - 1])) --visible;
        status = emit_line(shaper, p, style, start, visible, end == p->n,
                           scratch, layout);
        start = end;
    }
    return status;
}

void sr_text_layout_free(SrTextLayout *layout) {
    if (!layout) return;
    free(layout->glyphs);
    free(layout->lines);
    *layout = (SrTextLayout){0};
}

static int64_t size_26_6(double size) { return (int64_t)llround(size * 64.0); }

SrStatus sr_text_layout(SrFont *font, const char *utf8, const SrTextStyle *style,
                        SrTextLayout *out, char *err, size_t errlen) {
    *out = (SrTextLayout){0};
    if (!font || !utf8 || !style || !(style->size > 0.0) || style->size > 16384.0 ||
        !isfinite(style->letter_spacing) || !style->width || !style->height) {
        set_err(err, errlen, "invalid text style");
        return SR_ERR_ARGUMENT;
    }
    double line_height = style->line_height > 0.0 ? style->line_height : 1.2;
    int scale = (int)size_26_6(style->size);
    Shaper shaper = {
        .font = hb_font_create(font->hb_face),
        .buffer = hb_buffer_create(),
        .language = style->language && *style->language
                        ? hb_language_from_string(style->language, -1)
                        : HB_LANGUAGE_INVALID,
        .spacing = style->letter_spacing,
    };
    if (!hb_buffer_allocation_successful(shaper.buffer)) {
        hb_buffer_destroy(shaper.buffer);
        hb_font_destroy(shaper.font);
        return SR_ERR_MEMORY;
    }
    hb_font_set_scale(shaper.font, scale, scale);
    ShapedVec scratch = {0};
    SrStatus status = SR_OK;
    size_t length = strlen(utf8);
    for (size_t begin = 0; status == SR_OK;) {
        const char *newline = memchr(utf8 + begin, '\n', length - begin);
        size_t end = newline ? (size_t)(newline - utf8) : length;
        size_t stop = end > begin && utf8[end - 1] == '\r' ? end - 1 : end;
        Paragraph paragraph;
        status = paragraph_init(&paragraph, utf8, begin, stop, style->direction);
        if (status == SR_OK) {
            status = layout_paragraph(&shaper, &paragraph, style, &scratch, out);
            paragraph_free(&paragraph);
        }
        if (!newline) break;
        begin = end + 1;
    }
    if (status == SR_OK) {
        hb_font_extents_t extents = {0};
        hb_font_get_h_extents(shaper.font, &extents);
        double ascender = extents.ascender / 64.0;
        double descender = -extents.descender / 64.0;
        double advance = line_height * style->size;
        double leading = (advance - (ascender + descender)) / 2.0;
        out->block_height = advance * (double)out->line_count;
        double top = style->valign == SR_TEXT_VALIGN_MIDDLE
                         ? (style->height - out->block_height) / 2.0
                     : style->valign == SR_TEXT_VALIGN_BOTTOM
                         ? style->height - out->block_height
                         : 0.0;
        /* Overflow means ink actually lost: a glyph outline crossing the top
         * or bottom edge (half a pixel of anti-aliasing tolerated), or a line
         * wider than the box (a single cluster wider than the width). */
        for (size_t i = 0; i < out->line_count; ++i) {
            SrTextLine *line = &out->lines[i];
            line->baseline = floor(top + advance * (double)i + leading + ascender + 0.5);
            for (size_t g = 0; g < line->glyph_count; ++g) {
                SrTextGlyph *glyph = &out->glyphs[line->first_glyph + g];
                glyph->y += line->baseline;
                hb_glyph_extents_t ink;
                if (!hb_font_get_glyph_extents(shaper.font, glyph->glyph, &ink) ||
                    ink.width == 0 || ink.height == 0)
                    continue;
                double ink_top = glyph->y - ink.y_bearing / 64.0;
                double ink_bottom = ink_top - ink.height / 64.0;
                if (ink_top < -0.5 || ink_bottom > style->height + 0.5)
                    out->overflow = true;
            }
            if (line->width > style->width + SR_TEXT_FIT_EPSILON)
                out->overflow = true;
        }
    }
    free(scratch.items);
    hb_buffer_destroy(shaper.buffer);
    hb_font_destroy(shaper.font);
    if (status != SR_OK) {
        sr_text_layout_free(out);
        set_err(err, errlen, status == SR_ERR_MEMORY ? "out of memory"
                                                     : "text is too long");
    }
    return status;
}

/* ----------------------------------------------------------- rasterizing */

static int64_t quarter(double value) { return (int64_t)floor(value * 4.0 + 0.5); }

static int64_t floor_div4(int64_t value) {
    return value >= 0 ? value / 4 : -((-value + 3) / 4);
}

SrStatus sr_text_rasterize(SrFont *font, const SrTextLayout *layout,
                           double size, uint32_t width, uint32_t height,
                           float *coverage) {
    if (!font || !layout || !coverage || !(size > 0.0) || size > 16384.0)
        return SR_ERR_ARGUMENT;
    FT_Face face = font->face;
    FT_F26Dot6 char_size = (FT_F26Dot6)size_26_6(size);
    if (FT_Set_Char_Size(face, char_size, char_size, 72, 72) != 0)
        return SR_ERR_ASSET;
    for (size_t g = 0; g < layout->glyph_count; ++g) {
        const SrTextGlyph *glyph = &layout->glyphs[g];
        int64_t qx = quarter(glyph->x), qy = quarter(glyph->y);
        int64_t ix = floor_div4(qx), iy = floor_div4(qy);
        if (FT_Load_Glyph(face, glyph->glyph, FT_LOAD_NO_HINTING | FT_LOAD_NO_BITMAP) != 0)
            return SR_ERR_ASSET;
        FT_GlyphSlot slot = face->glyph;
        if (slot->format == FT_GLYPH_FORMAT_OUTLINE) {
            if (slot->outline.n_points == 0) continue;
            /* Quarter-pixel phase; FreeType's y axis points up. */
            FT_Outline_Translate(&slot->outline, (FT_Pos)((qx - ix * 4) * 16),
                                 -(FT_Pos)((qy - iy * 4) * 16));
        }
        if (FT_Render_Glyph(slot, FT_RENDER_MODE_NORMAL) != 0) return SR_ERR_ASSET;
        const FT_Bitmap *bitmap = &slot->bitmap;
        if (bitmap->pixel_mode != FT_PIXEL_MODE_GRAY &&
            bitmap->pixel_mode != FT_PIXEL_MODE_MONO)
            continue;
        int64_t left = ix + slot->bitmap_left, top = iy - slot->bitmap_top;
        for (unsigned row = 0; row < bitmap->rows; ++row) {
            int64_t y = top + (int64_t)row;
            if (y < 0 || y >= (int64_t)height) continue;
            const unsigned char *src = bitmap->buffer + (ptrdiff_t)row * bitmap->pitch;
            float *dst = coverage + (size_t)y * width;
            for (unsigned col = 0; col < bitmap->width; ++col) {
                int64_t x = left + (int64_t)col;
                if (x < 0 || x >= (int64_t)width) continue;
                float value = bitmap->pixel_mode == FT_PIXEL_MODE_GRAY
                                  ? src[col] / 255.0f
                                  : (float)((src[col >> 3] >> (7 - (col & 7))) & 1);
                float sum = dst[x] + value;
                dst[x] = sum > 1.0f ? 1.0f : sum;
            }
        }
    }
    return SR_OK;
}

/* ----------------------------------------------------------------- asset */

SrStatus sr_text_render_asset(SrScene *scene, SrAsset *asset, SrDiagnostics *diag) {
    if (!scene->font_cache) {
        scene->font_cache = calloc(1, sizeof(*scene->font_cache));
        if (!scene->font_cache) return SR_ERR_MEMORY;
    }
    SrFontCache *cache = scene->font_cache;
    char err[256] = "";
    const char *attribute = asset->font_file ? "fontFile" : "font";
    SrFont *font = NULL;
    SrStatus status;
    if (asset->font_file) {
        char *path = sr_path_join(scene->base_dir, asset->font_file);
        if (!path) return SR_ERR_MEMORY;
        status = cache_font(cache, path, 0, &font, err, sizeof(err));
        if (status != SR_OK && status != SR_ERR_MEMORY)
            sr_diag_error(diag, asset->source_line, "text", attribute,
                          "cannot open font file '%s': %s", path, err);
        free(path);
    } else {
        const char *family = asset->font_family ? asset->font_family : "sans-serif";
        const char *path = NULL;
        int index = 0;
        status = cache_family(cache, family, &path, &index, err, sizeof(err));
        if (status == SR_OK) {
            status = cache_font(cache, path, index, &font, err, sizeof(err));
            if (status != SR_OK && status != SR_ERR_MEMORY)
                sr_diag_error(diag, asset->source_line, "text", attribute,
                              "cannot open font '%s' for family '%s': %s", path,
                              family, err);
        } else if (status != SR_ERR_MEMORY) {
            sr_diag_error(diag, asset->source_line, "text", attribute, "%s", err);
        }
    }
    if (status != SR_OK) return status == SR_ERR_MEMORY ? status : SR_ERR_ASSET;

    SrTextStyle style = {
        .size = asset->text_size,
        .line_height = asset->text_line_height,
        .letter_spacing = asset->text_letter_spacing,
        .align = asset->text_align,
        .direction = asset->text_direction,
        .valign = asset->text_valign,
        .language = asset->text_language,
        .width = asset->width,
        .height = asset->height,
    };
    SrTextLayout layout;
    status = sr_text_layout(font, asset->text ? asset->text : "", &style, &layout,
                            err, sizeof(err));
    if (status != SR_OK) {
        sr_diag_error(diag, asset->source_line, "text", "text",
                      "cannot lay out text: %s", err);
        return status == SR_ERR_MEMORY ? status : SR_ERR_ASSET;
    }
    sr_diag_info(diag, "text asset '%s': %zu line(s) laid out in %ux%u",
                 asset->id ? asset->id : "", layout.line_count, asset->width,
                 asset->height);
    if (layout.overflow)
        sr_diag_warning(diag, asset->source_line, "text", "width/height",
                        "text of asset '%s' overflows its %ux%u box and is "
                        "clipped", asset->id ? asset->id : "", asset->width,
                        asset->height);
    size_t pixels = (size_t)asset->width * asset->height;
    if (pixels / asset->height != asset->width || pixels > SIZE_MAX / (4 * sizeof(float))) {
        sr_text_layout_free(&layout);
        return SR_ERR_MEMORY;
    }
    float *coverage = calloc(pixels, sizeof(*coverage));
    SrImage *image = calloc(1, sizeof(*image));
    float *px = coverage && image ? malloc(pixels * 4 * sizeof(*px)) : NULL;
    if (!px) {
        free(coverage);
        free(image);
        sr_text_layout_free(&layout);
        return SR_ERR_MEMORY;
    }
    status = sr_text_rasterize(font, &layout, asset->text_size, asset->width,
                               asset->height, coverage);
    sr_text_layout_free(&layout);
    if (status != SR_OK) {
        sr_diag_error(diag, asset->source_line, "text", attribute,
                      "FreeType cannot rasterize a glyph of asset '%s'",
                      asset->id ? asset->id : "");
        free(coverage);
        free(image);
        free(px);
        return status == SR_ERR_MEMORY ? status : SR_ERR_ASSET;
    }
    /* Text colors are working-space values, like every XML color. */
    float color[4];
    sr_color_to_blend(&scene->project, asset->color, color);
    for (size_t i = 0; i < pixels; ++i)
        for (int c = 0; c < 4; ++c) px[i * 4 + c] = color[c] * coverage[i];
    free(coverage);
    *image = (SrImage){asset->width, asset->height, px};
    asset->decoded = image;
    return SR_OK;
}
