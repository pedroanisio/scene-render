#include "scene_render/scene.h"
#include "scene_render/video.h"

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
        .background = {0.0, 0.0, 0.0, 1.0}, .mode = SR_MODE_STANDARD
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
    scene->audio.sample_rate = 48000;
    scene->audio.channels = 2;
    scene->scene360.width = 3840;
    scene->scene360.height = 1920;
    scene->physics.fixed_step = 1.0 / 120.0;
    scene->physics.gravity_y = 980.665;
    scene->root = sr_node_create(scene, SR_NODE_GROUP);
    if (scene->root) scene->root->id = sr_strdup("__root__");
}

static void asset_free(SrAsset *asset) {
    free(asset->id);
    free(asset->source);
    free(asset->text);
    free(asset->font_family);
    free(asset->font_file);
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
}

static void light_free(SrLight *light) {
    free(light->id);
    anim_free(&light->intensity);
    anim_free(&light->x); anim_free(&light->y); anim_free(&light->z);
    anim_free(&light->yaw); anim_free(&light->pitch);
}

void sr_scene_free(SrScene *scene) {
    if (!scene) return;
    sr_node_free(scene->root);
    for (size_t i = 0; i < scene->asset_count; ++i) asset_free(&scene->assets[i]);
    for (size_t i = 0; i < scene->audio.track_count; ++i) {
        free(scene->audio.tracks[i].id);
        free(scene->audio.tracks[i].asset_id);
    }
    for (size_t i = 0; i < scene->camera_count; ++i) camera_free(&scene->cameras[i]);
    for (size_t i = 0; i < scene->material_count; ++i)
        free(scene->materials[i].id);
    for (size_t i = 0; i < scene->light_count; ++i) light_free(&scene->lights[i]);
    for (size_t i = 0; i < scene->object3d_count; ++i) {
        free(scene->objects3d[i].id);
        free(scene->objects3d[i].material_id);
        free(scene->objects3d[i].mesh_id);
        transform_free(&scene->objects3d[i].transform);
    }
    for (size_t i = 0; i < scene->effect_count; ++i) {
        free(scene->effects[i].id);
        anim_free(&scene->effects[i].intensity);
        anim_free(&scene->effects[i].radius);
    }
    for (size_t i = 0; i < scene->physics.constraint_count; ++i) {
        free(scene->physics.constraints[i].id);
        free(scene->physics.constraints[i].a_id);
        free(scene->physics.constraints[i].b_id);
    }
    for (size_t i = 0; i < scene->physics.field_count; ++i)
        free(scene->physics.fields[i].id);
    free(scene->assets); free(scene->audio.tracks); free(scene->cameras);
    free(scene->materials); free(scene->lights); free(scene->objects3d);
    free(scene->effects); free(scene->physics.constraints);
    free(scene->physics.fields); free(scene->physics.cache_path);
    free(scene->scene360.viewport_camera_id);
    free(scene->source_path); free(scene->base_dir); free(scene->output.path);
    free(scene->output.pixel_format); free(scene->output.preset);
    free(scene->output.audio_codec);
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
    node->fill = (SrColor){1, 1, 1, 1};
    node->stroke = (SrColor){0, 0, 0, 0};
    node->particle_rate.base = 10.0;
    node->particle_lifetime.base = 1.0;
    node->particle_speed.base = 100.0;
    node->particle_size.base = 4.0;
    node->particle_color = (SrColor){1, 1, 1, 1};
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
        anim_free(&node->modifiers[i].amount);
        anim_free(&node->modifiers[i].frequency);
        anim_free(&node->modifiers[i].phase);
    }
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
    if (!node || !name) return NULL;
    if (strcmp(name, "opacity") == 0) return &node->opacity;
    if (strcmp(name, "position.x") == 0) return &node->transform.x;
    if (strcmp(name, "position.y") == 0) return &node->transform.y;
    if (strcmp(name, "rotation") == 0) return &node->transform.rotation;
    if (strcmp(name, "scale.x") == 0) return &node->transform.scale_x;
    if (strcmp(name, "scale.y") == 0) return &node->transform.scale_y;
    if (strcmp(name, "anchor.x") == 0) return &node->transform.anchor_x;
    if (strcmp(name, "anchor.y") == 0) return &node->transform.anchor_y;
    if (strcmp(name, "source.time") == 0) return &node->source_time;
    if (node->type == SR_NODE_PARTICLES) {
        if (strcmp(name, "rate") == 0) return &node->particle_rate;
        if (strcmp(name, "lifetime") == 0) return &node->particle_lifetime;
        if (strcmp(name, "speed") == 0) return &node->particle_speed;
        if (strcmp(name, "spread") == 0) return &node->particle_spread;
        if (strcmp(name, "size") == 0) return &node->particle_size;
    }
    return NULL;
}

bool sr_blend_parse(const char *text, SrBlendMode *mode) {
    if (!text || !mode) return false;
    static const char *names[] = {"normal", "add", "multiply", "screen",
                                  "overlay", "difference"};
    for (size_t i = 0; i < sizeof(names) / sizeof(names[0]); ++i) {
        if (strcmp(text, names[i]) == 0) {
            *mode = (SrBlendMode)i;
            return true;
        }
    }
    return false;
}

const char *sr_blend_name(SrBlendMode mode) {
    static const char *names[] = {"normal", "add", "multiply", "screen",
                                  "overlay", "difference"};
    return mode >= SR_BLEND_NORMAL && mode <= SR_BLEND_DIFFERENCE ? names[mode]
                                                                  : "unknown";
}
