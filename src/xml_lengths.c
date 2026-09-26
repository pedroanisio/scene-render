/* SPDX-License-Identifier: Apache-2.0 */
#include "xml_internal.h"

bool sr_xml_length_attr(ParseContext *ctx, const char *element,
                         const XML_Char **attrs, const char *attribute,
                         double *value, SrLengthUnit *unit, bool positive) {
    const char *text = sr_xml_attr(attrs, attribute);
    if (!text) return true;
    SrLength length;
    if (!sr_parse_length(text, &length) || (positive && length.value <= 0.0)) {
        sr_xml_fail(ctx, element, attribute, positive
                    ? "expected a positive length (pixels, %, vw, vh, vmin or vmax); "
                      "relative spelling limit 128 bytes, coefficient limit 1e6"
                    : "expected a finite length (pixels, %, vw, vh, vmin or vmax); "
                      "relative spelling limit 128 bytes, coefficient limit 1e6");
        return false;
    }
    *value = length.value;
    *unit = length.unit;
    if (length.unit != SR_LENGTH_PIXELS) ctx->scene->has_relative_lengths = true;
    return true;
}

bool sr_xml_anim_length_attr(ParseContext *ctx, const char *element,
                              const XML_Char **attrs, const char *attribute,
                              SrAnimValue *value, bool positive) {
    return sr_xml_length_attr(ctx, element, attrs, attribute,
                              &value->base, &value->unit, positive);
}
