"""`unsafe` stays inside a short, audited list of items, and every one of them says why it is sound.

The workspace denies `unsafe_code` (`[workspace.lints.rust]` in `Cargo.toml`) and every member inherits that with
`[lints] workspace = true`. The level is `deny`, not `forbid`, so that an audited item can opt out with
`#[allow(unsafe_code)]`; a crate root with no such item forbids `unsafe_code` outright. This test fails when a member
stops inheriting the lint, when the workspace level is lowered, when anything lowers the lint outside the items in
ALLOWED, or when an `unsafe` block or `unsafe impl` in an allowed item has no `// SAFETY:` comment directly above it.
A block inside an `unsafe impl` is covered by the comment on the impl.
"""
import json
import os
import pathlib
import re
import shutil
import subprocess
import tomllib
import unittest

from test_panic_patterns import ATTRIBUTE, SPACE, code_only, item_end, production_sources

ROOT = pathlib.Path(__file__).resolve().parents[2]

# The items that may lower `unsafe_code`, as (file, item) and the reason. The list may only shrink: an entry that
# excuses nothing fails the test, so an item that no longer needs `unsafe` is removed here in the same change. To add
# an entry, put `#[allow(unsafe_code)]` on the narrowest item that holds the `unsafe`, give each block a `// SAFETY:`
# comment, and say in the pull request why safe code cannot do the same.
ALLOWED = {
    # production code
    ("crates/sr-text/src/font.rs", "fn cached_face"):
        "a cached rustybuzz face borrows the font bytes that the same cache entry owns",
    ("crates/sr-volume/src/openvdb/codec.rs", "fn block"):
        "the Blosc decompressor is reached only through its C API",
    ("crates/sr-gpu/src/gpu.rs", "fn experimental_features"):
        "wgpu asks for an unsafe acknowledgement before it enables ray queries, an experimental feature",
    # test code: a #[global_allocator] that counts allocations and forwards them to System
    ("crates/sr-eval/tests/crater_memory.rs", "unsafe impl GlobalAlloc for ObservedSystem"): "allocation count",
    ("crates/sr-eval/tests/terrain_memory.rs", "unsafe impl GlobalAlloc for ObservedSystem"): "allocation count",
    ("crates/sr-fuzz/tests/parsers.rs", "unsafe impl GlobalAlloc for Counting"): "allocation count",
    ("crates/sr-geo/tests/pmtiles_memory.rs", "unsafe impl GlobalAlloc for ObservedSystem"): "allocation count",
    ("crates/sr-gpu/tests/drape_memory.rs", "unsafe impl GlobalAlloc for Counting"): "allocation count",
    ("crates/sr-sim/src/pyro/voxel_memory.rs", "unsafe impl GlobalAlloc for Counting"): "allocation count",
    ("crates/sr-sim/tests/pyro_blast_memory.rs", "unsafe impl GlobalAlloc for Counting"): "allocation count",
    ("crates/sr-sim/tests/pyro_export_memory.rs", "unsafe impl GlobalAlloc for Counting"): "allocation count",
    ("crates/sr-vector/tests/cpu_allocations.rs", "unsafe impl GlobalAlloc for Counting"): "allocation count",
}

# An attribute that lowers the lint: allow, expect or warn, also inside cfg_attr.
LOWERS = re.compile(r"\b(?:allow|expect|warn)\s*\([^()]*\bunsafe_code\b")
FORBIDS = re.compile(r"#!\[\s*(?:cfg_attr\s*\(\s*not\s*\(\s*test\s*\)\s*,\s*)?forbid\s*\([^()]*\bunsafe_code\b")
ATTRIBUTE_START = re.compile(r"#!?\[")
UNSAFE = re.compile(r"\bunsafe\b\s*(\{|impl\b)?")
FN = re.compile(r"(?:pub(?:\([^)]*\))?\s+)?(?:(?:async|const|unsafe|extern(?:\s+\S+)?)\s+)*fn\s+(\w+)")
VISIBILITY = re.compile(r"pub(?:\([^)]*\))?\s+")
WHOLE_FILE = "the whole module (inner attribute)"
SKIPPED_DIRS = {"target", "vendor", "_tmp", "node_modules"}


def attribute_end(code, i):
    """Where the attribute that starts at `i` (at its `#`) ends, in comment- and literal-free `code`."""
    depth, j = 0, code.index("[", i)
    while j < len(code):
        depth += {"[": 1, "]": -1}.get(code[j], 0)
        j += 1
        if depth == 0:
            break
    return j


def label(code, start, end):
    """A short name for the item or statement in code[start:end]: `fn name`, or the header of an impl or other item."""
    header = " ".join(re.split(r"[{;]", code[start:end], maxsplit=1)[0].split())
    if m := FN.match(header):
        return f"fn {m.group(1)}"
    return re.split(r"\swhere\s", VISIBILITY.sub("", header, count=1) if header.startswith("pub") else header)[0]


def documented(lines, index):
    """Whether the comments directly above line `index` (attributes between them are skipped) hold `SAFETY:`."""
    i = index - 1
    while i >= 0:
        line = lines[i].strip()
        if line.startswith("//"):
            if "SAFETY:" in line:
                return True
        elif not line.startswith("#["):
            return False
        i -= 1
    return False


def scan(text):
    """Every attribute in Rust source `text` that lowers `unsafe_code`, as (line, item label, undocumented lines).

    The undocumented lines are those of `unsafe` blocks and impls in the item that lack a `// SAFETY:` comment.
    """
    code, lines = code_only(text), text.splitlines()
    found = []
    for m in ATTRIBUTE_START.finditer(code):
        end = attribute_end(code, m.start())
        if not LOWERS.search(code, m.start(), end):
            continue
        line = code.count("\n", 0, m.start()) + 1
        if code[m.start() + 1] == "!":
            found.append((line, WHOLE_FILE, []))
            continue
        j = end
        while (a := ATTRIBUTE.match(code, j)) is not None:
            j = attribute_end(code, a.end() - 2)
        start = SPACE.match(code, j).end()
        stop = item_end(code, start)
        undocumented, covered = [], []
        for u in UNSAFE.finditer(code, start, stop):
            if u.group(1) is None or any(a <= u.start() < b for a, b in covered):
                continue
            at = code.count("\n", 0, u.start())
            if documented(lines, at):
                if u.group(1) == "impl":
                    covered.append((u.start(), item_end(code, u.start())))
            else:
                undocumented.append(at + 1)
        found.append((line, label(code, start, stop), undocumented))
    return found


def rust_sources(root=ROOT):
    """Every Rust file in the repository outside build output, vendored crates and scratch space."""
    for here, dirs, files in os.walk(root):
        dirs[:] = sorted(d for d in dirs if d not in SKIPPED_DIRS and not d.startswith("."))
        for name in sorted(files):
            if name.endswith(".rs"):
                yield pathlib.Path(here) / name


def members(manifest, root=ROOT):
    """The member directories of the workspace whose root manifest is `manifest` (a parsed Cargo.toml)."""
    workspace = manifest["workspace"]
    excluded = {(root / e).resolve() for e in workspace.get("exclude", [])}
    found = []
    for pattern in workspace.get("members", []):
        for path in sorted(root.glob(pattern)):
            if (path / "Cargo.toml").is_file() and path.resolve() not in excluded:
                found.append(path)
    return found


def lint_problems(manifest, member_manifests):
    """What weakens the lint configuration: the root manifest, and each member's parsed manifest by its name."""
    problems = []
    level = manifest.get("workspace", {}).get("lints", {}).get("rust", {}).get("unsafe_code")
    if isinstance(level, dict):
        level = level.get("level")
    if level != "deny":
        problems.append(f"[workspace.lints.rust] unsafe_code is {level!r}, not 'deny'")
    for name, member in sorted(member_manifests.items()):
        if member.get("lints") != {"workspace": True}:
            problems.append(f"{name}/Cargo.toml: [lints] is {member.get('lints')!r}, not {{workspace = true}}")
    return problems


class UnsafePolicyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.manifest = tomllib.loads((ROOT / "Cargo.toml").read_text())
        cls.sites, cls.undocumented, cls.whole_file = set(), [], []
        for path in rust_sources():
            name = path.relative_to(ROOT).as_posix()
            for line, item, undocumented in scan(path.read_text()):
                if item == WHOLE_FILE:
                    cls.whole_file.append(f"{name}:{line}")
                    continue
                cls.sites.add((name, item))
                cls.undocumented += [f"{name}:{n} (in {item})" for n in undocumented]

    def test_every_member_inherits_a_denied_unsafe_code_lint(self):
        found = {path.relative_to(ROOT).as_posix(): tomllib.loads((path / "Cargo.toml").read_text())
                 for path in members(self.manifest)}
        self.assertGreater(len(found), 1)
        self.assertEqual(lint_problems(self.manifest, found), [])

    def test_no_inner_attribute_lowers_unsafe_code(self):
        self.assertEqual(self.whole_file, [], "put #[allow(unsafe_code)] on the item that holds the unsafe instead")

    def test_unsafe_code_is_lowered_only_on_allowed_items(self):
        self.assertEqual(sorted(self.sites - set(ALLOWED)), [],
                         "write it without unsafe, or add the item to ALLOWED in this file with its reason")

    def test_every_allowed_item_is_still_there(self):
        self.assertEqual(sorted(set(ALLOWED) - self.sites), [], "remove these from ALLOWED: they no longer need it")

    def test_every_allowed_unsafe_block_states_its_invariant(self):
        self.assertEqual(self.undocumented, [], "add a `// SAFETY:` comment directly above each of these")

    @unittest.skipUnless(shutil.which("cargo"), "cargo is not installed")
    def test_crate_roots_without_allowed_production_items_forbid_unsafe_code(self):
        metadata = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--locked", "--format-version", "1"], cwd=ROOT))
        production = {p.resolve() for p in production_sources()}
        exempt = {(ROOT / f).resolve().parents[0] for f, _ in ALLOWED if (ROOT / f).resolve() in production}
        missing = []
        for package in metadata["packages"]:
            src = pathlib.Path(package["manifest_path"]).parent / "src"
            for target in package["targets"]:
                if {"test", "bench"} & set(target["kind"]):
                    continue
                root = pathlib.Path(target["src_path"]).resolve()
                if root.is_relative_to(src) and any(d.is_relative_to(src) for d in exempt):
                    continue
                if not FORBIDS.search(code_only(root.read_text())):
                    missing.append(root.relative_to(ROOT).as_posix())
        self.assertEqual(sorted(missing), [], "add #![forbid(unsafe_code)] to these crate roots")

    def test_the_scanner_finds_lowered_lints_and_undocumented_blocks(self):
        source = "\n".join([
            "// #[allow(unsafe_code)] in a comment",                             # 1
            'const S: &str = "#[allow(unsafe_code)]";',                         # 2
            "#[allow(unsafe_code)]",                                            # 3: found, documented
            "pub(crate) fn good(p: *const u8) -> u8 {",                         # 4
            "    // SAFETY: the caller passes a live pointer",                  # 5
            "    unsafe { *p }",                                                # 6
            "}",                                                                # 7
            "#[cfg_attr(feature = \"x\", allow(dead_code, unsafe_code))]",      # 8: found
            "#[inline]",                                                        # 9
            "fn bad(p: *const u8) -> u8 {",                                     # 10
            "    let v = unsafe { *p };",                                       # 11: undocumented
            "    // SAFETY: as above",                                          # 12
            "    v + unsafe { *p }",                                            # 13
            "}",                                                                # 14
            "#[expect(unsafe_code)]",                                           # 15: found
            "unsafe impl<T> Send for Wrapper<T> where T: Sync {}",              # 16: undocumented
            "// SAFETY: forwards to System",                                    # 17
            "#[allow(unsafe_code)]",                                            # 18: found, documented impl
            "unsafe impl GlobalAlloc for Counting {",                           # 19
            "    unsafe fn alloc(&self, l: Layout) -> *mut u8 { unsafe { System.alloc(l) } }",
            "}",                                                                # 21
            "#![warn(unsafe_code)]",                                            # 22: found, whole file
            "#![forbid(unsafe_code)]",                                          # 23
            "#[allow(clippy::all)]",                                            # 24
            "fn other() {}",                                                    # 25
        ])
        self.assertEqual(scan(source), [
            (3, "fn good", []),
            (8, "fn bad", [11]),
            (15, "unsafe impl<T> Send for Wrapper<T>", [16]),
            (18, "unsafe impl GlobalAlloc for Counting", []),
            (22, WHOLE_FILE, []),
        ])

    def test_lint_problems_name_each_weakened_setting(self):
        denied = {"workspace": {"lints": {"rust": {"unsafe_code": "deny"}}}}
        self.assertEqual(lint_problems(denied, {"crates/a": {"lints": {"workspace": True}}}), [])
        self.assertEqual(lint_problems({"workspace": {"lints": {"rust": {"unsafe_code": {"level": "deny"}}}}}, {}), [])
        for weaker in ("warn", "allow", "forbid", None):
            self.assertEqual(len(lint_problems({"workspace": {"lints": {"rust": {"unsafe_code": weaker}}}}, {})), 1)
        self.assertEqual(lint_problems({"workspace": {}}, {}), ["[workspace.lints.rust] unsafe_code is None, not 'deny'"])
        missing = lint_problems(denied, {"crates/a": {}, "crates/b": {"lints": {"rust": {"unsafe_code": "allow"}}},
                                         "crates/c": {"lints": {"workspace": False}}})
        self.assertEqual([p.split("/Cargo.toml")[0] for p in missing], ["crates/a", "crates/b", "crates/c"])

    def test_members_come_from_the_manifest_with_globs_and_excludes(self):
        manifest = {"workspace": {"members": ["crates/*", "crates/scene-render"], "exclude": ["crates/sr-model"]}}
        found = [p.relative_to(ROOT).as_posix() for p in members(manifest)]
        self.assertIn("crates/sr-text", found)
        self.assertIn("crates/scene-render", found)
        self.assertNotIn("crates/sr-model", found)


if __name__ == "__main__":
    unittest.main()
