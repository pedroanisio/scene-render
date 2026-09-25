#include "xml_internal.h"

#include <math.h>
#include <string.h>

static SrEffect *find_effect(SrScene *scene, const char *id) {
    for (size_t i = 0; i < scene->effect_count; ++i)
        if (strcmp(scene->effects[i].id, id) == 0) return &scene->effects[i];
    return NULL;
}

static bool resolve_nodes(ParseContext *ctx, SrNode *node, bool in_card) {
    if (node->card) {
        if (in_card) {
            sr_diag_error(ctx->diag, node->source_line, NULL, "depth",
                          "depth card '%s' is inside another depth card; the "
                          "outer card flattens its subtree", node->id);
            return false;
        }
        ctx->scene->has_cards = true;
    }
    if (node->effect_ref_count) {
        node->effect_refs = sr_alloc(node->effect_ref_count * sizeof(*node->effect_refs));
        if (!node->effect_refs) {
            sr_diag_error(ctx->diag, node->source_line, "group", "effects", "out of memory");
            return false;
        }
        for (size_t i = 0; i < node->effect_ref_count; ++i) {
            SrEffect *effect = find_effect(ctx->scene, node->effect_ids[i]);
            if (!effect) {
                sr_diag_error(ctx->diag, node->source_line, "group", "effects",
                              "unknown effect id '%s'", node->effect_ids[i]);
                return false;
            }
            effect->referenced = true;
            node->effect_refs[i] = effect;
        }
    }
    if (node->type == SR_NODE_MEDIA) {
        node->asset = sr_scene_find_asset(ctx->scene, node->asset_id);
        if (!node->asset) {
            sr_diag_error(ctx->diag, node->source_line, "layer", "asset",
                          "unknown asset id '%s'", node->asset_id);
            return false;
        }
        if (node->asset->type == SR_ASSET_AUDIO) {
            sr_diag_error(ctx->diag, node->source_line, "layer", "asset",
                          "audio assets cannot be used as visual layers");
            return false;
        }
        if (node->clip_out >= 0.0 && node->asset->type == SR_ASSET_VIDEO &&
            node->clip_out > node->asset->duration + 1e-9) {
            sr_diag_error(ctx->diag, node->source_line, "layer", "clipOut",
                          "clipOut %.6f exceeds video duration %.6f",
                          node->clip_out, node->asset->duration);
            return false;
        }
    }
    for (size_t i = 0; i < node->child_count; ++i)
        if (!resolve_nodes(ctx, node->children[i], in_card || node->card))
            return false;
    return true;
}

static bool resolve_audio(ParseContext *ctx) {
    for (size_t i = 0; i < ctx->scene->audio.track_count; ++i) {
        SrAudioTrack *track = &ctx->scene->audio.tracks[i];
        track->asset = sr_scene_find_asset(ctx->scene, track->asset_id);
        /* An audio file, or the soundtrack of a video asset. */
        if (!track->asset || (track->asset->type != SR_ASSET_AUDIO &&
                              track->asset->type != SR_ASSET_VIDEO)) {
            sr_diag_error(ctx->diag, track->source_line, "audioTrack", "asset",
                          "unknown audio or video asset id '%s'",
                          track->asset_id);
            return false;
        }
    }
    return true;
}

static bool resolve_camera(ParseContext *ctx) {
    if (ctx->scene->project.mode == SR_MODE_STANDARD) return true;
    if (!ctx->scene->scene360.enabled) {
        sr_diag_error(ctx->diag, 1, "project", "mode",
                      "360 rendering modes require a scene360 element");
        return false;
    }
    if (ctx->scene->project.mode == SR_MODE_EQUIRECTANGULAR) {
        if (ctx->scene->project.width != ctx->scene->scene360.width ||
            ctx->scene->project.height != ctx->scene->scene360.height) {
            sr_diag_error(ctx->diag, ctx->scene->scene360.source_line,
                          "scene360", "width/height",
                          "equirectangular project and scene360 dimensions must match");
            return false;
        }
        return true;
    }
    const char *wanted = ctx->scene->scene360.viewport_camera_id;
    for (size_t i = 0; i < ctx->scene->camera_count; ++i)
        if ((!wanted && ctx->scene->cameras[i].active) ||
            (wanted && strcmp(ctx->scene->cameras[i].id, wanted) == 0))
            return true;
    sr_diag_error(ctx->diag, ctx->scene->scene360.source_line,
                  "scene360", "viewportCamera",
                  "viewport camera was not found");
    return false;
}

static bool resolve_visual(ParseContext *ctx) {
    for (size_t i = 0; i < ctx->scene->object3d_count; ++i) {
        SrObject3D *object = &ctx->scene->objects3d[i];
        if (object->primitive == SR_OBJECT_MESH) {
            object->mesh_asset=sr_scene_find_asset(ctx->scene,object->mesh_id);
            if(!object->mesh_asset||object->mesh_asset->type!=SR_ASSET_MESH){
            sr_diag_error(ctx->diag,object->source_line,"object3D","mesh",
                              "unknown mesh asset id '%s'",object->mesh_id);
                return false;
            }
        }
        if (!object->material_id) continue;
        for (size_t j = 0; j < ctx->scene->material_count; ++j)
            if (strcmp(ctx->scene->materials[j].id, object->material_id) == 0) {
                object->material = &ctx->scene->materials[j];
                break;
            }
        if (!object->material) {
            sr_diag_error(ctx->diag, object->source_line, "object3D", "material",
                          "unknown material id '%s'", object->material_id);
            return false;
        }
    }
    return true;
}

static bool resolve_effects(ParseContext *ctx) {
    for (size_t i = 0; i < ctx->scene->effect_count; ++i) {
        SrEffect *effect = &ctx->scene->effects[i];
        if (!effect->light_count) continue;
        effect->lights = sr_alloc(effect->light_count * sizeof(*effect->lights));
        if (!effect->lights) {
            sr_diag_error(ctx->diag, effect->source_line, "effect", "lights", "out of memory");
            return false;
        }
        for (size_t j = 0; j < effect->light_count; ++j) {
            SrLight *light = NULL;
            for (size_t k = 0; k < ctx->scene->light_count; ++k)
                if (strcmp(ctx->scene->lights[k].id, effect->light_ids[j]) == 0)
                    light = &ctx->scene->lights[k];
            if (!light) {
                sr_diag_error(ctx->diag, effect->source_line, "effect", "lights",
                              "unknown light id '%s'", effect->light_ids[j]);
                return false;
            }
            light->used_2d = true;
            effect->lights[j] = light;
        }
    }
    return true;
}

static bool resolve_physics(ParseContext *ctx) {
    for (size_t i = 0; i < ctx->scene->physics.constraint_count; ++i) {
        SrConstraint *constraint = &ctx->scene->physics.constraints[i];
        constraint->a = sr_scene_find_node(ctx->scene, constraint->a_id);
        if (constraint->type == SR_CONSTRAINT_PIN) {
            if (!constraint->a || constraint->a->body.type == SR_BODY_NONE) {
                sr_diag_error(ctx->diag, constraint->source_line, "constraint", "a",
                              "pin constraint '%s' requires a rigid-body node id",
                              constraint->id);
                return false;
            }
            if (!constraint->rest_length_set)
                constraint->rest_length = hypot(
                    constraint->a->transform.x.base - constraint->x,
                    constraint->a->transform.y.base - constraint->y);
            continue;
        }
        constraint->b = sr_scene_find_node(ctx->scene, constraint->b_id);
        if (!constraint->a || !constraint->b ||
            constraint->a->body.type == SR_BODY_NONE ||
            constraint->b->body.type == SR_BODY_NONE) {
            sr_diag_error(ctx->diag, constraint->source_line, "constraint", "a/b",
                          "constraint '%s' requires two rigid-body node ids",
                          constraint->id);
            return false;
        }
        if (constraint->rest_length == 0.0) {
            double dx = constraint->b->transform.x.base -
                        constraint->a->transform.x.base;
            double dy = constraint->b->transform.y.base -
                        constraint->a->transform.y.base;
            constraint->rest_length = hypot(dx, dy);
        }
    }
    return true;
}


bool sr_xml_resolve_scene(ParseContext *ctx) {
    if (!resolve_nodes(ctx, ctx->scene->root, false)) return false;
    if (!resolve_audio(ctx)) return false;
    if (!resolve_camera(ctx)) return false;
    if (!resolve_visual(ctx)) return false;
    if (!resolve_effects(ctx)) return false;
    if (!resolve_physics(ctx)) return false;
    sr_node_sort_children(ctx->scene->root);
    return true;
}
