"""The release hygiene checker: every rule catches what it names, the staged check reads the index and not the
working tree, and the review tripwire skips the phrases that are not a diary."""
import importlib.util
import pathlib
import subprocess
import sys
import tempfile
import hashlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "tools" / "check_release_hygiene.py"
spec = importlib.util.spec_from_file_location("hygiene", SCRIPT)
hygiene = importlib.util.module_from_spec(spec)
spec.loader.exec_module(hygiene)


def run(*args, cwd=None):
    return subprocess.run([sys.executable, str(SCRIPT), *args], cwd=cwd, capture_output=True, text=True)


class Repo:
    """A throwaway git repository the checker is pointed at with --root."""

    def __init__(self):
        self.dir = tempfile.TemporaryDirectory()
        self.path = pathlib.Path(self.dir.name)
        for cmd in (["init", "-q"], ["config", "user.email", "t@example.com"], ["config", "user.name", "t"]):
            subprocess.run(["git", *cmd], cwd=self.path, check=True)

    def write(self, name, text):
        target = self.path / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text)

    def commit(self):
        # no hooks: a user's global commit hooks have no business in a throwaway repository
        subprocess.run(["git", "-c", "core.hooksPath=/dev/null", "commit", "-q", "-m", "first"], cwd=self.path,
                       check=True)

    def stage(self, name):
        subprocess.run(["git", "add", name], cwd=self.path, check=True)

    def check(self, *args):
        return run("--root", str(self.path), *args)


def findings(path, text):
    """The checker's errors and warnings for one file with this text."""
    with tempfile.TemporaryDirectory() as d:
        full = pathlib.Path(d) / path
        full.parent.mkdir(parents=True, exist_ok=True)
        full.write_text(text)
        return hygiene.check([path], d)


class PathRules(unittest.TestCase):
    def test_each_path_rule_names_what_it_rejects(self):
        for rule, path in [
            ("A1", "crates/sr-gpu/CLAUDE.md"), ("A1", "AGENTS.md"), ("A2", ".claude/settings.json"), ("A3", ".github/copilot-instructions.md"),
            ("A4", "docs/prompts/x.md"), ("A5", "notes/PLAN-x.md"), ("A6", "build.log"), ("A7", "x-findings.md"),
        ]:
            bad, _ = hygiene.check([path], "/nonexistent")
            self.assertTrue(any(f" {rule} " in b for b in bad), f"{rule} on {path}: {bad}")

    def test_the_root_project_guidelines_are_releasable(self):
        # CLAUDE.md at the root states the project's binding contributor rules, as CONTRIBUTING.md does
        bad, _ = hygiene.check(["CLAUDE.md"], "/nonexistent")
        self.assertEqual(bad, [])


class TextRules(unittest.TestCase):
    def test_each_text_rule_names_what_it_rejects(self):
        for rule, line in [
            ("B1", "see /home/someone/work/file"), ("B2", "kept in .claude/state"), ("B3", "done in Phase 2"),
            ("B4", "as in the Python renderer"), ("B5", "see D12 for why"), ("B6", "not verified on the target"),
            ("B7", "decided in this session"),
        ]:
            bad, _ = findings("notes.md", line + "\n")
            self.assertTrue(any(f": {rule} " in b for b in bad), f"{rule} on {line!r}: {bad}")

    def test_a_decision_code_in_code_is_judged_only_in_a_comment(self):
        bad, _ = findings("x.rs", "let D2 = 1;\n")
        self.assertEqual(bad, [])
        bad, _ = findings("x.rs", "let a = 1; // see D2\n")
        self.assertTrue(any(": B5 " in b for b in bad), bad)

    def test_slashes_inside_a_string_or_a_url_are_not_a_comment(self):
        bad, _ = findings("x.rs", 'let u = "http://example.com/D2"; let a = "a // D2";\n')
        self.assertEqual(bad, [])
        bad, _ = findings("x.rs", 'let s = "x"; // D2 explains it\n')
        self.assertTrue(any(": B5 " in b for b in bad), bad)

    def test_the_vendored_sources_are_not_read(self):
        bad, warn = findings("vendor/gpu-allocator/src/lib.rs", "// it used to work, see Phase 2 in /home/a/b/\n")
        self.assertEqual((bad, warn), ([], []))


class Tripwire(unittest.TestCase):
    def warns(self, text):
        return bool(findings("x.md", text + "\n")[1])

    def test_a_diary_is_flagged(self):
        for text in ["The loader used to reject it.", "It no longer rejects it.", "This now passes.", "since the loader work"]:
            self.assertTrue(self.warns(text), text)

    def test_a_use_or_a_current_rule_is_not(self):
        for text in [
            "FNV-1a over bytes, used to derive element seeds from ids.",
            "a base that is used to share child enums.",
            "it is used to start the run.",
            "must depend only on records that no longer change.",
            "a solver that is no longer deterministic.",
        ]:
            self.assertFalse(self.warns(text), text)


class CommitSubjects(unittest.TestCase):
    def subject(self, text):
        with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as f:
            f.write(text + "\n\nbody\n")
        return run("--commit-msg", f.name)

    def test_labels_of_a_phase_a_batch_an_item_or_wip_are_refused_in_any_case(self):
        for subject in ["Phase 2 work", "phase2/cosim merge", "phase-3 fix", "Batch 4", "item 5", "wip: x", "WIP x"]:
            self.assertEqual(self.subject(subject).returncode, 1, subject)

    def test_an_ordinary_subject_passes(self):
        for subject in ["fix(eval): a smoke fails", "feat: the phase of the moon", "docs: items in the list"]:
            self.assertEqual(self.subject(subject).returncode, 0, subject)


class Staged(unittest.TestCase):
    def test_the_check_reads_the_index_and_not_the_working_tree(self):
        repo = Repo()
        repo.write("a.md", "kept in /home/someone/private/\n")
        repo.stage("a.md")
        repo.write("a.md", "clean now\n")
        result = repo.check("--staged")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("B1", result.stdout)
        # and the converse: a clean index with a dirty working tree passes
        other = Repo()
        other.write("b.md", "clean\n")
        other.stage("b.md")
        other.write("b.md", "kept in /home/someone/private/\n")
        self.assertEqual(other.check("--staged").returncode, 0)


LEDGER = "tools/evidence/cinematic-impact.json"


def milestone(called="one", without=()):
    entry = {"name": called, "evidence": ["a.rs"], "validation": "v", "limits": "l"}
    for key in without:
        del entry[key]
    return entry


class Ledger(unittest.TestCase):
    """The acceptance ledger is read by a parser that refuses a repeated key: a merge that splices two lists of milestones can leave a file that loads
    and has lost an entry (the second copy of a key wins), and a file that loads is not a file that is whole."""

    def check_text(self, text):
        repo = Repo()
        repo.write(LEDGER, text)
        repo.write("a.md", "clean\n")
        repo.stage("a.md")
        return repo.check("--all")

    def check_milestones(self, milestones):
        import json
        return self.check_text(json.dumps({"milestones": milestones}, indent=2))

    def test_a_whole_ledger_passes(self):
        result = self.check_milestones([milestone("one"), milestone("two")])
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_the_ledger_of_this_repository_is_whole(self):
        self.assertEqual(hygiene.check_ledger(str(ROOT)), [])

    def test_a_key_that_is_repeated_in_an_object_is_refused(self):
        text = '{"milestones": [{"name": "a", "evidence": [], "validation": "v", "limits": "l", "name": "b"}]}'
        result = self.check_text(text)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("L1", result.stdout)
        self.assertIn("name", result.stdout)

    def test_a_milestone_that_lacks_a_field_or_a_name_is_refused(self):
        for missing in ("name", "evidence", "validation", "limits"):
            result = self.check_milestones([milestone("one"), milestone("two", without=(missing,))])
            self.assertEqual(result.returncode, 1, f"{missing}: {result.stdout}{result.stderr}")
            self.assertIn("L2", result.stdout, missing)
            self.assertIn(missing, result.stdout)

    def test_two_milestones_of_one_name_are_refused(self):
        result = self.check_milestones([milestone("one"), milestone("two"), milestone("one")])
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("L3", result.stdout)

    def test_a_ledger_that_does_not_parse_is_refused_by_name(self):
        result = self.check_text('{"milestones": [')
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("L1", result.stdout)

    def test_a_repository_with_no_ledger_is_not_asked_for_one(self):
        repo = Repo()
        repo.write("a.md", "clean\n")
        repo.stage("a.md")
        self.assertEqual(repo.check("--all").returncode, 0)


HISTORICAL = "tools/evidence/historical-digests.json"
SHA_A = hashlib.sha256(b"current\n").hexdigest()
SHA_OLD = hashlib.sha256(b"an earlier revision\n").hexdigest()


class LedgerDigests(unittest.TestCase):
    """A digest the ledger pins is either the file's digest now (L4) or declared as a past revision's (L5 keeps that list honest)."""

    def check(self, pins, historical=None, files=None, extra=None):
        import json
        repo = Repo()
        repo.write("a.md", "clean\n")
        repo.stage("a.md")
        repo.write("src/a.rs", "current\n")
        for name, text in (files or {}).items():
            repo.write(name, text)
        ledger = {"milestones": [dict(milestone("one"), **({"source_sha256": pins} if pins else {}), **(extra or {}))]}
        repo.write(LEDGER, json.dumps(ledger))
        if historical is not None:
            repo.write(HISTORICAL, json.dumps({"digests": [{"file": f, "sha256": d} for f, d in historical]}))
        return repo.check("--all")

    def test_a_pin_that_equals_the_file_passes(self):
        r = self.check({"src/a.rs": SHA_A})
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_a_pin_the_file_no_longer_matches_is_refused(self):
        r = self.check({"src/a.rs": SHA_OLD})
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("L4", r.stdout)
        self.assertIn("src/a.rs", r.stdout)

    def test_a_pin_of_a_missing_file_is_refused(self):
        r = self.check({"src/gone.rs": SHA_OLD})
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("L4", r.stdout)

    def test_a_declared_historical_pin_passes(self):
        r = self.check({"src/a.rs": SHA_OLD}, historical=[("src/a.rs", SHA_OLD)])
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_a_historical_entry_does_not_excuse_another_digest_of_the_file(self):
        r = self.check({"src/a.rs": hashlib.sha256(b"third\n").hexdigest()}, historical=[("src/a.rs", SHA_OLD)])
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("L4", r.stdout)

    def test_a_historical_entry_that_matches_the_tree_is_stale(self):
        r = self.check({"src/a.rs": SHA_A}, historical=[("src/a.rs", SHA_A)])
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("L5", r.stdout)

    def test_a_historical_entry_the_ledger_does_not_pin_is_stale(self):
        r = self.check({"src/a.rs": SHA_A}, historical=[("src/b.rs", SHA_OLD)])
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("L5", r.stdout)

    def test_maps_are_found_at_any_depth(self):
        r = self.check(None, extra={"nested": [{"x": {"inputs_sha256": {"src/a.rs": SHA_OLD}}}]})
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("L4", r.stdout)

    def committed(self, old_text, current_text, historical, origin=None):
        """A repository whose `src/a.rs` was committed as `old_text` and now reads `current_text`; the ledger pins
        the digest of `old_text`."""
        import json
        repo = Repo()
        repo.write("src/a.rs", old_text)
        repo.write("a.md", "clean\n")
        subprocess.run(["git", "add", "-A"], cwd=repo.path, check=True)
        repo.commit()
        repo.write("src/a.rs", current_text)
        ledger = {"milestones": [dict(milestone("one"), source_sha256={"src/a.rs": hashlib.sha256(old_text.encode()).hexdigest()})]}
        repo.write(LEDGER, json.dumps(ledger))
        entry = {"file": "src/a.rs", "sha256": historical}
        if origin:
            entry["origin"] = origin
        repo.write(HISTORICAL, json.dumps({"digests": [entry]}))
        return repo.check("--all")

    def test_a_historical_entry_that_is_a_committed_revision_passes(self):
        digest = hashlib.sha256(b"first\n").hexdigest()
        r = self.committed("first\n", "second\n", digest)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_a_historical_entry_no_commit_ever_had_is_refused(self):
        # the ledger pins the digest of the file as committed; the list names a different value, as a made-up
        # entry would, and the ledger's own pin then has no entry at all
        import json
        repo = Repo()
        repo.write("src/a.rs", "first\n")
        repo.write("a.md", "clean\n")
        subprocess.run(["git", "add", "-A"], cwd=repo.path, check=True)
        repo.commit()
        repo.write("src/a.rs", "second\n")
        invented = hashlib.sha256(b"never committed\n").hexdigest()
        ledger = {"milestones": [dict(milestone("one"), source_sha256={"src/a.rs": invented})]}
        repo.write(LEDGER, json.dumps(ledger))
        repo.write(HISTORICAL, json.dumps({"digests": [{"file": "src/a.rs", "sha256": invented}]}))
        r = repo.check("--all")
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("L6", r.stdout)

    def test_an_entry_from_the_previous_repository_is_not_asked_for_a_revision(self):
        import json
        repo = Repo()
        repo.write("src/a.rs", "now\n")
        repo.write("a.md", "clean\n")
        subprocess.run(["git", "add", "-A"], cwd=repo.path, check=True)
        repo.commit()
        old = hashlib.sha256(b"before this history\n").hexdigest()
        ledger = {"milestones": [dict(milestone("one"), source_sha256={"src/a.rs": old})]}
        repo.write(LEDGER, json.dumps(ledger))
        repo.write(HISTORICAL, json.dumps({"digests": [{"file": "src/a.rs", "sha256": old,
                                                       "origin": "previous-repository"}]}))
        r = repo.check("--all")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_a_repository_without_history_is_not_asked_for_revisions(self):
        r = self.check({"src/a.rs": SHA_OLD}, historical=[("src/a.rs", SHA_OLD)])
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_every_historical_entry_of_this_repository_is_a_revision_or_declared_inherited(self):
        errors = [e for e in hygiene.check_ledger(str(ROOT)) if "L6" in e]
        self.assertEqual(errors, [])

    def test_every_pin_of_this_repository_is_current_or_declared(self):
        errors = [e for e in hygiene.check_ledger(str(ROOT)) if "L4" in e or "L5" in e]
        self.assertEqual(errors, [])


if __name__ == "__main__":
    unittest.main()
