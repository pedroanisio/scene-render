#ifndef SCENE_RENDER_SCENE_H
#define SCENE_RENDER_SCENE_H

#include "scene_render/common.h"
#include "scene_render/timeline.h"

typedef enum {
    SR_BLEND_NORMAL,
    SR_BLEND_ADD,
    SR_BLEND_MULTIPLY,
    SR_BLEND_SCREEN,
    SR_BLEND_OVERLAY,
    SR_BLEND_DIFFERENCE
} SrBlendMode;

typedef enum {
    SR_MODE_STANDARD,
    SR_MODE_EQUIRECTANGULAR,
    SR_MODE_VIEWPORT
} SrRenderMode;

typedef enum {
    SR_ASSET_IMAGE,
    SR_ASSET_VIDEO,
    SR_ASSET_AUDIO,
    SR_ASSET_TEXT,
    SR_ASSET_VECTOR,
    SR_ASSET_MESH
} SrAssetType;

typedef enum {
    SR_NODE_GROUP,
    SR_NODE_MEDIA,
    SR_NODE_SHAPE,
    SR_NODE_PARTICLES
} SrNodeType;

typedef enum { SR_SHAPE_RECT, SR_SHAPE_ELLIPSE, SR_SHAPE_PATH } SrShapeType;
/* Vector fill rule; evenodd is the default to preserve pre-1.2 output. */
typedef enum { SR_FILL_EVENODD, SR_FILL_NONZERO } SrFillRule;
typedef enum { SR_MASK_RECT, SR_MASK_ELLIPSE, SR_MASK_ROUNDED_RECT } SrMaskType;
typedef enum { SR_BODY_NONE, SR_BODY_STATIC, SR_BODY_KINEMATIC, SR_BODY_DYNAMIC } SrBodyType;
typedef enum { SR_COLLIDER_BOX, SR_COLLIDER_CIRCLE } SrColliderType;
typedef enum { SR_MOD_BEND, SR_MOD_TWIST, SR_MOD_WAVE, SR_MOD_SQUASH, SR_MOD_STRETCH } SrModifierType;
typedef enum { SR_LIGHT_AMBIENT, SR_LIGHT_DIRECTIONAL, SR_LIGHT_POINT, SR_LIGHT_SPOT } SrLightType;
typedef enum { SR_OBJECT_SPHERE, SR_OBJECT_BOX, SR_OBJECT_PLANE,
               SR_OBJECT_MESH } SrPrimitive3D;
typedef enum { SR_COLOR_SRGB, SR_COLOR_DISPLAY_P3,
               SR_COLOR_REC2020, SR_COLOR_REC709 } SrColorSpace;
typedef enum { SR_EFFECT_GLOW, SR_EFFECT_BLOOM, SR_EFFECT_BLUR, SR_EFFECT_COLOR_GRADE,
               SR_EFFECT_VIGNETTE, SR_EFFECT_LENS_FLARE } SrEffectType;

/* Float premultiplied RGBA in the project blend space: 4 floats per pixel,
 * row-major and tightly packed. See docs/architecture.md. */
typedef struct SrImage {
    uint32_t width;
    uint32_t height;
    float *px;
} SrImage;

typedef struct {
    double position[3][3];
    double normal[3][3];
} SrMeshTriangle;

typedef struct {
    SrMeshTriangle *triangles;
    size_t triangle_count;
    size_t triangle_capacity;
} SrMesh;

struct SrVideoSource;

typedef struct {
    SrAssetType type;
    char *id;
    char *source;
    uint32_t width;
    uint32_t height;
    uint32_t fps_num;
    uint32_t fps_den;
    double duration;
    SrColorSpace source_color_space;
    size_t source_line;
    SrImage *decoded;
    struct SrVideoSource *video;   /* persistent decoder of a video asset */
    char *text;
    char *font_family;
    char *font_file;
    double text_size;
    SrColor color;
    SrShapeType vector_shape;
    char *vector_path;
    SrFillRule vector_fill_rule;
    SrColor vector_stroke;
    double vector_stroke_width;
    SrMesh *mesh;
    float *audio_pcm;       /* decoded soundtrack: interleaved float at the
                               audioMix rate and channel count */
    uint64_t audio_frames;  /* samples per channel in audio_pcm */
    bool audio_decoded;
} SrAsset;

typedef struct {
    SrAnimValue x;
    SrAnimValue y;
    SrAnimValue z;
    SrAnimValue rotation;
    SrAnimValue rotation_x;
    SrAnimValue rotation_y;
    SrAnimValue scale_x;
    SrAnimValue scale_y;
    SrAnimValue scale_z;
    SrAnimValue anchor_x;
    SrAnimValue anchor_y;
} SrTransform;

typedef struct {
    bool invert;
    SrMaskType type;
    SrAnimValue x;
    SrAnimValue y;
    SrAnimValue width;
    SrAnimValue height;
    SrAnimValue radius;
} SrMask;

typedef struct {
    SrModifierType type;
    SrAnimValue amount;
    SrAnimValue frequency;
    SrAnimValue phase;
    char axis;
} SrModifier;

typedef struct {
    SrBodyType type;
    SrColliderType collider;
    double mass;
    double friction;
    double restitution;
    double linear_damping;
    double angular_damping;
    double velocity_x;
    double velocity_y;
    double angular_velocity;
    double radius;
} SrRigidBody;

typedef struct {
    bool enabled;
    double mass;
    double stiffness;
    double damping;
    double pressure;
} SrSoftBody;

typedef struct {
    bool enabled;
    SrAnimValue x;
    SrAnimValue y;
    SrAnimValue rotation;
} SrPhysicsSample;

typedef struct SrNode {
    SrNodeType type;
    char *id;
    int z;
    size_t order;
    size_t source_line;
    bool visible;
    double start_time;
    double end_time;
    SrAnimValue opacity;
    SrTransform transform;
    SrBlendMode blend;
    SrMask *masks;
    size_t mask_count;
    size_t mask_capacity;
    char *asset_id;
    SrAsset *asset;
    double clip_in;
    double clip_out;
    int64_t loop_count;
    bool reverse;
    double speed;
    double time_stretch;
    SrAnimValue source_time;
    SrShapeType shape;
    double shape_width;
    double shape_height;
    SrColor fill;
    SrColor stroke;
    double stroke_width;
    char *particle_preset;
    SrAnimValue particle_rate;
    SrAnimValue particle_lifetime;
    SrAnimValue particle_speed;
    SrAnimValue particle_spread;
    SrAnimValue particle_size;
    SrColor particle_color;
    SrModifier *modifiers;
    size_t modifier_count;
    size_t modifier_capacity;
    SrRigidBody body;
    SrSoftBody soft_body;
    SrPhysicsSample *physics_samples;
    size_t physics_sample_count;
    struct SrNode **children;
    size_t child_count;
    size_t child_capacity;
} SrNode;

typedef enum { SR_CODEC_H264, SR_CODEC_H265, SR_CODEC_FFV1 } SrCodec;

typedef struct {
    uint32_t width;
    uint32_t height;
    uint32_t fps_num;
    uint32_t fps_den;
    double duration;
    uint64_t seed;
    bool linear_light;
    SrColorSpace working_color_space;
    SrColor background;
    SrRenderMode mode;
} SrProject;

typedef struct {
    char *path;
    SrCodec codec;
    char *pixel_format;
    char *preset;
    int crf;
    uint64_t bitrate;
    char *audio_codec;
    uint64_t audio_bitrate;
    SrColorSpace color_space;
    bool full_range;
    bool spherical_metadata;
    size_t source_line;
} SrOutput;

typedef struct {
    char *id;
    char *asset_id;
    SrAsset *asset;
    double start;
    double clip_in;
    double clip_out;
    int64_t loop_count;
    double volume;
    double pan;
    double fade_in;         /* seconds of linear gain ramp from the start */
    double fade_out;        /* seconds of linear gain ramp to the end */
    double speed;           /* source seconds per output second (> 0) */
    bool reverse;           /* play each loop of [clipIn, clipOut) backward */
    size_t source_line;
} SrAudioTrack;

typedef struct {
    uint32_t sample_rate;
    uint32_t channels;
    SrAudioTrack *tracks;
    size_t track_count;
    size_t track_capacity;
} SrAudioMix;

typedef struct {
    bool enabled;
    uint32_t width;
    uint32_t height;
    char *viewport_camera_id;
    size_t source_line;
} SrScene360;

typedef struct {
    char *id;
    bool active;
    bool orthographic;
    SrAnimValue x, y, z;
    SrAnimValue yaw, pitch, roll, fov;
    double near_plane;
    double far_plane;
    size_t source_line;
} SrCamera;

typedef struct {
    char *id;
    SrColor base_color;
    double metallic;
    double roughness;
    SrColor emissive;
} SrMaterial;

typedef struct {
    char *id;
    SrLightType type;
    SrColor color;
    SrAnimValue intensity;
    SrAnimValue x, y, z;
    SrAnimValue yaw, pitch;
    double range;
    double falloff;
    double spot_angle;
    bool cast_shadow;
} SrLight;

typedef struct {
    char *id;
    SrPrimitive3D primitive;
    char *material_id;
    SrMaterial *material;
    char *mesh_id;
    SrAsset *mesh_asset;
    SrTransform transform;
    double radius;
    bool cast_shadow;
    bool receive_shadow;
    size_t source_line;
} SrObject3D;

typedef struct {
    char *id;
    SrEffectType type;
    bool enabled;
    SrAnimValue intensity;
    SrAnimValue radius;
    double threshold;
    double saturation;
    double contrast;
    double brightness;
    SrColor color;
} SrEffect;

typedef struct {
    char *id;
    char *a_id;
    char *b_id;
    SrNode *a;
    SrNode *b;
    double rest_length;
    double stiffness;
    double damping;
    size_t source_line;
} SrConstraint;

typedef struct {
    char *id;
    bool radial;
    double x;
    double y;
    double force_x;
    double force_y;
    double strength;
    double falloff;
} SrForceField;

typedef struct {
    bool enabled;
    double fixed_step;
    double gravity_x;
    double gravity_y;
    char *cache_path;
    SrConstraint *constraints;
    size_t constraint_count;
    size_t constraint_capacity;
    SrForceField *fields;
    size_t field_count;
    size_t field_capacity;
} SrPhysicsWorld;

typedef struct {
    char *source_path;
    char *base_dir;
    SrProject project;
    SrOutput output;
    SrAsset *assets;
    size_t asset_count;
    size_t asset_capacity;
    SrNode *root;
    size_t next_order;
    SrAudioMix audio;
    SrScene360 scene360;
    SrCamera *cameras;
    size_t camera_count;
    size_t camera_capacity;
    SrMaterial *materials;
    size_t material_count;
    size_t material_capacity;
    SrLight *lights;
    size_t light_count;
    size_t light_capacity;
    SrObject3D *objects3d;
    size_t object3d_count;
    size_t object3d_capacity;
    SrEffect *effects;
    size_t effect_count;
    size_t effect_capacity;
    SrPhysicsWorld physics;
} SrScene;

void sr_scene_init(SrScene *scene);
void sr_scene_free(SrScene *scene);
SrAsset *sr_scene_add_asset(SrScene *scene);
SrAsset *sr_scene_find_asset(SrScene *scene, const char *id);
SrNode *sr_node_create(SrScene *scene, SrNodeType type);
SrStatus sr_node_add_child(SrNode *parent, SrNode *child);
SrStatus sr_node_add_modifier(SrNode *node, SrModifier modifier);
SrStatus sr_node_add_mask(SrNode *node, SrMask mask);
void sr_node_free(SrNode *node);
void sr_node_sort_children(SrNode *node);
SrNode *sr_scene_find_node(SrScene *scene, const char *id);
bool sr_scene_id_exists(const SrScene *scene, const char *id);
SrAnimValue *sr_node_property(SrNode *node, const char *name);
bool sr_blend_parse(const char *text, SrBlendMode *mode);
const char *sr_blend_name(SrBlendMode mode);

#endif
