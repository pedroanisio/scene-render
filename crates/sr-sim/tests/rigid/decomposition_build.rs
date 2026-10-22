//! A world whose bodies are closed meshes (the pieces of a fracture) decomposes each into convex parts when it is built.
//! The decompositions are independent of one another, so building them must not change the world: the bodies fall,
//! collide and come to rest exactly as the pinned run did. A fracture's pieces are all built at once, so the build of a
//! dozen of them stays well inside the frame that shows the break.
use std::time::Instant;

use sr_sim::{fields::Field, physics3d::*};

/// The closed triangle mesh of a box of half sizes `h`, outward windings.
fn cuboid(h: [f64; 3]) -> (Vec<[f64; 3]>, Vec<[u32; 3]>) {
    let points: Vec<[f64; 3]> =
        (0..8).map(|k| [0, 1, 2].map(|a| if (k >> a) & 1 == 1 { h[a] } else { -h[a] })).collect();
    let faces: [[u32; 3]; 12] = [
        [0, 2, 1],
        [1, 2, 3],
        [4, 5, 6],
        [5, 7, 6],
        [0, 1, 4],
        [1, 5, 4],
        [2, 6, 3],
        [3, 6, 7],
        [0, 4, 2],
        [2, 4, 6],
        [1, 3, 5],
        [3, 7, 5],
    ];
    (points, faces.to_vec())
}

fn body(kind: BodyKind, shape: Shape3, mass: f64, at: [f64; 3]) -> Body3Spec {
    Body3Spec {
        kind,
        shape,
        mass,
        friction: 0.6,
        restitution: 0.1,
        linear_damping: 0.0,
        angular_damping: 0.0,
        velocity: [0.; 3],
        angular_velocity: [0.; 3],
        group: 0,
        collides_with: None,
        sensor: false,
        fixed_rotation: false,
        bullet: false,
        activate_at: 0.,
        start: Pose3 { pos: at, ..Pose3::default() },
    }
}

struct Still;
impl Driver3 for Still {
    fn kinematic(&mut self, _: f64, which: &[usize]) -> Vec<Pose3> {
        vec![Pose3::default(); which.len()]
    }
    fn fields(&mut self, _: f64) -> Vec<Field> {
        vec![]
    }
}

/// A static ground and twelve mesh boxes of assorted sizes stacked in a loose column over it, gravity pulling them
/// down onto it and onto one another (scene axes: +y is down).
fn spec() -> World3Spec {
    let mut bodies = vec![body(BodyKind::Static, Shape3::Box([20.0, 0.5, 20.0]), 1.0, [0.0, 0.5, 0.0])];
    for k in 0..12 {
        let h = [0.3 + 0.05 * (k % 4) as f64, 0.25 + 0.04 * (k % 3) as f64, 0.35 - 0.03 * (k % 5) as f64];
        let (points, triangles) = cuboid(h);
        let at = [0.4 * ((k % 3) as f64 - 1.0), -1.0 - 0.9 * k as f64, 0.3 * ((k % 2) as f64 - 0.5)];
        bodies.push(body(BodyKind::Dynamic, Shape3::Decomposition(points, triangles), 50.0 + k as f64, at));
    }
    World3Spec {
        fix_internal_edges: false,
        start: 0.,
        step: 1.0 / 120.0,
        gravity: [0., -9.80665, 0.],
        pixels_per_meter: 1.,
        iterations: 8,
        bounds: Bounds3::None,
        joints: vec![],
        bodies,
    }
}

/// FNV-1a over the bits of every body's pose at 0.5, 1 and 2 s.
fn digest(w: &mut World3) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for t in [0.5, 1.0, 2.0] {
        let f = w.frame_at(t, &mut Still);
        assert!(f.errors.is_empty(), "{:?}", f.errors);
        for pose in &f.bodies {
            for v in pose.pos.iter().chain(pose.rot.iter()) {
                for b in v.to_bits().to_le_bytes() {
                    h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
                }
            }
        }
    }
    h
}

#[test]
fn mesh_bodies_fall_and_settle_as_the_pinned_run_did() {
    let mut w = World3::new(spec());
    let f = w.frame_at(2.0, &mut Still);
    // the lowest box has come down onto the ground, whose top is at y = 0
    assert!(f.bodies[1].pos[1] < 0.0 && f.bodies[1].pos[1] > -1.0, "{:?}", f.bodies[1].pos);
    let mut w = World3::new(spec());
    assert_eq!(digest(&mut w), 17_009_457_413_213_018_729);
}

#[test]
fn a_dozen_mesh_bodies_build_under_a_second() {
    let mut best = f64::MAX;
    for _ in 0..3 {
        let t = Instant::now();
        let w = World3::new(spec());
        best = best.min(t.elapsed().as_secs_f64() * 1e3);
        drop(w);
    }
    eprintln!("a world of twelve mesh bodies builds in {best:.1} ms");
    if !cfg!(debug_assertions) {
        assert!(best < 1000.0, "took {best:.1} ms");
    }
}
