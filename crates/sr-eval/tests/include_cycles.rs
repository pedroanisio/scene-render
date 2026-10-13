//! An `<include>` loads another document and expands it in place, so a document that includes itself, directly or
//! through others, or a chain of includes too deep, is refused with E13 rather than expanded without end. A file is
//! one file however its `src` spells the path.

use std::path::{Path, PathBuf};

/// A fresh directory for one test's documents, with an empty `sub/` for paths that pass through it.
fn dir(test: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr-eval-include-cycles-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("sub")).unwrap();
    d
}

/// Writes a document whose composition is `body`.
fn write(path: &Path, body: &str) {
    let text = format!(
        r#"<scene version="1.1"><project width="8" height="8" fps="1" duration="1"/><composition>{body}</composition></scene>"#
    );
    std::fs::write(path, text).unwrap();
}

/// Builds the evaluator of the document at `path`.
fn evaluate(path: &Path) -> Result<(), sr_model::Report> {
    let doc = sr_model::load_file(path, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
    sr_eval::Evaluator::new(&doc, &Default::default()).map(|_| ())
}

/// The messages of the E13 errors of a refused document.
fn e13(r: Result<(), sr_model::Report>) -> Vec<String> {
    let report = r.expect_err("the document is refused");
    let found: Vec<String> =
        report.diagnostics.iter().filter(|d| d.is_error() && d.code == "E13").map(|d| d.message.clone()).collect();
    assert!(!found.is_empty(), "{report}");
    found
}

#[test]
fn a_document_including_itself_under_other_spellings_is_one_cycle() {
    let d = dir("self");
    let a = d.join("A.scene.xml");
    write(&a, r#"<include id="i" src="./A.scene.xml"/><include id="j" src="sub/../A.scene.xml"/>"#);
    let found = e13(evaluate(&a));
    // each include of the root loads A once more; inside it, both includes name a file already open
    assert_eq!(found.len(), 4, "{found:#?}");
    assert!(found.iter().all(|m| m.contains("inside itself")), "{found:#?}");
}

#[test]
fn documents_including_each_other_are_one_cycle() {
    let d = dir("mutual");
    let (a, b) = (d.join("A.scene.xml"), d.join("B.scene.xml"));
    write(&a, r#"<include id="b" src="sub/../B.scene.xml"/>"#);
    write(&b, r#"<include id="a1" src="./A.scene.xml"/><include id="a2" src="A.scene.xml"/>"#);
    let found = e13(evaluate(&a));
    // A, then B, then each of B's two includes of A, whose include of B closes the cycle
    assert_eq!(found.len(), 2, "{found:#?}");
    assert!(found.iter().all(|m| m.contains("\"b\" includes") && m.contains("inside itself")), "{found:#?}");
}

#[test]
fn includes_nest_to_the_limit_and_no_deeper() {
    let max = sr_model::limits::MAX_INCLUDE_DEPTH;
    let chain = |d: &Path, levels: usize| {
        for i in 0..=levels {
            let body =
                if i < levels { format!(r#"<include id="n" src="D{}.scene.xml"/>"#, i + 1) } else { String::new() };
            write(&d.join(format!("D{i}.scene.xml")), &body);
        }
        d.join("D0.scene.xml")
    };
    let d = dir("deep-ok");
    assert!(evaluate(&chain(&d, max)).is_ok(), "{max} levels of includes are accepted");
    let d = dir("deep");
    let found = e13(evaluate(&chain(&d, max + 1)));
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].contains(&format!("nested more than {max} levels deep")), "{}", found[0]);
}
