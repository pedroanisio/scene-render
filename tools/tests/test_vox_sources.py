"""The provenance note of the real MagicaVoxel fixtures describes exactly the files beside it."""
import hashlib
import pathlib
import re
import unittest

DIR = pathlib.Path(__file__).resolve().parents[2] / "crates/sr-3d/tests/fixtures/vox/real"


class VoxSourcesTests(unittest.TestCase):
    def setUp(self):
        self.note = (DIR / "SOURCES.md").read_text()
        self.present = {p.name for p in DIR.glob("*.vox")}

    def test_inventory_rows_match_the_files_and_their_sizes(self):
        rows = re.findall(r"^\| (\S+\.vox) \| (\d+) \|", self.note, re.M)
        self.assertEqual({name for name, _ in rows}, self.present)
        for name, size in rows:
            with self.subTest(file=name):
                self.assertEqual((DIR / name).stat().st_size, int(size))

    def test_digests_match_the_files(self):
        digests = re.findall(r"^([0-9a-f]{64})  (\S+\.vox)$", self.note, re.M)
        self.assertEqual({name for _, name in digests}, self.present)
        for digest, name in digests:
            with self.subTest(file=name):
                self.assertEqual(hashlib.sha256((DIR / name).read_bytes()).hexdigest(), digest)

    def test_licences_named_in_the_note_exist(self):
        for licence in re.findall(r"\b(\w+_LICENSE)\b", self.note):
            with self.subTest(licence=licence):
                self.assertTrue((DIR / licence).is_file())


if __name__ == "__main__":
    unittest.main()
