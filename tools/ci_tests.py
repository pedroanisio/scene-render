"""Schedule individual libtest cases without losing process-local test fixtures.

Build and worker checkouts must use the same absolute workspace path because Rust
fixtures embed CARGO_MANIFEST_DIR. GitHub's Ubuntu jobs satisfy that contract.
The archive includes runtime executables, workspace build outputs and shared
libraries; workers verify the source commit and each test executable's SHA-256.
Doctests and release-profile performance checks stay in their dedicated jobs.

ci-test-durations.json starts from successful CI run 38083357412 (b7eb4edc).
GPU costs are serial completion intervals; CPU costs are target runtime times
its two threads divided by case count. The former whole-corpus backend test's
929 seconds are distributed over 32 batches. These are scheduling estimates,
not benchmark results. Missing cases always run and receive the mean known cost.
"""

import argparse
import hashlib
import heapq
import json
import math
import os
import re
import subprocess
import sys
from pathlib import Path, PurePosixPath


def partition_tests(names, count, durations=None):
    """Assign every case once, longest first; unknown cases receive the mean cost."""
    if count < 1 or len(names) != len(set(names)):
        raise ValueError("positive worker count and unique test names required")
    durations = durations or {}
    if any(not math.isfinite(value) or value < 0 for value in durations.values()):
        raise ValueError("test durations must be finite and nonnegative")
    known = [durations[name] for name in names if name in durations]
    fallback = max(1.0, sum(known) / len(known)) if known else 1.0
    cost = {name: durations.get(name, fallback) for name in names}
    shards = [[] for _ in range(count)]
    workers = [(0.0, index) for index in range(count)]
    heapq.heapify(workers)
    for name in sorted(names, key=lambda name: (-cost[name], name)):
        elapsed, index = heapq.heappop(workers)
        shards[index].append(name)
        heapq.heappush(workers, (elapsed + cost[name], index))
    return [sorted(shard) for shard in shards]


def parse_tests(output):
    """Read the stable libtest listing, excluding summary lines and benchmarks."""
    return [line[:-6] for line in output.splitlines() if line.endswith(": test")]


def test_command(binary, names, threads=1):
    """Keep the selected cases in one process and avoid substring-filter overlap."""
    if not names or threads < 1:
        raise ValueError("nonempty test selection and positive thread count required")
    return [str(binary), "--exact", *names, f"--test-threads={threads}"]


def complete_result(output, selected, ignored):
    """A successful process must also account for every selected case."""
    matches = re.findall(r"test result: ok\. (\d+) passed; 0 failed; (\d+) ignored;", output)
    return bool(matches) and tuple(map(int, matches[-1])) == (selected - ignored, ignored)


def validate_manifest(manifest):
    """Refuse ambiguous inventories and paths outside the checked-out workspace."""
    identifiers = set()
    for binary in manifest["binaries"]:
        if binary["id"] in identifiers:
            raise ValueError("duplicate test binary: " + binary["id"])
        identifiers.add(binary["id"])
        for key in ("path", "cwd"):
            path = PurePosixPath(binary[key])
            if path.is_absolute() or ".." in path.parts or "\\" in binary[key]:
                raise ValueError("test paths must stay inside the workspace")
        tests, ignored = binary["tests"], binary["ignored"]
        if len(tests) != len(set(tests)) or len(ignored) != len(set(ignored)) or not set(ignored) <= set(tests):
            raise ValueError("inconsistent test inventory: " + binary["id"])


def verify_reports(manifest, reports, count):
    """Require a successful, disjoint, complete report from every worker."""
    validate_manifest(manifest)
    expected = {f"{binary['id']}::{name}" for binary in manifest["binaries"] for name in binary["tests"]}
    if len(reports) != count or {report["index"] for report in reports} != set(range(count)):
        raise ValueError("missing or duplicate worker reports")
    seen = []
    for report in reports:
        if report["commit"] != manifest["commit"] or report["count"] != count or report["failures"]:
            raise ValueError("worker failed or belongs to another run")
        actual = []
        for result in report["results"]:
            if not result["success"] or result["exit_code"]:
                raise ValueError("test batch failed")
            actual.extend(f"{result['binary']}::{name}" for name in result["tests"])
        if sorted(actual) != sorted(report["selected"]):
            raise ValueError("worker did not execute its complete selection")
        seen.extend(actual)
    if len(seen) != len(set(seen)) or set(seen) != expected:
        raise ValueError("test coverage is incomplete or duplicated")
    return len(seen)


def verify_fuzz_reports(reports, count, iterations):
    """Every strided partition must execute its exact share of the serial campaign."""
    if count < 1 or iterations < 1 or len(reports) != count:
        raise ValueError("missing fuzz partitions")
    if {report["index"] for report in reports} != set(range(count)):
        raise ValueError("missing or duplicate fuzz partitions")
    seed_counts = {report.get("seeds", 0) for report in reports}
    if len(seed_counts) != 1 or min(seed_counts) < 545:
        raise ValueError("fuzz seed corpus is incomplete or differs between workers")
    for report in reports:
        expected = len(range(report["index"], iterations, count))
        if (report["seed"] != 0x5eed or report["iterations"] != iterations or report["count"] != count
                or report["crashes"] != 0 or report["docs"] < 0 or report["exprs"] < 0
                or report["docs"] + report["exprs"] != expected):
            raise ValueError("fuzz campaign failed or omitted inputs")
    return iterations


def digest(path):
    with Path(path).open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def build(args):
    root = Path.cwd().resolve()
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--locked", "--format-version=1"], text=True))
    packages = {package["id"]: package for package in metadata["packages"]}
    command = ["cargo", "test", "--locked", "--no-run", "--message-format=json", "--profile", args.profile]
    if args.package:
        for package in args.package:
            command.extend(["-p", package])
    else:
        command.append("--workspace")
    if args.features:
        command.extend(["--features", args.features])
    artifacts, files = [], set()
    with subprocess.Popen(command, stdout=subprocess.PIPE, text=True) as process:
        for line in process.stdout:
            item = json.loads(line)
            if item["reason"] == "compiler-message":
                print(item["message"].get("rendered", ""), file=sys.stderr, end="")
            elif item["reason"] == "compiler-artifact":
                if item.get("executable"):
                    files.add(item["executable"])
                    if item["profile"]["test"]:
                        artifacts.append(item)
                files.update(path for path in item["filenames"] if path.endswith((".so", ".dylib", ".dll")))
            elif item["reason"] == "build-script-executed" and item["package_id"] in packages:
                files.add(item["out_dir"])
        if process.wait():
            raise RuntimeError("test build failed")
    binaries = []
    for artifact in artifacts:
        package = packages[artifact["package_id"]]
        binary = Path(artifact["executable"])
        cwd = Path(package["manifest_path"]).parent
        tests = parse_tests(subprocess.check_output([str(binary), "--list", "--format=terse"], cwd=cwd, text=True))
        ignored = parse_tests(subprocess.check_output(
            [str(binary), "--list", "--ignored", "--format=terse"], cwd=cwd, text=True))
        kind = "+".join(artifact["target"]["kind"])
        binaries.append({"id": f"{package['name']}/{kind}/{artifact['target']['name']}",
                         "package": package["name"], "target": artifact["target"]["name"],
                         "path": binary.relative_to(root).as_posix(), "cwd": cwd.relative_to(root).as_posix(),
                         "sha256": digest(binary), "tests": tests, "ignored": ignored})
    manifest = {"commit": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
                "profile": args.profile, "binaries": sorted(binaries, key=lambda binary: binary["id"])}
    validate_manifest(manifest)
    if not binaries:
        raise RuntimeError("the build produced no test binaries")
    output = Path(args.output)
    output.mkdir(parents=True, exist_ok=True)
    manifest_path = output / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
    files.add(str(manifest_path.resolve()))
    write_archive(root, files, output / "tests.tar.zst")
    print(f"Archived {len(binaries)} binaries and {sum(len(binary['tests']) for binary in binaries)} test cases.")



def write_archive(root, files, archive):
    """Use a 256 MiB dictionary to compress code shared by large Rust executables."""
    root, archive = Path(root).resolve(), Path(archive).resolve()
    archive.parent.mkdir(parents=True, exist_ok=True)
    relative = sorted(str(Path(path).resolve().relative_to(root)) for path in files)
    if any("\n" in path for path in relative):
        raise ValueError("archive paths must not contain newlines")
    file_list = archive.with_suffix(".files.txt")
    file_list.write_text("\n".join(relative) + "\n")
    subprocess.run(["tar", "-I", "zstd -T2 --long=28", "-cf", str(archive), "-C", str(root),
                    "-T", str(file_list)], check=True)


def run(args):
    root = Path.cwd().resolve()
    manifest = json.loads(Path(args.manifest).read_text())
    validate_manifest(manifest)
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    if commit != manifest["commit"]:
        raise ValueError("test artifact and checkout commits differ")
    if not 0 <= args.index < args.count:
        raise ValueError("invalid worker index")
    cases = {f"{binary['id']}::{name}": (binary, name)
             for binary in manifest["binaries"] for name in binary["tests"]}
    durations = json.loads(Path(args.timings).read_text()) if args.timings else {}
    partitions = partition_tests(list(cases), args.count, durations)
    selected = partitions[args.index]
    groups = {}
    for case in selected:
        binary, name = cases[case]
        groups.setdefault(binary["id"], (binary, []))[1].append(name)
    results, failures = [], 0
    output = Path(args.output)
    output.mkdir(parents=True, exist_ok=True)
    for identifier, (binary, names) in groups.items():
        executable = root / binary["path"]
        if digest(executable) != binary["sha256"]:
            raise ValueError("test executable checksum differs: " + identifier)
        executable.chmod(executable.stat().st_mode | 0o111)
        env = dict(os.environ, CARGO_MANIFEST_DIR=str(root / binary["cwd"]))
        # Bound argv size on Windows while retaining shared state for each batch.
        for offset in range(0, len(names), 64):
            batch = names[offset:offset + 64]
            command = test_command(executable, batch, args.threads)
            print(f"Running {identifier}: {len(batch)} cases", flush=True)
            result = subprocess.run(command, cwd=root / binary["cwd"], env=env,
                                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
            print(result.stdout, end="", flush=True)
            ignored = len(set(batch) & set(binary["ignored"]))
            success = result.returncode == 0 and complete_result(result.stdout, len(batch), ignored)
            failures += not success
            results.append({"binary": identifier, "tests": batch, "ignored": ignored,
                            "exit_code": result.returncode, "success": success})
            (output / f"{args.index}-{len(results)}.log").write_text(result.stdout)
    report = {"commit": commit, "index": args.index, "count": args.count, "selected": selected,
              "inventory_count": len(cases), "results": results, "failures": failures}
    (output / f"worker-{args.index}.json").write_text(json.dumps(report, indent=2) + "\n")
    if failures:
        raise RuntimeError(f"{failures} test batches failed or omitted cases")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    build_parser = commands.add_parser("build")
    build_parser.add_argument("--output", default="target/ci-tests")
    build_parser.add_argument("--profile", default="test")
    build_parser.add_argument("--package", action="append")
    build_parser.add_argument("--features", default="")
    run_parser = commands.add_parser("run")
    run_parser.add_argument("index", type=int)
    run_parser.add_argument("count", type=int)
    run_parser.add_argument("--manifest", default="target/ci-tests/manifest.json")
    run_parser.add_argument("--timings")
    run_parser.add_argument("--threads", type=int, default=1)
    run_parser.add_argument("--output", default="target/ci-results")
    verify_parser = commands.add_parser("verify")
    verify_parser.add_argument("count", type=int)
    verify_parser.add_argument("--manifest", default="target/ci-tests/manifest.json")
    verify_parser.add_argument("--reports", default="target/ci-results")
    verify_parser.add_argument("--fuzz-iterations", type=int)
    args = parser.parse_args()
    if args.command == "verify":
        manifest = json.loads(Path(args.manifest).read_text())
        reports = [json.loads(path.read_text()) for path in Path(args.reports).rglob("worker-*.json")]
        total = verify_reports(manifest, reports, args.count)
        print(f"All {total} test cases accounted for exactly once across {args.count} successful workers.")
        if args.fuzz_iterations is not None:
            fuzz_reports = [json.loads(path.read_text()) for path in Path(args.reports).rglob("fuzz-*.json")]
            total = verify_fuzz_reports(fuzz_reports, args.count, args.fuzz_iterations)
            print(f"All {total} fuzz inputs accounted for exactly once across {args.count} successful workers.")
    else:
        (build if args.command == "build" else run)(args)


if __name__ == "__main__":
    main()
