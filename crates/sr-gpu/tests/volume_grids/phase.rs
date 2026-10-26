//! The phase functions of the media draft on the GPU: the shader's f32 port against sr-volume's and the single
//! scattering of a slab against the phase's closed form (the attributes of a document: light_grid.rs).

use super::common;

use std::sync::Arc;

use glam::{Mat4, Vec3};
use sr_3d::camera::{resolve, CameraParams};
use sr_gpu::three::{Scene3, ThreeEngine};
use sr_gpu::volume::{VolumeDraw, PHASE_WGSL};
use sr_volume::medium::{cdf_draine, fog_parameters, Bounds, March, Medium, Optical, Phase};
use sr_volume::{SparseGrid, Transform};

const PHASES: [Phase; 6] = [
    Phase::Hg { g: 0.6 },
    Phase::Hg { g: -0.3 },
    Phase::Rayleigh,
    Phase::Draine { g: 0.5, alpha: 2.0 },
    Phase::Draine { g: -0.4, alpha: 0.7 },
    Phase::Fog { diameter: 10.0 },
];

/// Runs `body`, a WGSL expression of `x: vec4<f32>` and the record row `p: vec4<f32>`, on every input; f32 results.
fn run(inputs: &[[f32; 4]], rows: &[[f32; 4]], body: &str) -> Option<Vec<f32>> {
    let g = common::gpu()?;
    let source = format!(
        "const PI: f32 = 3.14159265358979;
{PHASE_WGSL}
@group(0) @binding(0) var<storage, read> xs: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read> ps: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> out: array<f32>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id: vec3<u32>) {{
    if (id.x >= arrayLength(&xs)) {{ return; }}
    let x = xs[id.x]; let p = ps[id.x];
    out[id.x] = {body};
}}"
    );
    use wgpu::util::DeviceExt;
    let module = g.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("phase test"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = g.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let storage = |data: &[[f32; 4]]| {
        g.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(data),
            usage: wgpu::BufferUsages::STORAGE,
        })
    };
    let (xs, ps) = (storage(inputs), storage(rows));
    let size = (inputs.len() * 4) as u64;
    let out = g.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let read = g.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let group = g.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: xs.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: ps.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: out.as_entire_binding() },
        ],
    });
    let mut enc = g.device.create_command_encoder(&Default::default());
    {
        let mut pass = enc.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups((inputs.len() as u32).div_ceil(64), 1, 1);
    }
    enc.copy_buffer_to_buffer(&out, 0, &read, 0, size);
    sr_gpu::gpu::submit(&g.queue, [enc.finish()]);
    read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    g.wait();
    let data = read.slice(..).get_mapped_range().expect("mapped");
    Some(bytemuck::cast_slice::<u8, f32>(&data).to_vec())
}

/// The cumulative of `phase` in μ, from −1: the lobes' cumulatives mixed by their weights.
fn cdf(phase: Phase, mu: f64) -> f64 {
    let hg = |g: f64| cdf_draine(mu, g, 0.0);
    match phase {
        Phase::Hg { g } => hg(g),
        Phase::Rayleigh => cdf_draine(mu, 0.0, 1.0),
        Phase::Draine { g, alpha } => cdf_draine(mu, g, alpha),
        Phase::Fog { diameter } => {
            let p = fog_parameters(diameter);
            (1.0 - p.w_d) * hg(p.g_hg) + p.w_d * cdf_draine(mu, p.g_d, p.alpha)
        }
    }
}

#[test]
fn the_shader_evaluates_every_phase_as_sr_volume_does() {
    let mut inputs = Vec::new();
    let mut rows = Vec::new();
    let mut want = Vec::new();
    for phase in PHASES {
        for i in 0..=40 {
            let mu = -1.0 + f64::from(i) / 20.0;
            inputs.push([mu as f32, 0.0, 0.0, 0.0]);
            rows.push(sr_gpu::volume::phase_row(phase));
            want.push(phase.eval(mu));
        }
    }
    let Some(got) = run(&inputs, &rows, "volume_phase(x.x, p)") else { return };
    for (k, (got, want)) in got.iter().zip(&want).enumerate() {
        assert!(
            (f64::from(*got) - want).abs() <= 2e-4 * want.abs().max(1e-3),
            "{:?} at μ {}: {got} != {want}",
            PHASES[k / 41],
            inputs[k][0]
        );
    }
}

/// The shader's sampler inverts each phase's cumulative: the μ it returns for draws (lobe, u) has, under the mixture,
/// the cumulative the draws ask for, and over evenly spread draws its histogram is the phase.
#[test]
fn the_shader_samples_every_phase_by_its_cumulative() {
    let (lobes, draws) = (16, 64);
    let mut inputs = Vec::new();
    let mut rows = Vec::new();
    for phase in PHASES {
        for j in 0..lobes {
            for i in 0..draws {
                inputs.push([(j as f32 + 0.5) / lobes as f32, (i as f32 + 0.5) / draws as f32, 0.0, 0.0]);
                rows.push(sr_gpu::volume::phase_row(phase));
            }
        }
    }
    let Some(got) = run(&inputs, &rows, "phase_sample(x.x, x.y, p)") else { return };
    for (n, phase) in PHASES.iter().enumerate() {
        let block = &got[n * lobes * draws..(n + 1) * lobes * draws];
        assert!(block.iter().all(|mu| (-1.0..=1.0).contains(mu)), "{phase:?}");
        // each lobe's draw inverts its own cumulative, so the share of samples below any μ is the mixture's cumulative
        for cut in [-0.9, -0.5, 0.0, 0.5, 0.9, 0.99] {
            let below = block.iter().filter(|mu| f64::from(**mu) <= cut).count() as f64 / block.len() as f64;
            let want = cdf(*phase, cut);
            assert!((below - want).abs() < 2.5 / draws as f64, "{phase:?} below {cut}: {below} != {want}");
        }
    }
}

/// A slab lit by the sun and seen by an orthographic camera scatters once at the same angle everywhere, so the single
/// scattering of a phase is that of isotropic scattering times 4π p(μ).
#[test]
fn single_scattering_is_weighted_by_the_media_phase() {
    let Some(g) = common::gpu() else { return };
    let mut eng = ThreeEngine::new(g.device.clone(), g.queue.clone());
    let view = Vec3::new((1.0_f32 - 0.7 * 0.7).sqrt(), 0.0, -0.7);
    let sun = Vec3::new(-0.6, 0.0, -0.8);
    let center = Vec3::new(0.0, 0.0, 1.0);
    let mut scene = Scene3 {
        cam: resolve(
            &CameraParams {
                orthographic: true,
                position: Some(center - view * 10.0),
                target: Some(center),
                ..Default::default()
            },
            8.0,
            8.0,
        ),
        clip_fix: Mat4::IDENTITY,
        size: [8, 8],
        exposure: 1.0,
        dof: None,
        lens_k1: 0.0,
        draws: Vec::new(),
        lights: vec![sr_gpu::three::Light3 {
            kind: sr_gpu::three::LightKind::Directional,
            pos: Vec3::ZERO,
            dir: sun,
            right: Vec3::X,
            color: Vec3::ONE,
            range: 0.0,
            falloff: 2.0,
            cos_outer: 0.0,
            cos_inner: 0.0,
            cast_shadow: true,
            softness: 0.0,
            bias: 0.0005,
            map_size: 128,
            size: [0.0; 3],
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
        path: Some(sr_gpu::pathtrace::PathOpts { samples: 4, bounces: 1, denoise: false, noise: None }),
        geodesic: None,
        planet_shell: None,
    };
    let mut radiance = |phase: Phase| {
        let medium = Medium::new(
            Arc::new(SparseGrid::new(Transform::identity(), 1.0, 0).unwrap()),
            Some(Bounds::new([-100.0, -100.0, 0.0], [100.0, 100.0, 2.0]).unwrap()),
            Transform::identity(),
            Optical { phase, extinction: 0.5, ..Default::default() },
        )
        .unwrap();
        scene.volumes = vec![VolumeDraw::new(Arc::new(medium), March { step_size: 0.0625, max_steps: 65536 }).unwrap()];
        let pixels = eng.render_now(&scene, None);
        pixels.iter().map(|p| f64::from(p[0])).sum::<f64>() / pixels.len() as f64
    };
    let isotropic = radiance(Phase::Hg { g: 0.0 });
    assert!(isotropic > 0.01, "the slab scatters the sun: {isotropic}");
    // propagation directions: the sun's light travels along `sun`, leaves toward the camera along −view
    let mu = f64::from(sun.dot(-view));
    for phase in PHASES {
        let want = 4.0 * std::f64::consts::PI * phase.eval(mu);
        let got = radiance(phase) / isotropic;
        assert!((got - want).abs() < 1e-3 * want.max(1.0), "{phase:?} at μ {mu}: {got} != {want}");
    }
}
