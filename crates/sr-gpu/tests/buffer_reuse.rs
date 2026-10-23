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

#[test]
fn three_frames_under_f32_precision_reuse_their_f16_target_and_buffers() {
    let d = doc(
        r##"precision="f32" background="#101820""##,
        r##"<materials><material id="clay" baseColor="#808080" roughness="0.6"/></materials>"##,
        r##"<object3D id="box" primitive="box" width="30" height="20" depth="10" x="32" y="16" material="clay">
              <animate property="rotationY"><key time="0" value="0"/><key time="2" value="40"/></animate></object3D>"##,
    );
    let times: Vec<f64> = (0..4).map(|k| 0.2 + k as f64 / 10.0).collect();
    let Some(shots) = render_sub_frames(&d, &times) else { return };
    assert!(shots[1].px.iter().any(|p| p[3] > 0.0));
    let textures: Vec<usize> = shots.iter().map(|s| s.stats.textures_created).collect();
    assert_eq!(&textures[1..], &[0, 0, 0], "warm frames reuse the f16 target: {textures:?}");
    let made: Vec<usize> = shots.iter().map(|s| s.stats.buffers_created).collect();
    assert_eq!(&made[1..], &[0, 0, 0], "warm frames write the kept 3D buffers: {made:?}");
}
