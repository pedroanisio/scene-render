//! Step costs of the grid simulations at the size of a 1280 x 720 showcase panel (384 x 480 pixels), which steps them
//! 60 times a second of film: a 60 000-agent Physarum network on a 384-cell trail map and a 128-cell smoke solver.
//! Each is timed on one thread and on all of them, on the same machine; release builds on a machine of at least four
//! hardware threads assert that the step uses them, debug builds report the times without asserting.

use std::time::Instant;

use sr_sim::{fluid, slime};

/// The best of three runs of `steps` steps, in ms per step.
fn per_step(steps: u64, run: &mut impl FnMut(u64)) -> f64 {
    (0..3)
        .map(|_| {
            let t = Instant::now();
            for k in 0..steps {
                run(k);
            }
            t.elapsed().as_secs_f64() * 1e3 / steps as f64
        })
        .fold(f64::MAX, f64::min)
}

/// ms per step on one thread and on the default pool, and whether the machine has the threads to compare them on.
fn timed(steps: u64, mut run: impl FnMut(u64) + Send) -> (f64, f64, bool) {
    let one = rayon::ThreadPoolBuilder::new().num_threads(1).build().expect("a one-thread pool");
    let single = one.install(|| per_step(steps, &mut run));
    let many = per_step(steps, &mut run);
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    (single, many, !cfg!(debug_assertions) && threads >= 4)
}

#[test]
fn a_60000_agent_physarum_steps_on_every_core() {
    let s = slime::SlimeSpec {
        seed: 3,
        size: [384.0, 480.0],
        resolution: 384,
        agents: 60_000,
        spawn: slime::Spawn::Random,
        sensor_angle: 30.0,
        sensor_distance: 9.0,
        turn_angle: 45.0,
        speed: 60.0,
        deposit: 1.0,
        decay: 0.1,
        diffuse: 0.5,
    };
    let mut sl = slime::init(&s);
    let (single, many, assert) = timed(60, |k| slime::step(&s, &mut sl, k, 1.0 / 60.0));
    eprintln!("Physarum, 60 000 agents on 384 x 480 cells: {single:.2} ms per step on one thread, {many:.2} on all");
    if assert {
        assert!(many < 0.6 * single, "{many:.2} ms on all threads against {single:.2} on one");
    }
}

#[test]
fn a_128_cell_smoke_solver_steps_on_every_core() {
    let source = |x: f64, vx: f64| fluid::Source {
        pos: [x, 452.0],
        radius: 20.0,
        color: [1.0, 0.5, 0.2, 1.0],
        density: 5.0,
        velocity: [vx, -210.0],
        start: 0.0,
        end: None,
    };
    let s = fluid::FluidSpec {
        size: [384.0, 480.0],
        resolution: 128,
        viscosity: 0.0,
        diffusion: 0.0,
        dissipation: 0.12,
        velocity_dissipation: 0.0,
        vorticity: 4.0,
        buoyancy: 140.0,
        iterations: 40,
        edge: fluid::Edge::Closed,
        sources: vec![source(120.0, 45.0), source(268.0, -45.0)],
    };
    let mut f = fluid::init(&s);
    let dt = 1.0 / 60.0;
    let (single, many, assert) = timed(60, |k| fluid::step(&s, &mut f, k as f64 * dt, dt, &mut |_| [0.0, 0.0]));
    eprintln!("smoke, 128 cells across 384 x 480 pixels: {single:.2} ms per step on one thread, {many:.2} on all");
    if assert {
        assert!(many < 0.8 * single, "{many:.2} ms on all threads against {single:.2} on one");
    }
}
