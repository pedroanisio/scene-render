"""Full coverage must survive the CI scheduling changes."""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[2]

class WorkflowCoverage(unittest.TestCase):
    def test_tests_are_built_once_and_workers_only_execute_the_archive(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        self.assertIn("tools/ci_tests.py build", workflow)
        self.assertIn("tools/ci_tests.py run", workflow)
        self.assertIn("tools/ci_tests.py verify", workflow)
        self.assertIn("--workspace --doc", workflow)

    def test_portability_and_msrv_keep_their_checks_and_have_build_caches(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        for name in ("msrv:", "portability:"):
            job = re.split(r"\n  [a-z][\w-]*:", workflow.split("  " + name, 1)[1], maxsplit=1)[0]
            self.assertIn("Swatinem/rust-cache@v2", job)
        self.assertIn("cargo check --workspace --all-targets --locked", workflow)
        self.assertIn("cargo test -p sr-model -p sr-eval -p sr-resolve --lib --locked", workflow)

    def test_evidence_and_oracle_share_one_release_build(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        self.assertEqual(workflow.count("cargo build --locked --release -p scene-render -p sr-fuzz"), 1)
        for name in ("evidence:", "oracle:", "performance-smoke:"):
            job = re.split(r"\n  [a-z][\w-]*:", workflow.split("  " + name, 1)[1], maxsplit=1)[0]
            self.assertIn("needs: build-tools", job)
            self.assertIn("name: compiled-linux-tools", job)
            self.assertNotIn("cargo build", job)

    def test_performance_gate_reuses_exact_commit_tools_with_a_build_fallback(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        gate = workflow.split("  performance-gate:", 1)[1].split("  performance:", 1)[0]
        self.assertIn("needs: build-tools", gate)
        self.assertIn("tools/ci_artifact.py", gate)
        self.assertIn("name: compiled-linux-tools", gate)
        self.assertIn('cd "$RUNNER_TEMP/base" && cargo build', gate)

    def test_fixed_fuzz_campaign_is_distributed_and_its_complete_work_is_verified(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        self.assertIn("--iterations 2400000 --partition", workflow)
        self.assertIn("--fuzz-iterations 2400000", workflow)
        self.assertIn("--report target/ci-results/fuzz-", workflow)
        self.assertNotIn("sr-fuzz --seconds 600", workflow)

    def test_shader_cache_is_separated_by_driver_and_worker(self):
        runtime = (ROOT / ".github/actions/runtime/action.yml").read_text()
        self.assertIn("MESA_SHADER_CACHE_DIR", runtime)
        self.assertIn("actions/cache@v4", runtime)
        self.assertIn("steps.runtime.outputs.mesa-version", runtime)
        self.assertIn("inputs.cache-key", runtime)
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        workers = workflow.split("  tests:", 1)[1].split("  test-coverage:", 1)[0]
        self.assertIn("cache-key: ${{ matrix.shard }}", workers)

    def test_release_smokes_run_in_parallel_before_package_promotion(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        release = (ROOT / ".github/workflows/release.yml").read_text()
        package = (ROOT / ".github/actions/package/action.yml").read_text()
        job = workflow.split("  package:", 1)[1].split("  evidence:", 1)[0]
        self.assertIn("needs: build-tools", job)
        self.assertIn("./.github/actions/package", job)
        self.assertIn("name: linux-release-package", job)
        self.assertIn("name: linux-release-package", release)
        self.assertNotIn("Install runtime", release)
        for check in ("check_release_hygiene.py --all", "--strict", "--codec ffv1",
                      "nb_read_frames", 'sha256sum -c SHA256SUMS'):
            self.assertIn(check, package)
        self.assertIn('tar -xOf', release)
        self.assertIn('sha256sum -c', release)

    def test_test_builder_preserves_test_executables_with_their_fingerprints(self):
        setup = (ROOT / ".github/actions/setup/action.yml").read_text()
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        job = workflow.split("  build-tests:", 1)[1].split("  tests:", 1)[0]
        self.assertIn("cache-profile: debug", job)
        tools_job = workflow.split("  build-tools:", 1)[1].split("  package:", 1)[0]
        self.assertIn("cache-profile: release", tools_job)
        self.assertIn("path: |\n          target/${{ inputs.cache-profile }}\n          .cache/ci-source-times.json", setup)
        self.assertIn("steps.compiler.outputs.identity", setup)
        self.assertIn("github.sha", setup)
        self.assertIn("restore-keys:", setup)

    def test_compile_jobs_skip_runtime_installation_but_execution_workers_keep_it(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        for name in ("build-tests:", "build-tools:"):
            job = re.split(r"\n  [a-z][\w-]*:", workflow.split("  " + name, 1)[1], maxsplit=1)[0]
            self.assertIn("runtime: 'false'", job)
        workers = workflow.split("  tests:", 1)[1].split("  test-coverage:", 1)[0]
        self.assertIn("./.github/actions/runtime", workers)
        self.assertIn("heic: 'true'", workers)

    def test_rust_caches_keep_content_checked_source_timestamps(self):
        setup = (ROOT / ".github/actions/setup/action.yml").read_text()
        self.assertIn(".cache/ci-source-times.json", setup)
        self.assertIn("python3 tools/ci_source_times.py", setup)
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        for name in ("msrv:", "portability:"):
            job = re.split(r"\n  [a-z][\w-]*:", workflow.split("  " + name, 1)[1], maxsplit=1)[0]
            self.assertIn(".cache/ci-source-times.json", job)
            self.assertIn("python3 tools/ci_source_times.py", job)

    def test_ci_can_be_measured_as_a_fresh_manual_run(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        self.assertIn("  workflow_dispatch:", workflow)

    def test_rendering_evidence_and_performance_gates_remain(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        for required in ("tools/evidence.py", "tools/oracle_diff.py 2000 1", "tools/perf_gate.py",
                         "cargo test --release --test perf", "validated-linux-tools"):
            self.assertIn(required, workflow)

if __name__ == "__main__":
    unittest.main()
