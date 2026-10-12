"""The document checkers find stale figures, missing paths and broken links, and accept the repository as it is."""
import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import doc_facts
import doc_links

ROOT = pathlib.Path(__file__).resolve().parents[2]


class DocFactsTests(unittest.TestCase):
    def setUp(self):
        self.values = doc_facts.facts()

    def test_facts_come_from_their_sources(self):
        self.assertRegex(self.values["schema-upstream"], r"^\d+\.\d+\.\d+$")
        self.assertRegex(self.values["rule-v-max"], r"^V\d+$")
        self.assertEqual(int(self.values["corpus-total"]), sum(int(self.values[f"corpus-{k}"])
                                                               for k in ("valid", "invalid", "warnings")))

    def test_stale_figure_is_reported_and_rewritten(self):
        with tempfile.TemporaryDirectory() as d:
            doc = pathlib.Path(d) / "doc.md"
            doc.write_text("There are <!-- facts:sch-asserts -->1<!-- /facts --> asserts.\n")
            self.assertEqual(len(doc_facts.process(doc, self.values, write=False)), 1)
            doc_facts.process(doc, self.values, write=True)
            self.assertIn(f"-->{self.values['sch-asserts']}<!--", doc.read_text())
            self.assertEqual(doc_facts.process(doc, self.values, write=False), [])

    def test_unknown_fact_is_an_error(self):
        with tempfile.TemporaryDirectory() as d:
            doc = pathlib.Path(d) / "doc.md"
            doc.write_text("<!-- facts:no-such-fact -->1<!-- /facts -->\n")
            self.assertEqual(doc_facts.process(doc, self.values, write=False), ["unknown fact 'no-such-fact'"])

    def test_repository_documents_are_current(self):
        self.assertEqual(doc_facts.main(["--check"]), 0)


class TrackedDocsTests(unittest.TestCase):
    def test_tracked_docs_skip_files_deleted_from_the_working_tree(self):
        # git ls-files still lists a tracked file that was removed but not yet staged
        for module in (doc_facts, doc_links):
            with self.subTest(module=module.__name__):
                self.assertTrue(all(p.exists() for p in module.tracked_docs()))


class DocLinksTests(unittest.TestCase):
    def test_missing_path_and_link_are_found_and_placeholders_skipped(self):
        with tempfile.TemporaryDirectory(dir=ROOT) as d:
            doc = pathlib.Path(d) / "doc.md"
            doc.write_text("See `crates/no-such-crate/src/lib.rs`, [x](no-such-file.md), [y](https://e.org/a),\n"
                           "`crates/<crate>/tests/<area>.rs`, `tools/doc_links.py` and [z](doc.md#top).\n")
            self.assertEqual([(k, t) for _, k, t in doc_links.problems(doc)],
                             [("link", "no-such-file.md"), ("path", "crates/no-such-crate/src/lib.rs")])


if __name__ == "__main__":
    unittest.main()
