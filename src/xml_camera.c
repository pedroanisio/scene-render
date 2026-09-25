#include "xml_internal.h"

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

void sr_xml_start_scene360(ParseContext *ctx, const XML_Char **attrs) {
    const char *const allowed[] = {"layout", "width", "height",
                                    "viewportCamera"};
    if (!sr_xml_attrs_allowed(ctx, "scene360", attrs, allowed, 4)) return;
    ctx->scene->scene360.source_line = sr_xml_line(ctx);
    const char *layout = sr_xml_attr(attrs, "layout");
    if (layout && strcmp(layout, "equirectangular") != 0)
        SR_XML_FAIL_RETURN(ctx, "scene360", "layout",
                           "only equirectangular layout is supported");
    const char *value = sr_xml_attr(attrs, "width");
    if (value && (!sr_parse_u32(value, &ctx->scene->scene360.width) ||
                  !ctx->scene->scene360.width))
        SR_XML_FAIL_RETURN(ctx, "scene360", "width", "expected a positive integer");
    value = sr_xml_attr(attrs, "height");
    if (value && (!sr_parse_u32(value, &ctx->scene->scene360.height) ||
                  !ctx->scene->scene360.height))
        SR_XML_FAIL_RETURN(ctx, "scene360", "height", "expected a positive integer");
    if ((uint64_t)ctx->scene->scene360.width !=
        (uint64_t)ctx->scene->scene360.height * 2U)
        SR_XML_FAIL_RETURN(ctx, "scene360", "width/height",
                           "equirectangular canvases must have a 2:1 aspect ratio");
    value = sr_xml_attr(attrs, "viewportCamera");
    if (value) {
        ctx->scene->scene360.viewport_camera_id = sr_strdup(value);
        if (!ctx->scene->scene360.viewport_camera_id)
            SR_XML_FAIL_RETURN(ctx, "scene360", "viewportCamera", "out of memory");
    }
    ctx->scene->scene360.enabled = true;
    ctx->seen_scene360 = true;
}

void sr_xml_start_camera(ParseContext *ctx, const XML_Char **attrs) {
    const char *const allowed[] = {"id", "active", "projection", "x", "y", "z",
        "yaw", "pitch", "roll", "fov", "near", "far", "zoom", "focusDistance",
        "aperture"};
    if (!sr_xml_attrs_allowed(ctx, "camera", attrs, allowed, 15)) return;
    const char *id = sr_xml_required(ctx, "camera", attrs, "id");
    if (!id) return;
    if (!sr_id_valid(id))
        SR_XML_FAIL_RETURN(ctx, "camera", "id", "invalid identifier");
    if (sr_scene_id_exists(ctx->scene, id))
        SR_XML_FAIL_RETURN(ctx, "camera", "id", "id must be globally unique");
    if (ctx->scene->camera_count == ctx->scene->camera_capacity) {
        size_t capacity = ctx->scene->camera_capacity ?
                          ctx->scene->camera_capacity * 2 : 4;
        SrCamera *items = sr_realloc(ctx->scene->cameras,
                                     capacity * sizeof(*items));
        if (!items) SR_XML_FAIL_RETURN(ctx, "camera", NULL, "out of memory");
        memset(items + ctx->scene->camera_capacity, 0,
               (capacity - ctx->scene->camera_capacity) * sizeof(*items));
        ctx->scene->cameras = items;
        ctx->scene->camera_capacity = capacity;
    }
    SrCamera *camera = &ctx->scene->cameras[ctx->scene->camera_count++];
    camera->source_line = sr_xml_line(ctx);
    camera->id = sr_strdup(id);
    camera->active = true;
    camera->fov.base = 90.0;
    camera->focus_distance.base = 1000.0;
    camera->near_plane = 0.1;
    camera->far_plane = 10000.0;
    if (!camera->id) SR_XML_FAIL_RETURN(ctx, "camera", NULL, "out of memory");
    const char *value = sr_xml_attr(attrs, "active");
    if (value && !sr_parse_bool(value, &camera->active))
        SR_XML_FAIL_RETURN(ctx, "camera", "active", "expected true or false");
    value = sr_xml_attr(attrs, "projection");
    if (value && strcmp(value, "perspective") != 0 &&
        strcmp(value, "orthographic") != 0)
        SR_XML_FAIL_RETURN(ctx, "camera", "projection",
                           "expected perspective or orthographic");
    camera->orthographic = value && strcmp(value, "orthographic") == 0;
    if (!decimal(ctx, "camera", attrs, "x", &camera->x.base) ||
        !decimal(ctx, "camera", attrs, "y", &camera->y.base) ||
        !decimal(ctx, "camera", attrs, "z", &camera->z.base) ||
        !decimal(ctx, "camera", attrs, "yaw", &camera->yaw.base) ||
        !decimal(ctx, "camera", attrs, "pitch", &camera->pitch.base) ||
        !decimal(ctx, "camera", attrs, "roll", &camera->roll.base) ||
        !decimal(ctx, "camera", attrs, "fov", &camera->fov.base) ||
        !decimal(ctx, "camera", attrs, "near", &camera->near_plane) ||
        !decimal(ctx, "camera", attrs, "far", &camera->far_plane) ||
        !decimal(ctx, "camera", attrs, "zoom", &camera->zoom.base) ||
        !decimal(ctx, "camera", attrs, "focusDistance",
                 &camera->focus_distance.base) ||
        !decimal(ctx, "camera", attrs, "aperture", &camera->aperture.base)) return;
    camera->zoom_set = sr_xml_attr(attrs, "zoom") != NULL;
    if (camera->zoom_set && !(camera->zoom.base > 0.0))
        SR_XML_FAIL_RETURN(ctx, "camera", "zoom", "expected a positive focal length");
    if (!(camera->focus_distance.base > 0.0))
        SR_XML_FAIL_RETURN(ctx, "camera", "focusDistance", "expected a positive distance");
    if (camera->aperture.base < 0.0)
        SR_XML_FAIL_RETURN(ctx, "camera", "aperture", "expected a non-negative radius");
    if (camera->fov.base <= 1.0 || camera->fov.base >= 179.0 ||
        camera->near_plane <= 0.0 || camera->far_plane <= camera->near_plane)
        SR_XML_FAIL_RETURN(ctx, "camera", "fov/near/far",
                           "expected 1 < fov < 179 and 0 < near < far");
    sr_xml_push(ctx, (ParseFrame){.kind = E_CAMERA, .camera = camera}, "camera");
}
