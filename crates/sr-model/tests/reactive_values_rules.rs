//! The reactive-values draft (schema/UPSTREAM): the `value` asset, the animation children of a chart series, the
//! Schematron rules RV1 and RV2, and their clause of V17, the one version gate of the 1.8 drafts.

fn codes(version: &str, assets: &str) -> Vec<String> {
    let xml = format!(
        r#"<scene version="{version}"><project width="64" height="64" fps="30" duration="1"/><assets>{assets}</assets><composition/></scene>"#
    );
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
    sr_model::validate_str(&xml, &opts).diagnostics.into_iter().map(|d| d.code).collect()
}

#[track_caller]
fn valid(assets: &str) {
    let got = codes("1.8", assets);
    assert!(got.is_empty(), "{assets}: {got:?}");
}

#[track_caller]
fn refused(assets: &str, code: &str) {
    let got = codes("1.8", assets);
    assert!(got.iter().any(|c| c == code), "{assets}: want {code}, got {got:?}");
}

#[test]
fn values_folds_and_linked_series_are_valid_at_1_8() {
    valid(r#"<value id="a"/>"#);
    valid(r#"<value id="a" value="1 2 3"/>"#);
    valid(
        r#"<value id="a"><animate property="value"><key time="0" value="0"/><key time="1" value="4"/></animate></value>"#,
    );
    valid(r#"<value id="a" value="0" step="0.25" update="value + 2 * dt"/>"#);
    valid(r#"<value id="a" start="0.5" step="0.1" update="value + index" between="linear" seed="7"/>"#);
    valid(r#"<value id="a" count="400" update="value + random() / count"/>"#);
    valid(
        r#"<value id="n" step="1" update="value + 1"/><chart id="c" kind="counter" width="64" height="32"><series name="n" values="0"><link property="values" source="n.value"/></series></chart>"#,
    );
}

#[test]
fn malformed_values_are_structure_errors() {
    for bad in [
        r#"<value id="a" step="0" update="value"/>"#,
        r#"<value id="a" step="-1" update="value"/>"#,
        r#"<value id="a" count="0" update="value"/>"#,
        r#"<value id="a" count="1000001" update="value"/>"#,
        r#"<value id="a" start="-1"/>"#,
        r#"<value id="a" between="cubic" step="1" update="value"/>"#,
        r#"<value id="a" value="x"/>"#,
    ] {
        refused(bad, "S06");
    }
    refused(r#"<value/>"#, "S05");
    refused(r#"<value id="a" width="3"/>"#, "S04");
}

#[test]
fn an_update_goes_with_exactly_one_of_step_and_count_rv1() {
    refused(r#"<value id="a" update="value + 1"/>"#, "RV1");
    refused(r#"<value id="a" step="1"/>"#, "RV1");
    refused(r#"<value id="a" count="3"/>"#, "RV1");
    refused(r#"<value id="a" step="1" count="3" update="value"/>"#, "RV1");
}

#[test]
fn between_applies_to_a_time_fold_rv2() {
    refused(r#"<value id="a" between="linear"/>"#, "RV2");
    refused(r#"<value id="a" count="3" update="value" between="linear"/>"#, "RV2");
    valid(r#"<value id="a" count="3" update="value" between="hold"/>"#);
}

#[test]
fn before_1_8_values_are_refused_v17() {
    for v in ["1.0", "1.3", "1.6", "1.7"] {
        let got = codes(v, r#"<value id="a"/>"#);
        assert!(got.iter().any(|k| k == "V17"), "{v}: {got:?}");
    }
    // a series without children is not gated
    let got =
        codes("1.7", r#"<chart id="c" kind="counter" width="64" height="32"><series name="n" values="0"/></chart>"#);
    assert!(!got.iter().any(|k| k == "V17"), "{got:?}");
}
