/* SPDX-License-Identifier: Apache-2.0 */
#include "harness.h"
#include "scene_render/property.h"
#include "scene_render/xml.h"
#include "scene_text.h"

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
    SrMaterial material = {0};
    SrAudioTrack audio = {0};
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
        {SR_PROPERTY_MATERIAL, "baseColor", &material, &material.base_color,
         SR_PROPERTY_COLOR},
        {SR_PROPERTY_MATERIAL, "emissive", &material, &material.emissive,
         SR_PROPERTY_COLOR},
        {SR_PROPERTY_MATERIAL, "metallic", &material, &material.metallic,
         SR_PROPERTY_NUMBER},
        {SR_PROPERTY_MATERIAL, "roughness", &material, &material.roughness,
         SR_PROPERTY_NUMBER},
        {SR_PROPERTY_AUDIO_TRACK, "volume", &audio, &audio.volume, SR_PROPERTY_NUMBER},
        {SR_PROPERTY_AUDIO_TRACK, "pan", &audio, &audio.pan, SR_PROPERTY_NUMBER},
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
    CHECK(t, sr_property_find(SR_PROPERTY_MATERIAL, "volume") == NULL);
    CHECK(t, sr_property_find(SR_PROPERTY_AUDIO_TRACK, "metallic") == NULL);
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
    const SrProperty *metallic = sr_property_find(SR_PROPERTY_MATERIAL, "metallic");
    const SrProperty *pan = sr_property_find(SR_PROPERTY_AUDIO_TRACK, "pan");
    CHECK(t, sr_property_key_valid(metallic, 0) && sr_property_key_valid(metallic, 1));
    CHECK(t, !sr_property_key_valid(metallic, -0.1));
    CHECK(t, !sr_property_key_valid(metallic, 1.1));
    CHECK(t, sr_property_key_valid(pan, -1) && sr_property_key_valid(pan, 1));
    CHECK(t, !sr_property_key_valid(pan, -1.1));
    CHECK(t, !sr_property_key_valid(pan, 1.1));
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

static SrStatus load_animation_host(sr_test_ctx *t, bool audio, const char *version,
                                    const char *extra, const char *children,
                                    SrScene *scene, char **message) {
    char xml[4096];
    if (audio) {
        snprintf(xml, sizeof(xml), "<scene version=\"%s\">"
            "<project width=\"16\" height=\"16\" duration=\"2\" fps=\"12\"/>"
            "<assets><audio id=\"tone\" src=\"unused.wav\"/></assets><composition/>"
            "<audioMix><audioTrack id=\"a\" asset=\"tone\" %s>%s"
            "</audioTrack></audioMix></scene>", version, extra, children);
    } else {
        snprintf(xml, sizeof(xml), "<scene version=\"%s\">"
            "<project width=\"16\" height=\"16\" duration=\"2\" fps=\"12\"/>"
            "<materials><material id=\"m\" %s>%s</material></materials>"
            "<composition/></scene>", version, extra, children);
    }
    return st_load(t, "animation-host.xml", xml, scene, message);
}

static void animation_host_rejections(sr_test_ctx *t) {
    const struct {
        bool audio;
        const char *version, *extra, *children, *diagnostic;
    } cases[] = {
        {false, "1.0", "", "<animate property=\"roughness\">"
         "<key time=\"0\" value=\"0.5\"/></animate>", "requires version=\"1.1\""},
        {true, "1.0", "", "<animate property=\"volume\">"
         "<key time=\"0\" value=\"0.5\"/></animate>", "requires version=\"1.1\""},
        {false, "1.1", "", "<animate property=\"volume\">"
         "<key time=\"0\" value=\"0.5\"/></animate>", "not animatable"},
        {true, "1.1", "", "<animate property=\"metallic\">"
         "<key time=\"0\" value=\"0.5\"/></animate>", "not animatable"},
        {false, "1.1", "", "<animate property=\"metallic\">"
         "<key time=\"0\" value=\"1.1\"/></animate>", "[0,1]"},
        {false, "1.1", "", "<animate property=\"roughness\">"
         "<key time=\"0\" value=\"-0.1\"/></animate>", "[0,1]"},
        {true, "1.1", "", "<animate property=\"volume\">"
         "<key time=\"0\" value=\"1.1\"/></animate>", "[0,1]"},
        {true, "1.1", "", "<animate property=\"pan\">"
         "<key time=\"0\" value=\"-1.1\"/></animate>", "[-1,1]"},
        {false, "1.1", "", "<animate property=\"baseColor\">"
         "<key time=\"0\" value=\"#102030\"/></animate>"
         "<animate property=\"baseColor\"><key time=\"0\" value=\"#304050\"/>"
         "</animate>", "already has an animation"},
        {true, "1.1", "", "<animate property=\"pan\">"
         "<key time=\"0\" value=\"0\"/></animate>"
         "<animate property=\"pan\"><key time=\"0\" value=\"1\"/>"
         "</animate>", "already has an animation"},
        {true, "1.1", "start=\"2\"", "<animate property=\"volume\" "
         "timeBase=\"normalized\"><key time=\"0\" value=\"0\"/></animate>",
         "finite positive host span"},
        {false, "1.1", "", "<animate property=\"emissive\">"
         "<key time=\"0\" value=\"1,0,0,2\"/></animate>", "expected a color"},
        {true, "1.1", "", "<animate property=\"volume\"/>", "error:"},
    };
    for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); ++i) {
        SrScene scene;
        char *message = NULL;
        SrStatus status = load_animation_host(t, cases[i].audio, cases[i].version,
            cases[i].extra, cases[i].children, &scene, &message);
        CHECK_INT(t, status, SR_ERR_XML);
        CHECK_CONTAINS(t, message, cases[i].diagnostic);
        free(message);
        if (status == SR_OK) sr_scene_free(&scene);
    }
}

static void animation_host_parser_mutations(sr_test_ctx *t) {
    const uint32_t seeds[] = {7, 4711, 0x51a8a234};
    const char *properties[] = {"baseColor", "emissive", "metallic", "roughness",
                                "volume", "pan"};
    for (size_t seed = 0; seed < 3; ++seed) {
        uint32_t state = seeds[seed];
        for (size_t trial = 0; trial < 72; ++trial) {
            state = state * UINT32_C(1664525) + UINT32_C(1013904223);
            size_t host = trial % 6;
            char children[512];
            snprintf(children, sizeof(children), "<animate property=\"%s\" "
                "timeBase=\"normalized\" defaultInterpolation=\"%s\">"
                "<key time=\"0\" value=\"%s\"/><key time=\"1\" value=\"%s\"/>"
                "</animate>", properties[host],
                sr_curve_name((SrCurve)(SR_CURVE_SINE_IN + state % 30)),
                host < 2 ? "#102030" : "0.2", host < 2 ? "#A0B0C0" : "0.8");
            size_t mutation = (trial / 6) % 3;
            if (mutation == 1) children[strlen(children) - 1] = '\0';
            if (mutation == 2) children[state % strlen(children)] = '<';
            SrScene scene;
            SrStatus status = load_animation_host(t, host >= 4, "1.1", "",
                                                   children, &scene, NULL);
            CHECK(t, status == SR_OK || status == SR_ERR_XML);
            if (!mutation) CHECK_INT(t, status, SR_OK);
            if (status == SR_OK) sr_scene_free(&scene);
        }
    }
}

static void animation_host_count_limits(sr_test_ctx *t) {
    for (int audio = 0; audio < 2; ++audio) {
        unsigned limit = audio ? SR_MAX_AUDIO_TRACKS : SR_MAX_MATERIALS;
        for (unsigned excess = 0; excess < 2; ++excess) {
            size_t capacity = 1024 + (limit + 1) * 80;
            char *xml = malloc(capacity);
            if (!xml) { SR_FAIL(t, "host limit fixture allocation"); return; }
            size_t used = (size_t)snprintf(xml, capacity,
                "<scene version=\"1.1\"><project width=\"16\" height=\"16\" "
                "duration=\"1\" fps=\"1\"/>%s", audio ?
                "<assets><audio id=\"tone\" src=\"unused.wav\"/></assets>"
                "<composition/><audioMix>" : "<materials>");
            for (unsigned i = 0; i < limit + excess; ++i) {
                if (audio)
                    used += (size_t)snprintf(xml + used, capacity - used,
                        "<audioTrack id=\"a%u\" asset=\"tone\"/>", i);
                else
                    used += (size_t)snprintf(xml + used, capacity - used,
                        "<material id=\"m%u\"/>", i);
            }
            snprintf(xml + used, capacity - used, "%s</scene>", audio ?
                     "</audioMix>" : "</materials><composition/>");
            SrScene scene;
            char *message = NULL;
            SrStatus status = st_load(t, "host-limit.xml", xml, &scene, &message);
            CHECK_INT(t, status, excess ? SR_ERR_XML : SR_OK);
            if (excess) {
                CHECK_CONTAINS(t, message, "count exceeds 4096 limit");
                CHECK_CONTAINS(t, message, audio ? "<audioTrack>" : "<material>");
            }
            free(message);
            free(xml);
            if (status == SR_OK) sr_scene_free(&scene);
        }
    }
}

const sr_test_case sr_tests_property[] = {
    {"animation_host_rejections", animation_host_rejections},
    {"animation_host_parser_mutations", animation_host_parser_mutations},
    {"animation_host_count_limits", animation_host_count_limits},
    {"node_hosts", node_hosts},
    {"other_hosts", other_hosts},
    {"eligibility_and_activation", eligibility_and_activation},
    {"key_bounds", key_bounds},
    {"registry_contract", registry_contract},
    {"legacy_modifier_owner", legacy_modifier_owner},
    {NULL, NULL},
};
