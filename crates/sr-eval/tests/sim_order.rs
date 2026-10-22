//! A simulated document gives each frame the same graph whatever else was evaluated before it. Delivery evaluates
//! the next frame on a helper thread while the renderer asks for graphs around the current one (motion blur, time
//! effects), so a frame's graph must not depend on whether the frame after it was stepped first.

use sr_eval::Evaluator;

const FPS: f64 = 24.0;

const FLUIDS: &str = r##"<scene version="1.6"><project width="320" height="180" fps="24" duration="2" background="#000000FF" seed="4"/>
  <composition>
    <flock id="birds" width="160" height="180" count="300" seed="7" size="2" color="#FFFFFF" speed="90" maxSpeed="140"
           perception="20" separationDistance="6" bounds="wrap"/>
    <slime id="net" x="160" width="160" height="90" resolution="96" agents="4000" seed="3" spawn="random"/>
    <fluid id="smoke" x="160" y="90" width="160" height="90" resolution="48" vorticity="3" buoyancy="100" bounds="closed">
      <fluidSource x="80" y="80" radius="10" color="#FF7A2E" density="4" velocityY="-150"/>
    </fluid>
  </composition></scene>"##;

/// A sphere strikes a block that breaks into Voronoi pieces on a static ground, and a burst of 3D particles falls
/// onto it, in physical units: the rigid world, fracture and particle collisions.
const IMPACT: &str = r##"<scene version="1.6"><project width="320" height="180" fps="24" duration="3" background="#000000FF" seed="4"/>
  <materials><material id="m" baseColor="#808080FF"/></materials>
  <composition>
    <camera id="camera" x="14" y="-7" z="-40" fov="42"/>
    <object3D id="impactor" primitive="sphere" radius="2" x="-114.9" y="-181.2" material="m">
      <rigidBody shape="sphere" mass="90478" velocityX="50" velocityY="64.04" restitution="0"/>
    </object3D>
    <object3D id="block" primitive="box" width="6" height="6" depth="6" y="-3" material="m">
      <rigidBody mass="583200" restitution="0" friction="0.7"/>
      <fracture source="impactor" pieces="12" seed="20261006" interiorMaterial="m" energyFraction="0.3"/>
    </object3D>
    <object3D id="ground" primitive="box" width="200" height="1" depth="200" y="0.5" material="m">
      <rigidBody type="static" friction="0.7"/>
    </object3D>
    <particles3D id="debris" shape="sphere" segments="5" material="m" rate="0" seed="77" y="-5" size="0.22" speed="11"
                 spread="150" directionY="-1" gravityY="9.80665" lifetime="2.6" maxParticles="400" colliders="ground"
                 collisionRadius="0.15" bounce="0.2">
      <burst time="1.2" count="300"/>
    </particles3D>
  </composition>
  <physics gravityY="-9.80665" pixelsPerMeter="1" fixedStep="0.008333333333333333" bounds="none"/>
</scene>"##;

fn evaluator(xml: &str) -> Evaluator {
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
    let ev = Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"));
    assert!(ev.has_simulation());
    ev
}

/// Everything the graph at `t` holds, simulated state included: the serialised graph leaves out the simulations'
/// payloads (particles, fracture pieces, grid pictures), so the comparison is of the full debug form. Cache keys and
/// voxel grid lineages are masked: they are drawn from counters shared by everything in the process, so they name the
/// same content differently in two evaluators; the content they name is compared in full.
fn state(mut g: sr_eval::FrameGraph) -> String {
    // wall-clock seconds the solvers took, not state
    g.sim_seconds = sr_eval::SimSeconds::default();
    let mut text = format!("{g:?}");
    for field in ["key: ", "lineage: "] {
        let mut out = String::with_capacity(text.len());
        let mut rest = text.as_str();
        while let Some(at) = rest.find(field) {
            out.push_str(&rest[..at + field.len()]);
            rest = rest[at + field.len()..].trim_start_matches(|c: char| c.is_ascii_digit());
            out.push('_');
        }
        out.push_str(rest);
        text = out;
    }
    text
}

/// The graphs of frames `0..frames` and of the sub-frame samples a renderer takes around each, keyed by time, in
/// the order `order` lists the requests.
fn run(xml: &str, order: &[(f64, bool)]) -> Vec<(u64, bool, String)> {
    let ev = evaluator(xml);
    let mut out: Vec<_> = order
        .iter()
        .map(|&(t, frame)| {
            let g = ev.evaluate(t);
            assert!(g.failures.is_empty(), "{:?}", g.failures);
            (t.to_bits(), frame, state(g))
        })
        .collect();
    out.sort_by_key(|(t, frame, _)| (*t, *frame));
    out
}

/// Frames `0..frames` of `xml`, each followed by the renderer's samples a quarter frame either side, evaluated in
/// order and with a helper one frame ahead, must give the same graphs.
fn same_graphs_in_either_order(xml: &str, frames: usize, simulated: impl Fn(&sr_eval::FrameGraph) -> bool) {
    let last = evaluator(xml).evaluate((frames - 1) as f64 / FPS);
    assert!(simulated(&last), "the simulations have run by the last frame");
    let at = |k: usize| k as f64 / FPS;
    let around = |k: usize| [at(k) - 0.25 / FPS, at(k) + 0.25 / FPS];
    // serial: frame k, then the renderer's samples around it, then frame k + 1
    let mut serial = Vec::new();
    for k in 0..frames {
        serial.push((at(k), true));
        serial.extend(around(k).map(|t| (t.max(0.0), false)));
    }
    // a helper one frame ahead: frame k + 1 is evaluated before the renderer samples round frame k
    let mut ahead = vec![(at(0), true)];
    for k in 0..frames {
        if k + 1 < frames {
            ahead.push((at(k + 1), true));
        }
        ahead.extend(around(k).map(|t| (t.max(0.0), false)));
    }
    let (a, b) = (run(xml, &serial), run(xml, &ahead));
    assert_eq!(a.len(), b.len());
    for ((t, frame, x), (_, _, y)) in a.iter().zip(&b) {
        let at = x.bytes().zip(y.bytes()).position(|(p, q)| p != q).unwrap_or(x.len().min(y.len()));
        assert!(
            x == y,
            "the graph at {} ({}) depends on the order of evaluation, from byte {at}: {:?} against {:?}",
            f64::from_bits(*t),
            if *frame { "frame" } else { "sub-frame" },
            &x[at.saturating_sub(200)..(at + 200).min(x.len())],
            &y[at.saturating_sub(200)..(at + 200).min(y.len())],
        );
    }
}

#[test]
fn flocks_slime_and_fluids_give_each_frame_the_same_graph_evaluated_one_frame_ahead() {
    same_graphs_in_either_order(FLUIDS, 30, |g| {
        let node = |id: &str| g.nodes.iter().find(|n| &*n.id == id).unwrap_or_else(|| panic!("no node {id}"));
        // a flock draws as particles, grid simulations as a picture
        node("birds").particles.is_some() && node("net").sim_image.is_some() && node("smoke").sim_image.is_some()
    });
}

#[test]
fn rigid_bodies_fracture_and_particles_give_each_frame_the_same_graph_evaluated_one_frame_ahead() {
    same_graphs_in_either_order(IMPACT, 72, |g| {
        let broken = g.nodes.iter().any(|n| n.fracture.as_ref().is_some_and(|f| f.enabled.iter().any(|on| *on)));
        let debris = g.nodes.iter().any(|n| n.particles3d.is_some());
        broken && debris
    });
}

#[test]
fn every_simulated_corpus_document_gives_each_frame_the_same_graph_evaluated_one_frame_ahead() {
    // twelve output frames spread over each document, so that worlds which start late or break late are reached
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/valid");
    let mut paths: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).collect();
    paths.retain(|p| p.to_string_lossy().ends_with(".scene.xml"));
    paths.sort();
    let mut checked = Vec::new();
    for path in paths {
        let Ok(doc) = sr_model::load_file(&path, &sr_model::LoadOptions::default()) else { continue };
        let Ok(ev) = Evaluator::new(&doc, &Default::default()) else { continue };
        if !ev.has_simulation() {
            continue;
        }
        let fps = doc.fps().as_f64();
        let frames = 12usize;
        let span = (doc.duration() * fps).floor().max(frames as f64);
        let at = |k: usize| (k as f64 * span / frames as f64).floor() / fps;
        let around = |k: usize| [(at(k) - 0.25 / fps).max(0.0), at(k) + 0.25 / fps];
        let (mut serial, mut ahead) = (Vec::new(), vec![(at(0), true)]);
        for k in 0..frames {
            serial.push((at(k), true));
            serial.extend(around(k).map(|t| (t, false)));
            if k + 1 < frames {
                ahead.push((at(k + 1), true));
            }
            ahead.extend(around(k).map(|t| (t, false)));
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let order = |o: &[(f64, bool)]| {
            let ev = Evaluator::new(&doc, &Default::default()).unwrap();
            let mut out: Vec<_> = o.iter().map(|&(t, f)| (t.to_bits(), f, state(ev.evaluate(t)))).collect();
            out.sort_by_key(|(t, f, _)| (*t, *f));
            out
        };
        for ((t, _, x), (_, _, y)) in order(&serial).iter().zip(&order(&ahead)) {
            let at = x.bytes().zip(y.bytes()).position(|(p, q)| p != q).unwrap_or(x.len().min(y.len()));
            assert!(
                x == y,
                "{name}: the graph at {} depends on the order of evaluation, from byte {at}: {:?} against {:?}",
                f64::from_bits(*t),
                &x[at.saturating_sub(300)..(at + 200).min(x.len())],
                &y[at.saturating_sub(300)..(at + 200).min(y.len())],
            );
        }
        checked.push(name);
    }
    println!("{} simulated documents: {checked:?}", checked.len());
    assert!(checked.len() >= 10, "the corpus holds simulated documents: {checked:?}");
}
