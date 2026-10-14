//! SREP 18: the render report's codes, order, placement and determinism, without rendering.

use sr_deliver::render_report::{code, error_findings, report_code, Finding, OutputInfo, ReportWriter};
use sr_model::{Diagnostic, Loc, Severity};

const DOC: &str = r##"<scene version="1.2"><project width="64" height="48" fps="10" duration="2"/>
<assets><image id="img" src="a.png" width="4" height="4"/></assets>
<composition><group id="g"><shape id="a" shape="rect" width="4" height="4"/><shape id="b" shape="rect" width="4" height="4"/></group></composition></scene>"##;

fn output() -> OutputInfo {
    OutputInfo { id: Some("out".into()), width: 64, height: 48, start: 0.0, end: 2.0 }
}

fn offset_of(needle: &str) -> u32 {
    DOC.find(needle).unwrap() as u32
}

fn diag(code: &str, severity: Severity, path: &str, offset: u32) -> Diagnostic {
    let loc = Loc { line: 1, column: 1, offset };
    Diagnostic { severity, ..Diagnostic::error(code, format!("{code} message"), loc, path) }
}

#[test]
fn diagnostic_codes_map_to_the_registry() {
    let d = |c: &str, s: Severity| diag(c, s, "/scene", 0);
    assert_eq!(report_code(&d("S04", Severity::Error)), "XSD");
    assert_eq!(report_code(&d("XML", Severity::Error)), "XSD");
    assert_eq!(report_code(&d("C21", Severity::Error)), "SCH-C21");
    assert_eq!(report_code(&d("A01", Severity::Error)), "ASSET-MISSING");
    assert_eq!(report_code(&d("A02", Severity::Error)), "ASSET-MISSING");
    assert_eq!(report_code(&d("A04", Severity::Error)), "ASSET-MISSING");
    // a sequence that holds its missing frames is the engine's own warning
    assert_eq!(report_code(&d("A04", Severity::Warning)), "X-rs-scene-render-A04");
    assert_eq!(report_code(&d("SA01", Severity::Warning)), "SAFE-AREA");
    assert_eq!(report_code(&d("INERT-I2", Severity::Info)), "INERT-I2");
    assert_eq!(report_code(&d("E19", Severity::Info)), "X-rs-scene-render-E19");
    assert_eq!(report_code(&d("INERT-I13", Severity::Info)), "INERT-I13");
    assert_eq!(report_code(&d("MASK-MISS", Severity::Warning)), "MASK-MISS");
    // SREP 72 adds PREC1 to the registry (information: f32 working textures, 3D internal buffers stay f16)
    assert_eq!(report_code(&d("PREC1", Severity::Info)), "PREC1");
    assert_eq!(code::engine("RENDER"), "X-rs-scene-render-RENDER");
}

#[test]
fn findings_are_placed_by_path_offset_or_id() {
    let w = ReportWriter::new(DOC.as_bytes(), std::path::Path::new("."));
    let by_path = Finding::of_diagnostic(&diag(
        "INERT-I2",
        Severity::Info,
        "/scene/composition/group/shape[2]",
        offset_of(r#"<shape id="b""#),
    ));
    // the evaluator names nodes by id
    let by_id = Finding::of_diagnostic(&diag("INERT-I8", Severity::Info, "a", offset_of(r#"<shape id="a""#)));
    let unknown = Finding::node("TXT-FIT", Severity::Warning, "nowhere", "m");
    let r = w.report(output(), vec![by_path, by_id, unknown]);
    let placed: Vec<(&str, Option<&str>)> = r.findings.iter().map(|f| (f.path.as_str(), f.node.as_deref())).collect();
    assert_eq!(
        placed,
        [
            ("/scene/composition/group/shape[2]", Some("b")),
            ("/scene/composition/group/shape[1]", Some("a")),
            ("/scene", None)
        ]
    );
}

#[test]
fn findings_are_sorted_by_time_then_code_then_path() {
    let w = ReportWriter::new(DOC.as_bytes(), std::path::Path::new("."));
    let f = |c: &str, id: &str| Finding::node(c, Severity::Warning, id, "m");
    let r = w.report(
        output(),
        vec![
            f("TXT-FIT", "b").at_time(1.0, 1.5),
            f("TXT-CUT", "a").at_time(1.0, 1.0),
            f("SAFE-AREA", "a").at_time(0.5, 0.5),
            f("INERT-I2", "b"),
            f("INERT-I2", "a"),
            f("ACC-CAPTIONS", "g"),
            // an exact duplicate is listed once
            f("ACC-CAPTIONS", "g"),
        ],
    );
    let got: Vec<(String, String)> = r.findings.iter().map(|f| (f.code.clone(), f.node.clone().unwrap())).collect();
    let want = [
        ("ACC-CAPTIONS", "g"),
        ("INERT-I2", "a"),
        ("INERT-I2", "b"),
        ("SAFE-AREA", "a"),
        ("TXT-CUT", "a"),
        ("TXT-FIT", "b"),
    ];
    assert_eq!(got, want.map(|(a, b)| (a.to_string(), b.to_string())));
}

#[test]
fn the_report_serialises_the_fields_of_the_format() {
    let w = ReportWriter::new(DOC.as_bytes(), std::path::Path::new("."));
    let f =
        Finding::node("TXT-FIT", Severity::Warning, "a", "too tall").at_time(0.5, 1.0).measuring(12.5, Some(0.0), "px");
    let v = serde_json::to_value(w.report(output(), vec![f])).unwrap();
    assert_eq!(v["format"], "scene-render-report/1");
    assert_eq!(v["engine"]["name"], "rs-scene-render");
    assert_eq!(v["scene"]["version"], "1.2");
    assert_eq!(v["output"], serde_json::json!({"id": "out", "width": 64, "height": 48, "start": 0.0, "end": 2.0}));
    assert_eq!(
        v["findings"][0],
        serde_json::json!({"code": "TXT-FIT", "severity": "warning", "path": "/scene/composition/group/shape[1]",
            "node": "a", "time": [0.5, 1.0], "measured": 12.5, "limit": 0.0, "unit": "px", "message": "too tall"})
    );
    // the same input gives the same bytes
    let a = serde_json::to_string(&w.report(output(), vec![])).unwrap();
    let b = serde_json::to_string(&w.report(output(), vec![])).unwrap();
    assert_eq!(a, b);
}

#[test]
fn messages_keep_no_path_outside_the_document_folder() {
    let root =
        std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("sr-report-clean-{}", std::process::id()));
    let folder = root.join("scene");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(root.join("shared.png"), b"x").unwrap();
    let folder = std::fs::canonicalize(&folder).unwrap();
    let inside = folder.join("a.png");
    let outside = std::fs::canonicalize(root.join("shared.png")).unwrap();
    let msg = format!(
        "{} does not exist; \"{}\" differs; /scene/composition/layer[2] is fine",
        inside.display(),
        outside.display()
    );
    let w = ReportWriter::new(DOC.as_bytes(), &folder);
    let r = w.report(output(), vec![Finding::scene("ASSET-MISSING", Severity::Error, msg)]);
    let m = &r.findings[0].message;
    assert!(m.starts_with("a.png does not exist"), "{m}");
    assert!(!m.contains(&root.display().to_string()), "{m}");
    assert!(m.contains("…/shared.png"), "{m}");
    assert!(m.contains("/scene/composition/layer[2]"), "{m}");
}

#[test]
fn a_failed_delivery_reports_why() {
    let caps = error_findings(&sr_deliver::DeliverError::Accessibility(
        "requireCaptions: the output has no caption track".into(),
    ));
    assert_eq!(caps.len(), 1);
    assert_eq!((caps[0].code.as_str(), caps[0].severity), ("ACC-CAPTIONS", Severity::Error));
    let render = error_findings(&sr_deliver::DeliverError::Render { time: 1.5, message: "bad frame".into() });
    assert_eq!(render[0].code, "X-rs-scene-render-RENDER");
    assert_eq!(render[0].time, Some([1.5, 1.5]));
    // SREP 72, Device features: f32 on a device without float32 blending or filtering is the error PREC10
    let prec10 = error_findings(&sr_deliver::DeliverError::Render {
        time: 0.0,
        message: "PREC10: precision=\"f32\" needs ...; the adapter llvmpipe lacks FLOAT32_BLENDABLE".into(),
    });
    assert_eq!((prec10[0].code.as_str(), prec10[0].severity), ("PREC10", Severity::Error));
    assert!(prec10[0].message.contains("FLOAT32_BLENDABLE"), "{}", prec10[0].message);
    // an output layer renders the same document, precision included, and prefixes its frame's errors
    let layer = error_findings(&sr_deliver::DeliverError::Render {
        time: 0.5,
        message: "output layer: PREC10: precision=\"f32\" needs ...; the adapter llvmpipe lacks FLOAT32_FILTERABLE"
            .into(),
    });
    assert_eq!((layer[0].code.as_str(), layer[0].severity), ("PREC10", Severity::Error));
    assert!(layer[0].message.starts_with("output layer: "), "{}", layer[0].message);
    // the code is read at the start of the message only, not from a later error that quotes it
    let later = error_findings(&sr_deliver::DeliverError::Render {
        time: 0.5,
        message: "output layer: shader failed; PREC10: precision=\"f32\" ...".into(),
    });
    assert_eq!(later[0].code, "X-rs-scene-render-RENDER");
    // a check set to error is already a finding of the delivery
    assert!(error_findings(&sr_deliver::DeliverError::Accessibility("contrastCheck: …".into())).is_empty());
}

#[test]
fn a_document_that_cannot_load_still_names_its_outputs() {
    let doc = r#"<scene version="1.2"><project width="64" height="48" fps="10" duration="2"/><output id="m" path="m.mp4" report="m.report.json" height="720"/><composition/></scene>"#;
    let w = ReportWriter::new(doc.as_bytes(), std::path::Path::new("."));
    let outs = w.raw_outputs();
    assert_eq!(outs.len(), 1);
    assert_eq!(outs[0].report.as_deref(), Some("m.report.json"));
    assert_eq!(outs[0].info, OutputInfo { id: Some("m".into()), width: 64, height: 720, start: 0.0, end: 2.0 });
}

#[test]
fn a4_finding_name_is_additive_and_id_stays_addressable() {
    let xml = DOC.replace("id=\"a\"", "id=\"a\" name=\" Title \"");
    let w = ReportWriter::new(xml.as_bytes(), std::path::Path::new("."));
    let r = w.report(output(), vec![Finding::node("TXT-FIT", Severity::Warning, "a", "m")]);
    assert_eq!(r.findings[0].node.as_deref(), Some("a"));
    assert_eq!(r.findings[0].node_name.as_deref(), Some("Title"));
}

const CREDITS_DOC: &str = r##"<scene version="1.2"><project width="64" height="48" fps="30000/1001" duration="2"/>
<assets>
<image id="logo" src="a.png" width="4" height="4" license="CC-BY-4.0" credit="Ada Lovelace"/>
<image id="plain" src="b.png" width="4" height="4"/>
<video id="clip" src="c.mp4" width="64" height="48" fps="30000/1001" duration="10" timecodeStart="01:00:00;02" license="Editorial"/>
<audio id="music" src="m.wav" credit="The Band"/>
</assets>
<composition><layer id="v" asset="clip" clipIn="1" clipOut="2"/><layer id="whole" asset="clip"/><layer id="p" asset="plain"/></composition></scene>"##;

#[test]
fn licence_and_credit_of_every_asset_travel_in_the_report() {
    let w = ReportWriter::new(CREDITS_DOC.as_bytes(), std::path::Path::new("."));
    let v = serde_json::to_value(w.report(output(), vec![])).unwrap();
    let assets = v["assets"].as_array().expect("an assets array");
    // document order; an asset with neither attribute is not listed
    let ids: Vec<&str> = assets.iter().map(|a| a["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["logo", "clip", "music"], "{v}");
    assert_eq!(assets[0]["kind"], "image");
    assert_eq!(assets[0]["license"], "CC-BY-4.0");
    assert_eq!(assets[0]["credit"], "Ada Lovelace");
    assert_eq!(assets[1]["license"], "Editorial");
    assert!(assets[1].get("credit").is_none(), "an absent attribute leaves no key: {}", assets[1]);
    assert_eq!(assets[2]["kind"], "audio");
    assert_eq!(assets[2]["credit"], "The Band");
}

#[test]
fn video_layers_report_their_source_timecodes() {
    let w = ReportWriter::new(CREDITS_DOC.as_bytes(), std::path::Path::new("."));
    let v = serde_json::to_value(w.report(output(), vec![])).unwrap();
    let sources = v["sources"].as_array().expect("a sources array");
    assert_eq!(sources.len(), 2, "only layers of a video asset with a timecodeStart: {v}");
    // 01:00:00;02 at 29.97 fps (drop frame) plus clipIn 1 s (30 frames) and clipOut 2 s (60 frames)
    assert_eq!(sources[0]["layer"], "v");
    assert_eq!(sources[0]["asset"], "clip");
    assert_eq!(sources[0]["fps"], "30000/1001");
    assert_eq!(sources[0]["in"], "01:00:01;02");
    assert_eq!(sources[0]["out"], "01:00:02;02");
    // no clipOut: the asset's whole duration (10 s = 300 frames)
    assert_eq!(sources[1]["layer"], "whole");
    assert_eq!(sources[1]["in"], "01:00:00;02");
    assert_eq!(sources[1]["out"], "01:00:10;02");
}

#[test]
fn a_document_without_credits_or_source_timecodes_has_neither_key() {
    let w = ReportWriter::new(DOC.as_bytes(), std::path::Path::new("."));
    let v = serde_json::to_value(w.report(output(), vec![])).unwrap();
    assert!(v.get("assets").is_none() && v.get("sources").is_none(), "{v}");
}

#[test]
fn generated_asset_licence_is_reported_too() {
    let xml = CREDITS_DOC.replace("</assets>", r#"<generated id="speech" kind="speech" provider="p" model="m" cache="speech.wav" cacheSha256="0000000000000000000000000000000000000000000000000000000000000000" license="Provider terms"/></assets>"#);
    let w = ReportWriter::new(xml.as_bytes(), std::path::Path::new("."));
    let v = serde_json::to_value(w.report(output(), vec![])).unwrap();
    assert_eq!(v["assets"][3]["id"], "speech");
    assert_eq!(v["assets"][3]["license"], "Provider terms");
}

#[test]
fn source_timecodes_use_the_asset_rate_and_signed_clip_offsets() {
    let xml = CREDITS_DOC
        .replace(
            "fps=\"30000/1001\" duration=\"10\" timecodeStart=\"01:00:00;02\"",
            "fps=\"25\" duration=\"10\" timecodeStart=\"01:00:00:00\"",
        )
        .replace("clipIn=\"1\" clipOut=\"2\"", "clipIn=\"-1\" clipOut=\"0.5\"");
    let w = ReportWriter::new(xml.as_bytes(), std::path::Path::new("."));
    let v = serde_json::to_value(w.report(output(), vec![])).unwrap();
    assert_eq!(v["sources"][0]["fps"], "25");
    assert_eq!(v["sources"][0]["in"], "00:59:59:00");
    assert_eq!(v["sources"][0]["out"], "01:00:00:13");
}

#[test]
fn invalid_source_timecodes_are_reported_instead_of_invented() {
    let xml = CREDITS_DOC.replace("01:00:00;02", "00:01:00;00"); // skipped drop-frame label
    let w = ReportWriter::new(xml.as_bytes(), std::path::Path::new("."));
    let v = serde_json::to_value(w.report(output(), vec![])).unwrap();
    assert!(v.get("sources").is_none());
    assert_eq!(v["findings"][0]["code"], "X-rs-scene-render-TIMECODE");
    assert_eq!(v["findings"][0]["severity"], "warning");
}
