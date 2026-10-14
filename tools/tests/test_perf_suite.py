import json
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


TOOLS = Path(__file__).resolve().parents[1]


sys.path.insert(0, str(TOOLS))
import perf_suite  # noqa: E402


class EncodeReportTests(unittest.TestCase):
    def envelope(self, outputs):
        return json.dumps({"format": "scene-render-cli/1", "schema_version": "1.0.0", "command": "encode",
                           "ok": True, "exit_code": 0, "result": {"outputs": outputs}, "diagnostics": [],
                           "error": None})

    def test_the_timings_come_from_the_last_timed_output_of_the_envelope(self):
        stdout = self.envelope([{"path": "a.mp4", "seconds": 1.0, "frames": 5},
                                {"path": "b.mp4", "seconds": 2.5, "frames": 6, "stage_seconds": [0.1, 0.2],
                                 "encoder": "libx264"},
                                {"path": "cover.png"}]) + "\n"
        rec = perf_suite.parse_encode(stdout, "")
        self.assertEqual((rec["seconds"], rec["frames"], rec["encoder"]), (2.5, 6, "libx264"))
        self.assertEqual(rec["stage_seconds"],
                         {"evaluate": 0.1, "render": 0.2, "readback_wait": None, "encoder_blocked": None})

    def test_output_without_a_timed_report_is_an_error(self):
        for stdout in ("", "not json\n", self.envelope([]), self.envelope([{"path": "cover.png"}]),
                       '{"path": "a.mp4", "seconds": 1.0}\n',
                       json.dumps({"format": "scene-render-cli/1", "result": None})):
            with self.subTest(stdout=stdout):
                with self.assertRaisesRegex(perf_suite.RunError, "no JSON report"):
                    perf_suite.parse_encode(stdout, "")


class SuiteTests(unittest.TestCase):
    def test_failed_workload_fails_the_suite_and_preserves_diagnostics(self):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            fixture = work / "perf_text"
            fixture.mkdir()
            (fixture / "perf_text.scene.xml").write_text('<scene version="1.2"/>')
            binary = work / "renderer"
            binary.write_text("#!/bin/sh\necho 'deliberate renderer failure' >&2\nexit 7\n")
            binary.chmod(binary.stat().st_mode | stat.S_IEXEC)
            report = work / "result.json"
            result = subprocess.run(
                [sys.executable, str(TOOLS / "perf_suite.py"), str(report),
                 "--binary", str(binary), "--work", str(work), "--only", "perf_text_bench"],
                capture_output=True, text=True,
            )
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
            record = json.loads(report.read_text())["fixtures"]["perf_text_bench"]
            self.assertIn("deliberate renderer failure", record["error"])
            self.assertNotIn("value", record)


if __name__ == "__main__":
    unittest.main()
