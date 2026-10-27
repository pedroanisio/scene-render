//! The scale draft at the evaluator (rule 2): a `geometric` key interpolates the segment it leaves as
//! v0 (v1 / v0)^u, so equal times give equal ratios; the powers of ten camera from 10 to 1e20 over 40 s stands at
//! `sr_3d::scale::powers_of_ten` at every frame, and a frame evaluated alone equals the frame played to.

use sr_3d::scale::powers_of_ten;
use sr_eval::value::Value;
use sr_eval::{Evaluator, FrameGraph};

fn evaluator(animate: &str, fps: &str, duration: &str) -> Evaluator {
    let xml = format!(
        r##"<scene version="1.8"><project width="64" height="64" fps="{fps}" duration="{duration}" background="#000000FF"/><composition><camera id="eye" near="10" far="1e21" depth="logarithmic">{animate}</camera></composition></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
    let doc = sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e}\n{xml}"));
    Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"))
}

fn z(g: &FrameGraph) -> f64 {
    assert!(g.failures.is_empty() && g.problems.is_empty(), "{:?} {:?}", g.failures, g.problems);
    let n = g.nodes.iter().find(|n| &*n.id == "eye").expect("the camera");
    match n.props.get("z") {
        Some(Value::Num(v)) => *v,
        other => panic!("z is {other:?}"),
    }
}

const POWERS: &str = r#"<animate property="z"><key time="0" value="-10" interpolation="geometric"/><key time="40" value="-1e20"/></animate>"#;

#[test]
fn a_geometric_key_gives_equal_ratios_in_equal_times() {
    let ev = evaluator(POWERS, "24", "40");
    assert_eq!(z(&ev.evaluate(0.0)), -10.0);
    assert_eq!(z(&ev.evaluate(40.0)), -1e20);
    // nineteen decades in forty seconds: every two seconds the same ratio, 10^(19/20)
    let ratio = 10f64.powf(19.0 / 20.0);
    for t in [0.0, 2.0, 10.0, 20.0, 38.0] {
        let got = z(&ev.evaluate(t + 2.0)) / z(&ev.evaluate(t));
        assert!((got / ratio - 1.0).abs() < 1e-12, "t {t}: {got} against {ratio}");
    }
}

#[test]
fn the_powers_of_ten_camera_stands_at_the_reference_at_every_frame() {
    let ev = evaluator(POWERS, "24", "40");
    for frame in 0..=960u64 {
        let t = frame as f64 / 24.0;
        let want = -powers_of_ten(t, 0.0, 40.0, 10.0, 1e20).unwrap();
        assert_eq!(z(&ev.evaluate(t)), want, "frame {frame}");
    }
}

#[test]
fn a_frame_alone_equals_the_frame_played_to() {
    let played = evaluator(POWERS, "24", "40");
    let ahead: Vec<f64> = (0..=960u64).map(|f| z(&played.evaluate(f as f64 / 24.0))).collect();
    for frame in [960u64, 0, 481, 17, 700] {
        let alone = evaluator(POWERS, "24", "40");
        assert_eq!(z(&alone.evaluate(frame as f64 / 24.0)), ahead[frame as usize], "frame {frame}");
    }
}

#[test]
fn a_geometric_default_and_a_following_linear_segment() {
    let animate = r#"<animate property="z" defaultInterpolation="geometric"><key time="0" value="-1"/><key time="1" value="-100" interpolation="linear"/><key time="2" value="0"/></animate>"#;
    let ev = evaluator(animate, "24", "2");
    assert!((z(&ev.evaluate(0.5)) + 10.0).abs() < 1e-12);
    assert!((z(&ev.evaluate(1.5)) + 50.0).abs() < 1e-12);
}

#[test]
fn a_geometric_length_keeps_its_unit() {
    let xml = r##"<scene version="1.8"><project width="64" height="64" fps="24" duration="1" background="#000000FF"/><composition><shape id="s" shape="rect" width="1" height="1"><animate property="width"><key time="0" value="1vw" interpolation="geometric"/><key time="1" value="100vw"/></animate></shape></composition></scene>"##;
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
    let ev = Evaluator::new(&sr_model::load_str(xml, &opts).unwrap(), &Default::default()).unwrap();
    let g = ev.evaluate(0.5);
    let n = g.nodes.iter().find(|n| &*n.id == "s").expect("the shape");
    match n.props.get("width") {
        Some(Value::Len(l)) => assert!((l.value - 10.0).abs() < 1e-12 && l.to_string().ends_with("vw"), "{l:?}"),
        other => panic!("width is {other:?}"),
    }
}

/// The build's E04 diagnostics of a shape whose `animate` is `animate`.
fn build_errors(animate: &str) -> Vec<String> {
    let xml = format!(
        r##"<scene version="1.8"><project width="64" height="64" fps="24" duration="1" background="#000000FF"/><composition><shape id="s" shape="rect" width="1" height="1" fill="#FFFFFFFF">{animate}</shape></composition></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
    let doc = sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e}\n{xml}"));
    match Evaluator::new(&doc, &Default::default()) {
        Ok(_) => Vec::new(),
        Err(r) => r.diagnostics.into_iter().filter(|d| d.code == "E04").map(|d| d.message).collect(),
    }
}

#[test]
fn a_geometric_pair_the_schema_cannot_read_is_checked_at_the_build() {
    // SC2 sees only plain numbers; the build applies its rule to the magnitudes
    for animate in [
        r#"<animate property="width"><key time="0" value="1vw" interpolation="geometric"/><key time="1" value="-100vw"/></animate>"#,
        r#"<animate property="width"><key time="0" value="0vw" interpolation="geometric"/><key time="1" value="100vw"/></animate>"#,
        r##"<animate property="fill"><key time="0" value="#FF0000FF" interpolation="geometric"/><key time="1" value="#00FF00FF"/></animate>"##,
    ] {
        let got = build_errors(animate);
        assert!(got.iter().any(|l| l.contains("geometric")), "{animate}: {got:?}");
    }
    assert!(build_errors(r#"<animate property="width"><key time="0" value="1vw" interpolation="geometric"/><key time="1" value="100vw"/></animate>"#).is_empty());
}
