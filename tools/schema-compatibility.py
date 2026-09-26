#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Validate every 1.0-valid repository fixture against the 1.1 contract."""
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent


def validate(schema, scene):
    return subprocess.run(
        ["xmllint", "--noout", "--nonet", "--schema", str(schema), str(scene)],
        capture_output=True, text=True, check=False)


def main():
    old = ROOT / "schema/scene-v1.xsd"
    new = ROOT / "schema/scene-render-1.1.xsd"
    count = 0
    for folder in ("examples", "tests", "benchmarks"):
        for scene in sorted((ROOT / folder).rglob("*.xml")):
            if validate(old, scene).returncode:
                continue
            count += 1
            result = validate(new, scene)
            if result.returncode:
                sys.stderr.write(result.stderr)
                return 1
    if count < 42:
        sys.stderr.write(f"fixture inventory shrank: {count} < 42\n")
        return 1
    print(f"schema compatibility: {count}/{count} 1.0 fixtures pass 1.1")
    return 0


if __name__ == "__main__":
    sys.exit(main())
