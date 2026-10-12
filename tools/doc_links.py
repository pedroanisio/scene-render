#!/usr/bin/env python3
"""Checks that the relative links and repository paths written in Markdown documents exist.

usage: tools/doc_links.py [--exclude FILE.md ...] [FILE.md ...]   default: every tracked *.md outside vendor/

A relative link is the target of `[text](target)` that is not a URL or an anchor. A repository path is a code span
that starts with a top-level directory of this repository (`crates/`, `tools/`, `tests/`, `schema/`, `examples/`,
`vendor/`, `.github/`) and holds no placeholder (`<…>`, `{…}`, `*`, `%`). Exit status 1 lists every one that does not
exist.
"""
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LINK = re.compile(r"\]\(([^)\s]+)\)")
SPAN = re.compile(r"`((?:crates|tools|tests|schema|examples|vendor|\.github)/[^`\s]+)`")
PLACEHOLDER = re.compile(r"[<>{}*%]|\.\.\.|…")

# Paths that are written on purpose and do not exist here, with the reason.
ALLOW = {
    "schema/scene-render.xsd": "the canonical file name in sr-core, which this repository vendors under another name",
    "schema/scene-render.sch": "the canonical file name in sr-core",
}


def tracked_docs():
    out = subprocess.run(["git", "ls-files", "*.md"], cwd=ROOT, check=True, capture_output=True, text=True).stdout
    # a tracked file deleted from the working tree but not yet staged is still listed by git ls-files
    return [ROOT / p for p in out.split() if (not p.startswith("vendor/") or p == "vendor/README.md") and (ROOT / p).exists()]


def problems(path):
    text = path.read_text()
    found = []
    for n, line in enumerate(text.splitlines(), 1):
        for target in LINK.findall(line):
            target = target.split("#", 1)[0]
            if not target or re.match(r"^[a-z][a-z0-9+.-]*:", target):
                continue
            if not (path.parent / target).exists():
                found.append((n, "link", target))
        for span in SPAN.findall(line):
            span = span.rstrip(".,;:")
            if PLACEHOLDER.search(span) or span in ALLOW:
                continue
            if not (ROOT / span).exists():
                found.append((n, "path", span))
    return found


def main(argv):
    exclude = set()
    while "--exclude" in argv:
        i = argv.index("--exclude")
        exclude.add((ROOT / argv[i + 1]).resolve())
        argv = argv[:i] + argv[i + 2:]
    files = [f for f in ([Path(a).resolve() for a in argv] or tracked_docs()) if f.resolve() not in exclude]
    bad = 0
    for f in files:
        for n, kind, target in problems(f):
            print(f"{f.relative_to(ROOT)}:{n}: {kind} {target} does not exist")
            bad += 1
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
