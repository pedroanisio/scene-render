//! Before composition time 0, where a shutter that opens early samples the first frame (SREP 55), the scene is the one
//! the programme starts with: a node whose window opens at or before 0 is there, holding its first keys, and a node
//! that appears later is not.

fn eval(xml: &str, t: f64) -> sr_eval::FrameGraph {
    let d = sr_model::load_str(xml, &sr_model::LoadOptions { verify_assets: false, base_dir: None }).unwrap();
    sr_eval::Evaluator::new(&d, &sr_eval::EvalOptions::default()).unwrap().evaluate(t)
}

const DOC: &str = r##"<scene version="1.2"><project width="160" height="40" fps="10" duration="2"/><composition>
  <shape id="default" shape="rect" width="8" height="8"><animate property="x"><key time="0" value="10"/><key time="1" value="110"/></animate></shape>
  <shape id="zero" shape="rect" width="8" height="8" start="0"/>
  <group id="g" width="160" height="40"><shape id="child" shape="rect" width="8" height="8"/></group>
  <shape id="later" shape="rect" width="8" height="8" start="0.5"/>
</composition></scene>"##;

fn ids(g: &sr_eval::FrameGraph) -> Vec<&str> {
    g.nodes.iter().map(|n| &*n.id).collect()
}

#[test]
fn the_nodes_the_programme_starts_with_are_there_before_time_zero() {
    let g = eval(DOC, -0.025);
    let here = ids(&g);
    for id in ["default", "zero", "g", "child"] {
        assert!(here.contains(&id), "{id} is there from the start: {here:?}");
    }
    assert!(!here.contains(&"later"), "a node that appears at 0.5 s is not: {here:?}");
    // the property holds its first key before it
    let n = g.nodes.iter().find(|n| &*n.id == "default").unwrap();
    assert_eq!(n.world.apply([0.0, 0.0])[0], 10.0);
    // the same nodes as at time 0
    assert_eq!(here, ids(&eval(DOC, 0.0)));
}
