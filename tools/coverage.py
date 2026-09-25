#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Line and branch coverage of src/ for a build instrumented with --coverage.

Usage: tools/coverage.py BUILD_DIR [--fail-under-lines P] [--fail-under-branches P]

Configure with -DSR_COVERAGE=ON (or `make SR_COVERAGE=1 BUILD=...`), build,
run the tests (ctest or `make test`), then run this script on the build
directory. It runs gcov on every object directory holding .gcda files (no
gcovr needed), keeps the source files under src/, prints one row per file
(lowest branch coverage first) and a TOTAL row, and exits 1 when a total is
below the given threshold. gcov's own output files go to BUILD_DIR/gcov/.

"Branches" are gcov's "taken at least once" branch outcomes. A file compiled
into several targets is reported once (the last object directory wins: the
same source, the same counts). Headers are not counted.
"""
import argparse
import pathlib
import re
import subprocess
import sys

FILE_RE = re.compile(
    r"File '([^']+)'\nLines executed:([\d.]+)% of (\d+)\n"
    r"(?:Branches executed:[\d.]+% of \d+\nTaken at least once:([\d.]+)% of (\d+)\n"
    r"|No branches\n)?")


def gcov_report(build: pathlib.Path) -> str:
    out = build / "gcov"
    out.mkdir(exist_ok=True)
    dirs = sorted({p.parent for p in build.rglob("*.gcda") if "gcov" not in p.parts})
    report = ""
    for d in dirs:
        gcda = sorted(str(p) for p in d.glob("*.gcda"))
        res = subprocess.run(["gcov", "-b", "-o", str(d), *gcda], cwd=out,
                             capture_output=True, text=True, check=True)
        report += res.stdout
    return report


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("build_dir", type=pathlib.Path)
    ap.add_argument("--fail-under-lines", type=float, default=0.0)
    ap.add_argument("--fail-under-branches", type=float, default=0.0)
    args = ap.parse_args()
    build = args.build_dir.resolve()
    repo = pathlib.Path(__file__).resolve().parent.parent
    src_root = str(repo / "src") + "/"
    report = gcov_report(build)
    if not report:
        print("no .gcda files: build with -DSR_COVERAGE=ON and run the tests first",
              file=sys.stderr)
        return 2
    files = {}
    for m in FILE_RE.finditer(report):
        source = pathlib.Path(m.group(1))
        if not source.is_absolute():   # the Makefile compiles src/x.c from the root
            source = repo / source
        path = str(source.resolve())
        if not path.startswith(src_root) or not path.endswith(".c"):
            continue
        lines = (float(m.group(2)), int(m.group(3)))
        branches = (float(m.group(4)), int(m.group(5))) if m.group(4) else (100.0, 0)
        files[path[len(src_root):]] = (lines, branches)
    if not files:
        print("gcov reported no file under src/", file=sys.stderr)
        return 2
    lhit = ltotal = btaken = btotal = 0
    width = max(len(name) for name in files)
    for name, ((lp, ln), (bp, bn)) in sorted(files.items(), key=lambda kv: kv[1][1][0]):
        print(f"{name:{width}s}  lines {lp:6.2f}% of {ln:5d}  branches {bp:6.2f}% of {bn:5d}")
        lhit += round(lp * ln / 100)
        ltotal += ln
        btaken += round(bp * bn / 100)
        btotal += bn
    line_pct = 100.0 * lhit / ltotal if ltotal else 100.0
    branch_pct = 100.0 * btaken / btotal if btotal else 100.0
    print(f"TOTAL lines {lhit}/{ltotal} = {line_pct:.2f}%  "
          f"branches {btaken}/{btotal} = {branch_pct:.2f}%  ({len(files)} files)")
    status = 0
    if line_pct < args.fail_under_lines:
        print(f"line coverage {line_pct:.2f}% is below {args.fail_under_lines:.2f}%",
              file=sys.stderr)
        status = 1
    if branch_pct < args.fail_under_branches:
        print(f"branch coverage {branch_pct:.2f}% is below {args.fail_under_branches:.2f}%",
              file=sys.stderr)
        status = 1
    return status


if __name__ == "__main__":
    sys.exit(main())
