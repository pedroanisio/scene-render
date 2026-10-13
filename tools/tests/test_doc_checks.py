"""The document checkers find stale figures, missing paths and broken links, and accept the repository as it is."""
import pathlib
import re
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

    def test_rust_doc_comments_are_checked_too(self):
        docs = {p.relative_to(ROOT).as_posix() for p in doc_facts.tracked_docs()}
        self.assertIn("crates/sr-model/src/lib.rs", docs)
        with tempfile.TemporaryDirectory() as d:
            src = pathlib.Path(d) / "lib.rs"
            src.write_text("//! the <!-- facts:sch-patterns -->1<!-- /facts --> patterns\n")
            self.assertEqual(len(doc_facts.process(src, self.values, write=False)), 1)

    def test_ci_figures_come_from_the_workflow(self):
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        self.assertIn(f"--iterations {self.values['fuzz-iterations']} --partition", ci)
        self.assertIn(f"{self.values['ci-shards']} --timings", ci.replace("${{ matrix.shard }} ", ""))

    def test_the_figures_that_were_stale_are_marked(self):
        readme = (ROOT / "README.md").read_text()
        for name in ("sch-patterns", "rule-v-max", "rule-c-max", "rule-r-max", "corpus-total", "fuzz-iterations",
                     "ci-shards", "blend-modes"):
            self.assertIn(f"<!-- facts:{name} -->", readme, name)
        lib = (ROOT / "crates/sr-model/src/lib.rs").read_text()
        self.assertIn("<!-- facts:sch-patterns -->", lib)

    def test_the_blend_mode_count_is_the_schemas(self):
        import xml.etree.ElementTree as ET
        xs = "{http://www.w3.org/2001/XMLSchema}"
        root = ET.parse(ROOT / "schema/scene-render-1.1.xsd").getroot()
        members = [e.get("value") for t in root.iter(xs + "simpleType") if t.get("name") == "blendType"
                   for e in t.iter(xs + "enumeration")]
        self.assertEqual(int(self.values["blend-modes"]), len(members))
        self.assertGreater(len(members), 20)

    def test_the_tests_section_states_no_inventory_count_by_hand(self):
        section = (ROOT / "README.md").read_text().split("\n## Tests\n", 1)[1].split("\n## ", 1)[0]
        section = doc_facts.MARK.sub("", section)
        found = re.findall(r"\b(?:\d+|[Tt]wo|[Tt]hree|[Ff]our|[Ff]ive|[Ss]ix|[Ss]even|[Ee]ight|[Nn]ine|[Tt]en) "
                           r"(?:[A-Za-z-]+ ){0,2}(?:tests|deformers|presets|transitions|effect types|modifiers|blend modes)",
                           section)
        self.assertEqual(found, [], "a count the README states by hand: mark it as a fact or drop the number")

    def test_repository_documents_are_current(self):
        self.assertEqual(doc_facts.main(["--check"]), 0)


class LayoutTests(unittest.TestCase):
    """The README's Layout block names every crate, every module of a crate whose entry lists its `src/`, and every
    example directory."""

    def tree(self, root):
        for path in ("crates/sr-a/src/inner", "crates/sr-b/src", "crates/cli", "examples/one"):
            (root / path).mkdir(parents=True)
        for crate in ("sr-a", "sr-b", "cli"):
            (root / "crates" / crate / "Cargo.toml").write_text("")
        for path in ("crates/sr-a/src/lib.rs", "crates/sr-a/src/alpha.rs", "crates/sr-a/src/inner/mod.rs",
                     "crates/sr-b/src/lib.rs", "crates/sr-b/src/beta.rs"):
            (root / path).write_text("")

    def readme(self, entries):
        return "# x\n\n## Layout\n\n```\n" + "\n".join(entries) + "\n```\n\n## Tests\n"

    def problems(self, entries):
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d)
            self.tree(root)
            return doc_facts.layout_problems(self.readme(entries), root)

    COMPLETE = [
        "crates/sr-a/src/            alpha, inner (a directory module),",
        "                            continued on an indented line",
        "crates/sr-b/src/            beta",
        "crates/cli/                 the command line",
        "examples/one/               an example",
    ]

    def test_a_complete_block_is_accepted(self):
        self.assertEqual(self.problems(self.COMPLETE), [])

    def test_a_module_missing_from_its_entry_is_named(self):
        entries = [e.replace("alpha, ", "") for e in self.COMPLETE]
        self.assertEqual(self.problems(entries), ["Layout entry of crates/sr-a does not name module alpha"])

    def test_a_directory_module_is_required_too(self):
        entries = [e.replace("inner (a directory module),", "") for e in self.COMPLETE]
        self.assertEqual(self.problems(entries), ["Layout entry of crates/sr-a does not name module inner"])

    def test_a_crate_without_an_entry_is_named(self):
        entries = [e for e in self.COMPLETE if not e.startswith("crates/sr-b")]
        self.assertEqual(self.problems(entries), ["Layout names no entry for crates/sr-b"])

    def test_an_example_directory_without_a_line_is_named(self):
        entries = [e for e in self.COMPLETE if not e.startswith("examples/")]
        self.assertEqual(self.problems(entries), ["Layout does not name examples/one/"])

    def test_a_name_the_crate_does_not_have_is_reported(self):
        entries = [e.replace("beta", "beta, gone_module") for e in self.COMPLETE]
        self.assertEqual(self.problems(entries),
                         ["Layout entry of crates/sr-b names gone_module, which is not in the crate"])

    def test_names_of_files_directories_and_brace_groups_are_present(self):
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d)
            self.tree(root)
            (root / "crates/sr-b/src/beta_one.rs").write_text("")
            (root / "crates/sr-b/src/beta_two.rs").write_text("")
            (root / "crates/sr-b/tests").mkdir()
            (root / "crates/sr-b/tests/some_test.rs").write_text("")
            entries = [e for e in self.COMPLETE if not e.startswith("crates/sr-b")] + [
                "crates/sr-b/src/            beta, beta_{one,two}; some_test (a test)"]
            self.assertEqual(doc_facts.layout_problems(self.readme(entries), root), [])

    def test_the_readme_of_this_repository_names_everything(self):
        self.assertEqual(doc_facts.layout_problems((ROOT / "README.md").read_text()), [])


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

    def test_fenced_code_is_text_not_links(self):
        # CommonMark renders a fence literally: a template's `[x](<PATH>)` there is an example, not a link
        with tempfile.TemporaryDirectory(dir=ROOT) as d:
            doc = pathlib.Path(d) / "doc.md"
            doc.write_text("```markdown\n[a](<RELATIVE_PATH>)\n~~~\n[b](inside.md)\n```\n[c](after.md)\n"
                           "  ~~~~\n[d](tilde.md)\n```\n~~~~~\n[e](closed.md)\n")
            self.assertEqual([(n, k, t) for n, k, t in doc_links.problems(doc)],
                             [(6, "link", "after.md"), (11, "link", "closed.md")])


if __name__ == "__main__":
    unittest.main()
