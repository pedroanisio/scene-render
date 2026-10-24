//! Document version 1.8 (the random-streams draft): a simulation draws from the D24 streams S(e, p, t, i) with the
//! element seed e = E(project, id, @seed, "stream") (section 5); a document of 1.7 or before keeps the legacy draws
//! of Appendix A.

use sr_eval::{EvalOptions, Evaluator};
use sr_sim::particles::purpose;
use sr_vector::d24::{element_seed, stream};

/// Eight particles of a burst at rest, born on a 40 x 20 rectangle around (100, 50), with `seed="3"`.
fn births(version: &str, project_seed: &str) -> Vec<[f32; 2]> {
    let xml = format!(
        r#"<scene version="{version}"><project width="200" height="100" fps="30" duration="2"{project_seed}/><composition>
        <particleEmitter id="p" x="100" y="50" emitterShape="rect" emitterWidth="40" emitterHeight="20" seed="3" rate="0" speed="0" lifetime="10"><burst time="0" count="8"/></particleEmitter>
        </composition></scene>"#
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = Evaluator::new(&doc, &EvalOptions::default()).unwrap();
    let f = ev.evaluate(0.8);
    f.nodes.iter().find(|n| &*n.id == "p").unwrap().particles.as_ref().unwrap().pos.clone()
}

fn rect_point(u: f64, w: f64) -> [f32; 2] {
    [(100.0 + (u - 0.5) * 40.0) as f32, (50.0 + (w - 0.5) * 20.0) as f32]
}

fn assert_near(got: &[[f32; 2]], want: &[[f32; 2]]) {
    assert_eq!(got.len(), want.len());
    for (g, w) in got.iter().zip(want) {
        assert!((g[0] - w[0]).abs() < 1e-3 && (g[1] - w[1]).abs() < 1e-3, "{g:?} != {w:?}");
    }
}

#[test]
fn a_1_8_emitter_births_its_particles_at_the_stream_draws_of_its_element_seed() {
    for (project, attr) in [(0, ""), (5, r#" seed="5""#)] {
        let e = element_seed(project, "p", Some(3), "stream");
        let want: Vec<_> =
            (0..8).map(|k| rect_point(stream(e, purpose::SHAPE_U, 0, k), stream(e, purpose::SHAPE_V, 0, k))).collect();
        assert_near(&births("1.8", attr), &want);
    }
}

#[test]
fn a_1_7_emitter_keeps_the_legacy_draws_of_its_seed() {
    let want: Vec<_> = (0..8)
        .map(|k| rect_point(sr_sim::rng::unit(3, k, purpose::SHAPE_U as u64), sr_sim::rng::unit(3, k, 12)))
        .collect();
    for v in ["1.3", "1.7"] {
        assert_near(&births(v, ""), &want);
        // an explicit @seed ignores the project seed before 1.8
        assert_near(&births(v, r#" seed="5""#), &want);
    }
    assert_ne!(births("1.7", ""), births("1.8", ""));
}

#[test]
fn a_1_8_emitter_rendered_alone_at_a_frame_equals_the_played_frame() {
    let xml = r#"<scene version="1.8"><project width="200" height="100" fps="30" duration="2"/><composition>
        <particleEmitter id="p" x="100" y="50" emitterShape="ellipse" emitterWidth="60" emitterHeight="30" seed="3" rate="40" speed="80" speedVariance="30" spread="90" lifetime="1.5" turbulence="20"><burst time="0" count="8"/></particleEmitter>
        </composition></scene>"#;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let played = Evaluator::new(&doc, &EvalOptions::default()).unwrap();
    let mut at24 = None;
    for frame in 0..=24 {
        at24 = Some(played.evaluate(frame as f64 / 30.0));
    }
    let alone = Evaluator::new(&doc, &EvalOptions::default()).unwrap().evaluate(24.0 / 30.0);
    let pos = |f: &sr_eval::FrameGraph| {
        f.nodes.iter().find(|n| &*n.id == "p").unwrap().particles.as_ref().unwrap().pos.clone()
    };
    let p = pos(&at24.unwrap());
    assert!(p.len() > 8);
    assert_eq!(p, pos(&alone));
}
