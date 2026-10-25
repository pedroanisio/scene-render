//! The semantic-tex draft at the evaluator (sr-core srep/srep-0000-semantic-tex.md): a `formulaMorph`'s progress is an
//! ordinary animated number (Semantics 12), and the path keys of a shape with `morph="true"` interpolate by the path
//! morph of Semantics 6 to 9 at the eased progress of their segment, exactly each key at its own time; with `morph`
//! false or absent they hold, as before.

use sr_eval::{Evaluator, FrameGraph, Value};
use sr_vector::morph::{morph_path, MORPH_FLATNESS, MORPH_SAMPLES};

fn evaluator(assets: &str, composition: &str) -> Evaluator {
    let xml = format!(
        r##"<scene version="1.8"><project width="200" height="200" fps="24" duration="2" background="#000000FF"/>
  <assets>{assets}</assets><composition>{composition}</composition></scene>"##
    );
    let doc =
        sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}\n{xml}"));
    Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"))
}

fn path_at(g: &FrameGraph) -> String {
    let n = g.nodes.iter().find(|n| &*n.id == "s").expect("node s");
    match n.props.get("path") {
        Some(Value::Str(s)) => s.to_string(),
        other => panic!("path is {other:?}"),
    }
}

const SQUARE: &str = "M 0 0 L 100 0 L 100 100 L 0 100 Z";
const TRIANGLE: &str = "M 0 0 L 100 0 L 50 80 Z";

fn shape(morph: &str, a: &str, b: &str, ease: &str) -> String {
    format!(
        r#"<shape id="s" shape="path" {morph} width="100" height="100" path="{a}"><animate property="path"{ease}><key time="0" value="{a}"/><key time="1" value="{b}"/></animate></shape>"#
    )
}

fn polys(d: &str) -> Vec<Vec<[f64; 2]>> {
    sr_vector::Path::parse(d)
        .unwrap()
        .flatten(MORPH_FLATNESS)
        .into_iter()
        .map(|q| q.pts.iter().map(|p| [p.x, p.y]).collect())
        .collect()
}

#[track_caller]
fn same_polys(a: &[Vec<[f64; 2]>], b: &[Vec<[f64; 2]>]) {
    assert_eq!(a.len(), b.len());
    for (p, q) in a.iter().zip(b) {
        assert_eq!(p.len(), q.len());
        for (x, y) in p.iter().zip(q) {
            assert!((x[0] - y[0]).abs() < 1e-9 && (x[1] - y[1]).abs() < 1e-9, "{x:?} {y:?}");
        }
    }
}

#[test]
fn a_formula_morph_progress_animates() {
    let ev = evaluator(
        r#"<formula id="f" tex="\part{a}{x}" width="64" height="32"/><formula id="g" tex="\part{a}{y}" width="64" height="32"/>
           <formulaMorph id="m" from="f" to="g" width="64" height="32"><animate property="progress"><key time="0" value="0"/><key time="1" value="1"/></animate></formulaMorph>"#,
        r#"<layer id="l" asset="m"/>"#,
    );
    let g = ev.evaluate(0.25);
    let m = g.elements.iter().find(|e| &*e.key == "m").expect("element m");
    assert_eq!(m.props.get("progress").and_then(Value::as_num), Some(0.25));
}

#[test]
fn morphed_path_keys_interpolate_by_the_path_morph() {
    let ev = evaluator("", &shape(r#"morph="true""#, SQUARE, TRIANGLE, ""));
    // each key exactly at its time
    assert_eq!(path_at(&ev.evaluate(0.0)), SQUARE);
    assert_eq!(path_at(&ev.evaluate(1.0)), TRIANGLE);
    // between, the morph of the two keys' sub-paths at the progress
    for t in [0.25, 0.5, 0.75] {
        let want = morph_path(&polys(SQUARE), &polys(TRIANGLE), t, MORPH_SAMPLES).unwrap();
        same_polys(&polys(&path_at(&ev.evaluate(t))), &want);
    }
}

#[test]
fn the_progress_is_the_eased_progress_of_the_segment() {
    let ev = evaluator("", &shape(r#"morph="true""#, SQUARE, TRIANGLE, r#" defaultInterpolation="ease-in""#));
    let linear = evaluator("", &shape(r#"morph="true""#, SQUARE, TRIANGLE, ""));
    // ease-in lags the linear progress in the first half
    let (eased, lin) = (polys(&path_at(&ev.evaluate(0.5))), polys(&path_at(&linear.evaluate(0.5))));
    assert_ne!(eased, lin);
    let at = |p: f64| morph_path(&polys(SQUARE), &polys(TRIANGLE), p, MORPH_SAMPLES).unwrap();
    // the apex sample moves monotonically with progress: the eased one is nearer the square
    let apex = |ps: &[Vec<[f64; 2]>]| ps[0].iter().map(|q| q[1]).fold(f64::MIN, f64::max);
    assert!(apex(&eased) > apex(&lin) && apex(&eased) < apex(&at(0.0)), "{} {}", apex(&eased), apex(&lin));
}

#[test]
fn without_morph_path_keys_hold() {
    for morph in ["", r#"morph="false""#] {
        let ev = evaluator("", &shape(morph, SQUARE, TRIANGLE, ""));
        assert_eq!(path_at(&ev.evaluate(0.5)), SQUARE, "{morph}");
        assert_eq!(path_at(&ev.evaluate(1.0)), TRIANGLE, "{morph}");
    }
}

#[test]
fn a_key_with_no_sub_path_holds() {
    // rule 9 has no centroid to collapse to: the key holds as without morph
    let ev = evaluator("", &shape(r#"morph="true""#, SQUARE, "", ""));
    assert_eq!(path_at(&ev.evaluate(0.5)), SQUARE);
    assert_eq!(path_at(&ev.evaluate(1.0)), "");
}

#[test]
fn sub_paths_pair_by_centroid() {
    // two squares into one triangle: the far square collapses to the triangle's centroid
    let two = "M 0 0 L 40 0 L 40 40 L 0 40 Z M 150 0 L 190 0 L 190 40 L 150 40 Z";
    let ev = evaluator("", &shape(r#"morph="true""#, two, TRIANGLE, ""));
    let p = polys(&path_at(&ev.evaluate(0.5)));
    assert_eq!(p.len(), 2);
    let want = morph_path(&polys(two), &polys(TRIANGLE), 0.5, MORPH_SAMPLES).unwrap();
    same_polys(&p, &want);
}

#[test]
fn a_seek_equals_the_frame_played_from_zero() {
    // Semantics 12: no state between frames
    let ev = evaluator("", &shape(r#"morph="true""#, SQUARE, TRIANGLE, ""));
    let direct = path_at(&ev.evaluate(0.6));
    for t in [0.0, 0.2, 0.4] {
        ev.evaluate(t);
    }
    assert_eq!(path_at(&ev.evaluate(0.6)), direct);
}
