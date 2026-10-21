//! Clay meshing performance: a seven-blob fingerprinted creature at resolution 160 is re-meshed
//! on every boil frame (12 times a second), so one extraction has to stay well inside a frame.
//! Release builds assert the bound; debug builds report the time without asserting.

use std::time::Instant;

use glam::{Quat, Vec3};
use sr_3d::clay::{mesh, Blob, BlobShape, Finish};

fn blob(shape: BlobShape, at: [f32; 3], radius: f32, blend: f32, subtract: bool) -> Blob {
    Blob {
        shape,
        center: Vec3::from(at),
        rotation: Quat::IDENTITY,
        radius,
        size: Vec3::splat(60.0),
        length: 34.0,
        blend,
        subtract,
    }
}

/// Body, head, two ears, two carved eye sockets and a second blob about to merge.
fn creature() -> Vec<Blob> {
    use BlobShape::{Capsule, Sphere};
    vec![
        blob(Sphere, [0., 0., 0.], 95., 10., false),
        blob(Sphere, [0., -118., 0.], 64., 34., false),
        blob(Capsule, [-38., -188., 0.], 15., 14., false),
        blob(Capsule, [38., -188., 0.], 15., 14., false),
        blob(Sphere, [-24., -128., -56.], 17., 6., true),
        blob(Sphere, [24., -128., -56.], 17., 6., true),
        blob(Sphere, [180., 64., 0.], 38., 42., false),
    ]
}

#[test]
fn a_fingerprinted_creature_meshes_at_resolution_160_under_150_ms() {
    let blobs = creature();
    let finish = Finish { amount: 0.55, seed: 9, boil: 12.0 };
    let mut best = f64::MAX;
    for frame in 0..3 {
        let t = Instant::now();
        let m = mesh(&blobs, &finish, 160, frame as f64 / 12.0);
        best = best.min(t.elapsed().as_secs_f64() * 1e3);
        assert!(m.indices.len() > 100_000, "the creature's surface is extracted");
    }
    eprintln!("clay creature at resolution 160: {best:.1} ms per extraction");
    if !cfg!(debug_assertions) {
        assert!(best < 150.0, "took {best:.1} ms");
    }
}
