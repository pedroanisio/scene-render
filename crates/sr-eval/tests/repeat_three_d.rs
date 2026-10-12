//! A repeat with `threeD` is placed in 2.5D as one picture: its copies are not 2.5D nodes of their own.

use sr_eval::{EvalOptions, Evaluator};

#[test]
fn the_copies_of_a_threed_repeat_have_no_placement_of_their_own() {
    let xml = r##"<scene version="1.1"><project width="600" height="960" fps="10" duration="1"/><composition>
<repeat id="r" count="3" offsetX="100" threeD="true" rotationX="40" zDepth="50"><shape id="s" shape="rect" width="60" height="60" fill="#FFFFFF"/></repeat>
</composition></scene>"##;
    let d = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}"));
    let f = Evaluator::new(&d, &EvalOptions::default()).unwrap_or_else(|r| panic!("{r}")).evaluate(0.0);
    let three: Vec<(String, Option<[f64; 3]>)> = f.nodes.iter().map(|n| (n.id.to_string(), n.three_d)).collect();
    let of = |id: &str| three.iter().find(|(k, _)| k == id).unwrap_or_else(|| panic!("{id} in {three:?}")).1;
    assert_eq!(of("r"), Some([50.0, 40.0, 0.0]), "the repeat is placed: {three:?}");
    for k in 0..3 {
        assert_eq!(of(&format!("r[{k}]")), None, "copy {k} is not placed again: {three:?}");
    }
}
