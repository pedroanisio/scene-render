/* SPDX-License-Identifier: Apache-2.0 */
#include <errno.h>
#include <stdlib.h>
#include <sys/stat.h>

#include "harness.h"

#include <libavutil/log.h>

#ifndef SR_TEST_DATA_DIR
#error "SR_TEST_DATA_DIR must name the repository root"
#endif
#ifndef SR_TEST_TMP_DIR
#error "SR_TEST_TMP_DIR must name a scratch directory in the build tree"
#endif

static const char *rotating_path(const char *dir, const char *name)
{
    static char buf[4][1024];
    static int slot = 0;
    slot = (slot + 1) % 4;
    snprintf(buf[slot], sizeof buf[slot], "%s/%s", dir, name);
    return buf[slot];
}

const char *sr_test_data_path(const char *relative)
{
    return rotating_path(SR_TEST_DATA_DIR, relative);
}

const char *sr_test_tmp_path(const char *name)
{
    if (mkdir(SR_TEST_TMP_DIR, 0755) != 0 && errno != EEXIST)
        fprintf(stderr, "cannot create %s\n", SR_TEST_TMP_DIR);
    return rotating_path(SR_TEST_TMP_DIR, name);
}

typedef struct suite {
    const char *name;
    const sr_test_case *cases;
} suite;

int main(int argc, char **argv)
{
    const suite suites[] = {
        {"random", sr_tests_random},
        {"blend_color", sr_tests_blend_color},
        {"skew", sr_tests_skew},
        {"compositing", sr_tests_compositing},
        {"composite_resources", sr_tests_composite_resources},
        {"composite_geometry", sr_tests_composite_geometry},
        {"length", sr_tests_length},
        {"xml_lengths", sr_tests_xml_lengths},
        {"length_frame", sr_tests_length_frame},
        {"length_physics", sr_tests_length_physics},
        {"timeline", sr_tests_timeline}, {"curves", sr_tests_curves}, {"geometry", sr_tests_geometry},
        {"compositor", sr_tests_compositor}, {"color", sr_tests_color},
        {"vector", sr_tests_vector},     {"mesh", sr_tests_mesh},
        {"scene", sr_tests_scene}, {"property", sr_tests_property},       {"xml", sr_tests_xml},
        {"profile", sr_tests_profile}, {"styles", sr_tests_styles},
        {"metadata", sr_tests_metadata},
        {"camera", sr_tests_camera},     {"physics", sr_tests_physics},
        {"blend", sr_tests_blend},       {"group", sr_tests_group},
        {"raster", sr_tests_raster},     {"mask", sr_tests_mask},
        {"mask_path", sr_tests_mask_path},
        {"fuzz_mask_path", sr_tests_fuzz_mask_path},
        {"path", sr_tests_path},         {"image", sr_tests_image},
        {"encode", sr_tests_encode},     {"encode_faults", sr_tests_encode_faults},
        {"audio", sr_tests_audio},       {"video", sr_tests_video},
        {"fx", sr_tests_fx},             {"anim_color", sr_tests_anim_color},
        {"particles", sr_tests_particles}, {"deform", sr_tests_deform},
        {"shadow", sr_tests_shadow},     {"depth", sr_tests_depth},
        {"text", sr_tests_text},         {"args", sr_tests_args},
        {"resume", sr_tests_resume},
        /* Verification depth (P7). */
        {"golden", sr_tests_golden},     {"oom", sr_tests_oom},
    };
    if (argc > 2) {
        fprintf(stderr, "usage: %s [SUITE]\n", argv[0]);
        return EXIT_FAILURE;
    }
    const char *filter = argc > 1 ? argv[1] : NULL;
    av_log_set_level(AV_LOG_ERROR);   /* libav's own chatter, not ours */
    int run = 0, failed = 0;
    for (size_t s = 0; s < sizeof suites / sizeof suites[0]; ++s) {
        if (filter && strcmp(filter, suites[s].name) != 0) {
            continue;
        }
        for (const sr_test_case *c = suites[s].cases; c->name; ++c) {
            sr_test_ctx t = {c->name, 0};
            c->fn(&t);
            ++run;
            if (t.failures) {
                ++failed;
                fprintf(stderr, "[FAIL] %s.%s\n", suites[s].name, c->name);
            } else {
                printf("[ ok ] %s.%s\n", suites[s].name, c->name);
            }
        }
    }
    if (filter && run == 0) {
        fprintf(stderr, "unknown suite '%s'\n", filter);
    }
    printf("%d tests, %d failed\n", run, failed);
    return (failed == 0 && run > 0) ? EXIT_SUCCESS : EXIT_FAILURE;
}
