#ifndef SCENE_RENDER_COLOR_H
#define SCENE_RENDER_COLOR_H

#include "scene_render/compositor.h"
#include "scene_render/scene.h"

bool sr_color_space_parse(const char *text, SrColorSpace *space);
const char *sr_color_space_name(SrColorSpace space);
double sr_color_decode(double value, SrColorSpace space);
double sr_color_encode(double value, SrColorSpace space);
SrStatus sr_color_convert_frame(SrFrame *frame, SrColorSpace source,
                                SrColorSpace target, unsigned threads);

#endif
