//! SREP 83: the version gates of SREP 40 (V8, MSQ1, P3D1, OCN1, CRT1, FRX1, GEO1) refuse only the versions before
//! 1.3, as BH1 and VOX1 do (SREPs 76 and 80), so a document of 1.4, 1.5 or 1.6 can use what SREP 40 adds.

use std::path::PathBuf;

/// One valid corpus document of version 1.3 for each gate, and the gate.
const BASES: [(&str, &str); 11] = [
    ("V8", "volume"),
    ("V8", "pyro-follow"),
    ("V8", "volume-scatter-bounces"),
    ("MSQ1", "mesh-sequence"),
    ("P3D1", "particles3d"),
    ("OCN1", "ocean"),
    ("OCN1", "ocean-density"),
    ("CRT1", "crater"),
    ("CRT1", "crater-mantle"),
    ("FRX1", "fracture"),
    ("GEO1", "globe-relief"),
];

fn corpus(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../tests/corpus/valid/{name}.scene.xml"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn codes(xml: &str) -> Vec<String> {
    sr_model::validate_str(xml, &sr_model::LoadOptions::without_assets())
        .diagnostics
        .into_iter()
        .map(|d| d.code)
        .collect()
}

fn at(xml: &str, version: &str) -> String {
    assert_eq!(xml.matches(r#"<scene version="1.3""#).count(), 1);
    xml.replace(r#"<scene version="1.3""#, &format!(r#"<scene version="{version}""#))
}

#[test]
fn every_version_from_1_3_on_admits_what_srep_40_adds() {
    for (gate, name) in BASES {
        let xml = corpus(name);
        let as_1_3 = codes(&xml);
        assert!(!as_1_3.contains(&gate.to_string()), "{name}: {as_1_3:?}");
        for later in ["1.4", "1.5", "1.6"] {
            assert_eq!(codes(&at(&xml, later)), as_1_3, "{name} as {later}");
        }
    }
}

#[test]
fn the_versions_before_1_3_are_still_refused() {
    for (gate, name) in BASES {
        for earlier in ["1.0", "1.1", "1.2"] {
            let got = codes(&at(&corpus(name), earlier));
            assert!(got.contains(&gate.to_string()), "{name} as {earlier}: {got:?}");
        }
    }
}

#[test]
fn a_1_6_feature_and_a_1_3_feature_in_one_document() {
    // a parametric surface (SREP 70, V14 refuses 1.0 to 1.5) beside a crater and an ocean (SREP 40)
    let xml = corpus("version-1.6-parametric-beside-crater");
    assert!(
        xml.contains(r#"<scene version="1.6""#) && xml.contains("<crater") && xml.contains(r#"primitive="parametric""#)
    );
    assert!(codes(&xml).is_empty(), "{:?}", codes(&xml));
    let at_1_5 = codes(&xml.replace(r#"<scene version="1.6""#, r#"<scene version="1.5""#));
    assert!(at_1_5.contains(&"V14".to_string()), "{at_1_5:?}");
}
