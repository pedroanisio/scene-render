//! Compositor memory: vector raster targets, transient GPU memory and decoded images stay bounded, and the bounds
//! change no pixel.
//!
//! A run of simple shapes between two image layers rasterises into a target the size of the frame. Keeping one such
//! target per run for the whole frame (and four frames after) costs 265 MB per run at 8K.

mod common;
use common::*;

use sr_gpu::{Frame, Renderer};

const W: u32 = 256;
const H: u32 = 144;
/// An RGBA16F frame.
const FRAME_BYTES: u64 = W as u64 * H as u64 * 8;

fn alternating(runs: usize) -> sr_model::Document {
    let mut body = String::new();
    for k in 0..runs {
        let (x, y) = ((k * 19) % 220, (k * 11) % 120);
        body += &format!(r#"<layer id="i{k}" asset="red" x="{x}" y="{y}" scaleX="6" scaleY="6"/>"#);
        body += &format!(
            r##"<shape id="s{k}" shape="rect" x="{}" y="{}" width="30" height="20" fill="#00FF0080"/>"##,
            x + 9,
            y + 7
        );
    }
    doc(&format!(r#"width="{W}" height="{H}""#), "", &body)
}

fn blended(shapes: usize) -> sr_model::Document {
    let mut body = String::from(r#"<layer id="bg" asset="quad" scaleX="128" scaleY="72"/>"#);
    for k in 0..shapes {
        let (x, y) = ((k * 23) % 230, (k * 13) % 124);
        body += &format!(
            r##"<shape id="e{k}" shape="ellipse" x="{x}" y="{y}" width="{}" height="18" fill="#3366CCB0" blend="multiply"/>"##,
            12 + k % 7
        );
    }
    doc(&format!(r#"width="{W}" height="{H}""#), "", &body)
}

fn renderer(d: &sr_model::Document, setup: impl FnOnce(&mut Renderer)) -> Option<(Renderer, sr_eval::Evaluator)> {
    let gpu = gpu()?;
    let ev = sr_eval::Evaluator::new(d, &sr_eval::EvalOptions::default()).unwrap();
    let mut r = Renderer::new(gpu, ev.program());
    setup(&mut r);
    Some((r, ev))
}

fn frame(r: &mut Renderer, ev: &sr_eval::Evaluator, t: f64) -> (Frame, Vec<[f32; 4]>) {
    let f = r.render(&ev.evaluate(t), ev.program());
    let px = r.read(&f.texture);
    assert!(f.stats.errors.is_empty(), "{:?}", f.stats.errors);
    (f, px)
}

#[test]
fn alternating_images_and_shapes_keep_vector_targets_within_three_frames() {
    let d = alternating(12);
    let Some((mut all, ev)) = renderer(&d, |r| r.vector_cache_budget = Some(u64::MAX)) else { return };
    // every run kept, as before the bound: the scene does make a frame-size target per run
    let (kept, want) = frame(&mut all, &ev, 0.0);
    assert!(
        kept.stats.peak_vector_target_bytes >= 12 * FRAME_BYTES,
        "the scene should have 12 runs: {} bytes",
        kept.stats.peak_vector_target_bytes
    );
    assert_eq!(kept.stats.vector_targets_transient, 0);

    let (mut r, ev) = renderer(&d, |_| {}).unwrap();
    for t in [0.0, 0.0, 0.5] {
        let (f, px) = frame(&mut r, &ev, t);
        assert!(
            f.stats.peak_vector_target_bytes <= 3 * FRAME_BYTES,
            "t={t}: {} bytes of vector targets alive at once ({} frames)",
            f.stats.peak_vector_target_bytes,
            f.stats.peak_vector_target_bytes as f64 / FRAME_BYTES as f64
        );
        assert!(px == want, "t={t}: pixels differ from the renderer that keeps every run");
    }
    let (f, _) = frame(&mut r, &ev, 0.0);
    assert!(f.stats.vector_targets_transient >= 10, "{} transient runs", f.stats.vector_targets_transient);
}

#[test]
fn transient_blended_shapes_and_mid_frame_submits_keep_pixels() {
    let d = blended(40);
    let Some((mut all, ev)) = renderer(&d, |r| r.vector_cache_budget = Some(u64::MAX)) else { return };
    let (kept, want) = frame(&mut all, &ev, 0.0);
    assert_eq!(kept.stats.vector_targets_transient, 0);
    assert_eq!(kept.stats.mid_frame_submits, 0);

    let (mut r, ev) = renderer(&d, |r| {
        r.vector_cache_budget = Some(0);
        r.transient_submit_bytes = 1;
    })
    .unwrap();
    for t in [0.0, 0.0] {
        let (f, px) = frame(&mut r, &ev, t);
        assert!(f.stats.vector_targets_transient >= 40, "{} transient shapes", f.stats.vector_targets_transient);
        assert!(f.stats.mid_frame_submits > 0, "no submission freed transient memory");
        // one transient shape target alive at a time
        assert!(f.stats.peak_vector_target_bytes <= 24 * 24 * 8, "{} bytes", f.stats.peak_vector_target_bytes);
        assert!(px == want, "t={t}: pixels differ from the renderer that keeps every shape");
    }
}

#[test]
fn images_no_longer_drawn_are_evicted_like_other_cached_pictures() {
    // 'a' leaves the timeline at 0.05 s and is not drawn after that (a layer merely at opacity 0 with media time
    // is still drawn and keeps its picture in use)
    let body = r#"<layer id="a" asset="wide" scaleX="8" scaleY="8" end="0.05"/>
<layer id="b" asset="red" x="40" scaleX="4" scaleY="4"/>"#;
    let d = doc(r#"width="64" height="32""#, "", body);
    let Some((mut r, ev)) = renderer(&d, |_| {}) else { return };
    let (_, first) = frame(&mut r, &ev, 0.0);
    let before = r.cached_textures();
    for k in 0..6 {
        frame(&mut r, &ev, 0.5 + k as f64 * 0.1);
    }
    let after = r.cached_textures();
    assert!(after < before, "{before} cached pictures before, {after} after 'a' stopped drawing");
    // and one that comes back is decoded again, to the same pixels
    let (_, again) = frame(&mut r, &ev, 0.0);
    assert!(again == first);
}
