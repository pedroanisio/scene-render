"""The CI scheduler conserves every test while balancing measured runtime."""
import importlib.util
from pathlib import Path
import unittest
import tempfile
import subprocess
import sys
import json
import hashlib

spec = importlib.util.spec_from_file_location("ci_tests", Path(__file__).resolve().parents[1] / "ci_tests.py")
ci_tests = importlib.util.module_from_spec(spec)
if spec.loader:
    spec.loader.exec_module(ci_tests)


class PartitionTests(unittest.TestCase):
    def test_each_case_runs_once_even_when_there_are_more_workers_than_cases(self):
        for count in (1, 3, 32):
            names = [f"suite::case_{n}" for n in range(17)]
            shards = ci_tests.partition_tests(names, count)
            self.assertEqual(len(shards), count)
            self.assertCountEqual([name for shard in shards for name in shard], names)

    def test_measured_long_cases_are_distributed_instead_of_collecting_on_one_worker(self):
        names = ["a", "b", "c", "d", "e", "f"]
        durations = dict(zip(names, [100, 1, 100, 1, 100, 1]))
        shards = ci_tests.partition_tests(names, 2, durations)
        loads = [sum(durations[name] for name in shard) for shard in shards]
        self.assertLessEqual(max(loads), 200)

    def test_order_is_deterministic_and_new_cases_are_never_omitted(self):
        names = ["known", "new", "other"]
        self.assertEqual(ci_tests.partition_tests(names, 2, {"known": 9}),
                         ci_tests.partition_tests(names[::-1], 2, {"known": 9}))
        self.assertCountEqual(sum(ci_tests.partition_tests(names, 2, {"known": 9}), []), names)

    def test_duplicate_cases_and_invalid_partitions_or_durations_fail(self):
        for names, count, durations in [(["a", "a"], 2, {}), (["a"], 0, {}),
                (["a"], 2, {"a": -1}), (["a"], 2, {"a": float("nan")})]:
            with self.assertRaises(ValueError):
                ci_tests.partition_tests(names, count, durations)

    def test_libtest_listing_does_not_turn_benchmarks_or_summary_into_tests(self):
        self.assertEqual(ci_tests.parse_tests("alpha::one: test\nbeta: test\nbench: benchmark\n\n2 tests, 1 benchmark\n"),
                         ["alpha::one", "beta"])

    def test_selected_names_are_exact_and_share_one_test_process(self):
        args = ci_tests.test_command("/tmp/test-bin", ["alpha", "alpha_long"], threads=1)
        self.assertEqual(args, ["/tmp/test-bin", "--exact", "alpha", "alpha_long", "--test-threads=1"])
        with self.assertRaises(ValueError):
            ci_tests.test_command("/tmp/test-bin", [], threads=1)

    def test_result_requires_every_selected_case_and_keeps_ignored_cases_visible(self):
        self.assertTrue(ci_tests.complete_result(
            "test result: ok. 2 passed; 0 failed; 1 ignored; 0 measured; 8 filtered out; finished in 1s", 3, 1))
        self.assertFalse(ci_tests.complete_result(
            "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 8 filtered out; finished in 0s", 3, 1))
        self.assertFalse(ci_tests.complete_result(
            "test result: FAILED. 1 passed; 1 failed; 1 ignored; 0 measured; 8 filtered out; finished in 1s", 3, 1))

    def test_manifest_rejects_duplicate_binary_ids_and_escaping_paths(self):
        binary = {"id": "p::t", "path": "target/debug/t", "cwd": "crates/p",
                  "tests": ["one"], "ignored": []}
        ci_tests.validate_manifest({"commit": "abc", "binaries": [binary]})
        for binaries in ([binary, binary], [dict(binary, path="../foreign")],
                         [dict(binary, ignored=["absent"])], [dict(binary, tests=["one", "one"])]):
            with self.assertRaises(ValueError):
                ci_tests.validate_manifest({"commit": "abc", "binaries": binaries})

    def test_fuzz_reports_preserve_the_fixed_campaign_including_uneven_partitions(self):
        reports = [{"seed": 0x5eed, "iterations": 11, "index": index, "count": 3,
                    "docs": docs, "exprs": exprs, "crashes": 0, "seeds": 545}
                   for index, (docs, exprs) in enumerate([(3, 1), (2, 2), (1, 2)])]
        self.assertEqual(ci_tests.verify_fuzz_reports(reports, 3, 11), 11)
        for broken in [reports[:2], [reports[0], reports[0], reports[2]],
                       [dict(reports[0], iterations=10), *reports[1:]],
                       [dict(reports[0], docs=2), *reports[1:]],
                       [dict(reports[0], seed=1), *reports[1:]],
                       [dict(reports[0], seeds=2), *reports[1:]],
                       [dict(reports[0], crashes=1), *reports[1:]]]:
            with self.assertRaises(ValueError):
                ci_tests.verify_fuzz_reports(broken, 3, 11)

    def test_all_workers_must_report_complete_disjoint_coverage(self):
        manifest = {"commit": "abc", "binaries": [{"id": "p/t", "path": "target/t", "cwd": "p",
                    "tests": ["one", "two"], "ignored": []}]}
        reports = [{"commit": "abc", "index": index, "count": 2, "selected": [f"p/t::{name}"],
                    "failures": 0, "results": [{"binary": "p/t", "tests": [name], "success": True,
                                                 "exit_code": 0, "ignored": 0}]}
                   for index, name in enumerate(["one", "two"])]
        ci_tests.verify_reports(manifest, reports, 2)
        for broken in [reports[:1], [reports[0], reports[0]],
                       [dict(reports[0], commit="wrong"), reports[1]],
                       [dict(reports[0], results=[]), reports[1]]]:
            with self.assertRaises(ValueError):
                ci_tests.verify_reports(manifest, broken, 2)


class ArchiveIntegration(unittest.TestCase):
    def test_archive_round_trip_preserves_bytes_and_executable_permissions(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "target/test executable"
            binary.parent.mkdir()
            binary.write_bytes(bytes(range(256)) * 64)
            binary.chmod(0o755)
            archive = root / "target/ci-tests/tests.tar.zst"
            ci_tests.write_archive(root, {str(binary)}, archive)
            extracted = root / "extracted"
            extracted.mkdir()
            subprocess.run(["tar", "-I", "zstd -d --long=28", "-xf", str(archive), "-C", str(extracted)], check=True)
            restored = extracted / "target/test executable"
            self.assertEqual(restored.read_bytes(), binary.read_bytes())
            self.assertEqual(restored.stat().st_mode & 0o777, 0o755)


class WorkerIntegration(unittest.TestCase):
    def test_workers_execute_the_complete_inventory_and_reject_a_zero_test_success(self):
        script = Path(__file__).resolve().parents[1] / "ci_tests.py"
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            subprocess.run(["git", "init", "-q", directory], check=True)
            subprocess.run(["git", "-c", "user.name=Test", "-c", "user.email=test@example.test", "-c", "core.hooksPath=/dev/null",
                            "commit", "--allow-empty", "-qm", "fixture"], cwd=root, check=True)
            commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
            binary = root / "fake-libtest"
            binary.write_text("#!" + sys.executable + "\nimport sys\nnames = sys.argv[2:-1]\n"
                              "print(f'test result: ok. {len(names)} passed; 0 failed; 0 ignored;')\n")
            binary.chmod(0o755)
            manifest = {"commit": commit, "binaries": [{"id": "fixture/test/cases", "package": "fixture",
                "path": "fake-libtest", "cwd": ".", "sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                "tests": ["one", "two", "three", "four"], "ignored": []}]}
            (root / "manifest.json").write_text(json.dumps(manifest))
            for index in range(3):
                result = subprocess.run([sys.executable, str(script), "run", str(index), "3",
                    "--manifest", "manifest.json", "--output", "reports"], cwd=root, capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            reports = [json.loads(path.read_text()) for path in (root / "reports").glob("worker-*.json")]
            self.assertEqual(ci_tests.verify_reports(manifest, reports, 3), 4)
            binary.write_text("#!" + sys.executable + "\nprint('test result: ok. 0 passed; 0 failed; 0 ignored;')\n")
            manifest["binaries"][0]["sha256"] = hashlib.sha256(binary.read_bytes()).hexdigest()
            (root / "manifest.json").write_text(json.dumps(manifest))
            result = subprocess.run([sys.executable, str(script), "run", "0", "3", "--manifest", "manifest.json"],
                                    cwd=root, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
