/* SPDX-License-Identifier: Apache-2.0 */
/* B1-4 loader: shape geometry and stroke-style attributes, vector asset
 * extensions, the paints section, gradients, stops and url(#id) paint
 * references (docs/design/b1-4-shapes-paints.md, docs/xml-reference.md). */
#include "xml_internal.h"
#include "compositing_limits_internal.h"
#include "vector_path_internal.h"
#include "scene_render/paint.h"

#include <ctype.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

const char *const sr_xml_shape_attribute_names[] = {
    "path", "fillRule", "radius", "cornerRadii", "points", "innerRadius",
    "outerRadius", "innerRoundness", "outerRoundness", "strokeCap",
    "strokeJoin", "miterLimit", "dash", "dashOffset", "strokePosition",
    "paintOrder", "trimStart", "trimEnd", "trimOffset", "trimMode"};
const size_t sr_xml_shape_attribute_count =
    sizeof(sr_xml_shape_attribute_names) / sizeof(sr_xml_shape_attribute_names[0]);

static bool fail(ParseContext *ctx, const char *element, const char *attribute,
                 const char *message) {
    sr_xml_fail(ctx, element, attribute, message);
    return false;
}

static bool oom(ParseContext *ctx, const char *element, const char *attribute) {
    ctx->out_of_memory = true;
    return fail(ctx, element, attribute, "out of memory");
}

static bool keyword(ParseContext *ctx, const char *element,
                    const XML_Char **attrs, const char *name,
                    const char *const *names, size_t count, int *value,
                    const char *message) {
    const char *text = sr_xml_attr(attrs, name);
    if (!text) return true;
    for (size_t i = 0; i < count; ++i)
        if (!strcmp(text, names[i])) {
            *value = (int)i;
            return true;
        }
    return fail(ctx, element, name, message);
}

static bool bounded(ParseContext *ctx, const char *element,
                    const XML_Char **attrs, const char *name, double *value,
                    double low, double high, const char *message) {
    if (!sr_xml_parse_double_attr(ctx, element, attrs, name, value)) return false;
    if (sr_xml_attr(attrs, name) && !(*value >= low && *value <= high))
        return fail(ctx, element, name, message);
    return true;
}

/* Whitespace- or comma-separated finite numbers; at most `limit` values. */
static bool number_list(ParseContext *ctx, const char *element,
                        const char *attribute, const char *text, double *out,
                        size_t limit, size_t *count, const char *message) {
    *count = 0;
    const char *cursor = text;
    char token[128];
    while (*cursor) {
        while (*cursor && (isspace((unsigned char)*cursor) || *cursor == ',')) ++cursor;
        if (!*cursor) break;
        const char *start = cursor;
        while (*cursor && !isspace((unsigned char)*cursor) && *cursor != ',') ++cursor;
        size_t length = (size_t)(cursor - start);
        if (length >= sizeof(token) || *count == limit)
            return fail(ctx, element, attribute, message);
        memcpy(token, start, length);
        token[length] = '\0';
        double value;
        if (!sr_parse_double(token, &value) || !(value >= 0.0) ||
            value > SR_MAX_SHAPE_COORDINATE)
            return fail(ctx, element, attribute, message);
        out[(*count)++] = value;
    }
    return true;
}

bool sr_xml_parse_paint_ref(ParseContext *ctx, const char *element,
                            const char *attribute, const char *text,
                            SrPaintRef *ref) {
    size_t length = strlen(text);
    if (length < 7 || strncmp(text, "url(#", 5) || text[length - 1] != ')')
        return fail(ctx, element, attribute, "expected url(#id)");
    size_t id_length = length - 6;
    char *id = sr_alloc(id_length + 1);
    if (!id) return oom(ctx, element, attribute);
    memcpy(id, text + 5, id_length);
    id[id_length] = '\0';
    if (!sr_id_valid(id)) {
        free(id);
        return fail(ctx, element, attribute, "url(#id) needs an XML identifier");
    }
    free(ref->id);
    *ref = (SrPaintRef){id, NULL, sr_xml_line(ctx)};
    return true;
}

static bool is_paint_ref(const char *text) {
    return text && !strncmp(text, "url(", 4);
}

/* A colour or url(#id) attribute; colours keep the legacy diagnostics. */
static bool paint_attribute(ParseContext *ctx, const char *element,
                            const XML_Char **attrs, const char *attribute,
                            SrColor *color, SrPaintRef *ref, bool *is_ref) {
    const char *text = sr_xml_attr(attrs, attribute);
    *is_ref = false;
    if (!text) return true;
    if (is_paint_ref(text)) {
        *is_ref = true;
        return sr_xml_parse_paint_ref(ctx, element, attribute, text, ref);
    }
    if (!sr_xml_parse_color(ctx, element, attribute, text, color))
        return fail(ctx, element, attribute, "invalid color");
    return true;
}

static bool stroke_style(ParseContext *ctx, const char *element,
                         const XML_Char **attrs, SrStrokeStyle *style) {
    static const char *const caps[] = {"butt", "round", "square"};
    static const char *const joins[] = {"miter", "round", "bevel"};
    static const char *const positions[] = {"center", "inside", "outside"};
    static const char *const orders[] = {"fill-stroke", "stroke-fill"};
    int cap = (int)style->cap, join = (int)style->join;
    int position = (int)style->position, order = (int)style->order;
    if (!keyword(ctx, element, attrs, "strokeCap", caps, 3, &cap,
                 "expected butt, round or square") ||
        !keyword(ctx, element, attrs, "strokeJoin", joins, 3, &join,
                 "expected miter, round or bevel") ||
        !keyword(ctx, element, attrs, "strokePosition", positions, 3, &position,
                 "expected center, inside or outside") ||
        !keyword(ctx, element, attrs, "paintOrder", orders, 2, &order,
                 "expected fill-stroke or stroke-fill") ||
        !bounded(ctx, element, attrs, "miterLimit", &style->miter_limit, 1.0,
                 SR_MAX_MITER_LIMIT, "expected a miter limit in [1, 1e6]") ||
        !bounded(ctx, element, attrs, "dashOffset", &style->dash_offset.base,
                 -SR_MAX_SHAPE_COORDINATE, SR_MAX_SHAPE_COORDINATE,
                 "expected a finite dash offset within +-1e9"))
        return false;
    style->cap = (SrLineCap)cap;
    style->join = (SrLineJoin)join;
    style->position = (SrStrokePosition)position;
    style->order = (SrPaintOrder)order;
    const char *dash = sr_xml_attr(attrs, "dash");
    if (dash) {
        double values[SR_MAX_DASH_ENTRIES];
        size_t count;
        if (!number_list(ctx, element, "dash", dash, values, SR_MAX_DASH_ENTRIES,
                         &count, "expected at most 64 non-negative lengths "
                                 "within 1e9"))
            return false;
        /* An odd list repeats once (SVG 2); zero-sum lists are solid. */
        size_t expanded = count % 2 ? 2 * count : count;
        if (expanded) {
            style->dash = sr_alloc(expanded * sizeof(*style->dash));
            if (!style->dash) return oom(ctx, element, "dash");
            for (size_t i = 0; i < expanded; ++i) style->dash[i] = values[i % count];
            style->dash_count = expanded;
        }
    }
    static const char *const names[] = {"strokeCap", "strokeJoin", "miterLimit",
        "dash", "dashOffset", "strokePosition", "paintOrder"};
    for (size_t i = 0; i < sizeof(names) / sizeof(names[0]); ++i)
        if (sr_xml_attr(attrs, names[i])) style->set = true;
    return true;
}

static bool prepared_path(ParseContext *ctx, const char *element,
                          const char *text, SrPreparedPath **out) {
    SrPreparedPath *path = sr_alloc(sizeof(*path));
    if (!path) return oom(ctx, element, "path");
    SrPathParseInfo info;
    SrStatus status = sr_prepared_mask_path_parse(text, SR_MAX_COMPOSITE_BYTES,
                                                  path, &info);
    if (status != SR_OK) {
        free(path);
        if (status == SR_ERR_MEMORY) return oom(ctx, element, "path");
        char message[160];
        snprintf(message, sizeof(message),
                 "invalid or oversized path at byte %zu; supported commands are "
                 "M/L/H/V/C/Q/Z with at most 262144 points", info.byte_offset);
        return fail(ctx, element, "path", message);
    }
    *out = path;
    return true;
}

static bool shape_kind(const char *text, SrShapeType *type) {
    static const struct { const char *name; SrShapeType type; } kinds[] = {
        {"rect", SR_SHAPE_RECT}, {"ellipse", SR_SHAPE_ELLIPSE},
        {"rounded-rect", SR_SHAPE_ROUNDED_RECT}, {"polygon", SR_SHAPE_POLYGON},
        {"star", SR_SHAPE_STAR}, {"line", SR_SHAPE_LINE}, {"path", SR_SHAPE_PATH}};
    for (size_t i = 0; i < sizeof(kinds) / sizeof(kinds[0]); ++i)
        if (!strcmp(text, kinds[i].name)) {
            *type = kinds[i].type;
            return true;
        }
    return false;
}

/* Attribute present with a value other than the schema default. */
static bool authored(const XML_Char **attrs, const char *name, double fallback) {
    const char *text = sr_xml_attr(attrs, name);
    double value;
    return text && !(sr_parse_double(text, &value) && value == fallback);
}

static bool shape_geometry(ParseContext *ctx, const char *name,
                           const XML_Char **attrs, SrNode *node) {
    SrShapeStyle *style = &node->shape_style;
    SrShapeType type = node->shape;
    bool rect = type == SR_SHAPE_RECT || type == SR_SHAPE_ROUNDED_RECT;
    bool polygon = type == SR_SHAPE_POLYGON, star = type == SR_SHAPE_STAR;
    const char *path = sr_xml_attr(attrs, "path");
    if (type == SR_SHAPE_PATH && !path)
        return fail(ctx, name, "path", "path is required for shape=path");
    if (type != SR_SHAPE_PATH && path)
        return fail(ctx, name, "path", "path requires shape=path");
    if (!rect && (authored(attrs, "radius", 0.0) || sr_xml_attr(attrs, "cornerRadii")))
        return fail(ctx, name, sr_xml_attr(attrs, "cornerRadii") ? "cornerRadii" : "radius",
                    "corner radii require shape=rect or shape=rounded-rect");
    if (!polygon && !star && (authored(attrs, "points", 5.0) ||
                              sr_xml_attr(attrs, "outerRadius") ||
                              authored(attrs, "outerRoundness", 0.0)))
        return fail(ctx, name, "points/outerRadius/outerRoundness",
                    "requires shape=polygon or shape=star");
    if (!star && (sr_xml_attr(attrs, "innerRadius") ||
                  authored(attrs, "innerRoundness", 0.0)))
        return fail(ctx, name, "innerRadius/innerRoundness", "requires shape=star");
    if (!bounded(ctx, name, attrs, "radius", &style->radius.base, 0.0,
                 SR_MAX_SHAPE_COORDINATE, "expected a radius in [0, 1e9]") ||
        !bounded(ctx, name, attrs, "innerRadius", &style->inner_radius.base, 0.0,
                 SR_MAX_SHAPE_COORDINATE, "expected a radius in [0, 1e9]") ||
        !bounded(ctx, name, attrs, "outerRadius", &style->outer_radius.base, 0.0,
                 SR_MAX_SHAPE_COORDINATE, "expected a radius in [0, 1e9]") ||
        !bounded(ctx, name, attrs, "innerRoundness", &style->inner_roundness.base,
                 0.0, 1.0, "expected a roundness in [0, 1]") ||
        !bounded(ctx, name, attrs, "outerRoundness", &style->outer_roundness.base,
                 0.0, 1.0, "expected a roundness in [0, 1]"))
        return false;
    style->inner_radius_set = sr_xml_attr(attrs, "innerRadius") != NULL;
    style->outer_radius_set = sr_xml_attr(attrs, "outerRadius") != NULL;
    const char *text = sr_xml_attr(attrs, "points");
    if (text && (!sr_parse_u32(text, &style->points) || style->points < 3 ||
                 style->points > SR_MAX_SHAPE_POINTS))
        return fail(ctx, name, "points", "expected an integer in [3, 4096]");
    if (star && style->inner_radius_set && style->outer_radius_set &&
        style->inner_radius.base > style->outer_radius.base)
        return fail(ctx, name, "innerRadius", "innerRadius must not exceed outerRadius");
    text = sr_xml_attr(attrs, "cornerRadii");
    if (text) {
        double values[4];
        size_t count;
        if (!number_list(ctx, name, "cornerRadii", text, values, 4, &count,
                         "expected one to four non-negative radii within 1e9 "
                         "(TL TR BR BL)"))
            return false;
        if (!count)
            return fail(ctx, name, "cornerRadii", "expected one to four radii");
        /* CSS border-radius shorthand order. */
        static const size_t map[4][4] = {{0, 0, 0, 0}, {0, 1, 0, 1},
                                         {0, 1, 2, 1}, {0, 1, 2, 3}};
        for (int i = 0; i < 4; ++i) style->corner_radii[i] = values[map[count - 1][i]];
        style->corner_radii_set = true;
    }
    if (path && !prepared_path(ctx, name, path, &style->path)) return false;
    if (style->path) {
        double *b = style->path_bounds;
        b[0] = b[1] = INFINITY;
        b[2] = b[3] = -INFINITY;
        for (size_t c = 0; c < style->path->count; ++c)
            for (size_t i = 0; i < style->path->items[c].count; ++i) {
                SrPathPoint q = style->path->items[c].points[i];
                b[0] = fmin(b[0], q.x); b[1] = fmin(b[1], q.y);
                b[2] = fmax(b[2], q.x); b[3] = fmax(b[3], q.y);
            }
    }
    static const char *const rules[] = {"evenodd", "nonzero"};
    int rule = (int)style->fill_rule;
    if (!keyword(ctx, name, attrs, "fillRule", rules, 2, &rule,
                 "expected nonzero or evenodd"))
        return false;
    style->fill_rule = (SrFillRule)rule;
    static const char *const modes[] = {"simultaneous", "sequential"};
    int mode = (int)style->trim_mode;
    if (!keyword(ctx, name, attrs, "trimMode", modes, 2, &mode,
                 "expected simultaneous or sequential") ||
        !bounded(ctx, name, attrs, "trimStart", &style->trim_start.base, 0.0, 1.0,
                 "expected a fraction in [0, 1]") ||
        !bounded(ctx, name, attrs, "trimEnd", &style->trim_end.base, 0.0, 1.0,
                 "expected a fraction in [0, 1]") ||
        !bounded(ctx, name, attrs, "trimOffset", &style->trim_offset.base,
                 -SR_MAX_TRIM_OFFSET, SR_MAX_TRIM_OFFSET,
                 "expected an offset within +-1e6 outline fractions"))
        return false;
    style->trim_mode = (SrTrimMode)mode;
    if (!stroke_style(ctx, name, attrs, &style->stroke)) return false;
    if (type == SR_SHAPE_LINE && style->stroke.position != SR_STROKE_CENTER)
        return fail(ctx, name, "strokePosition",
                    "inside and outside strokes need a closed shape; a line "
                    "accepts only center");
    for (size_t i = 0; i < sr_xml_shape_attribute_count; ++i)
        if (sr_xml_attr(attrs, sr_xml_shape_attribute_names[i])) style->extended = true;
    if (!rect && type != SR_SHAPE_ELLIPSE) style->extended = true;
    if (type == SR_SHAPE_ROUNDED_RECT) style->extended = true;
    return true;
}

bool sr_xml_parse_shape(ParseContext *ctx, const char *name,
                        const XML_Char **attrs, SrNode *node) {
    const char *shape = sr_xml_required(ctx, name, attrs, "shape");
    if (!shape) return false;
    if (!shape_kind(shape, &node->shape))
        return fail(ctx, name, "shape",
                    "expected rect, ellipse, rounded-rect, polygon, star, line or path");
    if (!sr_xml_length_attr(ctx, name, attrs, "width", &node->shape_width,
                            &node->shape_width_unit, true) ||
        !sr_xml_length_attr(ctx, name, attrs, "height", &node->shape_height,
                            &node->shape_height_unit, true) ||
        !sr_xml_parse_double_attr(ctx, name, attrs, "strokeWidth", &node->stroke_width))
        return false;
    bool fill_ref, stroke_ref;
    if (!paint_attribute(ctx, name, attrs, "fill", &node->fill.base,
                         &node->shape_style.fill_paint, &fill_ref) ||
        !paint_attribute(ctx, name, attrs, "stroke", &node->stroke.base,
                         &node->shape_style.stroke_paint, &stroke_ref))
        return false;
    const char *value = sr_xml_attr(attrs, "blend");
    if (value && !sr_blend_parse(value, &node->blend))
        return fail(ctx, name, "blend", "unsupported blend mode");
    if (node->shape_width <= 0.0 || node->shape_height <= 0.0 ||
        node->stroke_width < 0.0)
        return fail(ctx, name, "width/height", "shape dimensions must be positive");
    if (!shape_geometry(ctx, name, attrs, node)) return false;
    if (fill_ref || stroke_ref) node->shape_style.extended = true;
    /* Only the path renderer has this bound; 1.0 shapes keep their range. */
    if (node->shape_style.extended && node->stroke_width > SR_MAX_SHAPE_COORDINATE)
        return fail(ctx, name, "strokeWidth", "expected a stroke width of at most 1e9");
    return true;
}

/* ---- vector assets ------------------------------------------------------ */

static SrVectorExtension *vector_ext(ParseContext *ctx, SrAsset *asset) {
    if (asset->vector_ext) return asset->vector_ext;
    SrVectorExtension *ext = sr_alloc(sizeof(*ext));
    if (!ext) {
        oom(ctx, "vector", NULL);
        return NULL;
    }
    ext->points = 5;
    sr_stroke_style_init(&ext->stroke);
    asset->vector_ext = ext;
    return ext;
}

bool sr_xml_vector_shape_kind(const char *text, SrShapeType *type) {
    return shape_kind(text, type);
}

bool sr_xml_parse_vector_paint(ParseContext *ctx, const XML_Char **attrs,
                               SrAsset *asset, const char *attribute,
                               SrColor *color) {
    const char *text = sr_xml_attr(attrs, attribute);
    if (!text) return true;
    if (!is_paint_ref(text)) {
        if (!sr_xml_parse_color(ctx, "vector", attribute, text, color))
            return fail(ctx, "vector", attribute, "invalid color");
        return true;
    }
    SrVectorExtension *ext = vector_ext(ctx, asset);
    if (!ext) return false;
    ext->extended = true;
    return sr_xml_parse_paint_ref(ctx, "vector", attribute, text,
                                  !strcmp(attribute, "fill") ? &ext->fill_paint
                                                            : &ext->stroke_paint);
}

bool sr_xml_parse_vector_style(ParseContext *ctx, const XML_Char **attrs,
                               SrAsset *asset) {
    static const char *const names[] = {"radius", "points", "innerRadius",
        "strokeCap", "strokeJoin", "miterLimit", "dash", "dashOffset",
        "strokePosition", "paintOrder"};
    bool any = asset->vector_shape != SR_SHAPE_RECT &&
               asset->vector_shape != SR_SHAPE_ELLIPSE &&
               asset->vector_shape != SR_SHAPE_PATH;
    for (size_t i = 0; i < sizeof(names) / sizeof(names[0]); ++i)
        if (sr_xml_attr(attrs, names[i])) any = true;
    if (!any && !asset->vector_ext) return true;
    SrVectorExtension *ext = vector_ext(ctx, asset);
    if (!ext) return false;
    ext->extended = true;
    if (asset->width > SR_MAX_COVERAGE_DIMENSION || asset->height > SR_MAX_COVERAGE_DIMENSION)
        return fail(ctx, "vector", "width/height",
                    "vector assets using 1.1 shapes, stroke styles or paints are "
                    "limited to 16384 px per side");
    if (asset->vector_stroke_width > SR_MAX_SHAPE_COORDINATE)
        return fail(ctx, "vector", "strokeWidth", "expected a stroke width of at most 1e9");
    if (asset->vector_shape == SR_SHAPE_PATH) {
        SrPreparedPath checked = {0};
        SrPathParseInfo info;
        SrStatus status = sr_prepared_mask_path_parse(asset->vector_path,
            SR_MAX_COMPOSITE_BYTES, &checked, &info);
        sr_prepared_path_free(&checked);
        if (status == SR_ERR_MEMORY) return oom(ctx, "vector", "path");
        if (status != SR_OK)
            return fail(ctx, "vector", "path", "path exceeds the bounded path "
                        "limits (262144 points, coordinates within 1e9)");
    }
    SrShapeType type = asset->vector_shape;
    bool polygon = type == SR_SHAPE_POLYGON || type == SR_SHAPE_STAR;
    if (type != SR_SHAPE_ROUNDED_RECT && type != SR_SHAPE_RECT &&
        authored(attrs, "radius", 0.0))
        return fail(ctx, "vector", "radius", "radius requires shape=rounded-rect");
    if (!polygon && authored(attrs, "points", 5.0))
        return fail(ctx, "vector", "points", "points requires shape=polygon or shape=star");
    if (type != SR_SHAPE_STAR && sr_xml_attr(attrs, "innerRadius"))
        return fail(ctx, "vector", "innerRadius", "innerRadius requires shape=star");
    if (!bounded(ctx, "vector", attrs, "radius", &ext->radius, 0.0,
                 SR_MAX_SHAPE_COORDINATE, "expected a radius in [0, 1e9]") ||
        !bounded(ctx, "vector", attrs, "innerRadius", &ext->inner_radius, 0.0,
                 SR_MAX_SHAPE_COORDINATE, "expected a radius in [0, 1e9]"))
        return false;
    ext->inner_radius_set = sr_xml_attr(attrs, "innerRadius") != NULL;
    const char *text = sr_xml_attr(attrs, "points");
    if (text && (!sr_parse_u32(text, &ext->points) || ext->points < 3 ||
                 ext->points > SR_MAX_SHAPE_POINTS))
        return fail(ctx, "vector", "points", "expected an integer in [3, 4096]");
    if (!stroke_style(ctx, "vector", attrs, &ext->stroke)) return false;
    if (type == SR_SHAPE_LINE && ext->stroke.position != SR_STROKE_CENTER)
        return fail(ctx, "vector", "strokePosition",
                    "inside and outside strokes need a closed shape; a line "
                    "accepts only center");
    return true;
}

/* ---- paints ------------------------------------------------------------- */

static const char *paint_element(SrPaintType type) {
    return type == SR_PAINT_LINEAR ? "linearGradient"
         : type == SR_PAINT_RADIAL ? "radialGradient" : "conicGradient";
}

static void start_gradient(ParseContext *ctx, const XML_Char **attrs,
                           SrPaintType type) {
    const char *name = paint_element(type);
    static const char *const common[] = {"id", "spread", "units",
        "interpolationSpace", "dither", "rotation"};
    static const char *const linear[] = {"x1", "y1", "x2", "y2"};
    static const char *const radial[] = {"cx", "cy", "r", "fx", "fy", "fr", "aspect"};
    static const char *const conic[] = {"cx", "cy", "angle"};
    const char *allowed[16];
    size_t count = 0;
    for (size_t i = 0; i < 6; ++i) allowed[count++] = common[i];
    const char *const *extra = type == SR_PAINT_LINEAR ? linear
                             : type == SR_PAINT_RADIAL ? radial : conic;
    size_t extra_count = type == SR_PAINT_LINEAR ? 4 : type == SR_PAINT_RADIAL ? 7 : 3;
    for (size_t i = 0; i < extra_count; ++i) allowed[count++] = extra[i];
    if (!sr_xml_attrs_allowed(ctx, name, attrs, allowed, count)) return;
    const char *id = sr_xml_required(ctx, name, attrs, "id");
    if (!id) return;
    if (!sr_id_valid(id)) SR_XML_FAIL_RETURN(ctx, name, "id", "invalid identifier");
    if (sr_scene_id_exists(ctx->scene, id))
        SR_XML_FAIL_RETURN(ctx, name, "id", "id must be globally unique");
    SrScene *scene = ctx->scene;
    if (scene->paint_count >= SR_MAX_PAINTS)
        SR_XML_FAIL_RETURN(ctx, name, NULL, "paint count exceeds 4096 limit");
    if (scene->paint_count == scene->paint_capacity) {
        size_t capacity = scene->paint_capacity ? scene->paint_capacity * 2 : 4;
        SrPaint *paints = sr_realloc(scene->paints, capacity * sizeof(*paints));
        if (!paints) {
            ctx->out_of_memory = true;
            SR_XML_FAIL_RETURN(ctx, name, NULL, "out of memory");
        }
        memset(paints + scene->paint_capacity, 0,
               (capacity - scene->paint_capacity) * sizeof(*paints));
        scene->paints = paints;
        scene->paint_capacity = capacity;
    }
    SrPaint *paint = &scene->paints[scene->paint_count++];
    sr_paint_init(paint, type);
    paint->source_line = sr_xml_line(ctx);
    paint->id = sr_strdup(id);
    if (!paint->id) {
        ctx->out_of_memory = true;
        SR_XML_FAIL_RETURN(ctx, name, "id", "out of memory");
    }
    static const char *const spreads[] = {"pad", "reflect", "repeat"};
    static const char *const units[] = {"object", "user"};
    static const char *const spaces[] = {"linear", "srgb", "oklab", "oklch"};
    int spread = 0, unit = 0, space = 0;
    const char *text = sr_xml_attr(attrs, "dither");
    if (text && !sr_parse_bool(text, &paint->dither))
        SR_XML_FAIL_RETURN(ctx, name, "dither", "expected true or false");
    const double limit = SR_MAX_PAINT_COORDINATE;
    const char *message = "expected a finite value within +-1e9";
    if (!keyword(ctx, name, attrs, "spread", spreads, 3, &spread,
                 "expected pad, reflect or repeat") ||
        !keyword(ctx, name, attrs, "units", units, 2, &unit,
                 "expected object or user") ||
        !keyword(ctx, name, attrs, "interpolationSpace", spaces, 4, &space,
                 "expected linear, srgb, oklab or oklch") ||
        !bounded(ctx, name, attrs, "rotation", &paint->rotation.base, -limit, limit,
                 message) ||
        !bounded(ctx, name, attrs, "x1", &paint->x1.base, -limit, limit, message) ||
        !bounded(ctx, name, attrs, "y1", &paint->y1.base, -limit, limit, message) ||
        !bounded(ctx, name, attrs, "x2", &paint->x2.base, -limit, limit, message) ||
        !bounded(ctx, name, attrs, "y2", &paint->y2.base, -limit, limit, message) ||
        !bounded(ctx, name, attrs, "cx", &paint->cx.base, -limit, limit, message) ||
        !bounded(ctx, name, attrs, "cy", &paint->cy.base, -limit, limit, message) ||
        !bounded(ctx, name, attrs, "fx", &paint->fx.base, -limit, limit, message) ||
        !bounded(ctx, name, attrs, "fy", &paint->fy.base, -limit, limit, message) ||
        !bounded(ctx, name, attrs, "angle", &paint->angle.base, -limit, limit, message) ||
        !bounded(ctx, name, attrs, "r", &paint->r.base, 1e-9, limit,
                 "expected a positive radius within 1e9") ||
        !bounded(ctx, name, attrs, "fr", &paint->fr.base, 0.0, limit,
                 "expected a non-negative radius within 1e9") ||
        !bounded(ctx, name, attrs, "aspect", &paint->aspect.base, 1e-9, limit,
                 "expected a positive aspect within 1e9"))
        return;
    paint->spread = (SrSpread)spread;
    paint->units = (SrPaintUnits)unit;
    paint->space = (SrInterpolationSpace)space;
    paint->fx_set = sr_xml_attr(attrs, "fx") != NULL;
    paint->fy_set = sr_xml_attr(attrs, "fy") != NULL;
    sr_xml_push(ctx, (ParseFrame){.kind = E_PAINT, .paint = paint,
                                  .curve = SR_CURVE_LINEAR}, name);
}

void sr_xml_start_linear_gradient(ParseContext *ctx, const XML_Char **attrs) {
    start_gradient(ctx, attrs, SR_PAINT_LINEAR);
}

void sr_xml_start_radial_gradient(ParseContext *ctx, const XML_Char **attrs) {
    start_gradient(ctx, attrs, SR_PAINT_RADIAL);
}

void sr_xml_start_conic_gradient(ParseContext *ctx, const XML_Char **attrs) {
    start_gradient(ctx, attrs, SR_PAINT_CONIC);
}

void sr_xml_start_stop(ParseContext *ctx, const XML_Char **attrs) {
    const char *const allowed[] = {"offset", "color", "opacity", "midpoint"};
    if (!sr_xml_attrs_allowed(ctx, "stop", attrs, allowed, 4)) return;
    ParseFrame *parent = sr_xml_parent(ctx);
    const char *offset = sr_xml_required(ctx, "stop", attrs, "offset");
    const char *color = sr_xml_required(ctx, "stop", attrs, "color");
    if (!parent || !parent->paint || !offset || !color) return;
    SrPaint *paint = parent->paint;
    if (paint->stop_count >= SR_MAX_GRADIENT_STOPS)
        SR_XML_FAIL_RETURN(ctx, "stop", NULL, "gradient stop count exceeds 256 limit");
    if (paint->stop_count == paint->stop_capacity) {
        size_t capacity = paint->stop_capacity ? paint->stop_capacity * 2 : 4;
        SrGradientStop *stops = sr_realloc(paint->stops, capacity * sizeof(*stops));
        if (!stops) {
            ctx->out_of_memory = true;
            SR_XML_FAIL_RETURN(ctx, "stop", NULL, "out of memory");
        }
        memset(stops + paint->stop_capacity, 0,
               (capacity - paint->stop_capacity) * sizeof(*stops));
        paint->stops = stops;
        paint->stop_capacity = capacity;
    }
    SrGradientStop *stop = &paint->stops[paint->stop_count++];
    *stop = (SrGradientStop){0};
    stop->source_line = sr_xml_line(ctx);
    stop->opacity.base = 1.0;
    stop->midpoint.base = 0.5;
    stop->color.space = ctx->scene->project.working_color_space;
    if (!sr_xml_parse_color(ctx, "stop", "color", color, &stop->color.base))
        SR_XML_FAIL_RETURN(ctx, "stop", "color", "invalid color");
    if (!bounded(ctx, "stop", attrs, "offset", &stop->offset.base, 0.0, 1.0,
                 "expected an offset in [0, 1]") ||
        !bounded(ctx, "stop", attrs, "opacity", &stop->opacity.base, 0.0, 1.0,
                 "expected an opacity in [0, 1]") ||
        !bounded(ctx, "stop", attrs, "midpoint", &stop->midpoint.base, 0.0, 1.0,
                 "expected a midpoint in [0, 1]"))
        return;
    sr_xml_push(ctx, (ParseFrame){.kind = E_STOP, .paint = paint, .stop = stop,
                                  .curve = SR_CURVE_LINEAR}, "stop");
}

bool sr_xml_parse_background(ParseContext *ctx, const char *text) {
    if (is_paint_ref(text))
        return sr_xml_parse_paint_ref(ctx, "project", "background", text,
                                      &ctx->scene->background_paint);
    if (!sr_xml_parse_color(ctx, "project", "background", text,
                            &ctx->scene->project.background))
        return fail(ctx, "project", "background",
                    "expected #RRGGBB, #RRGGBBAA, or r,g,b,a");
    return true;
}

/* ---- resolution --------------------------------------------------------- */

static SrPaint *find_paint(SrScene *scene, const char *id) {
    for (size_t i = 0; i < scene->paint_count; ++i)
        if (!strcmp(scene->paints[i].id, id)) return &scene->paints[i];
    return NULL;
}

static bool resolve_ref(ParseContext *ctx, SrPaintRef *ref, const char *element,
                        const char *attribute) {
    if (!ref->id) return true;
    ref->paint = find_paint(ctx->scene, ref->id);
    if (ref->paint) return true;
    bool other = sr_scene_id_exists(ctx->scene, ref->id);
    sr_diag_error(ctx->diag, ref->source_line, element, attribute,
                  other ? "'%s' is not a gradient paint" : "unknown paint id '%s'",
                  ref->id);
    return false;
}

/* Called from the recursive node resolution, after structural preflight. */
bool sr_xml_resolve_shape_node(ParseContext *ctx, SrNode *node) {
    if (node->type == SR_NODE_SHAPE) {
        SrShapeStyle *style = &node->shape_style;
        SrShapeType type = node->shape;
        const char *misplaced = NULL;
        if (style->radius.track.count && type != SR_SHAPE_RECT &&
            type != SR_SHAPE_ROUNDED_RECT)
            misplaced = "radius";
        else if ((style->inner_radius.track.count ||
                  style->inner_roundness.track.count) && type != SR_SHAPE_STAR)
            misplaced = "innerRadius/innerRoundness";
        else if ((style->outer_radius.track.count ||
                  style->outer_roundness.track.count) &&
                 type != SR_SHAPE_POLYGON && type != SR_SHAPE_STAR)
            misplaced = "outerRadius/outerRoundness";
        if (misplaced) {
            sr_diag_error(ctx->diag, node->source_line, "animate", misplaced,
                          "animated property does not apply to shape '%s'", node->id);
            return false;
        }
        if (style->inner_radius.track.count) style->inner_radius_set = true;
        if (style->outer_radius.track.count) style->outer_radius_set = true;
        if (!resolve_ref(ctx, &style->fill_paint, "shape", "fill") ||
            !resolve_ref(ctx, &style->stroke_paint, "shape", "stroke"))
            return false;
        if ((style->fill_paint.id && node->fill.r.count) ||
            (style->stroke_paint.id && node->stroke.r.count)) {
            sr_diag_error(ctx->diag, node->source_line, "shape",
                          style->fill_paint.id && node->fill.r.count ? "fill" : "stroke",
                          "a url(#id) paint cannot also have a colour animation; "
                          "animate the gradient instead");
            return false;
        }
        if (style->extended && !ctx->scene->compositing_required) {
            sr_diag_error(ctx->diag, node->source_line, "shape", NULL,
                          "B1-4 shape requires compositing preparation");
            return false;
        }
    }
    return true;
}

bool sr_xml_resolve_shapes(ParseContext *ctx) {
    SrScene *scene = ctx->scene;
    for (size_t i = 0; i < scene->paint_count; ++i) {
        SrPaint *paint = &scene->paints[i];
        if (!paint->stop_count) {
            sr_diag_error(ctx->diag, paint->source_line, paint_element(paint->type),
                          NULL, "gradient '%s' needs at least one stop", paint->id);
            return false;
        }
        if (paint->fx.track.count) paint->fx_set = true;
        if (paint->fy.track.count) paint->fy_set = true;
        paint->dither_key = sr_paint_dither_key(paint->id, scene->project.seed);
    }
    if (!resolve_ref(ctx, &scene->background_paint, "project", "background"))
        return false;
    for (size_t i = 0; i < scene->asset_count; ++i) {
        SrAsset *asset = &scene->assets[i];
        SrVectorExtension *ext = asset->type == SR_ASSET_VECTOR ? asset->vector_ext : NULL;
        if (!ext) continue;
        if (!resolve_ref(ctx, &ext->fill_paint, "vector", "fill") ||
            !resolve_ref(ctx, &ext->stroke_paint, "vector", "stroke"))
            return false;
        const SrPaintRef *refs[2] = {&ext->fill_paint, &ext->stroke_paint};
        for (int k = 0; k < 2; ++k)
            if (refs[k]->paint && sr_paint_has_animation(refs[k]->paint)) {
                sr_diag_error(ctx->diag, asset->source_line, "vector",
                              k ? "stroke" : "fill",
                              "vector assets are rasterized once; paint '%s' is animated",
                              refs[k]->id);
                return false;
            }
    }
    return true;
}
