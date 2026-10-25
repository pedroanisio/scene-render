//! SREP 18, Specification 5: inert attributes I1 to I7 are found by validation, at information severity, on the
//! element that carries them, and nowhere else.

use sr_model::{validate_str, Diagnostic, LoadOptions, Severity};

fn findings(assets: &str, composition: &str) -> Vec<Diagnostic> {
    let xml = format!(
        r##"<scene version="1.2"><project width="64" height="64" fps="24" duration="2"/><styles><token name="clear" value="#00000000"/><token name="red" value="#FF0000"/></styles><assets>{assets}</assets><composition>{composition}</composition></scene>"##
    );
    let report = validate_str(&xml, &LoadOptions::without_assets());
    assert!(!report.has_errors(), "the document must be valid:\n{report}");
    report.diagnostics.into_iter().filter(|d| d.code.starts_with("INERT-")).collect()
}

fn codes(assets: &str, composition: &str) -> Vec<String> {
    findings(assets, composition).into_iter().map(|d| d.code).collect()
}

fn shape(attrs: &str) -> String {
    format!(r#"<shape id="s" shape="rect" width="10" height="10" {attrs}/>"#)
}

#[test]
fn every_finding_is_information_on_the_element_that_carries_it() {
    let f = findings("", &format!(r#"<group id="g">{}</group>"#, shape(r#"strokeWidth="2""#)));
    assert_eq!(f.len(), 1, "{f:?}");
    assert_eq!(f[0].code, "INERT-I2");
    assert_eq!(f[0].severity, Severity::Info);
    assert_eq!(f[0].path, "/scene/composition/group/shape");
    assert!(f[0].message.contains("strokeWidth 2"), "{}", f[0].message);
}

#[test]
fn i1_a_stroke_paint_without_a_width() {
    assert_eq!(codes("", &shape(r##"stroke="#FF0000""##)), ["INERT-I1"]);
    assert_eq!(codes("", &shape(r##"stroke="#FF0000" strokeWidth="0""##)), ["INERT-I1"]);
    assert_eq!(codes("", &shape(r#"stroke="var(--red)""#)), ["INERT-I1"]);
    // drawn: nothing to report
    assert!(codes("", &shape(r##"stroke="#FF0000" strokeWidth="1""##)).is_empty());
    // a transparent paint without a width is the default: nothing is drawn and nothing was asked for
    assert!(codes("", &shape(r##"stroke="#00000000""##)).is_empty());
}

#[test]
fn i2_a_width_without_a_stroke_paint() {
    assert_eq!(codes("", &shape(r#"strokeWidth="2""#)), ["INERT-I2"]);
    assert_eq!(codes("", &shape(r##"strokeWidth="2" stroke="#FF000000""##)), ["INERT-I2"]);
    assert_eq!(codes("", &shape(r#"strokeWidth="2" stroke="var(--clear)""#)), ["INERT-I2"]);
    // a paint reference draws
    let paints = r##"<paints><linearGradient id="lg"><stop offset="0" color="#FF0000"/><stop offset="1" color="#0000FF"/></linearGradient></paints>"##;
    let xml = format!(
        r#"<scene version="1.2"><project width="64" height="64" fps="24" duration="2"/>{paints}<composition>{}</composition></scene>"#,
        shape(r#"strokeWidth="2" stroke="url(#lg)""#)
    );
    let r = validate_str(&xml, &LoadOptions::without_assets());
    assert!(!r.has_errors(), "{r}");
    assert!(r.diagnostics.iter().all(|d| !d.code.starts_with("INERT-")), "{r}");
}

#[test]
fn i3_stroke_styling_on_an_element_that_draws_no_stroke() {
    assert_eq!(codes("", &shape(r#"strokeCap="round" dash="4 2""#)), ["INERT-I3"]);
    let f = findings("", &shape(r#"strokeJoin="round" miterLimit="2" dashOffset="1""#));
    assert_eq!(f.len(), 1, "one finding per element: {f:?}");
    for a in ["@strokeJoin", "@miterLimit", "@dashOffset"] {
        assert!(f[0].message.contains(a), "{}", f[0].message);
    }
    // with I2 when the width has no paint
    assert_eq!(codes("", &shape(r#"strokeWidth="3" strokeCap="round""#)), ["INERT-I2", "INERT-I3"]);
    assert!(codes("", &shape(r##"stroke="#FFFFFF" strokeWidth="3" strokeCap="round" dash="2 2""##)).is_empty());
}

#[test]
fn i4_star_parameters_on_another_shape_kind() {
    assert_eq!(codes("", &shape(r#"points="6""#)), ["INERT-I4"]);
    assert_eq!(codes("", &shape(r#"innerRadius="2" outerRoundness="0.5""#)), ["INERT-I4"]);
    let star = r#"<shape id="st" shape="star" width="10" height="10" points="6" innerRadius="2" outerRadius="5" innerRoundness="1" outerRoundness="1"/>"#;
    assert!(codes("", star).is_empty());
    let poly = r#"<shape id="p" shape="polygon" width="10" height="10" points="6" outerRadius="5"/>"#;
    assert!(codes("", poly).is_empty());
}

#[test]
fn i5_path_data_on_a_shape_other_than_path() {
    assert_eq!(codes("", &shape(r#"path="M0 0 L10 10""#)), ["INERT-I5"]);
    let path = r#"<shape id="p" shape="path" width="10" height="10" path="M0 0 L10 10"/>"#;
    assert!(codes("", path).is_empty());
}

#[test]
fn i6_a_matte_mode_without_a_matte() {
    let img = r#"<image id="img" src="a.png" width="10" height="10"/>"#;
    assert_eq!(codes(img, r#"<layer id="l" asset="img" matteMode="luma"/>"#), ["INERT-I6"]);
    assert_eq!(codes(img, r#"<layer id="l" asset="img" matteVisible="true"/>"#), ["INERT-I6"]);
    let with_matte = r#"<layer id="m" asset="img"/><layer id="l" asset="img" matte="m" matteMode="luma"/>"#;
    assert!(codes(img, with_matte).is_empty());
}

#[test]
fn i7_audio_attributes_on_a_layer_without_sound() {
    let img = r#"<image id="img" src="a.png" width="10" height="10"/>"#;
    assert_eq!(codes(img, r#"<layer id="l" asset="img" volume="0.5"/>"#), ["INERT-I7"]);
    let f = findings(img, r#"<layer id="l" asset="img" volume="0.5" mute="true"/>"#);
    assert_eq!(f.len(), 1, "{f:?}");
    let video = r#"<video id="v" src="a.mp4" width="10" height="10" fps="10" duration="1"/>"#;
    assert!(codes(video, r#"<layer id="l" asset="v" volume="0.5"/>"#).is_empty());
}

#[test]
fn an_attribute_that_can_change_is_never_inert() {
    // animated width: the stroke draws once the width grows
    let animated = r##"<shape id="s" shape="rect" width="10" height="10" stroke="#FF0000"><animate property="strokeWidth"><key time="0" value="0"/><key time="1" value="4"/></animate></shape>"##;
    assert!(codes("", animated).is_empty());
    // a variant can give the shape a stroke
    let xml = format!(
        r##"<scene version="1.2"><project width="64" height="64" fps="24" duration="2"/><parameters><variant id="v"><override target="s" property="stroke" value="#FF0000"/></variant></parameters><composition>{}</composition></scene>"##,
        shape(r#"strokeWidth="2""#)
    );
    let r = validate_str(&xml, &LoadOptions::without_assets());
    assert!(!r.has_errors(), "{r}");
    assert!(r.diagnostics.iter().all(|d| !d.code.starts_with("INERT-")), "{r}");
}

#[test]
fn inert_findings_never_make_a_document_fail() {
    let xml = format!(
        r#"<scene version="1.2"><project width="64" height="64" fps="24" duration="2"/><composition>{}</composition></scene>"#,
        shape(r#"strokeWidth="2" points="5" path="M0 0""#)
    );
    let doc = sr_model::load_str(&xml, &LoadOptions::without_assets()).expect("inert attributes load");
    assert_eq!(doc.warnings().iter().filter(|d| d.is_info()).count(), 3);
}

#[test]
fn every_rule_is_explained() {
    for k in 1..=8 {
        let code = format!("INERT-I{k}");
        let c = sr_model::codes::lookup(&code).unwrap_or_else(|| panic!("{code} is not explained"));
        assert_eq!(c.stage, "inert");
    }
}

#[test]
fn a6_i17_node_switches_without_their_switch() {
    // P2-08: zDepth, rotationX and rotationY place a node in camera space only with threeD="true"
    assert_eq!(codes("", &shape(r#"zDepth="5""#)), ["INERT-I17"]);
    assert_eq!(codes("", &shape(r#"zDepth="5" rotationX="10" rotationY="20""#)), ["INERT-I17"], "once per element");
    assert!(codes("", &shape(r#"threeD="true" zDepth="5" rotationX="10""#)).is_empty());
    // the value can change: an animation drives the property or the switch
    let animated = r#"<shape id="s" shape="rect" width="10" height="10" zDepth="5"><animate property="zDepth"><key time="0" value="0"/><key time="1" value="9"/></animate></shape>"#;
    assert!(codes("", animated).is_empty());
    let switched = r#"<shape id="s" shape="rect" width="10" height="10" zDepth="5"><animate property="threeD"><key time="0" value="0"/></animate></shape>"#;
    assert!(codes("", switched).is_empty());
    // an object3D's rotationX is its own attribute, not the 2.5D one
    assert!(codes("", r#"<object3D id="o" primitive="box" rotationX="30"/>"#).is_empty());
}

#[test]
fn a6_i17_margin_and_align_to_need_an_alignment() {
    // P2-09: margin and alignTo apply with alignX or alignY only
    assert_eq!(codes("", &shape(r#"margin="8""#)), ["INERT-I17"]);
    assert_eq!(codes("", &shape(r#"margin="8" alignTo="frame""#)), ["INERT-I17"]);
    assert!(codes("", &shape(r#"margin="8" alignX="left""#)).is_empty());
    assert!(codes("", &shape(r#"alignTo="frame" alignY="top""#)).is_empty());
    // centring ignores the margin, which is documented, and is not reported
    assert!(codes("", &shape(r#"margin="8" alignX="center" alignY="middle""#)).is_empty());
}

#[test]
fn i18_morph_without_path_keys() {
    // the semantic-tex draft: shape/@morph is read only by the keys of an animate property="path"
    let at_1_8 = |shape: &str| -> Vec<String> {
        let xml = format!(
            r#"<scene version="1.8"><project width="64" height="64" fps="24" duration="2"/><composition>{shape}</composition></scene>"#
        );
        let report = validate_str(&xml, &LoadOptions::without_assets());
        assert!(!report.has_errors(), "the document must be valid:\n{report}");
        report.diagnostics.into_iter().filter(|d| d.code.starts_with("INERT-")).map(|d| d.code).collect()
    };
    let keys = r#"<animate property="path"><key time="0" value="M 0 0 L 9 0 L 9 9 Z"/><key time="1" value="M 0 0 L 9 9 L 0 9 Z"/></animate>"#;
    let path = |attrs: &str, kids: &str| {
        format!(
            r#"<shape id="s" shape="path" width="10" height="10" path="M 0 0 L 9 0 L 9 9 Z" {attrs}>{kids}</shape>"#
        )
    };
    assert_eq!(at_1_8(&path(r#"morph="true""#, "")), ["INERT-I18"]);
    assert_eq!(at_1_8(&path(r#"morph="false""#, "")), ["INERT-I18"]);
    // another property's keys do not make it read
    assert_eq!(
        at_1_8(&path(
            r#"morph="true""#,
            r#"<animate property="x"><key time="0" value="0"/><key time="1" value="5"/></animate>"#
        )),
        ["INERT-I18"]
    );
    assert!(at_1_8(&path(r#"morph="true""#, keys)).is_empty());
    assert!(at_1_8(&path("", "")).is_empty());
}
