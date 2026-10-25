//! The time-frames draft at the evaluator (rule 7): an `object3D` with `positionFrom="ephemeris"` stands at its
//! body's position about the space's origin at the frame's time, and one with `orientation="bodyFixed"` turns with
//! its body's fixed axes; the renderer reads both from the node's x, y, z, rotation, rotationY and rotationX.

use std::path::PathBuf;

use glam::{DMat3, EulerRot};
use sr_eval::value::Value;
use sr_eval::{Evaluator, FrameGraph, FrameNode};
use sr_space::{place_at, Frame, Positions, Space, Spk, TimeScale};

const EXCERPT_DIGEST: &str = "d113bb5117946244cf63de9d06287d5aac008acd1a561d28b760bcc7d4c3d1fd";

fn media() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/corpus/media"))
}

fn excerpt() -> Spk {
    Spk::open(&media().join("de440s-excerpt.bsp"), Some(&format!("sha256:{EXCERPT_DIGEST}"))).unwrap()
}

fn evaluator(composition: &str, space: &str) -> Evaluator {
    let xml = format!(
        r##"<scene version="1.8"><project width="64" height="64" fps="24" duration="4" background="#000000FF"/><assets><ephemeris id="de" src="de440s-excerpt.bsp" digest="sha256:{EXCERPT_DIGEST}"/></assets><composition>{composition}</composition><physics pixelsPerMeter="1e-6"/>{space}</scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(media()) };
    let doc = sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e}\n{xml}"));
    Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"))
}

fn frame(ev: &Evaluator, t: f64) -> FrameGraph {
    let g = ev.evaluate(t);
    assert!(g.failures.is_empty() && g.problems.is_empty(), "{:?} {:?}", g.failures, g.problems);
    g
}

fn node<'a>(g: &'a FrameGraph, id: &str) -> &'a FrameNode {
    g.nodes.iter().find(|n| &*n.id == id).unwrap_or_else(|| panic!("no node {id}"))
}

fn num(n: &FrameNode, k: &str) -> Option<f64> {
    match n.props.get(k) {
        Some(Value::Num(v)) => Some(*v),
        None => None,
        other => panic!("{}: {k} is {other:?}", n.id),
    }
}

fn position(n: &FrameNode) -> [f64; 3] {
    ["x", "y", "z"].map(|k| num(n, k).unwrap_or_else(|| panic!("{}: no {k}", n.id)))
}

/// The renderer's rotation of a node: Rz(rotation) · Ry(rotationY) · Rx(rotationX), degrees.
fn rotation(n: &FrameNode) -> DMat3 {
    let [z, y, x] = ["rotation", "rotationY", "rotationX"].map(|k| num(n, k).unwrap_or(0.0).to_radians());
    DMat3::from_euler(EulerRot::ZYX, z, y, x)
}

/// A placement's orientation (rows first) as a matrix.
fn matrix(rows: [[f64; 3]; 3]) -> DMat3 {
    DMat3::from_cols_array_2d(&rows).transpose()
}

fn close(a: [f64; 3], b: [f64; 3], tol: f64) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
}

const MOON: &str = r#"<object3D id="moon" primitive="sphere" radius="4" body="301" positionFrom="ephemeris"/>"#;

#[test]
fn a_body_stands_at_its_ephemeris_position_about_the_origin() {
    let space = r#"<space epoch="2451545.0" timeScale="tdb" rate="3600" origin="399" ephemeris="de"/>"#;
    let ev = evaluator(MOON, space);
    let s = Space::new("2451545.0", TimeScale::Tdb, 3600.0, 399, Frame::Icrf).unwrap();
    let spk = excerpt();
    for f in [0u64, 30, 48] {
        let want = sr_space::place_body(&spk, &s, 301, f, 24.0, 1e-6).unwrap().position;
        let got = position(node(&frame(&ev, f as f64 / 24.0), "moon"));
        assert!(close(got, want, 1e-9), "frame {f}: {got:?} {want:?}");
    }
    // the reference's distance, 402448.64 km to the metre, at 1e-6 px/m
    let p = position(node(&frame(&ev, 0.0), "moon"));
    assert!(((p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt() - 402.44864).abs() < 5e-7, "{p:?}");
}

#[test]
fn the_object_s_own_x_y_and_z_are_replaced_its_scale_kept() {
    let space = r#"<space epoch="2451545.0" timeScale="tdb" origin="399" ephemeris="de"/>"#;
    let placed = r#"<object3D id="moon" primitive="sphere" radius="4" body="301" positionFrom="ephemeris" x="50" y="50" z="50" scaleX="3"/>"#;
    let g = frame(&evaluator(placed, space), 0.0);
    let n = node(&g, "moon");
    let want = sr_space::place_body(
        &excerpt(),
        &Space::new("2451545.0", TimeScale::Tdb, 1.0, 399, Frame::Icrf).unwrap(),
        301,
        0,
        24.0,
        1e-6,
    )
    .unwrap()
    .position;
    assert!(close(position(n), want, 1e-9), "{:?}", position(n));
    // the static scale stays the element's, which the renderer reads beside the node's properties
    assert!(num(n, "scaleX").is_none_or(|v| v == 3.0));
    // positionFrom="document" keeps the document's place
    let own = r#"<object3D id="moon" primitive="sphere" radius="4" body="301" x="5" y="6" z="7"/>"#;
    let g = frame(&evaluator(own, space), 0.0);
    let n = node(&g, "moon");
    for (k, v) in [("x", 5.0), ("y", 6.0), ("z", 7.0)] {
        assert!(num(n, k).is_none_or(|got| got == v), "{k}: {:?}", num(n, k));
    }
}

#[test]
fn a_body_fixed_object_turns_with_its_body() {
    let space =
        r#"<space epoch="2451545.0" timeScale="tdb" rate="3600" origin="399" frame="eclipticJ2000" ephemeris="de"/>"#;
    let earth =
        r#"<object3D id="earth" primitive="sphere" radius="6" body="399" orientation="bodyFixed" x="1" y="2" z="3"/>"#;
    let ev = evaluator(earth, space);
    let s = Space::new("2451545.0", TimeScale::Tdb, 3600.0, 399, Frame::EclipticJ2000).unwrap();
    let spk = excerpt();
    for f in [0u64, 7, 48] {
        let g = frame(&ev, f as f64 / 24.0);
        let n = node(&g, "earth");
        let want = matrix(sr_space::place_body(&spk, &s, 399, f, 24.0, 1e-6).unwrap().orientation);
        let got = rotation(n);
        assert!(got.abs_diff_eq(want, 1e-12), "frame {f}: {got:?} {want:?}");
        // orientation alone keeps the document's position
        for (k, v) in [("x", 1.0), ("y", 2.0), ("z", 3.0)] {
            assert!(num(n, k).is_none_or(|got| got == v), "{k}: {:?}", num(n, k));
        }
    }
}

#[test]
fn the_approximate_elements_place_a_planet() {
    let mars = r#"<object3D id="mars" primitive="sphere" radius="2" body="4" positionFrom="ephemeris"/>"#;
    let ev = evaluator(mars, r#"<space epoch="2025-06-01T00:00:00Z" rate="86400" frame="eclipticJ2000"/>"#);
    let s = Space::new("2025-06-01T00:00:00Z", TimeScale::Utc, 86400.0, 0, Frame::EclipticJ2000).unwrap();
    for t in [0.0, 1.5] {
        let want = place_at(Positions::Approximate, &s, 4, s.time_at(t).unwrap(), 1e-6).unwrap();
        let g = frame(&ev, t);
        let n = node(&g, "mars");
        assert!(close(position(n), want.position, 1e-9), "{t}: {:?} {:?}", position(n), want.position);
        // a body without rotation elements keeps the document's orientation
        assert!(num(n, "rotation").is_none_or(|r| r == 0.0));
    }
}

#[test]
fn a_frame_past_the_elements_years_fails_and_names_the_object() {
    // 3000-12-31 plus a day per second of scene time: the second frame is past the long table's year 3000
    let ev = evaluator(
        r#"<object3D id="mars" primitive="sphere" radius="2" body="4" positionFrom="ephemeris"/>"#,
        r#"<space epoch="3000-12-31T12:00:00Z" rate="86400"/>"#,
    );
    frame(&ev, 0.0);
    let g = ev.evaluate(1.0);
    assert!(g.failures.iter().any(|f| f.starts_with("mars: ")), "{:?}", g.failures);
    // a frame that fails is a frame of the run, not a stuck one: an earlier time evaluates again
    frame(&ev, 0.0);
}
