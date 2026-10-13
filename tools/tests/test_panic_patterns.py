"""Production code neither propagates a poisoned lock nor panics when it orders a NaN.

`lock().unwrap()` turns one panic while a lock is held into a panic in every later caller; `partial_cmp(..).unwrap()`
panics on the first NaN a parameter or a decoded file brings. Code under `#[cfg(test)]` may do either.
"""
import pathlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]

# Sites not fixed yet, as (file, the source line stripped) and the reason. The list may only shrink: an entry that
# excuses nothing fails the test, so a fixed site is removed here in the same change.
ALLOWED = {}

LOCK = re.compile(r"\.(?:lock|read|write|get_mut)\(\)\s*\.(?:unwrap\(\)|expect\()")
PARTIAL_CMP = re.compile(r"\bpartial_cmp\(")
UNWRAPPED = re.compile(r"\s*\.(?:unwrap\(\)|expect\()")
TEST_ONLY = re.compile(r"#\[cfg\((?:all\()?test\b")
ITEM = re.compile(
    r"(?:pub(?:\([^)]*\))?\s+)?(?:(?:async|unsafe|const|extern)\s+)*"
    r"(?:fn|mod|impl|struct|enum|union|trait|const|static|type|use|macro_rules!|thread_local!)\W"
)
RAW_STRING = re.compile(r"b?r(#*)\"")
LITERAL_OR_COMMENT = re.compile(r"/[/*]|[\"'br]")
CHAR = re.compile(r"'(?:\\[^']{1,10}|[^\\'])'")
ATTRIBUTE = re.compile(r"\s*#\[")
SPACE = re.compile(r"\s*")
OPEN, CLOSE = "([{", ")]}"


def blank(text, start, end):
    """`text` with [start, end) replaced by spaces, newlines kept so that line numbers hold."""
    return text[:start] + re.sub(r"[^\n]", " ", text[start:end]) + text[end:]


def code_only(text):
    """Rust source with comments and string and character literals blanked."""
    out, i, n = list(text), 0, len(text)

    def wipe(a, b):
        out[a:b] = ["\n" if ch == "\n" else " " for ch in text[a:b]]

    while (start := LITERAL_OR_COMMENT.search(text, i)) is not None:
        i = start.start()
        c = text[i]
        if text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            wipe(i, j)
            i = j
        elif text.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if text.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif text.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            wipe(i, j)
            i = j
        elif (m := RAW_STRING.match(text, i)) and not (text[i - 1 : i].isalnum() or text[i - 1 : i] == "_"):
            close = '"' + m.group(1)
            j = text.find(close, m.end())
            j = n if j < 0 else j + len(close)
            wipe(i, j)
            i = j
        elif c == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            wipe(i, j + 1)
            i = j + 1
        elif c == "'" and (m := CHAR.match(text, i)):
            wipe(i, m.end())
            i = m.end()
        else:
            i += 1
    return "".join(out)


def item_end(code, i):
    """Where the item or statement that starts at `i` ends, in comment- and literal-free `code`."""
    is_item = ITEM.match(code, i) is not None
    depth, j = 0, i
    while j < len(code):
        c = code[j]
        if c in OPEN:
            depth += 1
        elif c in CLOSE:
            depth -= 1
            if depth < 0:
                return j
            if depth == 0 and c == "}" and (is_item or code[i] == "{"):
                return j + 1
        elif depth == 0 and (c == ";" or (c == "," and not is_item)):
            return j + 1
        j += 1
    return j


def test_only_file_modules(path, code):
    """Module files that a `#[cfg(test)] mod name;` in `path` brings in: everything under them is test code."""
    here = path.parent if path.name in ("lib.rs", "main.rs", "mod.rs") else path.parent / path.stem
    for m in re.finditer(r"#\[cfg\((?:all\()?test\b[^\]]*\]\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;", code):
        yield here / f"{m.group(1)}.rs"
        yield here / m.group(1)


def without_test_code(code):
    """`code` with every item, field or statement under `#[cfg(test)]` blanked."""
    at = 0
    while m := TEST_ONLY.search(code, at):
        j = m.start()
        # the attribute and any after it belong to the item
        while (a := ATTRIBUTE.match(code, j)) is not None:
            depth, j = 0, a.end() - 1
            while j < len(code):
                depth += {"[": 1, "]": -1}.get(code[j], 0)
                j += 1
                if depth == 0:
                    break
        j = SPACE.match(code, j).end()
        end = item_end(code, j)
        code = blank(code, m.start(), end)
        at = end
    return code


def findings(text):
    """(line number, kind) of every panicking lock or float order outside test code in Rust source `text`."""
    code = without_test_code(code_only(text))
    found = [(m.start(), "lock") for m in LOCK.finditer(code)]
    for m in PARTIAL_CMP.finditer(code):
        depth, j = 1, m.end()
        while j < len(code) and depth:
            depth += {"(": 1, ")": -1}.get(code[j], 0)
            j += 1
        if UNWRAPPED.match(code, j):
            found.append((m.start(), "partial_cmp"))
    return sorted((code.count("\n", 0, at) + 1, kind) for at, kind in found)


def production_sources():
    files = sorted(ROOT.glob("crates/*/src/**/*.rs"))
    skipped = set()
    for path in files:
        skipped.update(test_only_file_modules(path, code_only(path.read_text())))
    return [p for p in files if not any(p == s or s in p.parents for s in skipped)]


class PanicPatternTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.sites = {}
        for path in production_sources():
            text = path.read_text()
            lines = text.splitlines()
            for line, kind in findings(text):
                key = (path.relative_to(ROOT).as_posix(), lines[line - 1].strip())
                cls.sites.setdefault(key, []).append(f"{key[0]}:{line} ({kind})")

    def test_no_production_code_unwraps_a_lock_or_a_partial_order(self):
        new = sorted(where for key, at in self.sites.items() if key not in ALLOWED for where in at)
        self.assertEqual(new, [], "recover the lock with `unwrap_or_else(PoisonError::into_inner)`, or order with "
                                  "`total_cmp` (or handle the NaN), instead of unwrapping")

    def test_every_allowed_site_is_still_there(self):
        self.assertEqual(sorted(set(ALLOWED) - set(self.sites)), [], "remove these from ALLOWED: they are fixed")

    def test_the_scanner_sees_through_test_code_comments_and_literals(self):
        source = "\n".join([
            "fn a(m: &Mutex<u8>) { *m.lock().unwrap() }",                                     # 1: found
            "// m.lock().unwrap()",
            'const S: &str = "x.partial_cmp(y).unwrap()";',
            'const R: &str = r#"m.read().unwrap()"#;',
            "fn b(v: &mut [f64]) { v.sort_by(|a, b| a.partial_cmp(&(b * 2.0)).unwrap()); }",  # 5: found
            "fn c(v: &mut [f64]) { v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Equal)); }",
            "#[cfg(test)]",
            "fn d<A, B>(m: &RwLock<u8>) -> u8 { let c = '{'; *m.write().unwrap() }",
            "struct S {",
            "    #[cfg(test)]",
            "    f: bool,",
            "}",
            "#[cfg(all(test, target_os = \"linux\"))]",
            "mod tests {",
            "    #[test]",
            "    fn e() { m.lock().unwrap(); }",
            "}",
            "fn f<'a>(m: &'a RwLock<u8>) -> u8 { *m",
            "    .read()",                                                                       # 19: found
            "    .expect(\"poisoned\") }",
        ])
        self.assertEqual(findings(source), [(1, "lock"), (5, "partial_cmp"), (19, "lock")])

    def test_test_only_module_files_are_skipped(self):
        root = ROOT / "crates/x/src"
        code = "#[cfg(test)]\nmod checks;\nmod real;\n"
        self.assertEqual(list(test_only_file_modules(root / "lib.rs", code)), [root / "checks.rs", root / "checks"])
        self.assertEqual(list(test_only_file_modules(root / "pyro.rs", code))[0], root / "pyro" / "checks.rs")


if __name__ == "__main__":
    unittest.main()
