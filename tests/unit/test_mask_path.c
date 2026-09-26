/* SPDX-License-Identifier: Apache-2.0 */
#include "vector_path_internal.h"
#include "compositing_limits_internal.h"

#include <stdlib.h>

#include "harness.h"

static SrPathParseInfo parse(sr_test_ctx *t, const char *text, uint64_t quota,
                              SrPathParseError error, size_t offset) {
    SrPreparedPath path = {0};
    SrPathParseInfo info = {SR_PATH_PARSE_MEMORY, SIZE_MAX, UINT64_MAX};
    CHECK_INT(t, sr_prepared_mask_path_parse(text, quota, &path, &info),
              error == SR_PATH_PARSE_OK ? SR_OK : SR_ERR_ASSET);
    CHECK_INT(t, info.error, error);
    CHECK_INT(t, info.byte_offset, offset);
    if (error == SR_PATH_PARSE_OK) {
        uint64_t bytes = path.capacity * sizeof(*path.items);
        for (size_t i = 0; i < path.count; ++i)
            bytes += path.items[i].capacity * sizeof(*path.items[i].points);
        CHECK(t, info.owned_bytes == bytes && bytes <= quota);
    } else {
        CHECK(t, !path.items && !path.count && !path.capacity);
        CHECK(t, info.owned_bytes == 0);
    }
    sr_prepared_path_free(&path);
    return info;
}

static void diagnostics_and_coordinates(sr_test_ctx *t) {
    static const struct {
        const char *text;
        SrPathParseError error;
        size_t offset;
    } cases[] = {
        {NULL, SR_PATH_PARSE_SYNTAX, 0},
        {"", SR_PATH_PARSE_SYNTAX, 0},
        {"M0 0", SR_PATH_PARSE_SYNTAX, 4},
        {"M0 0L1", SR_PATH_PARSE_SYNTAX, 6},
        {"1 2", SR_PATH_PARSE_SYNTAX, 0},
        {"L0 0", SR_PATH_PARSE_SYNTAX, 0},
        {"H1", SR_PATH_PARSE_SYNTAX, 0},
        {"M0 0A1", SR_PATH_PARSE_SYNTAX, 4},
        {"M0 0H?", SR_PATH_PARSE_SYNTAX, 5},
        {"M0 0V?", SR_PATH_PARSE_SYNTAX, 5},
        {"M0 0C1 2 3", SR_PATH_PARSE_SYNTAX, 10},
        {"M0 0Q1 2 3", SR_PATH_PARSE_SYNTAX, 10},
        {"Mnan 0L1 1", SR_PATH_PARSE_COORDINATE, 1},
        {"M0 infL1 1", SR_PATH_PARSE_COORDINATE, 3},
        {"M0 0H1e309", SR_PATH_PARSE_COORDINATE, 5},
        {"M1000000001 0H0", SR_PATH_PARSE_COORDINATE, 1},
        {"M0 -1000000001H0", SR_PATH_PARSE_COORDINATE, 3},
        {"M1e9 0h1", SR_PATH_PARSE_COORDINATE, 6},
        {"M0 -1e9v-1", SR_PATH_PARSE_COORDINATE, 7},
        {"M1e9 0m1 0H0", SR_PATH_PARSE_COORDINATE, 6},
        {"M0 1e9l0 1", SR_PATH_PARSE_COORDINATE, 6},
        {"M1e9 0c1 0 -1 0 -1 0", SR_PATH_PARSE_COORDINATE, 6},
        {"M1e9 0c-1 0 1 0 -1 0", SR_PATH_PARSE_COORDINATE, 6},
        {"M1e9 0c-1 0 -1 0 1 0", SR_PATH_PARSE_COORDINATE, 6},
        {"M1e9 0q1 0 -1 0", SR_PATH_PARSE_COORDINATE, 6},
        {"M1e9 0q-1 0 1 0", SR_PATH_PARSE_COORDINATE, 6},
        /* Raw controls are bounded even if the curve itself stays in range. */
        {"M0 0Q1000000001 0 0 0", SR_PATH_PARSE_COORDINATE, 5},
        {"M0 0C1000000001 0 0 0 0 0", SR_PATH_PARSE_COORDINATE, 5}
    };
    for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); ++i)
        parse(t, cases[i].text, SR_MAX_COMPOSITE_BYTES,
              cases[i].error, cases[i].offset);
    parse(t, "M-1e9 1e9L1e9 -1e9", UINT64_MAX, SR_PATH_PARSE_OK, 0);
    SrPathParseInfo info;
    CHECK_INT(t, sr_prepared_mask_path_parse("M0 0H1", 1000, NULL, &info),
              SR_ERR_ARGUMENT);
    CHECK_INT(t, info.error, SR_PATH_PARSE_ARGUMENT);
    CHECK(t, !info.byte_offset && !info.owned_bytes);
    CHECK_INT(t, sr_prepared_mask_path_parse(NULL, 0, NULL, NULL), SR_ERR_ARGUMENT);
    SrPreparedPath path = {0};
    CHECK_INT(t, sr_prepared_mask_path_parse(NULL, 0, &path, NULL), SR_ERR_ASSET);
    CHECK_INT(t, sr_prepared_mask_path_parse("M0 0H1", 1000, &path, NULL), SR_OK);
    sr_prepared_path_free(&path);
    /* The new mask restrictions must not affect legacy path acceptance. */
    CHECK_INT(t, sr_prepared_path_parse("M0 0L3000000000 0", &path), SR_OK);
    sr_prepared_path_free(&path);
}

static void same_geometry_and_coverage(sr_test_ctx *t) {
    const char *texts[] = {
        "M.5 .5h2v2h-2z",
        "Z m1 1 2 0 0 2 -2 0z M4 4 L8 4 8 8 4 8Z",
        "M1 2 C3 14 11 -2 14 12 L2 12 Z M3 3 Q10 1 12 10 L4 12Z",
        "M1 2c2 12 10 -4 13 10l-12 0z m2 1q7 -2 9 7l-8 2z",
        "M0x1p0 1e0h2.5v3.5z"
    };
    for (size_t i = 0; i < sizeof(texts) / sizeof(texts[0]); ++i) {
        SrPreparedPath legacy = {0}, mask = {0};
        SrPathParseInfo info;
        CHECK_INT(t, sr_prepared_path_parse(texts[i], &legacy), SR_OK);
        CHECK_INT(t, sr_prepared_mask_path_parse(texts[i], SR_MAX_COMPOSITE_BYTES,
                                                 &mask, &info), SR_OK);
        CHECK_INT(t, legacy.count, mask.count);
        for (size_t j = 0; j < mask.count && j < legacy.count; ++j) {
            const SrPathContour *a = &legacy.items[j], *b = &mask.items[j];
            CHECK_INT(t, a->count, b->count);
            CHECK_INT(t, a->capacity, b->capacity);
            CHECK(t, a->closed == b->closed);
            if (a->count == b->count)
                CHECK(t, !memcmp(a->points, b->points, a->count * sizeof(*a->points)));
        }
        for (int rule = SR_FILL_NONZERO; rule <= SR_FILL_EVENODD; ++rule) {
            float a[256], b[256];
            CHECK_INT(t, sr_prepared_path_coverage(&legacy, (SrFillRule)rule,
                                                   0, 16, 16, a, NULL), SR_OK);
            CHECK_INT(t, sr_prepared_path_coverage(&mask, (SrFillRule)rule,
                                                   0, 16, 16, b, NULL), SR_OK);
            CHECK(t, !memcmp(a, b, sizeof(a)));
        }
        sr_prepared_path_free(&legacy);
        sr_prepared_path_free(&mask);
    }
}

static void curves_on_coordinate_boundary(sr_test_ctx *t) {
    const char *texts[] = {
        "M1000000000 0 Q1000000000 1 1000000000 2",
        "M-1000000000 0 Q-1000000000 1 -1000000000 2",
        "M1000000000 0 q0 1 0 2",
        "M-1000000000 0 q0 1 0 2",
        "M0 1000000000 Q1 1000000000 2 1000000000",
        "M0 -1000000000 Q1 -1000000000 2 -1000000000",
        "M0 1000000000 q1 0 2 0",
        "M0 -1000000000 q1 0 2 0"
    };
    for (size_t i = 0; i < sizeof(texts) / sizeof(texts[0]); ++i) {
        SrPreparedPath path = {0};
        SrPathParseInfo info;
        CHECK_INT(t, sr_prepared_mask_path_parse(texts[i], SR_MAX_COMPOSITE_BYTES,
                                                 &path, &info), SR_OK);
        CHECK_INT(t, info.error, SR_PATH_PARSE_OK);
        if (path.count) {
            CHECK_INT(t, path.items[0].count, 13);
            for (size_t j = 0; j < path.items[0].count; ++j) {
                SrPathPoint point = path.items[0].points[j];
                double constant = i < 4 ? point.x : point.y;
                CHECK_NEAR(t, constant, i % 2 ? -1e9 : 1e9, 1e-6);
                CHECK(t, fabs(constant) <= SR_MAX_MASK_COORDINATE);
            }
        }
        sr_prepared_path_free(&path);
    }
}

static char *append(char *at, const char *text) {
    size_t length = strlen(text);
    memcpy(at, text, length + 1);
    return at + length;
}

static void exact_input_and_command_limits(sr_test_ctx *t) {
    char *text = sr_alloc(SR_MAX_MASK_PATH_BYTES + 2u);
    CHECK(t, text != NULL);
    if (!text) return;
    memset(text, ' ', SR_MAX_MASK_PATH_BYTES + 1u);
    memcpy(text, "M0 0H1", 6);
    text[SR_MAX_MASK_PATH_BYTES] = '\0';
    parse(t, text, SR_MAX_COMPOSITE_BYTES, SR_PATH_PARSE_OK, 0);
    text[SR_MAX_MASK_PATH_BYTES] = ' ';
    text[SR_MAX_MASK_PATH_BYTES + 1u] = '\0';
    parse(t, text, SR_MAX_COMPOSITE_BYTES, SR_PATH_PARSE_BYTES, SR_MAX_MASK_PATH_BYTES);
    /* Close commands count even when they do not add geometry. */
    char *at = append(text, "M0 0H1");
    for (size_t i = 2; i < SR_MAX_MASK_PATH_COMMANDS; ++i) at = append(at, "Z");
    parse(t, text, SR_MAX_COMPOSITE_BYTES, SR_PATH_PARSE_OK, 0);
    size_t offset = (size_t)(at - text);
    append(at, "Z");
    parse(t, text, SR_MAX_COMPOSITE_BYTES, SR_PATH_PARSE_COMMANDS, offset);
    /* Each implicit H repetition is a command too. */
    at = append(text, "M0 0H");
    for (size_t i = 1; i < SR_MAX_MASK_PATH_COMMANDS; ++i) at = append(at, "1 ");
    parse(t, text, SR_MAX_COMPOSITE_BYTES, SR_PATH_PARSE_OK, 0);
    offset = (size_t)(at - text);
    append(at, "1");
    parse(t, text, SR_MAX_COMPOSITE_BYTES, SR_PATH_PARSE_COMMANDS, offset);
    free(text);
}

static void exact_contour_and_point_limits(sr_test_ctx *t) {
    char *text = sr_alloc(SR_MAX_MASK_PATH_BYTES + 1u);
    CHECK(t, text != NULL);
    if (!text) return;
    char *at = text;
    for (size_t i = 0; i < SR_MAX_MASK_PATH_CONTOURS; ++i)
        at = append(at, "M0 0H1 ");
    parse(t, text, SR_MAX_COMPOSITE_BYTES, SR_PATH_PARSE_OK, 0);
    size_t offset = (size_t)(at - text);
    append(at, "M0 0H1");
    parse(t, text, SR_MAX_COMPOSITE_BYTES, SR_PATH_PARSE_CONTOURS, offset);
    /* Exercise aggregate flattened points over two contours. */
    at = append(text, "M0 0H1 M0 0");
    size_t points = 3;
    while (points + 16 <= SR_MAX_MASK_PATH_POINTS) {
        at = append(at, "C0 0 0 0 0 0");
        points += 16;
    }
    while (points < SR_MAX_MASK_PATH_POINTS) {
        at = append(at, "H1");
        ++points;
    }
    CHECK(t, (size_t)(at - text) < SR_MAX_MASK_PATH_BYTES);
    parse(t, text, SR_MAX_COMPOSITE_BYTES, SR_PATH_PARSE_OK, 0);
    offset = (size_t)(at - text);
    /* The closing point must count too. */
    append(at, "Z");
    parse(t, text, SR_MAX_COMPOSITE_BYTES, SR_PATH_PARSE_POINTS, offset);
    append(at, "Q0 0 0 0");
    parse(t, text, SR_MAX_COMPOSITE_BYTES, SR_PATH_PARSE_POINTS, offset);
    append(at, "C0 0 0 0 0 0");
    parse(t, text, SR_MAX_COMPOSITE_BYTES, SR_PATH_PARSE_POINTS, offset);
    free(text);
}

static void live_storage_quota(sr_test_ctx *t) {
    const uint64_t contours = 4 * sizeof(SrPathContour);
    const uint64_t points = 16 * sizeof(SrPathPoint);
    parse(t, "M0 0H1", 0, SR_PATH_PARSE_STORAGE, 0);
    parse(t, "M0 0H1", contours - 1, SR_PATH_PARSE_STORAGE, 0);
    parse(t, "M0 0H1", contours + points - 1, SR_PATH_PARSE_STORAGE, 0);
    SrPathParseInfo info = parse(t, "M0 0H1", contours + points, SR_PATH_PARSE_OK, 0);
    CHECK(t, info.owned_bytes == contours + points);
    /* A cubic grows from 16 to 32 points. Both buffers must fit together. */
    const char *curve = "M0 0C1 0 1 1 0 1";
    uint64_t peak = contours + 3 * points;
    parse(t, curve, peak - 1, SR_PATH_PARSE_STORAGE, 4);
    info = parse(t, curve, peak, SR_PATH_PARSE_OK, 0);
    CHECK(t, info.owned_bytes == contours + 2 * points);
    /* Reallocating the contour array must include all existing point arrays. */
    char many[128], *at = many;
    for (size_t i = 0; i < 17; ++i) at = append(at, "M0 0H1 ");
    peak = 12 * contours + 16 * points;
    parse(t, many, peak - 1, SR_PATH_PARSE_STORAGE, 112);
    info = parse(t, many, peak, SR_PATH_PARSE_OK, 0);
    CHECK(t, info.owned_bytes == 8 * contours + 17 * points);
}

const sr_test_case sr_tests_mask_path[] = {
    {"diagnostics_and_coordinates", diagnostics_and_coordinates},
    {"same_geometry_and_coverage", same_geometry_and_coverage},
    {"curves_on_coordinate_boundary", curves_on_coordinate_boundary},
    {"exact_input_and_command_limits", exact_input_and_command_limits},
    {"exact_contour_and_point_limits", exact_contour_and_point_limits},
    {"live_storage_quota", live_storage_quota},
    {NULL, NULL}
};
