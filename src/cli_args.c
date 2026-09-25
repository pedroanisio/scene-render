#include "scene_render/cli_args.h"

#include <stdarg.h>
#include <stdio.h>
#include <string.h>

const char *sr_cli_usage(void) {
    return
        "scene-render " SR_VERSION "\n"
        "Usage: scene-render --scene FILE [options]\n\n"
        "  --scene FILE             Scene XML (validated against the embedded XSD)\n"
        "  --output FILE            Override XML output path (.mp4, .mov, .mkv)\n"
        "  --validate               Validate without decoding or rendering\n"
        "  --print-schema           Print the embedded XSD and exit\n"
        "  --frame-range A:B        Render half-open frame range [A,B)\n"
        "  --preview-frame N, --frame N\n"
        "                           Write one deterministic 8-bit preview frame and\n"
        "                           print \"<path> <FNV-1a 64 hex>\"\n"
        "  --preview-out FILE       Preview path (default frame-NNNNNN.png in the\n"
        "                           working directory); .png writes PNG, else PPM\n"
        "  --hash                   Render the range without encoding; print\n"
        "                           \"<frame> <hex>\" per frame and \"audio <hex>\"\n"
        "  --mode standard|equirectangular|viewport\n"
        "                           Override project mode (360 modes need scene360)\n"
        "  --resolution WIDTHxHEIGHT\n"
        "  --fps N or N/D           Override frame rate\n"
        "  --quality low|medium|high\n"
        "  --threads auto|N         Render/encoder worker count\n"
        "  --renderer cpu|gpu       Select backend (gpu falls back to CPU)\n"
        "  --resume                 Render in segments under OUTPUT.parts/; a rerun\n"
        "                           of the same command reuses finished segments\n"
        "  --segment-frames N       Frames per --resume segment (default 150)\n"
        "  --keep-parts             Keep OUTPUT.parts/ after a successful --resume\n"
        "  --physics-cache DIR      Physics cache directory (overrides the XML\n"
        "                           <physics cache>; file named by scene signature)\n"
        "  --metrics                Print timing, CPU, per-stage and memory metrics\n"
        "  --metrics-trace FILE     Write per-frame stage timings as JSON Lines\n"
        "  --verbose                Verbose diagnostics\n"
        "  --version                Print \"scene-render VERSION\"\n"
        "  --help, -h               Print this help\n"
        "\n"
        "Exit status: 0 ok, 2 usage/argument, 3 XML/schema, 4 asset, 5 render,\n"
        "6 encoder, 7 I/O, 8 out of memory.\n";
}

static SrStatus fail(char *error, size_t size, const char *format, ...) {
    if (error && size) {
        va_list args;
        va_start(args, format);
        vsnprintf(error, size, format, args);
        va_end(args);
    }
    return SR_ERR_ARGUMENT;
}

/* "A<sep>B" with two unsigned decimal integers. */
static bool parse_pair(const char *text, char separator, uint64_t *a,
                       uint64_t *b) {
    const char *middle = strchr(text, separator);
    if (!middle || middle == text) return false;
    char left[32];
    size_t length = (size_t)(middle - text);
    if (length >= sizeof(left)) return false;
    memcpy(left, text, length);
    left[length] = '\0';
    return sr_parse_u64(left, a) && sr_parse_u64(middle + 1, b);
}

static bool parse_resolution(const char *text, uint32_t *width,
                             uint32_t *height) {
    uint64_t a, b;
    if (!parse_pair(text, 'x', &a, &b) || a == 0 || b == 0 ||
        a > UINT32_MAX || b > UINT32_MAX)
        return false;
    *width = (uint32_t)a;
    *height = (uint32_t)b;
    return true;
}

static bool parse_fps(const char *text, uint32_t *num, uint32_t *den) {
    uint64_t a, b = 1;
    if (strchr(text, '/')) {
        if (!parse_pair(text, '/', &a, &b)) return false;
    } else if (!sr_parse_u64(text, &a)) {
        return false;
    }
    if (a == 0 || b == 0 || a > UINT32_MAX || b > UINT32_MAX) return false;
    *num = (uint32_t)a;
    *den = (uint32_t)b;
    return true;
}

static bool is(const char *arg, const char *name) {
    return strcmp(arg, name) == 0;
}

SrStatus sr_cli_parse(int argc, char *const *argv, SrCliOptions *out,
                      char *error, size_t error_size) {
    if (error && error_size) error[0] = '\0';
    if (!out) return fail(error, error_size, "error: no options storage");
    SrCliOptions o = {0};
    o.render.segment_frames = SR_RESUME_DEFAULT_SEGMENT_FRAMES;
    bool segment_given = false;
    for (int i = 1; i < argc; ++i) {
        const char *arg = argv[i];
        /* Flags without a value. */
        if (is(arg, "--validate")) { o.render.validate_only = true; continue; }
        if (is(arg, "--print-schema")) { o.print_schema = true; continue; }
        if (is(arg, "--hash")) { o.render.hash = true; continue; }
        if (is(arg, "--resume")) { o.render.resume = true; continue; }
        if (is(arg, "--keep-parts")) { o.render.keep_parts = true; continue; }
        if (is(arg, "--metrics")) {
            o.metrics = true;
            o.render.report_metrics = true;
            continue;
        }
        if (is(arg, "--verbose")) { o.verbose = true; continue; }
        if (is(arg, "--version")) { o.version = true; continue; }
        if (is(arg, "--help") || is(arg, "-h")) { o.help = true; continue; }
        /* Options with one value. */
        static const char *const valued[] = {
            "--scene", "--output", "--frame-range", "--preview-frame", "--frame",
            "--preview-out", "--mode", "--resolution", "--fps", "--quality",
            "--threads", "--renderer", "--metrics-trace", "--segment-frames",
            "--physics-cache"};
        bool known = false;
        for (size_t k = 0; k < sizeof(valued) / sizeof(valued[0]); ++k)
            known |= is(arg, valued[k]);
        if (!known) return fail(error, error_size, "error: unknown option '%s'", arg);
        if (i + 1 >= argc)
            return fail(error, error_size, "error: %s requires a value", arg);
        const char *value = argv[++i];
        if (is(arg, "--scene")) {
            o.scene_path = value;
        } else if (is(arg, "--output")) {
            o.render.output_override = value;
        } else if (is(arg, "--frame-range")) {
            if (!parse_pair(value, ':', &o.render.first_frame, &o.render.end_frame) ||
                o.render.end_frame <= o.render.first_frame)
                return fail(error, error_size,
                            "error: --frame-range expects A:B with A < B");
            o.render.has_range = true;
        } else if (is(arg, "--preview-frame") || is(arg, "--frame")) {
            if (!sr_parse_u64(value, &o.render.preview_frame))
                return fail(error, error_size, "error: %s expects an integer", arg);
            o.render.preview = true;
        } else if (is(arg, "--preview-out")) {
            o.render.preview_path = value;
        } else if (is(arg, "--mode")) {
            if (is(value, "standard")) o.mode = SR_MODE_STANDARD;
            else if (is(value, "equirectangular")) o.mode = SR_MODE_EQUIRECTANGULAR;
            else if (is(value, "viewport")) o.mode = SR_MODE_VIEWPORT;
            else
                return fail(error, error_size, "error: --mode expects standard, "
                            "equirectangular, or viewport");
            o.override_mode = true;
        } else if (is(arg, "--resolution")) {
            if (!parse_resolution(value, &o.width, &o.height))
                return fail(error, error_size,
                            "error: --resolution expects WIDTHxHEIGHT");
            o.override_resolution = true;
        } else if (is(arg, "--fps")) {
            if (!parse_fps(value, &o.fps_num, &o.fps_den))
                return fail(error, error_size, "error: --fps expects N or N/D");
            o.override_fps = true;
        } else if (is(arg, "--quality")) {
            if (is(value, "low")) o.quality = SR_QUALITY_LOW;
            else if (is(value, "medium")) o.quality = SR_QUALITY_MEDIUM;
            else if (is(value, "high")) o.quality = SR_QUALITY_HIGH;
            else
                return fail(error, error_size,
                            "error: --quality expects low, medium, or high");
        } else if (is(arg, "--threads")) {
            uint32_t count;
            if (is(value, "auto"))
                o.render.encoder_threads = 0;
            else if (sr_parse_u32(value, &count) && count > 0)
                o.render.encoder_threads = count;
            else
                return fail(error, error_size,
                            "error: --threads expects auto or a positive integer");
        } else if (is(arg, "--renderer")) {
            if (is(value, "gpu")) o.render.request_gpu = true;
            else if (is(value, "cpu")) o.render.request_gpu = false;
            else return fail(error, error_size, "error: --renderer expects cpu or gpu");
        } else if (is(arg, "--metrics-trace")) {
            o.render.trace_path = value;
        } else if (is(arg, "--segment-frames")) {
            uint32_t count;
            if (!sr_parse_u32(value, &count) || count == 0)
                return fail(error, error_size,
                            "error: --segment-frames expects a positive integer");
            o.render.segment_frames = count;
            segment_given = true;
        } else { /* --physics-cache */
            if (!*value)
                return fail(error, error_size, "error: --physics-cache expects a directory");
            o.physics_cache_dir = value;
        }
    }
    if (!o.help && !o.version && !o.print_schema) {
        if (!o.scene_path)
            return fail(error, error_size, "error: --scene is required");
        if (o.render.hash && o.render.preview)
            return fail(error, error_size,
                        "error: --hash cannot be combined with --preview-frame/--frame");
        if (o.render.hash && o.render.resume)
            return fail(error, error_size, "error: --hash cannot be combined with --resume");
        if (o.render.resume && o.render.preview)
            return fail(error, error_size,
                        "error: --resume cannot be combined with --preview-frame/--frame");
        if ((segment_given || o.render.keep_parts) && !o.render.resume)
            return fail(error, error_size,
                        "error: %s requires --resume",
                        segment_given ? "--segment-frames" : "--keep-parts");
    }
    *out = o;
    return SR_OK;
}
