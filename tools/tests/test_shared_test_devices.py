"""A Rust test opens its GPU device once per process, inside a `OnceLock::get_or_init`, never once per test.

Tests run on the threads of one process. When each test opens a device and drops it at its end, one thread destroys a
device while others create theirs or exit, and the NVIDIA driver has deadlocked the whole test binary there for good
(`vkDestroyDevice` joining a driver thread on one thread, device creation and thread-exit destructors blocked on the
driver's own lock on others). A device kept for the process is never destroyed while tests run, so tests share one per
backend: `common::gpu()` and `common::gpu_on(backends)` in `crates/sr-gpu/tests`, `common::gpu()` in
`crates/sr-deliver/tests`. A second device that a test means to open (`Gpu::open_like`) is not covered here.
"""
import pathlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
SKIPPED_DIRS = {"target", "vendor", "_tmp", "node_modules"}

# A call that opens a device from scratch.
OPEN = re.compile(r"\bGpu::(?:new|new_for|with_options|with_options_for)\s*\(")
CALLEE = re.compile(r"([A-Za-z_][A-Za-z_0-9]*)\s*$")

# Test files that may open a device outside a shared cell, and why. Keep this short: a new entry needs a reason that
# holds for every test in the file.
ALLOWED = {
    # One test per binary (it sets a process-wide environment variable), so no other test thread runs beside it.
    "crates/sr-deliver/tests/debug_layers_serial.rs": "one test per binary",
    # The command-line tests run the program in child processes; this probe is the only device the test process opens.
    "crates/scene-render/tests/cli/cli.rs": "the only in-process device of a binary whose tests run children",
}


def shared(text, at):
    """Whether position `at` lies inside the argument list of a `get_or_init` call, at any depth."""
    i, depth = at, 0
    while i > 0:
        i -= 1
        c = text[i]
        if c in ")]}":
            depth += 1
        elif c in "([{":
            if depth:
                depth -= 1
                continue
            if c == "(":
                m = CALLEE.search(text[:i])
                if m and m.group(1) == "get_or_init":
                    return True
    return False


def violations(text):
    """1-based line numbers of device openings in `text` that are not inside a `get_or_init`."""
    found = []
    for m in OPEN.finditer(text):
        line_start = text.rfind("\n", 0, m.start()) + 1
        if text[line_start:m.start()].lstrip().startswith("//"):
            continue
        if not shared(text, m.start()):
            found.append(text.count("\n", 0, m.start()) + 1)
    return found


def test_sources():
    for path in sorted((ROOT / "crates").glob("*/tests/**/*.rs")):
        rel = path.relative_to(ROOT)
        if not SKIPPED_DIRS.intersection(rel.parts):
            yield rel.as_posix(), path


class SharedTestDevices(unittest.TestCase):
    def test_tests_share_one_device_per_backend(self):
        bad = [
            f"{rel}:{line}"
            for rel, path in test_sources()
            if rel not in ALLOWED
            for line in violations(path.read_text(encoding="utf-8"))
        ]
        self.assertEqual(bad, [], "open test devices once per process, inside OnceLock::get_or_init (common::gpu_on)")

    def test_every_allowed_file_exists_and_still_opens_a_device(self):
        for rel in ALLOWED:
            path = ROOT / rel
            self.assertTrue(path.is_file(), f"{rel} is allowed but does not exist")
            self.assertTrue(violations(path.read_text(encoding="utf-8")), f"{rel} no longer needs its allowance")

    def test_the_sweep_sees_shared_devices(self):
        self.assertTrue(
            any(OPEN.search(path.read_text(encoding="utf-8")) for _, path in test_sources()),
            "no device opening found: the sweep reads the wrong tree",
        )


class Detector(unittest.TestCase):
    def test_a_device_per_test_is_flagged(self):
        self.assertEqual(violations("fn t() {\n    let g = Gpu::new().unwrap();\n}"), [2])
        self.assertEqual(violations("let g = sr_gpu::Gpu::with_options(&GpuOptions { backends, adapter: None });"), [1])
        self.assertEqual(violations("match Gpu::with_options_for(&o, true) { _ => {} }"), [1])

    def test_a_device_in_a_shared_cell_is_accepted(self):
        self.assertEqual(violations("G.get_or_init(|| Gpu::new().map_err(|e| e.to_string()))"), [])
        self.assertEqual(
            violations("cell.get_or_init(|| {\n    Gpu::with_options(&GpuOptions { backends, adapter: None })\n})"), []
        )

    def test_a_cell_elsewhere_in_the_function_does_not_excuse_a_call(self):
        self.assertEqual(violations("fn t() {\n    G.get_or_init(|| 1);\n    let g = Gpu::new();\n}"), [3])

    def test_a_second_device_and_comments_are_ignored(self):
        self.assertEqual(violations("let twin = g.open_like().unwrap();\n// Gpu::new() per test deadlocked\n"), [])


if __name__ == "__main__":
    unittest.main()
