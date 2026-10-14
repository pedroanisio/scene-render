"""The crates listed in `tools/no_panic_crates.json` do not panic outside their tests, and the list only grows.

A listed crate denies, in every library and binary root, the clippy lints that catch a panic written into the code:

    #![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unreachable,
                                clippy::todo, clippy::unimplemented))]

`cargo clippy -- -D warnings` then rejects any such call outside `#[cfg(test)]` code. Where a panic marks a genuine
invariant, the item that holds it carries `#[expect(clippy::<lint>, reason = "...")]`; any other way of lowering the
lints in a listed crate (`allow`, `warn`, an inner attribute, `cfg_attr`, an `expect` without a reason) fails here.

The list may only grow: it must hold every crate of every version of the file in the history of HEAD. That needs the
whole history, so a shallow clone or a copy without `.git` fails with what to do about it instead of passing unchecked.
"""
import json
import os
import pathlib
import re
import shutil
import subprocess
import tempfile
import unittest
from unittest import mock

from test_panic_patterns import code_only, test_only_file_modules, without_test_code

ROOT = pathlib.Path(__file__).resolve().parents[2]
LIST = "tools/no_panic_crates.json"
SCHEMA_MAJOR = 1
KEYS = {"schema_version", "crates"}
LINTS = (
    "clippy::unwrap_used",
    "clippy::expect_used",
    "clippy::panic",
    "clippy::unreachable",
    "clippy::todo",
    "clippy::unimplemented",
)
# groups that hold the lints above: lowering one lowers them all
GROUPS = ("clippy::restriction",)
ROOT_KINDS = {"lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro", "bin"}

ATTRIBUTE_START = re.compile(r"#!?\[")
SEMVER = re.compile(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)")
CRATE_NAME = re.compile(r"[A-Za-z0-9_-]+")
DENIES = re.compile(r"(?:deny|forbid)\((?P<lints>[^()]*)\)|cfg_attr\(not\(test\),(?:deny|forbid)\((?P<gated>[^()]*)\)\)")
LOWERS = re.compile(r"\b(?:allow|expect|warn)\((?P<lints>[^()]*)\)")
DOCUMENTED = re.compile(r"expect\((?P<lints>[^()]*),reason=,?\)")
REASON = re.compile(r"\breason\s*=\s*\"(?P<text>(?:[^\"\\]|\\.)*)\"")


class ListError(ValueError):
    """The covered-crates file does not follow its schema."""


class HistoryUnavailable(RuntimeError):
    """The earlier versions of the covered-crates file cannot be read from this checkout."""


def parse_list(text):
    """The crate names in a covered-crates file, checked against its schema."""

    def unique_keys(pairs):
        keys = [k for k, _ in pairs]
        if duplicates := sorted({k for k in keys if keys.count(k) > 1}):
            raise ListError(f"duplicate keys: {', '.join(duplicates)}")
        return dict(pairs)

    try:
        data = json.loads(text, object_pairs_hook=unique_keys)
    except json.JSONDecodeError as e:
        raise ListError(f"not JSON: {e}") from None
    if not isinstance(data, dict):
        raise ListError("the top level must be an object")
    if unknown := sorted(set(data) - KEYS):
        raise ListError(f"unknown keys: {', '.join(unknown)}")
    if missing := sorted(KEYS - set(data)):
        raise ListError(f"missing keys: {', '.join(missing)}")
    version = data["schema_version"]
    if not isinstance(version, str) or not (m := SEMVER.fullmatch(version)):
        raise ListError(f"schema_version must be major.minor.patch, not {version!r}")
    if int(m.group(1)) != SCHEMA_MAJOR:
        raise ListError(f"schema_version {version} is not major version {SCHEMA_MAJOR}")
    crates = data["crates"]
    if not isinstance(crates, list):
        raise ListError("crates must be a list")
    for name in crates:
        if not isinstance(name, str) or not CRATE_NAME.fullmatch(name):
            raise ListError(f"not a crate name: {name!r}")
    if duplicates := sorted({c for c in crates if crates.count(c) > 1}):
        raise ListError(f"listed twice: {', '.join(duplicates)}")
    if crates != sorted(crates):
        raise ListError("crates must be in sorted order")
    return tuple(crates)


def attributes(code, inner=None):
    """(start, end, is_inner, body with all whitespace removed) of every attribute in literal-free `code`."""
    for m in ATTRIBUTE_START.finditer(code):
        is_inner = m.group().startswith("#!")
        if inner is not None and is_inner != inner:
            continue
        depth, j = 0, m.end() - 1
        while j < len(code):
            depth += {"[": 1, "]": -1}.get(code[j], 0)
            j += 1
            if depth == 0:
                break
        yield m.start(), j, is_inner, "".join(code[m.end() : j - 1].split())


def denied(text):
    """The lints that the inner attributes of crate root `text` deny or forbid outside test builds."""
    found = set()
    for _, _, _, body in attributes(code_only(text), inner=True):
        if m := DENIES.fullmatch(body):
            found.update((m.group("lints") or m.group("gated")).split(","))
    return found


def missing_lints(text):
    """The lints that crate root `text` should deny and does not, in the order of LINTS."""
    have = denied(text)
    return [lint for lint in LINTS if lint not in have]


def lowerings(text):
    """(line, problem) for every attribute in `text`, outside test code, that lowers a lint in LINTS other than an
    outer `#[expect(..., reason = "...")]` with a reason that says something."""
    code = without_test_code(code_only(text))
    problems = []
    for start, end, is_inner, body in attributes(code):
        names = {n for m in LOWERS.finditer(body) for n in m.group("lints").split(",")}
        hit = sorted(names & set(LINTS + GROUPS))
        if not hit:
            continue
        line = code.count("\n", 0, start) + 1
        what = ", ".join(hit)
        if is_inner:
            problems.append((line, f"lowers {what} for a whole module; put #[expect] on the item that holds the panic"))
        elif not DOCUMENTED.fullmatch(body):
            problems.append((line, f"lowers {what} other than by #[expect({what}, reason = \"...\")]"))
        elif not (r := REASON.search(text, start, end)) or not r.group("text").strip():
            problems.append((line, f"#[expect({what})] needs a reason that says why the panic cannot happen"))
        elif set(hit) & set(GROUPS):
            problems.append((line, f"expects the whole group {what}; name the lint"))
    return problems


def git(root, *args):
    return subprocess.run(
        ["git", "-c", "core.quotepath=off", "--no-pager", *args],
        cwd=root, capture_output=True, text=True, env={**os.environ, "GIT_OPTIONAL_LOCKS": "0"},
    )


def committed_lists(root, path=LIST):
    """(commit, crates) for every commit in the history of HEAD that touched `path`, newest first; a commit that
    deleted the file lists no crates. Raises HistoryUnavailable when that history is not all here."""
    if shutil.which("git") is None:
        raise HistoryUnavailable("git is not installed, so the earlier versions of the list cannot be read")
    inside = git(root, "rev-parse", "--is-inside-work-tree")
    if inside.returncode != 0 or inside.stdout.strip() != "true":
        raise HistoryUnavailable(f"{root} is not a git checkout; run this test in a clone with its history")
    shallow = git(root, "rev-parse", "--is-shallow-repository")
    if shallow.returncode != 0:
        raise HistoryUnavailable(f"git rev-parse --is-shallow-repository failed: {shallow.stderr.strip()}")
    if shallow.stdout.strip() != "false":
        raise HistoryUnavailable(
            "this clone is shallow, so the earlier versions of the list are missing: run `git fetch --unshallow`, "
            "or check out with `fetch-depth: 0` in CI"
        )
    head = git(root, "rev-parse", "--verify", "--quiet", "HEAD")
    if head.returncode != 0:
        return []
    log = git(root, "log", "--full-history", "--format=%H", "HEAD", "--", path)
    if log.returncode != 0:
        raise HistoryUnavailable(f"git log failed: {log.stderr.strip()}")
    versions = []
    for commit in log.stdout.split():
        if git(root, "cat-file", "-e", f"{commit}:{path}").returncode != 0:
            versions.append((commit, ()))
            continue
        show = git(root, "show", f"{commit}:{path}")
        if show.returncode != 0:
            raise HistoryUnavailable(f"git show {commit}:{path} failed: {show.stderr.strip()}")
        try:
            versions.append((commit, parse_list(show.stdout)))
        except ListError as e:
            raise ListError(f"{path} at {commit}: {e}") from None
    return versions


def shrinks(current, versions):
    """A message for every crate that an earlier version lists and `current` does not."""
    have = set(current)
    return [
        f"{name} was listed at {commit[:12]} and is missing now"
        for commit, crates in versions
        for name in crates
        if name not in have
    ]


def cargo_metadata(root=ROOT):
    cmd = ["cargo", "metadata", "--no-deps", "--format-version", "1", "--offline"]
    run = subprocess.run(cmd, cwd=root, capture_output=True, text=True)
    if run.returncode != 0:
        raise RuntimeError(f"{' '.join(cmd)} exited {run.returncode}:\n{run.stderr}")
    return json.loads(run.stdout)


def crate_roots(metadata):
    """{package name: [library and binary root files]} for every workspace member."""
    members = set(metadata["workspace_members"])
    return {
        p["name"]: sorted(pathlib.Path(t["src_path"]) for t in p["targets"] if ROOT_KINDS & set(t["kind"]))
        for p in metadata["packages"]
        if p["id"] in members
    }


def production_files(roots):
    """The Rust files compiled into crates with these roots, less the modules that `#[cfg(test)] mod x;` brings."""
    files = set(roots)
    for root in roots:
        files.update(root.parent.glob("**/*.rs"))
    skipped = set()
    for path in files:
        skipped.update(test_only_file_modules(path, code_only(path.read_text())))
    return sorted(p for p in files if not any(p == s or s in p.parents for s in skipped))


ALL_DENIED = (
    "#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unreachable,\n"
    "                            clippy::todo, clippy::unimplemented))]\n"
)


class RootDetectorTests(unittest.TestCase):
    def test_the_documented_attribute_denies_every_lint(self):
        self.assertEqual(missing_lints("//! A crate.\n#![forbid(unsafe_code)]\n" + ALL_DENIED + "pub fn f() {}\n"), [])

    def test_deny_and_forbid_without_cfg_attr_also_count(self):
        plain = "#![deny(" + ", ".join(LINTS) + ")]\n"
        self.assertEqual(missing_lints(plain), [])
        split = "#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]\n" \
                "#![forbid(clippy::unreachable, clippy::todo, clippy::unimplemented)]\n"
        self.assertEqual(missing_lints(split), [])

    def test_a_root_without_the_attribute_misses_every_lint(self):
        self.assertEqual(missing_lints("#![forbid(unsafe_code)]\npub fn f() {}\n"), list(LINTS))

    def test_a_lint_left_out_is_named(self):
        text = ALL_DENIED.replace(" clippy::panic,", "")
        self.assertEqual(missing_lints(text), ["clippy::panic"])

    def test_denying_only_under_test_does_not_count(self):
        self.assertEqual(missing_lints(ALL_DENIED.replace("not(test)", "test")), list(LINTS))
        self.assertEqual(missing_lints(ALL_DENIED.replace("not(test)", "feature = \"strict\"")), list(LINTS))

    def test_warn_or_an_outer_attribute_does_not_count(self):
        self.assertEqual(missing_lints(ALL_DENIED.replace("deny(", "warn(")), list(LINTS))
        self.assertEqual(missing_lints(ALL_DENIED.replace("#![", "#[") + "fn f() {}\n"), list(LINTS))

    def test_the_attribute_in_a_comment_or_a_string_does_not_count(self):
        self.assertEqual(missing_lints("// " + ALL_DENIED), list(LINTS))
        self.assertEqual(missing_lints("/*\n" + ALL_DENIED + "*/\n"), list(LINTS))
        self.assertEqual(missing_lints("const S: &str = r#\"\n" + ALL_DENIED + "\"#;\n"), list(LINTS))


class LoweringDetectorTests(unittest.TestCase):
    def test_an_expect_with_a_reason_is_the_one_allowance(self):
        text = '#[expect(clippy::expect_used, reason = "the table is embedded and checked by a test")]\nfn f() {}\n'
        self.assertEqual(lowerings(text), [])
        text = '#[expect(\n    clippy::unwrap_used,\n    clippy::panic,\n    reason = "an invariant",\n)]\nfn f() {}\n'
        self.assertEqual(lowerings(text), [])

    def test_allow_and_warn_are_rejected(self):
        for level in ("allow", "warn"):
            with self.subTest(level=level):
                text = f"fn a() {{}}\n#[{level}(clippy::unwrap_used)]\nfn f() {{}}\n"
                self.assertEqual([line for line, _ in lowerings(text)], [2])

    def test_an_expect_without_a_reason_or_with_an_empty_one_is_rejected(self):
        for attr in ("#[expect(clippy::panic)]", '#[expect(clippy::panic, reason = "  ")]'):
            with self.subTest(attr=attr):
                self.assertEqual(len(lowerings(attr + "\nfn f() {}\n")), 1)

    def test_an_inner_allowance_is_rejected_even_with_a_reason(self):
        text = '#![expect(clippy::unwrap_used, reason = "the whole module")]\n'
        self.assertIn("whole module", lowerings(text)[0][1])

    def test_lowering_through_cfg_attr_or_the_group_is_rejected(self):
        self.assertEqual(len(lowerings('#[cfg_attr(feature = "x", allow(clippy::todo))]\nfn f() {}\n')), 1)
        self.assertEqual(len(lowerings("#[allow(clippy::restriction)]\nfn f() {}\n")), 1)
        self.assertEqual(len(lowerings('#[expect(clippy::restriction, reason = "all of it")]\nfn f() {}\n')), 1)

    def test_other_lints_test_code_and_comments_are_not_lowerings(self):
        text = (
            "#[allow(clippy::too_many_arguments)]\nfn f() {}\n"
            "// #[allow(clippy::unwrap_used)]\n"
            "#[cfg(test)]\nmod tests {\n    #[allow(clippy::unwrap_used)]\n    fn g() {}\n}\n"
        )
        self.assertEqual(lowerings(text), [])


class ListParseTests(unittest.TestCase):
    def test_a_well_formed_list_parses(self):
        self.assertEqual(parse_list('{"schema_version": "1.2.0", "crates": ["a", "b-c"]}'), ("a", "b-c"))
        self.assertEqual(parse_list('{"schema_version": "1.0.0", "crates": []}'), ())

    def test_malformed_lists_are_rejected(self):
        bad = {
            "not json": "{",
            "not an object": '["a"]',
            "unknown key": '{"schema_version": "1.0.0", "crates": [], "note": "x"}',
            "missing key": '{"crates": []}',
            "duplicate key": '{"schema_version": "1.0.0", "crates": [], "crates": ["a"]}',
            "bad version": '{"schema_version": "1.0", "crates": []}',
            "other major": '{"schema_version": "2.0.0", "crates": []}',
            "crates not a list": '{"schema_version": "1.0.0", "crates": "a"}',
            "not a name": '{"schema_version": "1.0.0", "crates": ["a b"]}',
            "not a string": '{"schema_version": "1.0.0", "crates": [1]}',
            "listed twice": '{"schema_version": "1.0.0", "crates": ["a", "a"]}',
            "unsorted": '{"schema_version": "1.0.0", "crates": ["b", "a"]}',
        }
        for case, text in bad.items():
            with self.subTest(case=case), self.assertRaises(ListError):
                parse_list(text)


def write_list(directory, crates):
    target = directory / LIST
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps({"schema_version": "1.0.0", "crates": crates}))


class Repo:
    """A throwaway repository with fixed dates, so that its history is the same on every run."""

    ENV = {
        "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@example.com", "GIT_AUTHOR_DATE": "2000-01-01T00:00:00Z",
        "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@example.com",
        "GIT_COMMITTER_DATE": "2000-01-01T00:00:00Z", "GIT_CONFIG_NOSYSTEM": "1",
    }

    def __init__(self, path):
        self.path = path
        self.run("init", "-q")

    def run(self, *args):
        subprocess.run(["git", "-c", "core.hooksPath=/dev/null", "-c", "commit.gpgsign=false", *args],
                       cwd=self.path, check=True, capture_output=True, env={**os.environ, **self.ENV})

    def commit(self, crates):
        if crates is None:
            self.run("rm", "-q", LIST)
        else:
            write_list(self.path, crates)
            self.run("add", LIST)
        self.run("commit", "-q", "--allow-empty", "-m", "change the list")


@unittest.skipUnless(shutil.which("git"), "git is not installed")
class HistoryTests(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.path = pathlib.Path(self.dir.name) / "repo"
        self.path.mkdir()

    def test_a_list_that_grew_passes_and_one_that_shrank_names_the_crate(self):
        repo = Repo(self.path)
        repo.commit(["a"])
        repo.commit(["a", "b"])
        versions = committed_lists(self.path)
        self.assertEqual([crates for _, crates in versions], [("a", "b"), ("a",)])
        self.assertEqual(shrinks(("a", "b", "c"), versions), [])
        self.assertEqual(len(shrinks(("a", "c"), versions)), 1)
        self.assertIn("b was listed at", shrinks(("a", "c"), versions)[0])

    def test_a_crate_dropped_in_an_earlier_commit_is_still_caught(self):
        repo = Repo(self.path)
        repo.commit(["a", "b"])
        repo.commit(["a"])
        repo.commit(None)
        repo.commit(["a"])
        versions = committed_lists(self.path)
        self.assertEqual([crates for _, crates in versions], [("a",), (), ("a",), ("a", "b")])
        self.assertEqual(len(shrinks(("a",), versions)), 1)

    def test_a_crate_listed_only_on_a_merged_branch_is_still_caught(self):
        repo = Repo(self.path)
        repo.commit(["a"])
        repo.run("checkout", "-q", "-b", "side")
        repo.commit(["a", "b"])
        repo.commit(["a"])
        repo.run("checkout", "-q", "-")
        repo.commit(["a", "c"])
        repo.run("merge", "-q", "--no-ff", "-m", "merge the side branch", "side")
        versions = committed_lists(self.path)
        self.assertIn(("a", "b"), [crates for _, crates in versions])
        self.assertEqual(len(shrinks(("a", "c"), versions)), 1)
        self.assertIn("b was listed at", shrinks(("a", "c"), versions)[0])

    def test_a_malformed_earlier_version_is_an_error(self):
        repo = Repo(self.path)
        (self.path / "tools").mkdir()
        (self.path / LIST).write_text('{"crates": []}')
        repo.run("add", LIST)
        repo.run("commit", "-q", "-m", "a list without a version")
        with self.assertRaises(ListError):
            committed_lists(self.path)

    def test_a_repository_without_commits_has_no_earlier_versions(self):
        Repo(self.path)
        self.assertEqual(committed_lists(self.path), [])

    def test_a_shallow_clone_fails_with_what_to_do(self):
        repo = Repo(self.path)
        repo.commit(["a", "b"])
        repo.commit(["a", "b", "c"])
        clone = pathlib.Path(self.dir.name) / "clone"
        subprocess.run(["git", "clone", "-q", "--depth", "1", self.path.as_uri(), str(clone)], check=True,
                       capture_output=True)
        with self.assertRaisesRegex(HistoryUnavailable, "fetch-depth: 0"):
            committed_lists(clone)

    def test_a_copy_without_git_fails(self):
        write_list(self.path, ["a"])
        # a temporary directory inside some other repository must not find that repository
        ceiling = mock.patch.dict(os.environ, {"GIT_CEILING_DIRECTORIES": str(self.path.parent)})
        with ceiling, self.assertRaisesRegex(HistoryUnavailable, "not a git checkout"):
            committed_lists(self.path)


class WorkspaceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.listed = parse_list((ROOT / LIST).read_text())
        cls.roots = crate_roots(cargo_metadata())

    def test_every_listed_crate_is_a_workspace_member_with_a_root(self):
        for name in self.listed:
            with self.subTest(crate=name):
                self.assertIn(name, self.roots, f"{LIST} lists {name}, which is not a workspace member")
                self.assertTrue(self.roots[name], f"{name} has no library or binary target")

    def test_every_root_of_a_listed_crate_denies_the_panic_lints(self):
        problems = []
        for name in self.listed:
            for root in self.roots.get(name, []):
                if missing := missing_lints(root.read_text()):
                    problems.append(f"{root.relative_to(ROOT)}: does not deny {', '.join(missing)} outside tests")
        self.assertEqual(problems, [], "\n" + "\n".join(problems))

    def test_listed_crates_lower_the_lints_only_with_a_documented_expect(self):
        problems = []
        for name in self.listed:
            for path in production_files(self.roots.get(name, [])):
                for line, problem in lowerings(path.read_text()):
                    problems.append(f"{path.relative_to(ROOT)}:{line}: {problem}")
        self.assertEqual(problems, [], "\n" + "\n".join(problems))

    def test_the_list_holds_every_crate_it_ever_held(self):
        problems = shrinks(self.listed, committed_lists(ROOT))
        self.assertEqual(problems, [], "\n" + "\n".join(problems))


if __name__ == "__main__":
    unittest.main()
