use sr_model::{diag, LoadOptions};

#[test]
fn a4_label_fallback_and_pin_diagnostics() {
    assert_eq!(diag::label(Some(" Title "), Some("n"), "layer", 2), "Title");
    assert_eq!(diag::label(Some(" "), Some("n"), "layer", 2), "n");
    assert_eq!(diag::label(None, None, "pin", 2), "pin[3]");
    for (name, label) in [(" name=\"Elbow\"", "Elbow"), ("", "pin[1]")] {
        let xml = format!(
            r#"<scene version="1.2"><project width="64" height="64" fps="24" duration="1"/><composition><shape id="s" shape="rect" width="20" height="20"><deform><modifier type="puppet"><pin{name} restX="0" restY="0" kind="invalid"/></modifier></deform></shape></composition></scene>"#
        );
        let report = sr_model::validate_str(&xml, &LoadOptions::without_assets());
        assert!(report.diagnostics.iter().any(|d| d.to_string().contains(label)), "{report:?}");
    }
}

#[test]
fn a4_symbol_and_animator_diagnostics_preserve_locators() {
    for (element, label) in [
        (r#"<symbol id="sym" name="Lower third" invalid="x"/>"#, "Lower third"),
        (r#"<symbol id="sym" invalid="x"/>"#, "sym"),
    ] {
        let xml = format!(
            r#"<scene version="1.2"><project width="64" height="64" fps="24" duration="1"/><symbols>{element}</symbols><composition/></scene>"#
        );
        let report = sr_model::validate_str(&xml, &LoadOptions::without_assets());
        assert!(
            report.diagnostics.iter().any(|d| d.label.as_deref() == Some(label) && d.path.contains("symbol")),
            "{report:?}"
        );
    }
}

#[test]
fn a4_xml_and_typed_label_indices_agree() {
    let doc = roxmltree::Document::parse("<root><pin/><other/><pin/></root>").unwrap();
    let labels: Vec<_> = doc.descendants().filter(|n| n.has_tag_name("pin")).map(diag::node_label).collect();
    assert_eq!(labels, ["pin[1]", "pin[2]"]);
}

#[test]
fn a4_animator_diagnostics_use_name_then_index() {
    for (name, label) in [(" name=\"Entrance\"", "Entrance"), ("", "textAnimator[1]")] {
        let xml = format!(
            r#"<scene version="1.2"><project width="64" height="64" fps="24" duration="1"/><assets><text id="t" width="64" height="64" text="M"/></assets><composition><layer id="l" asset="t"><textAnimator{name} selector="invalid"/></layer></composition></scene>"#
        );
        let report = sr_model::validate_str(&xml, &LoadOptions::without_assets());
        assert!(report.diagnostics.iter().any(|d| d.label.as_deref() == Some(label)), "{report:?}");
    }
}
