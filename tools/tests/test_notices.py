"""THIRD-PARTY-NOTICES.md names every crate in the build that compiles or links native code."""
import json
import pathlib
import shutil
import subprocess
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]

# Crates that compile bundled C sources without declaring `links` in their manifest.
BUNDLED_WITHOUT_LINKS = {"ufbx"}
# Crates that declare `links` but put no third-party native code in the binary, with the reason.
NO_NATIVE_CODE = {
    "rayon-core": "declares links only to keep a single copy of its global thread pool",
    "wasm-bindgen-shared": "declares links only to keep a single copy of its schema version",
    "link-cplusplus": "links the system C++ runtime dynamically",
}


def native_crates():
    out = subprocess.run(["cargo", "metadata", "--format-version", "1", "--locked"], cwd=ROOT, check=True,
                         capture_output=True, text=True).stdout
    meta = json.loads(out)
    resolved = {node["id"] for node in meta["resolve"]["nodes"]}
    return {p["name"] for p in meta["packages"]
            if p["id"] in resolved and (p.get("links") or p["name"] in BUNDLED_WITHOUT_LINKS)}


@unittest.skipUnless(shutil.which("cargo"), "cargo is not installed")
class NoticesTests(unittest.TestCase):
    def test_every_native_crate_is_named_or_exempted(self):
        notices = (ROOT / "THIRD-PARTY-NOTICES.md").read_text()
        missing = sorted(c for c in native_crates() - NO_NATIVE_CODE.keys() if f"`{c}`" not in notices)
        self.assertEqual(missing, [], "name these crates and their licences in THIRD-PARTY-NOTICES.md")

    def test_exemptions_are_still_in_the_build(self):
        stale = sorted(NO_NATIVE_CODE.keys() - native_crates())
        self.assertEqual(stale, [], "remove exemptions for crates that left the build")


if __name__ == "__main__":
    unittest.main()
