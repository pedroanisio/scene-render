"""CHANGELOG.md keeps the current release notes as its first entry."""
import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import changelog

ROOT = pathlib.Path(__file__).resolve().parents[2]


class ChangelogTests(unittest.TestCase):
    def test_entry_demotes_headings_and_dates_the_release(self):
        version, text = changelog.entry("# scene-render v1.2.3\n\nBody.\n\n## Detail\n", "2026-01-02")
        self.assertEqual(version, "v1.2.3")
        self.assertEqual(text, "## v1.2.3 — 2026-01-02\n\nBody.\n\n### Detail\n")

    def test_unreleased_notes_are_separate_from_released_history(self):
        notes = "# scene-render v1.2.3\n\nReleased.\n\n## Unreleased\n\nPending.\n\n## Detail\nOld.\n"
        released, pending = changelog.split_unreleased(notes)
        self.assertEqual(pending, "## Unreleased\n\nPending.\n")
        self.assertIn("## Detail\nOld.", released)
        self.assertNotIn("Pending", released)

    def test_entry_rejects_notes_without_a_release_title(self):
        with self.assertRaises(ValueError):
            changelog.entry("Release notes\n", "2026-01-02")

    def test_unreleased_is_separate_from_the_tagged_release(self):
        notes = "# scene-render v1.2.3\n\nOld body.\n\n## Unreleased\n\n- New fix.\n"
        released, pending = changelog.split_unreleased(notes)
        self.assertEqual(released, "# scene-render v1.2.3\n\nOld body.\n")
        self.assertEqual(pending, "## Unreleased\n\n- New fix.\n")
        self.assertEqual(changelog.split_unreleased(released), (released, ""))

    def test_first_entry_is_the_current_release_notes(self):
        # Works on shallow clones without tags: only the working tree is compared.
        notes = (ROOT / "RELEASE_NOTES.md").read_text()
        notes, pending = changelog.split_unreleased(notes)
        version, text = changelog.entry(notes, "DATE")
        body = text.split("\n", 1)[1]
        log = (ROOT / "CHANGELOG.md").read_text()
        if pending:
            self.assertIn("\n" + pending, log)
            log = log.replace("\n" + pending, "", 1)
        first = log.split("\n## ", 2)[1]
        self.assertTrue(first.startswith(f"{version} — "), f"CHANGELOG.md does not start with {version}")
        self.assertEqual(first.split("\n", 1)[1].rstrip(), body.rstrip())


if __name__ == "__main__":
    unittest.main()
