/* SPDX-License-Identifier: Apache-2.0 */
#include "scene_render/mesh.h"

#include <stdlib.h>

#include "harness.h"

static void test_load_octahedron_obj(sr_test_ctx *t)
{
    const char *path = sr_test_data_path("examples/assets/octahedron.obj");
    FILE *sink = tmpfile();
    CHECK(t, sink != NULL);
    if (!sink) return;
    SrDiagnostics diag;
    sr_diag_init(&diag, path, sink);
    SrMesh *mesh = NULL;
    CHECK(t, sr_mesh_load_obj(path, &mesh, 1, &diag) == SR_OK);
    CHECK(t, mesh && mesh->triangle_count == 8);
    if (mesh) {
        free(mesh->triangles);
        free(mesh);
    }
    fclose(sink);
}

const sr_test_case sr_tests_mesh[] = {
    {"load_octahedron_obj", test_load_octahedron_obj},
    {NULL, NULL},
};
