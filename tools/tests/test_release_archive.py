"""The release notes name every file the Linux archive holds, and only those."""
import pathlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
ACTION = ROOT / ".github/actions/package/action.yml"
NOTES = ROOT / "RELEASE_NOTES.md"

# what the notes call each archived file, in their words (lower case)
CALLED = {
    "target/release/scene-render": "executable",
    "COMMIT": "source commit identifier",
    "capabilities.json": "capability manifest",
    "README.md": "readme",
    "CHANGELOG.md": "changelog",
    "DISCLAIMER.md": "disclaimer",
    "LICENSE": "licence",
    "NOTICE": "notice",
    "COMMERCIAL.md": "commercial-licence page",
    "THIRD-PARTY-NOTICES.md": "third-party notices",
}


# Files a tagged release's notes omit although its archive held them. The notes of a tag are kept as released
# (CHANGELOG.md is generated from them), so the sentence is not corrected after the fact; the next release's notes must
# name them. Each entry is dropped by the test below once the notes name the file or the archive stops holding it.
RELEASED_OMISSIONS = {
    "v0.5.0": {"CHANGELOG.md", "DISCLAIMER.md"},
}


def release_of(notes):
    m = re.search(r"^# scene-render (v\d+\.\d+\.\d+)\s*$", notes, re.M)
    return m.group(1) if m else None


def archived(text):
    """The files the package step copies into the archive directory, and the one it writes."""
    files = []
    copy = re.search(r'^\s*cp (.+) "dist/\$package/"\s*$', text, re.M)
    if copy:
        files = copy.group(1).split()
    if re.search(r'> "dist/\$package/COMMIT"', text):
        files.append("COMMIT")
    return files


def sentence(notes):
    m = re.search(r"The Linux archive contains ([^.]*)\.", notes)
    return m.group(1).lower() if m else None


def problems(action_text, notes_text, omissions=None):
    out = []
    files = archived(action_text)
    said = sentence(notes_text)
    if not files:
        return ["the package step's `cp` line was not found"]
    if said is None:
        return ["the release notes have no `The Linux archive contains ...` sentence"]
    excused = RELEASED_OMISSIONS.get(release_of(notes_text), set()) if omissions is None else omissions
    for f in files:
        if f in excused:
            if CALLED.get(f) in said:
                out.append(f"{f} is in RELEASED_OMISSIONS and the notes name it: drop the exemption")
            continue
        if f not in CALLED:
            out.append(f"the archive holds {f}, which this test has no name for: add it to CALLED and the notes")
        elif CALLED[f] not in said:
            out.append(f"the archive holds {f} and the notes do not say {CALLED[f]!r}")
    for f in excused - set(files):
        out.append(f"{f} is in RELEASED_OMISSIONS and the archive does not hold it: drop the exemption")
    named = {CALLED[f] for f in files if f in CALLED}
    for f, word in CALLED.items():
        if word not in named and word in said:
            out.append(f"the notes name the {word} and the archive does not hold {f}")
    return out


class ReleaseArchiveTests(unittest.TestCase):
    def test_the_notes_list_what_the_archive_holds(self):
        self.assertEqual(problems(ACTION.read_text(), NOTES.read_text()), [])

    def test_the_package_step_is_read(self):
        files = archived(ACTION.read_text())
        self.assertIn("target/release/scene-render", files)
        self.assertIn("COMMIT", files)
        self.assertGreaterEqual(len(files), 9)

    def test_a_file_added_to_the_archive_and_not_the_notes_is_reported(self):
        action = ACTION.read_text().replace("capabilities.json \"dist", "capabilities.json SBOM.json \"dist")
        found = problems(action, NOTES.read_text())
        self.assertEqual(len(found), 1, found)
        self.assertIn("SBOM.json", found[0])
        # the exemptions of a released tag cover only the files they name
        self.assertEqual(problems(ACTION.read_text(), NOTES.read_text(), omissions=set()),
                         [f"the archive holds {f} and the notes do not say {CALLED[f]!r}"
                          for f in ("CHANGELOG.md", "DISCLAIMER.md")] if release_of(NOTES.read_text()) == "v0.5.0" else [])

    def test_a_file_the_notes_name_and_the_archive_dropped_is_reported(self):
        notes = NOTES.read_text().replace("README, licence", "README, changelog, disclaimer, licence")
        action = ACTION.read_text().replace(" DISCLAIMER.md", "")
        found = problems(action, notes, omissions=set())
        self.assertEqual(len(found), 1, found)
        self.assertIn("disclaimer", found[0])

    def test_a_file_the_archive_holds_and_the_notes_omit_is_reported(self):
        notes = NOTES.read_text().replace("README, licence", "README, changelog, licence")
        found = problems(ACTION.read_text(), notes.replace(" changelog,", ""), omissions=set())
        self.assertTrue(any("CHANGELOG.md" in f for f in found), found)

    def test_an_exemption_the_notes_have_outgrown_is_reported(self):
        notes = NOTES.read_text().replace("README, licence", "README, changelog, disclaimer, licence")
        found = problems(ACTION.read_text(), notes)
        self.assertEqual(len(found), 2, found)
        self.assertTrue(all("drop the exemption" in f for f in found))
        found = problems(ACTION.read_text().replace(" CHANGELOG.md", "").replace(" DISCLAIMER.md", ""), NOTES.read_text())
        self.assertEqual(len(found), 2, found)


if __name__ == "__main__":
    unittest.main()
