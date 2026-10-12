"""The CI job that runs these Python tests has what they import: the corpus regeneration test needs lxml, which only
the runtime action's `evidence` option installs, and the setup action only passes that on when `runtime` is not
switched off for the job."""
import pathlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
CI = ROOT / ".github/workflows/ci.yml"
SETUP = ROOT / ".github/actions/setup/action.yml"
RUNTIME = ROOT / ".github/actions/runtime/action.yml"


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


if __name__ == "__main__":
    unittest.main()
