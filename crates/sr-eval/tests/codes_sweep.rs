//! The diagnostic catalogues and the code that emits them agree. A code written at an emit site that no catalogue
//! lists is one `scene-render explain` cannot describe; a catalogued code that nothing emits is a description of a
//! diagnostic that cannot occur; a code the README names that no catalogue lists is a stale reference.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use regex::Regex;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Literals of the shape of a code that are not diagnostic codes: constants of the expression language, a WAV chunk
/// tag and a fuzz target's value type.
const NOT_CODES: &[&str] = &["LN10", "LN2", "SQRT2", "RF64", "VEC3"];

/// Every code of every catalogue: the fixed and inert ones of the model, the Schematron asserts, the evaluation codes.
fn catalogue() -> BTreeSet<String> {
    let mut all: BTreeSet<String> = sr_model::codes::all().iter().map(|c| c.code.to_string()).collect();
    all.extend(sr_eval::codes::CODES.iter().map(|(c, _)| c.to_string()));
    all
}

/// The codes a catalogue lists that an emit site writes by hand: not the Schematron asserts, which the rule engine
/// emits from the schema.
fn hand_emitted() -> BTreeSet<String> {
    let mut all: BTreeSet<String> = sr_model::codes::all()
        .iter()
        .filter(|c| c.rule.is_none() && c.stage != "inert")
        .map(|c| c.code.to_string())
        .collect();
    all.extend(sr_eval::codes::CODES.iter().map(|(c, _)| c.to_string()));
    all
}

/// The code-shaped literals of one source text: `"E01"` and `"CMP12: the histogram …"`.
fn literals(text: &str) -> BTreeSet<String> {
    let re = Regex::new(r#""((?:[A-Z]{1,5}[0-9]{1,3})|XML)(?:"|:)"#).unwrap();
    re.captures_iter(text).map(|c| c[1].to_string()).collect()
}

/// The literals of `text` that are not in `known`.
fn strays(text: &str, known: &BTreeSet<String>) -> Vec<String> {
    literals(text).into_iter().filter(|l| !known.contains(l) && !NOT_CODES.contains(&l.as_str())).collect()
}

/// The non-test Rust sources of the workspace's crates, without the catalogues themselves.
fn sources() -> Vec<(PathBuf, String)> {
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if path.is_dir() {
                if name != "tests" && name != "target" {
                    walk(&path, out);
                }
            } else if name.ends_with(".rs") && name != "codes.rs" {
                let text = std::fs::read_to_string(&path).unwrap();
                out.push((path, text));
            }
        }
    }
    let mut out = Vec::new();
    for krate in std::fs::read_dir(root().join("crates")).unwrap() {
        let src = krate.unwrap().path().join("src");
        if src.is_dir() {
            walk(&src, &mut out);
        }
    }
    assert!(out.len() > 50, "found only {} sources; the walk is broken", out.len());
    out
}

#[test]
fn every_code_an_emit_site_writes_is_in_a_catalogue() {
    let known = catalogue();
    let mut bad = Vec::new();
    for (path, text) in sources() {
        for s in strays(&text, &known) {
            bad.push(format!("{}: {s}", path.strip_prefix(root()).unwrap().display()));
        }
    }
    assert!(
        bad.is_empty(),
        "codes written in source that no catalogue lists (add them to codes.rs, or to NOT_CODES):\n{}",
        bad.join("\n")
    );
}

#[test]
fn every_hand_emitted_catalogue_code_is_written_somewhere() {
    let written: BTreeSet<String> = sources().iter().flat_map(|(_, t)| literals(t)).collect();
    let missing: Vec<String> = hand_emitted().into_iter().filter(|c| !written.contains(c)).collect();
    assert!(missing.is_empty(), "catalogued codes that no source writes: {missing:?}");
}

#[test]
fn every_code_the_readme_names_is_in_a_catalogue() {
    let known = catalogue();
    let text = std::fs::read_to_string(root().join("README.md")).unwrap();
    let re = Regex::new(r"`((?:[A-Z]{1,5}[0-9]{1,3})|INERT-I[0-9]+|I[0-9]+)`").unwrap();
    let mut bad = BTreeSet::new();
    for c in re.captures_iter(&text) {
        let token = &c[1];
        // `I7` is the short form of `INERT-I7`; `R24-*` names a family whose members are `R24-…`
        let long = if token.starts_with('I') && !token.starts_with("INERT") {
            format!("INERT-{token}")
        } else {
            token.to_string()
        };
        let family = format!("{token}-");
        if !known.contains(&long) && !known.iter().any(|k| k.starts_with(&family)) {
            bad.insert(token.to_string());
        }
    }
    assert!(bad.is_empty(), "README names codes that no catalogue lists: {bad:?}");
}

#[test]
fn the_sweep_flags_an_unlisted_code_and_a_prefixed_message() {
    let known: BTreeSet<String> = ["E01", "CMP12"].iter().map(|s| s.to_string()).collect();
    assert_eq!(strays(r#"diag("E99", "x"); diag("E01", "y");"#, &known), vec!["E99"]);
    assert_eq!(strays(r#"format!("CMP13: too big")"#, &known), vec!["CMP13"]);
    assert!(strays(r#"let n = "CMP12: ok";"#, &known).is_empty());
    assert!(strays(r#"("LN10", 2.30)"#, &known).is_empty());
}
