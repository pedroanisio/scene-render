//! The scale draft's rule 1 at the camera: within SC1 the projection keeps the document's near and far (no clamp),
//! the reversed mapping is `sr_3d::scale::depth` and resolves a relative gap of 1e-5 at every decade up to 1e19, and
//! a logarithmic camera's buffer value is the draft's ln(z / n) / ln(f / n).

use glam::Vec4;
use sr_3d::camera::{resolve, CameraParams, CameraView};
use sr_3d::scale::{depth, DepthMode};

fn camera(near: f32, far: f32, mode: DepthMode) -> CameraView {
    resolve(&CameraParams { near, far, depth: mode, ..Default::default() }, 640.0, 360.0)
}

/// The depth the projection gives a point on the view axis at distance `z`.
fn projected(c: &CameraView, z: f32) -> f32 {
    let v = c.proj * Vec4::new(0.0, 0.0, z, 1.0);
    v.z / v.w
}

#[test]
fn within_sc1_near_and_far_are_the_document_s() {
    for (n, f) in [(1e-6f32, 1e4f32), (0.1, 1e19), (10.0, 1e21), (1e-4, 1e16), (1.0, 2.0)] {
        let c = camera(n, f, DepthMode::Reversed);
        assert_eq!((c.near, c.far), (n, f));
        assert!((projected(&c, n) - 1.0).abs() < 1e-6, "{n} {f}: {}", projected(&c, n));
        assert!(projected(&c, f).abs() < 1e-6, "{n} {f}: {}", projected(&c, f));
    }
}

#[test]
fn outside_sc1_the_planes_stay_usable() {
    for (n, f) in [
        (0.0f32, 100.0f32),
        (-1.0, 100.0),
        (10.0, 5.0),
        (5.0, 5.0),
        (1e-20, 1e19),
        (f32::NAN, 1.0),
        (1.0, f32::INFINITY),
    ] {
        let c = camera(n, f, DepthMode::Reversed);
        assert!(c.near > 0.0 && c.far > c.near && c.far.is_finite() && c.proj.is_finite(), "{n} {f}: {c:?}");
    }
    // the fallback's floor: near at least 1e-4
    assert_eq!(camera(0.0, 100.0, DepthMode::Reversed).near, 1e-4);
}

#[test]
fn the_reversed_mapping_is_rule_one_s_and_resolves_every_decade() {
    let (n, f) = (0.1f32, 1e19f32);
    let c = camera(n, f, DepthMode::Reversed);
    for k in 0..19 {
        let z = 10f32.powi(k);
        let want = depth(z as f64, n as f64, f as f64, DepthMode::Reversed).unwrap();
        let got = projected(&c, z) as f64;
        assert!((got / want - 1.0).abs() < 4e-7, "1e{k}: {got} against {want}");
        assert_eq!(c.depth_value(z) as f64, want as f32 as f64, "1e{k}");
        // two surfaces one part in 1e5 apart keep their order
        assert!(projected(&c, z) > projected(&c, z * (1.0 + 1e-5)), "1e{k}");
    }
}

#[test]
fn a_logarithmic_camera_s_buffer_value_is_ln_z_over_n_over_ln_f_over_n() {
    let (n, f) = (0.1f32, 1e19f32);
    let c = camera(n, f, DepthMode::Logarithmic);
    assert_eq!(c.depth, DepthMode::Logarithmic);
    assert_eq!(c.depth_value(n), 0.0);
    assert_eq!(c.depth_value(f), 1.0);
    for k in 0..19 {
        let z = 10f32.powi(k);
        let want = depth(z as f64, n as f64, f as f64, DepthMode::Logarithmic).unwrap();
        assert!((c.depth_value(z) as f64 - want).abs() < 1e-7, "1e{k}: {} against {want}", c.depth_value(z));
    }
    assert_eq!(camera(n, f, DepthMode::Reversed).depth, DepthMode::Reversed);
}

#[test]
fn an_orthographic_camera_maps_near_to_one_and_far_to_zero() {
    let c = resolve(
        &CameraParams { orthographic: true, near: 1.0, far: 1e9, ortho_height: Some(2000.0), ..Default::default() },
        640.0,
        360.0,
    );
    assert_eq!((c.near, c.far), (1.0, 1e9));
    assert!((projected(&c, 1.0) - 1.0).abs() < 1e-6 && projected(&c, 1e9).abs() < 1e-6);
    assert!((c.depth_value(5e8) - 0.5).abs() < 1e-6);
    let near = resolve(&CameraParams { orthographic: true, near: 1.0, far: 3.0, ..Default::default() }, 640.0, 360.0);
    assert!((projected(&near, 1.0) - 1.0).abs() < 1e-6 && projected(&near, 3.0).abs() < 1e-6);
}
