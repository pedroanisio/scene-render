"""A test that stands in a fake tool writes the script with `sr_testkit::executable`, never with `std::fs::write`.

A test thread that writes a script and then runs it races every other test thread that forks a child: the child
inherits the still-open write descriptor, and exec fails with "Text file busy" (ETXTBSY, rust-lang/rust#114554).
`sr_testkit::executable` writes the script to a side file and has a separate process copy it into place, so the test
process never holds a writable descriptor on the file it runs. This test fails when a shebang script literal in a Rust
source is passed to anything but `executable`.
"""
import pathlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
SKIPPED_DIRS = {"target", "vendor", "_tmp", "node_modules"}

# A string literal, plain or raw, whose text starts with a shebang.
SHEBANG = re.compile(r'(?:\br#*)?"#!/')
CALLEE = re.compile(r"([A-Za-z_][A-Za-z_0-9]*!?)\s*$")
WRAPPERS = {"format!", "concat!"}


def callee(text, at):
    """The name of the call whose argument list holds position `at`, looking through `format!`; None outside a call."""
    i = at
    while True:
        depth = 0
        while i > 0:
            i -= 1
            c = text[i]
            if c in ")]}":
                depth += 1
            elif c in "([{":
                if depth == 0:
                    break
                depth -= 1
            elif c == ";" and depth == 0:
                return None
        else:
            return None
        if text[i] != "(":
            return None
        m = CALLEE.search(text[:i])
        if not m:
            return None
        if m.group(1) in WRAPPERS:
            continue
        return m.group(1)


def violations(text):
    """1-based line numbers of shebang literals in `text` that are not an argument of `executable`."""
    found = []
    for m in SHEBANG.finditer(text):
        line_start = text.rfind("\n", 0, m.start()) + 1
        if text[line_start:m.start()].lstrip().startswith("//"):
            continue
        if callee(text, m.start()) != "executable":
            found.append(text.count("\n", 0, m.start()) + 1)
    return found


def rust_sources():
    for path in sorted((ROOT / "crates").rglob("*.rs")):
        if not SKIPPED_DIRS.intersection(path.relative_to(ROOT).parts):
            yield path


class ExecutableFixtures(unittest.TestCase):
    def test_fake_tools_are_written_with_executable(self):
        bad = [
            f"{path.relative_to(ROOT)}:{line}"
            for path in rust_sources()
            for line in violations(path.read_text(encoding="utf-8"))
        ]
        self.assertEqual(bad, [], "write fake-tool scripts with sr_testkit::executable(&path, script)")

    def test_the_sweep_sees_fixtures(self):
        self.assertTrue(
            any(SHEBANG.search(p.read_text(encoding="utf-8")) for p in rust_sources()),
            "no shebang literal found: the sweep reads the wrong tree",
        )


class Detector(unittest.TestCase):
    def test_fs_write_of_a_script_is_flagged(self):
        self.assertEqual(violations('std::fs::write(&p, "#!/bin/sh\\nexit 0\\n").unwrap();'), [1])

    def test_raw_and_formatted_scripts_are_flagged(self):
        self.assertEqual(violations('fs::write(\n    &p,\n    r#"#!/usr/bin/env python3\nprint()\n"#,\n);'), [3])
        self.assertEqual(violations('std::fs::write(&tool, format!("#!/usr/bin/env python3\\n{s}\\n")).unwrap();'), [1])

    def test_a_script_held_outside_any_call_is_flagged(self):
        self.assertEqual(violations('let s = "#!/bin/sh\\n";\nstd::fs::write(&p, s).unwrap();'), [1])

    def test_executable_is_accepted(self):
        self.assertEqual(violations('sr_testkit::executable(&p, "#!/bin/sh\\nexit 0\\n");'), [])
        self.assertEqual(violations('sr_testkit::executable(&p, &format!("#!/bin/sh\\necho {t}\\n"));'), [])
        self.assertEqual(violations('executable(\n    &p,\n    r#"#!/usr/bin/env python3\n"#,\n);'), [])

    def test_a_nested_call_in_an_earlier_argument_does_not_hide_the_callee(self):
        self.assertEqual(violations('std::fs::write(dir.join(name("x")), "#!/bin/sh\\n").unwrap();'), [1])
        self.assertEqual(violations('executable(&dir.join(name("x")), "#!/bin/sh\\n");'), [])

    def test_comments_and_attributes_are_ignored(self):
        self.assertEqual(violations('// std::fs::write(&p, "#!/bin/sh")\n#![cfg(unix)]\n'), [])


if __name__ == "__main__":
    unittest.main()
