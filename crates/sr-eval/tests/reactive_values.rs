//! The reactive-values draft at the evaluator (sr-core srep/srep-0000-reactive-values.md): a `value` asset is a
//! property without a node (Semantics 1), read by `prop("id.value")`, `propAtTime` and `link source="id.value"`.

use sr_eval::{Evaluator, FrameGraph, Value};

fn build(assets: &str, composition: &str) -> Result<Evaluator, sr_model::Report> {
    let xml = format!(
        r##"<scene version="1.8"><project width="200" height="200" fps="24" duration="4" background="#000000FF"/>
  <assets>{assets}</assets><composition>{composition}</composition></scene>"##
    );
    let doc =
        sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}\n{xml}"));
    Evaluator::new(&doc, &Default::default())
}

fn evaluator(assets: &str, composition: &str) -> Evaluator {
    build(assets, composition).unwrap_or_else(|e| panic!("{e}"))
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

/// The values of the value assets `ids` at `t`, each read by a node's `x`.
fn values_at(ev: &Evaluator, ids: &[&str], t: f64) -> Vec<f64> {
    let g = ev.evaluate(t);
    ids.iter().map(|id| x_of(&g, &format!("r_{id}"))).collect()
}

fn readers(ids: &[&str]) -> String {
    ids.iter().map(|id| rect(&format!("r_{id}"), &format!(r#"<link property="x" source="{id}.value"/>"#))).collect()
}

fn fold(assets: &str, ids: &[&str]) -> Evaluator {
    evaluator(assets, &readers(ids))
}

fn one(value: &str, t: f64) -> f64 {
    values_at(&fold(value, &["v"]), &["v"], t)[0]
}

fn codes(assets: &str, ids: &[&str]) -> Vec<String> {
    match build(assets, &readers(ids)) {
        Ok(_) => Vec::new(),
        Err(r) => r.diagnostics.into_iter().map(|d| d.code).collect(),
    }
}

#[test]
fn a_time_fold_integrates_in_fixed_steps_rule_3() {
    assert_eq!(one(r#"<value id="v" value="0" step="0.25" update="value + 2 * dt"/>"#, 1.0), 2.0);
    assert_eq!(one(r#"<value id="v" step="1" update="value + index"/>"#, 4.0), 6.0);
    assert_eq!(one(r#"<value id="v" step="0.5" update="time"/>"#, 2.0), 2.0);
    // count is the step number, index + 1
    assert_eq!(one(r#"<value id="v" step="1" update="count"/>"#, 3.0), 3.0);
}

#[test]
fn a_time_fold_holds_or_interpolates_between_steps() {
    assert_eq!(one(r#"<value id="v" step="1" update="value + 1"/>"#, 2.5), 2.0);
    assert_eq!(one(r#"<value id="v" step="1" update="value + 1" between="linear"/>"#, 2.5), 2.5);
}

#[test]
fn the_initial_state_is_the_keyed_value_at_the_start() {
    assert_eq!(one(r#"<value id="v" value="5" start="2" step="1" update="value + 1"/>"#, 1.0), 5.0);
    let keyed = r#"<value id="v" start="1" step="1" update="value + 1"><animate property="value"><key time="0" value="0"/><key time="2" value="4"/></animate></value>"#;
    assert_eq!(one(keyed, 1.0), 2.0);
    assert_eq!(one(keyed, 2.0), 3.0);
}

#[test]
fn the_steps_count_with_the_tolerance_of_frame_rule_2() {
    // floor(0.3 / 0.1) is 2 in binary64; the 1e-9 tolerance makes it 3
    assert_eq!(one(r#"<value id="v" step="0.1" update="value + 1"/>"#, 0.3), 3.0);
}

#[test]
fn time_folds_of_one_step_read_each_other_before_the_step_rule_5() {
    let ev = fold(
        r#"<value id="a" value="1" step="1" update='prop("b.value")'/><value id="b" value="2" step="1" update='prop("a.value")'/>"#,
        &["a", "b"],
    );
    assert_eq!(values_at(&ev, &["a", "b"], 1.0), [2.0, 1.0]);
    assert_eq!(values_at(&ev, &["a", "b"], 2.0), [1.0, 2.0]);
}

#[test]
fn draws_are_functions_of_seed_channel_step_and_i_rule_6() {
    let v = one(r#"<value id="v" seed="7" step="1" update="index == 3 ? draw(0, 1) : value"/>"#, 4.0);
    assert_eq!(v, 0.8323654982572867);
    let channel = sr_eval::expr::vm::property_channel("value");
    let want = sr_vector::d24::d24_unit(1, 5, channel << 32);
    assert_eq!(one(r#"<value id="v" seed="1" step="1" update="random()"/>"#, 6.0), want);
}

#[test]
fn an_index_fold_runs_count_times_at_every_frame_rule_4() {
    assert_eq!(one(r#"<value id="v" count="10" update="value + index"/>"#, 0.0), 45.0);
    assert_eq!(one(r#"<value id="v" count="4" update="value + 1 / count"/>"#, 0.0), 1.0);
    assert_eq!(one(r#"<value id="v" count="3" update="time"/>"#, 2.5), 2.5);
    assert_eq!(one(r#"<value id="v" count="2" update="value + dt"/>"#, 1.0), 0.0);
}

#[test]
fn the_fourier_centre_of_shot_9() {
    let centre = |f: &str| {
        let tau = "(index / count * 4)";
        let g = format!("Math.cos(2 * Math.PI * 3 * {tau})");
        let assets = format!(
            r#"<value id="f" value="{f}"/><value id="x" count="400" update='value + {g} * Math.cos(2 * Math.PI * prop("f.value") * {tau}) / count'/><value id="y" count="400" update='value - {g} * Math.sin(2 * Math.PI * prop("f.value") * {tau}) / count'/>"#
        );
        values_at(&fold(&assets, &["x", "y"]), &["x", "y"], 0.0)
    };
    let [x, y] = centre("3")[..] else { unreachable!() };
    assert!((x - 0.5).abs() < 1e-9 && y.abs() < 1e-9, "({x}, {y})");
    let [x, y] = centre("2")[..] else { unreachable!() };
    assert!(x.abs() < 1e-9 && y.abs() < 1e-9, "({x}, {y})");
}

const CELLS: [&str; 4] = ["e0", "e1", "e2", "e3"];

fn einstein(seed: u64) -> String {
    let reads = CELLS.map(|c| format!(r#"prop("{c}.value")"#)).join(", ");
    (0..4)
        .map(|i| {
            let e = if i == 0 { 3 } else { 0 };
            format!(
                r#"<value id="e{i}" value="{e}" seed="{seed}" step="0.016666666666666666" update='var s = Math.min(Math.floor(draw(0, 0) * 4), 3); var d = (s + (draw(1, 0) &lt; 0.5 ? 3 : 1)) % 4; var ok = [{reads}][s] &gt; 0; value - (ok &amp;&amp; s == {i} ? 1 : 0) + (ok &amp;&amp; d == {i} ? 1 : 0)'/>"#
            )
        })
        .collect()
}

#[test]
fn the_einstein_solid_of_shot_5() {
    let ev = fold(&einstein(1), &CELLS);
    assert_eq!(values_at(&ev, &CELLS, 20.0 / 60.0), [1.0, 0.0, 1.0, 1.0]);
    for t in [0.5, 1.7, 3.9] {
        assert_eq!(values_at(&ev, &CELLS, t).iter().sum::<f64>(), 3.0, "t = {t}");
    }
    assert_ne!(values_at(&fold(&einstein(2), &CELLS), &CELLS, 20.0 / 60.0), [1.0, 0.0, 1.0, 1.0]);
}

#[test]
fn a_frame_alone_equals_the_frame_played_in_sequence() {
    let cases: [(String, &[&str], &[f64]); 3] = [
        (r#"<value id="v" step="0.1" update="value + index"/>"#.into(), &["v"], &[0.0, 0.55, 0.3, 1.0, 0.3]),
        (einstein(1), &CELLS, &[0.5, 0.2, 1.0, 0.2, 3.9, 0.0]),
        // 4000 steps: past several checkpoints, back before them and between them
        (
            r#"<value id="v" step="0.001" update="value + Math.sin(index)" between="linear"/>"#.into(),
            &["v"],
            &[3.5, 1.2, 3.9995, 0.0, 2.0485, 1.0245, 3.5],
        ),
    ];
    for (assets, ids, times) in cases {
        let played = fold(&assets, ids);
        for &t in times {
            let alone = values_at(&fold(&assets, ids), ids, t);
            assert_eq!(values_at(&played, ids, t), alone, "t = {t}");
        }
    }
}

#[test]
fn folds_of_different_steps_must_not_read_each_other_e1() {
    let got =
        codes(r#"<value id="a" step="1" update='prop("b.value")'/><value id="b" step="0.5" update="value"/>"#, &["a"]);
    assert!(got.iter().any(|c| c == "RVE1"), "{got:?}");
    let got = codes(
        r#"<value id="a" start="1" step="1" update='prop("b.value")'/><value id="b" step="1" update="value"/>"#,
        &["a"],
    );
    assert!(got.iter().any(|c| c == "RVE1"), "{got:?}");
    // no value reads an index fold inside an update
    let got =
        codes(r#"<value id="a" step="1" update='prop("b.value")'/><value id="b" count="3" update="value"/>"#, &["a"]);
    assert!(got.iter().any(|c| c == "RVE1"), "{got:?}");
    // an index fold reads a time fold, and anything reads a fold outside an update
    assert!(codes(
        r#"<value id="a" step="1" update="value + 1"/><value id="b" count="3" update='value + prop("a.value")'/>"#,
        &["b"]
    )
    .is_empty());
}

#[test]
fn a_time_fold_takes_at_most_a_million_steps_e2() {
    let got = codes(r#"<value id="v" step="0.000001" update="value"/>"#, &["v"]);
    assert!(got.iter().any(|c| c == "RVE2"), "{got:?}");
    assert!(codes(r#"<value id="v" step="0.000004" update="value"/>"#, &["v"]).is_empty());
}

#[test]
fn a_non_finite_update_is_a_render_error() {
    let ev = fold(r#"<value id="v" value="1" step="1" update="value / 0"/>"#, &["v"]);
    assert!(ev.evaluate(0.5).failures.is_empty());
    let g = ev.evaluate(1.0);
    assert!(g.failures.iter().any(|f| f.starts_with("v: ") && f.contains("non-finite")), "{:?}", g.failures);
}

#[test]
fn dt_and_draw_are_names_of_an_update_only() {
    for x in ["dt", "draw(0, 0)"] {
        let got = match build("", &rect("e", &format!(r#"<expression property="x">{x}</expression>"#))) {
            Ok(_) => Vec::new(),
            Err(r) => r.diagnostics.into_iter().map(|d| d.code).collect(),
        };
        assert!(got.iter().any(|c| c == "E01"), "{x}: {got:?}");
    }
}
