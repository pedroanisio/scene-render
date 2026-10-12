"""Checked-in files that a script or a build produces are what that script or build produces from what is checked in."""
import hashlib
import json
import os
import pathlib
import re
import shutil
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

    def test_the_module_is_what_the_sources_build_to(self):
        """Rebuilds in a scratch copy and compares bytes. The build needs the wasm target: without it the test is
        skipped, and SR_REQUIRE_WASM=1 (set it where the target is installed) makes its absence a failure."""
        libdir = subprocess.run(["rustc", "--print", "target-libdir", "--target", "wasm32-unknown-unknown"],
                                capture_output=True, text=True).stdout.strip()
        have_target = bool(libdir) and pathlib.Path(libdir).is_dir() and any(pathlib.Path(libdir).glob("libcore-*"))
        if not have_target:
            if os.environ.get("SR_REQUIRE_WASM") == "1":
                self.fail("wasm32-unknown-unknown is not installed and SR_REQUIRE_WASM=1")
            self.skipTest("wasm32-unknown-unknown is not installed")
        with tempfile.TemporaryDirectory() as d:
            work = pathlib.Path(d) / "maze"
            shutil.copytree(MAZE / "maze", work, ignore=shutil.ignore_patterns("target"))
            done = subprocess.run(
                ["cargo", "build", "--release", "--locked", "--target", "wasm32-unknown-unknown"],
                cwd=work, capture_output=True, text=True, env={**os.environ, "CARGO_TARGET_DIR": str(pathlib.Path(d) / "t")})
            self.assertEqual(done.returncode, 0, done.stderr[-2000:])
            built = pathlib.Path(d) / "t/wasm32-unknown-unknown/release/sr_example_maze.wasm"
            self.assertEqual(hashlib.sha256(built.read_bytes()).hexdigest(),
                             hashlib.sha256((MAZE / "maze.wasm").read_bytes()).hexdigest(),
                             "maze.wasm is not what its sources build to: run examples/program-maze/build.sh")


class OpenVdbFixtureTests(unittest.TestCase):
    """The .vdb fixtures come from tools/openvdb_fixture.cpp, which needs the OpenVDB library that CI does not have.
    The bytes cannot be regenerated here; what is checked is that the generator and the checked-in files list the same
    names, and that a test reads every file."""

    DATA = ROOT / "crates/sr-volume/tests/data/openvdb"

    def generated(self):
        text = (TOOLS / "openvdb_fixture.cpp").read_text()
        names = set(re.findall(r'save\("([a-z0-9-]+)"', text))
        # the two-frame loop saves `frame == 0 ? "render-0" : "render-1"`
        names |= set(re.findall(r'"(render-\d)"', text))
        return names

    def test_the_generator_and_the_checked_in_files_name_the_same_fixtures(self):
        checked_in = {p.stem for p in self.DATA.glob("*.vdb")}
        generated = self.generated()
        self.assertGreater(len(generated), 14)
        self.assertEqual(sorted(generated - checked_in), [], "the generator writes files that are not checked in")
        self.assertEqual(sorted(checked_in - generated), [], "checked-in files the generator does not write")

    def test_every_fixture_is_read_by_a_test(self):
        tests = "".join(p.read_text() for c in (ROOT / "crates").iterdir() for p in (c / "tests").rglob("*.rs")
                        if (c / "tests").is_dir())
        src = (ROOT / "crates/sr-volume/src/openvdb/codec.rs").read_text()
        unread = [n for n in sorted(self.generated())
                  if f'"{n}"' not in tests and f"{n}.vdb" not in tests + src
                  and not (n.startswith("render-") and "render-%d.vdb" in tests)]
        self.assertEqual(unread, [], "fixtures no test opens: delete them or test them")

    def test_the_generator_is_a_development_tool_the_ci_does_not_build(self):
        ci = "".join(p.read_text() for p in (ROOT / ".github").rglob("*.yml"))
        self.assertNotIn("openvdb_fixture", ci, "CI builds it now: compare its output with the checked-in files")


if __name__ == "__main__":
    unittest.main()
