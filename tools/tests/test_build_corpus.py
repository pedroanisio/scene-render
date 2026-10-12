"""Check generator categories without regenerating the checked-in corpus."""
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import build_corpus


class CorpusCategoriesTests(unittest.TestCase):
    def test_oracle_codes_exclude_rust_only_warnings_and_asset_checks(self):
        self.assertEqual(build_corpus.oracle_codes(["FRX4", "W01"]), ["FRX4"])
        self.assertEqual(build_corpus.oracle_codes(["S06", "A01", "W01", "FRX4"]), ["FRX4", "XSD"])
        self.assertEqual(build_corpus.oracle_codes(["S06", "FRX4"], blind=True), ["FRX4"])

    def test_case_names_are_unique_across_categories(self):
        names = list(build_corpus.VALID)
        for cases in (build_corpus.CASES, build_corpus.ASSET_CASES, build_corpus.WARN_CASES):
            names.extend(name for name, _, _ in cases)
        self.assertEqual(len(names), len(set(names)), "a category would overwrite another case")

    def test_asset_and_warning_cases_are_schema_valid(self):
        for name, _, mutate in build_corpus.ASSET_CASES + build_corpus.WARN_CASES:
            with self.subTest(case=name):
                self.assertEqual(build_corpus.verdict(mutate(build_corpus.base).encode()), ([], []))


ROOT = Path(__file__).resolve().parents[2]


def tree(base):
    """Relative path -> bytes of every file under base."""
    return {str(p.relative_to(base)): p.read_bytes() for p in sorted(base.rglob("*")) if p.is_file()}


class CheckedInCorpusTests(unittest.TestCase):
    """tests/corpus is what tools/build_corpus.py writes from tools/kitchen_sink.xml.in: an edit to the template, to the
    generator or to a media file that leaves the corpus behind fails here and not only in the oracle job."""

    def test_the_manifest_lists_exactly_the_documents_on_disk(self):
        corpus = ROOT / "tests" / "corpus"
        manifest = json.loads((corpus / "manifest.json").read_text())
        # a warning case is a valid document that carries warnings, and sits in valid/
        listed = {f"{'invalid' if kind == 'invalid' else 'valid'}/{name}" for kind, names in manifest.items() for name in names}
        on_disk = {f"{d}/{p.name}" for d in ("valid", "invalid") for p in (corpus / d).glob("*.scene.xml")}
        self.assertEqual(sorted(listed - on_disk), [], "listed but not on disk")
        self.assertEqual(sorted(on_disk - listed), [], "on disk but not listed")

    def test_regenerating_the_corpus_changes_nothing(self):
        with tempfile.TemporaryDirectory() as d:
            work = Path(d)
            for part in ("tools", "schema", "tests/corpus"):
                shutil.copytree(ROOT / part, work / part, ignore=shutil.ignore_patterns("__pycache__", "tests"))
            before = tree(work / "tests" / "corpus")
            done = subprocess.run([sys.executable, str(work / "tools" / "build_corpus.py")], cwd=work,
                                  capture_output=True, text=True)
            self.assertEqual(done.returncode, 0, done.stdout + done.stderr)
            after = tree(work / "tests" / "corpus")
            self.assertEqual(sorted(set(before) ^ set(after)), [], "files added or removed by regeneration")
            self.assertEqual([n for n in before if before[n] != after[n]], [], "files changed by regeneration")
            self.assertEqual(before, tree(ROOT / "tests" / "corpus"), "the copy is not the checked-in corpus")


if __name__ == "__main__":
    unittest.main()
