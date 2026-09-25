#!/usr/bin/env python3
"""Performance-regression guard for scene-render.

Renders a fixed benchmark scene several times and compares the median
engine CPU seconds per pipeline stage (from --metrics-trace) with a committed
baseline. It also checks that output bytes did not change:

  * sample frames rendered with 1 thread and with --threads N must match
    each other (thread-count determinism), and
  * they must match the SHA-256 hashes recorded in the baseline, so a
    "speedup" that alters frames cannot pass silently.

Usage:
  scripts/perf-check.py                 # compare against the baseline
  scripts/perf-check.py --update        # (re)record the baseline on this host

Exit status: 0 pass, 1 timing regression, 2 output changed or
nondeterministic, 3 missing or incompatible baseline.

Timings are host-specific. The baseline records the host and CPU; comparing
on a different machine prints a warning, and the result means little. Keep
the machine otherwise idle while this runs.
"""
import argparse
import datetime
import hashlib
import json
import os
import platform
import statistics
import subprocess
import sys
import tempfile

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
STAGES = ["clear", "lighting", "composite", "viewport", "effects", "convert",
          "resume", "encode"]
# Stages below this median (seconds per run) are too small to judge.
NOISE_FLOOR_S = 0.05


def cpu_model():
    try:
        with open("/proc/cpuinfo") as f:
            for line in f:
                if line.startswith("model name"):
                    return line.split(":", 1)[1].strip()
    except OSError:
        pass
    return platform.processor() or "unknown"


def host_info(binary):
    version = subprocess.run([binary, "--version"], capture_output=True,
                             text=True).stdout.strip()
    return {"host": platform.node(), "cpu": cpu_model(),
            "cpus": os.cpu_count(), "engine": version}


def render(binary, scene, frames, threads, workdir, run):
    trace = os.path.join(workdir, f"trace-{run}.jsonl")
    cmd = [binary, "--scene", scene, "--frame-range", frames,
           "--threads", str(threads), "--output",
           os.path.join(workdir, f"out-{run}.mkv"), "--metrics-trace", trace]
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        sys.exit(f"render failed ({result.returncode}):\n{result.stderr}")
    with open(trace) as f:
        rows = [json.loads(line) for line in f if line.strip()]
    summary = rows[-1]
    if not summary.get("summary"):
        sys.exit(f"{trace}: missing summary row")
    return summary


def frame_hash(binary, scene, frame, threads, workdir):
    path = os.path.join(workdir, f"frame-{frame}-t{threads}.ppm")
    cmd = [binary, "--scene", scene, "--preview-frame", str(frame),
           "--preview-out", path, "--threads", str(threads)]
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        sys.exit(f"preview failed ({result.returncode}):\n{result.stderr}")
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def measure(args, workdir):
    print(f"warm-up + {args.runs} runs of {args.scene} frames {args.frames} "
          f"at --threads {args.threads}", flush=True)
    render(args.binary, args.scene, args.frames, args.threads, workdir, "warm")
    runs = [render(args.binary, args.scene, args.frames, args.threads,
                   workdir, i) for i in range(args.runs)]
    stages = {}
    for stage in STAGES:
        values = sorted(r["cpu"][stage] for r in runs)
        stages[stage] = {"median": statistics.median(values),
                         "min": values[0], "max": values[-1]}
    totals = sorted(sum(r["cpu"].values()) for r in runs)
    walls = sorted(r["render_s"] for r in runs)
    return {"stages": stages,
            "total_cpu": {"median": statistics.median(totals),
                          "min": totals[0], "max": totals[-1]},
            "render_wall": {"median": statistics.median(walls),
                            "min": walls[0], "max": walls[-1]}}


def hashes(args, workdir):
    result, ok = {}, True
    for frame in args.hash_frames:
        single = frame_hash(args.binary, args.scene, frame, 1, workdir)
        multi = frame_hash(args.binary, args.scene, frame, args.threads, workdir)
        if single != multi:
            print(f"FAIL frame {frame}: 1-thread and {args.threads}-thread "
                  "output differ")
            ok = False
        result[str(frame)] = single
    return result, ok


def spread(entry):
    return (entry["max"] - entry["min"]) / entry["median"] if entry["median"] else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--binary", default=os.path.join(ROOT, "build", "scene-render"))
    parser.add_argument("--scene", default=os.path.join(ROOT, "benchmarks", "perf-scene.xml"))
    parser.add_argument("--baseline", default=os.path.join(ROOT, "benchmarks", "baseline.json"))
    parser.add_argument("--frames", default="0:30")
    parser.add_argument("--threads", type=int, default=4)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--tolerance", type=float, default=0.15,
                        help="allowed slowdown of a stage median (0.15 = 15%%)")
    parser.add_argument("--hash-frames", type=int, nargs="+", default=[0, 15, 29])
    parser.add_argument("--update", action="store_true",
                        help="record a new baseline instead of comparing")
    args = parser.parse_args()
    args.binary = os.path.abspath(args.binary)
    args.scene = os.path.abspath(args.scene)

    with tempfile.TemporaryDirectory(prefix="sr-perf-") as workdir:
        frame_hashes, deterministic = hashes(args, workdir)
        if not deterministic:
            return 2
        current = measure(args, workdir)

    info = host_info(args.binary)
    # What must match for timings to be comparable (run count need not).
    settings = {"scene": os.path.relpath(args.scene, ROOT), "frames": args.frames,
                "threads": args.threads}
    if args.update:
        baseline = {"recorded": datetime.datetime.now().isoformat(timespec="seconds"),
                    **info, "settings": settings, "runs": args.runs,
                    "frame_sha256": frame_hashes, **current}
        with open(args.baseline, "w") as f:
            json.dump(baseline, f, indent=2)
            f.write("\n")
        print(f"baseline written: {args.baseline}")
        return 0

    try:
        with open(args.baseline) as f:
            baseline = json.load(f)
    except FileNotFoundError:
        print(f"no baseline at {args.baseline}; run with --update first")
        return 3
    if baseline.get("settings") != settings:
        print(f"baseline settings {baseline.get('settings')} differ from {settings}")
        return 3
    if (baseline.get("host"), baseline.get("cpu")) != (info["host"], info["cpu"]):
        print(f"WARNING: baseline from {baseline.get('host')} / {baseline.get('cpu')}; "
              f"this is {info['host']} / {info['cpu']}. Timings are not comparable.")

    status = 0
    changed = [f for f, h in frame_hashes.items()
               if baseline["frame_sha256"].get(f) != h]
    if changed:
        print(f"FAIL output changed for frames {changed}. If intended, "
              "refresh golden data and re-run with --update.")
        status = 2

    print(f"{'stage':10s} {'base':>8s} {'now':>8s} {'change':>8s}")
    rows = [(s, baseline["stages"][s], current["stages"][s]) for s in STAGES]
    rows.append(("TOTAL", baseline["total_cpu"], current["total_cpu"]))
    for name, base, now in rows:
        b, n = base["median"], now["median"]
        change = (n - b) / b if b else 0.0
        verdict = ""
        if b >= NOISE_FLOOR_S or name == "TOTAL":
            if change > args.tolerance:
                verdict = "REGRESSION"
                status = status or 1
            elif change < -args.tolerance:
                verdict = "faster"
            if spread(now) > args.tolerance:
                verdict += " (noisy)"
        else:
            verdict = "(below noise floor)"
        print(f"{name:10s} {b:8.3f} {n:8.3f} {change:+7.1%}  {verdict}")
    print("cpu seconds per run (engine process, all threads); "
          f"tolerance {args.tolerance:.0%}")
    print("PASS" if status == 0 else "FAIL")
    return status


if __name__ == "__main__":
    sys.exit(main())
