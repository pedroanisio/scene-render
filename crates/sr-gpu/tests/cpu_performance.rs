//! Performance contracts paired with real rendered output, rather than timing thresholds.
mod common;
use glam::{Mat4, Vec3};
use sr_3d::{
    camera::{resolve, CameraParams},
    prim, MaterialParams,
};
use sr_gpu::three::{Draw3, MeshSrc, Scene3, ThreeEngine};

fn scene(draws: Vec<Draw3>) -> Scene3 {
    Scene3 {
        cam: resolve(&CameraParams::default(), 64., 64.),
        clip_fix: Mat4::IDENTITY,
        size: [64, 64],
        exposure: 1.,
        dof: None,
        lens_k1: 0.,
        draws,
        lights: Vec::new(),
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
    }
}
fn draw(engine: &ThreeEngine, at: Vec3) -> Draw3 {
    let mesh = prim::plane(24., 24., 1);
    Draw3 {
        mesh: MeshSrc::Cached(engine.upload_mesh(&mesh.vertices, &mesh.indices)),
        model: Mat4::from_translation(at),
        material: MaterialParams { unlit: true, double_sided: true, ..Default::default() },
        maps: Default::default(),
        opacity: 1.,
        cast_shadow: true,
        receive_shadow: true,
        shadow_catcher: false,
    }
}
#[test]
fn offscreen_transmission_does_not_schedule_camera_draws_or_mips() {
    let Some(gpu) = common::gpu() else { return };
    let mut engine = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    let visible = draw(&engine, Vec3::new(32., 32., 0.));
    let mut hidden = draw(&engine, Vec3::new(10000., 32., 0.));
    hidden.material.unlit = false;
    hidden.material.transmission = 1.;
    let mut s = scene(vec![visible, hidden]);
    let picture = engine.render_now(&s, None);
    assert!(picture.iter().any(|p| p[3] > 0.));
    assert_eq!(engine.stats.draws, 1, "offscreen meshes must not enter camera batches");
    assert_eq!(engine.stats.transmissive, 0, "offscreen glass must not trigger the scene-color mip chain");
    let MeshSrc::Cached(m) = &s.draws[1].mesh else { unreachable!() };
    let m = m.clone();
    s.draws[1].mesh = MeshSrc::Deformed(m.cpu.0.clone(), m);
    assert_eq!(picture, engine.render_now(&s, None), "conservative uncullable reference");
}

#[test]
fn camera_and_material_changes_reuse_pathtrace_geometry() {
    let Some(gpu) = common::gpu() else { return };
    let mut engine = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    let mut s = scene(vec![draw(&engine, Vec3::new(32., 32., 0.))]);
    s.path = Some(sr_gpu::pathtrace::PathOpts { samples: 1, bounces: 1, denoise: false, noise: None });
    let original = engine.render_now(&s, None);
    assert!(engine.stats.pt_bvh_seconds > 0.);
    s.cam = resolve(&CameraParams { offset: Vec3::new(4., 0., 0.), ..Default::default() }, 64., 64.);
    s.draws[0].material.base_color = [0.2, 0.8, 0.4, 1.];
    let warm = engine.render_now(&s, None);
    assert_eq!(engine.stats.pt_bvh_seconds, 0., "camera/material changes must not rebuild geometry BVHs");
    assert_ne!(warm, original);
    let mut fresh = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    assert_eq!(warm, fresh.render_now(&s, None), "cache must preserve the new camera/material");
}

fn same_payload(a: &sr_gpu::pathtrace::PtScene, b: &sr_gpu::pathtrace::PtScene) {
    macro_rules! bytes {
        ($field:ident) => {
            assert_eq!(
                bytemuck::cast_slice::<_, u8>(&a.$field),
                bytemuck::cast_slice::<_, u8>(&b.$field),
                stringify!($field)
            );
        };
    }
    bytes!(pos);
    bytes!(nrm);
    bytes!(uv);
    bytes!(colors);
    bytes!(tri_mat);
    bytes!(nodes);
    assert_eq!(a.atlas_key(), b.atlas_key(), "atlas");
    assert_eq!(a.atlas_pixels(), b.atlas_pixels(), "atlas pixels");
    bytes!(mats);
    bytes!(lights);
    for (left, right) in [(&a.instances, &b.instances), (&a.splats, &b.splats)] {
        assert_eq!(left.len(), right.len());
        for (k, v) in left {
            assert_eq!(bytemuck::cast_slice::<_, u8>(v), bytemuck::cast_slice::<_, u8>(&right[k]));
        }
    }
    assert_eq!(a.notes, b.notes);
}

#[test]
fn cached_geometry_matches_fresh_builds_and_invalidates_all_geometry_inputs() {
    use sr_gpu::pathtrace::{build, BuildCache};
    let Some(gpu) = common::gpu() else { return };
    let mut engine = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    let a = draw(&engine, Vec3::new(24., 32., 0.));
    let MeshSrc::Cached(mesh) = &a.mesh else { unreachable!() };
    let mesh = mesh.clone();
    let mut b = draw(&engine, Vec3::new(40., 32., 0.));
    b.mesh = MeshSrc::Cached(mesh.clone());
    let mut s = scene(vec![a, b]);
    let mut cache = BuildCache::default();
    let first = cache.build(&s);
    same_payload(&first, &build(&s));
    assert_eq!(first.timing.prototype_builds, 1);
    s.draws[0].material.base_color = [0.2, 0.7, 0.8, 1.];
    let warm = cache.build(&s);
    same_payload(&warm, &build(&s));
    assert!(warm.timing.geometry_reused);
    // Texture pixels, light data and IES offsets are rebuilt independently of geometry.
    s.draws[0].maps[0] = Some(engine.upload_rgba8(1, 1, &[64, 128, 192, 255], true));
    s.lights.push(sr_gpu::three::Light3 {
        kind: sr_gpu::three::LightKind::Directional,
        pos: Vec3::ZERO,
        dir: Vec3::Z,
        right: Vec3::X,
        color: Vec3::splat(2.),
        range: 0.,
        falloff: 0.,
        cos_outer: 0.,
        cos_inner: 0.,
        cast_shadow: true,
        softness: 0.,
        bias: 0.001,
        map_size: 64,
        size: [0.; 3],
        ies: Some(std::sync::Arc::new(vec![0.5; 128])),
        affects_diffuse: true,
        affects_specular: true,
        contact: 0.,
    });
    s.draws[0].material.foam_mix = Some(sr_3d::FoamMix { albedo: 0.7, roughness: 0.4 });
    let relit = cache.build(&s);
    same_payload(&relit, &build(&s));
    assert!(relit.timing.geometry_reused);
    s.draws[0].model = Mat4::from_translation(Vec3::new(25., 33., -1.));
    let moved = cache.build(&s);
    same_payload(&moved, &build(&s));
    assert!(!moved.timing.geometry_reused);
    assert_eq!(moved.timing.prototype_builds, 0);
    assert_eq!(moved.timing.prototype_hits, 1);
    for d in &mut s.draws {
        d.material.uv_scale = [2., 3.];
        d.material.separate_uvs = true;
    }
    let uv = cache.build(&s);
    same_payload(&uv, &build(&s));
    assert!(!uv.timing.geometry_reused);
    assert_eq!(uv.timing.prototype_builds, 1);
    s.draws[0].shadow_catcher = true;
    let hidden = cache.build(&s);
    same_payload(&hidden, &build(&s));
    assert!(!hidden.timing.geometry_reused);
    s.draws[0].shadow_catcher = false;
    let mut vertices = mesh.cpu.0.clone();
    vertices[0].pos[0] += 3.;
    s.draws[0].mesh = MeshSrc::Deformed(vertices, mesh.clone());
    for _ in 0..2 {
        let deformed = cache.build(&s);
        same_payload(&deformed, &build(&s));
        assert!(!deformed.timing.geometry_reused);
    }
    s.draws[0].mesh = MeshSrc::Cached(mesh);
    s.draws[0].maps[5] = Some(engine.upload_rgba8(1, 1, &[255; 4], false));
    for scale in [1., 2.] {
        s.draws[0].material.displacement_scale = scale;
        let displaced = cache.build(&s);
        same_payload(&displaced, &build(&s));
        assert!(!displaced.timing.geometry_reused);
    }
    s.draws[0].maps[5] = None;
    let replacement = draw(&engine, Vec3::new(12., 12., 0.));
    s.draws[0] = replacement;
    let replaced = cache.build(&s);
    same_payload(&replaced, &build(&s));
    assert!(!replaced.timing.geometry_reused);
    // Shadow catchers are omitted from tracing, but the existing instance
    // classifier counts their mesh identities. That can change packing topology.
    let mut catcher = draw(&engine, Vec3::new(9., 9., 0.));
    let MeshSrc::Cached(replacement) = &s.draws[0].mesh else { unreachable!() };
    catcher.mesh = MeshSrc::Cached(replacement.clone());
    catcher.shadow_catcher = true;
    s.draws.push(catcher);
    same_payload(&cache.build(&s), &build(&s));
    let mut disabled = BuildCache::new(1);
    for _ in 0..2 {
        same_payload(&disabled.build(&s), &build(&s));
        assert_eq!(disabled.resident_bytes(), 0);
    }
    assert!(cache.resident_bytes() > 0 && cache.resident_bytes() <= 256 << 20);
    // Exercise eviction rather than only the "too small to store anything" case.
    let mut bounded = BuildCache::new(4096);
    for i in 1..70 {
        for d in &mut s.draws {
            d.material.uv_scale = [i as f32, 1.];
        }
        same_payload(&bounded.build(&s), &build(&s));
        assert!(bounded.resident_bytes() <= 4096);
    }
    cache.clear();
    assert_eq!(cache.resident_bytes(), 0);
}

#[test]
fn software_splat_sort_preserves_rendered_pixels_and_depth_ties() {
    let Some(gpu) = common::gpu() else { return };
    let mut engine = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    let mut points = sr_3d::Splats { basis: Mat4::IDENTITY, ..Default::default() };
    for i in 0..1025 {
        points.pos.push([12. + (i % 11) as f32 * 4., 12. + (i % 9) as f32 * 4., ((i * 73) % 113) as f32]);
        points.scale.push([2.; 3]);
        points.rot.push([0., 0., 0., 1.]);
        points.color.push([0.2 + (i % 3) as f32 * 0.3, 0.6, 0.3, 0.2]);
    }
    let mut s = scene(Vec::new());
    s.splats.push(sr_gpu::three::SplatDraw { gpu: engine.upload_splats(&points), model: Mat4::IDENTITY, opacity: 1. });
    let reference = engine.render_now(&s, None);
    let mut software = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    software.software_adapter = true;
    assert_eq!(software.render_now(&s, None), reference);
    assert_eq!(software.stats.splat_sorts, 1);
    assert!(reference.iter().any(|p| p[3] > 0.));
}

#[test]
fn primary_culling_preserves_offscreen_shadows_and_displaced_geometry() {
    use sr_gpu::three::{Light3, LightKind};
    let Some(gpu) = common::gpu() else { return };
    let mut engine = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    let mut caster = draw(&engine, Vec3::new(-16., 32., 0.));
    caster.material.unlit = false;
    let mut receiver = draw(&engine, Vec3::new(32., 32., 60.));
    let plane = prim::plane(200., 200., 1);
    receiver.mesh = MeshSrc::Cached(engine.upload_mesh(&plane.vertices, &plane.indices));
    receiver.material.unlit = false;
    let mut s = scene(vec![caster, receiver]);
    s.lights.push(Light3 {
        kind: LightKind::Directional,
        pos: Vec3::ZERO,
        dir: Vec3::new(0.8, 0., 1.).normalize(),
        right: Vec3::X,
        color: Vec3::splat(3.),
        range: 0.,
        falloff: 0.,
        cos_outer: 0.,
        cos_inner: 0.,
        cast_shadow: true,
        softness: 0.,
        bias: 0.0005,
        map_size: 256,
        size: [0.; 3],
        ies: None,
        affects_diffuse: true,
        affects_specular: true,
        contact: 0.,
    });
    let culled = engine.render_now(&s, None);
    assert_eq!(engine.stats.draws, 1);
    let MeshSrc::Cached(mesh) = &s.draws[0].mesh else { unreachable!() };
    let mesh = mesh.clone();
    s.draws[0].mesh = MeshSrc::Deformed(mesh.cpu.0.clone(), mesh.clone());
    assert_eq!(culled, engine.render_now(&s, None));
    s.draws[0].cast_shadow = false;
    assert_ne!(culled, engine.render_now(&s, None), "the offscreen caster must cast a visible shadow");
    // A displacement map bypasses rigid bounds culling even when its current
    // sample happens to be zero; subsequent map/scale changes can move vertices.
    s.draws[0].mesh = MeshSrc::Cached(mesh);
    s.draws[0].maps[5] = Some(engine.upload_rgba8(1, 1, &[0, 0, 0, 255], false));
    engine.render_now(&s, None);
    assert_eq!(engine.stats.draws, 2);
}

#[test]
#[ignore = "CPU geometry preparation benchmark; run with --ignored --nocapture"]
fn benchmark_cached_pathtrace_geometry() {
    use sr_gpu::pathtrace::{build, BuildCache};
    let Some(gpu) = common::gpu() else { return };
    let engine = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    let sphere = prim::sphere(4., 114);
    let mesh = engine.upload_mesh(&sphere.vertices, &sphere.indices);
    let mut s = scene(
        (0..16)
            .map(|i| Draw3 {
                mesh: MeshSrc::Cached(mesh.clone()),
                model: Mat4::from_translation(Vec3::new(i as f32 * 8., 0., 0.)),
                material: Default::default(),
                maps: Default::default(),
                opacity: 1.,
                cast_shadow: true,
                receive_shadow: true,
                shadow_catcher: false,
            })
            .collect(),
    );
    let mut cache = BuildCache::default();
    cache.build(&s);
    let mut times = [Vec::new(), Vec::new(), Vec::new()];
    for round in 0..6 {
        for slot in 0..3 {
            let i = (slot + round) % 3;
            if i == 2 {
                s.draws[0].model.w_axis.x += 0.1;
            }
            let clock = std::time::Instant::now();
            let data = if i == 0 { build(&s) } else { cache.build(&s) };
            times[i].push(clock.elapsed().as_secs_f64() * 1000.);
            same_payload(&data, &build(&s));
        }
    }
    for (name, mut times) in ["fresh", "warm geometry", "moving instance"].into_iter().zip(times) {
        times.sort_by(f64::total_cmp);
        println!("{name}: median {:.3} ms; {times:?}", (times[2] + times[3]) * 0.5);
    }
}

#[test]
fn alternating_rigid_passes_reuse_geometry_within_the_budget() {
    let Some(gpu) = common::gpu() else { return };
    let engine = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    let a = scene(vec![draw(&engine, Vec3::new(32., 32., 0.))]);
    let b = scene(vec![draw(&engine, Vec3::new(30., 32., 0.))]);
    let mut cache = sr_gpu::pathtrace::BuildCache::new(1 << 20);
    cache.build(&a);
    cache.build(&b);
    for s in [&a, &b, &a, &b] {
        let data = cache.build(s);
        same_payload(&data, &sr_gpu::pathtrace::build(s));
        assert!(data.timing.geometry_reused, "a different pass must not evict geometry that fits the budget");
    }
}

#[test]
fn culled_noncasters_do_not_prepare_objects_or_materials() {
    let Some(gpu) = common::gpu() else { return };
    let mut engine = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    let mut s = scene(vec![draw(&engine, Vec3::new(32., 32., 0.))]);
    let reference = engine.render_now(&s, None);
    for i in 0..1000 {
        let mut d = draw(&engine, Vec3::new(10000. + i as f32, 32., 0.));
        d.cast_shadow = false;
        s.draws.push(d);
    }
    assert_eq!(engine.render_now(&s, None), reference);
    assert_eq!(engine.stats.prepared_objects, 1);
}

#[test]
fn warm_pathtrace_passes_do_not_upload_unchanged_geometry() {
    let Some(gpu) = common::gpu() else { return };
    let mut engine = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    let mut s = scene(vec![draw(&engine, Vec3::new(32., 32., 0.))]);
    s.path = Some(sr_gpu::pathtrace::PathOpts { samples: 1, bounces: 1, denoise: false, noise: None });
    engine.render_now(&s, None);
    assert_eq!(engine.stats.pt_geometry_uploads, 1);
    s.draws[0].material.base_color = [0.2, 0.8, 0.4, 1.];
    let warm = engine.render_now(&s, None);
    assert_eq!(engine.stats.pt_geometry_uploads, 0);
    let mut fresh = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    assert_eq!(warm, fresh.render_now(&s, None));
    s.draws[0].model.w_axis.x += 3.;
    let moved = engine.render_now(&s, None);
    assert_eq!(engine.stats.pt_geometry_uploads, 1);
    assert_eq!(moved, fresh.render_now(&s, None));
}

#[test]
fn opacity_changes_reuse_splat_order_but_view_changes_invalidate_it() {
    let Some(gpu) = common::gpu() else { return };
    let mut engine = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    engine.software_adapter = true;
    let points = sr_3d::Splats {
        basis: Mat4::IDENTITY,
        pos: vec![[28., 32., 0.], [32., 32., 1.], [32., 32., 1.]],
        scale: vec![[4.; 3]; 3],
        rot: vec![[0., 0., 0., 1.]; 3],
        color: vec![[1., 0., 0., 0.5], [0., 1., 0., 0.5], [0., 0., 1., 0.5]],
        ..Default::default()
    };
    let mut s = scene(Vec::new());
    s.splats.push(sr_gpu::three::SplatDraw { gpu: engine.upload_splats(&points), model: Mat4::IDENTITY, opacity: 1. });
    engine.render_now(&s, None);
    assert_eq!(engine.stats.splat_sorts, 1);
    s.splats[0].opacity = 0.7;
    let warm = engine.render_now(&s, None);
    assert_eq!(engine.stats.splat_sorts, 0);
    assert_eq!(engine.stats.buffers_created, 0, "a pass without a sort makes no buffers");
    let mut fresh = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    assert_eq!(warm, fresh.render_now(&s, None));
    s.splats[0].model = Mat4::from_rotation_y(0.5);
    let moved = engine.render_now(&s, None);
    assert_eq!(engine.stats.splat_sorts, 1);
    assert_eq!(engine.stats.buffers_created, 1, "a re-sort makes only the order buffer the cache keeps");
    assert_eq!(moved, fresh.render_now(&s, None));
}

#[test]
fn warm_raster_passes_reuse_their_buffers_and_material_bind_groups() {
    let Some(gpu) = common::gpu() else { return };
    let mut engine = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    let mut textured = draw(&engine, Vec3::new(40., 40., 0.));
    textured.maps[0] = Some(engine.upload_rgba8(1, 1, &[64, 128, 192, 255], true));
    let mut s = scene(vec![draw(&engine, Vec3::new(20., 20., 0.)), textured]);
    engine.render_now(&s, None);
    assert!(engine.stats.buffers_created > 0, "the first pass makes its buffers");
    assert_eq!(engine.stats.material_binds_created, 2, "one bind group per distinct set of maps");
    s.draws[0].model.w_axis.x += 2.;
    let warm = engine.render_now(&s, None);
    assert_eq!(engine.stats.buffers_created, 0, "a pass that fits the kept buffers makes none");
    assert_eq!(engine.stats.material_binds_created, 0, "the same maps reuse their bind groups");
    let mut fresh = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    assert_eq!(warm, fresh.render_now(&s, None));
    s.draws[1].maps[0] = Some(engine.upload_rgba8(1, 1, &[200, 20, 20, 255], true));
    engine.render_now(&s, None);
    assert_eq!(engine.stats.material_binds_created, 1, "other maps are another bind group");
    assert_eq!(engine.stats.buffers_created, 0);
    for k in 0..48 {
        s.draws.push(draw(&engine, Vec3::new(k as f32, 8., 0.)));
    }
    engine.render_now(&s, None);
    assert!(engine.stats.buffers_created > 0, "more objects than the kept buffers hold grow them");
    assert_eq!(engine.stats.material_binds_created, 0, "the grown material buffer keeps its bind groups valid");
}

#[test]
fn deformed_meshes_write_the_vertex_buffers_the_pass_keeps() {
    let Some(gpu) = common::gpu() else { return };
    let mut engine = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    let mut d = draw(&engine, Vec3::new(32., 32., 0.));
    let MeshSrc::Cached(m) = &d.mesh else { unreachable!() };
    let (m, mut vs) = (m.clone(), m.cpu.0.clone());
    d.mesh = MeshSrc::Deformed(vs.clone(), m.clone());
    let mut s = scene(vec![d]);
    engine.render_now(&s, None);
    assert!(engine.stats.buffers_created > 0);
    for v in &mut vs {
        v.pos[0] += 1.;
    }
    s.draws[0].mesh = MeshSrc::Deformed(vs, m);
    let warm = engine.render_now(&s, None);
    assert_eq!(engine.stats.buffers_created, 0, "the deformed vertices go into the buffer the pass kept");
    let mut fresh = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    assert_eq!(warm, fresh.render_now(&s, None));
}

#[test]
fn raster_passes_recorded_before_one_submit_keep_their_own_data() {
    let Some(gpu) = common::gpu() else { return };
    let mut engine = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    let mut a = scene(vec![draw(&engine, Vec3::new(20., 20., 0.))]);
    a.draws[0].material.base_color = [0.9, 0.1, 0.1, 1.];
    let mut b = scene(vec![draw(&engine, Vec3::new(44., 44., 0.)), draw(&engine, Vec3::new(10., 50., 0.))]);
    b.draws[1].material.base_color = [0.1, 0.1, 0.9, 1.];
    // warm the kept buffers, then record two passes and submit them in reverse
    engine.render_now(&a, None);
    let (ta, tb) = (engine.target(a.size), engine.target(b.size));
    let mut ea = gpu.device.create_command_encoder(&Default::default());
    engine.render(&mut ea, &a, None, &ta.create_view(&Default::default())).expect("scene a");
    let mut eb = gpu.device.create_command_encoder(&Default::default());
    engine.render(&mut eb, &b, None, &tb.create_view(&Default::default())).expect("scene b");
    sr_gpu::gpu::submit(&gpu.queue, [eb.finish(), ea.finish()]);
    let mut fresh = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    assert_eq!(engine.read(&ta), fresh.render_now(&a, None), "pass a draws its own objects");
    assert_eq!(engine.read(&tb), fresh.render_now(&b, None), "pass b draws its own objects");
    engine.submitted();
    engine.render_now(&a, None);
    assert_eq!(engine.stats.buffers_created, 0, "after the submit the first set serves again");
}

#[test]
fn warm_cpu_traced_passes_reuse_the_scene_the_tracer_built() {
    let Some(gpu) = common::gpu() else { return };
    let mut engine = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    engine.software_adapter = true;
    let mut s = scene(vec![draw(&engine, Vec3::new(32., 32., 0.))]);
    s.path = Some(sr_gpu::pathtrace::PathOpts { samples: 1, bounces: 1, denoise: false, noise: None });
    // the CPU tracer takes lit scenes under a sun
    s.lights.push(sr_gpu::three::Light3 {
        kind: sr_gpu::three::LightKind::Directional,
        pos: Vec3::ZERO,
        dir: Vec3::Z,
        right: Vec3::X,
        color: Vec3::splat(2.),
        range: 0.,
        falloff: 0.,
        cos_outer: 0.,
        cos_inner: 0.,
        cast_shadow: true,
        softness: 0.,
        bias: 0.001,
        map_size: 64,
        size: [0.; 3],
        ies: None,
        affects_diffuse: true,
        affects_specular: true,
        contact: 0.,
    });
    engine.render_now(&s, None);
    assert_eq!(engine.stats.pt_cpu_scenes, 1, "the first pass builds the tracer's scene");
    s.draws[0].material.base_color = [0.2, 0.8, 0.4, 1.];
    let warm = engine.render_now(&s, None);
    assert_eq!(engine.stats.pt_cpu_scenes, 0, "a pass over the same geometry and textures reuses it");
    let mut fresh = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    fresh.software_adapter = true;
    assert_eq!(warm, fresh.render_now(&s, None));
    s.draws[0].maps[0] = Some(engine.upload_rgba8(1, 1, &[64, 128, 192, 255], true));
    engine.render_now(&s, None);
    assert_eq!(engine.stats.pt_cpu_scenes, 1, "other textures are another scene");
    s.draws[0].model.w_axis.x += 3.;
    engine.render_now(&s, None);
    assert_eq!(engine.stats.pt_cpu_scenes, 1, "other geometry is another scene");
}

#[test]
fn packed_geometry_is_immutable_across_reversed_submissions_and_texture_changes() {
    let Some(gpu) = common::gpu() else { return };
    let mut e = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    let mut s = scene(vec![draw(&e, Vec3::new(32., 32., 0.))]);
    s.path = Some(sr_gpu::pathtrace::PathOpts { samples: 1, bounces: 1, denoise: false, noise: None });
    let a = e.target(s.size);
    let b = e.target(s.size);
    let mut first = gpu.device.create_command_encoder(&Default::default());
    let mut second = gpu.device.create_command_encoder(&Default::default());
    e.render(&mut first, &s, None, &a.create_view(&Default::default())).unwrap();
    s.draws[0].material.base_color = [0.2, 0.7, 0.4, 1.];
    e.render(&mut second, &s, None, &b.create_view(&Default::default())).unwrap();
    assert_eq!(e.stats.pt_geometry_uploads, 0);
    sr_gpu::gpu::submit(&gpu.queue, [second.finish(), first.finish()]);
    let mut fresh = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    assert_eq!(e.read(&b), fresh.render_now(&s, None));
    s.draws[0].material.base_color = [1.; 4];
    assert_eq!(e.read(&a), fresh.render_now(&s, None));
    let white = e.upload_rgba8(1, 1, &[255, 255, 255, 255], true);
    let blue = e.upload_rgba8(1, 1, &[32, 64, 255, 255], true);
    s.draws[0].maps[0] = Some(white);
    e.render_now(&s, None);
    // Different metadata with identical packed pixels is safe to reuse.
    assert_eq!(e.stats.pt_geometry_uploads, 0);
    s.draws[0].maps[0] = Some(blue);
    let changed = e.render_now(&s, None);
    assert_eq!(e.stats.pt_geometry_uploads, 1);
    assert_eq!(changed, fresh.render_now(&s, None));
    assert_eq!(e.render_now(&s, None), changed);
    assert_eq!(e.stats.pt_geometry_uploads, 0);
}

#[test]
fn warm_pathtrace_passes_pack_texture_pixels_only_when_their_textures_change() {
    let Some(gpu) = common::gpu() else { return };
    let mut e = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    let mut s = scene(vec![draw(&e, Vec3::new(32., 32., 0.))]);
    s.path = Some(sr_gpu::pathtrace::PathOpts { samples: 1, bounces: 1, denoise: false, noise: None });
    let red = e.upload_rgba8(2, 2, &[255, 32, 32, 255].repeat(4), true);
    let blue = e.upload_rgba8(2, 1, &[32, 64, 255, 255].repeat(2), true);
    s.draws[0].maps[0] = Some(red.clone());
    e.render_now(&s, None);
    assert_eq!((e.stats.pt_geometry_uploads, e.stats.pt_atlas_texels), (1, 4));
    // the same textures under another material: nothing packed, nothing uploaded
    s.draws[0].material.base_color = [0.2, 0.8, 0.4, 1.];
    let warm = e.render_now(&s, None);
    assert_eq!((e.stats.pt_geometry_uploads, e.stats.pt_atlas_texels), (0, 0));
    let mut fresh = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    let fresh_red = fresh.upload_rgba8(2, 2, &[255, 32, 32, 255].repeat(4), true);
    let mut f = scene(vec![draw(&fresh, Vec3::new(32., 32., 0.))]);
    f.path = s.path;
    f.draws[0].material = s.draws[0].material.clone();
    f.draws[0].maps[0] = Some(fresh_red);
    assert_eq!(warm, fresh.render_now(&f, None));
    // another texture: packed and uploaded once, then reused
    s.draws[0].maps[0] = Some(blue);
    let changed = e.render_now(&s, None);
    assert_eq!((e.stats.pt_geometry_uploads, e.stats.pt_atlas_texels), (1, 2));
    assert_eq!(e.render_now(&s, None), changed);
    assert_eq!((e.stats.pt_geometry_uploads, e.stats.pt_atlas_texels), (0, 0));
}

#[test]
fn splat_cache_ignores_unsubmitted_encoders_and_replaced_clouds() {
    let Some(gpu) = common::gpu() else { return };
    let mut e = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    e.software_adapter = true;
    let mut points = sr_3d::Splats {
        basis: Mat4::IDENTITY,
        pos: vec![[28., 32., 0.], [32., 32., 1.]],
        scale: vec![[4.; 3]; 2],
        rot: vec![[0., 0., 0., 1.]; 2],
        color: vec![[1., 0., 0., 0.5], [0., 1., 0., 0.5]],
        ..Default::default()
    };
    let mut s = scene(Vec::new());
    s.splats.push(sr_gpu::three::SplatDraw { gpu: e.upload_splats(&points), model: Mat4::IDENTITY, opacity: 1. });
    let a = e.target(s.size);
    let b = e.target(s.size);
    let mut abandoned = gpu.device.create_command_encoder(&Default::default());
    e.render(&mut abandoned, &s, None, &a.create_view(&Default::default())).unwrap();
    s.splats[0].opacity = 0.7;
    let mut submitted = gpu.device.create_command_encoder(&Default::default());
    e.render(&mut submitted, &s, None, &b.create_view(&Default::default())).unwrap();
    assert_eq!(e.stats.splat_sorts, 1, "unsubmitted data is not reusable");
    sr_gpu::gpu::submit(&gpu.queue, [submitted.finish()]);
    let mut fresh = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    assert_eq!(e.read(&b), fresh.render_now(&s, None));
    drop(abandoned);
    s.splats[0].opacity = 0.8;
    assert_eq!(e.render_now(&s, None), fresh.render_now(&s, None));
    assert_eq!(e.stats.splat_sorts, 0, "the later completed encoder can be reused");
    points.pos[0][2] = 5.;
    s.splats[0].gpu = e.upload_splats(&points);
    assert_eq!(e.render_now(&s, None), fresh.render_now(&s, None));
    assert_eq!(e.stats.splat_sorts, 1);
    s.cam = resolve(&CameraParams { offset: Vec3::new(2., 0., 0.), ..Default::default() }, 64., 64.);
    assert_eq!(e.render_now(&s, None), fresh.render_now(&s, None));
    assert_eq!(e.stats.splat_sorts, 1);
}

#[test]
fn software_occlusion_runs_at_half_size_within_45_db_of_the_full_size_pass() {
    use sr_gpu::three::{Light3, LightKind};
    let Some(gpu) = common::gpu() else { return };
    let mut engine = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    // at a video-like scale: an occlusion a few pixels wide is all edge at any size
    let mut floor = draw(&engine, Vec3::new(128., 128., 160.));
    let wall = prim::plane(800., 800., 1);
    floor.mesh = MeshSrc::Cached(engine.upload_mesh(&wall.vertices, &wall.indices));
    floor.material.unlit = false;
    let ball = prim::sphere(48., 96);
    let mut sphere = draw(&engine, Vec3::new(128., 128., 120.));
    sphere.mesh = MeshSrc::Cached(engine.upload_mesh(&ball.vertices, &ball.indices));
    sphere.material.unlit = false;
    let mut s = scene(vec![floor, sphere]);
    s.cam = resolve(&CameraParams::default(), 256., 256.);
    s.size = [256, 256];
    s.lights.push(Light3 {
        kind: LightKind::Ambient,
        pos: Vec3::ZERO,
        dir: Vec3::Z,
        right: Vec3::X,
        color: Vec3::splat(1.),
        range: 0.,
        falloff: 0.,
        cos_outer: 0.,
        cos_inner: 0.,
        cast_shadow: false,
        softness: 0.,
        bias: 0.,
        map_size: 0,
        size: [0.; 3],
        ies: None,
        affects_diffuse: true,
        affects_specular: true,
        contact: 0.,
    });
    let open = engine.render_now(&s, None);
    s.ao = Some([48., 1.]);
    let reference = engine.render_now(&s, None);
    assert_eq!(engine.stats.ao_size, Some([256, 256]));
    let mut software = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    software.software_adapter = true;
    let half = software.render_now(&s, None);
    assert_eq!(software.stats.ao_size, Some([128, 128]));
    let psnr = |a: &[[f32; 4]], b: &[[f32; 4]]| {
        let se: f64 = a.iter().zip(b).flat_map(|(p, q)| (0..3).map(move |c| (p[c] - q[c]) as f64)).map(|d| d * d).sum();
        10. * (1. / (se / (a.len() * 3) as f64)).log10()
    };
    assert_ne!(reference, open, "the occlusion must darken something");
    assert!(psnr(&half, &open) < psnr(&half, &reference), "the half-size pass still occludes");
    // lit only by the ambient light, which the occlusion darkens, the picture is all occlusion: the worst case
    assert!(psnr(&half, &reference) >= 40., "ambient only: {:.1} dB", psnr(&half, &reference));
    // with a key light as well, as a lit scene has, it stays within 45 dB
    let mut key = s.lights[0].clone();
    key.kind = LightKind::Directional;
    key.dir = Vec3::new(0.3, 0.4, 1.).normalize();
    key.color = Vec3::splat(2.);
    s.lights[0].color = Vec3::splat(0.6);
    s.lights.push(key);
    let reference = engine.render_now(&s, None);
    let half = software.render_now(&s, None);
    assert!(psnr(&half, &reference) >= 45., "with a key light: {:.1} dB", psnr(&half, &reference));
    // screen-space reflections and contact shadows read the prepass at the frame's own size
    s.ssr = true;
    software.render_now(&s, None);
    assert_eq!(software.stats.ao_size, Some([256, 256]));
    s.ssr = false;
    s.lights[0].contact = 32.;
    software.render_now(&s, None);
    assert_eq!(software.stats.ao_size, Some([256, 256]));
}
