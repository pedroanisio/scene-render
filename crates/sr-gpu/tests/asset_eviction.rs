//! Meshes and textures no frame draws are dropped once they exceed the 3D engine's idle budget.
mod common;
use common::{doc, gpu};
use sr_gpu::Renderer;

#[test]
fn a_mesh_no_frame_draws_is_dropped_once_the_idle_budget_is_spent() {
    let d = doc(
        r##"background="#101820""##,
        r##"<materials><material id="clay" baseColor="#808080" roughness="0.6"/></materials>"##,
        r##"<object3D id="box" primitive="box" width="30" height="20" depth="10" x="32" y="16" material="clay" end="1"/>
            <object3D id="ball" primitive="sphere" radius="10" x="32" y="16" material="clay" start="1"/>"##,
    );
    let Some(gpu) = gpu() else { return };
    let ev = sr_eval::Evaluator::new(&d, &sr_eval::EvalOptions::default()).unwrap();
    let mut r = Renderer::new(gpu, ev.program());
    // every cache budget rounds down to nothing: an asset no frame draws does not outlive the frame
    r.share_budgets(usize::MAX);
    let evicted: Vec<usize> = [0.2, 0.3, 1.2, 1.3]
        .iter()
        .map(|&t| {
            let g = ev.evaluate(t);
            r.render(&g, ev.program()).stats.assets_evicted
        })
        .collect();
    // the box is drawn by the first two frames and kept; the third draws the ball and drops the idle box
    assert_eq!(evicted, [0, 0, 1, 0]);
}
