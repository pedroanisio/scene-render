//! Warm frames write the buffers the engines keep rather than making new ones.
mod common;
use common::{doc, render_sub_frames};

/// Renders four frames of `d` and asserts only the first makes buffers.
fn warm_frames_make_no_buffers(name: &str, d: &sr_model::Document) {
    let times: Vec<f64> = (0..4).map(|k| 0.2 + k as f64 / 10.0).collect();
    let Some(shots) = render_sub_frames(d, &times) else { return };
    let made: Vec<usize> = shots.iter().map(|s| s.stats.buffers_created).collect();
    assert!(made[0] > 0, "{name}: the first frame makes its buffers: {made:?}");
    assert_eq!(&made[1..], &[0, 0, 0], "{name}: warm frames write the kept ones: {made:?}");
}

#[test]
fn particle_frames_write_the_instance_buffers_the_engine_keeps() {
    let d = doc(
        r##"background="#101820""##,
        "",
        r##"<particleEmitter id="p" x="32" y="16" emitterShape="point" rate="0" lifetime="3" speed="12" spread="360"
                 size="3" color="#FFFFFF" shape="disc" seed="7"><burst time="0" count="30"/></particleEmitter>"##,
    );
    warm_frames_make_no_buffers("particles", &d);
}
