//! Fixture files that tests write into the shared fixture directory while other test threads read them.

mod common;

use std::io::Read;
use std::path::Path;

#[test]
fn a_rewritten_fixture_replaces_the_file_whole() {
    let first = vec![1u8; 1 << 20];
    let path = common::write_fixture("fixture-files-whole.bin", &first);
    assert_eq!(path, common::fixtures().join("fixture-files-whole.bin"));
    // a reader that opened the file before it was written again: written in place, the file it reads is truncated
    // under it; replaced, it still reads the whole of what it opened
    let mut reader = std::fs::File::open(&path).unwrap();
    common::write_fixture("fixture-files-whole.bin", [2u8; 16]);
    let mut seen = Vec::new();
    reader.read_to_end(&mut seen).unwrap();
    assert!(seen == first, "the open file changed under its reader: {} bytes", seen.len());
    assert_eq!(std::fs::read(&path).unwrap(), [2u8; 16]);
    let left: Vec<_> = std::fs::read_dir(common::fixtures())
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(left.is_empty(), "no temporary file is left behind: {left:?}");
}

/// Every `.rs` file under `dir`.
fn sources(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            sources(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn no_test_writes_a_shared_fixture_in_place() {
    // the fixture directory is one per process and test threads read its files while others write them: a file
    // written in place can be read half written, so every test writes through `write_fixture`
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut files = Vec::new();
    sources(&tests, &mut files);
    // `fs::write` or an image's `save` straight into the directory, the call spread over lines or not
    let in_place = regex::Regex::new(r"(fs::write|\.save)\(\s*(common::)?fixtures\(\)").unwrap();
    let mut found: Vec<String> = Vec::new();
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        for m in in_place.find_iter(&text) {
            let line = text[..m.start()].lines().count();
            found.push(format!("{}:{line}", f.strip_prefix(&tests).unwrap().display()));
        }
    }
    assert!(found.is_empty(), "fixtures written in place:\n{}", found.join("\n"));
}
