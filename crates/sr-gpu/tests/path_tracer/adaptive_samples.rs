//! Adaptive sampling: with a noise target, a pixel stops tracing once its own samples say its mean is that close, and
//! `pathSamples` caps the rest. The scene is a lit sphere (noisy) on an empty background (no variance at all).

use super::common;

use glam::{Mat4, Vec3};
use sr_3d::camera::{resolve, CameraParams};
use sr_3d::{prim, MaterialParams};
use sr_gpu::pathtrace::{self, PathOpts, PtGpu, PtInputs};
use sr_gpu::three::{Draw3, Light3, LightKind, MeshSrc, Scene3, ThreeEngine};

fn scene(eng: &ThreeEngine, size: [u32; 2]) -> Scene3 {
    let mesh = prim::sphere(0.3 * size[1] as f32, 24);
    Scene3 {
        size,
        cam: resolve(&CameraParams::default(), size[0] as f32, size[1] as f32),
        clip_fix: Mat4::IDENTITY,
        exposure: 1.0,
        dof: None,
        lens_k1: 0.0,
        draws: vec![Draw3 {
            mesh: MeshSrc::Cached(eng.upload_mesh(&mesh.vertices, &mesh.indices)),
            model: Mat4::from_translation(Vec3::new(size[0] as f32 / 2.0, size[1] as f32 / 2.0, 10.0)),
            material: MaterialParams { base_color: [0.7, 0.3, 0.1, 1.0], roughness: 0.6, ..Default::default() },
            maps: Default::default(),
            opacity: 1.0,
            cast_shadow: true,
            receive_shadow: true,
            shadow_catcher: false,
        }],
        lights: vec![Light3 {
            kind: LightKind::Sphere,
            pos: Vec3::new(size[0] as f32 * 0.2, 0.0, -170.0),
            dir: Vec3::Z,
            right: Vec3::X,
            color: Vec3::splat(200.0),
            range: 0.0,
            falloff: 2.0,
            cos_outer: 0.0,
            cos_inner: 0.0,
            cast_shadow: true,
            softness: 1.0,
            bias: 0.0005,
            map_size: 128,
            size: [0.0, 0.0, 40.0],
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

struct Run {
    image: Vec<[f32; 4]>,
    traced: u64,
    budget: u64,
}

fn run(g: &sr_gpu::Gpu, eng: &ThreeEngine, pt: &PtGpu, sc: &Scene3, opts: PathOpts) -> Run {
    let black = eng.upload_f16([1, 1], &[[0.0; 4]]).create_view(&Default::default());
    let sampler = g.device.create_sampler(&Default::default());
    let inputs = PtInputs { env: &black, backdrop: None, black: &black, sampler: &sampler };
    let data = pathtrace::build(sc);
    let target = eng.target(sc.size);
    let mut enc = g.device.create_command_encoder(&Default::default());
    let timing = pathtrace::render_timed(
        pt,
        &g.device,
        &mut enc,
        sc,
        &data,
        opts,
        &inputs,
        &target.create_view(&Default::default()),
        false,
    );
    g.queue.submit([enc.finish()]);
    let image = eng.read(&target);
    Run { image, traced: timing.samples.traced(&g.device), budget: timing.samples.budget }
}

fn lum(p: [f32; 4]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

#[test]
fn without_a_noise_target_every_pixel_traces_every_sample() {
    let Some(g) = common::gpu() else { return };
    let eng = ThreeEngine::new(g.device.clone(), g.queue.clone());
    let pt = PtGpu::new(&g.device, wgpu::TextureFormat::Rgba16Float);
    let sc = scene(&eng, [64, 48]);
    let r = run(&g, &eng, &pt, &sc, PathOpts { samples: 18, bounces: 2, denoise: false, noise: None });
    assert_eq!(r.budget, 64 * 48 * 18);
    assert_eq!(r.traced, r.budget);
}

#[test]
fn a_cap_at_the_minimum_traces_every_sample() {
    let Some(g) = common::gpu() else { return };
    let eng = ThreeEngine::new(g.device.clone(), g.queue.clone());
    let pt = PtGpu::new(&g.device, wgpu::TextureFormat::Rgba16Float);
    let sc = scene(&eng, [64, 48]);
    let plain = run(&g, &eng, &pt, &sc, PathOpts { samples: 16, bounces: 2, denoise: false, noise: None });
    let r = run(&g, &eng, &pt, &sc, PathOpts { samples: 16, bounces: 2, denoise: false, noise: Some(0.5) });
    assert_eq!(r.traced, r.budget, "no pixel is tested before 16 samples");
    assert_eq!(r.image, plain.image);
}

#[test]
fn a_target_the_lit_pixels_cannot_meet_leaves_the_image_unchanged() {
    let Some(g) = common::gpu() else { return };
    let eng = ThreeEngine::new(g.device.clone(), g.queue.clone());
    let pt = PtGpu::new(&g.device, wgpu::TextureFormat::Rgba16Float);
    let sc = scene(&eng, [64, 48]);
    for denoise in [false, true] {
        let plain = run(&g, &eng, &pt, &sc, PathOpts { samples: 40, bounces: 2, denoise, noise: None });
        let tight = run(&g, &eng, &pt, &sc, PathOpts { samples: 40, bounces: 2, denoise, noise: Some(1e-7) });
        let differ: Vec<_> =
            tight.image.iter().zip(&plain.image).enumerate().filter(|(_, (a, b))| a != b).take(8).collect();
        assert!(differ.is_empty(), "denoise {denoise}: {differ:?}");
        // not even the empty background stops: the deviation a pixel is assumed to have needs 1 / noise samples to fall
        assert_eq!(tight.traced, tight.budget);
    }
}

#[test]
fn converged_pixels_stop_early_and_keep_their_mean() {
    let Some(g) = common::gpu() else { return };
    let eng = ThreeEngine::new(g.device.clone(), g.queue.clone());
    let pt = PtGpu::new(&g.device, wgpu::TextureFormat::Rgba16Float);
    let size = [96, 64];
    let sc = scene(&eng, size);
    let full = run(&g, &eng, &pt, &sc, PathOpts { samples: 256, bounces: 2, denoise: false, noise: None });
    let noise = 0.05;
    let adaptive = run(&g, &eng, &pt, &sc, PathOpts { samples: 256, bounces: 2, denoise: false, noise: Some(noise) });
    let pixels = u64::from(size[0] * size[1]);
    assert_eq!(adaptive.budget, pixels * 256);
    assert!(adaptive.traced * 10 < adaptive.budget * 6, "{} of {}", adaptive.traced, adaptive.budget);
    assert!(adaptive.traced > pixels * 16, "the lit pixels trace past the minimum: {}", adaptive.traced);
    let (mut lit, mut error) = (0usize, 0.0f64);
    for (a, f) in adaptive.image.iter().zip(&full.image) {
        if f[3] == 0.0 {
            assert_eq!(a, f, "the background stays exactly empty");
            continue;
        }
        if f[3] > 0.99 && lum(*f) > 0.05 {
            lit += 1;
            error += f64::from((lum(*a) - lum(*f)).abs() / lum(*f));
        }
    }
    assert!(lit > 200, "the sphere is lit: {lit} pixels");
    // each pixel stops within the target of its own mean, and the 256-sample one carries a smaller error of its own
    let mean = error / lit as f64;
    assert!(mean < 2.0 * f64::from(noise), "mean relative difference {mean} against the full 256 samples");
}

/// A pixel crossed by the sphere's edge on few samples, none of the first 16, keeps them: the coverage at the edge is
/// the full render's.
#[test]
fn edge_pixels_hit_on_few_samples_keep_their_coverage() {
    let Some(g) = common::gpu() else { return };
    let eng = ThreeEngine::new(g.device.clone(), g.queue.clone());
    let pt = PtGpu::new(&g.device, wgpu::TextureFormat::Rgba16Float);
    let sc = scene(&eng, [96, 64]);
    let full = run(&g, &eng, &pt, &sc, PathOpts { samples: 256, bounces: 2, denoise: false, noise: None });
    let adaptive = run(&g, &eng, &pt, &sc, PathOpts { samples: 256, bounces: 2, denoise: false, noise: Some(0.05) });
    let coverage = |r: &Run| r.image.iter().map(|p| f64::from(p[3])).sum::<f64>();
    let edge = |r: &Run| r.image.iter().filter(|p| p[3] > 0.0 && p[3] < 0.99).count();
    let (a, f) = (coverage(&adaptive), coverage(&full));
    assert!((a - f).abs() < 0.002 * f, "total coverage {a} against {f}");
    assert!(edge(&adaptive) * 10 >= edge(&full) * 9, "edge pixels {} against {}", edge(&adaptive), edge(&full));
}

#[test]
fn tiled_adaptive_render_matches_the_whole_frame() {
    let Some(g) = common::gpu() else { return };
    let eng = ThreeEngine::new(g.device.clone(), g.queue.clone());
    let mut pt = PtGpu::new(&g.device, wgpu::TextureFormat::Rgba16Float);
    let sc = scene(&eng, [257, 193]);
    for denoise in [false, true] {
        let opts = PathOpts { samples: 40, bounces: 2, denoise, noise: Some(0.05) };
        pt.limit_buffers(32 << 20).unwrap();
        let whole = run(&g, &eng, &pt, &sc, opts);
        pt.limit_buffers(192 * 192 * 32).unwrap();
        let tiled = run(&g, &eng, &pt, &sc, opts);
        assert!(whole.traced < whole.budget, "the background converges");
        for (index, (a, b)) in whole.image.iter().zip(&tiled.image).enumerate() {
            assert_eq!(a, b, "pixel {index}, denoise {denoise}");
        }
    }
}
