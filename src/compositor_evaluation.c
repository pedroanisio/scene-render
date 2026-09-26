/* SPDX-License-Identifier: Apache-2.0 */
#include "compositor_evaluation_internal.h"
#include "scene_render/card.h"

#include <math.h>
#include <stdlib.h>

SrCompositeOwner sr_composite_node_owner(const SrScene *scene, const SrNode *node,
                                          const char *attribute) {
    const char *element = node == scene->root ? "composition"
        : node->type == SR_NODE_GROUP ? "group"
        : node->type == SR_NODE_MEDIA ? "layer"
        : node->type == SR_NODE_PARTICLES ? "particleEmitter" : "shape";
    return (SrCompositeOwner){node->source_line, element, attribute};
}

static bool values_work(SrCompositeResources *resources,
                          const SrAnimValue *const *values, size_t count) {
    for (size_t i = 0; i < count; ++i)
        if (!sr_composite_anim_work(resources, values[i], false)) return false;
    return true;
}

static bool camera_transform_work(SrCompositeResources *resources,
                                    const SrCamera *camera) {
    const SrAnimValue *values[] = {&camera->x, &camera->y, &camera->z,
        &camera->yaw, &camera->pitch, &camera->roll};
    return values_work(resources, values, sizeof(values) / sizeof(values[0]));
}

bool sr_composite_view_work(SrCompositeResources *resources, const SrScene *scene,
                             double time) {
    if (!resources) return true;
    SrCompositeOwner previous = sr_composite_owner(resources,
        sr_composite_node_owner(scene, scene->root, "camera/object3D/time"));
    bool valid = isfinite(time) && fabs(time) <= SR_MAX_DURATION &&
        scene->camera_count <= SR_MAX_COMPOSITE_CAMERAS &&
        (!scene->camera_count || scene->cameras) &&
        scene->object3d_count <= SR_MAX_COMPOSITE_OBJECTS &&
        (!scene->object3d_count || scene->objects3d);
    if (!valid)
        sr_composite_resource_fail(resources, SR_ERR_RENDER,
            "invalid compositing clock or camera/object count/storage");
    /* This admission scan and the unchanged view's active-camera scan. */
    if (valid) valid = sr_composite_work(resources, scene->camera_count, 2) &&
        sr_composite_work(resources, 1, 32);
    const SrCamera *camera = valid ? sr_active_camera(scene) : NULL;
    if (camera) {
        sr_composite_owner(resources,
            (SrCompositeOwner){camera->source_line, "camera", "transform/focus/zoom/fov"});
        valid = camera_transform_work(resources, camera) &&
            sr_composite_anim_work(resources, &camera->focus_distance, false) &&
            sr_composite_anim_work(resources, &camera->aperture, false);
        if (valid && !camera->orthographic)
            valid = sr_composite_anim_work(resources,
                camera->zoom_set ? &camera->zoom : &camera->fov, false);
    }
    sr_composite_owner(resources, previous);
    return valid;
}

bool sr_composite_world_work(SrCompositeResources *resources, const SrScene *scene,
                              const SrNode *node, bool resolved) {
    if (!resources) return true;
    SrCompositeOwner previous = sr_composite_owner(resources,
        sr_composite_node_owner(scene, node, "transform"));
    const SrTransform *tr = &node->transform;
    const SrAnimValue *values[] = {&tr->rotation, &tr->scale_x, &tr->scale_y};
    bool valid = sr_composite_work(resources, 1, 32) &&
        values_work(resources, values, sizeof(values) / sizeof(values[0]));
    if (valid && !resolved) {
        const SrAnimValue *coordinates[] = {&tr->x, &tr->y, &tr->anchor_x, &tr->anchor_y};
        valid = values_work(resources, coordinates,
            sizeof(coordinates) / sizeof(coordinates[0]));
    }
    if (valid && (tr->skew_x.base != 0 || tr->skew_y.base != 0 ||
                  tr->skew_x.track.count || tr->skew_y.track.count))
        valid = sr_composite_anim_work(resources, &tr->skew_x, false) &&
            sr_composite_anim_work(resources, &tr->skew_y, false);
    sr_composite_owner(resources, previous);
    return valid;
}

bool sr_composite_pivot_work(SrCompositeResources *resources, const SrScene *scene,
                              const SrNode *node, bool resolved, bool pose) {
    if (!resources) return true;
    SrCompositeOwner previous = sr_composite_owner(resources,
        sr_composite_node_owner(scene, node, "depth/rotationX/rotationY"));
    const SrTransform *tr = &node->transform;
    bool valid = sr_composite_work(resources, 1, pose ? 128 : 32) &&
        sr_composite_anim_work(resources, &tr->z, false);
    if (valid && !resolved)
        valid = sr_composite_anim_work(resources, &tr->anchor_x, false) &&
            sr_composite_anim_work(resources, &tr->anchor_y, false);
    if (valid && pose)
        valid = sr_composite_anim_work(resources, &tr->rotation_x, false) &&
            sr_composite_anim_work(resources, &tr->rotation_y, false);
    sr_composite_owner(resources, previous);
    return valid;
}

bool sr_composite_masks_work(SrCompositeResources *resources, const SrNode *node,
                              bool resolved) {
    if (!resources) return true;
    SrCompositeOwner previous = resources->owner;
    bool valid = true;
    for (size_t i = 0; valid && i < node->mask_count; ++i) {
        const SrMask *mask = &node->masks[i];
        sr_composite_owner(resources,
            (SrCompositeOwner){mask->source_line, "mask", "x/y/width/height/radius"});
        /* Admission/evaluation, four-corner clipping and link setup. */
        valid = sr_composite_work(resources, 1, 16) &&
            sr_composite_anim_work(resources, &mask->radius, false);
        if (valid && !resolved) {
            const SrAnimValue *values[] = {&mask->x, &mask->y, &mask->width, &mask->height};
            valid = values_work(resources, values, sizeof(values) / sizeof(values[0]));
        }
    }
    sr_composite_owner(resources, previous);
    return valid;
}

bool sr_composite_color_work(SrCompositeResources *resources, const SrAnimColor *color) {
    if (!resources) return true;
    const SrTrack *tracks[] = {&color->r, &color->g, &color->b, &color->a};
    if (!sr_composite_work(resources, 1, 16)) return false;
    for (size_t i = 0; i < sizeof(tracks) / sizeof(tracks[0]); ++i)
        if (!sr_composite_track_work(resources, tracks[i], false)) return false;
    return true;
}

bool sr_composite_object_key_work(SrCompositeResources *resources,
                                   const SrScene *scene, size_t index) {
    if (!resources) return true;
    const SrObject3D *object = &scene->objects3d[index];
    SrCompositeOwner previous = sr_composite_owner(resources,
        (SrCompositeOwner){object->source_line, "object3D", "x/y/z"});
    const SrTransform *tr = &object->transform;
    const SrAnimValue *values[] = {&tr->x, &tr->y, &tr->z};
    bool valid = sr_composite_work(resources, scene->camera_count, 2) &&
        sr_composite_work(resources, 1, 32) &&
        values_work(resources, values, sizeof(values) / sizeof(values[0]));
    const SrCamera *camera = valid ? sr_active_camera(scene) : NULL;
    if (camera) {
        sr_composite_owner(resources,
            (SrCompositeOwner){camera->source_line, "camera", "transform"});
        valid = camera_transform_work(resources, camera);
    }
    sr_composite_owner(resources, previous);
    return valid;
}

static int item_compare(const void *a, const void *b) {
    const SrDrawItem *x = a, *y = b;
    if (x->key != y->key) return x->key < y->key ? 1 : -1;
    return (x->order > y->order) - (x->order < y->order);
}

static void item_swap(SrDrawItem *a, SrDrawItem *b) {
    SrDrawItem temporary = *a;
    *a = *b;
    *b = temporary;
}

static void sift(SrDrawItem *items, size_t root, size_t count) {
    while (root < count / 2) {
        size_t child = root * 2 + 1;
        if (child + 1 < count && item_compare(&items[child], &items[child + 1]) < 0)
            ++child;
        if (item_compare(&items[root], &items[child]) >= 0) return;
        item_swap(&items[root], &items[child]);
        root = child;
    }
}

bool sr_composite_sort_items(SrCompositeResources *resources, SrDrawItem *items,
                              size_t count) {
    if (!resources) {
        qsort(items, count, sizeof(*items), item_compare);
        return true;
    }
    if (count > SR_MAX_COMPOSITE_DRAW_ITEMS || (count && !items))
        return sr_composite_resource_fail(resources, SR_ERR_RENDER,
            "invalid compositing draw-list count/storage");
    if (count < 2) return true;
    size_t levels = 0;
    for (size_t n = count - 1; n; n >>= 1) ++levels;
    /* Fewer than 2*N sifts, each at most ceil(log2(N)) levels. Root swaps
     * fit the extra level; each level has two compares and three copies. */
    if (!sr_composite_work(resources, 2 * count * (levels + 1),
                            4 + 3 * ((sizeof(*items) + 3) / 4))) return false;
    for (size_t root = count / 2; root > 0; --root) sift(items, root - 1, count);
    for (size_t end = count; end > 1; --end) {
        item_swap(&items[0], &items[end - 1]);
        sift(items, 0, end - 1);
    }
    return true;
}
