//! The compositor compiles a pipeline the first time a draw uses it, not when the renderer is built: a frame that
//! draws with one variant leaves the others uncompiled, and its pixels are those the same draw gives on a renderer
//! that has compiled more.

mod common;
use common::*;

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];

fn scene(body: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.2"><project width="64" height="32" fps="10" duration="1" background="#000000"/>
  <composition>{body}</composition></scene>"##
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}"))
}

const SOLID: &str = r##"<shape id="r" shape="rect" x="8" y="8" width="32" height="16" fill="#FF0000FF"/>"##;
const OTHER: &str = r##"<shape id="b" shape="rect" x="8" y="8" width="32" height="16" fill="#0000FFFF" blend="add"/>"##;

#[test]
fn a_frame_compiles_only_the_pipelines_it_draws() {
    let Some(r) = render(&scene(SOLID)) else { return };
    let created = r.renderer.pipelines_created();
    assert!(created >= 1, "the draw needs a pipeline");
    assert!(created <= 3, "a renderer built for one solid rect compiled {created} pipelines");
    assert_px(&r, 20, 16, RED, 0.02);
}

#[test]
fn a_second_frame_with_the_same_draws_compiles_nothing() {
    let Some(once) = render_times(&scene(SOLID), &[0.0]) else { return };
    let Some(twice) = render_times(&scene(SOLID), &[0.0, 0.5]) else { return };
    assert_eq!(once.renderer.pipelines_created(), twice.renderer.pipelines_created());
    assert_eq!(once.px, twice.px, "the frame is the same on a renderer that has already drawn");
}

#[test]
fn a_new_variant_is_compiled_when_first_drawn() {
    let Some(plain) = render(&scene(SOLID)) else { return };
    let Some(added) = render(&scene(&format!("{SOLID}{OTHER}"))) else { return };
    assert!(added.renderer.pipelines_created() > plain.renderer.pipelines_created());
    assert_px(&added, 20, 16, [1.0, 0.0, 1.0, 1.0], 0.02);
    assert_px(&plain, 20, 16, RED, 0.02);
}
