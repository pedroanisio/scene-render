//! W15: a path-traced camera whose frame costs far more than the defaults. The work of a frame grows with its pixels,
//! its samples per pixel and its bounces, and the warning names it before a render is started, not after it has run
//! for hours.

const SCENE: &str = r##"<scene version="1.7"><project width="1920" height="1080" fps="24" duration="1"/><composition>
  <camera id="eye" x="0" y="0" z="-10" renderer="pathtrace"/>
  <object3D id="ball" primitive="sphere" radius="1"/>
</composition></scene>"##;

fn codes(xml: &str) -> Vec<String> {
    sr_model::validate_str(xml, &sr_model::LoadOptions::without_assets())
        .diagnostics
        .into_iter()
        .map(|d| d.code)
        .collect()
}

fn with(camera: &str) -> String {
    SCENE.replace(r#"renderer="pathtrace""#, &format!(r#"renderer="pathtrace" {camera}"#))
}

#[test]
fn the_defaults_at_1080p_are_not_warned() {
    assert!(codes(SCENE).is_empty(), "{:?}", codes(SCENE));
}

#[test]
fn many_samples_and_bounces_at_1080p_are_warned_as_w15() {
    assert_eq!(codes(&with(r#"pathSamples="320" maxBounces="20""#)), ["W15"]);
    // 256 samples at 4 bounces is the limit itself, and one sample more is past it
    assert!(codes(&with(r#"pathSamples="256""#)).is_empty());
    assert_eq!(codes(&with(r#"pathSamples="257""#)), ["W15"]);
    assert_eq!(codes(&with(r#"maxBounces="17""#)), ["W15"]);
}

#[test]
fn the_bounces_count_as_much_as_the_samples() {
    // 16 samples at the most bounces the schema admits, 64, is the limit itself
    assert!(codes(&with(r#"pathSamples="16" maxBounces="64""#)).is_empty());
    assert_eq!(codes(&with(r#"pathSamples="17" maxBounces="64""#)), ["W15"]);
}

#[test]
fn a_smaller_frame_takes_more_samples() {
    let small = with(r#"pathSamples="1000" maxBounces="4""#)
        .replace(r#"width="1920" height="1080""#, r#"width="960" height="540""#);
    assert!(codes(&small).is_empty(), "{:?}", codes(&small));
}

#[test]
fn a_camera_in_a_viewport_is_costed_at_the_viewport_size() {
    let xml = r##"<scene version="1.7"><project width="1920" height="1080" fps="24" duration="1"/><composition>
  <viewport3D id="vp" x="0" y="0" width="480" height="270" camera="eye">
    <camera id="eye" x="0" y="0" z="-10" renderer="pathtrace" pathSamples="2048"/>
    <object3D id="ball" primitive="sphere" radius="1"/>
  </viewport3D>
</composition></scene>"##;
    assert!(codes(xml).is_empty(), "{:?}", codes(xml));
    assert_eq!(codes(&xml.replace(r#"pathSamples="2048""#, r#"pathSamples="5000""#)), ["W15"]);
}

#[test]
fn a_raster_camera_is_not_costed() {
    let xml = SCENE.replace(r#" renderer="pathtrace""#, r#" pathSamples="5000" maxBounces="20""#);
    assert!(codes(&xml).is_empty(), "{:?}", codes(&xml));
}

#[test]
fn the_message_names_the_cost_and_what_lowers_it() {
    let report =
        sr_model::validate_str(&with(r#"pathSamples="320" maxBounces="20""#), &sr_model::LoadOptions::without_assets());
    let w15 = report.diagnostics.iter().find(|d| d.code == "W15").expect("W15");
    for word in ["pathSamples", "maxBounces", "1920", "1080"] {
        assert!(w15.message.contains(word), "{word} is not in: {}", w15.message);
    }
}

/// The cost W15 weighs is also reported per camera (`inspect` lists it), with the size, samples and bounces it is made
/// of, so the work of a frame is known before it is rendered.
#[test]
fn every_path_traced_camera_has_its_cost_listed() {
    let xml = r##"<scene version="1.7"><project width="1920" height="1080" fps="24" duration="1"/><composition>
  <camera id="eye" x="0" y="0" z="-10" renderer="pathtrace" pathSamples="320" maxBounces="20"/>
  <camera id="flat" x="0" y="0" z="-10"/>
  <viewport3D id="vp" x="0" y="0" width="480" height="270" camera="small">
    <camera id="small" x="0" y="0" z="-10" renderer="pathtrace"/>
    <object3D id="ball" primitive="sphere" radius="1"/>
  </viewport3D>
</composition></scene>"##;
    let costs = sr_model::rules::path_costs(xml).unwrap();
    assert_eq!(costs.len(), 2, "the raster camera has no path cost: {costs:?}");
    let eye = &costs[0];
    assert_eq!((eye.camera.as_str(), eye.width, eye.height, eye.samples, eye.bounces), ("eye", 1920, 1080, 320, 20));
    assert_eq!(eye.segments, 1920 * 1080 * 320 * 20);
    assert!(eye.over_limit());
    let small = &costs[1];
    assert_eq!(
        (small.camera.as_str(), small.width, small.height, small.samples, small.bounces),
        ("small", 480, 270, 64, 4)
    );
    assert!(!small.over_limit());
    assert_eq!(sr_model::rules::PATH_COST_LIMIT, 1920 * 1080 * 256 * 4);
    assert!(sr_model::rules::path_costs("<scene").is_err(), "a document that does not parse is an error");
    // values past the renderer's range are costed at what it clamps them to
    let clamped = SCENE.replace(r#"renderer="pathtrace""#, r#"renderer="pathtrace" pathSamples="1e9" maxBounces="0""#);
    let c = &sr_model::rules::path_costs(&clamped).unwrap()[0];
    assert_eq!((c.samples, c.bounces), (65536, 1));
}
