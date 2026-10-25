//! The reactive-values draft at the evaluator (sr-core srep/srep-0000-reactive-values.md): a `value` asset is a
//! property without a node (Semantics 1), read by `prop("id.value")`, `propAtTime` and `link source="id.value"`.

use sr_eval::{Evaluator, FrameGraph, Value};

fn evaluator(assets: &str, composition: &str) -> Evaluator {
    let xml = format!(
        r##"<scene version="1.8"><project width="200" height="200" fps="24" duration="2" background="#000000FF"/>
  <assets>{assets}</assets><composition>{composition}</composition></scene>"##
    );
    let doc =
        sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}\n{xml}"));
    Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"))
}

fn x_of(g: &FrameGraph, id: &str) -> f64 {
    let n = g.nodes.iter().find(|n| &*n.id == id).unwrap_or_else(|| panic!("node {id}"));
    n.props.get("x").and_then(Value::as_num).unwrap_or_else(|| panic!("x of {id}: {:?}", n.props.get("x")))
}

const KEYED: &str =
    r#"<value id="v"><animate property="value"><key time="0" value="0"/><key time="1" value="4"/></animate></value>"#;

fn rect(id: &str, children: &str) -> String {
    format!(r#"<shape id="{id}" shape="rect" width="10" height="10">{children}</shape>"#)
}

#[test]
fn a_keyed_value_is_read_by_prop_and_link() {
    let ev = evaluator(
        KEYED,
        &format!(
            "{}{}{}",
            rect("e", r#"<expression property="x">prop("v.value") * 10</expression>"#),
            rect("l", r#"<link property="x" source="v.value"/>"#),
            rect("t", r#"<expression property="x">propAtTime("v.value", 1)</expression>"#),
        ),
    );
    let g = ev.evaluate(0.5);
    assert_eq!(x_of(&g, "e"), 20.0);
    assert_eq!(x_of(&g, "l"), 2.0);
    assert_eq!(x_of(&g, "t"), 4.0);
}

#[test]
fn a_plain_value_is_its_attribute() {
    let ev = evaluator(
        r#"<value id="v" value="3"/>"#,
        &rect("e", r#"<expression property="x">prop("v.value")</expression>"#),
    );
    assert_eq!(x_of(&ev.evaluate(1.0), "e"), 3.0);
}

#[test]
fn an_expressed_value_reads_time() {
    let ev = evaluator(
        r#"<value id="v"><expression property="value">time * 3</expression></value>"#,
        &rect("e", r#"<link property="x" source="v.value"/>"#),
    );
    assert_eq!(x_of(&ev.evaluate(0.5), "e"), 1.5);
}
