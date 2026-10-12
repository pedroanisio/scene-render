"""Compare successful end-to-end GitHub Actions runs, including queue time."""

import argparse
from datetime import datetime
import json
from pathlib import Path


def timestamp(value):
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


def elapsed(run):
    if run["status"] != "completed" or run["conclusion"] != "success" or not run["jobs"]:
        raise ValueError("a complete successful run is required")
    if any(job["status"] != "completed" or job["conclusion"] not in ("success", "skipped") for job in run["jobs"]):
        raise ValueError("all jobs must have completed successfully")
    seconds = (max(timestamp(job["completedAt"]) for job in run["jobs"]) - timestamp(run["createdAt"])).total_seconds()
    if seconds <= 0:
        raise ValueError("workflow duration must be positive")
    return seconds


def pipeline_elapsed(ci, release):
    """Measure push through packaging, including waiting but not double-counting it."""
    elapsed(ci)
    elapsed(release)
    if not ci.get("headSha") or ci["headSha"] != release.get("headSha"):
        raise ValueError("CI and release must validate the same source commit")
    start = min(timestamp(run["createdAt"]) for run in (ci, release))
    end = max(timestamp(job["completedAt"]) for run in (ci, release) for job in run["jobs"])
    return (end - start).total_seconds()


def compare(baseline, candidate, target):
    before, after = elapsed(baseline), elapsed(candidate)
    return {"baseline_seconds": before, "candidate_seconds": after, "target_speedup": target,
            "speedup": before / after, "achieved": before / after >= target}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline")
    parser.add_argument("candidate")
    parser.add_argument("--target", type=float, default=20)
    parser.add_argument("--baseline-release")
    parser.add_argument("--candidate-release")
    parser.add_argument("--cache-state", choices=["cold", "warm", "mixed"], required=True)
    args = parser.parse_args()
    if bool(args.baseline_release) != bool(args.candidate_release):
        parser.error("both release run files are required for a full-pipeline comparison")
    baseline, candidate = (json.loads(Path(path).read_text()) for path in (args.baseline, args.candidate))
    result = compare(baseline, candidate, args.target)
    result["scope"] = "CI"
    if args.baseline_release:
        before = pipeline_elapsed(baseline, json.loads(Path(args.baseline_release).read_text()))
        after = pipeline_elapsed(candidate, json.loads(Path(args.candidate_release).read_text()))
        result.update(baseline_seconds=before, candidate_seconds=after, speedup=before / after,
                      achieved=before / after >= args.target, scope="CI through release packaging")
    result["candidate_cache_state"] = args.cache_state
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
