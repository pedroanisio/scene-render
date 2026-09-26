/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SR_TEST_HARNESS_H
#define SR_TEST_HARNESS_H

#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

typedef struct sr_test_ctx {
    const char *name;
    int failures;
} sr_test_ctx;

typedef void (*sr_test_fn)(sr_test_ctx *t);

typedef struct sr_test_case {
    const char *name;
    sr_test_fn fn;
} sr_test_case;

#define SR_FAIL(t, ...)                                                   \
    do {                                                                  \
        fprintf(stderr, "  FAIL %s (%s:%d): ", (t)->name, __FILE__, __LINE__); \
        fprintf(stderr, __VA_ARGS__);                                     \
        fputc('\n', stderr);                                              \
        (t)->failures++;                                                  \
    } while (0)

#define CHECK(t, cond)                                                    \
    do {                                                                  \
        if (!(cond)) SR_FAIL(t, "%s", #cond);                             \
    } while (0)

#define CHECK_INT(t, a, b)                                                \
    do {                                                                  \
        long long a_ = (long long)(a), b_ = (long long)(b);               \
        if (a_ != b_) SR_FAIL(t, "%s == %lld, expected %lld", #a, a_, b_); \
    } while (0)

#define CHECK_NEAR(t, a, b, eps)                                          \
    do {                                                                  \
        double a_ = (double)(a), b_ = (double)(b);                        \
        if (!(fabs(a_ - b_) <= (eps)))                                    \
            SR_FAIL(t, "%s == %.9g, expected %.9g (eps %g)", #a, a_, b_, (double)(eps)); \
    } while (0)

#define CHECK_STR(t, a, b)                                                \
    do {                                                                  \
        const char *a_ = (a), *b_ = (b);                                  \
        if (!a_ || !b_ || strcmp(a_, b_) != 0)                            \
            SR_FAIL(t, "%s == \"%s\", expected \"%s\"", #a, a_ ? a_ : "(null)", b_ ? b_ : "(null)"); \
    } while (0)

#define CHECK_CONTAINS(t, hay, needle)                                    \
    do {                                                                  \
        const char *h_ = (hay);                                           \
        if (!h_ || !strstr(h_, (needle)))                                 \
            SR_FAIL(t, "\"%s\" does not contain \"%s\"", h_ ? h_ : "(null)", (needle)); \
    } while (0)

/* Paths are resolved from compile definitions set by the build:
 * SR_TEST_DATA_DIR is the repository root, SR_TEST_TMP_DIR a scratch
 * directory inside the build tree. Each returns one of four rotating static
 * buffers, so callers copy or use the result before the fifth next call.
 * Tests run single-threaded. */
const char *sr_test_data_path(const char *relative);
const char *sr_test_tmp_path(const char *name);

extern const sr_test_case sr_tests_timeline[];
extern const sr_test_case sr_tests_random[];
extern const sr_test_case sr_tests_blend_color[];
extern const sr_test_case sr_tests_skew[];
extern const sr_test_case sr_tests_compositing[];
extern const sr_test_case sr_tests_length[];
extern const sr_test_case sr_tests_xml_lengths[];
extern const sr_test_case sr_tests_length_frame[];
extern const sr_test_case sr_tests_length_physics[];
extern const sr_test_case sr_tests_curves[];
extern const sr_test_case sr_tests_geometry[];
extern const sr_test_case sr_tests_compositor[];
extern const sr_test_case sr_tests_color[];
extern const sr_test_case sr_tests_vector[];
extern const sr_test_case sr_tests_mesh[];
extern const sr_test_case sr_tests_scene[];
extern const sr_test_case sr_tests_property[];
extern const sr_test_case sr_tests_xml[];
extern const sr_test_case sr_tests_profile[];
extern const sr_test_case sr_tests_styles[];
extern const sr_test_case sr_tests_metadata[];
extern const sr_test_case sr_tests_camera[];
extern const sr_test_case sr_tests_physics[];
extern const sr_test_case sr_tests_blend[];
extern const sr_test_case sr_tests_encode[];
extern const sr_test_case sr_tests_encode_faults[];
extern const sr_test_case sr_tests_audio[];
extern const sr_test_case sr_tests_video[];
extern const sr_test_case sr_tests_group[];
extern const sr_test_case sr_tests_raster[];
extern const sr_test_case sr_tests_mask[];
extern const sr_test_case sr_tests_path[];
extern const sr_test_case sr_tests_mask_path[];
extern const sr_test_case sr_tests_fuzz_mask_path[];
extern const sr_test_case sr_tests_image[];
extern const sr_test_case sr_tests_fx[];
extern const sr_test_case sr_tests_anim_color[];
extern const sr_test_case sr_tests_particles[];
extern const sr_test_case sr_tests_deform[];
extern const sr_test_case sr_tests_shadow[];
extern const sr_test_case sr_tests_depth[];
extern const sr_test_case sr_tests_text[];
extern const sr_test_case sr_tests_args[];
extern const sr_test_case sr_tests_resume[];
/* Verification depth (P7): golden images and allocation-failure injection. */
extern const sr_test_case sr_tests_golden[];
extern const sr_test_case sr_tests_oom[];

#endif
