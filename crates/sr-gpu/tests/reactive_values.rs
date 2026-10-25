//! The reactive-values draft in the text builder, without a device: a chart series whose `values` is linked to a
//! value reads it at the frame, so a counter chart shows a live count (Semantics 7).

use sr_eval::{FrameGraph, Value};
use sr_gpu::text::{asset_drawing, asset_of, Cx, TextCache};
use sr_text::Drawing;
use sr_vector::scene::Paint;
use std::collections::HashMap;

fn evaluator(assets: &str) -> sr_eval::Evaluator {
    let xml = format!(
        r##"<scene version="1.8"><project width="320" height="160" fps="24" duration="4" background="#000000FF"/>
  <assets>{assets}</assets><composition><layer id="l" asset="c"/></composition></scene>"##
    );
    let doc =
        sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}\n{xml}"));
    sr_eval::Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"))
}

/// The drawing of layer `l`'s asset at time `t`.
fn draw(ev: &sr_eval::Evaluator, t: f64) -> Drawing {
    let g: FrameGraph = ev.evaluate(t);
    assert!(g.failures.is_empty(), "{:?}", g.failures);
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
    r.unwrap()
}

fn counter(series: &str) -> String {
    format!(r#"<chart id="c" kind="counter" width="320" height="160">{series}</chart>"#)
}

#[test]
fn a_counter_chart_shows_a_linked_value() {
    let live = evaluator(&format!(
        r#"<value id="n" step="1" update="value + 1"/>{}"#,
        counter(r#"<series name="n" values="0"><link property="values" source="n.value"/></series>"#)
    ));
    for (t, shown) in [(0.5, "0"), (2.5, "2"), (3.0, "3")] {
        let fixed = evaluator(&counter(&format!(r#"<series name="n" values="{shown}"/>"#)));
        assert_eq!(draw(&live, t), draw(&fixed, 0.0), "t = {t}");
    }
    assert_ne!(draw(&live, 2.5), draw(&live, 0.5));
}

#[test]
fn a_bar_chart_follows_an_expressed_series() {
    let live = evaluator(
        &counter(
            r#"<series name="s" values="0 0"><expression property="values">[time, 2 * time]</expression></series>"#,
        )
        .replace("counter", "bar"),
    );
    let fixed = evaluator(&counter(r#"<series name="s" values="1 2"/>"#).replace("counter", "bar"));
    assert_eq!(draw(&live, 1.0), draw(&fixed, 0.0));
}
