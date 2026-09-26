/* SPDX-License-Identifier: Apache-2.0 */
#include "scene_render/random.h"
#include "scene_render/particles.h"
#include "random_internal.h"
#include "harness.h"

static void mix_vectors(sr_test_ctx *t) {
    static const struct { uint64_t input, output; } cases[] = {
        {0, UINT64_C(0xe220a8397b1dcdaf)},
        {1, UINT64_C(0x910a2dec89025cc1)},
        {7, UINT64_C(0x63cbe1e459320dd7)},
        {UINT64_MAX, UINT64_C(0xe4d971771b652c20)},
        {UINT64_C(0x8000000000000000), UINT64_C(0x481ec0a212a9f3db)},
        {UINT64_C(0x0123456789abcdef), UINT64_C(0x157a3807a48faa9d)},
    };
    for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); ++i) {
        CHECK(t, sr_random_mix64(cases[i].input) == cases[i].output);
        CHECK(t, sr_random_mix64_inline(cases[i].input) == cases[i].output);
    }
}

/* Fixed values pin the old particle API as well as its extracted helpers.
 * In particular, index wraparound and streams outside 0..7 are intentional. */
static void particle_vectors(sr_test_ctx *t) {
    static const struct {
        uint64_t seed, index;
        unsigned stream;
        double expected;
    } cases[] = {
        {0, 0, 0, 0x1.4e0dba5e9a32fp-1},
        {0, 0, 7, 0x1.7169852efd579p-1},
        {7, 1, 0, 0x1.9e962cdf86d2ep-1},
        {7, 1, 4, 0x1.2cfb7416e3385p-1},
        {UINT64_MAX, UINT64_MAX, 0, 0x1.dd30481103f78p-1},
        {UINT64_MAX, UINT64_MAX, 0xffffffffu, 0x1.dcf307912c032p-1},
        {0, UINT64_C(0x2000000000000000), 0, 0x1.4e0dba5e9a32fp-1},
        {0, 0, 8, 0x1.110b9099b02b2p-2},
        {123, 987654321, 3, 0x1.041a283da3a8ep-2},
    };
    for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); ++i) {
        CHECK_NEAR(t, sr_random_particle_value(cases[i].seed, cases[i].index,
                    cases[i].stream), cases[i].expected, 0);
        CHECK_NEAR(t, sr_particles_random(cases[i].seed, cases[i].index,
                    cases[i].stream), cases[i].expected, 0);
        CHECK_NEAR(t, sr_random_particle_value_inline(cases[i].seed,
                    cases[i].index, cases[i].stream), cases[i].expected, 0);
    }
}

static void seed_vectors(sr_test_ctx *t) {
    static const struct { const char *id; uint64_t hash; } cases[] = {
        {NULL, UINT64_C(0x14650fb0739d0383)},
        {"", UINT64_C(0x14650fb0739d0383)},
        {"p", UINT64_C(0x44bd9bd473cdb5e9)},
        {"burst", UINT64_C(0xa1dda1e9209422e7)},
        {"scope/child", UINT64_C(0x77e4e9995ca5e788)},
        {"\xc3\xa9", UINT64_C(0x99850a00c486b16b)},
    };
    SrScene scene = {0};
    SrNode node = {0};
    scene.project.seed = 7;
    for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); ++i) {
        CHECK(t, sr_random_particle_seed(0, cases[i].id) == cases[i].hash);
        CHECK(t, sr_random_particle_seed(UINT64_MAX, cases[i].id) ==
                 (UINT64_MAX ^ cases[i].hash));
        node.id = (char *)cases[i].id;
        CHECK(t, sr_particles_seed(&scene, &node) == (cases[i].hash ^ 7));
    }
    node.particle_seed_set = true;
    node.particle_seed = UINT64_MAX;
    CHECK(t, sr_particles_seed(&scene, &node) == UINT64_MAX);
    node.particle_seed = 0;
    CHECK(t, sr_particles_seed(&scene, &node) == 0);
}

const sr_test_case sr_tests_random[] = {
    {"mix_vectors", mix_vectors},
    {"particle_vectors", particle_vectors},
    {"seed_vectors", seed_vectors},
    {NULL, NULL}
};
