"""The README documents the command line and the environment the code reads."""
import pathlib
import re
import subprocess
import tomllib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
MAIN = ROOT / "crates/scene-render/src/main.rs"

# long flags the README need not name, with the reason
UNDOCUMENTED_FLAGS = {
    "path": "the long form of -o on encode, documented as `-o PATH`",
}
# environment variables of test tools and example providers, not of the engine
TEST_ONLY_ENV = {
    "SR_EXAMPLE_LOG": "the example provider's log, for its tests",
    "SR_FUZZ_SCRATCH": "sr-fuzz's scratch directory",
}


def kebab(name):
    return name.replace("_", "-")


def long_flags(source):
    """`--flag` names of the clap `#[arg(...)]` attributes of main.rs, except hidden ones."""
    found = set()
    for m in re.finditer(r"#\[arg\(([^\]]*)\)\]\s*(?:///[^\n]*\n\s*)*(?:#\[[^\]]*\]\s*)*(?:pub\s+)?(\w+)\s*:", source):
        attrs, field = m.groups()
        if not re.search(r"\blong\b", attrs) or re.search(r"hide\s*=\s*true", attrs):
            continue
        named = re.search(r'long\s*=\s*"([^"]+)"', attrs)
        found.add(named.group(1) if named else kebab(field))
    return found


def subcommands(source):
    body = source[source.index("enum Command {"):]
    body = body[: body.index("\n}\n")]
    return {kebab(re.sub(r"(?<!^)(?=[A-Z])", "-", v).lower()) for v in re.findall(r"^    ([A-Z]\w+)\s*[{(,]", body, re.M)}


class CommandLineTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.readme = (ROOT / "README.md").read_text()
        cls.source = MAIN.read_text()

    def test_the_flag_parser_reads_the_attribute_forms(self):
        src = '''
            #[arg(long)]
            first: u64,
            #[arg(long = "param", value_name = "X")]
            params: Vec<String>,
            #[arg(long, hide = true)]
            max_runs: Option<u32>,
            #[arg(short = 'o', long = "path")]
            path: Option<String>,
        '''
        self.assertEqual(long_flags(src), {"first", "param", "path"})

    def test_every_long_flag_is_in_the_readme(self):
        flags = long_flags(self.source)
        self.assertGreater(len(flags), 40, "main.rs was not parsed")
        missing = sorted(f for f in flags if f"--{f}" not in self.readme and f not in UNDOCUMENTED_FLAGS)
        self.assertEqual(missing, [], "document these flags in README.md, or list them with a reason")

    def test_the_exemptions_are_still_flags(self):
        self.assertEqual(sorted(set(UNDOCUMENTED_FLAGS) - long_flags(self.source)), [])

    def test_every_subcommand_is_in_the_readme(self):
        names = subcommands(self.source)
        self.assertIn("bake-volume", names)
        missing = sorted(n for n in names if f"scene-render {n}" not in self.readme)
        self.assertEqual(missing, [])


class EnvironmentTests(unittest.TestCase):
    def test_every_environment_variable_the_engine_reads_is_documented(self):
        files = subprocess.run(["git", "ls-files", "crates/*/src/*.rs", "crates/*/src/**/*.rs"], cwd=ROOT, check=True,
                               capture_output=True, text=True).stdout.split()
        read = set()
        for f in files:
            text = (ROOT / f).read_text()
            read |= set(re.findall(r'env(?:::|\.)var(?:_os)?\(\s*"(SR_[A-Z0-9_]+)"', text))
            read |= set(re.findall(r'(?:env\s*=\s*|var\()"(SR_[A-Z0-9_]+)"', text))
        docs = "".join((ROOT / p).read_text() for p in ("README.md", "CONTRIBUTING.md", "crates/sr-resolve/README.md"))
        self.assertGreater(len(read), 15)
        missing = sorted(v for v in read if v not in docs and v not in TEST_ONLY_ENV)
        self.assertEqual(missing, [])


class FeatureDefaultTests(unittest.TestCase):
    def test_a_readme_does_not_call_an_opt_in_feature_default(self):
        manifest = tomllib.loads((ROOT / "crates/sr-resolve/Cargo.toml").read_text())
        self.assertNotIn("pdf", manifest["features"]["default"])
        text = (ROOT / "crates/sr-resolve/README.md").read_text()
        self.assertNotIn("on by default", text)
        self.assertRegex(text, r"\| `pdf` \| off \|")


if __name__ == "__main__":
    unittest.main()
