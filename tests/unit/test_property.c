/* SPDX-License-Identifier: Apache-2.0 */
#include "harness.h"
#include "scene_render/property.h"
#include "scene_render/xml.h"

#include <float.h>
#include <math.h>

static void node_hosts(sr_test_ctx *t) {
    SrNode node = {0};
    const SrNodeType types[] = {SR_NODE_GROUP, SR_NODE_MEDIA, SR_NODE_SHAPE,
                                SR_NODE_PARTICLES};
    const SrPropertyHost hosts[] = {SR_PROPERTY_GROUP, SR_PROPERTY_LAYER,
                                    SR_PROPERTY_SHAPE, SR_PROPERTY_PARTICLES};
    for (size_t i = 0; i < 4; ++i) {
        node.type = types[i];
        CHECK_INT(t, sr_property_node_host(&node), hosts[i]);
        CHECK(t, sr_node_property(&node, "position.x") == &node.transform.x);
        CHECK(t, sr_node_property(&node, "source.time") == &node.source_time);
        CHECK(t, sr_node_property(&node, "depth") == &node.transform.z);
        CHECK(t, sr_node_property(&node, "rate") ==
              (node.type == SR_NODE_PARTICLES ? &node.particle_rate : NULL));
        CHECK(t, sr_node_color_property(&node, "fill") ==
              (node.type == SR_NODE_SHAPE ? &node.fill : NULL));
        CHECK(t, sr_node_property(&node, "fill") == NULL);
        CHECK(t, sr_node_color_property(&node, "opacity") == NULL);
    }
    CHECK(t, sr_node_property(NULL, "opacity") == NULL);
    CHECK(t, sr_node_property(&node, NULL) == NULL);
    CHECK(t, sr_node_color_property(NULL, "fill") == NULL);
    CHECK_INT(t, sr_property_node_host(NULL), SR_PROPERTY_HOST_COUNT);
    node.type = (SrNodeType)999;
    CHECK_INT(t, sr_property_node_host(&node), SR_PROPERTY_HOST_COUNT);
}

static void other_hosts(sr_test_ctx *t) {
    SrMask mask = {0};
    SrAnimValue point[2] = {0};
    SrCamera camera = {0};
    SrLight light = {0};
    SrEffect effect = {0};
    SrForceField field = {0};
    SrModifier modifier = {0};
    SrObject3D object = {0};
    struct {
        SrPropertyHost host;
        const char *name;
        void *object, *expected;
        SrPropertyType type;
    } cases[] = {
        {SR_PROPERTY_MASK, "radius", &mask, &mask.radius, SR_PROPERTY_NUMBER},
        {SR_PROPERTY_POINT, "y", point, &point[1], SR_PROPERTY_NUMBER},
        {SR_PROPERTY_CAMERA, "focusDistance", &camera, &camera.focus_distance,
         SR_PROPERTY_NUMBER},
        {SR_PROPERTY_LIGHT, "color", &light, &light.color, SR_PROPERTY_COLOR},
        {SR_PROPERTY_EFFECT, "offsetY", &effect, &effect.offset_y, SR_PROPERTY_NUMBER},
        {SR_PROPERTY_FIELD, "forceX", &field, &field.force_x, SR_PROPERTY_NUMBER},
        {SR_PROPERTY_MODIFIER, "phase", &modifier, &modifier.phase, SR_PROPERTY_NUMBER},
        {SR_PROPERTY_OBJECT3D, "scale.z", &object, &object.transform.scale_z,
         SR_PROPERTY_NUMBER},
    };
    for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); ++i) {
        const SrProperty *p = sr_property_find(cases[i].host, cases[i].name);
        CHECK(t, p != NULL);
        if (!p) continue;
        CHECK_INT(t, p->type, cases[i].type);
        CHECK(t, sr_property_target(p, cases[i].object) == cases[i].expected);
    }
    CHECK(t, sr_property_find(SR_PROPERTY_MASK, "position.x") == NULL);
    CHECK(t, sr_property_find(SR_PROPERTY_POINT, "radius") == NULL);
    CHECK(t, sr_property_find(SR_PROPERTY_CAMERA, "intensity") == NULL);
    CHECK(t, sr_property_find(SR_PROPERTY_FIELD, "color") == NULL);
}

static void eligibility_and_activation(sr_test_ctx *t) {
    SrCamera camera = {0};
    const SrProperty *zoom = sr_property_find(SR_PROPERTY_CAMERA, "zoom");
    CHECK(t, sr_property_target(zoom, &camera) == NULL);
    camera.zoom_set = true;
    CHECK(t, sr_property_target(zoom, &camera) == &camera.zoom);
    SrNode node = {.type = SR_NODE_PARTICLES};
    const SrProperty *end = sr_property_find(SR_PROPERTY_PARTICLES, "colorEnd");
    CHECK(t, sr_property_target(end, &node) == &node.particle_color_end);
    CHECK(t, !node.particle_color_end_set);
    sr_property_activate(end, &node);
    CHECK(t, node.particle_color_end_set);
    node.particle_color_end_set = false;
    CHECK(t, sr_node_color_property(&node, "colorEnd") == &node.particle_color_end);
    CHECK(t, node.particle_color_end_set);
    CHECK(t, sr_property_target(NULL, &node) == NULL);
    CHECK(t, sr_property_target(end, NULL) == NULL);
    sr_property_activate(NULL, &node);
    sr_property_activate(end, NULL);
    const SrProperty *depth = sr_property_find(SR_PROPERTY_LAYER, "depth");
    CHECK(t, depth && (depth->flags & SR_PROPERTY_DEPTH_CARD));
    CHECK(t, !node.card);
}

static void key_bounds(sr_test_ctx *t) {
    const SrProperty *light = sr_property_find(SR_PROPERTY_LIGHT, "intensity");
    const SrProperty *radius = sr_property_find(SR_PROPERTY_EFFECT, "radius");
    const SrProperty *offset = sr_property_find(SR_PROPERTY_EFFECT, "offsetX");
    CHECK(t, sr_property_key_valid(light, SR_MAX_LIGHT_INTENSITY));
    CHECK(t, !sr_property_key_valid(light, SR_MAX_LIGHT_INTENSITY + 1));
    CHECK(t, sr_property_key_valid(light, -DBL_MAX));
    CHECK(t, sr_property_key_valid(radius, SR_MAX_EFFECT_RADIUS));
    CHECK(t, !sr_property_key_valid(radius, SR_MAX_EFFECT_RADIUS + 1));
    CHECK(t, sr_property_key_valid(offset, SR_MAX_EFFECT_OFFSET));
    CHECK(t, sr_property_key_valid(offset, -SR_MAX_EFFECT_OFFSET));
    CHECK(t, !sr_property_key_valid(offset, SR_MAX_EFFECT_OFFSET + 1));
    CHECK(t, !sr_property_key_valid(offset, -SR_MAX_EFFECT_OFFSET - 1));
    CHECK(t, !sr_property_key_valid(light, NAN));
    CHECK(t, !sr_property_key_valid(light, INFINITY));
    CHECK(t, !sr_property_key_valid(NULL, 0));
}

static void registry_contract(sr_test_ctx *t) {
    CHECK(t, sr_property_count() > 60);
    for (size_t i = 0; i < sr_property_count(); ++i) {
        const SrProperty *p = sr_property_at(i);
        CHECK(t, p != NULL);
        if (!p) continue;
        CHECK(t, p->hosts != 0);
        CHECK(t, p->minimum <= p->maximum);
        for (int host = 0; host < SR_PROPERTY_HOST_COUNT; ++host) {
            if (!(p->hosts & (UINT32_C(1) << host))) continue;
            CHECK(t, sr_property_find((SrPropertyHost)host, p->name) == p);
        }
        for (size_t j = 0; j < i; ++j) {
            const SrProperty *previous = sr_property_at(j);
            CHECK(t, !(previous->hosts & p->hosts) || strcmp(previous->name, p->name));
        }
    }
    CHECK(t, sr_property_at(sr_property_count()) == NULL);
    CHECK(t, sr_property_find(SR_PROPERTY_HOST_COUNT, "x") == NULL);
    CHECK(t, sr_property_find((SrPropertyHost)-1, "x") == NULL);
    CHECK(t, sr_property_find(SR_PROPERTY_SHAPE, "missing") == NULL);
    CHECK(t, sr_property_find(SR_PROPERTY_SHAPE, NULL) == NULL);
}

static void legacy_modifier_owner(sr_test_ctx *t) {
    const char *path = sr_test_tmp_path("property-owner.xml");
    FILE *file = fopen(path, "w");
    CHECK(t, file != NULL);
    if (!file) return;
    fputs("<scene version=\"1.0\"><project width=\"16\" height=\"16\" "
          "fps=\"1\" duration=\"1\"/>"
          "<composition><shape id=\"s\" shape=\"rect\" width=\"8\" height=\"8\">"
          "<deform><modifier type=\"bend\"><animate property=\"position.x\">"
          "<key time=\"0\" value=\"3\"/></animate></modifier></deform>"
          "</shape></composition></scene>", file);
    fclose(file);
    SrScene scene;
    SrDiagnostics diag;
    sr_diag_init(&diag, NULL, NULL);
    SrStatus status = sr_scene_load_xml(path, &scene, &diag);
    CHECK_INT(t, status, SR_OK);
    if (status == SR_OK) {
        SrNode *node = sr_scene_find_node(&scene, "s");
        CHECK(t, node != NULL);
        if (node) CHECK_NEAR(t, sr_anim_eval(&node->transform.x, 0), 3, 1e-12);
        sr_scene_free(&scene);
    }
}

const sr_test_case sr_tests_property[] = {
    {"node_hosts", node_hosts},
    {"other_hosts", other_hosts},
    {"eligibility_and_activation", eligibility_and_activation},
    {"key_bounds", key_bounds},
    {"registry_contract", registry_contract},
    {"legacy_modifier_owner", legacy_modifier_owner},
    {NULL, NULL},
};
