//! A texture that lands texel for pixel, at full opacity with the normal blend, as the first draw into a cleared
//! target is copied instead of drawn: over a transparent target that draw writes each texel unchanged
//! (`src + 0 · (1 − a)`), so a texture copy gives the same frame without running the compositor's fragment shader on
//! every pixel. Each test renders the same frames with the copy turned off and compares every channel bit for bit.

mod common;
use common::*;

fn scene(body: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.2"><project width="64" height="32" fps="10" duration="2" background="#203040"/>
  <composition>{body}</composition></scene>"##
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}"))
}

/// Shapes the compositor rasterises into one frame-sized vector target, one of them moving.
const SHAPES: &str = r##"<shape id="a" shape="rect" x="4" y="4" width="20" height="12" fill="#FF8000C0"/>
<shape id="b" shape="ellipse" x="30" y="6" width="22" height="18" fill="#20C060FF">
  <animate property="x"><key time="0" value="30"/><key time="2" value="38"/></animate>
</shape>
<shape id="c" shape="rect" x="10" y="20" width="44" height="6" fill="#FFFFFF80"/>"##;

/// The frames at `ts` with the copy allowed (`true`) or not, and each frame's stats.
fn frames(d: &sr_model::Document, ts: &[f64], copy: bool) -> Option<Vec<(Vec<[f32; 4]>, sr_gpu::RenderStats)>> {
    let gpu = gpu()?;
    let ev = sr_eval::Evaluator::new(d, &sr_eval::EvalOptions::default()).unwrap();
    let mut r = sr_gpu::Renderer::new(gpu, ev.program());
    r.texel_copy = copy;
    Some(
        ts.iter()
            .map(|&t| {
                let f = r.render(&ev.evaluate(t), ev.program());
                (r.read(&f.texture), f.stats)
            })
            .collect(),
    )
}

/// Every channel of every pixel, as bits; a zero compares equal to a negative zero, which every later stage reads alike.
fn bits(px: &[[f32; 4]]) -> Vec<[u32; 4]> {
    px.iter().map(|p| p.map(|v| if v == 0.0 { 0 } else { v.to_bits() })).collect()
}

#[test]
fn a_frame_sized_vector_target_is_copied_with_the_same_pixels() {
    let d = scene(SHAPES);
    let Some(copied) = frames(&d, &[0.0], true) else { return };
    let Some(drawn) = frames(&d, &[0.0], false) else { return };
    assert_eq!(copied[0].1.texel_copies, 1, "the vector target is the first draw into the cleared frame");
    assert_eq!(drawn[0].1.texel_copies, 0);
    assert_eq!(copied[0].1.draws, drawn[0].1.draws, "a copied draw still counts as a draw");
    assert_eq!(bits(&copied[0].0), bits(&drawn[0].0));
}

#[test]
fn frames_that_reuse_the_cached_target_stay_the_same() {
    let d = scene(SHAPES);
    let ts = [0.0, 0.1, 0.2, 1.0, 1.0, 1.9];
    let Some(copied) = frames(&d, &ts, true) else { return };
    let Some(drawn) = frames(&d, &ts, false) else { return };
    for (k, (c, w)) in copied.iter().zip(&drawn).enumerate() {
        assert_eq!(bits(&c.0), bits(&w.0), "frame {k} at {} s", ts[k]);
        assert_eq!(c.1.texel_copies, 1, "frame {k}");
    }
}

#[test]
fn a_translucent_layer_is_drawn() {
    // at opacity 0.5 the draw scales every texel: no copy
    let d = scene(&format!(r#"<group id="g" opacity="0.5">{SHAPES}</group>"#));
    let Some(copied) = frames(&d, &[0.0], true) else { return };
    let Some(drawn) = frames(&d, &[0.0], false) else { return };
    assert_eq!(bits(&copied[0].0), bits(&drawn[0].0));
    let root_copy = copied[0].1.texel_copies;
    assert!(root_copy <= 1, "only the group's own content may be copied, into its offscreen: {root_copy}");
}

#[test]
fn a_target_partly_covered_keeps_its_cleared_pixels() {
    // a moved group's content is offset in the frame: whatever is copied, the pixels it does not cover stay clear
    let d = scene(&format!(r#"<group id="g" x="17" y="9">{SHAPES}</group>"#));
    let Some(copied) = frames(&d, &[0.0, 0.5], true) else { return };
    let Some(drawn) = frames(&d, &[0.0, 0.5], false) else { return };
    for (c, w) in copied.iter().zip(&drawn) {
        assert_eq!(bits(&c.0), bits(&w.0));
    }
}

#[test]
fn a_reused_target_is_cleared_where_the_copy_does_not_reach() {
    // a translucent group draws offscreen into a target kept across frames; its content moves, so last frame's pixels
    // lie outside this frame's copy and must be cleared
    let d = scene(
        r##"<group id="g" opacity="0.5"><shape id="m" shape="rect" x="2" y="3" width="12" height="10" fill="#FF8000FF">
  <animate property="x"><key time="0" value="2"/><key time="2" value="42"/></animate>
</shape></group>"##,
    );
    let ts = [0.0, 1.0, 1.5, 0.0];
    let Some(copied) = frames(&d, &ts, true) else { return };
    let Some(drawn) = frames(&d, &ts, false) else { return };
    for (k, (c, w)) in copied.iter().zip(&drawn).enumerate() {
        assert_eq!(bits(&c.0), bits(&w.0), "frame {k} at {} s", ts[k]);
    }
    assert!(copied.iter().any(|c| c.1.texel_copies > 0), "the group's content is copied into its offscreen");
}
