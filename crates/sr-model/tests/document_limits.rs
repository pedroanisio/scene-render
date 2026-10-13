//! A scene document is untrusted input: one past a limit of `sr_model::limits` is refused with a diagnostic, before
//! the parser allocates for it or recurses into it, and every document the project ships stays far inside them.

use std::path::{Path, PathBuf};

use sr_model::limits::{MAX_DEPTH, MAX_DOCUMENT_BYTES, MAX_XML_NODES};
use sr_model::{load_file, load_str, read_document, validate_file, validate_str, LoadError, LoadOptions, Report};

const HEAD: &str = r#"<scene version="1.1"><project width="8" height="8" fps="1" duration="1"/><composition>"#;
const TAIL: &str = "</composition></scene>";

/// A document whose deepest `<group>` lies `depth` levels below the root element (`<composition>` is level 1).
fn nested(depth: usize) -> String {
    let groups = depth - 1;
    let mut s = String::from(HEAD);
    for i in 0..groups {
        s.push_str(&format!("<group id=\"g{i}\">"));
    }
    s.push_str(&"</group>".repeat(groups));
    s.push_str(TAIL);
    s
}

fn codes(r: &Report) -> Vec<&str> {
    r.diagnostics.iter().filter(|d| d.is_error()).map(|d| d.code.as_str()).collect()
}

/// Runs `f` on a thread with a quarter of the default 2 MiB stack: enough for a document nested to the limit, far
/// too little for a parse or a validator that recursed once per level of a deeply nested one.
fn small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new().stack_size(512 << 10).spawn(f).unwrap().join().unwrap()
}

#[test]
fn deep_nesting_is_refused_before_the_parse() {
    small_stack(|| {
        let xml = nested(100_000);
        let report = validate_str(&xml, &LoadOptions::without_assets());
        assert_eq!(codes(&report), ["S12"], "{report}");
        let d = &report.diagnostics[0];
        assert!(d.message.contains(&format!("more than {MAX_DEPTH} levels")), "{}", d.message);
        // the first group past the limit, where it is written: `<composition>` is level 1, group `g0` level 2
        let at = xml.find(&format!("<group id=\"g{}\">", MAX_DEPTH - 1)).unwrap();
        assert_eq!(d.loc.offset as usize, at);
        assert_eq!(d.loc.line, 1);
        match load_str(&xml, &LoadOptions::without_assets()) {
            Err(LoadError::Invalid(r)) => assert_eq!(codes(&r), ["S12"]),
            other => panic!("{:?}", other.map(|_| ())),
        }
    });
}

#[test]
fn nesting_at_the_limit_is_accepted() {
    small_stack(|| {
        let at = validate_str(&nested(MAX_DEPTH), &LoadOptions::without_assets());
        assert!(!at.has_errors(), "{at}");
        let past = validate_str(&nested(MAX_DEPTH + 1), &LoadOptions::without_assets());
        assert_eq!(codes(&past), ["S12"], "{past}");
    });
}

#[test]
fn too_many_nodes_are_refused() {
    let mut xml = String::with_capacity(HEAD.len() + 4 * MAX_XML_NODES as usize + 64);
    xml.push_str(HEAD);
    xml.push_str("<group id=\"g\">");
    for _ in 0..MAX_XML_NODES {
        xml.push_str("<a/>");
    }
    xml.push_str("</group>");
    xml.push_str(TAIL);
    assert!((xml.len() as u64) < MAX_DOCUMENT_BYTES);
    let report = validate_str(&xml, &LoadOptions::without_assets());
    assert_eq!(codes(&report), ["XML"], "{report}");
    assert!(report.diagnostics[0].message.contains(&format!("more than {MAX_XML_NODES} XML nodes")), "{report}");
}

#[test]
fn an_oversize_file_is_refused_unread() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("document_limits");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("huge.scene.xml");
    // sparse: no disk space is used, and a reader that trusted the file would allocate it all
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(MAX_DOCUMENT_BYTES + 1).unwrap();
    drop(file);
    let refused = |r: &Report| {
        assert_eq!(codes(r), ["XML"], "{r}");
        assert!(r.diagnostics[0].message.contains(&format!("larger than {MAX_DOCUMENT_BYTES} bytes")), "{r}");
    };
    match read_document(&path) {
        Err(LoadError::Invalid(r)) => refused(&r),
        other => panic!("{:?}", other.map(|s| s.len())),
    }
    refused(&validate_file(&path, &LoadOptions::without_assets()).unwrap());
    match load_file(&path, &LoadOptions::without_assets()) {
        Err(LoadError::Invalid(r)) => refused(&r),
        other => panic!("{:?}", other.map(|_| ())),
    }
    // the same bytes in memory
    let zeros = "\0".repeat(MAX_DOCUMENT_BYTES as usize + 1);
    refused(&validate_str(&zeros, &LoadOptions::without_assets()));
    std::fs::remove_file(&path).unwrap();
}

/// Every `.xml` file under `dir`.
fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().is_some_and(|e| e == "xml") {
            out.push(path);
        }
    }
}

#[test]
fn shipped_documents_are_far_inside_the_limits() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    walk(&root.join("tests/corpus"), &mut files);
    walk(&root.join("examples"), &mut files);
    assert!(files.len() > 100, "{}", files.len());
    let (mut bytes, mut nodes, mut depth) = (0u64, 0usize, 0usize);
    let mut largest = PathBuf::new();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        if text.len() as u64 > bytes {
            bytes = text.len() as u64;
            largest = f.clone();
        }
        // a corpus document that is not well-formed on purpose has no tree to measure
        let Ok(doc) = roxmltree::Document::parse(&text) else { continue };
        nodes = nodes.max(doc.descendants().count());
        depth = depth.max(doc.descendants().map(|n| n.ancestors().count()).max().unwrap_or(0));
    }
    assert!(bytes * 1000 < MAX_DOCUMENT_BYTES, "{bytes} bytes");
    assert!(nodes * 1000 < MAX_XML_NODES as usize, "{nodes} nodes");
    assert!(depth * 4 < MAX_DEPTH, "{depth} levels");
    // the largest still goes through the whole pipeline with no limit in its way
    let report = small_stack(move || validate_file(&largest, &LoadOptions::without_assets()).unwrap());
    let limited = |d: &&sr_model::Diagnostic| {
        d.code == "S12" || d.message.contains("larger than") || d.message.contains("XML nodes")
    };
    assert!(!report.diagnostics.iter().any(|d| limited(&d)), "{report}");
}
