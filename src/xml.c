#include "scene_render/xml.h"

#include "xml_internal.h"

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
    if (ctx->depth >= sizeof(ctx->stack) / sizeof(ctx->stack[0])) {
        sr_xml_fail(ctx, element, NULL, "XML nesting exceeds 96 elements");
        return;
    }
    ctx->stack[ctx->depth++] = frame;
}

static void XMLCALL on_start(void *user, const XML_Char *name,
                             const XML_Char **attrs) {
    ParseContext *ctx = user;
    if (ctx->failed) return;
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
    if (strcmp(name, "animate") == 0 &&
        (p->kind == E_GROUP || p->kind == E_LAYER || p->kind == E_PARTICLES ||
         p->kind == E_CAMERA ||
         p->kind == E_LIGHT || p->kind == E_EFFECT || p->kind == E_MODIFIER ||
         p->kind == E_OBJECT3D)) {
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
        if (!ctx->failed) sr_xml_push(ctx, (ParseFrame){.kind = E_FORCE_FIELD}, name);
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
    if (ctx->failed || ctx->depth == 0) return;
    ParseFrame *frame = &ctx->stack[ctx->depth - 1];
    if (frame->kind == E_ANIMATE) {
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

SrStatus sr_scene_load_xml(const char *path, SrScene *scene,
                           SrDiagnostics *diag) {
    if (!path || !scene || !diag) return SR_ERR_ARGUMENT;
    sr_scene_init(scene);
    scene->source_path = sr_strdup(path);
    scene->base_dir = sr_path_dirname(path);
    if (!scene->source_path || !scene->base_dir || !scene->root) {
        sr_scene_free(scene);
        return SR_ERR_MEMORY;
    }
    FILE *file = fopen(path, "rb");
    if (!file) {
        sr_diag_error(diag, 0, NULL, NULL, "cannot open XML scene: %s",
                      strerror(errno));
        sr_scene_free(scene);
        return SR_ERR_IO;
    }
    XML_Parser parser = XML_ParserCreate(NULL);
    if (!parser) {
        fclose(file);
        sr_scene_free(scene);
        return SR_ERR_MEMORY;
    }
    ParseContext ctx = {.parser = parser, .scene = scene, .diag = diag};
    XML_SetUserData(parser, &ctx);
    XML_SetElementHandler(parser, on_start, on_end);
    XML_SetCharacterDataHandler(parser, on_text);
    XML_SetStartDoctypeDeclHandler(parser, on_doctype);
    char buffer[16384];
    bool done = false;
    while (!done && !ctx.failed) {
        size_t count = fread(buffer, 1, sizeof(buffer), file);
        done = count < sizeof(buffer);
        if (ferror(file)) {
            sr_xml_fail(&ctx, NULL, NULL, "I/O error while reading XML");
            break;
        }
        if (XML_Parse(parser, buffer, (int)count, done) == XML_STATUS_ERROR &&
            !ctx.failed) {
            sr_diag_error(diag, (size_t)XML_GetCurrentLineNumber(parser), NULL,
                          NULL, "XML syntax: %s",
                          XML_ErrorString(XML_GetErrorCode(parser)));
            ctx.failed = true;
        }
    }
    fclose(file);
    XML_ParserFree(parser);
    if (!ctx.failed && (!ctx.seen_project || !ctx.seen_composition)) {
        sr_diag_error(diag, 1, "scene", NULL,
                      "scene requires exactly one project and one composition");
        ctx.failed = true;
    }
    if (!ctx.failed && !sr_xml_resolve_scene(&ctx)) ctx.failed = true;
    if (ctx.failed) {
        sr_scene_free(scene);
        return SR_ERR_XML;
    }
    return SR_OK;
}
