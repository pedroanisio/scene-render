//! The semantic-tex draft (schema/UPSTREAM): `\part` in a formula's TeX, the layer child formulaAnimator, the
//! formulaMorph asset and shape/@morph, the Schematron rules FRM1 and FRM2, and their clause of V17, the one version
//! gate of the 1.8 drafts.

fn codes(version: &str, assets: &str, composition: &str) -> Vec<String> {
    let xml = format!(
        r#"<scene version="{version}"><project width="64" height="64" fps="30" duration="1"/><assets>{assets}</assets><composition>{composition}</composition></scene>"#
    );
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
    sr_model::validate_str(&xml, &opts).diagnostics.into_iter().map(|d| d.code).collect()
}

const FORMULAS: &str = r#"<formula id="f" tex="\part{lhs}{E} = \part{rhs}{m c^2}" width="64" height="32"/><formula id="g" tex="\part{lhs}{E} = \part{rhs}{m^2}" width="64" height="32"/><text id="t" text="hi" width="64" height="32" size="12"/>"#;

fn morph(attrs: &str) -> String {
    format!(r#"{FORMULAS}<formulaMorph id="m" {attrs}/>"#)
}

const PATH_MORPH: &str = r#"<shape id="s" shape="path" morph="true" width="10" height="10" path="M 0 0 L 10 0 L 10 10 Z"><animate property="path"><key time="0" value="M 0 0 L 10 0 L 10 10 Z"/><key time="1" value="M 0 0 L 10 0 L 10 10 L 0 10 Z"/></animate></shape>"#;

#[track_caller]
fn valid(assets: &str, composition: &str) {
    let got = codes("1.8", assets, composition);
    assert!(got.is_empty(), "{assets} {composition}: {got:?}");
}

#[track_caller]
fn refused(assets: &str, composition: &str, code: &str) {
    let got = codes("1.8", assets, composition);
    assert!(got.iter().any(|c| c == code), "{assets} {composition}: want {code}, got {got:?}");
}

#[test]
fn parts_animators_morphs_and_path_morphs_are_valid_at_1_8() {
    valid(
        FORMULAS,
        r##"<layer id="l" asset="f"><formulaAnimator part="rhs" y="-40" x="2" scale="1.5" scaleX="1" scaleY="1" rotation="10" opacity="0.5" fill="#FF0000FF" stroke="#00FF00FF" strokeWidth="1" blur="2" anchorX="0" anchorY="0"/><formulaAnimator part="lhs"><animate property="x"><key time="0" value="0"/><key time="1" value="20"/></animate></formulaAnimator></layer>"##,
    );
    valid(&morph(r#"from="f" to="g" width="64" height="32""#), r#"<layer id="l" asset="m"/>"#);
    valid(
        &morph(r##"from="f" to="g" match="tex" progress="0.5" width="64" height="32" size="24" color="#FFFF00FF""##),
        r#"<layer id="l" asset="m"/>"#,
    );
    valid(
        &format!(
            r#"{FORMULAS}<formulaMorph id="m" from="f" to="g" width="64" height="32"><animate property="progress"><key time="0" value="0"/><key time="1" value="1"/></animate></formulaMorph>"#
        ),
        r#"<layer id="l" asset="m"/>"#,
    );
    valid("", PATH_MORPH);
    // valid, and inert without path keys (INERT-I18, information)
    assert_eq!(codes("1.8", "", r#"<shape id="s" shape="rect" width="10" height="10" morph="false"/>"#), ["INERT-I18"]);
}

#[test]
fn malformed_values_are_structure_errors() {
    for bad in [r#"match="glyph""#, r#"progress="1.5""#, r#"progress="-0.1""#] {
        refused(&morph(&format!(r#"from="f" to="g" width="64" height="32" {bad}"#)), "", "S06");
    }
    refused(&morph(r#"from="f" to="g" width="0" height="32""#), "", "S06");
    refused(FORMULAS, r#"<layer id="l" asset="f"><formulaAnimator part="a b"/></layer>"#, "S06");
    refused(FORMULAS, r#"<layer id="l" asset="f"><formulaAnimator part="rhs" opacity="2"/></layer>"#, "S06");
    refused("", r#"<shape id="s" shape="rect" width="10" height="10" morph="yes"/>"#, "S06");
    // the text animator's other offsets are not the formula animator's
    refused(FORMULAS, r#"<layer id="l" asset="f"><formulaAnimator part="rhs" tracking="2"/></layer>"#, "S04");
    refused(FORMULAS, r#"<layer id="l" asset="f"><formulaAnimator/></layer>"#, "S05");
    refused(&morph(r#"from="f" width="64" height="32""#), "", "S05");
    refused(&morph(r#"from="f" to="nowhere" width="64" height="32""#), "", "S10");
}

#[test]
fn a_formula_animator_needs_a_formula_on_its_layer_frm1() {
    refused(FORMULAS, r#"<layer id="l" asset="t"><formulaAnimator part="rhs"/></layer>"#, "FRM1");
    // a morph is shown as a formula, but its parts are the two formulas'; the animator names one formula's part
    refused(
        &morph(r#"from="f" to="g" width="64" height="32""#),
        r#"<layer id="l" asset="m"><formulaAnimator part="rhs"/></layer>"#,
        "FRM1",
    );
}

#[test]
fn a_morph_goes_between_formulas_frm2() {
    refused(&morph(r#"from="t" to="g" width="64" height="32""#), "", "FRM2");
    refused(&morph(r#"from="f" to="t" width="64" height="32""#), "", "FRM2");
}

#[test]
fn before_1_8_the_draft_is_refused_v17() {
    let animated = r#"<layer id="l" asset="f"><formulaAnimator part="rhs"/></layer>"#;
    let morphed = morph(r#"from="f" to="g" width="64" height="32""#);
    for v in ["1.0", "1.3", "1.6", "1.7"] {
        for (assets, comp) in [
            (FORMULAS, animated),
            (morphed.as_str(), ""),
            ("", PATH_MORPH),
            ("", r#"<shape id="s" shape="rect" width="10" height="10" morph="false"/>"#),
            (r#"<formula id="f" tex="\part{a}{x}" width="64" height="32"/>"#, ""),
        ] {
            let got = codes(v, assets, comp);
            assert!(got.iter().any(|k| k == "V17"), "{v} {assets} {comp}: {got:?}");
        }
    }
    // an untagged formula and a path animation without morph are not gated
    let got = codes(
        "1.7",
        r#"<formula id="f" tex="\partial x" width="64" height="32"/>"#,
        r#"<shape id="s" shape="path" width="1" height="1" path="M 0 0 L 1 1"/>"#,
    );
    assert!(!got.iter().any(|k| k == "V17"), "{got:?}");
}
