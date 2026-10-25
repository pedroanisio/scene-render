//! The semantic-tex draft in the text builder, without a device: a formula layer's `formulaAnimator`s, read at the
//! frame like a `textAnimator`'s, draw the formula by part (Semantics 5), a `formulaMorph` asset draws at its animated
//! progress between the two formulas it names (Semantics 2 to 4, 6 to 9, 12), and a part the formula lacks is an
//! error of the layer.

use sr_eval::{FrameGraph, Value};
use sr_gpu::text::{asset_drawing, asset_of, Cx, TextCache};
use sr_text::animate::Props;
use sr_text::parts::MatchBy;
use sr_text::{formula, formula_anim, Drawing};
use sr_vector::scene::Paint;
use std::collections::HashMap;

const EQ: &str = r"\part{lhs}{E} = \part{rhs}{m c^2}";
const EQ2: &str = r"\part{lhs}{E} = \part{rhs}{m^2}";
const RENAMED: &str = r"\part{x}{E} = \part{y}{m c^2}";

fn white() -> Paint {
    Paint::Solid { rgba: [1.0; 4], srgb: false }
}

fn evaluator(assets: &str, composition: &str) -> sr_eval::Evaluator {
    let xml = format!(
        r##"<scene version="1.8"><project width="640" height="320" fps="24" duration="2" background="#000000FF"/>
  <assets>{assets}</assets><composition>{composition}</composition></scene>"##
    );
    let doc =
        sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}\n{xml}"));
    sr_eval::Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"))
}

/// The drawing of layer `l`'s asset at time `t`, in the asset's pixels.
fn draw(ev: &sr_eval::Evaluator, t: f64) -> Result<Drawing, String> {
    let g: FrameGraph = ev.evaluate(t);
    let n = g.nodes.iter().find(|n| &*n.id == "l").expect("layer l");
    let key = n.asset.as_deref().expect("an asset");
    let p = ev.program();
    let (a, _) = asset_of(p, key).expect("the asset");
    let mut paint = |v: &Value, _: [f64; 4]| match v {
        Value::Color(c) => Some(Paint::Solid { rgba: *c, srgb: false }),
        _ => None,
    };
    let (tokens, mut unsupported) = (HashMap::new(), Vec::new());
    let mut cx = Cx {
        p,
        g: &g,
        n,
        base: Default::default(),
        tol: 0.1,
        projection: None,
        paint: &mut paint,
        tokens: &tokens,
        unsupported: &mut unsupported,
    };
    let r = asset_drawing(&mut TextCache::default(), &mut cx, key, a).expect("a drawable asset");
    assert!(unsupported.is_empty(), "{unsupported:?}");
    r
}

fn lib() -> sr_text::FontLib {
    sr_text::FontLib::new(true)
}

fn formula_asset(id: &str, tex: &str) -> String {
    format!(r##"<formula id="{id}" tex="{tex}" width="640" height="320" size="48" color="#FFFFFFFF"/>"##)
}

#[test]
fn a_formula_animator_draws_its_part_at_the_frame() {
    let ev = evaluator(
        &formula_asset("f", EQ),
        r#"<layer id="l" asset="f"><formulaAnimator part="rhs" y="-40"><animate property="x"><key time="0" value="0"/><key time="1" value="20"/></animate></formulaAnimator></layer>"#,
    );
    let got = draw(&ev, 0.5).unwrap();
    let want = formula_anim::draw_animated(
        &mut lib(),
        EQ,
        48.0,
        [640.0, 320.0],
        white(),
        &[("rhs".into(), Props { x: Some(10.0), y: Some(-40.0), ..Default::default() })],
        0.1,
    )
    .unwrap();
    assert_eq!(got, want);
    assert_ne!(got, formula::draw(&mut lib(), EQ, 48.0, [640.0, 320.0], white(), 0.1).unwrap());
}

#[test]
fn a_formula_without_animators_draws_as_before() {
    let ev = evaluator(&formula_asset("f", EQ), r#"<layer id="l" asset="f"/>"#);
    assert_eq!(draw(&ev, 0.0).unwrap(), formula::draw(&mut lib(), EQ, 48.0, [640.0, 320.0], white(), 0.1).unwrap());
}

#[test]
fn a_part_the_formula_lacks_is_an_error_of_the_layer() {
    let ev =
        evaluator(&formula_asset("f", EQ), r#"<layer id="l" asset="f"><formulaAnimator part="mid" x="4"/></layer>"#);
    let e = draw(&ev, 0.0).unwrap_err();
    assert!(e.contains("l:") && e.contains("\"mid\""), "{e}");
}

fn morph_doc(to: &str, attrs: &str) -> sr_eval::Evaluator {
    evaluator(
        &format!(
            r##"{}{}<formulaMorph id="m" from="f" to="g" width="640" height="320" size="48" {attrs}><animate property="progress"><key time="0" value="0"/><key time="1" value="1"/></animate></formulaMorph>"##,
            formula_asset("f", EQ),
            formula_asset("g", to),
        ),
        r#"<layer id="l" asset="m"/>"#,
    )
}

#[test]
fn a_formula_morph_draws_at_its_animated_progress() {
    let ev = morph_doc(EQ2, "");
    for t in [0.0, 0.3, 0.75, 1.0] {
        let want = formula_anim::draw_morph(&mut lib(), EQ, EQ2, MatchBy::Name, t, 48.0, [640.0, 320.0], white(), 0.1)
            .unwrap();
        assert_eq!(draw(&ev, t).unwrap(), want, "t = {t}");
    }
}

#[test]
fn the_morph_matches_by_its_match_attribute_and_paints_in_its_colour() {
    let by_tex = morph_doc(RENAMED, r##"match="tex" color="#FF0000FF""##);
    let red = Paint::Solid { rgba: [1.0, 0.0, 0.0, 1.0], srgb: false };
    let want =
        formula_anim::draw_morph(&mut lib(), EQ, RENAMED, MatchBy::Tex, 0.5, 48.0, [640.0, 320.0], red.clone(), 0.1)
            .unwrap();
    assert_eq!(draw(&by_tex, 0.5).unwrap(), want);
    let by_name =
        formula_anim::draw_morph(&mut lib(), EQ, RENAMED, MatchBy::Name, 0.5, 48.0, [640.0, 320.0], red, 0.1).unwrap();
    assert_ne!(want, by_name);
}
