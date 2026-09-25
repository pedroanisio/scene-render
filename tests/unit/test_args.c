/* SPDX-License-Identifier: Apache-2.0 */
/* Command-line parsing (src/cli_args.c): the same code the executable runs. */
#include "scene_render/cli_args.h"

#include "harness.h"

/* argv is NULL-terminated; argc is derived from it so the two cannot drift. */
static SrStatus parse_v(SrCliOptions *options, char *error, size_t size,
                        char **argv)
{
    int argc = 0;
    while (argv[argc]) ++argc;
    return sr_cli_parse(argc, argv, options, error, size);
}

#define PARSE(o, e, ...) \
    parse_v((o), (e), sizeof(e), (char *[]){"scene-render", __VA_ARGS__, NULL})

static void defaults_and_every_option(sr_test_ctx *t)
{
    SrCliOptions o;
    char e[256] = "";
    CHECK(t, PARSE(&o, e, "--scene", "s.xml") == SR_OK);
    CHECK_STR(t, o.scene_path, "s.xml");
    CHECK(t, !o.render.preview && !o.render.has_range && !o.render.hash);
    CHECK(t, o.render.preview_path == NULL);
    CHECK_INT(t, o.render.encoder_threads, 0);
    CHECK_INT(t, o.render.segment_frames, SR_RESUME_DEFAULT_SEGMENT_FRAMES);
    CHECK_INT(t, o.quality, SR_QUALITY_KEEP);
    CHECK(t, !o.render.request_gpu && !o.override_mode && !o.override_fps &&
             !o.override_resolution && !o.physics_cache_dir);

    CHECK(t, PARSE(&o, e, "--scene", "s.xml", "--output", "o.mp4",
                   "--frame-range", "2:9", "--threads", "8", "--quality", "high",
                   "--renderer", "gpu", "--resolution", "1920x1080",
                   "--fps", "30000/1001", "--hash", "--metrics", "--verbose",
                   "--validate", "--mode", "viewport", "--metrics-trace", "t.jsonl",
                   "--physics-cache", "cache") == SR_OK);
    CHECK_STR(t, o.render.output_override, "o.mp4");
    CHECK(t, o.render.has_range);
    CHECK_INT(t, o.render.first_frame, 2);
    CHECK_INT(t, o.render.end_frame, 9);
    CHECK_INT(t, o.render.encoder_threads, 8);
    CHECK_INT(t, o.quality, SR_QUALITY_HIGH);
    CHECK(t, o.render.request_gpu);
    CHECK(t, o.override_resolution);
    CHECK_INT(t, o.width, 1920);
    CHECK_INT(t, o.height, 1080);
    CHECK(t, o.override_fps);
    CHECK_INT(t, o.fps_num, 30000);
    CHECK_INT(t, o.fps_den, 1001);
    CHECK(t, o.render.hash && o.metrics && o.render.report_metrics && o.verbose &&
             o.render.validate_only);
    CHECK(t, o.override_mode && o.mode == SR_MODE_VIEWPORT);
    CHECK_STR(t, o.render.trace_path, "t.jsonl");
    CHECK_STR(t, o.physics_cache_dir, "cache");

    CHECK(t, PARSE(&o, e, "--scene", "s.xml", "--frame", "12", "--threads", "auto",
                   "--fps", "25", "--quality", "low", "--renderer", "cpu") == SR_OK);
    CHECK(t, o.render.preview);
    CHECK_INT(t, o.render.preview_frame, 12);
    CHECK_INT(t, o.render.encoder_threads, 0);
    CHECK_INT(t, o.fps_num, 25);
    CHECK_INT(t, o.fps_den, 1);
    CHECK_INT(t, o.quality, SR_QUALITY_LOW);
    CHECK(t, !o.render.request_gpu);

    CHECK(t, PARSE(&o, e, "--scene", "s.xml", "--preview-frame", "3",
                   "--preview-out", "p.png", "--quality", "medium") == SR_OK);
    CHECK(t, o.render.preview);
    CHECK_INT(t, o.render.preview_frame, 3);
    CHECK_STR(t, o.render.preview_path, "p.png");
    CHECK_INT(t, o.quality, SR_QUALITY_MEDIUM);

    CHECK(t, PARSE(&o, e, "--scene", "s", "--resume", "--keep-parts",
                   "--segment-frames", "40") == SR_OK);
    CHECK(t, o.render.resume && o.render.keep_parts);
    CHECK_INT(t, o.render.segment_frames, 40);
    CHECK(t, PARSE(&o, e, "--help") == SR_OK && o.help);
    CHECK(t, PARSE(&o, e, "-h") == SR_OK && o.help);
    CHECK(t, PARSE(&o, e, "--version") == SR_OK && o.version);
    CHECK(t, PARSE(&o, e, "--print-schema") == SR_OK && o.print_schema);
}

/* Each row is one invalid invocation and the message it must produce. */
static void rejects_invalid_values(sr_test_ctx *t)
{
    static const struct {
        const char *opt, *val, *message;
    } rows[] = {
        {"--frame", "-1", "--frame expects"},
        {"--frame", "3x", "--frame expects"},
        {"--frame", "", "--frame expects"},
        {"--frame", "99999999999999999999", "--frame expects"},
        {"--preview-frame", "x", "--preview-frame expects"},
        {"--frame-range", "5:5", "--frame-range"},
        {"--frame-range", "6:5", "--frame-range"},
        {"--frame-range", "5", "--frame-range"},
        {"--frame-range", ":5", "--frame-range"},
        {"--frame-range", "5:", "--frame-range"},
        {"--frame-range", "123456789012345678901234567890123:4", "--frame-range"},
        {"--threads", "0", "--threads"},
        {"--threads", "-2", "--threads"},
        {"--quality", "best", "--quality"},
        {"--renderer", "tpu", "--renderer"},
        {"--mode", "cube", "--mode"},
        {"--resolution", "0x2", "--resolution"},
        {"--resolution", "4x0", "--resolution"},
        {"--resolution", "12", "--resolution"},
        {"--resolution", "99999999999x2", "--resolution"},
        {"--fps", "0/1", "--fps"},
        {"--fps", "1/0", "--fps"},
        {"--fps", "0", "--fps"},
        {"--fps", "abc", "--fps"},
        {"--segment-frames", "0", "--segment-frames"},
        {"--physics-cache", "", "--physics-cache"},
        {"--bogus", "1", "unknown option '--bogus'"},
    };
    for (size_t i = 0; i < sizeof rows / sizeof rows[0]; ++i) {
        SrCliOptions o;
        char e[256] = "";
        if (PARSE(&o, e, "--scene", "s.xml", (char *)rows[i].opt,
                  (char *)rows[i].val) == SR_OK)
            SR_FAIL(t, "%s %s was accepted", rows[i].opt, rows[i].val);
        CHECK_CONTAINS(t, e, rows[i].message);
    }
}

static void rejects_structural_errors(sr_test_ctx *t)
{
    SrCliOptions o;
    char e[256] = "";
    CHECK(t, PARSE(&o, e, "--scene") == SR_ERR_ARGUMENT);
    CHECK_CONTAINS(t, e, "--scene requires a value");
    CHECK(t, PARSE(&o, e, "--hash") == SR_ERR_ARGUMENT);
    CHECK_CONTAINS(t, e, "--scene is required");
    CHECK(t, PARSE(&o, e, "--frobnicate") == SR_ERR_ARGUMENT);
    CHECK_CONTAINS(t, e, "unknown option '--frobnicate'");
    CHECK(t, PARSE(&o, e, "--scene", "s", "--hash", "--frame", "1") != SR_OK);
    CHECK_CONTAINS(t, e, "--hash cannot be combined");
    CHECK(t, PARSE(&o, e, "--scene", "s", "--hash", "--resume") != SR_OK);
    CHECK_CONTAINS(t, e, "--hash cannot be combined with --resume");
    CHECK(t, PARSE(&o, e, "--scene", "s", "--resume", "--frame", "0") != SR_OK);
    CHECK_CONTAINS(t, e, "--resume cannot be combined");
    CHECK(t, PARSE(&o, e, "--scene", "s", "--keep-parts") != SR_OK);
    CHECK_CONTAINS(t, e, "--keep-parts requires --resume");
    CHECK(t, PARSE(&o, e, "--scene", "s", "--segment-frames", "4") != SR_OK);
    CHECK_CONTAINS(t, e, "--segment-frames requires --resume");
    /* A too-small error buffer truncates instead of overflowing. */
    char tiny[8];
    CHECK(t, parse_v(&o, tiny, sizeof(tiny),
                     (char *[]){"scene-render", "--nope", NULL}) == SR_ERR_ARGUMENT);
    CHECK_INT(t, strlen(tiny), sizeof(tiny) - 1);
}

static void usage_lists_every_option(sr_test_ctx *t)
{
    const char *usage = sr_cli_usage();
    const char *opts[] = {"--scene", "--output", "--validate", "--frame N",
                          "--preview-frame", "--preview-out", "--frame-range",
                          "--threads", "--quality", "--renderer", "--resolution",
                          "--fps", "--mode", "--hash", "--metrics",
                          "--metrics-trace", "--verbose", "--print-schema",
                          "--version", "--help", "--resume", "--segment-frames",
                          "--keep-parts", "--physics-cache", SR_VERSION};
    for (size_t i = 0; i < sizeof opts / sizeof opts[0]; ++i)
        CHECK_CONTAINS(t, usage, opts[i]);
}

const sr_test_case sr_tests_args[] = {
    {"defaults_and_every_option", defaults_and_every_option},
    {"rejects_invalid_values", rejects_invalid_values},
    {"rejects_structural_errors", rejects_structural_errors},
    {"usage_lists_every_option", usage_lists_every_option},
    {NULL, NULL},
};
