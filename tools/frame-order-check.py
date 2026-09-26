#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Check all golden frames sequentially, shuffled, sliced, and crash/resumed.

Run in the SDK with scene-render and scene-render-testhooks as arguments.
Lossless BGRA encodes are decoded to RGBA and checked against the same raw
frame hashes as --hash and preview, including alpha. No golden is skipped.
"""
import math
import os
import pathlib
import random
import re
import signal
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET

ROOT = pathlib.Path(__file__).resolve().parent.parent


def run(args, env=None, expect=None):
    result = subprocess.run([str(a) for a in args], cwd=ROOT, env=env,
                            capture_output=True, text=True, check=False)
    if re.search(r"\[FAIL\]|runtime error:|ERROR: AddressSanitizer",
                 result.stdout + result.stderr):
        raise RuntimeError(f"sanitizer/test failure: {args}\n"
                           f"{result.stdout}{result.stderr}")
    if expect == "abort":
        if result.returncode != -signal.SIGKILL:
            raise RuntimeError(f"expected hook SIGKILL, got {result.returncode}: "
                               f"{result.stderr}")
    elif result.returncode:
        raise RuntimeError(f"command failed: {args}\n{result.stdout}{result.stderr}")
    return result


def frame_hashes(text):
    return {int(frame): value for frame, value in
            re.findall(r"^(\d+) ([0-9a-f]{16})$", text, re.MULTILINE)}


def fnv(data):
    value = 14695981039346656037
    for byte in data:
        value = ((value ^ byte) * 1099511628211) & ((1 << 64) - 1)
    return f"{value:016x}"


def decoded_hashes(path, width, height, count):
    raw = path.with_suffix(".rgba")
    run(["ffmpeg", "-v", "error", "-y", "-i", path, "-map", "0:v:0",
         "-pix_fmt", "rgba", "-f", "rawvideo", raw])
    size = width * height * 4
    data = raw.read_bytes()
    if len(data) != count * size:
        raise RuntimeError(f"wrong decoded size: {path}: {len(data)}")
    return {i: fnv(data[i * size:(i + 1) * size]) for i in range(count)}


def check(binary, hooks, scene, work, threads):
    root = ET.parse(scene).getroot()
    project = root.find("project")
    fps = project.get("fps").split("/")
    rate = int(fps[0]) / (int(fps[1]) if len(fps) > 1 else 1)
    count = math.ceil(float(project.get("duration")) * rate)
    width, height = int(project.get("width")), int(project.get("height"))
    if count > 24 or width > 320 or height > 180:
        raise RuntimeError(f"golden exceeds batch limits: {scene}")
    # Scene copied to scratch: preserve the original asset/font base directory.
    for element in root.iter():
        for attribute in ("src", "fontFile"):
            value = element.get(attribute)
            if value:
                element.set(attribute, str((scene.parent / value).resolve()))
    for output in root.findall("output"):
        root.remove(output)
    output = ET.Element("output", path=str(work / "unused.mkv"),
                        codec="ffv1", pixelFormat="bgra")
    before_output = {"project", "metadata", "parameters", "styles", "colorManagement"}
    position = next((i for i, node in enumerate(root) if node.tag not in before_output),
                    len(root))
    root.insert(position, output)
    local = work / "scene.xml"
    ET.ElementTree(root).write(local, encoding="utf-8", xml_declaration=True)
    common = ["--scene", local, "--threads", threads]
    reference = frame_hashes(run([binary, *common, "--hash"]).stdout)
    if set(reference) != set(range(count)):
        raise RuntimeError(f"missing sequential frames: {scene}")
    frames = list(range(count))
    random.Random(110).shuffle(frames)
    for frame in frames:
        result = run([binary, *common, "--preview-frame", frame,
                      "--preview-out", work / "preview.png"])
        if result.stdout.strip().split()[-1] != reference[frame]:
            raise RuntimeError(f"shuffled preview mismatch: {scene}:{frame}")
    slices = [(start, min(start + 5, count)) for start in range(0, count, 5)]
    for start, end in reversed(slices):
        actual = frame_hashes(run([binary, *common, "--hash", "--frame-range",
                                  f"{start}:{end}"]).stdout)
        if actual != {i: reference[i] for i in range(start, end)}:
            raise RuntimeError(f"range mismatch: {scene}:{start}:{end}")
    full = work / "full.mkv"
    run([binary, *common, "--output", full])
    if decoded_hashes(full, width, height, count) != reference:
        raise RuntimeError(f"sequential encode mismatch: {scene}")
    resumed = work / "resumed.mkv"
    args = [*common, "--output", resumed, "--resume", "--keep-parts",
            "--segment-frames", 5]
    env = dict(os.environ, SR_TEST_ABORT_AFTER_SEGMENTS="1")
    run([hooks, *args], env=env, expect="abort")
    parts = pathlib.Path(str(resumed) + ".parts")
    if not (parts / "manifest").exists() or not list(parts.glob("seg-*.mkv")):
        raise RuntimeError(f"no committed segment after interruption: {scene}")
    result = run([binary, *args, "--metrics"])
    if not re.search(r"segments_reused=[1-9]", result.stderr):
        raise RuntimeError(f"no segment reused: {scene}\n{result.stderr}")
    if decoded_hashes(resumed, width, height, count) != reference:
        raise RuntimeError(f"resumed encode mismatch: {scene}")
    print(f"PASS frame order: {scene.name} ({count} frames, four paths, {threads} threads)", flush=True)
    return reference


def main():
    if len(sys.argv) != 3:
        sys.stderr.write("usage: frame-order-check.sh BINARY TESTHOOKS_BINARY\n")
        return 2
    binary, hooks = [pathlib.Path(p).resolve() for p in sys.argv[1:]]
    scenes = sorted((ROOT / "tests/golden").glob("*.xml"))
    if len(scenes) < 14:
        raise RuntimeError("golden inventory shrank below 14")
    for scene in scenes:
        single = None
        for threads in (1, 4):
            with tempfile.TemporaryDirectory(prefix="sr-order-") as temp:
                current = check(binary, hooks, scene, pathlib.Path(temp), threads)
                if single is not None and current != single:
                    raise RuntimeError(f"thread-count mismatch: {scene}")
                single = current
    print(f"PASS: frame order verified for all {len(scenes)} goldens")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, RuntimeError, ET.ParseError) as error:
        sys.stderr.write(str(error) + "\n")
        sys.exit(1)
