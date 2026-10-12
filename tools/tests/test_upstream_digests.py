"""The vendored schema files are the sr-core release that schema/UPSTREAM names, byte for byte."""
import hashlib
import pathlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]

# the file name in sr-core -> the vendored file (schema/UPSTREAM names the renaming)
VENDORED = {
    "scene-render.xsd": "schema/scene-render-1.1.xsd",
    "scene-render.sch": "schema/scene-render-1.1.sch",
    "capabilities-1.schema.json": "schema/capabilities-1.schema.json",
}


def recorded(text):
    """`name: <sha256>` lines of the canonical-files block of schema/UPSTREAM."""
    return dict(re.findall(r"^\s+([\w.-]+):\s+([0-9a-f]{64})\s*$", text, re.M))


class UpstreamDigestTests(unittest.TestCase):
    def test_the_parser_reads_the_block(self):
        text = "Canonical upstream files:\n  a.xsd: " + "ab" * 32 + "\n  not a line: x\n"
        self.assertEqual(recorded(text), {"a.xsd": "ab" * 32})

    def test_every_vendored_file_is_recorded(self):
        digests = recorded((ROOT / "schema/UPSTREAM").read_text())
        self.assertEqual(sorted(digests), sorted(VENDORED), "schema/UPSTREAM and this table name different files")

    def test_every_vendored_file_has_the_recorded_digest(self):
        digests = recorded((ROOT / "schema/UPSTREAM").read_text())
        for upstream, vendored in VENDORED.items():
            actual = hashlib.sha256((ROOT / vendored).read_bytes()).hexdigest()
            self.assertEqual(
                actual,
                digests[upstream],
                f"{vendored} is not sr-core's {upstream}: take the release again, or record the local change in "
                "schema/UPSTREAM and update the digest there",
            )


if __name__ == "__main__":
    unittest.main()
