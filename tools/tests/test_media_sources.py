"""SOURCES.json says where every media file and test shader in the tree comes from, and what it says is true of the files.

Every tracked file with a media extension, every shader under a `tests/` directory and every file beside a NOTICE or
SOURCES.md has one entry, with its SHA-256 and size. An entry's origin decides what else it must say, and the checks
below hold it to that: a generated file names the in-tree generator that writes it, a third-party file names its source,
a licence that allows redistribution and the licence text in the tree, a copy names the tracked file it is byte-equal
to, and a file whose origin is not known says so and describes the bytes.
"""
import hashlib
import json
import pathlib
import re
import shutil
import subprocess
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
MANIFEST = "SOURCES.json"
NOTICES = "THIRD-PARTY-NOTICES.md"

MEDIA = {
    # images and colour
    ".png", ".jpg", ".jpeg", ".gif", ".webp", ".tif", ".tiff", ".tga", ".qoi", ".psd", ".ppm", ".bmp", ".avif", ".jxl",
    ".jp2", ".dds", ".ff", ".exr", ".hdr", ".heic", ".icc", ".svg", ".ico", ".cube", ".spi1d", ".ocio",
    # fonts
    ".ttf", ".otf", ".woff", ".woff2", ".jhf",
    # meshes, scenes and voxels
    ".glb", ".gltf", ".obj", ".fbx", ".usda", ".usdc", ".usdz", ".vox",
    # volumes
    ".vdb", ".srvol", ".srvseq",
    # maps
    ".pmtiles", ".geojson", ".kml", ".gpx",
    # audio, video and captions
    ".wav", ".mp3", ".flac", ".mp4", ".mov", ".webm", ".vtt", ".srt",
    # programs
    ".wasm",
}
TEST_SHADERS = {".glsl", ".fs", ".wgsl"}
RECORDS = {"NOTICE", "SOURCES.md"}
LICENCE_NAME = re.compile(r"(^|_)LICEN[SC]E")
OUTSIDE = ("vendor/",)

COMMON = {"origin", "sha256", "bytes"}
FIELDS = {
    "generated": ({"generator"}, {"record", "checked_by", "note"}),
    "third-party": ({"source", "licence", "licence_file"}, {"notice", "in_binary", "note"}),
    "upstream": ({"source", "upstream_path"}, {"note"}),
    "copy": ({"of"}, {"note"}),
    "authored": ({"evidence"}, set()),
    "unknown": ({"note"}, set()),
}
NOT_REDISTRIBUTABLE = re.compile(r"(?i)(^|[-\s])(NC|ND)([-\s.]|$)|noncommercial|noderiv")
SEMVER = re.compile(r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")


def in_scope(path):
    """Whether a tracked path needs an entry, given every tracked path's directory holding a record (see `records`)."""
    p = pathlib.PurePosixPath(path)
    if path.startswith(OUTSIDE) or p.name in RECORDS or LICENCE_NAME.search(p.name):
        return False
    suffix = p.suffix.lower()
    return suffix in MEDIA or (suffix in TEST_SHADERS and "tests" in p.parts[:-1])


def scope(tracked):
    """The tracked paths that need an entry: media, test shaders and every file beside a NOTICE or SOURCES.md."""
    # the root NOTICE is the project's own licence notice, not a record of media
    recorded = {str(pathlib.PurePosixPath(t).parent) for t in tracked if pathlib.PurePosixPath(t).name in RECORDS} - {"."}
    beside = {t for t in tracked if str(pathlib.PurePosixPath(t).parent) in recorded and not t.startswith(OUTSIDE)
              and pathlib.PurePosixPath(t).name not in RECORDS and not LICENCE_NAME.search(pathlib.PurePosixPath(t).name)}
    return {t for t in tracked if in_scope(t)} | beside


def problems(manifest, tracked, read):
    """Every way `manifest` is not a true record of the in-scope files of `tracked`; `read(path)` gives a file's bytes."""
    found = []
    if not isinstance(manifest, dict) or set(manifest) != {"schema_version", "metadata", "files"}:
        return ["the manifest has exactly the keys schema_version, metadata and files"]
    if not isinstance(manifest["schema_version"], str) or not SEMVER.match(manifest["schema_version"]):
        found.append("schema_version is major.minor.patch")
    meta = manifest["metadata"]
    if not isinstance(meta, dict) or not {"model", "tool"} <= set(meta):
        found.append("metadata names the model and tool that wrote the manifest")
    files = manifest["files"]
    if not isinstance(files, dict):
        return found + ["files maps each path to its entry"]
    tracked = set(tracked)
    wanted = scope(tracked)
    found += [f"{p}: no entry" for p in sorted(wanted - set(files))]
    found += [f"{p}: entry for a file that is not tracked or not in scope" for p in sorted(set(files) - wanted)]
    for path in sorted(set(files) & wanted):
        found += [f"{path}: {p}" for p in entry_problems(path, files[path], files, tracked, read)]
    return found


def entry_problems(path, entry, files, tracked, read):
    if not isinstance(entry, dict) or entry.get("origin") not in FIELDS:
        return [f"origin is one of {sorted(FIELDS)}"]
    required, optional = FIELDS[entry["origin"]]
    keys = set(entry)
    found = [f"has no {k}" for k in sorted((COMMON | required) - keys)]
    found += [f"unknown field {k}" for k in sorted(keys - COMMON - required - optional)]
    if found:
        return found
    data = read(path)
    if entry["sha256"] != hashlib.sha256(data).hexdigest():
        found.append("sha256 does not match the file")
    if entry["bytes"] != len(data):
        found.append(f"bytes is {entry['bytes']}, the file has {len(data)}")
    for key in ("note", "evidence"):
        if key in entry and not (isinstance(entry[key], str) and entry[key].strip()):
            found.append(f"{key} is empty")

    def named(key):
        value = entry.get(key)
        if value is None:
            return None
        if not isinstance(value, str) or value not in tracked:
            found.append(f"{key} {value!r} is not a tracked file")
            return None
        return value

    origin = entry["origin"]
    name = pathlib.PurePosixPath(path)
    if origin == "generated":
        generator = named("generator")
        named("record")
        named("checked_by")
        # the generator (or the record beside its output) names the file it writes
        texts = [read(p).decode("utf-8", "replace") for p in (generator, entry.get("record")) if p in tracked]
        if generator and not any(name.name in t or name.stem in t for t in texts):
            found.append(f"neither the generator nor the record names {name.name}")
    elif origin == "third-party":
        if not str(entry["source"]).startswith("https://"):
            found.append("source is an https URL")
        if not isinstance(entry["licence"], str) or not entry["licence"].strip():
            found.append("licence is an SPDX expression")
        elif NOT_REDISTRIBUTABLE.search(entry["licence"]):
            found.append(f"licence {entry['licence']} does not allow redistribution with this project")
        named("licence_file")
        notice = named("notice")
        if notice and name.name not in read(notice).decode("utf-8", "replace"):
            found.append(f"{notice} does not name {name.name}")
        if entry.get("in_binary") is True and name.name not in read(NOTICES).decode("utf-8", "replace"):
            found.append(f"compiled into the binary but {NOTICES} does not name {name.name}")
        elif "in_binary" in entry and not isinstance(entry["in_binary"], bool):
            found.append("in_binary is true or false")
    elif origin == "upstream":
        if not str(entry["source"]).startswith("https://"):
            found.append("source is an https URL")
    elif origin == "copy":
        twin = named("of")
        if twin == path:
            found.append("is a copy of itself")
        elif twin and read(twin) != read(path):
            found.append(f"is not byte-equal to {twin}")
        elif twin and files.get(twin, {}).get("origin") == "copy":
            found.append(f"{twin} is itself a copy; name the original")
    return found


def tracked_files():
    out = subprocess.run(["git", "ls-files", "-z"], cwd=ROOT, check=True, capture_output=True).stdout
    return [p for p in out.decode().split("\0") if p]


def read_tree(path):
    return (ROOT / path).read_bytes()


@unittest.skipUnless(shutil.which("git") and (ROOT / ".git").exists(), "not a git checkout")
class SourcesTests(unittest.TestCase):
    def test_the_manifest_is_a_true_record_of_every_media_file(self):
        manifest = json.loads((ROOT / MANIFEST).read_text())
        self.assertEqual(problems(manifest, tracked_files(), read_tree), [])

    def test_every_licence_and_record_file_is_named_by_an_entry_or_a_notice(self):
        """A licence text in the tree belongs to something: an entry's licence_file, or a vendored crate."""
        manifest = json.loads((ROOT / MANIFEST).read_text())
        named = {e.get("licence_file") for e in manifest["files"].values()}
        for path in tracked_files():
            p = pathlib.PurePosixPath(path)
            if LICENCE_NAME.search(p.name) and not path.startswith(OUTSIDE) and str(p.parent) != ".":
                with self.subTest(licence=path):
                    self.assertIn(path, named)


class ProblemsTests(unittest.TestCase):
    """The checks reject what they exist to reject."""

    def setUp(self):
        self.tree = {
            "a/x.png": b"png", "a/y.png": b"png", "a/z.png": b"other", "tools/make.py": b"writes x.png and z",
            "a/LICENSE": b"MIT", "b/NOTICE": b"x.png and w.json from upstream", "b/w.json": b"{}",
            "c/tests/s.glsl": b"//", "src/s.glsl": b"//", "vendor/v.png": b"v", NOTICES: b"names nothing",
            "NOTICE": b"the project's own", "README.md": b"beside the root NOTICE",
        }
        self.files = {
            "a/x.png": self.entry("a/x.png", origin="generated", generator="tools/make.py"),
            "a/y.png": self.entry("a/y.png", origin="copy", of="a/x.png"),
            "a/z.png": self.entry("a/z.png", origin="unknown", note="three bytes"),
            "b/w.json": self.entry("b/w.json", origin="third-party", source="https://example.org", licence="MIT",
                                   licence_file="a/LICENSE", notice="b/NOTICE"),
            "c/tests/s.glsl": self.entry("c/tests/s.glsl", origin="authored", evidence="its header says so"),
        }

    def entry(self, path, **fields):
        data = self.tree[path]
        return {"sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data), **fields}

    def check(self):
        manifest = {"schema_version": "1.0.0", "metadata": {"model": "m", "tool": "t"}, "files": self.files}
        return problems(manifest, list(self.tree), lambda p: self.tree[p])

    def test_a_true_record_has_no_problem(self):
        self.assertEqual(self.check(), [])

    def test_scope_takes_media_test_shaders_and_files_beside_a_notice(self):
        self.assertEqual(scope(self.tree), {"a/x.png", "a/y.png", "a/z.png", "b/w.json", "c/tests/s.glsl"})

    def test_a_missing_and_an_orphan_entry_are_named(self):
        self.files["a/q.png"] = self.files.pop("a/z.png")
        self.assertEqual(self.check(), ["a/z.png: no entry",
                                        "a/q.png: entry for a file that is not tracked or not in scope"])

    def test_a_changed_file_fails_its_digest_and_size(self):
        self.tree["a/z.png"] = b"changed"
        self.assertEqual(self.check(), ["a/z.png: sha256 does not match the file", "a/z.png: bytes is 5, the file has 7"])

    def test_an_unknown_origin_and_an_unknown_field_are_refused(self):
        self.files["a/z.png"]["origin"] = "found"
        self.files["a/x.png"]["licence"] = "MIT"
        self.assertEqual(self.check(), ["a/x.png: unknown field licence",
                                        f"a/z.png: origin is one of {sorted(FIELDS)}"])

    def test_a_missing_required_field_is_named(self):
        del self.files["b/w.json"]["licence_file"]
        self.assertEqual(self.check(), ["b/w.json: has no licence_file"])

    def test_an_empty_note_does_not_count_as_saying_the_origin_is_unknown(self):
        self.files["a/z.png"]["note"] = "  "
        self.assertEqual(self.check(), ["a/z.png: note is empty"])

    def test_a_generator_must_exist_and_name_its_output(self):
        self.files["a/x.png"]["generator"] = "tools/gone.py"
        self.assertEqual(self.check(), ["a/x.png: generator 'tools/gone.py' is not a tracked file"])
        self.files["a/x.png"]["generator"] = "a/LICENSE"
        self.assertEqual(self.check(), ["a/x.png: neither the generator nor the record names x.png"])

    def test_a_copy_must_be_byte_equal_to_an_original(self):
        self.files["a/y.png"]["of"] = "a/z.png"
        self.assertEqual(self.check(), ["a/y.png: is not byte-equal to a/z.png"])
        self.files["a/y.png"]["of"] = "a/y.png"
        self.assertEqual(self.check(), ["a/y.png: is a copy of itself"])

    def test_third_party_needs_an_https_source_a_free_licence_its_text_and_its_notice(self):
        w = self.files["b/w.json"]
        w.update(source="http://example.org", licence="CC-BY-NC-4.0", licence_file="a/COPYING", notice="b/NOTICE")
        self.tree["b/NOTICE"] = b"names nothing"
        self.assertEqual(self.check(), [
            "b/w.json: source is an https URL",
            "b/w.json: licence CC-BY-NC-4.0 does not allow redistribution with this project",
            "b/w.json: licence_file 'a/COPYING' is not a tracked file",
            "b/w.json: b/NOTICE does not name w.json",
        ])

    def test_a_no_derivatives_licence_is_refused(self):
        self.files["b/w.json"]["licence"] = "CC-BY-ND-4.0"
        self.assertEqual(self.check(), ["b/w.json: licence CC-BY-ND-4.0 does not allow redistribution with this project"])

    def test_third_party_data_in_the_binary_is_named_in_the_notices(self):
        self.files["b/w.json"]["in_binary"] = True
        self.assertEqual(self.check(), [f"b/w.json: compiled into the binary but {NOTICES} does not name w.json"])
        self.tree[NOTICES] = b"w.json, MIT"
        self.assertEqual(self.check(), [])

    def test_a_malformed_manifest_is_refused_whole(self):
        self.assertEqual(problems([], [], None), ["the manifest has exactly the keys schema_version, metadata and files"])
        bad = {"schema_version": "1.0", "metadata": {}, "files": []}
        self.assertEqual(problems(bad, [], None), ["schema_version is major.minor.patch",
                                                   "metadata names the model and tool that wrote the manifest",
                                                   "files maps each path to its entry"])


if __name__ == "__main__":
    unittest.main()
