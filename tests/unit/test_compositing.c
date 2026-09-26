/* SPDX-License-Identifier: Apache-2.0 */
#include "fixture.h"
#include "scene_text.h"
#include "compositing_internal.h"
#include "xml_internal.h"
#include "scene_render/physics.h"
#include "scene_render/random.h"
#include "scene_render/renderer.h"

static SrStatus prepare_message(SrScene *scene, char **message) {
    FILE *sink = tmpfile();
    SrDiagnostics diag;
    sr_diag_init(&diag, "compositing-test", sink ? sink : stderr);
    SrStatus status = sr_scene_prepare_compositing(scene, &diag);
    if (sink) {
        fflush(sink);
        long size = ftell(sink);
        rewind(sink);
        *message = calloc(1, (size_t)size + 1);
        if (*message) (void)fread(*message, 1, (size_t)size, sink);
        fclose(sink);
    }
    return status;
}

static void lifecycle_and_legacy(sr_test_ctx *t) {
    SrScene scene;
    fx_scene(&scene, 16, 16);
    SrNode *node = fx_rect(&scene, NULL, 2, 2, 8, 8, (SrColor){1,0,0,1}, 1);
    CHECK(t, node != NULL);
    if (!node) {
        sr_scene_free(&scene);
        return;
    }
    SrFrame frame = {0};
    CHECK_INT(t, sr_frame_init(&frame, 16, 16), SR_OK);
    SrCompositor compositor;
    sr_compositor_init(&compositor, 4);
    CHECK_INT(t, sr_compositor_render(&compositor, &scene, 0, &frame, NULL), SR_OK);
    CHECK(t, !scene.compositing && !scene.compositing_required);
    node->blend = SR_BLEND_COLOR_BURN;
    CHECK_INT(t, sr_compositor_render(&compositor, &scene, 0, &frame, NULL),
              SR_ERR_RENDER);
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
    CHECK(t, scene.compositing_required && scene.compositing);
    CHECK_INT(t, scene.compositing->count, 2);
    CHECK_INT(t, sr_compositor_render(&compositor, &scene, 0, &frame, NULL), SR_OK);
    sr_scene_invalidate_compositing(&scene);
    node->transform.skew_x.base = 20;
    CHECK_INT(t, sr_compositor_render(&compositor, &scene, 0, &frame, NULL),
              SR_ERR_RENDER);
    CHECK_INT(t, sr_compositor_render_scene(&compositor, &scene, 0, &frame, NULL),
              SR_ERR_RENDER);
    CHECK_INT(t, sr_physics_prepare(&scene, NULL), SR_ERR_RENDER);
    SrRenderOptions options = {.validate_only=true};
    SrRenderMetrics metrics;
    CHECK_INT(t, sr_render(&scene, &options, &metrics, NULL), SR_ERR_RENDER);
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
    CHECK_INT(t, sr_compositor_render(&compositor, &scene, 0, &frame, NULL), SR_OK);
    sr_scene_invalidate_compositing(&scene);
    node->blend = SR_BLEND_NORMAL;
    node->transform.skew_x.base = 0;
    /* Removing a feature does not silently revive invalidated authored data. */
    CHECK_INT(t, sr_composite_scene_ready(&scene, NULL), SR_ERR_RENDER);
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
    sr_compositor_free(&compositor);
    sr_frame_free(&frame);
    sr_scene_free(&scene);
    sr_scene_invalidate_compositing(NULL);
    sr_composite_plan_free(NULL);
    CHECK_INT(t, sr_scene_prepare_compositing(NULL, NULL), SR_ERR_ARGUMENT);
    SrScene empty = {0};
    CHECK_INT(t, sr_scene_prepare_compositing(&empty, NULL), SR_ERR_ARGUMENT);
    CHECK(t, empty.compositing_required && !empty.compositing);
    CHECK_INT(t, sr_composite_plan_build(&empty, NULL, NULL), SR_ERR_ARGUMENT);
}

static void invalid_ownership_and_diagnostics(sr_test_ctx *t) {
    SrNode root = {.source_line=1}, child = {.source_line=37};
    SrNode *children[2] = {&child, &child};
    root.children = children;
    root.child_count = 1;
    SrScene scene = {.root=&root};
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
    sr_scene_invalidate_compositing(&scene);
    root.child_count = 2;
    char *message = NULL;
    CHECK_INT(t, prepare_message(&scene, &message), SR_ERR_RENDER);
    CHECK_CONTAINS(t, message, ":37:");
    CHECK_CONTAINS(t, message, "<group> @children");
    CHECK_CONTAINS(t, message, "shared child");
    free(message);
    CHECK(t, !scene.compositing);
    root.child_count = 1;
    SrNode *back = &root;
    child.children = &back;
    child.child_count = 1;
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_ERR_RENDER);
    child.child_count = 0;
    child.children = NULL;
    children[0] = NULL;
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_ERR_RENDER);
    children[0] = &child;
    child.child_count = 1;
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_ERR_RENDER);
    child.child_count = 0;
    child.mask_count = 1;
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_ERR_RENDER);
    child.mask_count = 0;
    child.child_count = SIZE_MAX;
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_ERR_RENDER);
    child.child_count = 0;
    child.mask_count = SIZE_MAX;
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_ERR_RENDER);
    child.mask_count = 0;
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
    sr_scene_invalidate_compositing(&scene);
}

static void exact_structural_limits(sr_test_ctx *t) {
    const size_t maximum = SR_MAX_COMPOSITE_NODES;
    SrNode root = {0};
    SrScene scene = {.root=&root};
    SrNode *nodes = sr_alloc(maximum * sizeof(*nodes));
    SrNode **children = sr_alloc((maximum - 1) * sizeof(*children));
    CHECK(t, nodes && children);
    if (!nodes || !children) {
        free(nodes);
        free(children);
        return;
    }
    root.children = children;
    root.child_count = maximum - 1;
    for (size_t i = 0; i < root.child_count; ++i) children[i] = &nodes[i];
    /* Inactive nodes still count; no relative-length flag is involved. */
    nodes[maximum - 2].transform.skew_x.base = 1;
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
    CHECK_INT(t, scene.compositing->count, maximum);
    CHECK(t, !scene.has_relative_lengths);
    sr_scene_invalidate_compositing(&scene);
    SrNode *extra = &nodes[maximum - 1];
    nodes[maximum - 2].children = &extra;
    nodes[maximum - 2].child_count = 1;
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_ERR_RENDER);
    root.child_count = 0;
    root.children = NULL;
    free(children);
    free(nodes);

    size_t depth = SR_MAX_COMPOSITE_DEPTH;
    nodes = sr_alloc((depth + 1) * sizeof(*nodes));
    children = sr_alloc(depth * sizeof(*children));
    CHECK(t, nodes && children);
    if (!nodes || !children) {
        free(nodes);
        free(children);
        return;
    }
    scene.root = nodes;
    for (size_t i = 0; i < depth; ++i) {
        children[i] = &nodes[i + 1];
        nodes[i].children = &children[i];
        nodes[i].child_count = i + 1 < depth;
    }
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
    CHECK_INT(t, scene.compositing->nodes[depth - 1].depth, depth);
    sr_scene_invalidate_compositing(&scene);
    nodes[depth - 1].child_count = 1;
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_ERR_RENDER);
    free(children);
    free(nodes);

    SrMask *masks = sr_alloc((SR_MAX_COMPOSITE_MASKS + 1u) * sizeof(*masks));
    CHECK(t, masks != NULL);
    if (!masks) return;
    SrNode child = {0}, *link = &child;
    root = (SrNode){.children=&link, .child_count=1, .masks=masks,
                    .mask_count=SR_MAX_COMPOSITE_MASKS / 2};
    child.masks = masks + root.mask_count;
    child.mask_count = root.mask_count;
    scene.root = &root;
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
    CHECK_INT(t, scene.compositing->mask_count, SR_MAX_COMPOSITE_MASKS);
    CHECK_INT(t, scene.compositing->nodes[1].mask_offset, root.mask_count);
    sr_scene_invalidate_compositing(&scene);
    ++child.mask_count;
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_ERR_RENDER);
    free(masks);
}

static void lookup_order_and_immutable_frames(sr_test_ctx *t) {
    SrScene scene;
    fx_scene(&scene, 16, 16);
    SrNode *group = fx_add(&scene, NULL, SR_NODE_GROUP);
    CHECK(t, group != NULL);
    if (!group) {
        sr_scene_free(&scene);
        return;
    }
    group->transform.skew_x.base = 5;
    for (size_t i = 0; i < 257; ++i)
        CHECK(t, fx_rect(&scene, group, 1, 2, 5, 4, (SrColor){1,0,0,1}, 1));
    uint64_t seed = 9274;
    for (size_t i = group->child_count; i > 1; --i) {
        seed = sr_random_mix64(seed);
        size_t j = (size_t)(seed % i);
        SrNode *tmp = group->children[j];
        group->children[j] = group->children[i - 1];
        group->children[i - 1] = tmp;
    }
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
    SrCompositePlan *plan = scene.compositing;
    if (!plan) {
        sr_scene_free(&scene);
        return;
    }
    for (size_t i = 0; i < group->child_count; ++i) {
        const SrCompositeNode *entry = sr_composite_plan_node(plan, group->children[i]);
        CHECK(t, entry == &plan->nodes[i + 2]);
        if (entry) {
            CHECK_INT(t, entry->parent, 1);
            CHECK_INT(t, entry->depth, 3);
        }
    }
    SrNode absent = {0};
    CHECK(t, !sr_composite_plan_node(plan, &absent));
    CHECK(t, !sr_composite_plan_node(NULL, &absent));
    CHECK(t, !sr_composite_plan_node(plan, NULL));
    uint64_t before = sr_fnv1a64(SR_FNV_OFFSET, plan->nodes,
                                  plan->count * sizeof(*plan->nodes));
    SrFrame frames[2] = {{0}};
    SrCompositor warm, other;
    sr_compositor_init(&warm, 1);
    sr_compositor_init(&other, 4);
    CHECK_INT(t, sr_frame_init(&frames[0], 16, 16), SR_OK);
    CHECK_INT(t, sr_frame_init(&frames[1], 16, 16), SR_OK);
    const float clear[4] = {0};
    const double times[] = {.5, 0, .25, .5};
    for (size_t i = 0; i < 4; ++i) {
        sr_frame_clear(&frames[0], clear, 1);
        sr_frame_clear(&frames[1], clear, 1);
        CHECK_INT(t, sr_compositor_render(&warm, &scene, times[i], &frames[0], NULL), SR_OK);
        CHECK_INT(t, sr_compositor_render(&other, &scene, times[i], &frames[1], NULL), SR_OK);
        CHECK(t, st_frames_equal(&frames[0], &frames[1]));
    }
    CHECK(t, before == sr_fnv1a64(SR_FNV_OFFSET, plan->nodes,
                                  plan->count * sizeof(*plan->nodes)));
    CHECK(t, scene.compositing == plan);
    sr_compositor_free(&warm);
    sr_compositor_free(&other);
    sr_frame_free(&frames[0]);
    sr_frame_free(&frames[1]);
    sr_scene_free(&scene);
}

static void xml_prepares_and_preflights(sr_test_ctx *t) {
    const char *attrs[] = {"blend=\"hard-light\"", "skewX=\"12\"", "", ""};
    const char *animation = "<animate property=\"skew.y\">"
        "<key time=\"0\" value=\"0\"/></animate>";
    for (size_t i = 0; i < 4; ++i) {
        char xml[1024];
        snprintf(xml, sizeof(xml), "<scene version=\"1.1\">"
            "<project width=\"16\" height=\"16\" fps=\"1\" duration=\"1\"/>"
            "<composition><group id=\"g\" %s>%s</group></composition></scene>",
            attrs[i], i == 2 ? animation : "");
        SrScene scene;
        SrStatus status = st_load(t, "compositing-ready.xml", xml, &scene, NULL);
        CHECK_INT(t, status, SR_OK);
        if (status != SR_OK) continue;
        CHECK(t, !scene.has_relative_lengths);
        CHECK_INT(t, scene.compositing != NULL, i != 3);
        CHECK_INT(t, scene.compositing_required, i != 3);
        sr_scene_free(&scene);
    }
    /* Exercise the loader's preflight ordering directly with an ownership
     * cycle: recursive resolve_nodes must never get control of this graph. */
    SrNode root = {.source_line=5}, *back = &root;
    root.children = &back;
    root.child_count = 1;
    SrScene scene = {.root=&root, .compositing_required=true};
    SrDiagnostics diag;
    FILE *sink;
    st_diag(&diag, &sink);
    ParseContext context = {.scene=&scene, .diag=&diag};
    CHECK(t, !sr_xml_resolve_scene(&context));
    CHECK_INT(t, diag.errors, 1);
    CHECK(t, !context.out_of_memory && !scene.compositing);
    if (sink) fclose(sink);
}

static void xml_repreparation_keeps_indices(sr_test_ctx *t) {
    const char *xml = "<scene version=\"1.1\">"
        "<project width=\"16\" height=\"16\" fps=\"1\" duration=\"1\"/>"
        "<composition><group id=\"a\" z=\"10\" blend=\"hard-light\">"
        "<mask type=\"rect\" width=\"3\" height=\"3\"/></group>"
        "<group id=\"b\" z=\"0\">"
        "<mask type=\"rect\" width=\"4\" height=\"4\"/>"
        "<mask type=\"ellipse\" width=\"2\" height=\"2\"/>"
        "</group></composition></scene>";
    SrScene scene;
    SrStatus status = st_load(t, "compositing-order.xml", xml, &scene, NULL);
    CHECK_INT(t, status, SR_OK);
    if (status != SR_OK) return;
    const SrNode *nodes[2] = {sr_scene_find_node(&scene, "a"),
                             sr_scene_find_node(&scene, "b")};
    size_t indices[2], offsets[2];
    for (size_t i = 0; i < 2; ++i) {
        const SrCompositeNode *entry = sr_composite_plan_node(scene.compositing, nodes[i]);
        CHECK(t, entry != NULL);
        indices[i] = entry ? (size_t)(entry - scene.compositing->nodes) : SIZE_MAX;
        offsets[i] = entry ? entry->mask_offset : SIZE_MAX;
    }
    CHECK(t, scene.root->children[0] == nodes[1]);
    CHECK(t, scene.compositing->nodes[1].node == nodes[1]);
    sr_scene_invalidate_compositing(&scene);
    CHECK_INT(t, sr_scene_prepare_compositing(&scene, NULL), SR_OK);
    for (size_t i = 0; i < 2; ++i) {
        const SrCompositeNode *entry = sr_composite_plan_node(scene.compositing, nodes[i]);
        CHECK(t, entry != NULL);
        if (entry) {
            CHECK_INT(t, (size_t)(entry - scene.compositing->nodes), indices[i]);
            CHECK_INT(t, entry->mask_offset, offsets[i]);
        }
    }
    sr_scene_free(&scene);
}

const sr_test_case sr_tests_compositing[] = {
    {"lifecycle_and_legacy", lifecycle_and_legacy},
    {"invalid_ownership_and_diagnostics", invalid_ownership_and_diagnostics},
    {"exact_structural_limits", exact_structural_limits},
    {"lookup_order_and_immutable_frames", lookup_order_and_immutable_frames},
    {"xml_prepares_and_preflights", xml_prepares_and_preflights},
    {"xml_repreparation_keeps_indices", xml_repreparation_keeps_indices},
    {NULL, NULL}
};
