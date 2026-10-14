//! `videoAsset/@pixelAspect` (non-square pixels): a clip lays out at its displayed shape, width x pixelAspect.

use sr_eval::{EvalOptions, Evaluator, FrameGraph};

fn graph(pixel_aspect: &str) -> FrameGraph {
    let xml = format!(
        r#"<scene version="1.1"><project width="400" height="200" fps="10" duration="1"/>
<assets><video id="v" src="clip.mp4" width="100" height="50" fps="10" duration="1" pixelAspect="{pixel_aspect}"/></assets>
<composition><layer id="l" asset="v" fit="contain" boxWidth="200" boxHeight="50"/></composition></scene>"#
    );
    let d = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}"));
    Evaluator::new(&d, &EvalOptions::default()).unwrap_or_else(|r| panic!("{r}")).evaluate(0.0)
}

fn dest(g: &FrameGraph) -> [f64; 4] {
    g.nodes.iter().find(|n| &*n.id == "l").unwrap().content.expect("placed content").dest
}

#[test]
fn an_anamorphic_clip_fills_a_box_of_its_displayed_shape() {
    // 100x50 coded with pixelAspect 2 displays 200x50: it fills a 200x50 box exactly, no letterbox
    assert_eq!(dest(&graph("2")), [0.0, 0.0, 200.0, 50.0]);
}

#[test]
fn square_pixels_keep_the_coded_shape() {
    // 100x50 in a 200x50 box: contained, centred horizontally
    assert_eq!(dest(&graph("1")), [50.0, 0.0, 150.0, 50.0]);
}
