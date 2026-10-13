#!/usr/bin/env python3
"""Keeps the figures that documents quote about the schema, the corpus and the codes equal to their sources.

usage: tools/doc_facts.py --check [FILE.md ...]   exit 1 when a marked figure differs from its source
       tools/doc_facts.py --write [FILE.md ...]   rewrite every marked figure from its source
       tools/doc_facts.py --list                  print every fact and its value

A document marks a figure as <!-- facts:NAME -->VALUE<!-- /facts -->; the comments do not render. Without file
arguments, every tracked *.md and crates/**/*.rs outside vendor/ is processed. Facts come only from files in the repository, so the
check needs no build: the vendored schema release (schema/UPSTREAM), the document version (sr-model's
SCHEMA_VERSION), the Schematron, the conformance corpus manifest, the evaluator's codes, the workspace members and
the crate layers (tools/tests/test_crate_layers.py).
"""
import argparse
import importlib.util
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

    xsd = ET.parse(ROOT / "schema/scene-render-1.1.xsd").getroot()
    blend = [t for t in xsd.iter(XS + "simpleType") if t.get("name") == "blendType"]
    out["blend-modes"] = str(len(list(blend[0].iter(XS + "enumeration"))))

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
    out["crate-layers"] = crate_layers().render()
    return out


def crate_layers():
    """The module that states the crate layering once: tools/tests/test_crate_layers.py."""
    spec = importlib.util.spec_from_file_location("test_crate_layers", ROOT / "tools/tests/test_crate_layers.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


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


XS = "{http://www.w3.org/2001/XMLSchema}"
SKIPPED_MODULES = {"lib", "main", "mod", "bin"}


def layout_problems(readme, root=ROOT):
    """The crates, modules and example directories that the README's Layout block does not name.

    The block is a hand-written inventory of what the tree holds; an entry per crate (`crates/NAME/...`, continued on
    indented lines) names the modules of its `src/`, and `examples/NAME/` names an example directory."""
    start = readme.find("\n## Layout")
    if start < 0:
        return ["README has no Layout section"]
    block = readme[start:].split("\n```")[1]
    entries = {}
    current = None
    for line in block.splitlines():
        m = re.match(r"(?:crates/([A-Za-z0-9_-]+)\b|(examples)/|(\S))", line)
        if line.startswith(" ") or not line.strip():
            if current is not None:
                entries[current] = entries.get(current, "") + " " + line
            continue
        current = m.group(1) or ("examples" if m.group(2) else "other:" + line.split()[0])
        entries[current] = entries.get(current, "") + " " + line
    problems = []
    for crate in sorted(p.name for p in (root / "crates").iterdir() if (p / "Cargo.toml").exists()):
        text = entries.get(crate)
        if text is None:
            problems.append(f"Layout names no entry for crates/{crate}")
            continue
        src = root / "crates" / crate / "src"
        if "src/" not in text or not src.is_dir():
            continue  # an entry that describes the crate as a whole, such as the CLI
        # `three_{bind,depth}` stands for three_bind and three_depth
        expanded = re.sub(r"(\w+)\{([^}]*)\}", lambda m: " ".join(m.group(1) + x.strip() for x in m.group(2).split(",")), text)
        words = set(re.findall(r"[A-Za-z0-9_]+", text)) | set(re.findall(r"[A-Za-z0-9_]+", expanded))
        # the other direction: a snake_case name in the entry is a module, file or directory of the crate
        present = {p.stem for p in (root / "crates" / crate).rglob("*") if "target" not in p.parts}
        for name in sorted(set(re.findall(r"\b[a-z][a-z0-9]*(?:_[a-z0-9]+)+\b", expanded)) - present):
            problems.append(f"Layout entry of crates/{crate} names {name}, which is not in the crate")
        for item in sorted(src.iterdir()):
            name = item.stem if item.is_file() and item.suffix == ".rs" else item.name if item.is_dir() else None
            if name and name not in SKIPPED_MODULES and name not in words:
                problems.append(f"Layout entry of crates/{crate} does not name module {name}")
    examples = entries.get("examples", "")
    for d in sorted(p.name for p in (root / "examples").iterdir() if p.is_dir()):
        if f"examples/{d}/" not in examples:
            problems.append(f"Layout does not name examples/{d}/")
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
    if not args.files:
        for p in layout_problems((ROOT / "README.md").read_text()):
            failed = True
            print(f"README.md: {p}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
