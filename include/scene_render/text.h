#ifndef SCENE_RENDER_TEXT_H
#define SCENE_RENDER_TEXT_H

/* In-process text engine: Fontconfig font lookup, FriBidi bidirectional
 * reordering (UAX #9, per line), HarfBuzz shaping, line breaking and
 * alignment inside the asset box, FreeType rasterization. See
 * docs/xml-reference.md for the XML surface and docs/architecture.md. */

#include "scene_render/diagnostics.h"
#include "scene_render/scene.h"

/* One font face. Each font owns its FreeType library and HarfBuzz face, so
 * fonts share no mutable state; a single font is not thread-safe. */
typedef struct SrFont SrFont;

SrStatus sr_font_open(const char *path, int face_index, SrFont **out,
                      char *err, size_t errlen);
void sr_font_close(SrFont *font);
bool sr_font_has_codepoint(const SrFont *font, uint32_t codepoint);

/* Resolves a family name through Fontconfig (FcNameParse, substitution,
 * FcFontMatch). Fails with SR_ERR_ASSET when the best match belongs to a
 * different family (compared case-insensitively), except for the generic
 * aliases sans/sans-serif/serif/monospace/mono/system-ui. *path is owned by
 * the caller. */
SrStatus sr_font_match_family(const char *family, char **path, int *face_index,
                              char *err, size_t errlen);

/* Per-scene cache of opened fonts keyed by (resolved path, face index), plus
 * the Fontconfig configuration used for family lookup. */
typedef struct SrFontCache SrFontCache;
void sr_font_cache_free(SrFontCache *cache);
/* Fonts opened through the cache (NULL cache: none), and the resolved file
 * path of font i (NULL when i is out of range): explicit font files and
 * the files font families resolved to alike. */
size_t sr_font_cache_count(const SrFontCache *cache);
const char *sr_font_cache_path(const SrFontCache *cache, size_t i);

typedef struct {
    double size;            /* px per em */
    double line_height;     /* baseline distance as a multiple of size */
    double letter_spacing;  /* px added after each cluster */
    SrTextAlign align;
    SrTextDirection direction;
    SrTextVAlign valign;
    const char *language;   /* BCP-47 or NULL */
    uint32_t width;         /* box: lines wrap to width */
    uint32_t height;
} SrTextStyle;

typedef struct {
    uint32_t glyph;         /* font glyph index */
    size_t cluster;         /* byte offset of the cluster in the source text */
    double x;               /* pen origin in box pixels (y grows down) */
    double y;
    double advance;         /* px: letter spacing and justification included */
} SrTextGlyph;

typedef struct {
    size_t first_glyph;     /* glyphs in visual (left-to-right) order */
    size_t glyph_count;
    size_t cluster_count;
    size_t byte_start;      /* visible source bytes [start, end) */
    size_t byte_end;
    double x;               /* left edge of the line's advance box */
    double width;           /* advance width (justification included) */
    double baseline;        /* whole pixels */
    double ink_left;        /* glyph outline extents, box pixels */
    double ink_right;
    bool rtl;               /* paragraph direction */
    bool paragraph_end;
} SrTextLine;

typedef struct {
    SrTextGlyph *glyphs;
    size_t glyph_count;
    size_t glyph_capacity;
    SrTextLine *lines;
    size_t line_count;
    size_t line_capacity;
    double block_height;    /* line count x line advance */
    bool overflow;          /* ink crosses the box edges and is clipped */
} SrTextLayout;

/* True when `tag` has the shape of a BCP 47 language tag:
 * [A-Za-z]{2,8}(-[A-Za-z0-9]{1,8})*, at most 35 characters. Only the shape
 * is checked, not the IANA registry. The bound matters because HarfBuzz
 * interns every language tag it is given (hb_language_from_string) in a
 * process-wide list that is freed only at exit, so each distinct tag
 * outlives the scene that used it. */
bool sr_text_language_valid(const char *tag);

/* Lays out UTF-8 text: paragraphs split at U+000A, each resolved with UAX #9
 * (direction auto = first strong character), wrapped at spaces and after
 * hyphens to style->width (words wider than the box break at cluster
 * boundaries; breaks only fall on cluster starts), shaped per line and per
 * bidi/script run with HarfBuzz (unhinted, 26.6 positions; each line is
 * shaped with context limited to itself and re-checked against the width
 * after shaping), reordered per line (UAX #9 L1 and L2), aligned and
 * vertically placed in the box. Fails with SR_ERR_ARGUMENT for an invalid
 * style (size outside (0, 16384], non-finite letter spacing or line
 * advance, malformed language tag) and SR_ERR_MEMORY, with a message in
 * `err`, when HarfBuzz or FriBidi cannot allocate. */
SrStatus sr_text_layout(SrFont *font, const char *utf8, const SrTextStyle *style,
                        SrTextLayout *out, char *err, size_t errlen);
void sr_text_layout_free(SrTextLayout *layout);

/* Rasterizes a layout into `coverage` (width*height floats, accumulated with
 * a saturating add; the caller clears it). Unhinted FreeType outlines at
 * glyph origins quantized to 1/4 px, 8-bit anti-aliased. Deterministic for
 * identical inputs and library versions. */
SrStatus sr_text_rasterize(SrFont *font, const SrTextLayout *layout,
                           double size, uint32_t width, uint32_t height,
                           float *coverage);

/* Renders a text asset into asset->decoded (premultiplied blend space),
 * opening its font through the scene's font cache. */
SrStatus sr_text_render_asset(SrScene *scene, SrAsset *asset,
                              SrDiagnostics *diag);

#endif
