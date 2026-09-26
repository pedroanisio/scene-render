/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SCENE_RENDER_PROPERTY_H
#define SCENE_RENDER_PROPERTY_H

#include "scene_render/scene.h"

typedef enum {
    SR_PROPERTY_GROUP, SR_PROPERTY_LAYER, SR_PROPERTY_SHAPE,
    SR_PROPERTY_PARTICLES, SR_PROPERTY_MASK, SR_PROPERTY_POINT,
    SR_PROPERTY_CAMERA, SR_PROPERTY_LIGHT, SR_PROPERTY_EFFECT,
    SR_PROPERTY_FIELD, SR_PROPERTY_MODIFIER, SR_PROPERTY_OBJECT3D,
    SR_PROPERTY_MATERIAL, SR_PROPERTY_AUDIO_TRACK,
    SR_PROPERTY_HOST_COUNT
} SrPropertyHost;

typedef enum {
    SR_PROPERTY_NUMBER, SR_PROPERTY_POINT_VALUE, SR_PROPERTY_COLOR,
    SR_PROPERTY_PAINT, SR_PROPERTY_PATH, SR_PROPERTY_STRING
} SrPropertyType;

enum {
    SR_PROPERTY_CAMERA_ZOOM = 1u,
    SR_PROPERTY_PARTICLE_END_COLOR = 2u,
    SR_PROPERTY_DEPTH_CARD = 4u,
    SR_PROPERTY_LENGTH_X = 8u,
    SR_PROPERTY_LENGTH_Y = 16u
};

/* Immutable registry entries. Offsets address scene-owned animation storage;
 * bounds describe accepted numeric keys, preserving legacy loader ranges. */
typedef struct {
    uint32_t hosts;
    const char *name;
    const char *attribute;
    SrPropertyType type;
    size_t offset;
    double minimum, maximum;
    unsigned flags;
    const char *bounds_error;
} SrProperty;

SrPropertyHost sr_property_node_host(const SrNode *node);
const SrProperty *sr_property_find(SrPropertyHost host, const char *name);
const SrProperty *sr_property_at(size_t index);
size_t sr_property_count(void);
/* Returns borrowed mutable storage, or NULL when the property is ineligible.
 * Finding/addressing a property has no activation side effects. */
void *sr_property_target(const SrProperty *property, void *host);
void sr_property_activate(const SrProperty *property, void *host);
bool sr_property_key_valid(const SrProperty *property, double value);

#endif
