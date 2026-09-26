#ifndef SCENE_RENDER_XML_INTERNAL_H
#define SCENE_RENDER_XML_INTERNAL_H

#include "scene_render/diagnostics.h"
#include "scene_render/scene.h"
#include "scene_render/property.h"

#include <expat.h>

typedef enum {
    E_SCENE, E_PROJECT, E_OUTPUT, E_ASSETS, E_IMAGE, E_VIDEO, E_AUDIO,
    E_TEXT, E_VECTOR, E_MESH, E_COMPOSITION, E_GROUP, E_LAYER, E_MASK, E_ANIMATE,
    E_KEY, E_AUDIO_MIX, E_AUDIO_TRACK, E_SCENE360, E_CAMERA, E_MATERIALS,
    E_MATERIAL, E_LIGHTS, E_LIGHT, E_EFFECTS, E_EFFECT, E_PHYSICS,
    E_FORCE_FIELD, E_CONSTRAINT, E_PARTICLES, E_RIGID_BODY, E_SOFT_BODY,
    E_DEFORM, E_MODIFIER, E_OBJECT3D, E_POINT, E_STYLES, E_TOKEN,
    E_METADATA, E_META
} ElementKind;

typedef struct {
    ElementKind kind;
    SrNode *node;
    SrAnimValue *anim;
    SrCamera *camera;
    SrLight *light;
    SrEffect *effect;
    SrModifier *modifier;
    SrObject3D *object3d;
    SrMaterial *material;
    SrAudioTrack *audio_track;
    SrMask *mask;
    SrForceField *field;
    SrAnimValue *point;         /* mesh-warp point: [0] = x, [1] = y */
    const SrProperty *property; /* animate/key: immutable registry entry */
    SrAnimColor *color_anim;    /* animate/key: target color track */
    SrCurve curve;
} ParseFrame;

typedef struct {
    XML_Parser parser;
    SrScene *scene;
    SrDiagnostics *diag;
    ParseFrame *stack;
    size_t depth;
    size_t stack_capacity;
    size_t element_depth;       /* open elements, bounded by SR_XML_MAX_DEPTH */
    size_t key_count;
    bool failed;
    bool out_of_memory;         /* the failure was an allocation (exit 8) */
    bool seen_styles;
    bool seen_metadata;
    bool seen_project;
    bool seen_output;
    bool seen_assets;
    bool seen_composition;
    bool seen_audio_mix;
    bool seen_scene360;
    bool seen_materials;
    bool seen_lights;
    bool seen_effects;
    bool seen_physics;
} ParseContext;

size_t sr_xml_line(ParseContext *ctx);
void sr_xml_start_metadata(ParseContext *ctx, const XML_Char **attrs);
void sr_xml_start_meta(ParseContext *ctx, const XML_Char **attrs);
void sr_xml_fail(ParseContext *ctx, const char *element, const char *attribute,
                 const char *message);
void sr_xml_fail_at(ParseContext *ctx, size_t line, const char *element,
                    const char *attribute, const char *message);
const char *sr_xml_attr(const XML_Char **attrs, const char *name);
bool sr_xml_attrs_allowed(ParseContext *ctx, const char *element,
                          const XML_Char **attrs, const char *const *allowed,
                          size_t count);
const char *sr_xml_required(ParseContext *ctx, const char *element,
                            const XML_Char **attrs, const char *name);
ParseFrame *sr_xml_parent(ParseContext *ctx);
void sr_xml_push(ParseContext *ctx, ParseFrame frame, const char *element);
bool sr_xml_resolve_scene(ParseContext *ctx);
bool sr_xml_parse_double_attr(ParseContext *ctx, const char *element,
                              const XML_Char **attrs, const char *name,
                              double *target);
bool sr_xml_parse_node_common(ParseContext *ctx, const char *element,
                              const XML_Char **attrs, SrNode *node);

bool sr_xml_parse_color(ParseContext *ctx, const char *element,
                         const char *attribute, const char *text, SrColor *color);
void sr_xml_start_token(ParseContext *ctx, const XML_Char **attrs);

void sr_xml_start_project(ParseContext *ctx, const XML_Char **attrs);
void sr_xml_start_output(ParseContext *ctx, const XML_Char **attrs);
void sr_xml_start_image(ParseContext *ctx, const XML_Char **attrs);
void sr_xml_start_video(ParseContext *ctx, const XML_Char **attrs);
void sr_xml_start_audio(ParseContext *ctx, const XML_Char **attrs);
void sr_xml_start_audio_mix(ParseContext *ctx, const XML_Char **attrs);
void sr_xml_start_audio_track(ParseContext *ctx, const XML_Char **attrs);
void sr_xml_start_scene360(ParseContext *ctx, const XML_Char **attrs);
void sr_xml_start_camera(ParseContext *ctx, const XML_Char **attrs);
void sr_xml_start_text(ParseContext *ctx, const XML_Char **attrs);
void sr_xml_start_vector(ParseContext *ctx, const XML_Char **attrs);
void sr_xml_start_mesh(ParseContext *ctx, const XML_Char **attrs);
void sr_xml_start_material(ParseContext *ctx, const XML_Char **attrs);
void sr_xml_start_light(ParseContext *ctx, const XML_Char **attrs);
void sr_xml_start_effect(ParseContext *ctx, const XML_Char **attrs);
void sr_xml_start_object3d(ParseContext *ctx, const XML_Char **attrs);
void sr_xml_start_physics(ParseContext *ctx, const XML_Char **attrs);
void sr_xml_start_force_field(ParseContext *ctx, const XML_Char **attrs);
void sr_xml_start_constraint(ParseContext *ctx, const XML_Char **attrs);
void sr_xml_start_rigid_body(ParseContext *ctx, const XML_Char **attrs);
void sr_xml_start_soft_body(ParseContext *ctx, const XML_Char **attrs);
void sr_xml_start_deform(ParseContext *ctx, const XML_Char **attrs);
void sr_xml_start_modifier(ParseContext *ctx, const XML_Char **attrs);
void sr_xml_start_point(ParseContext *ctx, const XML_Char **attrs);
void sr_xml_start_node(ParseContext *ctx, const char *name,
                       const XML_Char **attrs, SrNodeType type);
void sr_xml_start_mask(ParseContext *ctx, const XML_Char **attrs);
void sr_xml_start_animate(ParseContext *ctx, const XML_Char **attrs);
void sr_xml_start_key(ParseContext *ctx, const XML_Char **attrs);
bool sr_xml_animation_options(ParseContext *ctx, const XML_Char **attrs,
                               ParseFrame *host, SrAnimValue *value,
                               SrAnimColor *color);
bool sr_xml_key_options(ParseContext *ctx, const XML_Char **attrs, SrKeyframe *key);
bool sr_xml_length_attr(ParseContext *ctx, const char *element,
                         const XML_Char **attrs, const char *attribute,
                         double *value, SrLengthUnit *unit, bool positive);
bool sr_xml_anim_length_attr(ParseContext *ctx, const char *element,
                              const XML_Char **attrs, const char *attribute,
                              SrAnimValue *value, bool positive);
bool sr_xml_finish_animation(ParseContext *ctx, ParseFrame *frame);

#define SR_XML_FAIL_RETURN(context, element, attribute, message)               \
    do {                                                                       \
        sr_xml_fail((context), (element), (attribute), (message));             \
        return;                                                                \
    } while (0)

#endif
