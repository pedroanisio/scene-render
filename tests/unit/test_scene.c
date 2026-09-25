/* SPDX-License-Identifier: Apache-2.0 */
#include "scene_render/scene.h"

#include "harness.h"

/* Regression: sorting an empty group used to hand qsort a NULL base
 * pointer, which is undefined behaviour (UBSan: "null pointer passed as
 * argument 1"). */
static void test_sort_empty_group(sr_test_ctx *t)
{
    SrScene scene;
    sr_scene_init(&scene);
    CHECK(t, scene.root != NULL);
    if (!scene.root) return;
    CHECK(t, scene.root->children == NULL);
    CHECK_INT(t, scene.root->child_count, 0);
    sr_node_sort_children(scene.root);
    CHECK_INT(t, scene.root->child_count, 0);

    SrNode *empty = sr_node_create(&scene, SR_NODE_GROUP);
    CHECK(t, empty != NULL);
    if (empty) {
        CHECK(t, sr_node_add_child(scene.root, empty) == SR_OK);
        sr_node_sort_children(scene.root);
        CHECK_INT(t, scene.root->child_count, 1);
        CHECK_INT(t, empty->child_count, 0);
    }
    sr_node_sort_children(NULL);
    sr_scene_free(&scene);
}

static void test_sort_orders_by_z_then_order(sr_test_ctx *t)
{
    SrScene scene;
    sr_scene_init(&scene);
    if (!scene.root) { CHECK(t, scene.root != NULL); return; }
    SrNode *a = sr_node_create(&scene, SR_NODE_GROUP);
    SrNode *b = sr_node_create(&scene, SR_NODE_GROUP);
    SrNode *c = sr_node_create(&scene, SR_NODE_GROUP);
    CHECK(t, a && b && c);
    if (!a || !b || !c) {
        sr_node_free(a); sr_node_free(b); sr_node_free(c);
        sr_scene_free(&scene);
        return;
    }
    a->z = 2; b->z = 1; c->z = 1;
    CHECK(t, sr_node_add_child(scene.root, a) == SR_OK);
    CHECK(t, sr_node_add_child(scene.root, b) == SR_OK);
    CHECK(t, sr_node_add_child(scene.root, c) == SR_OK);
    sr_node_sort_children(scene.root);
    CHECK(t, scene.root->children[0] == b);
    CHECK(t, scene.root->children[1] == c);
    CHECK(t, scene.root->children[2] == a);
    sr_scene_free(&scene);
}

const sr_test_case sr_tests_scene[] = {
    {"sort_empty_group", test_sort_empty_group},
    {"sort_orders_by_z_then_order", test_sort_orders_by_z_then_order},
    {NULL, NULL},
};
