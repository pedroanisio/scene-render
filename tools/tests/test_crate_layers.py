"""The workspace crates stay in their layers: a crate builds only on crates in strictly lower layers.

LAYERS is the one statement of the layering. The README's list of layers is generated from it by
`tools/doc_facts.py` (fact `crate-layers`), and the tests below hold the real dependency graph, read from
`cargo metadata`, to it:

* a normal or build dependency between workspace members points to a strictly lower layer. Same-layer edges are
  refused: a layer is a set of peers, and a crate that needs a peer is a sign that the layer has split in two, which
  the table should then say. It also means a cycle of normal dependencies can never hide inside a layer;
* a dev dependency may point to any layer, but must not close a cycle with normal and build dependencies, because
  the crate under test would then be compiled twice and its types would not match its own test helpers;
* every member has a layer, and every crate in the table is a member;
* a crate in DEV_ONLY (test support) is never a normal or build dependency, whether or not it exists yet. When it
  does exist it sits above every layer, so it may build on anything that does not dev-depend on it.

Target-specific dependencies count like any other.
"""
import json
import pathlib
import shutil
import subprocess
import sys
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]

# (name, role, members), lowest layer first.
LAYERS = (
    ("kernels", "document-independent geometry, volumes, WebAssembly and FFmpeg I/O that everything shares",
     ("sr-media", "sr-vector", "sr-volume", "sr-wasm")),
    ("primitives", "document-independent text, 3D assets, simulation, geography and audio",
     ("sr-3d", "sr-audio", "sr-geo", "sr-sim", "sr-text")),
    ("model", "the document: validation, the typed model, asset verification", ("sr-model",)),
    ("evaluation", "documents to frames of drawable state", ("sr-eval",)),
    ("rendering", "frames to pixels on the GPU", ("sr-gpu",)),
    ("delivery", "audio mix, encode pipeline, outputs and destinations", ("sr-deliver",)),
    ("resolution", "the provider step that fills generated-media and transcription caches", ("sr-resolve",)),
    ("entry points", "the CLI and the fuzzing harness", ("scene-render", "sr-fuzz")),
)

# Crates that only tests may use. They need no entry in LAYERS: when one is a member, it sits in this layer, above
# every other.
DEV_ONLY = ("sr-testkit",)
TEST_SUPPORT = "test support"

RUNTIME_KINDS = ("normal", "build")


def layer_index(layers=LAYERS, dev_only=DEV_ONLY):
    """crate name -> (position, layer name); dev-only crates take the position above the top layer."""
    out = {}
    for i, (name, _, members) in enumerate(layers):
        for crate in members:
            out[crate] = (i, name)
    for crate in dev_only:
        out.setdefault(crate, (len(layers), TEST_SUPPORT))
    return out


def members(metadata):
    ids = set(metadata["workspace_members"])
    return {p["name"] for p in metadata["packages"] if p["id"] in ids}


def edges(metadata):
    """(from, to, kind, target) for every declared dependency of one member on another, on every target."""
    names = members(metadata)
    ids = set(metadata["workspace_members"])
    out = []
    for p in metadata["packages"]:
        if p["id"] not in ids:
            continue
        for d in p["dependencies"]:
            if d["name"] in names:
                out.append((p["name"], d["name"], d.get("kind") or "normal", d.get("target")))
    return sorted(out, key=lambda e: (e[0], e[1], e[2], e[3] or ""))


def _path(graph, start, goal):
    """A path start -> ... -> goal in graph (name -> set of names), or None."""
    seen, stack = {start}, [[start]]
    while stack:
        path = stack.pop()
        if path[-1] == goal:
            return path
        for nxt in sorted(graph.get(path[-1], ())):
            if nxt not in seen:
                seen.add(nxt)
                stack.append(path + [nxt])
    return None


def violations(metadata, layers=LAYERS, dev_only=DEV_ONLY):
    """Every way the dependency graph in metadata breaks the layering, as messages naming the edge and layers."""
    index = layer_index(layers, dev_only)
    names = members(metadata)
    problems = []

    listed = [c for _, _, ms in layers for c in ms]
    for crate in sorted({c for c in listed if listed.count(c) > 1}):
        problems.append(f"{crate} is listed in more than one layer")
    for crate in sorted(set(listed) & set(dev_only)):
        problems.append(f"{crate} is dev-only, so it belongs to no layer in the table")
    for crate in sorted(names - index.keys()):
        problems.append(f"{crate} is a workspace member with no layer: add it to LAYERS in {pathlib.Path(__file__).name}")
    for crate in sorted(set(listed) - names):
        problems.append(f"{crate} is in layer {index[crate][1]!r} but is not a workspace member")

    def where(crate):
        return f"{crate} ({index[crate][1]})" if crate in index else f"{crate} (no layer)"

    all_edges = edges(metadata)
    runtime = {}
    for src, dst, kind, _ in all_edges:
        if kind in RUNTIME_KINDS:
            runtime.setdefault(src, set()).add(dst)

    for src, dst, kind, target in all_edges:
        edge = f"{where(src)} -> {where(dst)} [{kind}{', ' + target if target else ''}]"
        if dst in dev_only and kind != "dev":
            problems.append(f"{edge}: {dst} is test support and may be only a dev dependency")
        elif kind in RUNTIME_KINDS and src in index and dst in index and index[dst][0] >= index[src][0]:
            relation = "the same layer" if index[dst][0] == index[src][0] else "a higher layer"
            problems.append(f"{edge}: a {kind} dependency must point to a strictly lower layer, not to {relation}")
        back = _path(runtime, dst, src)
        if back is not None:
            problems.append(f"{edge}: closes the cycle {' -> '.join([src] + back)} through normal and build dependencies")
    return problems


def cargo_metadata():
    cmd = ["cargo", "metadata", "--format-version", "1", "--offline", "--locked"]
    run = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
    if run.returncode != 0:
        raise RuntimeError(f"{' '.join(cmd)} exited {run.returncode}:\n{run.stderr}")
    return json.loads(run.stdout)


def render(layers=LAYERS, dev_only=DEV_ONLY):
    """The README's list of layers, the value of doc fact `crate-layers`."""
    lines = [f"{i}. **{name}**: {', '.join(f'`{c}`' for c in ms)}. {role[0].upper() + role[1:]}."
             for i, (name, role, ms) in enumerate(layers, 1)]
    lines.append(f"{len(layers) + 1}. **{TEST_SUPPORT}**: {', '.join(f'`{c}`' for c in dev_only)}, when present. "
                 "Only a dev dependency, never a normal or build one.")
    return "\n" + "\n".join(lines) + "\n"


def synthetic(deps, extra_members=()):
    """Metadata for members {name: [(dep, kind, target), ...]} in the shape `cargo metadata` prints."""
    crates = sorted(set(deps) | set(extra_members))
    packages = [{"id": f"path+file:///w/{c}#0.1.0", "name": c,
                 "dependencies": [{"name": d, "kind": None if k == "normal" else k, "target": t}
                                  for d, k, t in deps.get(c, ())]} for c in crates]
    packages.append({"id": "registry+https://example/serde#1.0.0", "name": "serde", "dependencies": []})
    return {"workspace_members": [p["id"] for p in packages if p["name"] != "serde"], "packages": packages}


TOY_LAYERS = (
    ("low", "the bottom", ("a", "b")),
    ("mid", "the middle", ("c",)),
    ("top", "the top", ("d",)),
)


def toy(**changes):
    """A valid graph over TOY_LAYERS, with crate entries replaced by changes."""
    deps = {
        "a": [],
        "b": [("a", "dev", None), ("serde", "normal", None)],
        "c": [("a", "normal", None), ("b", "build", None), ("d", "dev", None)],
        "d": [("b", "normal", None), ("a", "normal", 'cfg(target_os = "linux")')],
    }
    deps.update(changes)
    return synthetic(deps)


class CheckerTests(unittest.TestCase):
    """Each rule rejects a graph that breaks it, and names the edge and the layers."""

    def check(self, metadata, layers=TOY_LAYERS):
        return violations(metadata, layers, dev_only=("sr-testkit",))

    def test_a_valid_graph_passes(self):
        self.assertEqual(self.check(toy()), [])

    def test_an_upward_normal_edge_fails(self):
        problems = self.check(toy(a=[("c", "normal", None)]))
        self.assertIn("a (low) -> c (mid) [normal]: a normal dependency must point to a strictly lower layer, "
                      "not to a higher layer", problems)

    def test_an_upward_target_specific_build_edge_fails(self):
        problems = self.check(toy(a=[("d", "build", "cfg(windows)")]))
        self.assertIn("a (low) -> d (top) [build, cfg(windows)]: a build dependency must point to a strictly lower "
                      "layer, not to a higher layer", problems)

    def test_a_same_layer_edge_fails(self):
        problems = self.check(toy(b=[("a", "normal", None)]))
        self.assertIn("b (low) -> a (low) [normal]: a normal dependency must point to a strictly lower layer, "
                      "not to the same layer", problems)

    def test_a_dev_edge_upward_is_allowed(self):
        metadata = toy()
        self.assertIn(("c", "d", "dev", None), edges(metadata))
        self.assertEqual(self.check(metadata), [])

    def test_a_dev_edge_that_closes_a_cycle_fails(self):
        problems = self.check(toy(a=[("d", "dev", None)], c=[("a", "normal", None)], d=[("c", "normal", None)]))
        self.assertEqual(problems, ["a (low) -> d (top) [dev]: closes the cycle a -> d -> c -> a through normal and "
                                    "build dependencies"])

    def test_a_normal_cycle_fails_on_both_rules(self):
        problems = self.check(toy(a=[("c", "normal", None)]))
        self.assertIn("a (low) -> c (mid) [normal]: closes the cycle a -> c -> a through normal and build dependencies",
                      problems)

    def test_an_unassigned_crate_fails(self):
        problems = self.check(toy(e=[("a", "normal", None)]))
        self.assertEqual(problems, ["e is a workspace member with no layer: add it to LAYERS in test_crate_layers.py"])

    def test_a_layer_entry_that_is_not_a_member_fails(self):
        problems = self.check(toy(), layers=TOY_LAYERS + (("ghost", "gone", ("z",)),))
        self.assertEqual(problems, ["z is in layer 'ghost' but is not a workspace member"])

    def test_a_crate_in_two_layers_fails(self):
        layers = (("low", "", ("a", "b")), ("mid", "", ("c", "a")), ("top", "", ("d",)))
        self.assertIn("a is listed in more than one layer", self.check(toy(), layers))

    def test_testkit_may_be_absent(self):
        self.assertEqual(self.check(toy()), [])

    def test_testkit_as_a_dev_dependency_passes(self):
        metadata = toy(d=[("sr-testkit", "dev", None)], **{"sr-testkit": [("c", "normal", None)]})
        self.assertEqual(self.check(metadata), [])

    def test_testkit_as_a_normal_dependency_fails(self):
        metadata = toy(c=[("a", "normal", None), ("sr-testkit", "normal", None)], **{"sr-testkit": []})
        self.assertEqual(self.check(metadata), ["c (mid) -> sr-testkit (test support) [normal]: sr-testkit is test "
                                                "support and may be only a dev dependency"])

    def test_testkit_as_a_build_dependency_fails_even_before_it_is_a_member(self):
        metadata = toy(c=[("a", "normal", None)])
        # a dependency on a crate outside the workspace is not an edge; once sr-testkit joins, the edge is checked
        self.assertEqual(self.check(metadata), [])
        metadata = toy(c=[("sr-testkit", "build", None)], **{"sr-testkit": []})
        self.assertEqual(self.check(metadata), ["c (mid) -> sr-testkit (test support) [build]: sr-testkit is test "
                                                "support and may be only a dev dependency"])

    def test_testkit_that_depends_on_a_crate_testing_with_it_fails(self):
        metadata = toy(c=[("sr-testkit", "dev", None)], **{"sr-testkit": [("c", "normal", None)]})
        self.assertEqual(self.check(metadata), ["c (mid) -> sr-testkit (test support) [dev]: closes the cycle "
                                                "c -> sr-testkit -> c through normal and build dependencies"])

    def test_testkit_may_not_also_be_given_a_layer(self):
        layers = TOY_LAYERS + (("helpers", "", ("sr-testkit",)),)
        metadata = toy(**{"sr-testkit": []})
        self.assertIn("sr-testkit is dev-only, so it belongs to no layer in the table", self.check(metadata, layers))


@unittest.skipUnless(shutil.which("cargo"), "cargo is not installed")
class WorkspaceLayerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.metadata = cargo_metadata()

    def test_the_workspace_keeps_its_layers(self):
        self.assertEqual(violations(self.metadata), [])

    def test_the_graph_has_edges_of_every_kind_it_is_checked_for(self):
        kinds = {kind for _, _, kind, _ in edges(self.metadata)}
        self.assertIn("normal", kinds)
        self.assertIn("dev", kinds)

    def test_an_upward_edge_injected_into_the_real_graph_fails(self):
        metadata = json.loads(json.dumps(self.metadata))
        model = next(p for p in metadata["packages"]
                     if p["name"] == "sr-model" and p["id"] in metadata["workspace_members"])
        model["dependencies"].append({"name": "sr-gpu", "kind": None, "target": None})
        problems = violations(metadata)
        self.assertIn("sr-model (model) -> sr-gpu (rendering) [normal]: a normal dependency must point to a strictly "
                      "lower layer, not to a higher layer", problems)
        self.assertTrue(any(p.startswith("sr-model (model) -> sr-gpu (rendering) [normal]: closes the cycle")
                            for p in problems), problems)


class DocumentedLayersTests(unittest.TestCase):
    def test_the_readme_lists_the_layers_of_this_table(self):
        sys.path.insert(0, str(ROOT / "tools"))
        import doc_facts
        self.assertEqual(doc_facts.facts()["crate-layers"], render())
        readme = (ROOT / "README.md").read_text()
        self.assertIn(f"<!-- facts:crate-layers -->{render()}<!-- /facts -->", readme)

    def test_the_list_names_every_layer_in_order(self):
        text = render()
        positions = [text.index(f"**{name}**") for name, _, _ in LAYERS] + [text.index(f"**{TEST_SUPPORT}**")]
        self.assertEqual(positions, sorted(positions))


if __name__ == "__main__":
    unittest.main()
