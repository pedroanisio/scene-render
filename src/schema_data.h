#ifndef SCENE_RENDER_SCHEMA_DATA_H
#define SCENE_RENDER_SCHEMA_DATA_H

#include <stddef.h>

/* schema/scene-v1.xsd embedded at build time (CMake: cmake/schema_data.c.in;
 * Makefile: od/sed recipe). NUL-terminated; the length excludes the NUL. */
extern const unsigned char sr_schema_xsd[];
extern const size_t sr_schema_xsd_len;

#endif
