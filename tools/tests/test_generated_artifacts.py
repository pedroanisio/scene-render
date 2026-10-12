"""Checked-in files that a script or a build produces are what that script or build produces from what is checked in."""
import hashlib
import json
import pathlib
import re
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
TOOLS = ROOT / "tools"
VOX_DIR = "crates/sr-3d/tests/fixtures/vox"
PALETTE = "crates/sr-3d/src/voxel/default_palette.rs"
MAZE = ROOT / "examples" / "program-maze"


def run(script, *args, cwd):
    done = subprocess.run([sys.executable, str(TOOLS / script), *args], cwd=cwd, capture_output=True, text=True)
    assert done.returncode == 0, done.stdout + done.stderr
    return done


def source_digest(directory, names):
    """What `sha256sum NAMES... | sha256sum` gives in `directory` for the sorted regular files under NAMES (see build.sh)."""
    files = []
    for name in names:
        path = directory / name
        files += [path] if path.is_file() else [p for p in path.rglob("*") if p.is_file()]
    lines = "".join(
        f"{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.relative_to(directory).as_posix()}\n"
        for p in sorted(files, key=lambda p: p.relative_to(directory).as_posix().encode())
    )
    return hashlib.sha256(lines.encode()).hexdigest()


class SkyTests(unittest.TestCase):
    def test_the_example_sky_is_what_the_script_writes(self):
        with tempfile.TemporaryDirectory() as d:
            out = pathlib.Path(d) / "sky.hdr"
            run("make_sky_hdr.py", str(out), cwd=d)
            self.assertEqual(out.read_bytes(), (ROOT / "examples/cinematic-impact/sky.hdr").read_bytes())


class VoxFixtureTests(unittest.TestCase):
    def test_every_vox_fixture_and_its_expected_file_is_what_the_script_writes(self):
        with tempfile.TemporaryDirectory() as d:
            run("make_vox.py", d, cwd=d)
            made = {p.name: p.read_bytes() for p in pathlib.Path(d).iterdir()}
            kept = {p.name: p.read_bytes() for p in (ROOT / VOX_DIR).iterdir() if p.is_file()}
            self.assertGreater(len(made), 10)
            for name, data in made.items():
                with self.subTest(file=name):
                    self.assertIn(name, kept, "the script writes a file that is not checked in")
                    self.assertEqual(data, kept[name], "the checked-in file differs from the script's output")
            # the files the script does not write are real files (`real/` is a directory and is not listed here)
            self.assertEqual(sorted(set(kept) - set(made)), [])


class PaletteTests(unittest.TestCase):
    def test_the_default_palette_is_the_generators_output_for_its_own_table(self):
        """The table is rebuilt as the text the generator reads (0xAABBGGRR words), so a hand edit of an entry or of the
        file's layout, and a generator change that is not regenerated, both show."""
        source = (ROOT / PALETTE).read_text()
        rows = re.findall(r"\[(\d+), (\d+), (\d+), (\d+)\],", source)
        self.assertEqual(len(rows), 256)
        words = ", ".join(f"0x{int(a):02x}{int(b):02x}{int(g):02x}{int(r):02x}" for r, g, b, a in rows)
        with tempfile.TemporaryDirectory() as d:
            work = pathlib.Path(d)
            (work / "crates/sr-3d/src/voxel").mkdir(parents=True)
            text = work / "format.txt"
            text.write_text("static const uint32_t default_palette[256] = { " + words + " };\n")
            run("make_default_palette.py", str(text), cwd=work)
            self.assertEqual((work / PALETTE).read_text(), source)

    def test_the_table_is_the_one_that_was_generated_from_the_upstream_text(self):
        """The round trip above cannot see a changed value, since it reads the table it then compares with. This digest of
        the 1024 bytes was taken of the table the generator wrote from the upstream description; a deliberate
        regeneration from a changed description updates it here."""
        rows = re.findall(r"\[(\d+), (\d+), (\d+), (\d+)\],", (ROOT / PALETTE).read_text())
        digest = hashlib.sha256(bytes(int(v) for row in rows for v in row)).hexdigest()
        self.assertEqual(digest, "cc9800abac4eea5f0dc2b29399d7d6440165026723622ca30ffd77dd71653605")

    def test_index_zero_is_empty_and_index_one_is_white(self):
        rows = re.findall(r"\[(\d+), (\d+), (\d+), (\d+)\],", (ROOT / PALETTE).read_text())
        self.assertEqual(rows[0], ("0", "0", "0", "0"))
        self.assertEqual(rows[1], ("255", "255", "255", "255"))


class MazeTests(unittest.TestCase):
    """maze.wasm is built by examples/program-maze/build.sh, which needs a wasm target that CI does not install, so the
    check is on the sources: build.sh records their digest, and a source edited without a rebuild no longer matches."""

    def test_the_module_was_built_from_the_checked_in_sources(self):
        recorded = (MAZE / "maze.source-sha256").read_text().strip()
        now = source_digest(MAZE / "maze", ["Cargo.toml", "Cargo.lock", "src"])
        self.assertEqual(now, recorded, "examples/program-maze/maze changed: run examples/program-maze/build.sh")

    def test_the_scene_names_the_module_that_is_checked_in(self):
        scene = (MAZE / "maze.scene.xml").read_text()
        pinned = re.search(r'<program id="maze" src="maze.wasm" sha256="([0-9a-f]{64})"', scene)
        self.assertIsNotNone(pinned)
        self.assertEqual(pinned.group(1), hashlib.sha256((MAZE / "maze.wasm").read_bytes()).hexdigest())

    def test_the_build_script_records_the_digest_the_test_recomputes(self):
        script = (MAZE / "build.sh").read_text()
        self.assertIn("maze.source-sha256", script)
        self.assertIn("Cargo.toml Cargo.lock src", script)


if __name__ == "__main__":
    unittest.main()
