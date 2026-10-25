//! The space-transforms draft at the evaluator: a `map` modifier's `x` and `y` are expressions of the sampling
//! variables `u` and `v`, compiled with the document and run at points the renderer chooses while it subdivides
//! (sr-core srep/srep-0000-space-transforms.md, Semantics 2 and 4). Each frame node lists its map modifiers by part
//! key, and the frame graph turns one into a point function that holds the compiled program.

use sr_eval::{Evaluator, FrameGraph, FrameNode, MapSpec};

fn document(composition: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.8"><project width="640" height="360" fps="24" duration="2" background="#000000FF" seed="1"/>
  <parameters><param id="k" type="number" default="3"/></parameters>
  <composition>{composition}</composition></scene>"##
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}\n{xml}"))
}

fn evaluator(composition: &str) -> Evaluator {
    Evaluator::new(&document(composition), &Default::default()).unwrap_or_else(|e| panic!("{e}"))
}

fn node<'a>(g: &'a FrameGraph, id: &str) -> &'a FrameNode {
    g.nodes.iter().find(|n| &*n.id == id).unwrap_or_else(|| panic!("no node {id}"))
}

fn spec<'a>(n: &'a FrameNode, key: &str) -> &'a MapSpec {
    n.deform_maps.iter().find(|(k, _)| &**k == key).map(|(_, s)| s).unwrap_or_else(|| {
        panic!("{}: no map {key} in {:?}", n.id, n.deform_maps.iter().map(|m| &*m.0).collect::<Vec<_>>())
    })
}

fn shape(modifiers: &str) -> String {
    format!(r#"<shape id="s" shape="rect" width="40" height="30"><deform>{modifiers}</deform></shape>"#)
}

#[test]
fn a_map_runs_its_expressions_at_any_point_and_time() {
    let ev = evaluator(&shape(r#"<modifier type="map" x="u * u - v * v + time" y="2 * u * v + param('k')"/>"#));
    let g = ev.evaluate(0.5);
    let f = g.point_map(spec(node(&g, "s"), "s/deform[0]/modifierType[0]")).unwrap();
    assert_eq!(f(2.0, 1.0), [3.5, 7.0]);
    assert_eq!(f(-1.0, 0.5), [1.25, 2.0]);
    // a later frame reads its own time
    let g = ev.evaluate(1.5);
    let f = g.point_map(spec(node(&g, "s"), "s/deform[0]/modifierType[0]")).unwrap();
    assert_eq!(f(2.0, 1.0), [4.5, 7.0]);
}

#[test]
fn the_key_counts_deforms_and_modifiers_as_the_renderer_does() {
    let ev = evaluator(
        r#"<group id="g"><deform><modifier type="matrix" b="0.5"/><modifier type="map" x="u" y="-v"/></deform>
           <deform><modifier type="twist" amount="10"/><modifier type="map" x="v" y="u"/></deform>
           <shape id="l" shape="rect" width="20" height="20"/></group>"#,
    );
    let g = ev.evaluate(0.0);
    let n = node(&g, "g");
    assert_eq!(n.deform_maps.len(), 2);
    assert_eq!(g.point_map(spec(n, "g/deform[0]/modifierType[1]")).unwrap()(1.0, 2.0), [1.0, -2.0]);
    assert_eq!(g.point_map(spec(n, "g/deform[1]/modifierType[1]")).unwrap()(1.0, 2.0), [2.0, 1.0]);
    assert!(node(&g, "l").deform_maps.is_empty());
}

#[test]
fn other_modifier_types_list_no_map() {
    let ev = evaluator(&shape(r#"<modifier type="complex" function="square"/><modifier type="bend" amount="5"/>"#));
    assert!(node(&ev.evaluate(0.0), "s").deform_maps.is_empty());
}

#[test]
fn a_value_that_is_no_number_maps_to_nan() {
    // rule 5: a non-finite image breaks the outline there; the renderer decides, the evaluator only reports
    let ev = evaluator(&shape(r#"<modifier type="map" x="'left'" y="1 / (u - u)"/>"#));
    let g = ev.evaluate(0.0);
    let [x, y] = g.point_map(spec(node(&g, "s"), "s/deform[0]/modifierType[0]")).unwrap()(1.0, 1.0);
    assert!(x.is_nan(), "{x}");
    assert!(!y.is_finite(), "{y}");
}

#[test]
fn an_expression_that_does_not_compile_is_e01_naming_the_attribute() {
    let doc = document(&shape(r#"<modifier type="map" x="u" y="v +"/>"#));
    let Err(report) = Evaluator::new(&doc, &Default::default()) else { panic!("compiled") };
    let d = report.diagnostics.iter().find(|d| d.code == "E01").unwrap_or_else(|| panic!("{report}"));
    assert!(d.message.starts_with("s/deform[0]/modifier[0]@y: "), "{}", d.message);
}

#[test]
fn a_graph_evaluated_without_its_program_refuses_the_map() {
    let ev = evaluator(&shape(r#"<modifier type="map" x="u" y="v"/>"#));
    let g = sr_eval::eval::evaluate(ev.program(), 0.0);
    let s = spec(node(&g, "s"), "s/deform[0]/modifierType[0]");
    let Err(e) = g.point_map(s) else { panic!("mapped without a program") };
    assert!(e.contains("s/deform[0]/modifierType[0]"), "{e}");
}

#[test]
fn the_point_function_runs_on_other_threads() {
    let ev = evaluator(&shape(r#"<modifier type="map" x="u + v" y="u - v"/>"#));
    let g = ev.evaluate(0.0);
    let f = g.point_map(spec(node(&g, "s"), "s/deform[0]/modifierType[0]")).unwrap();
    let got: Vec<[f64; 2]> = std::thread::scope(|s| {
        let f = &f;
        let hs: Vec<_> = (0..4).map(|k| s.spawn(move || f(k as f64, 1.0))).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(got, vec![[1.0, -1.0], [2.0, 0.0], [3.0, 1.0], [4.0, 2.0]]);
    // and outlives the graph and the evaluator
    drop(g);
    drop(ev);
    assert_eq!(f(1.0, 1.0), [2.0, 0.0]);
}
