/* SPDX-License-Identifier: Apache-2.0 */
#include "vector_path_internal.h"
#include "compositing_limits_internal.h"
#include "scene_render/random.h"

#include "harness.h"

static void check_input(sr_test_ctx *t, const char *text, uint64_t quota,
                         bool must_accept) {
    SrPreparedPath path = {0}, repeat = {0};
    SrPathParseInfo info, again;
    SrStatus status = sr_prepared_mask_path_parse(text, quota, &path, &info);
    CHECK(t, status == SR_OK || status == SR_ERR_ASSET);
    if (must_accept) CHECK_INT(t, status, SR_OK);
    CHECK_INT(t, sr_prepared_mask_path_parse(text, quota, &repeat, &again), status);
    CHECK_INT(t, again.error, info.error);
    CHECK_INT(t, again.byte_offset, info.byte_offset);
    CHECK(t, again.owned_bytes == info.owned_bytes);
    CHECK(t, info.byte_offset <= strlen(text));
    if (status == SR_OK) {
        SrPreparedPath legacy = {0};
        CHECK_INT(t, sr_prepared_path_parse(text, &legacy), SR_OK);
        CHECK(t, path.count > 0 && path.count <= SR_MAX_MASK_PATH_CONTOURS);
        CHECK_INT(t, path.count, legacy.count);
        CHECK_INT(t, path.count, repeat.count);
        size_t points = 0;
        uint64_t bytes = path.capacity * sizeof(*path.items);
        for (size_t i = 0; i < path.count; ++i) {
            const SrPathContour *c = &path.items[i];
            CHECK(t, c->count >= 2 && c->count <= c->capacity);
            points += c->count;
            bytes += c->capacity * sizeof(*c->points);
            for (size_t j = 0; j < c->count; ++j) {
                CHECK(t, isfinite(c->points[j].x) && isfinite(c->points[j].y));
                CHECK(t, fabs(c->points[j].x) <= SR_MAX_MASK_COORDINATE);
                CHECK(t, fabs(c->points[j].y) <= SR_MAX_MASK_COORDINATE);
            }
            if (i < legacy.count && i < repeat.count) {
                CHECK_INT(t, c->count, legacy.items[i].count);
                CHECK_INT(t, c->count, repeat.items[i].count);
                CHECK(t, c->closed == legacy.items[i].closed);
                CHECK(t, c->closed == repeat.items[i].closed);
                if (c->count == legacy.items[i].count)
                    CHECK(t, !memcmp(c->points, legacy.items[i].points,
                                      c->count * sizeof(*c->points)));
                if (c->count == repeat.items[i].count)
                    CHECK(t, !memcmp(c->points, repeat.items[i].points,
                                      c->count * sizeof(*c->points)));
            }
        }
        CHECK(t, points <= SR_MAX_MASK_PATH_POINTS);
        CHECK(t, bytes == info.owned_bytes && bytes <= quota);
        sr_prepared_path_free(&legacy);
    } else {
        CHECK(t, !path.items && !path.count && !path.capacity && !info.owned_bytes);
        CHECK(t, !repeat.items && !repeat.count && !repeat.capacity);
    }
    sr_prepared_path_free(&path);
    sr_prepared_path_free(&repeat);
}

static int generated_command(char *out, size_t size, uint64_t value) {
    int x = (int)(value % 17) - 8;
    int y = (int)((value >> 8) % 17) - 8;
    switch ((value >> 16) % 8) {
    case 0: return snprintf(out, size, " M%d %dH2", x, y);
    case 1: return snprintf(out, size, " l%d %d", x, y);
    case 2: return snprintf(out, size, " H%d V%d", x, y);
    case 3: return snprintf(out, size, " h%d v%d", x, y);
    case 4: return snprintf(out, size, " C%d %d 2 4 6 8", x, y);
    case 5: return snprintf(out, size, " c%d %d -2 4 6 -8", x, y);
    case 6: return snprintf(out, size, " Q%d %d 2 4", x, y);
    default: return snprintf(out, size, " q%d %d 2 -4z", x, y);
    }
}

static void seeded_valid_mutated_truncated(sr_test_ctx *t) {
    const uint64_t seeds[] = {0, 1, UINT64_C(0x47c1b36eaa9205d8), UINT64_MAX};
    const char mutations[] = "MmlLhHvVcCqQzZA?-,.+012eE \t\n\xff";
    for (size_t seed = 0; seed < sizeof(seeds) / sizeof(seeds[0]); ++seed) {
        for (uint64_t sample = 0; sample < 64; ++sample) {
            char text[1024], edited[1100];
            size_t used = (size_t)snprintf(text, sizeof(text), "M1 2L3 4");
            uint64_t value = sr_random_mix64(seeds[seed] ^ sample);
            for (unsigned command = 0; command < 16; ++command) {
                value = sr_random_mix64(value);
                int added = generated_command(text + used, sizeof(text) - used, value);
                CHECK(t, added > 0 && (size_t)added < sizeof(text) - used);
                if (added <= 0 || (size_t)added >= sizeof(text) - used) return;
                used += (size_t)added;
            }
            check_input(t, text, SR_MAX_COMPOSITE_BYTES, true);
            memcpy(edited, text, used + 1);
            edited[value % used] = mutations[(value >> 24) % (sizeof(mutations) - 1)];
            check_input(t, edited, SR_MAX_COMPOSITE_BYTES, false);
            memcpy(edited, text, used + 1);
            edited[(value >> 32) % used] = '\0';
            check_input(t, edited, SR_MAX_COMPOSITE_BYTES, false);
            snprintf(edited, sizeof(edited), "%s L1e309 0", text);
            check_input(t, edited, SR_MAX_COMPOSITE_BYTES, false);
            check_input(t, text, value % 4096, false);
        }
    }
}

const sr_test_case sr_tests_fuzz_mask_path[] = {
    {"seeded_valid_mutated_truncated", seeded_valid_mutated_truncated},
    {NULL, NULL}
};
