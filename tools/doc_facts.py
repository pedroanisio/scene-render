#!/usr/bin/env python3
"""Keeps the figures that documents quote about the schema, the corpus and the codes equal to their sources.

usage: tools/doc_facts.py --check [FILE.md ...]   exit 1 when a marked figure differs from its source
       tools/doc_facts.py --write [FILE.md ...]   rewrite every marked figure from its source
       tools/doc_facts.py --list                  print every fact and its value

A document marks a figure as <!-- facts:NAME -->VALUE<!-- /facts -->; the comments do not render. Without file
arguments, every tracked *.md and crates/**/*.rs outside vendor/ is processed. Facts come only from files in the repository, so the
check needs no build: the vendored schema release (schema/UPSTREAM), the document version (sr-model's
SCHEMA_VERSION), the Schematron, the conformance corpus manifest, the evaluator's codes and the workspace members.
"""
import argparse
import json
import re
import subprocess
import sys
import tomllib
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCH = "{http://purl.oclc.org/dsdl/schematron}"
MARK = re.compile(r"<!-- facts:([a-z0-9-]+) -->(.*?)<!-- /facts -->", re.S)


def facts():
    out = {}
    upstream = (ROOT / "schema/UPSTREAM").read_text()
    out["schema-upstream"] = re.search(r"sr-core release (\d+\.\d+\.\d+)", upstream).group(1)
    lib = (ROOT / "crates/sr-model/src/lib.rs").read_text()
    out["schema-version"] = re.search(r'pub const SCHEMA_VERSION: &str = "([^"]+)"', lib).group(1)

    sch = ET.parse(ROOT / "schema/scene-render-1.1.sch").getroot()
    asserts = list(sch.iter(SCH + "assert")) + list(sch.iter(SCH + "report"))
    out["sch-asserts"] = str(len(asserts))
    out["sch-patterns"] = str(len(list(sch.iter(SCH + "pattern"))))
    families = {}
    for a in asserts:
        m = re.fullmatch(r"([A-Z]+)(\d+)", a.get("id", ""))
        if m:
            families[m.group(1)] = max(families.get(m.group(1), 0), int(m.group(2)))
    for fam in ("V", "C", "R"):
        out[f"rule-{fam.lower()}-max"] = f"{fam}{families[fam]}"
    out["rule-families"] = str(len(families))

    corpus = json.loads((ROOT / "tests/corpus/manifest.json").read_text())
    for kind in ("valid", "invalid", "warnings"):
        out[f"corpus-{kind}"] = str(len(corpus[kind]))
    out["corpus-total"] = str(sum(len(corpus[k]) for k in ("valid", "invalid", "warnings")))

    codes = re.findall(r'"E(\d{2})"', (ROOT / "crates/sr-eval/src/codes.rs").read_text())
    out["eval-code-max"] = f"E{max(int(c) for c in codes):02d}"

    ci = (ROOT / ".github/workflows/ci.yml").read_text()
    out["fuzz-iterations"] = re.search(r"sr-fuzz --iterations (\d+) --partition", ci).group(1)
    matrix = re.search(r"^\s+shard: \[([0-9, ]+)\]", ci, re.M).group(1)
    out["ci-shards"] = str(len(matrix.split(",")))

    members = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["members"]
    out["crates"] = str(len(members))
    return out


def tracked_docs():
    names = subprocess.run(["git", "ls-files", "*.md", "crates/**/*.rs"], cwd=ROOT, check=True, capture_output=True, text=True).stdout
    # a tracked file deleted from the working tree but not yet staged is still listed by git ls-files
    return [ROOT / p for p in names.split() if not p.startswith("vendor/") and (ROOT / p).exists()]


def process(path, values, write):
    """Returns the problems in one document; with write, fixes them in place."""
    text = path.read_text()
    problems = []

    def fix(m):
        name, current = m.group(1), m.group(2)
        if name not in values:
            problems.append(f"unknown fact {name!r}")
            return m.group(0)
        if current != values[name]:
            problems.append(f"{name} is {current!r}, its source says {values[name]!r}")
        return f"<!-- facts:{name} -->{values[name]}<!-- /facts -->"

    new = MARK.sub(fix, text)
    if write and new != text:
        path.write_text(new)
    return problems


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--write", action="store_true")
    mode.add_argument("--list", action="store_true")
    parser.add_argument("files", nargs="*")
    args = parser.parse_args(argv)
    values = facts()
    if args.list:
        for k, v in sorted(values.items()):
            print(f"{k} = {v}")
        return 0
    failed = False
    for f in [Path(a).resolve() for a in args.files] or tracked_docs():
        for p in process(f, values, args.write):
            unknown = p.startswith("unknown")
            failed |= args.check or unknown
            print(f"{f.relative_to(ROOT)}: {p}" + ("" if args.check or unknown else " (rewritten)"))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
