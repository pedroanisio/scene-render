#!/usr/bin/env python3
"""Render both deliverables of "The Archive Beacon" with a measured end card.

The end card prints each version's render time, which only exists after
rendering. The pipeline therefore:

  1. builds the scene with PENDING render times and renders the full UHD
     viewport and 360 equirectangular passes in parallel (--metrics);
  2. rebuilds the XML with each pass's measured wall time;
  3. re-renders only frames [K, 900), where K is the last pass-1 keyframe at
     or before the end card (frame 810, t=27 s). The end card group starts at
     27 s, so frames before it are identical under both XML versions;
  4. stream-copies pass-1 frames [0, K) + the new tail and keeps pass 1's
     continuous audio track (audio does not depend on the card text).

Requires ffmpeg on PATH and a built ./build/scene-render. Pass --skip-pass1 to
reuse existing build/pass1 outputs and their logs. build_scene.py needs
fontTools, so run this with a Python that has it installed.
"""
import argparse
import os
import re
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
ENGINE = os.path.join(ROOT, "build", "scene-render")
SCENE = os.path.join(ROOT, "examples", "archive-beacon.xml")
BUILD = os.path.join(ROOT, "build")
PASS1 = os.path.join(BUILD, "pass1")
FPS = 30
CARD_FRAME = 810
TOTAL = 900
VERSIONS = {"uhd": [], "360": ["--mode", "equirectangular"]}


def run(cmd, log=None):
    print("+", " ".join(cmd), flush=True)
    if log:
        with open(log, "w") as f:
            subprocess.run(cmd, check=True, stdout=f, stderr=subprocess.STDOUT)
    else:
        subprocess.run(cmd, check=True)


def build_scene(uhd="PENDING", pano="PENDING"):
    run([sys.executable, os.path.join(HERE, "build_scene.py"),
         "--render-time-uhd", uhd, "--render-time-360", pano])


def render(version, out, log, frame_range=None):
    cmd = [ENGINE, "--scene", SCENE, "--output", out, "--threads", "auto",
           "--metrics", *VERSIONS[version]]
    if frame_range:
        cmd += ["--frame-range", f"{frame_range[0]}:{frame_range[1]}"]
    run(cmd, log)


def wall_seconds(log):
    with open(log, errors="replace") as f:
        match = re.findall(r"metrics: frames=(\d+) .*?wall_s=([\d.]+)", f.read())
    if not match or int(match[-1][0]) != TOTAL:
        raise SystemExit(f"{log}: no complete {TOTAL}-frame metrics line")
    return float(match[-1][1])


def spoken(seconds):
    seconds = int(round(seconds))
    minutes, rest = divmod(seconds, 60)
    return f"{minutes} min {rest} s" if minutes else f"{rest} s"


def last_keyframe(path, limit):
    probe = subprocess.run(
        ["ffmpeg", "-nostdin", "-skip_frame", "nokey", "-i", path,
         "-vf", "showinfo", "-f", "null", "-"],
        capture_output=True, text=True, check=True).stderr
    frames = [round(float(t) * FPS) for t in re.findall(r"pts_time:([\d.]+)", probe)]
    usable = [f for f in frames if 0 < f <= limit]
    if not usable:
        raise SystemExit(f"{path}: no keyframe in (0, {limit}]")
    return max(usable)


def count_frames(path):
    out = subprocess.run(["ffmpeg", "-nostdin", "-i", path, "-map", "0:v",
                          "-f", "null", "-"], capture_output=True, text=True,
                         check=True).stderr
    return int(re.findall(r"frame=\s*(\d+)", out)[-1])


def spatial_injector():
    """Build the helper that re-applies the engine's spherical metadata."""
    tool = os.path.join(BUILD, "sr-spatial-inject")
    objects = [os.path.join(BUILD, f"{name}.o")
               for name in ("spatial", "diagnostics", "common")]
    run([os.environ.get("CC", "cc"), "-std=c17", "-O2",
         "-D_POSIX_C_SOURCE=200809L", "-I", os.path.join(ROOT, "include"),
         os.path.join(HERE, "spatial_inject.c"), *objects, "-lm", "-pthread",
         "-o", tool])
    return tool


def splice(version, key):
    full = os.path.join(PASS1, f"archive-beacon-{version}.mp4")
    tail = os.path.join(PASS1, f"archive-beacon-{version}.tail.mp4")
    head = os.path.join(PASS1, f"archive-beacon-{version}.head.mp4")
    joined = os.path.join(PASS1, f"archive-beacon-{version}.video.mp4")
    final = os.path.join(BUILD, f"archive-beacon-{version}.mp4")
    run(["ffmpeg", "-nostdin", "-v", "error", "-y", "-i", full, "-map", "0:v",
         "-c", "copy", "-frames:v", str(key), head])
    listing = os.path.join(PASS1, f"concat-{version}.txt")
    with open(listing, "w") as f:
        f.write(f"file '{head}'\nfile '{tail}'\n")
    run(["ffmpeg", "-nostdin", "-v", "error", "-y", "-f", "concat", "-safe",
         "0", "-i", listing, "-map", "0:v", "-c", "copy", joined])
    # No +faststart: the spatial injector patches a trailing moov in place.
    run(["ffmpeg", "-nostdin", "-v", "error", "-y", "-i", joined, "-i", full,
         "-map", "0:v", "-map", "1:a", "-c", "copy", final])
    if version == "360":
        run([spatial_injector(), final, "3840", "1920"])
    frames = count_frames(final)
    if frames != TOTAL:
        raise SystemExit(f"{final}: {frames} frames, expected {TOTAL}")
    return final


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--skip-pass1", action="store_true")
    args = parser.parse_args()
    os.makedirs(PASS1, exist_ok=True)
    logs = {v: os.path.join(PASS1, f"{v}.log") for v in VERSIONS}
    if not args.skip_pass1:
        build_scene()
        with ThreadPoolExecutor(len(VERSIONS)) as pool:
            jobs = [pool.submit(render, v,
                                os.path.join(PASS1, f"archive-beacon-{v}.mp4"),
                                logs[v]) for v in VERSIONS]
            for job in jobs:
                job.result()
    times = {v: wall_seconds(logs[v]) for v in VERSIONS}
    build_scene(spoken(times["uhd"]), spoken(times["360"]))
    keys = {v: last_keyframe(os.path.join(PASS1, f"archive-beacon-{v}.mp4"),
                             CARD_FRAME) for v in VERSIONS}
    with ThreadPoolExecutor(len(VERSIONS)) as pool:
        jobs = [pool.submit(render, v,
                            os.path.join(PASS1, f"archive-beacon-{v}.tail.mp4"),
                            os.path.join(PASS1, f"{v}.tail.log"),
                            (keys[v], TOTAL)) for v in VERSIONS]
        for job in jobs:
            job.result()
    for v in VERSIONS:
        final = splice(v, keys[v])
        print(f"{final}: pass-1 wall {times[v]:.1f}s, tail from frame {keys[v]}")


if __name__ == "__main__":
    main()
