#include "xml_internal.h"
#include "scene_render/vector_path.h"

#include <ctype.h>
#include <stdlib.h>
#include <string.h>

static bool decimal(ParseContext *ctx, const char *element,
                    const XML_Char **attrs, const char *name, double *target) {
    const char *text = sr_xml_attr(attrs, name);
    if (!text) return true;
    if (!sr_parse_double(text, target)) {
        sr_xml_fail(ctx, element, name, "expected a finite decimal number");
        return false;
    }
    return true;
}

static bool unique_asset(ParseContext *ctx, const char *element,
                         const char *id) {
    if (!sr_id_valid(id)) {
        sr_xml_fail(ctx, element, "id", "expected an XML-compatible identifier");
        return false;
    }
    if (sr_scene_id_exists(ctx->scene, id)) {
        sr_xml_fail(ctx, element, "id", "asset id must be unique");
        return false;
    }
    return true;
}

static bool font_name_valid(const char *name) {
    if (!name || !*name) return false;
    for (const unsigned char *p=(const unsigned char *)name;*p;++p)
        if (!isalnum(*p) && *p!=' ' && *p!='_' && *p!='-' && *p!='.')
            return false;
    return true;
}

static bool keyword(const char *text, const char *const *names, size_t count,
                    int *value) {
    for (size_t i = 0; i < count; ++i)
        if (strcmp(text, names[i]) == 0) {
            *value = (int)i;
            return true;
        }
    return false;
}

/* align, lineHeight, letterSpacing, direction, language, verticalAlign. */
static void text_layout_attributes(ParseContext *ctx, const XML_Char **attrs,
                                   SrAsset *asset) {
    static const char *const aligns[] = {"start", "center", "end", "justify"};
    static const char *const directions[] = {"auto", "ltr", "rtl"};
    static const char *const valigns[] = {"top", "middle", "bottom"};
    int value = 0;
    const char *text = sr_xml_attr(attrs, "align");
    if (text && !keyword(text, aligns, 4, &value))
        SR_XML_FAIL_RETURN(ctx, "text", "align", "expected start, center, end or justify");
    asset->text_align = (SrTextAlign)value;
    value = 0;
    text = sr_xml_attr(attrs, "direction");
    if (text && !keyword(text, directions, 3, &value))
        SR_XML_FAIL_RETURN(ctx, "text", "direction", "expected auto, ltr or rtl");
    asset->text_direction = (SrTextDirection)value;
    value = 0;
    text = sr_xml_attr(attrs, "verticalAlign");
    if (text && !keyword(text, valigns, 3, &value))
        SR_XML_FAIL_RETURN(ctx, "text", "verticalAlign", "expected top, middle or bottom");
    asset->text_valign = (SrTextVAlign)value;
    asset->text_line_height = 1.2;
    if (!decimal(ctx, "text", attrs, "lineHeight", &asset->text_line_height)) return;
    if (asset->text_line_height <= 0.0)
        SR_XML_FAIL_RETURN(ctx, "text", "lineHeight", "expected a positive multiple of size");
    if (!decimal(ctx, "text", attrs, "letterSpacing", &asset->text_letter_spacing)) return;
    text = sr_xml_attr(attrs, "language");
    if (text) {
        bool valid = *text && isalpha((unsigned char)*text);
        for (const unsigned char *p = (const unsigned char *)text; valid && *p; ++p)
            valid = isalnum(*p) || *p == '-';
        if (!valid)
            SR_XML_FAIL_RETURN(ctx, "text", "language", "expected a BCP 47 language tag");
        asset->text_language = sr_strdup(text);
        if (!asset->text_language) SR_XML_FAIL_RETURN(ctx, "text", "language", "out of memory");
    }
}

void sr_xml_start_text(ParseContext *ctx, const XML_Char **attrs) {
    const char *const allowed[] = {"id", "text", "width", "height", "size",
                                    "color", "font", "fontFile", "align",
                                    "lineHeight", "letterSpacing", "direction",
                                    "language", "verticalAlign"};
    if (!sr_xml_attrs_allowed(ctx, "text", attrs, allowed, 14)) return;
    const char *id = sr_xml_required(ctx, "text", attrs, "id");
    const char *content = sr_xml_required(ctx, "text", attrs, "text");
    const char *width = sr_xml_required(ctx, "text", attrs, "width");
    const char *height = sr_xml_required(ctx, "text", attrs, "height");
    const char *size = sr_xml_required(ctx, "text", attrs, "size");
    if (ctx->failed || !unique_asset(ctx, "text", id)) return;
    SrAsset *asset = sr_scene_add_asset(ctx->scene);
    if (!asset) SR_XML_FAIL_RETURN(ctx, "text", NULL, "out of memory");
    asset->type = SR_ASSET_TEXT;
    asset->source_line = sr_xml_line(ctx);
    asset->id = sr_strdup(id); asset->text = sr_strdup(content);
    if (!asset->id || !asset->text) SR_XML_FAIL_RETURN(ctx, "text", NULL, "out of memory");
    if (!sr_parse_u32(width, &asset->width) || !asset->width ||
        !sr_parse_u32(height, &asset->height) || !asset->height ||
        !sr_parse_double(size, &asset->text_size) || asset->text_size <= 0.0)
        SR_XML_FAIL_RETURN(ctx, "text", "width/height/size", "expected positive values");
    const char *color = sr_xml_attr(attrs, "color");
    if (color && !sr_parse_color(color, &asset->color))
        SR_XML_FAIL_RETURN(ctx, "text", "color", "invalid color");
    const char *font=sr_xml_attr(attrs,"font");
    if(font&&!font_name_valid(font))
        SR_XML_FAIL_RETURN(ctx,"text","font","font family contains unsupported characters");
    asset->font_family=sr_strdup(font?font:"Sans");
    const char *font_file=sr_xml_attr(attrs,"fontFile");
    if(font_file&&!*font_file)
        SR_XML_FAIL_RETURN(ctx,"text","fontFile","font file path cannot be empty");
    if(font_file)asset->font_file=sr_strdup(font_file);
    if(!asset->font_family||(font_file&&!asset->font_file))
        SR_XML_FAIL_RETURN(ctx,"text",font_file?"fontFile":"font","out of memory");
    text_layout_attributes(ctx, attrs, asset);
}

void sr_xml_start_vector(ParseContext *ctx, const XML_Char **attrs) {
    const char *const allowed[] = {"id", "shape", "path", "width", "height", "fill",
                                    "fillRule", "stroke", "strokeWidth"};
    if (!sr_xml_attrs_allowed(ctx, "vector", attrs, allowed, 9)) return;
    const char *id = sr_xml_required(ctx, "vector", attrs, "id");
    const char *shape = sr_xml_required(ctx, "vector", attrs, "shape");
    const char *width = sr_xml_required(ctx, "vector", attrs, "width");
    const char *height = sr_xml_required(ctx, "vector", attrs, "height");
    if (ctx->failed || !unique_asset(ctx, "vector", id)) return;
    SrAsset *asset = sr_scene_add_asset(ctx->scene);
    if (!asset) SR_XML_FAIL_RETURN(ctx, "vector", NULL, "out of memory");
    asset->type = SR_ASSET_VECTOR;
    asset->source_line = sr_xml_line(ctx);
    asset->id = sr_strdup(id);
    if (!asset->id) SR_XML_FAIL_RETURN(ctx, "vector", NULL, "out of memory");
    if (strcmp(shape, "rect") == 0) asset->vector_shape = SR_SHAPE_RECT;
    else if (strcmp(shape, "ellipse") == 0) asset->vector_shape = SR_SHAPE_ELLIPSE;
    else if (strcmp(shape,"path")==0) asset->vector_shape=SR_SHAPE_PATH;
    else SR_XML_FAIL_RETURN(ctx,"vector","shape","expected rect, ellipse, or path");
    const char *path=sr_xml_attr(attrs,"path");
    if(asset->vector_shape==SR_SHAPE_PATH){
        if(!path)SR_XML_FAIL_RETURN(ctx,"vector","path","path is required for shape=path");
        if(!sr_vector_path_valid(path))SR_XML_FAIL_RETURN(ctx,"vector","path",
            "invalid path; supported commands are M/L/H/V/C/Q/Z");
        asset->vector_path=sr_strdup(path);
        if(!asset->vector_path)SR_XML_FAIL_RETURN(ctx,"vector","path","out of memory");
    }else if(path)SR_XML_FAIL_RETURN(ctx,"vector","path","path requires shape=path");
    if (!sr_parse_u32(width, &asset->width) || !asset->width ||
        !sr_parse_u32(height, &asset->height) || !asset->height)
        SR_XML_FAIL_RETURN(ctx, "vector", "width/height", "expected positive integers");
    const char *fill = sr_xml_attr(attrs, "fill");
    if (fill && !sr_parse_color(fill, &asset->color))
        SR_XML_FAIL_RETURN(ctx, "vector", "fill", "invalid color");
    const char *rule = sr_xml_attr(attrs, "fillRule");
    if (rule) {
        if (strcmp(rule, "evenodd") == 0) asset->vector_fill_rule = SR_FILL_EVENODD;
        else if (strcmp(rule, "nonzero") == 0) asset->vector_fill_rule = SR_FILL_NONZERO;
        else SR_XML_FAIL_RETURN(ctx, "vector", "fillRule", "expected nonzero or evenodd");
    }
    const char *stroke = sr_xml_attr(attrs, "stroke");
    if (stroke && !sr_parse_color(stroke, &asset->vector_stroke))
        SR_XML_FAIL_RETURN(ctx, "vector", "stroke", "invalid color");
    if (!decimal(ctx, "vector", attrs, "strokeWidth", &asset->vector_stroke_width))
        return;
    if (asset->vector_stroke_width < 0.0)
        SR_XML_FAIL_RETURN(ctx, "vector", "strokeWidth", "expected a non-negative width");
}

void sr_xml_start_mesh(ParseContext *ctx, const XML_Char **attrs) {
    const char *const allowed[]={"id","src"};
    if(!sr_xml_attrs_allowed(ctx,"mesh",attrs,allowed,2))return;
    const char *id=sr_xml_required(ctx,"mesh",attrs,"id");
    const char *source=sr_xml_required(ctx,"mesh",attrs,"src");
    if(ctx->failed||!unique_asset(ctx,"mesh",id))return;
    SrAsset *asset=sr_scene_add_asset(ctx->scene);
    if(!asset)SR_XML_FAIL_RETURN(ctx,"mesh",NULL,"out of memory");
    asset->type=SR_ASSET_MESH;asset->source_line=sr_xml_line(ctx);
    asset->id=sr_strdup(id);asset->source=sr_strdup(source);
    if(!asset->id||!asset->source)SR_XML_FAIL_RETURN(ctx,"mesh",NULL,"out of memory");
}

void sr_xml_start_material(ParseContext *ctx, const XML_Char **attrs) {
    const char *const allowed[] = {"id", "baseColor", "metallic", "roughness", "emissive"};
    if (!sr_xml_attrs_allowed(ctx, "material", attrs, allowed, 5)) return;
    const char *id = sr_xml_required(ctx, "material", attrs, "id");
    if (!id) return;
    if (!sr_id_valid(id))
        SR_XML_FAIL_RETURN(ctx, "material", "id", "invalid identifier");
    if (sr_scene_id_exists(ctx->scene, id))
        SR_XML_FAIL_RETURN(ctx, "material", "id", "id must be globally unique");
    if (ctx->scene->material_count == ctx->scene->material_capacity) {
        size_t capacity = ctx->scene->material_capacity ? ctx->scene->material_capacity * 2 : 4;
        SrMaterial *items = sr_realloc(ctx->scene->materials, capacity * sizeof(*items));
        if (!items) SR_XML_FAIL_RETURN(ctx, "material", NULL, "out of memory");
        memset(items + ctx->scene->material_capacity, 0,
               (capacity - ctx->scene->material_capacity) * sizeof(*items));
        ctx->scene->materials = items; ctx->scene->material_capacity = capacity;
    }
    SrMaterial *material = &ctx->scene->materials[ctx->scene->material_count++];
    material->id = sr_strdup(id);
    material->base_color = (SrColor){1,1,1,1};
    material->roughness = 0.5;
    if (!material->id) SR_XML_FAIL_RETURN(ctx, "material", NULL, "out of memory");
    const char *value = sr_xml_attr(attrs, "baseColor");
    if (value && !sr_parse_color(value, &material->base_color))
        SR_XML_FAIL_RETURN(ctx, "material", "baseColor", "invalid color");
    value = sr_xml_attr(attrs, "emissive");
    if (value && !sr_parse_color(value, &material->emissive))
        SR_XML_FAIL_RETURN(ctx, "material", "emissive", "invalid color");
    if (!decimal(ctx, "material", attrs, "metallic", &material->metallic) ||
        !decimal(ctx, "material", attrs, "roughness", &material->roughness)) return;
    if (material->metallic < 0 || material->metallic > 1 ||
        material->roughness < 0 || material->roughness > 1)
        SR_XML_FAIL_RETURN(ctx, "material", "metallic/roughness", "expected values in [0,1]");
}

void sr_xml_start_light(ParseContext *ctx, const XML_Char **attrs) {
    const char *const allowed[] = {"id","type","color","intensity","x","y","z",
        "yaw","pitch","range","falloff","spotAngle","castShadow","shadowMapSize"};
    if (!sr_xml_attrs_allowed(ctx, "light", attrs, allowed, 14)) return;
    const char *id = sr_xml_required(ctx, "light", attrs, "id");
    const char *type = sr_xml_required(ctx, "light", attrs, "type");
    if (ctx->failed) return;
    if (!sr_id_valid(id)) SR_XML_FAIL_RETURN(ctx,"light","id","invalid identifier");
    if (sr_scene_id_exists(ctx->scene, id))
        SR_XML_FAIL_RETURN(ctx, "light", "id", "id must be globally unique");
    if (ctx->scene->light_count == ctx->scene->light_capacity) {
        size_t capacity = ctx->scene->light_capacity ? ctx->scene->light_capacity * 2 : 4;
        SrLight *items = sr_realloc(ctx->scene->lights, capacity * sizeof(*items));
        if (!items) SR_XML_FAIL_RETURN(ctx, "light", NULL, "out of memory");
        memset(items + ctx->scene->light_capacity, 0,
               (capacity - ctx->scene->light_capacity) * sizeof(*items));
        ctx->scene->lights = items; ctx->scene->light_capacity = capacity;
    }
    SrLight *light = &ctx->scene->lights[ctx->scene->light_count++];
    light->id = sr_strdup(id);
    light->color = sr_anim_color_static((SrColor){1,1,1,1});
    light->color.space = ctx->scene->project.working_color_space;
    light->shadow_map_size = 2048; light->source_line = sr_xml_line(ctx);
    light->intensity.base = 1.0; light->range = 1000.0;
    light->falloff = 2.0; light->spot_angle = 45.0;
    if (!light->id) SR_XML_FAIL_RETURN(ctx, "light", NULL, "out of memory");
    if (!strcmp(type,"ambient")) light->type=SR_LIGHT_AMBIENT;
    else if (!strcmp(type,"directional")) light->type=SR_LIGHT_DIRECTIONAL;
    else if (!strcmp(type,"point")) light->type=SR_LIGHT_POINT;
    else if (!strcmp(type,"spot")) light->type=SR_LIGHT_SPOT;
    else SR_XML_FAIL_RETURN(ctx,"light","type","unsupported light type");
    const char *value = sr_xml_attr(attrs,"color");
    if (value && !sr_parse_color(value,&light->color.base)) SR_XML_FAIL_RETURN(ctx,"light","color","invalid color");
    if (!decimal(ctx,"light",attrs,"intensity",&light->intensity.base) ||
        !decimal(ctx,"light",attrs,"x",&light->x.base) || !decimal(ctx,"light",attrs,"y",&light->y.base) ||
        !decimal(ctx,"light",attrs,"z",&light->z.base) || !decimal(ctx,"light",attrs,"yaw",&light->yaw.base) ||
        !decimal(ctx,"light",attrs,"pitch",&light->pitch.base) || !decimal(ctx,"light",attrs,"range",&light->range) ||
        !decimal(ctx,"light",attrs,"falloff",&light->falloff) || !decimal(ctx,"light",attrs,"spotAngle",&light->spot_angle)) return;
    if(light->intensity.base<0||light->range<=0||light->falloff<0||
       light->spot_angle<=0||light->spot_angle>=180)
        SR_XML_FAIL_RETURN(ctx,"light","intensity/range/falloff/spotAngle",
                           "expected non-negative intensity/falloff, positive range, and 0 < spotAngle < 180");
    value = sr_xml_attr(attrs,"castShadow");
    if (value && !sr_parse_bool(value,&light->cast_shadow)) SR_XML_FAIL_RETURN(ctx,"light","castShadow","expected true or false");
    value = sr_xml_attr(attrs,"shadowMapSize");
    if (value && (!sr_parse_u32(value,&light->shadow_map_size) ||
                  light->shadow_map_size < 16 || light->shadow_map_size > 8192))
        SR_XML_FAIL_RETURN(ctx,"light","shadowMapSize","expected an integer in [16, 8192]");
    sr_xml_push(ctx,(ParseFrame){.kind=E_LIGHT,.light=light},"light");
}

static bool anim_decimal(ParseContext *ctx, const XML_Char **attrs,
                         const char *name, SrAnimValue *target) {
    return decimal(ctx, "effect", attrs, name, &target->base);
}

/* Splits the lighting effect's `lights` id list. */
static bool parse_light_ids(SrEffect *effect, const char *text) {
    size_t capacity = 0;
    const char *cursor = text;
    while (*cursor) {
        while (*cursor && isspace((unsigned char)*cursor)) ++cursor;
        if (!*cursor) break;
        const char *begin = cursor;
        while (*cursor && !isspace((unsigned char)*cursor)) ++cursor;
        if (effect->light_count == capacity) {
            capacity = capacity ? capacity * 2 : 4;
            char **ids = sr_realloc(effect->light_ids, capacity * sizeof(*ids));
            if (!ids) return false;
            effect->light_ids = ids;
        }
        size_t length = (size_t)(cursor - begin);
        char *id = sr_alloc(length + 1);
        if (!id) return false;
        memcpy(id, begin, length);
        id[length] = '\0';
        effect->light_ids[effect->light_count++] = id;
    }
    return true;
}

void sr_xml_start_effect(ParseContext *ctx, const XML_Char **attrs) {
    const char *const allowed[] = {"id","type","enabled","intensity","radius","threshold",
        "saturation","contrast","brightness","color","offsetX","offsetY","lights",
        "falloff","relief"};
    if (!sr_xml_attrs_allowed(ctx,"effect",attrs,allowed,15)) return;
    const char *id=sr_xml_required(ctx,"effect",attrs,"id");
    const char *type=sr_xml_required(ctx,"effect",attrs,"type");
    if(ctx->failed)return;
    if(!sr_id_valid(id))SR_XML_FAIL_RETURN(ctx,"effect","id","invalid identifier");
    if(sr_scene_id_exists(ctx->scene,id))SR_XML_FAIL_RETURN(ctx,"effect","id","id must be globally unique");
    if(ctx->scene->effect_count==ctx->scene->effect_capacity){size_t cap=ctx->scene->effect_capacity?ctx->scene->effect_capacity*2:4;
        SrEffect *items=sr_realloc(ctx->scene->effects,cap*sizeof(*items));if(!items)SR_XML_FAIL_RETURN(ctx,"effect",NULL,"out of memory");
        memset(items+ctx->scene->effect_capacity,0,(cap-ctx->scene->effect_capacity)*sizeof(*items));ctx->scene->effects=items;ctx->scene->effect_capacity=cap;}
    SrEffect *effect=&ctx->scene->effects[ctx->scene->effect_count++];effect->id=sr_strdup(id);effect->enabled=true;
    effect->source_line=sr_xml_line(ctx);
    effect->intensity.base=1;effect->radius.base=4;effect->threshold.base=.7;effect->saturation.base=1;effect->contrast.base=1;
    effect->offset_x.base=8;effect->offset_y.base=8;
    effect->color=sr_anim_color_static((SrColor){1,1,1,1});
    effect->color.space=ctx->scene->project.working_color_space;
    effect->falloff=SR_FALLOFF_SMOOTH;
    if(!effect->id)SR_XML_FAIL_RETURN(ctx,"effect",NULL,"out of memory");
    if(!strcmp(type,"glow"))effect->type=SR_EFFECT_GLOW;else if(!strcmp(type,"bloom"))effect->type=SR_EFFECT_BLOOM;
    else if(!strcmp(type,"blur"))effect->type=SR_EFFECT_BLUR;else if(!strcmp(type,"color-grade"))effect->type=SR_EFFECT_COLOR_GRADE;
    else if(!strcmp(type,"vignette"))effect->type=SR_EFFECT_VIGNETTE;else if(!strcmp(type,"lens-flare"))effect->type=SR_EFFECT_LENS_FLARE;
    else if(!strcmp(type,"drop-shadow")){effect->type=SR_EFFECT_DROP_SHADOW;effect->color.base=(SrColor){0,0,0,1};}
    else if(!strcmp(type,"lighting"))effect->type=SR_EFFECT_LIGHTING;
    else SR_XML_FAIL_RETURN(ctx,"effect","type","unsupported effect type");
    const char *value=sr_xml_attr(attrs,"enabled");if(value&&!sr_parse_bool(value,&effect->enabled))SR_XML_FAIL_RETURN(ctx,"effect","enabled","expected true or false");
    value=sr_xml_attr(attrs,"color");if(value&&!sr_parse_color(value,&effect->color.base))SR_XML_FAIL_RETURN(ctx,"effect","color","invalid color");
    if(!anim_decimal(ctx,attrs,"intensity",&effect->intensity)||!anim_decimal(ctx,attrs,"radius",&effect->radius)||
       !anim_decimal(ctx,attrs,"threshold",&effect->threshold)||!anim_decimal(ctx,attrs,"saturation",&effect->saturation)||
       !anim_decimal(ctx,attrs,"contrast",&effect->contrast)||!anim_decimal(ctx,attrs,"brightness",&effect->brightness)||
       !anim_decimal(ctx,attrs,"offsetX",&effect->offset_x)||!anim_decimal(ctx,attrs,"offsetY",&effect->offset_y)||
       !anim_decimal(ctx,attrs,"relief",&effect->relief))return;
    if(effect->intensity.base<0||effect->radius.base<0||effect->threshold.base<0||
       effect->threshold.base>1||effect->saturation.base<0||effect->contrast.base<0||
       effect->relief.base<0)
        SR_XML_FAIL_RETURN(ctx,"effect","intensity/radius/threshold/saturation/contrast/relief",
                           "expected non-negative values and threshold in [0,1]");
    value=sr_xml_attr(attrs,"falloff");
    if(value){
        if(effect->type!=SR_EFFECT_LIGHTING)SR_XML_FAIL_RETURN(ctx,"effect","falloff","falloff requires type=lighting");
        if(!strcmp(value,"smooth"))effect->falloff=SR_FALLOFF_SMOOTH;
        else if(!strcmp(value,"linear"))effect->falloff=SR_FALLOFF_LINEAR;
        else if(!strcmp(value,"quadratic"))effect->falloff=SR_FALLOFF_QUADRATIC;
        else if(!strcmp(value,"none"))effect->falloff=SR_FALLOFF_NONE;
        else SR_XML_FAIL_RETURN(ctx,"effect","falloff","expected linear, quadratic, smooth, or none");
    }
    value=sr_xml_attr(attrs,"lights");
    if(value){
        if(effect->type!=SR_EFFECT_LIGHTING)SR_XML_FAIL_RETURN(ctx,"effect","lights","lights requires type=lighting");
        if(!parse_light_ids(effect,value))SR_XML_FAIL_RETURN(ctx,"effect","lights","out of memory");
    }
    sr_xml_push(ctx,(ParseFrame){.kind=E_EFFECT,.effect=effect},"effect");
}

void sr_xml_start_object3d(ParseContext *ctx, const XML_Char **attrs) {
    const char *const allowed[]={"id","primitive","mesh","material","x","y","z","rotation","rotationX","rotationY","scaleX","scaleY","scaleZ","radius","castShadow","receiveShadow"};
    if(!sr_xml_attrs_allowed(ctx,"object3D",attrs,allowed,16))return;
    const char *id=sr_xml_required(ctx,"object3D",attrs,"id");const char *primitive=sr_xml_required(ctx,"object3D",attrs,"primitive");if(ctx->failed)return;
    if(!sr_id_valid(id))SR_XML_FAIL_RETURN(ctx,"object3D","id","invalid identifier");
    if(sr_scene_id_exists(ctx->scene,id))SR_XML_FAIL_RETURN(ctx,"object3D","id","id must be globally unique");
    if(ctx->scene->object3d_count==ctx->scene->object3d_capacity){size_t cap=ctx->scene->object3d_capacity?ctx->scene->object3d_capacity*2:4;
        SrObject3D *items=sr_realloc(ctx->scene->objects3d,cap*sizeof(*items));if(!items)SR_XML_FAIL_RETURN(ctx,"object3D",NULL,"out of memory");
        memset(items+ctx->scene->object3d_capacity,0,(cap-ctx->scene->object3d_capacity)*sizeof(*items));ctx->scene->objects3d=items;ctx->scene->object3d_capacity=cap;}
    SrObject3D *object=&ctx->scene->objects3d[ctx->scene->object3d_count++];object->source_line=sr_xml_line(ctx);object->id=sr_strdup(id);object->radius=50;object->cast_shadow=true;object->receive_shadow=true;
    if(!object->id)SR_XML_FAIL_RETURN(ctx,"object3D",NULL,"out of memory");
    object->transform.scale_x.base=object->transform.scale_y.base=object->transform.scale_z.base=1;
    if(!strcmp(primitive,"sphere"))object->primitive=SR_OBJECT_SPHERE;else if(!strcmp(primitive,"box"))object->primitive=SR_OBJECT_BOX;
    else if(!strcmp(primitive,"plane"))object->primitive=SR_OBJECT_PLANE;
    else if(!strcmp(primitive,"mesh"))object->primitive=SR_OBJECT_MESH;
    else SR_XML_FAIL_RETURN(ctx,"object3D","primitive","expected sphere, box, plane, or mesh");
    const char *value;
    value=sr_xml_attr(attrs,"mesh");
    if(object->primitive==SR_OBJECT_MESH){if(!value)SR_XML_FAIL_RETURN(ctx,"object3D","mesh","mesh asset is required for primitive=mesh");
        object->mesh_id=sr_strdup(value);if(!object->mesh_id)SR_XML_FAIL_RETURN(ctx,"object3D","mesh","out of memory");}
    else if(value)SR_XML_FAIL_RETURN(ctx,"object3D","mesh","mesh requires primitive=mesh");
    value=sr_xml_attr(attrs,"material");if(value){object->material_id=sr_strdup(value);if(!object->material_id)SR_XML_FAIL_RETURN(ctx,"object3D","material","out of memory");}
    if(!decimal(ctx,"object3D",attrs,"x",&object->transform.x.base)||!decimal(ctx,"object3D",attrs,"y",&object->transform.y.base)||
       !decimal(ctx,"object3D",attrs,"z",&object->transform.z.base)||!decimal(ctx,"object3D",attrs,"rotation",&object->transform.rotation.base)||
       !decimal(ctx,"object3D",attrs,"rotationX",&object->transform.rotation_x.base)||!decimal(ctx,"object3D",attrs,"rotationY",&object->transform.rotation_y.base)||
       !decimal(ctx,"object3D",attrs,"scaleX",&object->transform.scale_x.base)||
       !decimal(ctx,"object3D",attrs,"scaleY",&object->transform.scale_y.base)||!decimal(ctx,"object3D",attrs,"scaleZ",&object->transform.scale_z.base)||
       !decimal(ctx,"object3D",attrs,"radius",&object->radius))return;
    if(object->radius<=0)
        SR_XML_FAIL_RETURN(ctx,"object3D","radius","expected a positive radius");
    value=sr_xml_attr(attrs,"castShadow");if(value&&!sr_parse_bool(value,&object->cast_shadow))SR_XML_FAIL_RETURN(ctx,"object3D","castShadow","expected true or false");
    value=sr_xml_attr(attrs,"receiveShadow");if(value&&!sr_parse_bool(value,&object->receive_shadow))SR_XML_FAIL_RETURN(ctx,"object3D","receiveShadow","expected true or false");
    sr_xml_push(ctx,(ParseFrame){.kind=E_OBJECT3D,.object3d=object},"object3D");
}
