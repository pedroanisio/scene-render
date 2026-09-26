#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Compare preview and container bytes with a pre-change binary in the SDK.

Usage: equivalence-oracle.sh REF_BINARY NEW_BINARY [SCENE_LIST]
SCENE_LIST is a newline-separated list of repository-relative scene paths.
The default covers every 1.0-valid repository fixture, plus time samples
from the original performance oracle. Failed renders always fail the check.
"""
import hashlib
import pathlib
import re
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET

ROOT = pathlib.Path(__file__).resolve().parent.parent
THREADS = (1, 3, 22)
# XSD-valid fixtures deliberately rejected before rendering. They are part
# of compatibility coverage, but cannot produce a reference image.
NEGATIVE = {
    "tests/cli/missing-asset.xml": (4, "does-not-exist.ppm"),
    "tests/data-doctype.xml": (3, "document type declarations are not allowed"),
}
SAMPLES = {
    "benchmarks/perf-scene.xml": [0, 15, 29],
    "examples/archive-beacon.xml": [60, 300, 420, 560, 735, 870],
    "examples/v1.1-feature-showcase.xml": [30, 150, 270],
    "examples/lighting-physics-deformation.xml": [45, 150],
    "examples/production-features.xml": [0, 45, 89],
    "examples/feature-parity.xml": [0, 30, 60],
    "examples/dusk-parallax.xml": [0, 90],
    "examples/text-layout.xml": [0],
    "examples/video-audio-remap.xml": [10, 150, 290],
    "examples/viewport-360.xml": [0, 150],
    "examples/equirectangular-360.xml": [100],
    "examples/ten-layer-composition.xml": [120],
    "examples/keyframe-curves.xml": [0, 60],
    "examples/basic-multilayer.xml": [100],
    "examples/gravity-well.xml": [200],
    "examples/dusk-depth.xml": [0, 60, 150],
    "tests/data-depth.xml": [0, 5],
}
ENCODES = [
    ("benchmarks/perf-scene.xml", "0:30", "mkv"),
    ("examples/video-audio-remap.xml", "0:60", "mp4"),
    ("examples/production-features.xml", "0:90", "mp4"),
]


def run(args, expected=0):
    result = subprocess.run([str(a) for a in args], cwd=ROOT,
                            capture_output=True, text=True, check=False)
    if re.search(r"\[FAIL\]|runtime error:|ERROR: AddressSanitizer",
                 result.stdout + result.stderr):
        raise RuntimeError(f"sanitizer/test failure: {args}\n"
                           f"{result.stdout}{result.stderr}")
    if result.returncode != expected:
        raise RuntimeError(f"command failed ({result.returncode}): {args}\n"
                           f"{result.stdout}{result.stderr}")
    return result.stdout if not expected else result.stderr


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def fixtures():
    for folder in ("examples", "tests", "benchmarks"):
        for scene in sorted((ROOT / folder).rglob("*.xml")):
            result = subprocess.run(
                ["xmllint", "--noout", "--nonet", "--schema",
                 str(ROOT / "schema/scene-v1.xsd"), str(scene)],
                capture_output=True, check=False)
            if result.returncode == 0:
                yield scene.relative_to(ROOT).as_posix()


def main():
    if len(sys.argv) not in (3, 4):
        sys.stderr.write(__doc__ + "\n")
        return 2
    binaries = [pathlib.Path(p).resolve() for p in sys.argv[1:3]]
    if len(sys.argv) == 4:
        scenes = [s.strip() for s in pathlib.Path(sys.argv[3]).read_text().splitlines()
                  if s.strip() and not s.lstrip().startswith("#")]
    else:
        scenes = list(fixtures())
        if len(scenes) < 42:
            raise RuntimeError(f"fixture inventory shrank: {len(scenes)} < 42")
    if not scenes:
        raise RuntimeError("empty scene list")
    previews = encodes = rejected = 0
    with tempfile.TemporaryDirectory(prefix="sr-oracle-") as temp:
        work = pathlib.Path(temp)
        for scene in scenes:
            if scene in NEGATIVE:
                code, diagnostic = NEGATIVE[scene]
                messages = [run([binary, "--scene", ROOT / scene, "--hash"],
                                expected=code) for binary in binaries]
                if messages[0] != messages[1] or diagnostic not in messages[0]:
                    raise RuntimeError(f"negative-fixture mismatch: {scene}: {messages}")
                rejected += 1
                print(f"PASS {scene} (expected rejection {code})", flush=True)
                continue
            root = ET.parse(ROOT / scene).getroot()
            project = root.find("project")
            fps = project.get("fps").split("/")
            rate = int(fps[0]) / (int(fps[1]) if len(fps) > 1 else 1)
            last = max(0, int(float(project.get("duration")) * rate) - 1)
            frames = SAMPLES.get(scene, sorted({0, last // 2, last}))
            for frame in frames:
                hashes = []
                for binary in binaries:
                    for threads in THREADS:
                        out = work / "frame.png"
                        run([binary, "--scene", ROOT / scene, "--preview-frame",
                             frame, "--preview-out", out, "--threads", threads])
                        hashes.append(digest(out))
                if len(set(hashes)) != 1:
                    raise RuntimeError(f"preview mismatch: {scene} frame {frame}: {hashes}")
                previews += len(THREADS)
            print(f"PASS {scene} ({len(frames)} frames x {len(THREADS)} threads)",
                  flush=True)
        for scene, frame_range, extension in ENCODES:
            if len(sys.argv) == 4 and scene not in scenes:
                continue
            hashes = []
            for binary in binaries:
                out = work / f"encode.{extension}"
                run([binary, "--scene", ROOT / scene, "--frame-range", frame_range,
                     "--output", out, "--threads", 4])
                hashes.append(digest(out))
            if hashes[0] != hashes[1]:
                raise RuntimeError(f"encode mismatch: {scene}: {hashes}")
            encodes += 1
    print(f"PASS: {previews}/{previews} previews across {len(scenes)} scenes; "
          f"{encodes}/{encodes} encodes match; {rejected} expected rejections match")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, RuntimeError, ET.ParseError) as error:
        sys.stderr.write(str(error) + "\n")
        sys.exit(1)
