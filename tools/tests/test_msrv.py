"""The minimum compiler must cover every locked dependency and workspace crate."""
import json
import re
import pathlib
import subprocess
import tomllib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]


class MinimumRustTests(unittest.TestCase):
    def test_locked_dependencies_fit_declared_minimum(self):
        root = tomllib.loads((ROOT / "Cargo.toml").read_text())
        minimum = tuple(map(int, root["workspace"]["package"]["rust-version"].split(".")[:2]))
        data = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--locked", "--format-version", "1"], cwd=ROOT))
        for package in data["packages"]:
            version = package.get("rust_version")
            if version:
                self.assertLessEqual(tuple(map(int, version.split(".")[:2])), minimum, package["name"])
        for member in root["workspace"]["members"]:
            manifest = tomllib.loads((ROOT / member / "Cargo.toml").read_text())
            self.assertEqual(manifest["package"].get("rust-version"), {"workspace": True}, member)


class MinimumRustProse(unittest.TestCase):
    """The places that state the minimum compiler say what Cargo.toml says, and CI checks exactly that compiler."""

    @classmethod
    def setUpClass(cls):
        cls.minimum = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["rust-version"]

    def test_the_ci_msrv_job_pins_the_declared_minimum(self):
        ci = (ROOT / ".github" / "workflows" / "ci.yml").read_text()
        job = ci.split("\n  msrv:\n", 1)[1].split("\n  test:\n", 1)[0]
        pins = re.findall(r"dtolnay/rust-toolchain@(\S+)", job)
        self.assertEqual(pins, [self.minimum + ".0"], "the msrv job must check the workspace on the declared minimum")

    def test_the_prose_names_the_declared_minimum(self):
        for name, pattern in [
            ("README.md", r"Rust (\d+\.\d+) or newer is required; CI checks the complete workspace on (\d+\.\d+)"),
            ("CONTRIBUTING.md", r"\*\*Rust:\*\* (\d+\.\d+) or newer .*?exactly (\d+\.\d+)"),
        ]:
            m = re.search(pattern, (ROOT / name).read_text())
            self.assertIsNotNone(m, f"{name}: the sentence that states the minimum compiler is gone or reworded")
            self.assertEqual(m.groups(), (self.minimum, self.minimum), name)

    def test_the_pdf_feature_toolchain_is_stated_alike_in_ci_and_the_readme(self):
        ci = (ROOT / ".github" / "workflows" / "ci.yml").read_text()
        in_ci = set(re.findall(r"older than (\d+\.\d+)", ci))
        in_readme = set(re.findall(r"it needs Rust (\d+\.\d+), while", (ROOT / "README.md").read_text()))
        self.assertEqual(in_ci, in_readme)
        self.assertEqual(len(in_ci), 1)
