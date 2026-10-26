//! The tracer keeps its tile scratch (sums, guides, the denoiser's halves) between passes and grows it only when a
//! tile needs more, without a pass reading what the one before it left.

use super::common;

use glam::{Mat4, Vec3};
use sr_3d::camera::{resolve, CameraParams};
use sr_3d::{prim, MaterialParams};
use sr_gpu::pathtrace::{self, PathOpts, PtGpu, PtInputs};
use sr_gpu::three::{Draw3, Light3, LightKind, MeshSrc, Scene3, ThreeEngine};

fn scene(eng: &ThreeEngine, size: [u32; 2]) -> Scene3 {
    let mesh = prim::sphere(30.0, 16);
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
            pos: Vec3::new(20.0, 10.0, -120.0),
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
            size: [0.0, 0.0, 30.0],
            ies: None,
            affects_diffuse: true,
            affects_specular: true,
            contact: 0.0,
        }],
        env: None,
        stars: Vec::new(),
        star_overlay: None,
        splats: Vec::new(),
        volumes: Vec::new(),
        encode_srgb: false,
        ao: None,
        ssr: false,
        path: None,
        geodesic: None,
        planet_shell: None,
    }
}

#[test]
fn tile_scratch_is_kept_between_passes_and_grows_only_for_larger_tiles() {
    let Some(g) = common::gpu() else { return };
    let eng = ThreeEngine::new(g.device.clone(), g.queue.clone());
    let black = eng.upload_f16([1, 1], &[[0.0; 4]]).create_view(&Default::default());
    let sampler = g.device.create_sampler(&Default::default());
    let inputs =
        PtInputs { env: &black, backdrop: None, black: &black, sampler: &sampler, env_texels: None, aerial: None };
    let mut pt = PtGpu::new(&g.device, wgpu::TextureFormat::Rgba16Float);
    let pass = |pt: &PtGpu, size: [u32; 2], denoise: bool| {
        let sc = scene(&eng, size);
        let data = pathtrace::build(&sc);
        let opts = PathOpts { samples: 2, bounces: 1, denoise, noise: None };
        let target = eng.target(size);
        let mut enc = g.device.create_command_encoder(&Default::default());
        let view = target.create_view(&Default::default());
        let timing = pathtrace::render_timed(pt, &g.device, &mut enc, &sc, &data, opts, &inputs, &view, false);
        sr_gpu::gpu::submit(&g.queue, [enc.finish()]);
        (timing.scratch_buffers, eng.read(&target))
    };
    let (first, image) = pass(&pt, [128, 96], false);
    assert!(first > 0, "the first pass allocates its scratch");
    assert!(image.iter().any(|p| p[0] > 0.01 && p[3] > 0.9), "must render lit geometry");
    let (again, same) = pass(&pt, [128, 96], false);
    assert_eq!(again, 0, "a pass of the same size reuses the scratch");
    assert_eq!(image, same, "a reused scratch must not carry the previous pass into the next");
    let (denoised, _) = pass(&pt, [128, 96], true);
    assert!(denoised > 0 && denoised < first, "denoising adds its halves to the kept scratch");
    assert_eq!(pass(&pt, [128, 96], true).0, 0, "and they are kept too");
    assert!(pass(&pt, [160, 120], true).0 > 0, "a larger tile grows the scratch");
    assert_eq!(pass(&pt, [64, 64], true).0, 0, "a smaller tile fits the grown scratch");
    assert_eq!(pass(&pt, [160, 120], false).0, 0, "a pass without denoising fits what the denoised one grew");
    // a CPU adapter's buffers are host memory: made for each pass, never kept
    pt.cpu_adapter = true;
    assert_eq!(pass(&pt, [64, 64], false).0, 3, "a CPU adapter makes its scratch for the pass");
    assert_eq!(pass(&pt, [64, 64], false).0, 3, "and keeps none of it");
}
