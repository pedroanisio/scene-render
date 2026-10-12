//! SREP 72: `project/@precision="f32"` covers the 2D working textures; a document that also draws 3D content is told
//! that the 3D renderer keeps 16-bit buffers, with the information finding PREC1 that SREP 72 names (Semantics 1,
//! Errors). The finding is the evaluator's because the 3D content is known only after expansion: an object3D inside a
//! symbol instance or an included document counts, a symbol that nothing instances does not.

use std::path::Path;

use sr_model::Severity;

fn head(precision: &str) -> String {
    format!(
        r##"<scene version="1.6"><project width="64" height="64" fps="24" duration="1" {precision}/>
  <materials><material id="m" baseColor="#808080"/></materials>"##
    )
}

fn findings_of(xml: &str, base: Option<&Path>) -> Vec<sr_model::Diagnostic> {
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: base.map(Path::to_path_buf) };
    let doc = sr_model::load_str(xml, &opts).unwrap_or_else(|e| panic!("{e}"));
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"));
    ev.warnings().iter().filter(|d| d.code.starts_with("PREC") || d.code == "E23").cloned().collect()
}

fn warnings(precision: &str, composition: &str) -> Vec<sr_model::Diagnostic> {
    findings_of(&format!("{}<composition>{composition}</composition></scene>", head(precision)), None)
}

const SPHERE: &str = r#"<object3D id="ball" primitive="sphere" radius="10" x="32" y="32" material="m"/>"#;
const RECT: &str = r##"<shape id="r" shape="rect" width="10" height="10" fill="#FFFFFF"/>"##;

#[test]
fn f32_with_3d_content_is_reported_once_as_the_information_finding_prec1() {
    let w = warnings(r#"precision="f32""#, &format!("{RECT}{SPHERE}{}", SPHERE.replace("ball", "ball2")));
    assert_eq!(w.len(), 1, "{w:?}");
    assert_eq!(w[0].code, "PREC1", "SREP 72 names the finding PREC1: {w:?}");
    assert_eq!(w[0].severity, Severity::Info);
    assert!(w[0].message.contains("3D internal buffers stay f16"), "SREP 72's wording: {}", w[0].message);
    assert!(w[0].message.contains("ball"), "names the 3D node: {}", w[0].message);
}

#[test]
fn every_kind_of_3d_content_that_srep_72_names_is_reported() {
    let w = warnings(r#"precision="f32""#, SPHERE);
    assert!(w.len() == 1 && w[0].code == "PREC1", "object3D: {w:?}");
    // particles3D and ocean are valid in version 1.3 and later (P3D1 and OCN1 refuse 1.0 to 1.2 only, SREP 83), so
    // they meet precision in the same 1.6 document as the object3D
    for (kind, node) in [
        ("particles3D", r#"<particles3D id="p" rate="10" lifetime="1" maxParticles="10"/>"#),
        ("ocean", r#"<ocean id="sea" width="8" depth="8" cellSize="0.5" bottomDepth="2"/>"#),
    ] {
        let xml = head(r#"precision="f32""#) + &format!("<composition>{node}</composition></scene>");
        let w = findings_of(&xml, None);
        assert!(w.len() == 1 && w[0].code == "PREC1", "{kind}: {w:?}");
    }
}

#[test]
fn no_report_without_3d_content_or_at_f16() {
    assert!(warnings(r#"precision="f32""#, RECT).is_empty());
    assert!(warnings("", SPHERE).is_empty());
    assert!(warnings(r#"precision="f16""#, SPHERE).is_empty());
}

#[test]
fn three_d_content_counts_after_expansion() {
    let symbol = format!(r#"<symbols><symbol id="s" width="64" height="64">{SPHERE}</symbol></symbols>"#);
    let instanced = format!(
        "{}{symbol}<composition>{RECT}<instance id=\"i\" symbol=\"s\"/></composition></scene>",
        head(r#"precision="f32""#)
    );
    let w = findings_of(&instanced, None);
    assert!(w.len() == 1 && w[0].code == "PREC1", "a sphere inside an instance: {w:?}");
    let unused = format!("{}{symbol}<composition>{RECT}</composition></scene>", head(r#"precision="f32""#));
    assert!(findings_of(&unused, None).is_empty(), "a symbol that nothing instances draws no 3D");

    let d = std::env::temp_dir().join(format!("sr-eval-srep72-include-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("B.scene.xml"), format!("{}<composition>{SPHERE}</composition></scene>", head(""))).unwrap();
    let including = format!(
        "{}<composition>{RECT}<include id=\"inc\" src=\"B.scene.xml\"/></composition></scene>",
        head(r#"precision="f32""#)
    );
    let w = findings_of(&including, Some(&d));
    assert!(w.len() == 1 && w[0].code == "PREC1", "a sphere in an included document: {w:?}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn prec1_is_in_the_evaluator_codes_and_e23_is_gone() {
    let codes = sr_eval::codes::CODES;
    assert!(codes.iter().any(|(c, s)| *c == "PREC1" && s.contains("SREP 72")), "PREC1 is explained");
    assert!(!codes.iter().any(|(c, _)| *c == "E23"), "E23 meant only this finding; PREC1 replaces it");
}

#[test]
fn the_corpus_document_reports_prec1_and_stays_valid() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/valid/prec1-f32-3d.scene.xml");
    let doc = sr_model::load_file(&path, &sr_model::LoadOptions::default()).unwrap_or_else(|e| panic!("{e}"));
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"));
    let w: Vec<_> = ev.warnings().iter().filter(|d| d.code == "PREC1").collect();
    assert_eq!(w.len(), 1, "{:?}", ev.warnings());
    assert!(ev.warnings().iter().all(|d| !d.is_error()));
}
