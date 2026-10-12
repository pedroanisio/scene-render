"""Speedup claims require complete, successful, comparable workflow runs."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("ci_timings", Path(__file__).resolve().parents[1] / "ci_timings.py")
ci_timings = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ci_timings)


def workflow(start, end, conclusion="success"):
    return {"status": "completed", "conclusion": conclusion, "createdAt": start,
            "jobs": [{"name": "checks", "status": "completed", "conclusion": conclusion,
                      "startedAt": start, "completedAt": end}]}


class TimingTests(unittest.TestCase):
    def test_elapsed_includes_queue_time_and_uses_the_last_job(self):
        run = workflow("2026-10-10T10:00:00Z", "2026-10-10T10:10:00Z")
        run["jobs"][0]["startedAt"] = "2026-10-10T10:02:00Z"
        self.assertEqual(ci_timings.elapsed(run), 600)

    def test_only_a_measured_twentyfold_reduction_satisfies_the_target(self):
        base = workflow("2026-10-10T10:00:00Z", "2026-10-10T11:00:00Z")
        fast = workflow("2026-10-10T12:00:00Z", "2026-10-10T12:03:00Z")
        result = ci_timings.compare(base, fast, 20)
        self.assertEqual(result["speedup"], 20)
        self.assertTrue(result["achieved"])
        slower = workflow("2026-10-10T12:00:00Z", "2026-10-10T12:04:00Z")
        self.assertFalse(ci_timings.compare(base, slower, 20)["achieved"])

    def test_full_pipeline_includes_packaging_without_counting_overlap_twice(self):
        ci = dict(workflow("2026-10-10T10:00:00Z", "2026-10-10T11:00:00Z"), headSha="abc")
        release = dict(workflow("2026-10-10T10:30:00Z", "2026-10-10T11:05:00Z"), headSha="abc")
        self.assertEqual(ci_timings.pipeline_elapsed(ci, release), 3900)
        with self.assertRaises(ValueError):
            ci_timings.pipeline_elapsed(ci, dict(release, headSha="other"))
        with self.assertRaises(ValueError):
            ci_timings.pipeline_elapsed(ci, dict(release, conclusion="failure"))

    def test_failed_incomplete_and_empty_runs_cannot_prove_speedup(self):
        base = workflow("2026-10-10T10:00:00Z", "2026-10-10T11:00:00Z")
        for bad in [dict(base, status="in_progress"), dict(base, conclusion="failure"), dict(base, jobs=[])]:
            with self.assertRaises(ValueError):
                ci_timings.elapsed(bad)

if __name__ == "__main__":
    unittest.main()
