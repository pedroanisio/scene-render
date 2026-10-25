//! The space-transforms draft (schema/UPSTREAM): the modifier types matrix, map and complex with their attributes,
//! deform on a group, the Schematron rules SPT1 and SPT2, and their clause of V17, the one version gate of the 1.8
//! drafts.

fn codes(version: &str, composition: &str) -> Vec<String> {
    let xml = format!(
        r#"<scene version="{version}"><project width="64" height="64" fps="30" duration="1"/><composition>{composition}</composition></scene>"#
    );
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
    sr_model::validate_str(&xml, &opts).diagnostics.into_iter().map(|d| d.code).collect()
}

fn shape(modifier: &str) -> String {
    format!(r#"<shape id="s" shape="rect" width="40" height="30"><deform><modifier {modifier}/></deform></shape>"#)
}

const GRID: &str = r##"<group id="g"><deform><modifier type="complex" function="square" unit="100"/></deform><shape id="l" shape="rect" width="40" height="30" stroke="#FFFFFFFF" strokeWidth="2"/></group>"##;

#[track_caller]
fn valid(composition: &str) {
    let got = codes("1.8", composition);
    assert!(got.is_empty(), "{composition}: {got:?}");
}

#[track_caller]
fn refused(composition: &str, code: &str) {
    let got = codes("1.8", composition);
    assert!(got.iter().any(|c| c == code), "{composition}: want {code}, got {got:?}");
}

#[test]
fn the_three_types_and_a_group_deform_are_valid_at_1_8() {
    valid(&shape(
        r#"type="matrix" a="1" b="0.5" c="0.25" d="1" e="10" f="-5" centerX="0" centerY="0" tolerance="0.1""#,
    ));
    valid(&shape(r#"type="matrix""#));
    valid(&shape(r#"type="map" x="u * u - v * v" y="2 * u * v" unit="50""#));
    for f in ["square", "cube", "inverse", "exp", "log", "sqrt", "sin", "cos", "joukowski"] {
        valid(&shape(&format!(r#"type="complex" function="{f}""#)));
    }
    valid(&shape(r#"type="complex" function="mobius" coefficients="1 0 0 1 0 1 1 0""#));
    valid(GRID);
    // the numeric attributes animate as the other modifiers' do
    valid(
        r#"<shape id="s" shape="rect" width="40" height="30"><deform><modifier type="matrix"><animate property="b"><key time="0" value="0"/><key time="1" value="0.5"/></animate></modifier></deform></shape>"#,
    );
}

#[test]
fn malformed_values_are_structure_errors() {
    for bad in [
        r#"type="complex" function="tan""#,
        r#"type="matrix" a="one""#,
        r#"type="map" x="u" y="v" unit="0""#,
        r#"type="map" x="u" y="v" tolerance="-1""#,
        r#"type="complex" function="mobius" coefficients="1 0 x 1 0 1 1 0""#,
    ] {
        refused(&shape(bad), "S06");
    }
}

#[test]
fn each_type_has_its_attributes_and_no_other_spt1() {
    for bad in [
        r#"type="map" x="u""#,
        r#"type="map" y="v""#,
        r#"type="complex""#,
        r#"type="complex" function="mobius""#,
        r#"type="complex" function="mobius" coefficients="1 0 0 1 0 1 1""#,
        r#"type="complex" function="square" coefficients="1 0 0 1 0 1 1 0""#,
        r#"type="bend" x="u" y="v""#,
        r#"type="matrix" function="square""#,
        r#"type="map" x="u" y="v" function="exp""#,
        r#"type="twist" coefficients="1 0 0 1 0 1 1 0""#,
    ] {
        refused(&shape(bad), "SPT1");
    }
}

#[test]
fn a_mobius_map_is_invertible_spt2() {
    // a = 1, b = 2, c = 1, d = 2: ad − bc = 0
    refused(&shape(r#"type="complex" function="mobius" coefficients="1 0 2 0 1 0 2 0""#), "SPT2");
    // a = i, b = 1, c = −1, d = i: ad − bc = −1 + 1 = 0, on the complex numbers alone
    refused(&shape(r#"type="complex" function="mobius" coefficients="0 1 1 0 -1 0 0 1""#), "SPT2");
    // a = i, b = 1, c = 1, d = i: ad − bc = −2
    valid(&shape(r#"type="complex" function="mobius" coefficients="0 1 1 0 1 0 0 1""#));
}

#[test]
fn before_1_8_the_types_and_a_group_deform_are_refused_v17() {
    for v in ["1.0", "1.1", "1.4", "1.6", "1.7"] {
        for c in [shape(r#"type="matrix""#), shape(r#"type="map" x="u" y="v""#), GRID.to_string()] {
            let got = codes(v, &c);
            assert!(got.iter().any(|k| k == "V17"), "{v} {c}: {got:?}");
        }
    }
    // the fourteen existing types are not gated
    let got = codes("1.7", &shape(r#"type="twist" amount="30""#));
    assert!(!got.iter().any(|k| k == "V17"), "{got:?}");
}

#[test]
fn a_group_takes_no_rigid_or_soft_body() {
    // the draft gives a group deform alone: bodyBehaviour's other two have no meaning on a group
    refused(r#"<group id="g"><rigidBody/></group>"#, "S02");
    refused(r#"<group id="g"><softBody/></group>"#, "S02");
}
