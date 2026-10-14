"""The vendored crates keep their changes and are the copies the build uses."""
import pathlib
import subprocess
import tempfile
import tomllib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]


class VendoredAllocatorTests(unittest.TestCase):
    def test_the_build_uses_the_vendored_copy(self):
        root = tomllib.loads((ROOT / "Cargo.toml").read_text())
        self.assertEqual(root["patch"]["crates-io"]["gpu-allocator"], {"path": "vendor/gpu-allocator"})
        self.assertIn("vendor", root["workspace"]["exclude"])
        lock = tomllib.loads((ROOT / "Cargo.lock").read_text())
        entries = [p for p in lock["package"] if p["name"] == "gpu-allocator"]
        vendored = tomllib.loads((ROOT / "vendor/gpu-allocator/Cargo.toml").read_text())["package"]["version"]
        self.assertEqual([(p["version"], p.get("source")) for p in entries], [(vendored, None)])

    def test_upload_memory_selection_behavior(self):
        # Compile the exact dependency-free policy module used by Allocator::allocate.
        with tempfile.TemporaryDirectory() as directory:
            binary = pathlib.Path(directory) / "memory-types"
            subprocess.run(["rustc", "--edition=2021", "--test", str(ROOT / "vendor/gpu-allocator/src/vulkan/memory_type.rs"), "-o", str(binary)], check=True)
            subprocess.run([str(binary)], check=True)


class VendoredHalTests(unittest.TestCase):
    HAL = ROOT / "vendor/wgpu-hal"

    def test_the_build_uses_the_vendored_copy(self):
        root = tomllib.loads((ROOT / "Cargo.toml").read_text())
        self.assertEqual(root["patch"]["crates-io"]["wgpu-hal"], {"path": "vendor/wgpu-hal"})
        lock = tomllib.loads((ROOT / "Cargo.lock").read_text())
        entries = [p for p in lock["package"] if p["name"] == "wgpu-hal"]
        vendored = tomllib.loads((self.HAL / "Cargo.toml").read_text())["package"]["version"]
        self.assertEqual([(p["version"], p.get("source")) for p in entries], [(vendored, None)])

    def test_a_sampled_multisampled_texture_has_the_multisampled_target(self):
        # without it the texture gets no storage on OpenGL (GL_INVALID_ENUM) and a 3D frame is empty
        gles = self.HAL / "src/gles"
        self.assertIn(
            "(false, 1) if desc.sample_count > 1 => glow::TEXTURE_2D_MULTISAMPLE,",
            (gles / "mod.rs").read_text(),
        )
        self.assertIn("_ if target == glow::TEXTURE_2D_MULTISAMPLE => {}", (gles / "device.rs").read_text())
        self.assertIn(
            "glow::TEXTURE_2D | glow::TEXTURE_2D_MULTISAMPLE | glow::TEXTURE_CUBE_MAP => false,",
            (gles / "conv.rs").read_text(),
        )
        self.assertIn("glow::TEXTURE_2D | glow::TEXTURE_2D_MULTISAMPLE => target,", (gles / "queue.rs").read_text())

    def test_a_desktop_context_is_asked_for_newest_version_first(self):
        # the NVIDIA driver gives exactly the version asked for, and 3.3 has no storage buffers or compute shaders
        egl = (self.HAL / "src/gles/egl.rs").read_text()
        self.assertIn("const DESKTOP_GL_VERSIONS: [(i32, i32); 5] = [(4, 6), (4, 5), (4, 4), (4, 3), (3, 3)];", egl)
        self.assertIn("for attributes in &gl_context_attributes {", egl)


if __name__ == "__main__":
    unittest.main()
