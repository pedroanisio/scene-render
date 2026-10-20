//! The à-trous denoiser against a converged reference: it filters the light, not the surface (the colour is divided
//! by the albedo at the first hit before filtering and multiplied by the pixel's own after), and how far it smooths
//! follows the variance of each pixel's own samples, so a clean input stays clean.
//!
//! The scene is a floor whose albedo is a fine, low-contrast stripe texture, under a soft sphere light, with a ball
//! that casts a penumbra across it. The floor fills the frame, or with a smaller one its edges show the empty
//! background.

use super::common;

use glam::{Mat4, Vec3};
use sr_3d::camera::{resolve, CameraParams};
use sr_3d::{prim, MaterialParams};
use sr_gpu::pathtrace::PathOpts;
use sr_gpu::three::{Draw3, Light3, LightKind, MeshSrc, Scene3, ThreeEngine};

const SIZE: [u32; 2] = [96, 64];

fn engine() -> Option<ThreeEngine> {
    let g = common::gpu()?;
    Some(ThreeEngine::new(g.device.clone(), g.queue.clone()))
}

fn lum(p: [f32; 4]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

fn draw(mesh: MeshSrc, at: Vec3, material: MaterialParams) -> Draw3 {
    Draw3 {
        mesh,
        model: Mat4::from_translation(at),
        material,
        maps: Default::default(),
        opacity: 1.0,
        cast_shadow: true,
        receive_shadow: true,
        shadow_catcher: false,
    }
}

/// The floor is `floor` times the frame's size at its own depth, where a scene unit is 0.81 pixels: at 1.5 it fills
/// the frame, at 0.75 it is surrounded by the background.
fn scene(eng: &mut ThreeEngine, floor: f32) -> Scene3 {
    let [w, h] = SIZE;
    // texels 3 units (about 2.4 pixels) wide, alternating between two greys 0.05 apart in linear light
    let texels = (w as f32 * floor / 3.0).round() as u32;
    let stripes: Vec<u8> =
        (0..texels * 2).flat_map(|i| if i % 2 == 0 { [120, 120, 120, 255] } else { [135, 135, 135, 255] }).collect();
    let texture = eng.upload_rgba8(texels, 2, &stripes, true);
    let floor = prim::plane(w as f32 * floor, h as f32 * floor, 1);
    let floor = MeshSrc::Cached(eng.upload_mesh(&floor.vertices, &floor.indices));
    let mut floor = draw(
        floor,
        Vec3::new(w as f32 / 2.0, h as f32 / 2.0, 20.0),
        MaterialParams { base_color: [1.0; 4], roughness: 0.9, ..Default::default() },
    );
    floor.maps[0] = Some(texture);
    let ball = prim::sphere(9.0, 24);
    let ball = draw(
        MeshSrc::Cached(eng.upload_mesh(&ball.vertices, &ball.indices)),
        Vec3::new(w as f32 * 0.4, h as f32 * 0.45, 4.0),
        MaterialParams { base_color: [0.7, 0.3, 0.1, 1.0], roughness: 0.6, ..Default::default() },
    );
    Scene3 {
        size: SIZE,
        cam: resolve(&CameraParams::default(), w as f32, h as f32),
        clip_fix: Mat4::IDENTITY,
        exposure: 1.0,
        dof: None,
        lens_k1: 0.0,
        draws: vec![floor, ball],
        lights: vec![Light3 {
            kind: LightKind::Sphere,
            pos: Vec3::new(w as f32 * 0.8, h as f32 * 0.1, -40.0),
            dir: Vec3::Z,
            right: Vec3::X,
            color: Vec3::splat(40.0),
            range: 0.0,
            falloff: 2.0,
            cos_outer: 0.0,
            cos_inner: 0.0,
            cast_shadow: true,
            softness: 1.0,
            bias: 0.0005,
            map_size: 128,
            size: [0.0, 0.0, 12.0],
            ies: None,
            affects_diffuse: true,
            affects_specular: true,
            contact: 0.0,
        }],
        env: None,
        splats: Vec::new(),
        volumes: Vec::new(),
        encode_srgb: false,
        ao: None,
        ssr: false,
        path: None,
        geodesic: None,
    }
}

fn render(eng: &mut ThreeEngine, floor: f32, samples: u32, denoise: bool) -> Vec<[f32; 4]> {
    let mut s = scene(eng, floor);
    s.path = Some(PathOpts { samples, bounces: 2, denoise, noise: None });
    eng.render_now(&s, None)
}

fn luminance(eng: &mut ThreeEngine, samples: u32, denoise: bool) -> Vec<f32> {
    render(eng, 1.5, samples, denoise).into_iter().map(lum).collect()
}

/// The pixels compared: inside a margin of 2, where the second difference across a row is defined.
fn inner() -> impl Iterator<Item = usize> {
    let [w, h] = SIZE;
    (2..h - 2).flat_map(move |y| (2..w - 2).map(move |x| (y * w + x) as usize))
}

/// Root-mean-square difference relative to the reference's mean.
fn error(image: &[f32], reference: &[f32]) -> f64 {
    let (mut sq, mut level, mut n) = (0.0f64, 0.0f64, 0usize);
    for i in inner() {
        sq += f64::from(image[i] - reference[i]).powi(2);
        level += f64::from(reference[i]);
        n += 1;
    }
    (sq / n as f64).sqrt() / (level / n as f64)
}

/// How much of the reference's detail across a row an image keeps: its second difference projected onto the
/// reference's, 1 for the reference itself. Noise is uncorrelated with the reference and averages out of it.
fn detail(image: &[f32], reference: &[f32]) -> f64 {
    let hp = |px: &[f32], i: usize| f64::from(px[i] - 0.5 * (px[i - 1] + px[i + 1]));
    let (mut dot, mut norm) = (0.0, 0.0);
    for i in inner() {
        dot += hp(image, i) * hp(reference, i);
        norm += hp(reference, i).powi(2);
    }
    dot / norm
}

#[test]
fn the_denoiser_keeps_texture_the_light_does_not_vary_across() {
    let Some(mut eng) = engine() else { return };
    let reference = luminance(&mut eng, 4096, false);
    let noisy = luminance(&mut eng, 8, false);
    let denoised = luminance(&mut eng, 8, true);
    let (noisy_error, denoised_error) = (error(&noisy, &reference), error(&denoised, &reference));
    let kept = detail(&denoised, &reference);
    println!("8 samples: error noisy {noisy_error:.4}, denoised {denoised_error:.4}; detail kept {kept:.3}");
    assert!(detail(&reference, &reference) > 0.999);
    assert!(kept > 0.8, "the stripes of the albedo are blurred: {kept:.3} of their contrast is left");
    assert!(
        denoised_error < 0.5 * noisy_error,
        "error {denoised_error:.4} against {noisy_error:.4} without the denoiser"
    );
}

#[test]
fn on_a_clean_input_the_denoiser_does_no_harm() {
    let Some(mut eng) = engine() else { return };
    let reference = luminance(&mut eng, 4096, false);
    let noisy = luminance(&mut eng, 256, false);
    let denoised = luminance(&mut eng, 256, true);
    let (noisy_error, denoised_error) = (error(&noisy, &reference), error(&denoised, &reference));
    println!("256 samples: error noisy {noisy_error:.4}, denoised {denoised_error:.4}");
    assert!(denoised_error <= noisy_error, "error {denoised_error:.4} against {noisy_error:.4} without the denoiser");
}

/// Coverage is geometry, not light: at the floor's edges against the empty background the denoiser keeps each pixel's
/// own, and the colour there follows it.
#[test]
fn the_denoiser_keeps_the_silhouette_against_the_background() {
    let Some(mut eng) = engine() else { return };
    let reference = render(&mut eng, 0.75, 4096, false);
    let noisy = render(&mut eng, 0.75, 8, false);
    let denoised = render(&mut eng, 0.75, 8, true);
    let edge: Vec<usize> = (0..reference.len()).filter(|&i| reference[i][3] > 0.01 && reference[i][3] < 0.99).collect();
    assert!(edge.len() > 100, "the floor's edges are in the frame: {} pixels", edge.len());
    let alpha = |px: &[[f32; 4]]| px.iter().zip(&reference).map(|(a, r)| f64::from((a[3] - r[3]).abs())).sum::<f64>();
    let colour = |px: &[[f32; 4]]| edge.iter().map(|&i| f64::from((lum(px[i]) - lum(reference[i])).abs())).sum::<f64>();
    let ((noisy_alpha, noisy_colour), (alpha_error, colour_error)) =
        ((alpha(&noisy), colour(&noisy)), (alpha(&denoised), colour(&denoised)));
    println!("8 samples, edges: coverage error noisy {noisy_alpha:.2}, denoised {alpha_error:.2}; colour error noisy {noisy_colour:.2}, denoised {colour_error:.2}");
    assert!(alpha_error <= noisy_alpha, "the denoiser blurs the silhouette: {alpha_error:.2} against {noisy_alpha:.2}");
    assert!(colour_error < noisy_colour, "colour at the edges: {colour_error:.2} against {noisy_colour:.2}");
}
