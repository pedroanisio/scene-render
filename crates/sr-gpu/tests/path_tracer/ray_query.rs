//! Hardware ray traversal: on a device with ray queries the path tracer finds its hits through an acceleration
//! structure the driver builds, and elsewhere, or when it is switched off, through its own BVH. Both trace the same
//! paths with the same random numbers, so their images agree up to the few rays whose nearest hit differs by rounding.
//!
//! The scene has every kind of primitive the tracer intersects: world triangles (a floor and a ball), instances of a
//! shared prototype (repeated cubes) and Gaussian splats, under a sphere light whose shadow rays cross all of them.

use super::common;

use glam::{Mat4, Quat, Vec3};
use sr_3d::camera::{resolve, CameraParams};
use sr_3d::{prim, MaterialParams};
use sr_gpu::pathtrace::PathOpts;
use sr_gpu::three::{Draw3, Light3, LightKind, MeshSrc, Scene3, SplatDraw, ThreeEngine};

const SIZE: [u32; 2] = [96, 64];

fn engine() -> Option<(ThreeEngine, bool)> {
    let g = common::gpu()?;
    let has = g.device.features().contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY);
    Some((ThreeEngine::new(g.device.clone(), g.queue.clone()), has))
}

fn lum(p: [f32; 4]) -> f64 {
    f64::from(0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2])
}

fn draw(mesh: MeshSrc, model: Mat4, base_color: [f32; 4]) -> Draw3 {
    Draw3 {
        mesh,
        model,
        material: MaterialParams { base_color, roughness: 0.7, ..Default::default() },
        maps: Default::default(),
        opacity: 1.0,
        cast_shadow: true,
        receive_shadow: true,
        shadow_catcher: false,
    }
}

fn scene(eng: &ThreeEngine, samples: u32) -> Scene3 {
    let [w, h] = SIZE.map(|v| v as f32);
    let plane = prim::plane(w * 0.9, h * 0.9, 1);
    let floor = draw(
        MeshSrc::Cached(eng.upload_mesh(&plane.vertices, &plane.indices)),
        Mat4::from_translation(Vec3::new(w / 2.0, h / 2.0, 20.0)),
        [0.8, 0.8, 0.8, 1.0],
    );
    let ball = prim::sphere(8.0, 24);
    let ball = draw(
        MeshSrc::Cached(eng.upload_mesh(&ball.vertices, &ball.indices)),
        Mat4::from_translation(Vec3::new(w * 0.3, h * 0.4, 6.0)),
        [0.7, 0.3, 0.1, 1.0],
    );
    let cube = prim::cuboid(6.0, 6.0, 6.0);
    let cube = eng.upload_mesh(&cube.vertices, &cube.indices);
    let mut draws = vec![floor, ball];
    draws.extend((0..6).map(|i| {
        draw(
            MeshSrc::Cached(cube.clone()),
            Mat4::from_scale_rotation_translation(
                Vec3::splat(0.8 + 0.1 * i as f32),
                Quat::from_rotation_z(0.3 * i as f32) * Quat::from_rotation_x(0.5),
                Vec3::new(w * (0.55 + 0.07 * i as f32), h * (0.25 + 0.1 * (i % 3) as f32), 10.0),
            ),
            [0.2, 0.4 + 0.08 * i as f32, 0.7, 1.0],
        )
    }));
    let mut cloud = sr_3d::Splats { basis: Mat4::IDENTITY, ..Default::default() };
    for i in 0..12 {
        cloud.pos.push([w * (0.2 + 0.05 * i as f32), h * 0.78, 8.0 + i as f32]);
        cloud.scale.push([2.5, 1.5, 2.0]);
        cloud.rot.push([0.0, 0.0, 0.0, 1.0]);
        cloud.color.push([0.9, 0.8 - 0.05 * i as f32, 0.2, 0.9]);
    }
    Scene3 {
        size: SIZE,
        cam: resolve(&CameraParams::default(), w, h),
        clip_fix: Mat4::IDENTITY,
        exposure: 1.0,
        dof: None,
        lens_k1: 0.0,
        draws,
        lights: vec![Light3 {
            kind: LightKind::Sphere,
            pos: Vec3::new(w * 0.8, h * 0.1, -40.0),
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
            size: [0.0, 0.0, 10.0],
            ies: None,
            affects_diffuse: true,
            affects_specular: true,
            contact: 0.0,
        }],
        env: None,
        stars: Vec::new(),
        splats: vec![SplatDraw { gpu: eng.upload_splats(&cloud), model: Mat4::IDENTITY, opacity: 1.0 }],
        volumes: Vec::new(),
        encode_srgb: false,
        ao: None,
        ssr: false,
        path: Some(PathOpts { samples, bounces: 3, denoise: false, noise: None }),
        geodesic: None,
    }
}

/// Root-mean-square luminance difference relative to `reference`'s mean.
fn error(image: &[[f32; 4]], reference: &[[f32; 4]]) -> f64 {
    let sq = image.iter().zip(reference).map(|(a, b)| (lum(*a) - lum(*b)).powi(2)).sum::<f64>();
    let level = reference.iter().map(|p| lum(*p)).sum::<f64>();
    (sq / image.len() as f64).sqrt() / (level / image.len() as f64)
}

#[test]
fn the_path_tracer_traverses_with_ray_queries_where_the_device_has_them() {
    let Some((mut eng, has)) = engine() else { return };
    let s = scene(&eng, 4);
    eng.render_now(&s, None);
    assert_eq!(eng.hardware_rays(), has, "hardware traversal follows the device's ray-query feature");
    eng.set_hardware_rays(false);
    eng.render_now(&s, None);
    assert!(!eng.hardware_rays(), "switched off, the tracer walks its own BVH");
    eng.set_hardware_rays(true);
    eng.render_now(&s, None);
    assert_eq!(eng.hardware_rays(), has);
}

#[test]
fn hardware_and_software_traversal_trace_the_same_image() {
    let Some((mut eng, has)) = engine() else { return };
    if !has {
        println!("SKIPPED: the device has no ray queries, so there is no hardware traversal to compare");
        return;
    }
    let s = scene(&eng, 64);
    let hardware = eng.render_now(&s, None);
    assert!(eng.hardware_rays());
    eng.set_hardware_rays(false);
    let software = eng.render_now(&s, None);
    let reference = eng.render_now(&scene(&eng, 1024), None);
    // the noise of a 64-sample image, which the difference between the two traversals must stay well below
    let noise = error(&software, &reference);
    let difference = error(&hardware, &software);
    let coverage = hardware.iter().zip(&software).map(|(a, b)| f64::from((a[3] - b[3]).abs())).sum::<f64>();
    let covered = software.iter().filter(|p| p[3] > 0.5).count();
    let means = [&hardware, &software].map(|px| px.iter().map(|p| lum(*p)).sum::<f64>() / px.len() as f64);
    println!(
        "difference {difference:.5} against the noise {noise:.5}; coverage difference {coverage:.2} over {covered} \
         covered pixels; mean luminance {:.5} and {:.5}",
        means[0], means[1]
    );
    assert!(covered > SIZE[0] as usize * SIZE[1] as usize / 2, "the floor fills most of the frame: {covered}");
    assert!(difference < 0.25 * noise, "the traversals disagree: {difference:.5} against the noise {noise:.5}");
    assert!(coverage < 0.01 * covered as f64, "coverage differs by {coverage:.2} over {covered} pixels");
    assert!((means[0] - means[1]).abs() < 0.01 * means[1], "mean luminance {} against {}", means[0], means[1]);
}

/// What a scene keeps of its primitives.
type Keep = fn(&mut Scene3);

/// Each kind of primitive alone: the hardware traversal finds the instances and the splats through its
/// box geometry, and a scene with no geometry at all traces nothing.
#[test]
fn hardware_traversal_finds_every_kind_of_primitive() {
    let Some((mut eng, has)) = engine() else { return };
    let kinds: [(&str, Keep); 4] = [
        ("triangles", |s| {
            s.draws.truncate(2);
            s.splats.clear();
        }),
        ("instances", |s| {
            s.draws.drain(..2);
            s.splats.clear();
        }),
        ("splats", |s| s.draws.clear()),
        ("nothing", |s| {
            s.draws.clear();
            s.splats.clear();
        }),
    ];
    for (name, keep) in kinds {
        let mut s = scene(&eng, 16);
        keep(&mut s);
        eng.set_hardware_rays(true);
        let hardware = eng.render_now(&s, None);
        eng.set_hardware_rays(false);
        let software = eng.render_now(&s, None);
        let cover = |px: &[[f32; 4]]| px.iter().map(|p| f64::from(p[3])).sum::<f64>();
        println!("{name}: coverage {:.2} (hardware: {has}) and {:.2}", cover(&hardware), cover(&software));
        if name == "nothing" {
            assert_eq!(cover(&hardware), 0.0, "an empty scene covers nothing");
        } else {
            assert!(cover(&software) > 20.0, "{name} are in the frame: {:.2}", cover(&software));
        }
        assert!(
            (cover(&hardware) - cover(&software)).abs() <= 0.02 * cover(&software).max(1.0),
            "{name}: coverage {:.2} against {:.2} with the software traversal",
            cover(&hardware),
            cover(&software)
        );
    }
}
