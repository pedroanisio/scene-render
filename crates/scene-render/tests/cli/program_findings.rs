//! `validate` reports what loading a document with its build-mode programs (SREP 66) rejects, as render does:
//! an emitted id that collides is PRG14 at validation, not only at render, and the bare references of SREP 87 are
//! W13 or PRG17 there too.

use std::path::PathBuf;
use std::process::{Command, Output};

fn modules() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sr-wasm/tests/modules")
}

fn sha(name: &str) -> String {
    use sha2::Digest;
    let b = std::fs::read(modules().join(format!("{name}.wasm"))).unwrap();
    sha2::Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

fn program(id: &str, name: &str) -> String {
    format!(r#"<program id="{id}" src="{name}.wasm" sha256="{}"/>"#, sha(name))
}

/// Writes the document into a fresh directory and validates it with the modules as the base directory.
fn validate(tag: &str, body: &str, extra: &[&str]) -> Output {
    let dir = std::env::temp_dir().join(format!("sr-cli-programs-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("doc.scene.xml");
    std::fs::write(
        &file,
        format!(
            r#"<scene version="1.6"><project width="640" height="360" fps="24" duration="1" seed="1"/><composition>{body}</composition></scene>"#
        ),
    )
    .unwrap();
    let modules = modules().display().to_string();
    let mut args = vec!["validate", "--base-dir", &modules];
    args.extend_from_slice(extra);
    let file = file.display().to_string();
    args.push(&file);
    let o = Command::new(env!("CARGO_BIN_EXE_scene-render"))
        .args(&args)
        .env("NO_COLOR", "1")
        .env("SR_PROGRAM_CACHE", "off")
        .output()
        .expect("binary runs");
    let _ = std::fs::remove_dir_all(&dir);
    o
}

/// The result of the `--format json` envelope a run printed.
fn result(o: &Output) -> serde_json::Value {
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).expect("json");
    v["result"].clone()
}

#[test]
fn a_written_id_equal_to_a_namespaced_one_fails_validation_with_prg14() {
    // SREP 87: program p1 emits g1 as p1.g1, which the document also writes
    let body = program("p1", "srep66-rect") + r#"<shape id="p1.g1" shape="rect" width="1" height="1"/>"#;
    let o = validate("collide", &body, &[]);
    let all = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    assert_eq!(o.status.code(), Some(1), "{all}");
    assert!(all.contains("PRG14"), "{all}");

    let j = validate("collide-json", &body, &["--format", "json"]);
    assert_eq!(j.status.code(), Some(1));
    let v = result(&j);
    let f = &v["files"][0];
    assert_eq!(f["valid"], false, "{v}");
    let codes: Vec<&str> = f["diagnostics"].as_array().unwrap().iter().filter_map(|d| d["code"].as_str()).collect();
    assert!(codes.contains(&"PRG14"), "{codes:?}");
}

#[test]
fn two_programs_emitting_the_same_id_validate() {
    // SREP 87: their ids are p1.g1 and p2.g1
    let o = validate("reuse", &(program("p1", "srep66-rect") + &program("p2", "srep66-rect")), &[]);
    let all = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    assert_eq!(o.status.code(), Some(0), "{all}");
}

#[test]
fn validate_reports_w13_and_prg17_for_bare_references() {
    // SREP 87, Semantics 4: one emitter warns W13, two fail PRG17
    let follower =
        r#"<shape id="f" shape="rect" width="1" height="1"><expression property="x">prop("g1.x")</expression></shape>"#;
    let one = validate("w13", &(program("p1", "srep66-rect") + follower), &["--format", "json"]);
    let v = result(&one);
    let f = &v["files"][0];
    assert_eq!(f["valid"], true, "{v}");
    let codes: Vec<&str> = f["diagnostics"].as_array().unwrap().iter().filter_map(|d| d["code"].as_str()).collect();
    assert_eq!(codes, ["W13"], "{v}");

    let two = validate("prg17", &(program("p1", "srep66-rect") + &program("p2", "srep66-rect") + follower), &[]);
    let all = format!("{}{}", String::from_utf8_lossy(&two.stdout), String::from_utf8_lossy(&two.stderr));
    assert_eq!(two.status.code(), Some(1), "{all}");
    assert!(all.contains("PRG17"), "{all}");
}

#[test]
fn a_single_program_still_validates() {
    let o = validate("single", &program("p1", "srep66-rect"), &[]);
    let all = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    assert_eq!(o.status.code(), Some(0), "{all}");
    assert!(!all.contains("PRG14"), "{all}");
}

#[test]
fn an_idref_to_an_emitted_id_validates_with_w13() {
    // SREP 87, Semantics 4: validation without the programs cannot resolve matte="g1"; loading can
    let o = validate(
        "idref",
        &(program("p1", "srep66-rect") + r#"<shape id="f" shape="rect" width="1" height="1" matte="g1"/>"#),
        &["--format", "json"],
    );
    let v = result(&o);
    let f = &v["files"][0];
    assert_eq!(f["valid"], true, "{v}");
    let codes: Vec<&str> = f["diagnostics"].as_array().unwrap().iter().filter_map(|d| d["code"].as_str()).collect();
    assert_eq!(codes, ["W13"], "{v}");
    // a name no program emits is still S10
    let o = validate(
        "idref-s10",
        &(program("p1", "srep66-rect") + r#"<shape id="f" shape="rect" width="1" height="1" matte="g9"/>"#),
        &["--format", "json"],
    );
    let v = result(&o);
    let codes: Vec<&str> =
        v["files"][0]["diagnostics"].as_array().unwrap().iter().filter_map(|d| d["code"].as_str()).collect();
    assert!(codes.contains(&"S10"), "{v}");
}
