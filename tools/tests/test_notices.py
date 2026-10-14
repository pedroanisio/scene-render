"""THIRD-PARTY-NOTICES.md names every crate in the build that compiles or links native code, and every source file
adapted from another project."""
import json
import pathlib
import re
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


def adapted_files(project):
    """Source files whose opening comment says they are adapted from `project`, relative to the repository."""
    marker = re.compile(r"\A(?://|#)[^\n]*\bAdapted from " + re.escape(project) + r"\b")
    found = []
    for path in sorted((ROOT / "crates").rglob("*.rs")):
        if "target" not in path.relative_to(ROOT).parts and marker.match(path.read_text(encoding="utf-8")):
            found.append(path.relative_to(ROOT).as_posix())
    return found


def section(notices, title):
    match = re.search(r"^## " + re.escape(title) + r"\n(.*?)(?=^## |\Z)", notices, re.M | re.S)
    return match[1] if match else ""


class SectionTests(unittest.TestCase):
    def test_each_section_title_appears_once(self):
        titles = re.findall(r"^## (.+)$", (ROOT / "THIRD-PARTY-NOTICES.md").read_text(), re.M)
        repeated = sorted({t for t in titles if titles.count(t) > 1})
        self.assertEqual(repeated, [], "merge the repeated sections: a lookup by title reads only the first")


class AdaptedSourceTests(unittest.TestCase):
    def test_files_adapted_from_filmcraft_are_listed_with_the_mit_licence(self):
        files = adapted_files("FilmCraft")
        self.assertIn("crates/sr-media/src/scopes.rs", files)
        notice = section((ROOT / "THIRD-PARTY-NOTICES.md").read_text(), "FilmCraft")
        self.assertEqual([f for f in files if f"`{f}`" not in notice], [],
                         "list these files in the FilmCraft section of THIRD-PARTY-NOTICES.md")
        self.assertIn("Copyright (c) 2026 ArtCraft Team and the FilmCraft contributors", notice)
        self.assertIn("Permission is hereby granted, free of charge", notice)


if __name__ == "__main__":
    unittest.main()
