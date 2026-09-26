/* SPDX-License-Identifier: Apache-2.0 */
#include "scene_render/property.h"

#include <float.h>
#include <math.h>
#include <stddef.h>
#include <string.h>

#define H(kind) (UINT32_C(1) << SR_PROPERTY_##kind)
#define NODES (H(GROUP) | H(LAYER) | H(SHAPE) | H(PARTICLES))
#define NUMBER(hosts, type, field, name, attr, flags) \
    {hosts, name, attr, SR_PROPERTY_NUMBER, offsetof(type, field), \
     -DBL_MAX, DBL_MAX, flags, NULL}
#define COLOR(hosts, type, field, name, attr, flags) \
    {hosts, name, attr, SR_PROPERTY_COLOR, offsetof(type, field), \
     0.0, 1.0, flags, NULL}
#define BOUNDED(hosts, type, field, name, attr, low, high, error) \
    {hosts, name, attr, SR_PROPERTY_NUMBER, offsetof(type, field), \
     low, high, 0, error}

static const SrProperty properties[] = {
    COLOR(H(MATERIAL), SrMaterial, base_color, "baseColor", "baseColor", 0),
    COLOR(H(MATERIAL), SrMaterial, emissive, "emissive", "emissive", 0),
    BOUNDED(H(MATERIAL), SrMaterial, metallic, "metallic", "metallic", 0, 1,
            "material metallic keys must be in [0,1]"),
    BOUNDED(H(MATERIAL), SrMaterial, roughness, "roughness", "roughness", 0, 1,
            "material roughness keys must be in [0,1]"),
    BOUNDED(H(AUDIO_TRACK), SrAudioTrack, volume, "volume", "volume", 0, 1,
            "audio volume keys must be in [0,1]"),
    BOUNDED(H(AUDIO_TRACK), SrAudioTrack, pan, "pan", "pan", -1, 1,
            "audio pan keys must be in [-1,1]"),
    NUMBER(NODES, SrNode, opacity, "opacity", "opacity", 0),
    NUMBER(NODES, SrNode, transform.x, "position.x", "x", SR_PROPERTY_LENGTH_X),
    NUMBER(NODES, SrNode, transform.y, "position.y", "y", SR_PROPERTY_LENGTH_Y),
    NUMBER(NODES, SrNode, transform.rotation, "rotation", "rotation", 0),
    NUMBER(NODES, SrNode, transform.scale_x, "scale.x", "scaleX", 0),
    NUMBER(NODES, SrNode, transform.scale_y, "scale.y", "scaleY", 0),
    NUMBER(NODES, SrNode, transform.anchor_x, "anchor.x", "anchorX", SR_PROPERTY_LENGTH_X),
    NUMBER(NODES, SrNode, transform.anchor_y, "anchor.y", "anchorY", SR_PROPERTY_LENGTH_Y),
    NUMBER(NODES, SrNode, source_time, "source.time", NULL, 0),
    NUMBER(NODES, SrNode, transform.z, "depth", "depth", SR_PROPERTY_DEPTH_CARD),
    NUMBER(NODES, SrNode, transform.rotation_x, "rotation.x", "rotationX",
           SR_PROPERTY_DEPTH_CARD),
    NUMBER(NODES, SrNode, transform.rotation_y, "rotation.y", "rotationY",
           SR_PROPERTY_DEPTH_CARD),
    NUMBER(H(PARTICLES), SrNode, particle_rate, "rate", "rate", 0),
    NUMBER(H(PARTICLES), SrNode, particle_lifetime, "lifetime", "lifetime", 0),
    NUMBER(H(PARTICLES), SrNode, particle_speed, "speed", "speed", 0),
    NUMBER(H(PARTICLES), SrNode, particle_spread, "spread", "spread", 0),
    NUMBER(H(PARTICLES), SrNode, particle_size, "size", "size", 0),
    NUMBER(H(PARTICLES), SrNode, particle_direction, "direction", "direction", 0),
    COLOR(H(SHAPE), SrNode, fill, "fill", "fill", 0),
    COLOR(H(SHAPE), SrNode, stroke, "stroke", "stroke", 0),
    COLOR(H(PARTICLES), SrNode, particle_color, "color", "color", 0),
    COLOR(H(PARTICLES), SrNode, particle_color_end, "colorEnd", "colorEnd",
          SR_PROPERTY_PARTICLE_END_COLOR),
    NUMBER(H(MASK), SrMask, x, "x", "x", SR_PROPERTY_LENGTH_X),
    NUMBER(H(MASK), SrMask, y, "y", "y", SR_PROPERTY_LENGTH_Y),
    NUMBER(H(MASK), SrMask, width, "width", "width", SR_PROPERTY_LENGTH_X),
    NUMBER(H(MASK), SrMask, height, "height", "height", SR_PROPERTY_LENGTH_Y),
    NUMBER(H(MASK), SrMask, radius, "radius", "radius", 0),
    {H(POINT), "x", "x", SR_PROPERTY_NUMBER, 0,
     -DBL_MAX, DBL_MAX, 0, NULL},
    {H(POINT), "y", "y", SR_PROPERTY_NUMBER, sizeof(SrAnimValue),
     -DBL_MAX, DBL_MAX, 0, NULL},
    NUMBER(H(CAMERA), SrCamera, x, "position.x", "x", 0),
    NUMBER(H(CAMERA), SrCamera, y, "position.y", "y", 0),
    NUMBER(H(CAMERA), SrCamera, z, "position.z", "z", 0),
    NUMBER(H(CAMERA), SrCamera, yaw, "yaw", "yaw", 0),
    NUMBER(H(CAMERA), SrCamera, pitch, "pitch", "pitch", 0),
    NUMBER(H(CAMERA), SrCamera, roll, "roll", "roll", 0),
    NUMBER(H(CAMERA), SrCamera, fov, "fov", "fov", 0),
    NUMBER(H(CAMERA), SrCamera, zoom, "zoom", "zoom", SR_PROPERTY_CAMERA_ZOOM),
    NUMBER(H(CAMERA), SrCamera, focus_distance, "focusDistance", "focusDistance", 0),
    NUMBER(H(CAMERA), SrCamera, aperture, "aperture", "aperture", 0),
    COLOR(H(LIGHT), SrLight, color, "color", "color", 0),
    BOUNDED(H(LIGHT), SrLight, intensity, "intensity", "intensity",
            -DBL_MAX, SR_MAX_LIGHT_INTENSITY, "light intensity must be at most 1e6"),
    NUMBER(H(LIGHT), SrLight, x, "position.x", "x", 0),
    NUMBER(H(LIGHT), SrLight, y, "position.y", "y", 0),
    NUMBER(H(LIGHT), SrLight, z, "position.z", "z", 0),
    NUMBER(H(LIGHT), SrLight, yaw, "yaw", "yaw", 0),
    NUMBER(H(LIGHT), SrLight, pitch, "pitch", "pitch", 0),
    COLOR(H(EFFECT), SrEffect, color, "color", "color", 0),
    NUMBER(H(EFFECT), SrEffect, intensity, "intensity", "intensity", 0),
    BOUNDED(H(EFFECT), SrEffect, radius, "radius", "radius", -DBL_MAX,
            SR_MAX_EFFECT_RADIUS, "effect radius must be at most 4096 px"),
    NUMBER(H(EFFECT), SrEffect, threshold, "threshold", "threshold", 0),
    NUMBER(H(EFFECT), SrEffect, saturation, "saturation", "saturation", 0),
    NUMBER(H(EFFECT), SrEffect, contrast, "contrast", "contrast", 0),
    NUMBER(H(EFFECT), SrEffect, brightness, "brightness", "brightness", 0),
    BOUNDED(H(EFFECT), SrEffect, offset_x, "offsetX", "offsetX",
            -SR_MAX_EFFECT_OFFSET, SR_MAX_EFFECT_OFFSET,
            "effect offsets must be within +-1e5 px"),
    BOUNDED(H(EFFECT), SrEffect, offset_y, "offsetY", "offsetY",
            -SR_MAX_EFFECT_OFFSET, SR_MAX_EFFECT_OFFSET,
            "effect offsets must be within +-1e5 px"),
    NUMBER(H(EFFECT), SrEffect, relief, "relief", "relief", 0),
    NUMBER(H(FIELD), SrForceField, strength, "strength", "strength", 0),
    NUMBER(H(FIELD), SrForceField, force_x, "forceX", "forceX", 0),
    NUMBER(H(FIELD), SrForceField, force_y, "forceY", "forceY", 0),
    NUMBER(H(FIELD), SrForceField, x, "x", "x", 0),
    NUMBER(H(FIELD), SrForceField, y, "y", "y", 0),
    NUMBER(H(MODIFIER), SrModifier, amount, "amount", "amount", 0),
    NUMBER(H(MODIFIER), SrModifier, frequency, "frequency", "frequency", 0),
    NUMBER(H(MODIFIER), SrModifier, phase, "phase", "phase", 0),
    NUMBER(H(OBJECT3D), SrObject3D, transform.x, "position.x", "x", 0),
    NUMBER(H(OBJECT3D), SrObject3D, transform.y, "position.y", "y", 0),
    NUMBER(H(OBJECT3D), SrObject3D, transform.z, "position.z", "z", 0),
    NUMBER(H(OBJECT3D), SrObject3D, transform.rotation, "rotation", "rotation", 0),
    NUMBER(H(OBJECT3D), SrObject3D, transform.rotation_x, "rotation.x", "rotationX", 0),
    NUMBER(H(OBJECT3D), SrObject3D, transform.rotation_y, "rotation.y", "rotationY", 0),
    NUMBER(H(OBJECT3D), SrObject3D, transform.scale_x, "scale.x", "scaleX", 0),
    NUMBER(H(OBJECT3D), SrObject3D, transform.scale_y, "scale.y", "scaleY", 0),
    NUMBER(H(OBJECT3D), SrObject3D, transform.scale_z, "scale.z", "scaleZ", 0),
};

SrPropertyHost sr_property_node_host(const SrNode *node) {
    if (!node) return SR_PROPERTY_HOST_COUNT;
    switch (node->type) {
    case SR_NODE_GROUP: return SR_PROPERTY_GROUP;
    case SR_NODE_MEDIA: return SR_PROPERTY_LAYER;
    case SR_NODE_SHAPE: return SR_PROPERTY_SHAPE;
    case SR_NODE_PARTICLES: return SR_PROPERTY_PARTICLES;
    default: return SR_PROPERTY_HOST_COUNT;
    }
}

size_t sr_property_count(void) {
    return sizeof(properties) / sizeof(properties[0]);
}

const SrProperty *sr_property_at(size_t index) {
    return index < sr_property_count() ? &properties[index] : NULL;
}

const SrProperty *sr_property_find(SrPropertyHost host, const char *name) {
    if (!name || host < 0 || host >= SR_PROPERTY_HOST_COUNT) return NULL;
    uint32_t mask = UINT32_C(1) << host;
    for (size_t i = 0; i < sr_property_count(); ++i)
        if ((properties[i].hosts & mask) && !strcmp(properties[i].name, name))
            return &properties[i];
    return NULL;
}

void *sr_property_target(const SrProperty *property, void *host) {
    if (!property || !host) return NULL;
    if ((property->flags & SR_PROPERTY_CAMERA_ZOOM) &&
        !((SrCamera *)host)->zoom_set) return NULL;
    return (unsigned char *)host + property->offset;
}

void sr_property_activate(const SrProperty *property, void *host) {
    if (property && host && (property->flags & SR_PROPERTY_PARTICLE_END_COLOR))
        ((SrNode *)host)->particle_color_end_set = true;
}

bool sr_property_key_valid(const SrProperty *property, double value) {
    return property && isfinite(value) && value >= property->minimum &&
           value <= property->maximum;
}
