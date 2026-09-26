#include "scene_render/scene.h"
#include "scene_render/property.h"
#include "scene_render/text.h"
#include "scene_render/video.h"
#include "scene_render/color.h"
#include "scene_render/compositing.h"

#include <math.h>
#include <stdlib.h>
#include <string.h>

static void anim_free(SrAnimValue *value) {
    sr_track_free(&value->track);
}

static void transform_free(SrTransform *transform) {
    anim_free(&transform->x);
    anim_free(&transform->y);
    anim_free(&transform->z);
    anim_free(&transform->rotation);
    anim_free(&transform->rotation_x);
    anim_free(&transform->rotation_y);
    anim_free(&transform->scale_x);
    anim_free(&transform->scale_y);
    anim_free(&transform->skew_x);
    anim_free(&transform->skew_y);
    anim_free(&transform->scale_z);
    anim_free(&transform->anchor_x);
    anim_free(&transform->anchor_y);
}

static void image_free(SrImage *image) {
    if (!image) return;
    free(image->px);
    free(image);
}

void sr_scene_init(SrScene *scene) {
    *scene = (SrScene){0};
    scene->project = (SrProject){
        .width = 3840, .height = 2160, .fps_num = 30, .fps_den = 1,
        .duration = 10.0, .linear_light = true,
        .working_color_space = SR_COLOR_SRGB,
        .background = {0.0, 0.0, 0.0, 1.0}, .mode = SR_MODE_STANDARD,
        .antialias3d = 1
    };
    scene->output.codec = SR_CODEC_H264;
    scene->output.path = sr_strdup("build/output.mp4");
    scene->output.pixel_format = sr_strdup("yuv420p");
    scene->output.preset = sr_strdup("medium");
    scene->output.audio_codec = sr_strdup("aac");
    scene->output.crf = 18;
    scene->output.audio_bitrate = 192000;
    scene->output.color_space = SR_COLOR_SRGB;
    scene->output.spherical_metadata = true;
    scene->output.embed_metadata = true;
    scene->audio.sample_rate = 48000;
    scene->audio.channels = 2;
    scene->scene360.width = 3840;
    scene->scene360.height = 1920;
    scene->physics.fixed_step = 1.0 / 120.0;
    scene->physics.gravity_y = 980.665;
    scene->root = sr_node_create(scene, SR_NODE_GROUP);
    if (scene->root) {
        scene->root->id = sr_strdup("__root__");
        /* A root without its id is an allocation failure too: callers
         * check scene->root. */
        if (!scene->root->id) {
            sr_node_free(scene->root);
            scene->root = NULL;
        }
    }
}

static void asset_free(SrAsset *asset) {
    free(asset->id);
    free(asset->source);
    free(asset->text);
    free(asset->font_family);
    free(asset->font_file);
    free(asset->text_language);
    free(asset->vector_path);
    free(asset->audio_pcm);
    if (asset->mesh) {
        free(asset->mesh->triangles);
        free(asset->mesh);
    }
    image_free(asset->decoded);
    sr_video_close(asset->video);
}

static void camera_free(SrCamera *camera) {
    free(camera->id);
    anim_free(&camera->x); anim_free(&camera->y); anim_free(&camera->z);
    anim_free(&camera->yaw); anim_free(&camera->pitch);
    anim_free(&camera->roll); anim_free(&camera->fov);
    anim_free(&camera->zoom); anim_free(&camera->focus_distance);
    anim_free(&camera->aperture);
}

static void light_free(SrLight *light) {
    free(light->id);
    sr_anim_color_free(&light->color);
    anim_free(&light->intensity);
    anim_free(&light->x); anim_free(&light->y); anim_free(&light->z);
    anim_free(&light->yaw); anim_free(&light->pitch);
}

void sr_scene_free(SrScene *scene) {
    if (!scene) return;
    sr_scene_invalidate_compositing(scene);
    sr_node_free(scene->root);
    for (size_t i = 0; i < scene->token_count; ++i) {
        free(scene->tokens[i].name);
        free(scene->tokens[i].value);
    }
    free(scene->tokens);
    for (size_t i = 0; i < scene->metadata_count; ++i) {
        free(scene->metadata[i].name);
        free(scene->metadata[i].value);
    }
    free(scene->metadata);
    for (size_t i = 0; i < scene->asset_count; ++i) asset_free(&scene->assets[i]);
    for (size_t i = 0; i < scene->audio.track_count; ++i) {
        free(scene->audio.tracks[i].id);
        free(scene->audio.tracks[i].asset_id);
        anim_free(&scene->audio.tracks[i].volume);
        anim_free(&scene->audio.tracks[i].pan);
    }
    for (size_t i = 0; i < scene->camera_count; ++i) camera_free(&scene->cameras[i]);
    for (size_t i = 0; i < scene->material_count; ++i) {
        free(scene->materials[i].id);
        sr_anim_color_free(&scene->materials[i].base_color);
        sr_anim_color_free(&scene->materials[i].emissive);
        anim_free(&scene->materials[i].metallic);
        anim_free(&scene->materials[i].roughness);
    }
    for (size_t i = 0; i < scene->light_count; ++i) light_free(&scene->lights[i]);
    for (size_t i = 0; i < scene->object3d_count; ++i) {
        free(scene->objects3d[i].id);
        free(scene->objects3d[i].material_id);
        free(scene->objects3d[i].mesh_id);
        transform_free(&scene->objects3d[i].transform);
    }
    for (size_t i = 0; i < scene->effect_count; ++i) {
        SrEffect *effect = &scene->effects[i];
        free(effect->id);
        anim_free(&effect->intensity);
        anim_free(&effect->radius);
        anim_free(&effect->threshold);
        anim_free(&effect->saturation);
        anim_free(&effect->contrast);
        anim_free(&effect->brightness);
        anim_free(&effect->offset_x);
        anim_free(&effect->offset_y);
        anim_free(&effect->relief);
        sr_anim_color_free(&effect->color);
        for (size_t j = 0; j < effect->light_count; ++j)
            free(effect->light_ids[j]);
        free(effect->light_ids);
        free(effect->lights);
    }
    for (size_t i = 0; i < scene->physics.constraint_count; ++i) {
        free(scene->physics.constraints[i].id);
        free(scene->physics.constraints[i].a_id);
        free(scene->physics.constraints[i].b_id);
    }
    for (size_t i = 0; i < scene->physics.field_count; ++i) {
        SrForceField *field = &scene->physics.fields[i];
        free(field->id);
        anim_free(&field->x); anim_free(&field->y);
        anim_free(&field->force_x); anim_free(&field->force_y);
        anim_free(&field->strength);
    }
    free(scene->assets); free(scene->audio.tracks); free(scene->cameras);
    free(scene->materials); free(scene->lights); free(scene->objects3d);
    free(scene->effects); free(scene->physics.constraints);
    free(scene->physics.fields); free(scene->physics.cache_path);
    free(scene->physics.cache_dir);
    free(scene->scene360.viewport_camera_id);
    free(scene->source_path); free(scene->base_dir); free(scene->output.path);
    free(scene->output.pixel_format); free(scene->output.preset);
    free(scene->output.audio_codec);
    sr_font_cache_free(scene->font_cache);
    *scene = (SrScene){0};
}

SrAsset *sr_scene_add_asset(SrScene *scene) {
    if (scene->asset_count == scene->asset_capacity) {
        size_t capacity = scene->asset_capacity ? scene->asset_capacity * 2 : 8;
        SrAsset *assets = sr_realloc(scene->assets, capacity * sizeof(*assets));
        if (!assets) return NULL;
        memset(assets + scene->asset_capacity, 0,
               (capacity - scene->asset_capacity) * sizeof(*assets));
        scene->assets = assets;
        scene->asset_capacity = capacity;
    }
    SrAsset *asset = &scene->assets[scene->asset_count++];
    asset->fps_num = 30;
    asset->fps_den = 1;
    asset->color = (SrColor){1, 1, 1, 1};
    return asset;
}

SrAsset *sr_scene_find_asset(SrScene *scene, const char *id) {
    if (!scene || !id) return NULL;
    for (size_t i = 0; i < scene->asset_count; ++i)
        if (scene->assets[i].id && strcmp(scene->assets[i].id, id) == 0)
            return &scene->assets[i];
    return NULL;
}

SrNode *sr_node_create(SrScene *scene, SrNodeType type) {
    SrNode *node = sr_alloc(sizeof(*node));
    if (!node) return NULL;
    node->type = type;
    node->order = scene ? scene->next_order++ : 0;
    node->visible = true;
    node->end_time = INFINITY;
    node->opacity.base = 1.0;
    node->transform.scale_x.base = 1.0;
    node->transform.scale_y.base = 1.0;
    node->transform.scale_z.base = 1.0;
    node->blend = SR_BLEND_NORMAL;
    node->clip_out = -1.0;
    node->speed = 1.0;
    node->time_stretch = 1.0;
    node->fill = sr_anim_color_static((SrColor){1, 1, 1, 1});
    node->stroke = sr_anim_color_static((SrColor){0, 0, 0, 0});
    node->particle_rate.base = 10.0;
    node->particle_lifetime.base = 1.0;
    node->particle_speed.base = 100.0;
    node->particle_size.base = 4.0;
    node->particle_direction.base = -90.0;
    node->particle_color = sr_anim_color_static((SrColor){1, 1, 1, 1});
    node->particle_color_end = sr_anim_color_static((SrColor){1, 1, 1, 0});
    node->particle_max = 10000;
    node->particle_speed_factor = 1.0;
    node->particle_wobble_frequency = 3.0;
    node->soft_body.rows = 4;
    node->soft_body.cols = 4;
    node->body.mass = 1.0;
    node->body.friction = 0.5;
    node->body.linear_damping = 0.01;
    node->body.angular_damping = 0.01;
    node->body.collider = SR_COLLIDER_BOX;
    return node;
}

SrStatus sr_node_add_child(SrNode *parent, SrNode *child) {
    if (!parent || !child || parent->type != SR_NODE_GROUP)
        return SR_ERR_ARGUMENT;
    if (parent->child_count == parent->child_capacity) {
        size_t capacity = parent->child_capacity ? parent->child_capacity * 2 : 8;
        SrNode **children = sr_realloc(parent->children,
                                       capacity * sizeof(*children));
        if (!children) return SR_ERR_MEMORY;
        parent->children = children;
        parent->child_capacity = capacity;
    }
    parent->children[parent->child_count++] = child;
    return SR_OK;
}

SrStatus sr_node_add_modifier(SrNode *node, SrModifier modifier) {
    if (node->modifier_count == node->modifier_capacity) {
        size_t capacity = node->modifier_capacity ? node->modifier_capacity * 2 : 4;
        SrModifier *items = sr_realloc(node->modifiers,
                                       capacity * sizeof(*items));
        if (!items) return SR_ERR_MEMORY;
        node->modifiers = items;
        node->modifier_capacity = capacity;
    }
    node->modifiers[node->modifier_count++] = modifier;
    return SR_OK;
}

SrStatus sr_node_add_mask(SrNode *node, SrMask mask) {
    if (node->mask_count == node->mask_capacity) {
        size_t capacity = node->mask_capacity ? node->mask_capacity * 2 : 2;
        SrMask *items = sr_realloc(node->masks, capacity * sizeof(*items));
        if (!items) return SR_ERR_MEMORY;
        node->masks = items;
        node->mask_capacity = capacity;
    }
    node->masks[node->mask_count++] = mask;
    return SR_OK;
}

void sr_node_free(SrNode *node) {
    if (!node) return;
    for (size_t i = 0; i < node->child_count; ++i) sr_node_free(node->children[i]);
    for (size_t i = 0; i < node->modifier_count; ++i) {
        SrModifier *modifier = &node->modifiers[i];
        anim_free(&modifier->amount);
        anim_free(&modifier->frequency);
        anim_free(&modifier->phase);
        if (modifier->points) {
            size_t count = (size_t)modifier->rows * modifier->cols * 2;
            for (size_t j = 0; j < count; ++j) anim_free(&modifier->points[j]);
            free(modifier->points);
        }
    }
    for (size_t i = 0; i < node->effect_ref_count; ++i) free(node->effect_ids[i]);
    free(node->effect_ids);
    free(node->effect_refs);
    free(node->soft_body.offsets);
    sr_anim_color_free(&node->fill);
    sr_anim_color_free(&node->stroke);
    sr_anim_color_free(&node->particle_color);
    sr_anim_color_free(&node->particle_color_end);
    anim_free(&node->particle_direction);
    for (size_t i = 0; i < node->mask_count; ++i) {
        anim_free(&node->masks[i].x);
        anim_free(&node->masks[i].y);
        anim_free(&node->masks[i].width);
        anim_free(&node->masks[i].height);
        anim_free(&node->masks[i].radius);
    }
    free(node->masks);
    free(node->children); free(node->modifiers); free(node->physics_samples);
    free(node->id); free(node->asset_id); free(node->particle_preset);
    free(node->particle_rate_cache);
    anim_free(&node->opacity); anim_free(&node->source_time);
    anim_free(&node->particle_rate); anim_free(&node->particle_lifetime);
    anim_free(&node->particle_speed); anim_free(&node->particle_spread);
    anim_free(&node->particle_size);
    transform_free(&node->transform);
    free(node);
}

static int node_compare(const void *left, const void *right) {
    const SrNode *a = *(const SrNode *const *)left;
    const SrNode *b = *(const SrNode *const *)right;
    if (a->z != b->z) return (a->z > b->z) - (a->z < b->z);
    return (a->order > b->order) - (a->order < b->order);
}

void sr_node_sort_children(SrNode *node) {
    if (!node || node->type != SR_NODE_GROUP) return;
    /* qsort's base must be non-null even for zero elements (C17 7.1.4). */
    if (node->child_count > 1)
        qsort(node->children, node->child_count, sizeof(*node->children),
              node_compare);
    for (size_t i = 0; i < node->child_count; ++i)
        sr_node_sort_children(node->children[i]);
}

static SrNode *find_node(SrNode *node, const char *id) {
    if (node->id && strcmp(node->id, id) == 0) return node;
    for (size_t i = 0; i < node->child_count; ++i) {
        SrNode *found = find_node(node->children[i], id);
        if (found) return found;
    }
    return NULL;
}

SrNode *sr_scene_find_node(SrScene *scene, const char *id) {
    return scene && id ? find_node(scene->root, id) : NULL;
}

bool sr_scene_id_exists(const SrScene *scene, const char *id) {
    if (!scene || !id) return false;
    if (find_node(scene->root, id)) return true;
    for (size_t i = 0; i < scene->asset_count; ++i)
        if (scene->assets[i].id && strcmp(scene->assets[i].id, id) == 0) return true;
    for (size_t i = 0; i < scene->audio.track_count; ++i)
        if (scene->audio.tracks[i].id && strcmp(scene->audio.tracks[i].id, id) == 0) return true;
    for (size_t i = 0; i < scene->camera_count; ++i)
        if (scene->cameras[i].id && strcmp(scene->cameras[i].id, id) == 0) return true;
    for (size_t i = 0; i < scene->material_count; ++i)
        if (scene->materials[i].id && strcmp(scene->materials[i].id, id) == 0) return true;
    for (size_t i = 0; i < scene->light_count; ++i)
        if (scene->lights[i].id && strcmp(scene->lights[i].id, id) == 0) return true;
    for (size_t i = 0; i < scene->object3d_count; ++i)
        if (scene->objects3d[i].id && strcmp(scene->objects3d[i].id, id) == 0) return true;
    for (size_t i = 0; i < scene->effect_count; ++i)
        if (scene->effects[i].id && strcmp(scene->effects[i].id, id) == 0) return true;
    for (size_t i = 0; i < scene->physics.constraint_count; ++i)
        if (scene->physics.constraints[i].id &&
            strcmp(scene->physics.constraints[i].id, id) == 0) return true;
    for (size_t i = 0; i < scene->physics.field_count; ++i)
        if (scene->physics.fields[i].id &&
            strcmp(scene->physics.fields[i].id, id) == 0) return true;
    return false;
}

SrAnimValue *sr_node_property(SrNode *node, const char *name) {
    const SrProperty *property = sr_property_find(sr_property_node_host(node), name);
    return property && property->type == SR_PROPERTY_NUMBER
        ? sr_property_target(property, node) : NULL;
}

SrAnimColor *sr_node_color_property(SrNode *node, const char *name) {
    const SrProperty *property = sr_property_find(sr_property_node_host(node), name);
    if (!property || property->type != SR_PROPERTY_COLOR) return NULL;
    sr_property_activate(property, node);
    return sr_property_target(property, node);
}

static const char *const blend_names[SR_BLEND_COUNT] = {
    "normal", "add", "multiply", "screen", "overlay", "difference",
    "plus-lighter", "exclusion", "subtract", "divide", "darken", "lighten",
    "darker-color", "lighter-color", "color-dodge", "color-burn",
    "linear-dodge", "linear-burn", "soft-light", "hard-light", "linear-light",
    "vivid-light", "pin-light", "hard-mix", "hue", "saturation", "color",
    "luminosity"
};

bool sr_blend_parse(const char *text, SrBlendMode *mode) {
    if (!text || !mode) return false;
    for (size_t i = 0; i < SR_BLEND_COUNT; ++i) {
        if (strcmp(text, blend_names[i]) == 0) {
            *mode = (SrBlendMode)i;
            return true;
        }
    }
    return false;
}

const char *sr_blend_name(SrBlendMode mode) {
    return mode >= SR_BLEND_NORMAL && mode < SR_BLEND_COUNT
         ? blend_names[mode] : "unknown";
}
