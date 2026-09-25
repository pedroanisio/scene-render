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
    if (!ctx->failed) {
        sr_diag_error(ctx->diag, sr_xml_line(ctx), element, attribute, "%s", message);
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
    ParseFrame *p = sr_xml_parent(ctx);
    if (!p) {
        const char *const allowed[] = {"version"};
        if (strcmp(name, "scene") != 0)
            SR_XML_FAIL_RETURN(ctx, name, NULL, "document root must be <scene>");
        if (!sr_xml_attrs_allowed(ctx, name, attrs, allowed, 1)) return;
        const char *version = sr_xml_required(ctx, name, attrs, "version");
        if (!version || strcmp(version, "1.0") != 0)
            SR_XML_FAIL_RETURN(ctx, name, "version",
                               "this build accepts scene version 1.0");
        sr_xml_push(ctx, (ParseFrame){.kind = E_SCENE,
                                      .node = ctx->scene->root,
                                      .curve = SR_CURVE_LINEAR}, name);
        return;
    }
    if (strcmp(name, "project") == 0 && p->kind == E_SCENE &&
        !ctx->seen_project) {
        sr_xml_start_project(ctx, attrs);
        if (!ctx->failed) sr_xml_push(ctx, (ParseFrame){.kind = E_PROJECT}, name);
        return;
    }
    if (strcmp(name, "output") == 0 && p->kind == E_SCENE &&
        !ctx->seen_output) {
        sr_xml_start_output(ctx, attrs);
        if (!ctx->failed) sr_xml_push(ctx, (ParseFrame){.kind = E_OUTPUT}, name);
        return;
    }
    if (strcmp(name, "assets") == 0 && p->kind == E_SCENE &&
        !ctx->seen_assets) {
        if (attrs && attrs[0])
            SR_XML_FAIL_RETURN(ctx, name, attrs[0], "assets has no attributes");
        ctx->seen_assets = true;
        sr_xml_push(ctx, (ParseFrame){.kind = E_ASSETS}, name);
        return;
    }
    if (strcmp(name, "image") == 0 && p->kind == E_ASSETS) {
        sr_xml_start_image(ctx, attrs);
        if (!ctx->failed) sr_xml_push(ctx, (ParseFrame){.kind = E_IMAGE}, name);
        return;
    }
    if (strcmp(name, "video") == 0 && p->kind == E_ASSETS) {
        sr_xml_start_video(ctx, attrs);
        if (!ctx->failed) sr_xml_push(ctx, (ParseFrame){.kind = E_VIDEO}, name);
        return;
    }
    if (strcmp(name, "audio") == 0 && p->kind == E_ASSETS) {
        sr_xml_start_audio(ctx, attrs);
        if (!ctx->failed) sr_xml_push(ctx, (ParseFrame){.kind = E_AUDIO}, name);
        return;
    }
    if (strcmp(name, "text") == 0 && p->kind == E_ASSETS) {
        sr_xml_start_text(ctx, attrs);
        if (!ctx->failed) sr_xml_push(ctx, (ParseFrame){.kind = E_TEXT}, name);
        return;
    }
    if (strcmp(name, "vector") == 0 && p->kind == E_ASSETS) {
        sr_xml_start_vector(ctx, attrs);
        if (!ctx->failed) sr_xml_push(ctx, (ParseFrame){.kind = E_VECTOR}, name);
        return;
    }
    if (strcmp(name, "mesh") == 0 && p->kind == E_ASSETS) {
        sr_xml_start_mesh(ctx, attrs);
        if (!ctx->failed) sr_xml_push(ctx, (ParseFrame){.kind = E_MESH}, name);
        return;
    }
    if (strcmp(name, "materials") == 0 && p->kind == E_SCENE &&
        !ctx->seen_materials) {
        if (attrs && attrs[0])
            SR_XML_FAIL_RETURN(ctx, name, attrs[0], "materials has no attributes");
        ctx->seen_materials = true;
        sr_xml_push(ctx, (ParseFrame){.kind = E_MATERIALS}, name);
        return;
    }
    if (strcmp(name, "material") == 0 && p->kind == E_MATERIALS) {
        sr_xml_start_material(ctx, attrs);
        if (!ctx->failed) sr_xml_push(ctx, (ParseFrame){.kind = E_MATERIAL}, name);
        return;
    }
    if (strcmp(name, "composition") == 0 && p->kind == E_SCENE &&
        !ctx->seen_composition) {
        if (attrs && attrs[0])
            SR_XML_FAIL_RETURN(ctx, name, attrs[0],
                               "composition has no attributes");
        ctx->seen_composition = true;
        sr_xml_push(ctx, (ParseFrame){.kind = E_COMPOSITION,
                                      .node = ctx->scene->root}, name);
        return;
    }
    if (strcmp(name, "scene360") == 0 && p->kind == E_SCENE &&
        !ctx->seen_scene360) {
        sr_xml_start_scene360(ctx, attrs);
        if (!ctx->failed) sr_xml_push(ctx, (ParseFrame){.kind = E_SCENE360}, name);
        return;
    }
    if (strcmp(name, "group") == 0 &&
        (p->kind == E_GROUP || p->kind == E_COMPOSITION)) {
        sr_xml_start_node(ctx, name, attrs, SR_NODE_GROUP);
        return;
    }
    if (strcmp(name, "layer") == 0 &&
        (p->kind == E_GROUP || p->kind == E_COMPOSITION)) {
        sr_xml_start_node(ctx, name, attrs, SR_NODE_MEDIA);
        return;
    }
    if (strcmp(name, "shape") == 0 &&
        (p->kind == E_GROUP || p->kind == E_COMPOSITION)) {
        sr_xml_start_node(ctx, name, attrs, SR_NODE_SHAPE);
        return;
    }
    if (strcmp(name, "particleEmitter") == 0 &&
        (p->kind == E_GROUP || p->kind == E_COMPOSITION)) {
        sr_xml_start_node(ctx, name, attrs, SR_NODE_PARTICLES);
        return;
    }
    if (strcmp(name, "object3D") == 0 &&
        (p->kind == E_GROUP || p->kind == E_COMPOSITION)) {
        sr_xml_start_object3d(ctx, attrs);
        return;
    }
    if (strcmp(name, "camera") == 0 &&
        (p->kind == E_GROUP || p->kind == E_COMPOSITION)) {
        sr_xml_start_camera(ctx, attrs);
        return;
    }
    if (strcmp(name, "mask") == 0 &&
        (p->kind == E_GROUP || p->kind == E_LAYER)) {
        sr_xml_start_mask(ctx, attrs);
        return;
    }
    if (strcmp(name, "rigidBody") == 0 && p->kind == E_LAYER) {
        sr_xml_start_rigid_body(ctx, attrs);
        return;
    }
    if (strcmp(name, "softBody") == 0 && p->kind == E_LAYER) {
        sr_xml_start_soft_body(ctx, attrs);
        return;
    }
    if (strcmp(name, "deform") == 0 && p->kind == E_LAYER) {
        sr_xml_start_deform(ctx, attrs);
        return;
    }
    if (strcmp(name, "modifier") == 0 && p->kind == E_DEFORM) {
        sr_xml_start_modifier(ctx, attrs);
        return;
    }
    if (strcmp(name, "point") == 0 && p->kind == E_MODIFIER) {
        sr_xml_start_point(ctx, attrs);
        return;
    }
    if (strcmp(name, "animate") == 0 &&
        (p->kind == E_GROUP || p->kind == E_LAYER || p->kind == E_PARTICLES ||
         p->kind == E_CAMERA || p->kind == E_MASK ||
         p->kind == E_LIGHT || p->kind == E_EFFECT || p->kind == E_MODIFIER ||
         p->kind == E_OBJECT3D || p->kind == E_FORCE_FIELD ||
         p->kind == E_POINT)) {
        sr_xml_start_animate(ctx, attrs);
        return;
    }
    if (strcmp(name, "key") == 0 && p->kind == E_ANIMATE) {
        sr_xml_start_key(ctx, attrs);
        return;
    }
    if (strcmp(name, "audioMix") == 0 && p->kind == E_SCENE &&
        !ctx->seen_audio_mix) {
        sr_xml_start_audio_mix(ctx, attrs);
        if (!ctx->failed)
            sr_xml_push(ctx, (ParseFrame){.kind = E_AUDIO_MIX}, name);
        return;
    }
    if (strcmp(name, "audioTrack") == 0 && p->kind == E_AUDIO_MIX) {
        sr_xml_start_audio_track(ctx, attrs);
        if (!ctx->failed)
            sr_xml_push(ctx, (ParseFrame){.kind = E_AUDIO_TRACK}, name);
        return;
    }
    if (strcmp(name, "lights") == 0 && p->kind == E_SCENE &&
        !ctx->seen_lights) {
        if (attrs && attrs[0])
            SR_XML_FAIL_RETURN(ctx, name, attrs[0], "lights has no attributes");
        ctx->seen_lights = true;
        sr_xml_push(ctx, (ParseFrame){.kind = E_LIGHTS}, name);
        return;
    }
    if (strcmp(name, "light") == 0 && p->kind == E_LIGHTS) {
        sr_xml_start_light(ctx, attrs);
        return;
    }
    if (strcmp(name, "effects") == 0 && p->kind == E_SCENE &&
        !ctx->seen_effects) {
        if (attrs && attrs[0])
            SR_XML_FAIL_RETURN(ctx, name, attrs[0], "effects has no attributes");
        ctx->seen_effects = true;
        sr_xml_push(ctx, (ParseFrame){.kind = E_EFFECTS}, name);
        return;
    }
    if (strcmp(name, "effect") == 0 && p->kind == E_EFFECTS) {
        sr_xml_start_effect(ctx, attrs);
        return;
    }
    if (strcmp(name, "physics") == 0 && p->kind == E_SCENE &&
        !ctx->seen_physics) {
        sr_xml_start_physics(ctx, attrs);
        if (!ctx->failed) sr_xml_push(ctx, (ParseFrame){.kind = E_PHYSICS}, name);
        return;
    }
    if (strcmp(name, "forceField") == 0 && p->kind == E_PHYSICS) {
        sr_xml_start_force_field(ctx, attrs);
        return;
    }
    if (strcmp(name, "constraint") == 0 && p->kind == E_PHYSICS) {
        sr_xml_start_constraint(ctx, attrs);
        if (!ctx->failed) sr_xml_push(ctx, (ParseFrame){.kind = E_CONSTRAINT}, name);
        return;
    }
    sr_xml_fail(ctx, name, NULL,
                "element is not valid in this context or supported by scene v1");
}

static void XMLCALL on_end(void *user, const XML_Char *name) {
    ParseContext *ctx = user;
    (void)name;
    if (ctx->element_depth) --ctx->element_depth;
    if (ctx->failed || ctx->depth == 0) return;
    ParseFrame *frame = &ctx->stack[ctx->depth - 1];
    if (frame->kind == E_ANIMATE && frame->color_anim) {
        if (frame->color_anim->r.count == 0)
            SR_XML_FAIL_RETURN(ctx, "animate", NULL,
                               "animation track requires at least one key");
        if (sr_anim_color_finalize(frame->color_anim) != SR_OK)
            SR_XML_FAIL_RETURN(ctx, "animate", NULL,
                               "keyframe times must be unique");
    } else if (frame->kind == E_ANIMATE) {
        if (frame->anim->track.count == 0)
            SR_XML_FAIL_RETURN(ctx, "animate", NULL,
                               "animation track requires at least one key");
        if (sr_track_finalize(&frame->anim->track) != SR_OK)
            SR_XML_FAIL_RETURN(ctx, "animate", NULL,
                               "keyframe times must be unique");
    }
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
    status = sr_xml_schema_check(data, size, path, diag, &deferral);
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
