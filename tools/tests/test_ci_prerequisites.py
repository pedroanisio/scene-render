"""The CI job that runs these Python tests has what they import: the corpus regeneration test needs lxml, which only
the runtime action's `evidence` option installs, and the setup action only passes that on when `runtime` is not
switched off for the job. That job also installs the toolchain the maze.wasm rebuild check pins and runs the tests
with SR_REQUIRE_WASM=1, and the runtime action makes a missing GPU, Vulkan or OpenGL adapter, or FFmpeg fail the renderer
tests instead of skipping them."""
import pathlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
CI = ROOT / ".github/workflows/ci.yml"
SETUP = ROOT / ".github/actions/setup/action.yml"
RUNTIME = ROOT / ".github/actions/runtime/action.yml"
MAZE_TOOLCHAIN = ROOT / "examples/program-maze/maze/rust-toolchain.toml"


def jobs(text):
    """Job name -> its YAML text, for the top-level entries under `jobs:`."""
    body = text.split("\njobs:\n", 1)[1]
    parts = re.split(r"^  ([A-Za-z0-9_-]+):\s*$", body, flags=re.M)
    return {parts[i]: parts[i + 1] for i in range(1, len(parts) - 1, 2)}


def chain_problems(ci, setup, runtime):
    out = []
    holders = [n for n, j in jobs(ci).items() if "unittest discover -s tools/tests" in j]
    if len(holders) != 1:
        return [f"expected one job that runs tools/tests, found {holders}"]
    job = jobs(ci)[holders[0]]
    if "./.github/actions/setup" not in job:
        out.append(f"job {holders[0]} does not use the setup action")
    if re.search(r"runtime:\s*'?false'?", job):
        out.append(f"job {holders[0]} turns the runtime off, so lxml is not installed")
    if not re.search(r"uses: \./\.github/actions/runtime\s*\n\s*if: inputs\.runtime == 'true'\s*\n\s*with:\s*\n"
                     r"(?:\s+\w+: '[a-z]+'\s*\n)*?\s*evidence: 'true'", setup):
        out.append("the setup action does not ask the runtime action for the evidence tools")
    if not re.search(r'inputs\.evidence \}\}" = true \]; then\s*\n\s*runtime_packages\+=\([^)]*python3-lxml', runtime):
        out.append("the runtime action does not install python3-lxml with the evidence tools")
    return out


def requirement_problems(ci, runtime, toolchain):
    """What would let a check that CI is meant to run skip instead."""
    out = []
    holders = [n for n, j in jobs(ci).items() if "unittest discover -s tools/tests" in j]
    if len(holders) != 1:
        return [f"expected one job that runs tools/tests, found {holders}"]
    job = jobs(ci)[holders[0]]
    steps = re.split(r"^      - ", job, flags=re.M)
    install = [i for i, st in enumerate(steps)
               if re.match(r"run: rustup toolchain install\s*\n\s+working-directory: examples/program-maze/maze\s*$", st)]
    tests = [i for i, st in enumerate(steps) if "unittest discover -s tools/tests" in st]
    if not install or install[0] > tests[0]:
        out.append(f"job {holders[0]} does not install the maze toolchain before it runs tools/tests")
    if not re.search(r"^\s+env:\s*\n(?:\s+\w+: .*\n)*?\s+SR_REQUIRE_WASM: '?1'?\s*$", steps[tests[0]], flags=re.M):
        out.append(f"job {holders[0]} runs tools/tests without SR_REQUIRE_WASM: 1")
    if not re.search(r'^targets = \[[^\]]*"wasm32-unknown-unknown"', toolchain, flags=re.M):
        out.append("the maze toolchain file does not list the wasm32-unknown-unknown target")
    for name in ["SR_REQUIRE_GPU", "SR_REQUIRE_FFMPEG", "SR_REQUIRE_VULKAN", "SR_REQUIRE_GL"]:
        if not re.search(r'^\s+echo "' + name + r'=1" >> "\$GITHUB_ENV"$', runtime, flags=re.M):
            out.append(f"the runtime action does not set {name}=1")
    if not re.search(r"runtime_packages=\([^)]*\bffmpeg\b", runtime):
        out.append("the runtime action does not install ffmpeg")
    return out


class CiPrerequisiteTests(unittest.TestCase):
    def test_the_python_test_job_installs_lxml(self):
        self.assertEqual(chain_problems(CI.read_text(), SETUP.read_text(), RUNTIME.read_text()), [])

    def test_a_job_that_turns_the_runtime_off_is_found(self):
        ci = CI.read_text()
        holder = next(n for n, j in jobs(ci).items() if "unittest discover -s tools/tests" in j)
        broken = ci.replace("  " + holder + ":\n", "  " + holder + ":\n    # x\n", 1)
        broken = re.sub(r"(  " + holder + r":\n(?:.*\n)*?      - uses: \./\.github/actions/setup\n)",
                        r"\1        with:\n          runtime: 'false'\n", broken, count=1)
        found = chain_problems(broken, SETUP.read_text(), RUNTIME.read_text())
        self.assertTrue(any("turns the runtime off" in f for f in found), found)

    def test_a_runtime_action_without_lxml_is_found(self):
        found = chain_problems(CI.read_text(), SETUP.read_text(), RUNTIME.read_text().replace("python3-lxml", ""))
        self.assertTrue(any("python3-lxml" in f for f in found), found)

    def test_a_setup_action_that_drops_the_evidence_option_is_found(self):
        found = chain_problems(CI.read_text(), SETUP.read_text().replace("evidence: 'true'", "evidence: 'false'"),
                               RUNTIME.read_text())
        self.assertTrue(any("evidence tools" in f for f in found), found)


class CiRequirementTests(unittest.TestCase):
    def files(self):
        return CI.read_text(), RUNTIME.read_text(), MAZE_TOOLCHAIN.read_text()

    def test_ci_requires_the_wasm_target_ffmpeg_and_a_gpu(self):
        self.assertEqual(requirement_problems(*self.files()), [])

    def test_a_missing_toolchain_install_is_found(self):
        ci, runtime, toolchain = self.files()
        broken = ci.replace("      - run: rustup toolchain install\n        working-directory: examples/program-maze/maze\n", "")
        self.assertNotEqual(broken, ci)
        found = requirement_problems(broken, runtime, toolchain)
        self.assertTrue(any("maze toolchain before" in f for f in found), found)

    def test_a_dropped_wasm_target_is_found(self):
        ci, runtime, toolchain = self.files()
        found = requirement_problems(ci, runtime, toolchain.replace('"wasm32-unknown-unknown"', ""))
        self.assertTrue(any("wasm32-unknown-unknown target" in f for f in found), found)

    def test_a_dropped_sr_require_wasm_is_found(self):
        ci, runtime, toolchain = self.files()
        broken = ci.replace("SR_REQUIRE_WASM: 1", "SR_REQUIRE_WASM: 0")
        self.assertNotEqual(broken, ci)
        found = requirement_problems(broken, runtime, toolchain)
        self.assertTrue(any("SR_REQUIRE_WASM" in f for f in found), found)

    def test_a_dropped_requirement_in_the_runtime_action_is_found(self):
        ci, runtime, toolchain = self.files()
        for name in ["SR_REQUIRE_GPU", "SR_REQUIRE_FFMPEG", "SR_REQUIRE_VULKAN", "SR_REQUIRE_GL"]:
            broken = runtime.replace(f'echo "{name}=1"', f'echo "{name}=0"')
            self.assertNotEqual(broken, runtime)
            found = requirement_problems(ci, broken, toolchain)
            self.assertTrue(any(name in f for f in found), found)


if __name__ == "__main__":
    unittest.main()
