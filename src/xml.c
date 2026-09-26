#include "scene_render/xml.h"

#include "xml_internal.h"
#include "xml_schema.h"

#include <ctype.h>
#include <errno.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

size_t sr_xml_line(ParseContext *ctx) {
    return (size_t)XML_GetCurrentLineNumber(ctx->parser);
}

void sr_xml_fail(ParseContext *ctx, const char *element, const char *attribute,
                 const char *message) {
    sr_xml_fail_at(ctx, sr_xml_line(ctx), element, attribute, message);
}

void sr_xml_fail_at(ParseContext *ctx, size_t line, const char *element,
                    const char *attribute, const char *message) {
    if (!ctx->failed) {
        sr_diag_error(ctx->diag, line, element, attribute, "%s", message);
        ctx->failed = true;
        XML_StopParser(ctx->parser, XML_FALSE);
    }
}

const char *sr_xml_attr(const XML_Char **attrs, const char *name) {
    for (size_t i = 0; attrs && attrs[i]; i += 2)
        if (strcmp(attrs[i], name) == 0) return attrs[i + 1];
    return NULL;
}

static bool name_in(const char *name, const char *const *allowed, size_t count) {
    for (size_t i = 0; i < count; ++i)
        if (strcmp(name, allowed[i]) == 0) return true;
    return false;
}

bool sr_xml_attrs_allowed(ParseContext *ctx, const char *element,
                          const XML_Char **attrs, const char *const *allowed,
                          size_t count) {
    for (size_t i = 0; attrs && attrs[i]; i += 2) {
        if (!name_in(attrs[i], allowed, count)) {
            char message[192];
            snprintf(message, sizeof(message), "unknown attribute '%s'", attrs[i]);
            sr_xml_fail(ctx, element, attrs[i], message);
            return false;
        }
    }
    return true;
}

const char *sr_xml_required(ParseContext *ctx, const char *element,
                            const XML_Char **attrs, const char *name) {
    const char *value = sr_xml_attr(attrs, name);
    if (!value || !*value) {
        char message[160];
        snprintf(message, sizeof(message), "required attribute '%s' is missing", name);
        sr_xml_fail(ctx, element, name, message);
        return NULL;
    }
    return value;
}

ParseFrame *sr_xml_parent(ParseContext *ctx) {
    return ctx->depth ? &ctx->stack[ctx->depth - 1] : NULL;
}

void sr_xml_push(ParseContext *ctx, ParseFrame frame, const char *element) {
    if (ctx->depth == ctx->stack_capacity) {
        size_t capacity = ctx->stack_capacity ? ctx->stack_capacity * 2 : 16;
        if (capacity < ctx->stack_capacity ||
            capacity > SIZE_MAX / sizeof(*ctx->stack)) {
            ctx->out_of_memory = true;
            sr_xml_fail(ctx, element, NULL, "XML nesting exceeds available memory");
            return;
        }
        ParseFrame *stack = sr_realloc(ctx->stack,
                                       capacity * sizeof(*stack));
        if (!stack) {
            ctx->out_of_memory = true;
            sr_xml_fail(ctx, element, NULL, "out of memory while nesting XML");
            return;
        }
        ctx->stack = stack;
        ctx->stack_capacity = capacity;
    }
    ctx->stack[ctx->depth++] = frame;
}

typedef void (*StartHandler)(ParseContext *, const XML_Char **);

typedef struct {
    const char *name;
    uint64_t parents;
    StartHandler handler;
    ElementKind kind;
    size_t seen_offset;
    unsigned minimum_version;
    bool implemented;
    bool push;
} ElementDispatch;

#define P(kind) (UINT64_C(1) << (kind))
#define CONTAINERS (P(E_GROUP) | P(E_COMPOSITION))
#define NODES (P(E_GROUP) | P(E_LAYER) | P(E_PARTICLES))
#define ANIM_HOSTS (NODES | P(E_CAMERA) | P(E_MASK) | P(E_LIGHT) | \
                    P(E_EFFECT) | P(E_MODIFIER) | P(E_OBJECT3D) | \
                    P(E_FORCE_FIELD) | P(E_POINT) | P(E_MATERIAL) | P(E_AUDIO_TRACK))
#define ENTRY(name, parents, handler, kind, push) \
    {name, parents, handler, kind, 0, 10, true, push}
#define SECTION(name, handler, kind, seen) \
    {name, P(E_SCENE), handler, kind, offsetof(ParseContext, seen), \
     10, true, true}

static void start_group(ParseContext *ctx, const XML_Char **attrs) {
    sr_xml_start_node(ctx, "group", attrs, SR_NODE_GROUP);
}

static void start_layer(ParseContext *ctx, const XML_Char **attrs) {
    sr_xml_start_node(ctx, "layer", attrs, SR_NODE_MEDIA);
}

static void start_shape(ParseContext *ctx, const XML_Char **attrs) {
    sr_xml_start_node(ctx, "shape", attrs, SR_NODE_SHAPE);
}

static void start_particles(ParseContext *ctx, const XML_Char **attrs) {
    sr_xml_start_node(ctx, "particleEmitter", attrs, SR_NODE_PARTICLES);
}

/* Handlers that construct an animation host push their own frame. Plain
 * sections and asset declarations are pushed here after their handler. */
static const ElementDispatch dispatch[] = {
    SECTION("project", sr_xml_start_project, E_PROJECT, seen_project),
    SECTION("output", sr_xml_start_output, E_OUTPUT, seen_output),
    SECTION("assets", NULL, E_ASSETS, seen_assets),
    ENTRY("image", P(E_ASSETS), sr_xml_start_image, E_IMAGE, true),
    ENTRY("video", P(E_ASSETS), sr_xml_start_video, E_VIDEO, true),
    ENTRY("audio", P(E_ASSETS), sr_xml_start_audio, E_AUDIO, true),
    ENTRY("text", P(E_ASSETS), sr_xml_start_text, E_TEXT, true),
    ENTRY("vector", P(E_ASSETS), sr_xml_start_vector, E_VECTOR, true),
    ENTRY("mesh", P(E_ASSETS), sr_xml_start_mesh, E_MESH, true),
    SECTION("materials", NULL, E_MATERIALS, seen_materials),
    ENTRY("material", P(E_MATERIALS), sr_xml_start_material, E_MATERIAL, false),
    SECTION("composition", NULL, E_COMPOSITION, seen_composition),
    SECTION("scene360", sr_xml_start_scene360, E_SCENE360, seen_scene360),
    ENTRY("group", CONTAINERS, start_group, E_GROUP, false),
    ENTRY("layer", CONTAINERS, start_layer, E_LAYER, false),
    ENTRY("shape", CONTAINERS, start_shape, E_LAYER, false),
    ENTRY("particleEmitter", CONTAINERS, start_particles, E_PARTICLES, false),
    ENTRY("object3D", CONTAINERS, sr_xml_start_object3d, E_OBJECT3D, false),
    ENTRY("camera", CONTAINERS, sr_xml_start_camera, E_CAMERA, false),
    ENTRY("mask", NODES, sr_xml_start_mask, E_MASK, false),
    ENTRY("rigidBody", P(E_LAYER) | P(E_PARTICLES), sr_xml_start_rigid_body,
          E_RIGID_BODY, false),
    ENTRY("softBody", P(E_LAYER) | P(E_PARTICLES), sr_xml_start_soft_body,
          E_SOFT_BODY, false),
    ENTRY("deform", P(E_LAYER) | P(E_PARTICLES), sr_xml_start_deform,
          E_DEFORM, false),
    ENTRY("modifier", P(E_DEFORM), sr_xml_start_modifier, E_MODIFIER, false),
    ENTRY("point", P(E_MODIFIER), sr_xml_start_point, E_POINT, false),
    ENTRY("animate", ANIM_HOSTS, sr_xml_start_animate, E_ANIMATE, false),
    ENTRY("key", P(E_ANIMATE), sr_xml_start_key, E_KEY, false),
    SECTION("audioMix", sr_xml_start_audio_mix, E_AUDIO_MIX, seen_audio_mix),
    ENTRY("audioTrack", P(E_AUDIO_MIX), sr_xml_start_audio_track,
          E_AUDIO_TRACK, false),
    SECTION("lights", NULL, E_LIGHTS, seen_lights),
    ENTRY("light", P(E_LIGHTS), sr_xml_start_light, E_LIGHT, false),
    SECTION("effects", NULL, E_EFFECTS, seen_effects),
    ENTRY("effect", P(E_EFFECTS), sr_xml_start_effect, E_EFFECT, false),
    SECTION("physics", sr_xml_start_physics, E_PHYSICS, seen_physics),
    ENTRY("forceField", P(E_PHYSICS), sr_xml_start_force_field,
          E_FORCE_FIELD, false),
    ENTRY("constraint", P(E_PHYSICS), sr_xml_start_constraint,
          E_CONSTRAINT, true)
};

#undef SECTION
#undef ENTRY
#undef ANIM_HOSTS
#undef NODES
#undef CONTAINERS

static void XMLCALL on_start(void *user, const XML_Char *name,
                             const XML_Char **attrs) {
    ParseContext *ctx = user;
    ++ctx->element_depth;
    if (ctx->failed) return;
    if (ctx->element_depth > SR_XML_MAX_DEPTH) {
        char message[96];
        snprintf(message, sizeof(message), "element nesting exceeds %d levels",
                 SR_XML_MAX_DEPTH);
        SR_XML_FAIL_RETURN(ctx, name, NULL, message);
    }
    ParseFrame *parent = sr_xml_parent(ctx);
    if (!parent) {
        const char *const allowed[] = {"version"};
        if (strcmp(name, "scene") != 0)
            SR_XML_FAIL_RETURN(ctx, name, NULL, "document root must be <scene>");
        if (!sr_xml_attrs_allowed(ctx, name, attrs, allowed, 1)) return;
        const char *version = sr_xml_required(ctx, name, attrs, "version");
        if (!version || (strcmp(version, "1.0") && strcmp(version, "1.1")))
            SR_XML_FAIL_RETURN(ctx, name, "version", "expected version 1.0 or 1.1");
        ctx->scene->format_version = !strcmp(version, "1.1") ? 11 : 10;
        sr_xml_push(ctx, (ParseFrame){.kind = E_SCENE,
                    .node = ctx->scene->root, .curve = SR_CURVE_LINEAR}, name);
        return;
    }
    for (size_t i = 0; i < sizeof(dispatch) / sizeof(dispatch[0]); ++i) {
        const ElementDispatch *entry = &dispatch[i];
        if (!(entry->parents & P(parent->kind)) || strcmp(name, entry->name))
            continue;
        if (!entry->implemented)
            SR_XML_FAIL_RETURN(ctx, name, NULL, "unsupported in this build");
        if (entry->minimum_version > ctx->scene->format_version)
            SR_XML_FAIL_RETURN(ctx, name, NULL, "requires version=\"1.1\"");
        bool *seen = entry->seen_offset
            ? (bool *)((char *)ctx + entry->seen_offset) : NULL;
        if (seen && *seen) break;
        if (entry->handler) {
            entry->handler(ctx, attrs);
        } else if (attrs && attrs[0]) {
            SR_XML_FAIL_RETURN(ctx, name, attrs[0], "section has no attributes");
        }
        if (ctx->failed) return;
        if (seen) *seen = true;
        if (entry->push) {
            ParseFrame frame = {.kind = entry->kind};
            if (entry->kind == E_COMPOSITION) frame.node = ctx->scene->root;
            sr_xml_push(ctx, frame, name);
        }
        return;
    }
    sr_xml_fail(ctx, name, NULL,
                "element is not valid in this context or supported by scene v1");
}

#undef P

static void XMLCALL on_end(void *user, const XML_Char *name) {
    ParseContext *ctx = user;
    (void)name;
    if (ctx->element_depth) --ctx->element_depth;
    if (ctx->failed || ctx->depth == 0) return;
    ParseFrame *frame = &ctx->stack[ctx->depth - 1];
    if (frame->kind == E_ANIMATE && !sr_xml_finish_animation(ctx, frame)) return;
    --ctx->depth;
}

static void XMLCALL on_text(void *user, const XML_Char *text, int length) {
    ParseContext *ctx = user;
    for (int i = 0; !ctx->failed && i < length; ++i)
        if (!isspace((unsigned char)text[i]))
            SR_XML_FAIL_RETURN(ctx, "text", NULL,
                               "text nodes are not allowed; use attributes");
}

static void XMLCALL on_doctype(void *user, const XML_Char *name,
                               const XML_Char *system_id,
                               const XML_Char *public_id,
                               int has_internal_subset) {
    ParseContext *ctx = user;
    (void)name;
    (void)system_id;
    (void)public_id;
    (void)has_internal_subset;
    sr_xml_fail(ctx, "DOCTYPE", NULL, "document type declarations are not allowed");
}

/* Reads the whole file (at most SR_XML_MAX_BYTES) into *data. */
static SrStatus read_scene_file(const char *path, char **data, size_t *size,
                                SrDiagnostics *diag) {
    *data = NULL;
    *size = 0;
    FILE *file = fopen(path, "rb");
    if (!file) {
        sr_diag_error(diag, 0, NULL, NULL, "cannot open XML scene: %s",
                      strerror(errno));
        return SR_ERR_IO;
    }
    size_t capacity = 0, used = 0;
    char *buffer = NULL;
    SrStatus status = SR_OK;
    for (;;) {
        if (used == capacity) {
            if (capacity > SR_XML_MAX_BYTES) break;  /* over the limit */
            size_t grown = capacity ? capacity * 2 : 65536;
            if (grown > SR_XML_MAX_BYTES + 1) grown = SR_XML_MAX_BYTES + 1;
            char *next = sr_realloc(buffer, grown);
            if (!next) {
                status = SR_ERR_MEMORY;
                break;
            }
            buffer = next;
            capacity = grown;
        }
        size_t want = capacity - used;
        size_t count = fread(buffer + used, 1, want, file);
        used += count;
        if (count < want) {
            if (ferror(file)) {
                sr_diag_error(diag, 0, NULL, NULL, "I/O error while reading XML");
                status = SR_ERR_IO;
            }
            break;
        }
    }
    if (status == SR_OK && used > SR_XML_MAX_BYTES) {
        sr_diag_error(diag, 0, NULL, NULL, "scene file too large (limit %zu MiB)",
                      SR_XML_MAX_BYTES >> 20);
        status = SR_ERR_XML;
    }
    fclose(file);
    if (status != SR_OK) {
        free(buffer);
        return status;
    }
    *data = buffer;
    *size = used;
    return SR_OK;
}

static bool starts_with(const char *at, const char *end, const char *prefix) {
    size_t length = strlen(prefix);
    return (size_t)(end - at) >= length && memcmp(at, prefix, length) == 0;
}

/* Scans the prolog (BOM, XML declaration, comments, processing
 * instructions, whitespace) up to the root element; any markup declaration
 * there ("<!DOCTYPE", or any other "<!" that is not a comment) is rejected
 * before a parser sees the document. Both parsers decode the bytes as
 * UTF-8, so a byte scan cannot be fooled by another encoding. Returns the
 * line of the declaration, or 0 when there is none. */
static size_t find_declaration(const char *data, size_t size) {
    const char *at = data, *end = data + size;
    size_t line = 1;
    if (starts_with(at, end, "\xEF\xBB\xBF")) at += 3;
    while (at < end) {
        if (*at == '\n') {
            ++line;
            ++at;
        } else if (*at == ' ' || *at == '\t' || *at == '\r') {
            ++at;
        } else if (starts_with(at, end, "<!--")) {
            for (at += 4; at < end && !starts_with(at, end, "-->"); ++at)
                if (*at == '\n') ++line;
            at = at < end ? at + 3 : end;
        } else if (starts_with(at, end, "<?")) {
            for (at += 2; at < end && !starts_with(at, end, "?>"); ++at)
                if (*at == '\n') ++line;
            at = at < end ? at + 2 : end;
        } else if (starts_with(at, end, "<!")) {
            return line;
        } else {
            break;  /* root element or not well-formed: the parsers decide */
        }
    }
    return 0;
}

SrStatus sr_scene_load_xml(const char *path, SrScene *scene,
                           SrDiagnostics *diag) {
    return sr_scene_load_xml_report(path, scene, diag, false);
}

SrStatus sr_scene_load_xml_report(const char *path, SrScene *scene,
                                 SrDiagnostics *diag, bool report_unsupported) {
    if (!path || !scene || !diag) return SR_ERR_ARGUMENT;
    sr_scene_init(scene);
    scene->source_path = sr_strdup(path);
    scene->base_dir = sr_path_dirname(path);
    if (!scene->source_path || !scene->base_dir || !scene->root ||
        !scene->output.path || !scene->output.pixel_format ||
        !scene->output.preset || !scene->output.audio_codec) {
        sr_scene_free(scene);
        return SR_ERR_MEMORY;
    }
    /* Read once: both parsers and the resume fingerprint see these bytes. */
    char *data = NULL;
    size_t size = 0;
    SrStatus status = read_scene_file(path, &data, &size, diag);
    if (status != SR_OK) {
        sr_scene_free(scene);
        return status;
    }
    scene->source_hash = sr_fnv1a64(SR_FNV_OFFSET, data, size);
    size_t declaration = find_declaration(data, size);
    if (declaration) {
        sr_diag_error(diag, declaration, "DOCTYPE", NULL,
                      "document type declarations are not allowed");
        free(data);
        sr_scene_free(scene);
        return SR_ERR_XML;
    }
    SrSchemaDeferral deferral;
    status = sr_xml_schema_check_profile(data, size, path, diag, &deferral,
                                         report_unsupported);
    if (status != SR_OK) {
        free(data);
        sr_scene_free(scene);
        return status;
    }
    XML_Parser parser = XML_ParserCreate("UTF-8");
    if (!parser) {
        free(data);
        sr_scene_free(scene);
        return SR_ERR_MEMORY;
    }
    ParseContext ctx = {.parser = parser, .scene = scene, .diag = diag};
    XML_SetUserData(parser, &ctx);
    XML_SetElementHandler(parser, on_start, on_end);
    XML_SetCharacterDataHandler(parser, on_text);
    XML_SetStartDoctypeDeclHandler(parser, on_doctype);
    /* Fed in chunks: XML_Parse takes an int length. */
    const size_t chunk = 1u << 20;
    for (size_t offset = 0; !ctx.failed;) {
        size_t count = size - offset < chunk ? size - offset : chunk;
        bool last = offset + count == size;
        if (XML_Parse(parser, data + offset, (int)count, last) == XML_STATUS_ERROR &&
            !ctx.failed) {
            enum XML_Error code = XML_GetErrorCode(parser);
            if (code == XML_ERROR_NO_MEMORY) {
                XML_ParserFree(parser);
                free(ctx.stack);
                free(data);
                sr_scene_free(scene);
                return SR_ERR_MEMORY;
            }
            sr_diag_error(diag, (size_t)XML_GetCurrentLineNumber(parser), NULL,
                          NULL, "XML syntax: %s", XML_ErrorString(code));
            ctx.failed = true;
        }
        offset += count;
        if (last) break;
    }
    free(data);
    XML_ParserFree(parser);
    if (!ctx.failed && deferral.deferred) {
        sr_diag_error(diag, deferral.line, NULL, NULL, "%s", deferral.message);
        ctx.failed = true;
    }
    if (!ctx.failed && (!ctx.seen_project || !ctx.seen_composition)) {
        sr_diag_error(diag, 1, "scene", NULL,
                      "scene requires exactly one project and one composition");
        ctx.failed = true;
    }
    if (!ctx.failed && !sr_xml_resolve_scene(&ctx)) ctx.failed = true;
    free(ctx.stack);
    if (ctx.failed) {
        sr_scene_free(scene);
        return ctx.out_of_memory ? SR_ERR_MEMORY : SR_ERR_XML;
    }
    return SR_OK;
}
