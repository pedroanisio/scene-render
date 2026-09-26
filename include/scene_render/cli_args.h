#ifndef SCENE_RENDER_CLI_ARGS_H
#define SCENE_RENDER_CLI_ARGS_H

#include "scene_render/renderer.h"

/* Command-line options of the scene-render executable. Parsing is a pure
 * function (no I/O, no exit) so the unit tests exercise the same code the
 * executable runs. String fields point into argv. */
typedef enum {
    SR_QUALITY_KEEP,        /* no --quality: the XML settings stand */
    SR_QUALITY_LOW,
    SR_QUALITY_MEDIUM,
    SR_QUALITY_HIGH
} SrQuality;

typedef struct {
    const char *scene_path;
    SrRenderOptions render;     /* preview_path NULL: frame-NNNNNN.png */
    bool help;
    bool version;
    bool print_schema;
    bool report_unsupported;
    bool verbose;
    bool metrics;
    bool override_resolution;
    uint32_t width, height;
    bool override_fps;
    uint32_t fps_num, fps_den;
    SrQuality quality;
    bool override_mode;
    SrRenderMode mode;
    const char *physics_cache_dir;  /* --physics-cache DIR, or NULL */
} SrCliOptions;

/* Parses argv[1..argc-1] into *out. On failure returns SR_ERR_ARGUMENT and
 * writes one line naming the offending option into error (NUL-terminated,
 * truncated to error_size). --help, --version and --print-schema make
 * --scene optional. */
SrStatus sr_cli_parse(int argc, char *const *argv, SrCliOptions *out,
                      char *error, size_t error_size);

/* The --help text, listing every option. */
const char *sr_cli_usage(void);

#endif
