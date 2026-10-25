//! Progressive path tracing of a 3D pass (camera `renderer="pathtrace"`).
//!
//! Repeated rigid meshes use shared object-space prototype BVHs and world-space instance bounds.
//! Unique, deformed and displaced geometry is flattened into world-space triangles. Both levels
//! use a binned (12 bins) surface-area hierarchy; materials and lights remain per draw.
//! `pathtrace.wgsl` then traces `samples` paths per pixel in compute dispatches of a few
//! samples each: camera rays (thin lens for depth of field), next-event estimation toward the
//! analytic lights with shadow rays, a metallic-roughness BSDF (Lambert plus GGX, sampled by
//! lobe), smooth dielectric transmission, Russian roulette, and the dome environment and ambient
//! light for rays that leave the scene. Rays that leave through glass see the 2D layers behind
//! the pass where they cross the composition plane (z = 0). An edge-aware à-trous filter
//! (Dammertz et al. 2010) guided by first-hit normals and albedo optionally denoises the result.
//!
//! Gaussian splats share the acceleration structure and composite along rays;
//! camera rays use the complete projection, including offscreen and lens transforms.

use glam::{Mat4, Vec3};

use crate::three::{Draw3, LightKind, MeshSrc, Scene3};
mod cache;
mod cpu;
mod instances;
mod packed;
mod rays;
pub use cache::{BuildCache, GEOMETRY_BUDGET};

/// Path-tracing options of a pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathOpts {
    /// Samples per pixel; with `noise`, the most a pixel traces.
    pub samples: u32,
    pub bounces: u32,
    pub denoise: bool,
    /// Adaptive sampling: a pixel stops once the standard error of its mean is at most this fraction of the mean
    /// (luminance, with a floor of 0.01 after exposure) and this much coverage. It is tested from
    /// [`ADAPTIVE_MIN_SAMPLES`] samples on, after each dispatch. None (or a value that is not positive) traces every
    /// sample.
    pub noise: Option<f32>,
}

/// The samples a pixel traces before adaptive sampling tests it: fewer would let a run of similar samples stop a
/// noisy pixel. `pathtrace.wgsl` holds the same number.
pub const ADAPTIVE_MIN_SAMPLES: u32 = 16;

impl PathOpts {
    /// The noise target in effect: a positive, finite `noise`.
    pub fn noise_target(&self) -> Option<f32> {
        self.noise.filter(|n| n.is_finite() && *n > 0.0)
    }
}

/// A material as the tracer reads it (16-byte rows).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PtMat {
    /// Linear base colour, opacity.
    pub base: [f32; 4],
    /// metallic, roughness, transmission, ior
    pub params: [f32; 4],
    /// emitted radiance, unlit flag
    pub emissive: [f32; 4],
    /// Specular weight, cast-shadow flag, receive-shadow flag, unused
    pub extra: [f32; 4],
    /// Offset, width, height and packed sRGB/sampler flags for each shared pixel map.
    pub maps: [[u32; 4]; 6],
    /// Alpha cutoff, alpha mode, normal scale, occlusion strength.
    pub texture_params: [f32; 4],
    pub borders: [[f32; 4]; 6],
    /// Absorption of the medium a refracting surface encloses: Beer-Lambert coefficient per scene
    /// unit for each colour channel (from the attenuation colour and distance), then the albedo of the foam
    /// mixed into the surface (0 when none).
    pub attenuation: [f32; 4],
    /// SREP 71 subsurface: the profile's shape parameter d per channel (scene units), the lobe's weight
    pub sss: [f32; 4],
    /// The subsurface albedo (linear), unused.
    pub sss_color: [f32; 4],
}

/// Beer-Lambert coefficients per scene unit: the light left after `distance` units is `color`.
/// No distance (infinite or not positive) or a white colour means no absorption.
fn absorption(color: &[f32; 3], distance: f32) -> [f32; 4] {
    if !(distance.is_finite() && distance > 0.0) {
        return [0.0; 4];
    }
    let coefficient = |c: f32| (-(c.clamp(1e-4, 1.0).ln()) / distance).max(0.0);
    [coefficient(color[0]), coefficient(color[1]), coefficient(color[2]), 0.0]
}

/// A BVH node: bounds, and either (first triangle, count) for a leaf or (second child, 0) for an
/// interior node, whose first child is the second child's index − 1.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PtNode {
    pub lo: [f32; 3],
    pub a: u32,
    pub hi: [f32; 3],
    pub b: u32,
}

/// A light as the tracer reads it.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PtLight {
    /// position, kind (0 ambient, 1 directional, 2 point, 3 spot, 4 rect, 5 disk, 6 sphere)
    pub pos: [f32; 4],
    /// direction of travel, range (0 = unbounded)
    pub dir: [f32; 4],
    /// radiance or intensity, distance falloff exponent
    pub color: [f32; 4],
    /// cos outer, cos inner, width, height
    pub spot: [f32; 4],
    /// Radius, casts shadows, packed IES pixel offset, IES width.
    pub size: [f32; 4],
    /// Right axis (rect lights), light influence bits (1 diffuse, 2 specular)
    pub right: [f32; 4],
}

/// Packed triangles, shared prototypes and top-level instances.
#[derive(Default)]
pub struct PtScene {
    /// Triangle corners in world space or in their shared prototype space.
    pub pos: Vec<[f32; 4]>,
    pub nrm: Vec<[f32; 4]>,
    pub uv: Vec<[[f32; 2]; 6]>,
    /// The texture atlas's slices in order: packed into words ([`PtScene::atlas_pixels`]) only when the packed
    /// storage cache does not already hold them.
    pub atlas: Vec<AtlasPart>,
    pub colors: Vec<[f32; 4]>,
    pub splats: std::collections::HashMap<usize, [[f32; 4]; 24]>,
    /// Top-level instance records referencing shared local-space prototype BVHs.
    pub instances: std::collections::HashMap<usize, [[f32; 4]; 24]>,
    /// Material of each triangle.
    pub tri_mat: Vec<u32>,
    pub mats: Vec<PtMat>,
    pub nodes: Vec<PtNode>,
    pub lights: Vec<PtLight>,
    /// What the tracer leaves out, for the render notes.
    pub notes: Vec<String>,
    /// CPU time `build` spent, for statistics.
    pub timing: BuildTiming,
}

/// A slice of the path tracer's texture atlas: a texture's base level, or words kept as they are (an IES profile, the
/// one word of an empty atlas).
#[derive(Clone)]
pub enum AtlasPart {
    Texture { key: u64, rgba: std::sync::Arc<[u8]> },
    Words(Vec<u32>),
}

/// What identifies an atlas slice without its pixels: a texture by its upload key (an upload never changes its pixels),
/// other words by value.
#[derive(Clone, Debug, PartialEq)]
pub enum AtlasKey {
    Texture(u64),
    Words(Vec<u32>),
}

impl AtlasPart {
    fn len(&self) -> usize {
        match self {
            AtlasPart::Texture { rgba, .. } => rgba.len() / 4,
            AtlasPart::Words(w) => w.len(),
        }
    }
}

impl PtScene {
    /// Appends `part` to the atlas and returns the word it starts at.
    fn push_atlas(&mut self, part: AtlasPart) -> u32 {
        let offset = self.atlas.iter().map(AtlasPart::len).sum::<usize>() as u32;
        self.atlas.push(part);
        offset
    }

    /// The atlas's identity, as the packed storage cache keys it.
    pub fn atlas_key(&self) -> Vec<AtlasKey> {
        let key = |p: &AtlasPart| match p {
            AtlasPart::Texture { key, .. } => AtlasKey::Texture(*key),
            AtlasPart::Words(w) => AtlasKey::Words(w.clone()),
        };
        self.atlas.iter().map(key).collect()
    }

    /// The atlas packed into words: RGBA8 texels little-endian, other words as they are.
    pub fn atlas_pixels(&self) -> Vec<u32> {
        let mut out = Vec::with_capacity(self.atlas.iter().map(AtlasPart::len).sum());
        for part in &self.atlas {
            match part {
                AtlasPart::Texture { rgba, .. } => {
                    out.extend(rgba.as_chunks::<4>().0.iter().map(|p| u32::from_le_bytes(*p)))
                }
                AtlasPart::Words(w) => out.extend_from_slice(w),
            }
        }
        out
    }
}

/// CPU seconds `build` spent, split so BVH construction shows apart from the rest.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BuildTiming {
    /// The complete rigid geometry and its BVHs came from a retained build.
    pub geometry_reused: bool,
    /// Object-space prototype BVHs built or reused in this call.
    pub prototype_builds: usize,
    pub prototype_hits: usize,
    /// Everything but the BVHs: materials, textures, triangle expansion, prototypes, lights.
    pub assemble_seconds: f64,
    /// Construction of the top-level and prototype BVHs (`bvh`).
    pub bvh_seconds: f64,
}

/// What recording the tracer's passes cost beyond the scene build.
#[derive(Default)]
pub struct RenderTiming {
    /// Packed geometry storage uploads recorded this pass.
    pub geometry_uploads: usize,
    /// Texture atlas words packed this pass: none when the cache holds the pass's textures.
    pub atlas_texels: usize,
    /// Scenes the CPU tracer built for its tiles this pass: none when the GPU traced it.
    pub cpu_scenes: usize,
    /// Tile scratch buffers created this pass: none when the ones the tracer keeps were large enough.
    pub scratch_buffers: usize,
    /// CPU seconds packing and uploading the scene buffers before the first tile.
    pub pack_seconds: f64,
    /// Timestamps of the trace and denoise passes, when requested and supported.
    pub gpu: Option<crate::fx::Timer>,
    /// The samples the pass traced.
    pub samples: SampleCount,
}

/// The samples a pass traced, halos of tiles included. Without a noise target that is the budget; with one, a counter the
/// pass fills and [`SampleCount::traced`] reads back.
#[derive(Default)]
pub struct SampleCount {
    /// What `samples` per pixel over the pass's tiles traces.
    pub budget: u64,
    /// The mappable copy of the counter: two u32 words, low then high.
    counter: Option<wgpu::Buffer>,
    /// Whether the pass found its hits with ray queries of an acceleration structure rather than its own BVH walk.
    pub hardware_rays: bool,
}

impl SampleCount {
    /// The samples traced, once the pass's work has completed (waits for it).
    pub fn traced(&self, device: &wgpu::Device) -> u64 {
        let Some(read) = &self.counter else { return self.budget };
        let slice = read.slice(..);
        crate::gpu::map_read(device, slice).unwrap_or_else(|e| panic!("reading back the path-trace sample count: {e}"));
        let words: [u32; 2] = {
            let bytes = slice.get_mapped_range().expect("mapped");
            bytemuck::pod_read_unaligned(&bytes[..8])
        };
        read.unmap();
        u64::from(words[0]) | (u64::from(words[1]) << 32)
    }
}

fn srgb_luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// What the tracer leaves out of `scene`.
pub fn notes(scene: &Scene3) -> Vec<String> {
    let mut notes = Vec::new();
    // the tracer has no per-point shading hook: the procedural unevenness of a material (a clay finish) is raster-only
    if scene.draws.iter().any(|d| d.material.unevenness > 0.0 && !d.material.unlit) {
        notes.push("material unevenness is not applied by the path tracer".to_string());
    }
    if scene.draws.iter().any(|d| d.shadow_catcher) {
        notes.push("a shadow catcher is not drawn by the path tracer".to_string());
    }
    notes
}

/// Whether the tracer's scene buffers fit the device. Image working buffers are tiled;
/// geometry remains one binding. The error is the note reported on raster fallback.
pub fn fits(size: [u32; 2], primitives: u64, limits: &wgpu::Limits) -> Result<(), String> {
    let cap = limits.max_storage_buffer_binding_size.min(limits.max_buffer_size);
    let mib = |b: u64| b.div_ceil(1 << 20);
    // Pixel indices and shader seeds are u32. Reject impossible frames before planning tiles.
    if u64::from(size[0].max(1)) * u64::from(size[1].max(1)) > u64::from(u32::MAX) {
        return Err("path tracing pixel indices exceed u32; rasterised instead".into());
    }
    if cap < MIN_TILE_BYTES {
        return Err("path tracing device cannot bind a denoising tile; rasterised instead".into());
    }
    // Each triangle, splat or instance occupies a 384-byte record.
    let corners = primitives.saturating_mul(384);
    if corners > cap {
        return Err(format!(
            "path tracing {primitives} stored primitives need a {} MiB buffer and this device binds at most {} MiB; rasterised instead",
            mib(corners),
            cap >> 20
        ));
    }
    Ok(())
}

/// Why `scene` cannot be path traced on a device with `limits`, if it cannot.
pub fn limit_note(scene: &Scene3, limits: &wgpu::Limits) -> Option<String> {
    if let Err(error) = crate::volume::validate(&scene.volumes) {
        return Some(error);
    }
    let lights = scene.lights.len().max(1) as u32;
    if let Err(error) = crate::volume::validate_light_grid(&scene.volumes, lights, scene.env.is_some(), limits) {
        return Some(error);
    }
    let triangles = instances::storage_primitives(scene);
    if let Err(note) = fits(scene.size, triangles, limits) {
        return Some(note);
    }
    if (scene.draws.len() as u64).saturating_mul(std::mem::size_of::<PtMat>() as u64)
        > limits.max_storage_buffer_binding_size.min(limits.max_buffer_size)
    {
        return Some("path tracing materials exceed device storage binding; rasterised instead".into());
    }
    let mut keys = std::collections::HashSet::new();
    let texture_bytes = scene
        .draws
        .iter()
        .flat_map(|d| d.maps.iter().flatten())
        .filter(|t| keys.insert(t.key))
        .fold(0u64, |n, t| n.saturating_add(t.rgba.len() as u64));
    let cap = limits.max_storage_buffer_binding_size.min(limits.max_buffer_size);
    let ies_bytes = scene.lights.iter().filter_map(|l| l.ies.as_ref()).map(|p| p.len() as u64 * 4).sum::<u64>();
    let bytes = triangles
        .saturating_mul(384)
        .saturating_add(texture_bytes)
        .saturating_add(ies_bytes)
        .saturating_add(crate::volume::bytes(&scene.volumes))
        .saturating_add(128);
    if bytes > cap {
        return Some(format!(
            "path tracing geometry, textures and volumes require {} MiB, device permits {} MiB; rasterised instead",
            bytes.div_ceil(1 << 20),
            cap >> 20
        ));
    }
    None
}

struct Triangle {
    p: [Vec3; 3],
    n: [Vec3; 3],
    uv: [[[f32; 2]; 6]; 3],
    colors: [[f32; 4]; 3],
}
fn triangle(dr: &Draw3, t: &[u32; 3], model: Mat4, nmat: Mat4) -> Triangle {
    let m = &dr.material;
    let verts = dr.mesh.cpu().0;
    let p = std::array::from_fn(|c| {
        let v = &verts[t[c] as usize];
        let mut pos = Vec3::from(v.pos);
        if let Some(map) = &dr.maps[5] {
            let uv = [v.uv[0] * m.uv_scale[0], v.uv[1] * m.uv_scale[1]];
            pos += Vec3::from(v.normal).normalize_or_zero() * map.sample(uv)[0] * m.displacement_scale;
        }
        model.transform_point3(pos)
    });
    let n =
        std::array::from_fn(|c| nmat.transform_vector3(Vec3::from(verts[t[c] as usize].normal)).normalize_or_zero());
    let uv = std::array::from_fn(|c| {
        let v = &verts[t[c] as usize];
        std::array::from_fn(|slot| {
            let uv = if m.separate_uvs && (1..5).contains(&slot) { v.map_uv[slot - 1] } else { v.uv };
            [uv[0] * m.uv_scale[0], uv[1] * m.uv_scale[1]]
        })
    });
    Triangle { p, n, uv, colors: std::array::from_fn(|i| verts[t[i] as usize].color) }
}

/// Builds world-space and shared prototype geometry, materials, lights and both BVH levels.
pub fn build(scene: &Scene3) -> PtScene {
    BuildCache::new(0).build(scene)
}

fn build_inner(scene: &Scene3, cache: &mut BuildCache, take: bool) -> (PtScene, Option<cache::Ticket>) {
    let started = std::time::Instant::now();
    let mut bvh_seconds = 0.0;
    let mut s = PtScene::default();
    let key = cache::SceneKey::of(scene);
    let reused = cache.reuse(key.as_ref(), take);
    let id = reused.as_ref().map_or_else(cache::next_id, |g| g.id);
    s.timing.geometry_reused = reused.is_some();
    let mut tris: Vec<[Vec3; 3]> = Vec::new();
    let mut norms: Vec<[Vec3; 3]> = Vec::new();
    let mut tmat: Vec<u32> = Vec::new();
    let mut texcoords = Vec::new();
    let mut colors: Vec<[[f32; 4]; 3]> = Vec::new();
    let mut texture_offsets = std::collections::HashMap::new();
    let repetitions = instances::repetitions(scene);
    let mut prototype_ids = std::collections::HashMap::new();
    let mut prototypes: Vec<std::sync::Arc<cache::Prototype>> = Vec::new();
    let mut instance_records = std::collections::HashMap::new();
    // a shadow catcher has no colour of its own and the tracer cannot evaluate its darkening: it is left out (see `notes`)
    for dr in scene.draws.iter().filter(|d| !d.shadow_catcher) {
        let m = &dr.material;
        let maps = std::array::from_fn(|slot| {
            dr.maps[slot].as_ref().map_or([0; 4], |texture| {
                let offset = *texture_offsets.entry(texture.key).or_insert_with(|| {
                    s.push_atlas(AtlasPart::Texture { key: texture.key, rgba: texture.rgba.clone() })
                });
                [
                    offset,
                    texture.size[0],
                    texture.size[1],
                    texture.srgb as u32 | texture.sampler.map_or(0, |s| s.flags()),
                ]
            })
        });
        let unlit = m.unlit;
        let emissive = [
            m.emissive[0] * m.emissive_strength,
            m.emissive[1] * m.emissive_strength,
            m.emissive[2] * m.emissive_strength,
        ];
        let opacity =
            if m.alpha_mode != sr_3d::AlphaMode::Opaque { m.base_color[3] * m.opacity } else { 1.0 } * dr.opacity;
        let mi = s.mats.len() as u32;
        s.mats.push(PtMat {
            base: [m.base_color[0], m.base_color[1], m.base_color[2], opacity.clamp(0.0, 1.0)],
            params: [
                m.metallic.clamp(0.0, 1.0),
                m.roughness.clamp(0.02, 1.0),
                m.transmission.clamp(0.0, 1.0),
                m.ior.max(1.0),
            ],
            emissive: [emissive[0], emissive[1], emissive[2], unlit as u32 as f32],
            extra: [
                m.specular * srgb_luma(m.specular_color).max(0.0),
                dr.cast_shadow as u32 as f32,
                dr.receive_shadow as u32 as f32,
                // foam mixed into the surface (the water shader's hook): 1 + the foam's roughness, or 0
                m.foam_mix.map_or(0.0, |f| 1.0 + f.roughness.clamp(0.0, 1.0)),
            ],
            maps,
            borders: std::array::from_fn(|i| {
                dr.maps[i].as_ref().and_then(|t| t.sampler).map_or([0.0; 4], |s| s.border)
            }),
            texture_params: [
                m.alpha_cutoff,
                match m.alpha_mode {
                    sr_3d::AlphaMode::Opaque => 0.0,
                    sr_3d::AlphaMode::Mask => 1.0,
                    sr_3d::AlphaMode::Blend => 2.0,
                },
                m.normal_scale,
                m.occlusion_strength,
            ],
            attenuation: {
                // the fourth value is the albedo of foam mixed into the surface, when it is
                let mut a = absorption(&m.attenuation_color, m.attenuation_distance);
                a[3] = m.foam_mix.map_or(0.0, |f| f.albedo.clamp(0.0, 1.0));
                a
            },
            sss: {
                let d = sr_3d::subsurface::shape_parameters(
                    m.subsurface_radius,
                    m.subsurface_radius_scale,
                    m.subsurface_color,
                );
                [d[0], d[1], d[2], m.subsurface.clamp(0.0, 1.0)]
            },
            sss_color: [m.subsurface_color[0], m.subsurface_color[1], m.subsurface_color[2], 0.0],
        });
        if reused.is_some() {
            continue;
        }
        if let Some(key) = instances::key(dr).filter(|k| repetitions[k] > 1) {
            let prototype = *prototype_ids.entry(key).or_insert_with(|| {
                prototypes.push(cache.prototype(dr, &mut s.timing));
                prototypes.len() - 1
            });
            let MeshSrc::Cached(mesh) = &dr.mesh else { unreachable!() };
            instance_records.insert(tris.len(), instances::record(dr.model, prototype, mi));
            tris.push(instances::bounds(dr.model, mesh.lo, mesh.hi));
            norms.push([Vec3::ZERO; 3]);
            texcoords.push([[[0.; 2]; 6]; 3]);
            colors.push([[1.; 4]; 3]);
            tmat.push(u32::MAX - 1);
        } else {
            let normal = dr.model.inverse().transpose();
            for t in dr.mesh.cpu().1.as_chunks::<3>().0 {
                let t = triangle(dr, t, dr.model, normal);
                tris.push(t.p);
                norms.push(t.n);
                texcoords.push(t.uv);
                colors.push(t.colors);
                tmat.push(mi);
            }
        }
    }

    if s.atlas.is_empty() {
        s.push_atlas(AtlasPart::Words(vec![u32::MAX]));
    }
    s.notes = notes(scene);
    if let Some(geometry) = reused {
        geometry.apply(&mut s);
    } else {
        let mut splat_records = std::collections::HashMap::new();
        for cloud in &scene.splats {
            let linear = glam::Mat3::from_mat4(cloud.model);
            let local = glam::Mat3::from_mat4(cloud.model.inverse());
            for (index, data) in cloud.gpu.cpu.iter().take(cloud.gpu.n as usize).enumerate() {
                let center = cloud.model.transform_point3(Vec3::new(data[0], data[1], data[2]));
                let covariance = glam::Mat3::from_cols_array(&[
                    data[4], data[5], data[6], data[5], data[7], data[8], data[6], data[8], data[9],
                ]);
                let covariance = linear * covariance * linear.transpose();
                let inverse = covariance.inverse();
                if !inverse.is_finite() {
                    continue;
                }
                let extent = Vec3::from_array(
                    [covariance.x_axis.x, covariance.y_axis.y, covariance.z_axis.z].map(|v| v.max(0.0).sqrt()),
                ) * 3.0;
                let mut record = [[0.0; 4]; 24];
                record[0] = [center.x, center.y, center.z, f32::from_bits(u32::MAX)];
                for (row, axis) in inverse.to_cols_array_2d().into_iter().enumerate() {
                    record[row + 1] = [axis[0], axis[1], axis[2], 0.0];
                }
                record[4] = [data[12], data[13], data[14], data[3] * cloud.opacity];
                for (row, axis) in local.to_cols_array_2d().into_iter().enumerate() {
                    record[row + 5] = [axis[0], axis[1], axis[2], 0.0];
                }
                if let Some(sh) = cloud.gpu.cpu_sh.get(index) {
                    for (row, values) in sh.as_chunks::<4>().0.iter().enumerate() {
                        record[row + 8].copy_from_slice(values);
                    }
                    record[20][0] = cloud.gpu.sh_degree as f32;
                }
                splat_records.insert(tris.len(), record);
                tris.push([center - extent, center + extent, center]);
                norms.push([Vec3::ZERO; 3]);
                texcoords.push([[[0.0; 2]; 6]; 3]);
                colors.push([[1.0; 4]; 3]);
                tmat.push(u32::MAX);
            }
        }
        // Top-level BVH, then world triangles / splats / instance records in leaf order
        let clock = std::time::Instant::now();
        let order = bvh(&tris, &mut s.nodes);
        bvh_seconds += clock.elapsed().as_secs_f64();
        for (new_index, &t) in order.iter().enumerate() {
            if let Some(record) = instance_records.remove(&t) {
                s.instances.insert(new_index, record);
            }
            if let Some(record) = splat_records.remove(&t) {
                s.splats.insert(new_index, record);
            }
            for c in 0..3 {
                let p = tris[t][c];
                let q = norms[t][c];
                s.pos.push([p.x, p.y, p.z, 0.0]);
                s.nrm.push([q.x, q.y, q.z, 0.0]);
                s.uv.push(texcoords[t][c]);
                s.colors.push(colors[t][c]);
            }
            s.tri_mat.push(tmat[t]);
        }
        let mut roots = Vec::new();
        for prototype in &prototypes {
            let mut nodes = prototype.nodes.clone();
            let order = &prototype.order;
            let root = s.nodes.len() as u32;
            let first = s.tri_mat.len() as u32;
            for node in &mut nodes {
                node.a += if node.b == 0 { root } else { first };
            }
            roots.push(root);
            s.nodes.extend(nodes);
            for &index in order {
                let t = &prototype.triangles[index];
                for c in 0..3 {
                    s.pos.push(t.p[c].extend(0.).to_array());
                    s.nrm.push(t.n[c].extend(0.).to_array());
                    s.uv.push(t.uv[c]);
                    s.colors.push(t.colors[c]);
                }
                s.tri_mat.push(0);
            }
        }
        for record in s.instances.values_mut() {
            record[13][0] = f32::from_bits(roots[record[13][0].to_bits() as usize]);
        }
    }
    for l in &scene.lights {
        let kind = match l.kind {
            LightKind::Ambient => 0.0,
            LightKind::Directional => 1.0,
            LightKind::Point => 2.0,
            LightKind::Spot => 3.0,
            LightKind::Rect => 4.0,
            LightKind::Disk => 5.0,
            LightKind::Sphere => 6.0,
        };
        let ies =
            l.ies.as_ref().map(|profile| s.push_atlas(AtlasPart::Words(profile.iter().map(|v| v.to_bits()).collect())));
        s.lights.push(PtLight {
            pos: [l.pos.x, l.pos.y, l.pos.z, kind],
            dir: [l.dir.x, l.dir.y, l.dir.z, l.range],
            color: [l.color.x, l.color.y, l.color.z, l.falloff],
            spot: [l.cos_outer, l.cos_inner, l.size[0], l.size[1]],
            size: [
                l.size[2],
                l.cast_shadow as u32 as f32,
                f32::from_bits(ies.unwrap_or(0)),
                if ies.is_some() { 128.0 } else { 0.0 },
            ],
            right: [
                l.right.x,
                l.right.y,
                l.right.z,
                (l.affects_diffuse as u32 | ((l.affects_specular as u32) << 1)) as f32,
            ],
        });
    }
    if s.lights.is_empty() {
        s.lights.push(PtLight {
            pos: [0.0, 0.0, 0.0, -1.0],
            dir: [0.0; 4],
            color: [0.0; 4],
            spot: [0.0; 4],
            size: [0.0; 4],
            right: [0.0; 4],
        });
    }
    if s.mats.is_empty() {
        s.mats.push(PtMat {
            base: [0.0; 4],
            params: [0.0; 4],
            emissive: [0.0; 4],
            extra: [0.0; 4],
            maps: [[0; 4]; 6],
            borders: [[0.0; 4]; 6],
            texture_params: [0.0; 4],
            attenuation: [0.0; 4],
            sss: [0.0; 4],
            sss_color: [0.0; 4],
        });
    }
    if s.pos.is_empty() {
        s.pos.extend([[0.0; 4]; 3]);
        s.nrm.extend([[0.0; 4]; 3]);
        s.tri_mat.push(0);
    }
    let ticket = if take {
        key.map(|key| cache::Ticket { key, id })
    } else {
        if !s.timing.geometry_reused {
            cache.remember(key, &s, id);
        }
        None
    };
    bvh_seconds += s.timing.bvh_seconds;
    s.timing.bvh_seconds = bvh_seconds;
    s.timing.assemble_seconds = (started.elapsed().as_secs_f64() - bvh_seconds).max(0.0);
    (s, ticket)
}

/// Binned SAH BVH. Returns the triangle order its leaves index.
fn bvh(tris: &[[Vec3; 3]], nodes: &mut Vec<PtNode>) -> Vec<usize> {
    let n = tris.len();
    let mut idx: Vec<usize> = (0..n).collect();
    nodes.clear();
    if n == 0 {
        nodes.push(PtNode { lo: [1.0; 3], a: 0, hi: [-1.0; 3], b: 1 });
        return idx;
    }
    let bounds: Vec<(Vec3, Vec3)> = tris.iter().map(|t| (t[0].min(t[1]).min(t[2]), t[0].max(t[1]).max(t[2]))).collect();
    let cent: Vec<Vec3> = bounds.iter().map(|(l, h)| (*l + *h) * 0.5).collect();
    fn area(lo: Vec3, hi: Vec3) -> f32 {
        let d = (hi - lo).max(Vec3::ZERO);
        2.0 * (d.x * d.y + d.y * d.z + d.z * d.x)
    }
    // (node index, start, end) to build; a node's second child index is patched when known
    let mut stack = vec![(0usize, 0usize, n, 0u32)];
    nodes.push(PtNode::default());
    while let Some((ni, start, end, depth)) = stack.pop() {
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        let (mut clo, mut chi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for &t in &idx[start..end] {
            lo = lo.min(bounds[t].0);
            hi = hi.max(bounds[t].1);
            clo = clo.min(cent[t]);
            chi = chi.max(cent[t]);
        }
        let count = end - start;
        let leaf = |nodes: &mut Vec<PtNode>| {
            nodes[ni] = PtNode { lo: lo.into(), a: start as u32, hi: hi.into(), b: count as u32 };
        };
        if count <= 4 || depth >= 30 {
            leaf(nodes);
            continue;
        }
        // best binned split over the three axes
        const BINS: usize = 12;
        let mut best: Option<(usize, f32, f32)> = None;
        let ext = chi - clo;
        for axis in 0..3 {
            if ext[axis] <= 1e-9 {
                continue;
            }
            let mut bin_n = [0usize; BINS];
            let mut bin_b = [(Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)); BINS];
            for &t in &idx[start..end] {
                let k = (((cent[t][axis] - clo[axis]) / ext[axis]) * BINS as f32).min(BINS as f32 - 1.0) as usize;
                bin_n[k] += 1;
                bin_b[k].0 = bin_b[k].0.min(bounds[t].0);
                bin_b[k].1 = bin_b[k].1.max(bounds[t].1);
            }
            for split in 1..BINS {
                let (mut ln, mut rn) = (0, 0);
                let (mut ll, mut lh, mut rl, mut rh) =
                    (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN), Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                for k in 0..split {
                    ln += bin_n[k];
                    ll = ll.min(bin_b[k].0);
                    lh = lh.max(bin_b[k].1);
                }
                for k in split..BINS {
                    rn += bin_n[k];
                    rl = rl.min(bin_b[k].0);
                    rh = rh.max(bin_b[k].1);
                }
                if ln == 0 || rn == 0 {
                    continue;
                }
                let cost = ln as f32 * area(ll, lh) + rn as f32 * area(rl, rh);
                if best.map(|b| cost < b.1).unwrap_or(true) {
                    best = Some((axis, cost, clo[axis] + ext[axis] * split as f32 / BINS as f32));
                }
            }
        }
        let Some((axis, cost, pos)) = best else {
            leaf(nodes);
            continue;
        };
        if cost >= count as f32 * area(lo, hi) && count <= 16 {
            leaf(nodes);
            continue;
        }
        let mid = {
            let s = &mut idx[start..end];
            let mut i = 0;
            for j in 0..s.len() {
                if cent[s[j]][axis] < pos {
                    s.swap(i, j);
                    i += 1;
                }
            }
            start + i
        };
        if mid == start || mid == end {
            leaf(nodes);
            continue;
        }
        // the children are allocated as a pair: an interior node keeps the second's index in
        // `a` (and `b` = 0); the first is `a` − 1
        let left = nodes.len();
        nodes.push(PtNode::default());
        let right = nodes.len();
        nodes.push(PtNode::default());
        nodes[ni] = PtNode { lo: lo.into(), a: right as u32, hi: hi.into(), b: 0 };
        stack.push((right, mid, end, depth + 1));
        stack.push((left, start, mid, depth + 1));
    }
    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Measures how long the adapter takes to compile and link one trace variant, named by `SR_PT_VARIANT` as
    /// space-separated flags out of `media grid lighting water foam scatter` (the README's cold-start figures).
    /// Run it once per variant in its own process, with the driver's shader cache off, cold or warm.
    #[test]
    #[ignore = "a measurement: set SR_PT_VARIANT and run in its own process"]
    fn link_one_trace_variant() {
        let names = std::env::var("SR_PT_VARIANT").expect("SR_PT_VARIANT, e.g. \"media grid lighting\"");
        let flags: Vec<&str> = names.split_whitespace().collect();
        let known = ["media", "grid", "lighting", "water", "foam", "scatter"];
        assert!(flags.iter().all(|f| known.contains(f)), "{names:?} names a flag out of {known:?}");
        let on = |f: &str| flags.contains(&f);
        let gpu = sr_testkit::require_gpu!(crate::gpu::test_gpu(), "link_one_trace_variant");
        println!("adapter {} {:?}", gpu.info.name, gpu.info.backend);
        let d = &gpu.device;
        let pt = PtGpu::new(d, crate::resources::FORMAT);
        let start = std::time::Instant::now();
        let (media, grid) = (on("media"), on("grid"));
        let (source, constants) = trace_shader(media, on("lighting"), grid, on("water"), on("foam"), on("scatter"));
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let plain;
        let layout = if media && grid {
            &pt.pipelines.grid_pipelines(d).layout
        } else {
            plain = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[Some(&pt.pipelines.bgl0)],
                immediate_size: 0,
            });
            &plain
        };
        let scope = d.push_error_scope(wgpu::ErrorFilter::Validation);
        let _pipeline = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: Some(layout),
            module: &module,
            entry_point: Some("cs_trace"),
            compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() },
            cache: None,
        });
        let error = pollster::block_on(scope.pop());
        d.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        println!("link {:.3} s", start.elapsed().as_secs_f64());
        assert!(error.is_none(), "{names}: {error:?}");
    }

    #[test]
    fn tracers_on_one_device_share_its_pipelines_and_keep_their_own_settings() {
        let gpu = sr_testkit::require_gpu!(
            crate::gpu::test_gpu(),
            "tracers_on_one_device_share_its_pipelines_and_keep_their_own_settings"
        );
        let mut a = PtGpu::new(&gpu.device, crate::resources::FORMAT);
        let b = PtGpu::new(&gpu.device, crate::resources::FORMAT);
        assert!(std::sync::Arc::ptr_eq(&a.pipelines, &b.pipelines), "one set per device and output format");
        let other = PtGpu::new(&gpu.device, wgpu::TextureFormat::Rgba8Unorm);
        assert!(!std::sync::Arc::ptr_eq(&a.pipelines, &other.pipelines), "another output format has its own");
        assert_ne!(a.pipelines.output, other.pipelines.output);
        a.set_hardware_rays(false);
        a.limit_buffers(MIN_TILE_BYTES).unwrap();
        assert_eq!((a.rays, a.tile_bytes), (false, MIN_TILE_BYTES));
        assert_eq!((b.rays, b.tile_bytes), (true, DEFAULT_TILE_BYTES), "settings stay with their tracer");
        assert_eq!(a.rays_layout(), None);
        assert_eq!(b.rays_layout(), b.pipelines.bgl_rays.as_ref());
    }

    #[test]
    fn foam_alone_selects_the_foam_variant_and_never_the_water_one_and_every_variant_has_its_own_pipeline_slot() {
        let plain = sr_3d::MaterialParams::default();
        let glass = sr_3d::MaterialParams { transmission: 1.0, ..Default::default() };
        let mixed = sr_3d::MaterialParams {
            foam_mix: Some(sr_3d::FoamMix { albedo: 0.9, roughness: 0.8 }),
            ..Default::default()
        };
        let mixed_glass = sr_3d::MaterialParams { foam_mix: mixed.foam_mix, ..glass.clone() };
        assert_eq!(variant_of([&plain].into_iter()), (false, false));
        assert_eq!(variant_of([&plain, &glass].into_iter()), (true, false));
        assert_eq!(variant_of([&plain, &mixed].into_iter()), (false, true), "foam does not bring in the water variant");
        assert_eq!(variant_of([&glass, &mixed].into_iter()), (true, true));
        assert_eq!(variant_of([&mixed_glass].into_iter()), (true, true));
        // only scenes the plain shader traces can go to the CPU tracer, which ports neither media nor water nor foam
        assert!(plain_trace([&plain].into_iter(), false));
        assert!(!plain_trace([&plain].into_iter(), true), "media");
        assert!(!plain_trace([&plain, &glass].into_iter(), false), "water");
        assert!(!plain_trace([&plain, &mixed].into_iter(), false), "foam");
        // 5 shapes of scene (media, grid, lighting) times 3 variants: 15 distinct slots in 0..15
        let mut slots = std::collections::BTreeSet::new();
        for (media, grid, lighting) in
            [(false, false, false), (true, false, false), (true, false, true), (true, true, false), (true, true, true)]
        {
            for (water, foam) in [(true, false), (true, true), (false, true)] {
                slots.insert(variant_slot(media, grid, lighting, water, foam, false));
            }
        }
        assert_eq!(slots.len(), 15);
        assert_eq!((slots.first(), slots.last()), (Some(&0), Some(&14)));
    }

    #[test]
    fn scatter_bounces_other_than_one_is_a_variant_of_its_own_and_one_leaves_every_text_as_it_was() {
        // one is the single scattering that the shader has: no pipeline of its own, the text it had
        assert!(!needs_scatter([1u32, 1].into_iter()));
        assert!(!needs_scatter(std::iter::empty()));
        assert!(needs_scatter([1u32, 4].into_iter()) && needs_scatter([0u32].into_iter()));
        // the slots: the fifteen there were keep their numbers, a scene with more scattering and no water or foam has its own, and every
        // shape of scene with scattering is another set of twenty
        let shapes =
            [(false, false, false), (true, false, false), (true, false, true), (true, true, false), (true, true, true)];
        let mut before = std::collections::BTreeSet::new();
        let mut all = std::collections::BTreeSet::new();
        for (media, grid, lighting) in shapes {
            for (water, foam) in [(true, false), (true, true), (false, true), (false, false)] {
                for scatter in [false, true] {
                    let slot = variant_slot(media, grid, lighting, water, foam, scatter);
                    assert!(all.insert(slot), "slot {slot} is taken twice");
                    if !scatter && (water || foam) {
                        before.insert(slot);
                    }
                }
            }
        }
        assert_eq!(before.len(), 15);
        assert_eq!(
            (before.first(), before.last()),
            (Some(&0), Some(&14)),
            "the slots of the variants that were are those they had"
        );
        assert_eq!((all.first(), all.last(), all.len()), (Some(&0), Some(&39), 40));
        // the text: without scattering the base is not touched, by any of the additions
        let base = format!("{}\n{}", include_str!("pathtrace.wgsl"), VOLUME_WGSL);
        assert_eq!(variant_source(&base, false, false, false), base);
        assert_eq!(variant_source(&base, true, false, false), variant_source_without_scatter(&base, true, false));
    }

    #[test]
    fn every_variant_translates_to_the_glsl_opengl_compiles() {
        let plain = format!("{}\n{}\n{}", include_str!("sampling.wgsl"), include_str!("pathtrace.wgsl"), VOLUME_WGSL);
        for base in [plain, grid_source()] {
            for water in [false, true] {
                for foam in [false, true] {
                    for scatter in [false, true] {
                        let source = variant_source(&base, water, foam, scatter);
                        for media in [false, true] {
                            let constants = variant_constants(media, media, water, foam);
                            crate::three::assert_translates_to_glsl("path tracer", &source, &constants);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn specialize_takes_out_the_blocks_a_false_override_guards_and_keeps_the_rest() {
        let source = "override A: bool = false;\noverride B: bool = false;\noverride C: bool = true;\nfn f() {\n    \
                      if (A) { x(); }\n    if (A && y > 0.0) { if (z) { w(); } }\n    if (B) { b(); }\n    \
                      if (q) { r(); } else if (A) { s(); }\n    if (C) { c(); }\n    if (AB) { t(); }\n}\n";
        assert_eq!(
            specialize(source, &[("B", 1.0)]),
            "override A: bool = false;\noverride B: bool = false;\noverride C: bool = true;\nfn f() {\n    \
             if (B) { b(); }\n    if (q) { r(); }\n    if (C) { c(); }\n    if (AB) { t(); }\n}\n",
            "A is false by default, B is set, C is true by default; AB is not an override"
        );
        // a constant turns a default off
        assert_eq!(
            specialize("override C: bool = true;\nfn f() {\n    if (C) { c(); }\n}\n", &[("C", 0.0)]),
            "override C: bool = true;\nfn f() {\n}\n"
        );
    }

    #[test]
    #[should_panic(expected = "a use of A that specialize does not take out")]
    fn specialize_refuses_a_guard_it_cannot_take_out_exactly() {
        specialize("override A: bool = false;\nfn f() {\n    if (!A) { x(); }\n}\n", &[]);
    }

    #[test]
    #[should_panic(expected = "an else after a false guard of A")]
    fn specialize_refuses_an_else_after_a_false_guard() {
        specialize("override A: bool = false;\nfn f() {\n    if (A) { x(); } else { y(); }\n}\n", &[]);
    }

    /// NVIDIA's OpenGL compiler inlines and optimizes what an `if (false)` holds before it drops it, so the volume and
    /// water calls under false overrides cost every trace shader minutes and gigabytes to compile (the `backends` test
    /// was killed for memory). Every trace shader the pipelines are given has them taken out, and interpolates sparse
    /// grids with one loop instead of eight inlined lookups.
    #[test]
    fn every_trace_shader_is_handed_over_without_the_code_its_false_overrides_guard() {
        for media in [false, true] {
            for lighting in [false, true] {
                for grid in [false, true] {
                    for water in [false, true] {
                        for foam in [false, true] {
                            for scatter in [false, true] {
                                let (source, constants) = trace_shader(media, lighting, grid, water, foam, scatter);
                                let which = format!("media {media} lighting {lighting} grid {grid} water {water} foam {foam} scatter {scatter}");
                                for flag in ["HAS_MEDIA", "MEDIUM_LIGHTING", "WATER", "FOAM"] {
                                    let on = constants.iter().any(|(k, v)| *k == flag && *v != 0.0);
                                    assert_eq!(
                                        source.contains(&format!("if ({flag}")),
                                        on,
                                        "{which}: the blocks {flag} guards are there exactly when it is on"
                                    );
                                }
                                assert!(
                                    !source.contains("for (var z=0; z<2; z++)"),
                                    "{which}: sparse grids are interpolated with one runtime loop"
                                );
                                crate::three::assert_translates_to_glsl("path tracer", &source, &constants);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Drivers inline every call (NVIDIA's OpenGL compiler then takes minutes and gigabytes per copy of the sparse
    /// search), so each step of sampling a domain is called from one place: a march samples density and emission
    /// together, each frame's point is advected once, and the stages, components and corners are runtime loops.
    #[test]
    fn every_trace_shader_samples_a_domain_from_one_call_site_of_each_step() {
        let calls = |source: &str, name: &str| {
            regex::Regex::new(&format!(r"\b{name}\(")).unwrap().find_iter(source).count()
                - source.matches(&format!("fn {name}(")).count()
        };
        for (grid, water, foam, scatter) in
            (0..16u8).map(|bits| (bits & 1 != 0, bits & 2 != 0, bits & 4 != 0, bits & 8 != 0))
        {
            for lighting in [false, true] {
                let (source, _) = trace_shader(true, lighting, grid, water, foam, scatter);
                let which = format!("lighting {lighting} grid {grid} water {water} foam {foam} scatter {scatter}");
                for (name, sites) in [
                    ("volume_voxel", 1),
                    ("volume_grid_sample", 2),
                    ("volume_grid", 1),
                    ("volume_advected_point", 1),
                    ("volume_velocity", 0),
                    ("volume_emission", 0),
                ] {
                    assert_eq!(calls(&source, name), sites, "{which}: call sites of {name}");
                }
            }
        }
    }

    /// NVIDIA's OpenGL compiler inlines every call, so each march over the media brings its own copy of the media's
    /// scattering and, with the random walk, of the walk: two marches doubled the link time of a lit water variant
    /// with scattering (29 s to 59 s on an RTX 3050 with the shader cache off). A shader with water marches the media
    /// outside the water with the water's march, at no absorption.
    #[test]
    fn every_trace_shader_marches_its_media_from_one_call_site() {
        let calls = |source: &str, name: &str| {
            regex::Regex::new(&format!(r"\b{name}\(")).unwrap().find_iter(source).count()
                - source.matches(&format!("fn {name}(")).count()
        };
        for (grid, water, foam, scatter) in
            (0..16u8).map(|bits| (bits & 1 != 0, bits & 2 != 0, bits & 4 != 0, bits & 8 != 0))
        {
            for lighting in [false, true] {
                let (source, _) = trace_shader(true, lighting, grid, water, foam, scatter);
                let which = format!("lighting {lighting} grid {grid} water {water} foam {foam} scatter {scatter}");
                let marches = calls(&source, "volume_transport") + calls(&source, "water_transport");
                assert_eq!(marches, 1, "{which}: call sites of the media's marches");
                assert_eq!(calls(&source, "scatter_walk"), scatter as usize, "{which}: call sites of the walk");
                assert_eq!(
                    calls(&source, "volume_scattering"),
                    1 + scatter as usize,
                    "{which}: call sites of the media's scattering"
                );
            }
        }
    }

    /// The functions of a WGSL source by name, each with the text of its body.
    fn wgsl_functions(source: &str) -> std::collections::HashMap<String, String> {
        let head = regex::Regex::new(r"(?m)^fn (\w+)\(").unwrap();
        head.captures_iter(source)
            .map(|c| {
                let start = c.get(0).unwrap().end();
                let open = start + source[start..].find('{').expect("a body");
                let mut depth = 0;
                let end = open
                    + source[open..]
                        .char_indices()
                        .find(|&(_, ch)| {
                            depth += match ch {
                                '{' => 1,
                                '}' => -1,
                                _ => 0,
                            };
                            depth == 0
                        })
                        .expect("the body closes")
                        .0;
                (c[1].to_string(), source[open..=end].to_string())
            })
            .collect()
    }

    /// The functions `from` calls, directly or through others.
    fn wgsl_reached(
        functions: &std::collections::HashMap<String, String>,
        from: &str,
    ) -> std::collections::BTreeSet<String> {
        let mut reached = std::collections::BTreeSet::new();
        let mut pending = vec![from.to_string()];
        while let Some(f) = pending.pop() {
            for name in functions.keys() {
                let call = regex::Regex::new(&format!(r"\b{name}\(")).unwrap();
                if *name != f && call.is_match(&functions[&f]) && reached.insert(name.clone()) {
                    pending.push(name.clone());
                }
            }
        }
        reached
    }

    /// Drivers inline every call, so a march that marches a shadow ray at each of its steps holds a copy of the shadow
    /// ray's march, of the BVH walk past surfaces and of the map lookup of their opacity inside its own loops: NVIDIA's
    /// OpenGL compiler takes over 7 GB and minutes on the shader of lit smoke. The light a lit domain scatters is the
    /// events of one loop instead (each light, each domain its shadow ray crosses and each step of it, each surface in
    /// its way), called once by each march, which reaches no function that marches a shadow ray of its own.
    #[test]
    fn no_trace_shader_marches_a_shadow_ray_inside_a_march() {
        let walk_call = regex::Regex::new(r"\bvolume_scattering\(").unwrap();
        for (grid, water, foam, scatter) in
            (0..16u8).map(|bits| (bits & 1 != 0, bits & 2 != 0, bits & 4 != 0, bits & 8 != 0))
        {
            let (source, _) = trace_shader(true, true, grid, water, foam, scatter);
            let which = format!("grid {grid} water {water} foam {foam} scatter {scatter}");
            let functions = wgsl_functions(&source);
            let marches: Vec<&str> = ["volume_transport", "water_transport", "scatter_walk"]
                .into_iter()
                .filter(|m| functions.contains_key(*m))
                .collect();
            // the water's march marches the media outside the water too
            assert_eq!(marches.len(), 1 + scatter as usize, "{which}: the marches {marches:?}");
            for march in &marches {
                let reached = wgsl_reached(&functions, march);
                for inner in ["volume_transmittance", "visibility", "volume_incident"] {
                    assert!(!reached.contains(inner), "{which}: {march} reaches {inner}");
                }
                assert!(reached.contains("volume_scattering"), "{which}: {march} lights the media");
            }
            let walk = wgsl_reached(&functions, "volume_scattering");
            for name in ["volume_sample", "trace", "light_sample", "map_sample"] {
                assert!(walk.contains(name), "{which}: the walk reaches {name}");
            }
            let calls = |name: &str| {
                regex::Regex::new(&format!(r"\b{name}\(")).unwrap().find_iter(&functions["volume_scattering"]).count()
            };
            for name in ["volume_sample", "trace", "light_sample", "map_sample", "volume_incident_grid"] {
                assert_eq!(calls(name), 1, "{which}: the walk calls {name} from one place");
            }
            let sites = walk_call.find_iter(&source).count() - 1;
            assert_eq!(sites, marches.len(), "{which}: each march calls the walk once");
        }
    }

    #[test]
    fn scatter_shader_validates_with_water_foam_and_grid_combinations() {
        let plain = format!("{}\n{}\n{}", include_str!("sampling.wgsl"), include_str!("pathtrace.wgsl"), VOLUME_WGSL);
        for base in [plain, grid_source()] {
            for water in [false, true] {
                for foam in [false, true] {
                    for scatter in [false, true] {
                        let source = variant_source(&base, water, foam, scatter);
                        let module = naga::front::wgsl::parse_str(&source)
                            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
                        naga::valid::Validator::new(
                            naga::valid::ValidationFlags::all(),
                            naga::valid::Capabilities::all(),
                        )
                        .validate(&module)
                        .unwrap_or_else(|e| panic!("{e:?}"));
                    }
                }
            }
        }
    }

    #[test]
    fn every_trace_shader_validates_with_hardware_traversal_in_place_of_the_bvh_walk() {
        let plain = format!("{}\n{}\n{}", include_str!("sampling.wgsl"), include_str!("pathtrace.wgsl"), VOLUME_WGSL);
        for base in [plain, grid_source()] {
            for water in [false, true] {
                for foam in [false, true] {
                    for scatter in [false, true] {
                        let source = ray_query_source(&variant_source(&base, water, foam, scatter));
                        assert!(!source.contains("var stack: array<u32, 64>;\n    var sp = 0;\n    stack[0] = 0u;"));
                        assert_eq!(source.matches("fn trace(o: vec3<f32>").count(), 1);
                        let module = naga::front::wgsl::parse_str(&source)
                            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
                        naga::valid::Validator::new(
                            naga::valid::ValidationFlags::all(),
                            naga::valid::Capabilities::all(),
                        )
                        .validate(&module)
                        .unwrap_or_else(|e| panic!("{e:?}"));
                    }
                }
            }
        }
    }

    #[test]
    fn a_variant_pipeline_is_given_water_only_for_water_and_foam_only_for_foam() {
        let plain = format!("{}\n{}\n{}", include_str!("sampling.wgsl"), include_str!("pathtrace.wgsl"), VOLUME_WGSL);
        let names = |c: Vec<(&str, f64)>| c.into_iter().map(|(n, v)| (n.to_string(), v)).collect::<Vec<_>>();
        // foam alone: the media constants, and neither WATER nor FOAM (the module has no FOAM override to give a value to)
        assert_eq!(
            names(variant_constants(false, false, false, true)).iter().map(|c| c.0.as_str()).collect::<Vec<_>>(),
            ["HAS_MEDIA", "MEDIUM_LIGHTING"]
        );
        // water: WATER on and FOAM off; water with foam: FOAM on
        assert!(variant_constants(true, false, true, false).contains(&("WATER", 1.0)));
        assert!(variant_constants(true, false, true, false).contains(&("FOAM", 0.0)));
        assert!(variant_constants(true, true, true, true).contains(&("FOAM", 1.0)));
        assert!(variant_constants(true, true, true, true).contains(&("MEDIUM_LIGHTING", 1.0)));
        // the text: nothing added for a scene with neither (the plain pipeline is not a variant), the refracted shadow rays only for
        // water, the hook only for foam
        assert_eq!(variant_source(&plain, false, false, false), plain);
        let foam_only = variant_source(&plain, false, true, false);
        assert!(foam_only.contains("pick < foam") && !foam_only.contains("fn light_through("));
        let water_only = variant_source(&plain, true, false, false);
        assert!(water_only.contains("fn light_through(") && !water_only.contains("pick < foam"));
        assert!(
            water_only.contains("override FOAM: bool = false;"),
            "the water shader declares the override the constant sets"
        );
        let both = variant_source(&plain, true, true, false);
        assert!(both.contains("fn light_through(") && both.contains("pick < foam"));
        assert!(
            !foam_only.contains("override FOAM"),
            "the foam-only shader has no override: a constant for it would be an error"
        );
    }

    #[test]
    fn the_foam_hook_changes_the_surface_at_the_hit_and_brings_in_no_refracted_shadow_ray() {
        let plain = format!("{}\n{}\n{}", include_str!("sampling.wgsl"), include_str!("pathtrace.wgsl"), VOLUME_WGSL);
        assert!(!plain.contains("foam_u"), "scenes without foam keep their shader text");
        for base in [plain.clone(), grid_source(), water_source(&plain)] {
            let foamy = foam_source(&base);
            assert!(foamy.contains("var s = surf_of(m);") && foamy.contains("if (pick < foam) {"));
            assert!(foamy.contains("foam_u = fract("), "the stratified number is set for every sample");
            assert!(foamy.contains("guide_albedo = mix(guide_albedo"), "the denoiser's guide is the mean albedo");
            // the light that gets through keeps the water's tint: the foam sample is the one that lets nothing through
            assert!(foamy.contains("s.trans = 0.0;") && !foamy.contains("s.albedo = mix("));
            assert_eq!(foamy.contains("fn light_through("), base.contains("fn light_through("));
        }
    }

    #[test]
    fn the_water_shader_adds_the_refracted_shadow_ray_and_the_plain_one_does_not_have_it() {
        let plain = format!("{}\n{}\n{}", include_str!("sampling.wgsl"), include_str!("pathtrace.wgsl"), VOLUME_WGSL);
        assert!(!plain.contains("light_through"), "scenes without transmissive materials keep their shader text");
        for base in [plain.clone(), grid_source()] {
            let water = water_source(&base);
            assert!(water.contains("fn light_through(") && water.contains("inside = entering;"));
            assert!(water.contains("thr *= eta * eta;") && water.contains("let probe = first_interface("));
            assert!(
                water.contains("fn water_transport(")
                    && water.contains("let seen = water_transport(o, d, hit.t, in_sigma);")
            );
            assert!(
                water.contains("if (!entering && dot(t, t) >= 1e-8) { cosi"),
                "the exit uses the cosine of the air side"
            );
            assert_eq!(
                water.matches("light_through(p, ng, n, l, ls.w, in_sigma, lt.size.y > 0.5 && m.extra.z > 0.5)").count(),
                1
            );
            assert!(water.contains("fn volume_transmittance("), "shared code is kept");
            assert!(
                water.contains("let c = dielectric_crossing(")
                    && water.contains("if (c.refracted) { inside = c.inside; }"),
                "the water shader takes whether the path is inside from a refraction only, not from a reflection"
            );
        }
    }

    /// The crossing of a dielectric surface is one function, called once from `radiance`, and its
    /// body is not repeated in the loop.
    #[test]
    fn the_crossing_of_a_dielectric_surface_is_a_function_radiance_calls() {
        let plain = include_str!("pathtrace.wgsl");
        assert_eq!(plain.matches("fn dielectric_crossing(").count(), 1);
        assert_eq!(plain.matches("dielectric_crossing(s, m, ").count(), 1, "called once");
        assert_eq!(plain.matches("refract(d, n, eta)").count(), 1, "the refraction is in the function only");
        let radiance = &plain[plain.find("fn radiance(").expect("radiance")..];
        assert!(!radiance.contains("refract("), "radiance does not refract itself");
    }

    #[test]
    fn the_grid_shader_reads_a_lit_domains_light_from_its_grids_and_leaves_the_rest() {
        let source = grid_source();
        let plain = VOLUME_WGSL;
        assert_eq!(source.matches("fn volume_incident_grid(").count(), 1, "one function reads a domain's light");
        assert_eq!(plain.matches("fn volume_incident_grid(").count(), 1, "the shader without grids has its own");
        let body = |text: &str, name: &str| {
            let start = text.find(name).expect("function");
            let end = start + text[start..].find("\n}\n").expect("end of function");
            text[start + name.len()..end].to_string()
        };
        assert_eq!(
            body(&source, "fn volume_incident_grid("),
            body(include_str!("volume_grid.wgsl"), "fn volume_incident_grid("),
            "the grid shader's is the one that reads the grids"
        );
        assert!(
            body(plain, "fn volume_incident_grid(").contains("return vec4(0.0);"),
            "without grids every lit domain is marched"
        );
        assert!(!source.contains("//@incident-grid"), "the markers are removed");
        // everything outside the replaced span is the shader without grids
        let (a, b) = (plain.find("//@incident-grid-begin\n").unwrap(), plain.find("//@incident-grid-end\n").unwrap());
        assert!(source.contains(&plain[..a]) && source.contains(&plain[b + "//@incident-grid-end\n".len()..]));
    }

    /// Brute-force nearest hit versus BVH traversal on the CPU.
    fn hit(t: &[Vec3; 3], o: Vec3, d: Vec3) -> Option<f32> {
        let (e1, e2) = (t[1] - t[0], t[2] - t[0]);
        let p = d.cross(e2);
        let det = e1.dot(p);
        if det.abs() < 1e-9 {
            return None;
        }
        let s = o - t[0];
        let u = s.dot(p) / det;
        let q = s.cross(e1);
        let v = d.dot(q) / det;
        let tt = e2.dot(q) / det;
        (u >= 0.0 && v >= 0.0 && u + v <= 1.0 && tt > 1e-4).then_some(tt)
    }

    fn traverse(nodes: &[PtNode], tris: &[[Vec3; 3]], o: Vec3, d: Vec3) -> Option<f32> {
        let mut best: Option<f32> = None;
        let mut stack = vec![0usize];
        while let Some(i) = stack.pop() {
            let nd = nodes[i];
            let (lo, hi) = (Vec3::from(nd.lo), Vec3::from(nd.hi));
            let inv = d.recip();
            let (t0, t1) = ((lo - o) * inv, (hi - o) * inv);
            let (tn, tf) = (t0.min(t1).max_element(), t0.max(t1).min_element());
            if tf < tn.max(0.0) || best.map(|b| tn > b).unwrap_or(false) {
                continue;
            }
            if nd.b > 0 {
                for t in &tris[nd.a as usize..(nd.a + nd.b) as usize] {
                    if let Some(h) = hit(t, o, d) {
                        best = Some(best.map_or(h, |b| b.min(h)));
                    }
                }
            } else {
                stack.push(nd.a as usize);
                stack.push(nd.a as usize - 1);
            }
        }
        best
    }

    #[test]
    fn bvh_traversal_finds_the_same_nearest_hits_as_brute_force() {
        let mut tris = Vec::new();
        let mut seed = 1u64;
        let mut rnd = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 33) as f32 / (1u64 << 31) as f32) * 2.0 - 1.0
        };
        for _ in 0..3000 {
            let c = Vec3::new(rnd(), rnd(), rnd()) * 100.0;
            tris.push([c, c + Vec3::new(rnd(), rnd(), rnd()) * 6.0, c + Vec3::new(rnd(), rnd(), rnd()) * 6.0]);
        }
        let mut nodes = Vec::new();
        let order = bvh(&tris, &mut nodes);
        let sorted: Vec<[Vec3; 3]> = order.iter().map(|&k| tris[k]).collect();
        let mut hits = 0;
        for _ in 0..500 {
            let o = Vec3::new(rnd(), rnd(), rnd()) * 150.0;
            let d = Vec3::new(rnd(), rnd(), rnd()).normalize_or(Vec3::X);
            let brute = tris
                .iter()
                .filter_map(|t| hit(t, o, d))
                .fold(None, |b: Option<f32>, h| Some(b.map_or(h, |x| x.min(h))));
            let fast = traverse(&nodes, &sorted, o, d);
            assert_eq!(brute.is_some(), fast.is_some());
            if let (Some(a), Some(b)) = (brute, fast) {
                assert!((a - b).abs() < 1e-3);
                hits += 1;
            }
        }
        assert!(hits > 20, "{hits} rays hit");
        // leaves cover every triangle once
        let covered: usize = nodes.iter().filter(|n| n.b > 0).map(|n| n.b as usize).sum();
        assert_eq!(covered, tris.len());
    }
}

// ---------------------------------------------------------------- GPU

use wgpu::util::DeviceExt;

/// Samples a trace dispatch takes.
const PER_DISPATCH: u32 = 4;
const DENOISE_PASSES: u32 = 5;
// A pass has radius 2 * step, with step = 1, 2, 4, 8, 16. The complete
// dependency footprint is 62 pixels on each side, not just the last pass's 32.
const DENOISE_HALO: u32 = 2 * ((1 << DENOISE_PASSES) - 1);
// Retain a 64×64 output core even at the smallest accepted budget. A one-pixel
// core would turn UHD into millions of largely redundant halo dispatches.
const MIN_TILE_BYTES: u64 = ((2 * DENOISE_HALO + 64) as u64).pow(2) * 32;
const DEFAULT_TILE_BYTES: u64 = 32 << 20;

#[derive(Clone, Copy, Debug)]
struct Tile {
    origin: [u32; 2],
    size: [u32; 2],
    output_origin: [u32; 2],
    output_size: [u32; 2],
}

/// Non-overlapping output rectangles with complete filter support around each one.
fn tiles(size: [u32; 2], bytes: u64, denoise: bool) -> Vec<Tile> {
    let pixels = bytes / 32;
    if u64::from(size[0]) * u64::from(size[1]) <= pixels {
        return vec![Tile { origin: [0; 2], size, output_origin: [0; 2], output_size: size }];
    }
    let halo = if denoise { DENOISE_HALO } else { 0 };
    let side = pixels.isqrt().min(u64::from(u32::MAX)) as u32;
    let stride = side.saturating_sub(2 * halo).max(1);
    let mut result = Vec::new();
    for y in (0..size[1]).step_by(stride as usize) {
        for x in (0..size[0]).step_by(stride as usize) {
            let origin = [x.saturating_sub(halo), y.saturating_sub(halo)];
            let end = [x.saturating_add(stride).min(size[0]), y.saturating_add(stride).min(size[1])];
            result.push(Tile {
                origin,
                size: std::array::from_fn(|i| end[i].saturating_add(halo).min(size[i]) - origin[i]),
                output_origin: [x, y],
                output_size: [end[0] - x, end[1] - y],
            });
        }
    }
    result
}
/// Spacing of per-dispatch parameter slots in the uniform buffer.
const SLOT: u64 = (std::mem::size_of::<Params>() as u64).div_ceil(256) * 256;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    cam_to_world: [[f32; 4]; 4],
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    env_rot: [[f32; 4]; 4],
    size: [f32; 4],
    cam: [f32; 4],
    env: [f32; 4],
    ambient: [f32; 4],
    misc: [f32; 4],
    out: [f32; 4],
    /// Global pixel origin (xy) and extent (zw) of the working tile.
    tile: [u32; 4],
    media: [u32; 4],
    /// Trace workgroup permutation stride (x), coprime to the workgroup count.
    perm: [u32; 4],
    /// SREP 71 procedural sky (sr_3d::sky::Sky::uniforms); sky[1][3] is 1 when the dome is one
    sky: [[f32; 4]; 5],
}

/// The stride of the trace workgroup permutation for `n` workgroups: the coprime number nearest n / phi, so that
/// consecutive workgroup indices land far apart in the image.
fn trace_stride(n: u32) -> u32 {
    fn gcd(a: u32, b: u32) -> u32 {
        if b == 0 {
            a
        } else {
            gcd(b, a % b)
        }
    }
    if n < 3 {
        return 1;
    }
    // The shader computes `lin * stride` in u32, so the stride stays below u32::MAX / n.
    let mut s = ((n as f64 * 0.618_033_988_75) as u32).clamp(1, n - 1).min(u32::MAX / n).max(1);
    while gcd(s, n) != 1 {
        s -= 1;
    }
    s
}

/// The permutation stride of the trace dispatch: a CPU adapter hands each thread a contiguous run of workgroups, so
/// it takes the spread order; a GPU keeps the grid order (stride 1), which its caches and scheduler favour.
fn trace_stride_for(cpu_adapter: bool, n: u32) -> u32 {
    if cpu_adapter {
        trace_stride(n)
    } else {
        1
    }
}

/// Compute pipelines of the shader variant with light grids.
#[derive(Debug)]
struct GridPipelines {
    layout: wgpu::PipelineLayout,
    light: wgpu::ComputePipeline,
    dome: wgpu::ComputePipeline,
}

/// The media's shader functions: the phase functions (`volume_phase.wgsl`) and the march (`volume.wgsl`).
const VOLUME_WGSL: &str = concat!(include_str!("volume_phase.wgsl"), "\n", include_str!("volume.wgsl"));

/// The shader with light grids: the function the media's walk asks for a lit domain's light from grids is the one
/// that reads them, everything else is the shader without grids.
fn grid_source() -> String {
    const BEGIN: &str = "//@incident-grid-begin\n";
    const END: &str = "//@incident-grid-end\n";
    let volume = VOLUME_WGSL;
    let (a, b) = (volume.find(BEGIN).expect("grid begin marker"), volume.find(END).expect("grid end marker"));
    // the function that reads no grids goes; volume_grid.wgsl brings the one that reads them
    let shared = format!("{}{}", &volume[..a], &volume[b + END.len()..]);
    format!(
        "{}\n{}\n{}\n{}",
        include_str!("sampling.wgsl"),
        include_str!("pathtrace.wgsl"),
        shared,
        include_str!("volume_grid.wgsl")
    )
}

/// The shader for scenes with a transmissive material: the shader it is given (with or without
/// light grids) plus the refracted shadow ray, the Fresnel term of a ray leaving the denser medium,
/// the scaling of radiance across an interface and the start of a camera under the water, which a
/// few small replacements hook in. Scenes
/// without such a material keep the text, and so the compiled code, they had.
fn water_source(base: &str) -> String {
    const SIGMA: &str = "    var in_sigma = vec3(0.0);\n";
    const ENTER: &str = "        if (WATER) { in_sigma = select(vec3(0.0), m.attenuation.rgb, entering); }\n";
    const CROSSED: &str = "            in_sigma = c.in_sigma;\n";
    const FRESNEL: &str = "    let cosi = clamp(dot(v, n), 0.0, 1.0);\n    let r0 = (1.0 - eta) / (1.0 + eta);\n    let fr = r0 * r0 + (1.0 - r0 * r0) * pow(1.0 - cosi, 5.0);\n    let t = refract(d, n, eta);\n";
    const FOG: &str = "        if (WATER && any(in_sigma > vec3(0.0))) { thr *= exp(-in_sigma * min(hit.t, 1e4)); }\n        if (HAS_MEDIA) {\n            let fog = volume_transport(o, d, hit.t);\n            col += thr * fog.rgb;\n            thr *= fog.a;\n            if (met == 0u) { alpha += (1.0 - alpha) * (1.0 - fog.a); }\n        }\n";
    const VISIBLE: &str = "            var visible = 1.0;\n            if (lt.size.y > 0.5 && m.extra.z > 0.5) { visible = visibility(p + ng * 1e-2, l, ls.w - 2e-2); }";
    for hook in [SIGMA, ENTER, CROSSED, FRESNEL, FOG, VISIBLE] {
        assert_eq!(base.matches(hook).count(), 1, "one place hooks in the refracted shadow ray: {hook}");
    }
    let hooked = base
        // Schlick's term takes the cosine of the side the light goes to when it leaves the denser medium
        .replace(
            FRESNEL,
            "    let t = refract(d, n, eta);\n\
             \x20   var cosi = clamp(dot(v, n), 0.0, 1.0);\n\
             \x20   if (!entering && dot(t, t) >= 1e-8) { cosi = clamp(-dot(normalize(t), n), 0.0, 1.0); }\n\
             \x20   let r0 = (1.0 - eta) / (1.0 + eta);\n\
             \x20   let fr = r0 * r0 + (1.0 - r0 * r0) * pow(1.0 - cosi, 5.0);\n",
        )
        // inside absorbing water the media's light is dimmed by the water in front of it, and the water
        // absorbs across the gaps; outside it the same march, at no absorption, is the plain one. The driver inlines
        // every call, so one march compiles the media's scattering (and the random walk) once.
        .replace(
            FOG,
            "        var marched = any(in_sigma > vec3(0.0));\n\
             \x20       if (HAS_MEDIA) { marched = true; }\n\
             \x20       if (marched) {\n\
             \x20           let seen = water_transport(o, d, hit.t, in_sigma);\n\
             \x20           col += thr * seen.color;\n\
             \x20           thr *= seen.trans;\n\
             \x20           if (met == 0u) { alpha += (1.0 - alpha) * (1.0 - seen.open); }\n\
             \x20       }\n",
        )
        .replace(
            SIGMA,
            // a camera under the water starts inside the medium: the first transmissive surface
            // straight up (the scene's up is -y) is met from the inside
            &format!(
                "{SIGMA}    var inside = false;\n\
                 \x20   {{\n\
                 \x20       let probe = first_interface(o, vec3(0.0, -1.0, 0.0), 1e30);\n\
                 \x20       if (probe.found && !probe.entering) {{ inside = true; in_sigma = probe.sigma; }}\n\
                 \x20   }}\n"
            ),
        )
        .replace(
            ENTER,
            // radiance over eta^2 is the same on both sides of an interface: a path that goes
            // from a medium of index n1 into one of index n2 carries (n1 / n2)^2
            "        if (WATER) {\n\
             \x20           in_sigma = select(vec3(0.0), m.attenuation.rgb, entering);\n\
             \x20           inside = entering;\n\
             \x20           thr *= eta * eta;\n\
             \x20       }\n",
        )
        // the path leaves the crossing inside the medium or not
        .replace(CROSSED, &format!("{CROSSED}            if (c.refracted) {{ inside = c.inside; }}\n"))
        .replace(
            VISIBLE,
            &format!(
                "            if (WATER && inside) {{\n\
                 \x20               // a surface under water (or glass): the light reaches it refracted\n\
                 \x20               let seen = light_through(p, ng, n, l, ls.w, in_sigma, lt.size.y > 0.5 && m.extra.z > 0.5);\n\
                 \x20               var through = seen.vis;\n\
                 \x20               if (HAS_MEDIA && m.extra.z > 0.5) {{\n\
                 \x20                   through *= volume_transmittance(p + ng * 1e-2, seen.dir, seen.inside) * volume_transmittance(seen.q, seen.la, seen.outside);\n\
                 \x20               }}\n\
                 \x20               var c = thr * bsdf(s, n, v, seen.dir, light_lobes(lt)) * max(dot(n, seen.dir), 0.0) * rad * through;\n\
                 \x20               if (bounce > 0u) {{ c = min(c, vec3(20.0)); }}\n\
                 \x20               col += c;\n\
                 \x20               continue;\n\
                 \x20           }}\n{VISIBLE}"
            ),
        );
    // the water's march is the only one this shader calls, so the media's own march is left out
    const MARCH: &str = "fn volume_transport(o: vec3<f32>, d: vec3<f32>, distance: f32) -> vec4<f32> {";
    let start = hooked.find(MARCH).expect("the media's march");
    let end = start + hooked[start..].find("\n}\n").expect("the march ends") + 3;
    format!("{}{}\n{}", &hooked[..start], &hooked[end..], include_str!("pathtrace_water.wgsl"))
}

/// The shader for scenes where some surface has foam mixed into its material: the shader it is given plus the foam at the hit. A
/// surface's share of foam, the alpha of its vertex colour, is the share of its area that the foam covers: a sample is on the foam
/// with that probability (and then the surface is the foam's, a white diffuse one that lets nothing through), on the water otherwise
/// (and then the light that gets through keeps the water's tint), so the picture is the mean of the two, weighted by the share. The
/// albedo guide of the denoiser takes the mean albedo. Scenes without such a surface keep the text, and so the compiled code, they had.
fn foam_source(base: &str) -> String {
    const TRACE: &str = "            rng = pcg(global_pix * 9781u + pcg(si * 6271u + 1u));\n";
    const GUIDE: &str = "            guide[pix * 2u] += vec4(m.base.rgb, 1.0);\n";
    const SURFACE: &str = "        let s = surf_of(m);\n";
    for hook in [TRACE, GUIDE, SURFACE] {
        assert_eq!(base.matches(hook).count(), 1, "one place hooks in the foam: {hook}");
    }
    let hooked = base
        // the number that picks the lobe at the first hit of a sample: a sequence with a random offset for each pixel (a hash of it, so
        // that neighbours are not correlated), so the samples of a pixel are spread over the share instead of being drawn
        .replace(
            TRACE,
            &format!(
                "{TRACE}            foam_u = fract(f32(pcg(global_pix * 2654435761u + 40503u)) * (1.0 / 4294967296.0) + f32(si) * 0.6180339887);\n"
            ),
        )
        .replace(
            GUIDE,
            "            var guide_albedo = m.base.rgb;\n\
             \x20           if (m.extra.w > 0.5) { guide_albedo = mix(guide_albedo, vec3(m.attenuation.w), clamp(color.a, 0.0, 1.0)); }\n\
             \x20           guide[pix * 2u] += vec4(guide_albedo, 1.0);\n",
        )
        .replace(
            SURFACE,
            "        var s = surf_of(m);\n\
             \x20       if (m.extra.w > 0.5) {\n\
             \x20           let foam = clamp(color.a, 0.0, 1.0);\n\
             \x20           if (foam > 0.0) {\n\
             \x20               // the first hit of a sample takes the stratified number, the later ones a random one\n\
             \x20               var pick = foam_u;\n\
             \x20               if (met != 1u || !first) { pick = rnd(); }\n\
             \x20               if (pick < foam) {\n\
             \x20                   let r0 = (s.ior - 1.0) / (s.ior + 1.0);\n\
             \x20                   s.albedo = vec3(m.attenuation.w);\n\
             \x20                   s.metallic = 0.0;\n\
             \x20                   s.a = max((m.extra.w - 1.0) * (m.extra.w - 1.0), 1e-3);\n\
             \x20                   s.trans = 0.0;\n\
             \x20                   s.f0 = vec3(r0 * r0);\n\
             \x20                   s.specw = clamp(m.extra.x, 0.0, 1.0);\n\
             \x20               }\n\
             \x20           }\n\
             \x20       }\n",
        );
    format!("var<private> foam_u: f32 = 0.0;\n{hooked}")
}

/// The variant of the trace shader the materials of a scene need beyond the plain one: `(water, foam)`, where `water` is a
/// transmissive material (the refracted shadow rays) and `foam` a foam mix (the hook at the hit). Foam alone does not make
/// `water`: the refracted shadow rays cost several times the plain shader.
fn variant_of<'a>(materials: impl Iterator<Item = &'a sr_3d::MaterialParams>) -> (bool, bool) {
    materials.fold((false, false), |(water, foam), m| (water || m.transmission > 0.0, foam || m.foam_mix.is_some()))
}

/// Whether a scene traces with the plain shader: no transmissive material, no foam mixed into a surface and no media.
/// The CPU tracer reproduces that shader only.
fn plain_trace<'a>(materials: impl Iterator<Item = &'a sr_3d::MaterialParams>, media: bool) -> bool {
    !media && variant_of(materials) == (false, false)
}

/// The slot of the variant pipeline for a scene: five by media and grid lighting, times water, water with foam, or foam alone.
fn variant_slot(media: bool, grid: bool, lighting: bool, water: bool, foam: bool, scatter: bool) -> usize {
    (match (media, grid) {
        (false, _) => 0,
        (true, false) => 1 + lighting as usize,
        (true, true) => 3 + lighting as usize,
    }) + 5 * if water {
        foam as usize
    } else if foam {
        2
    } else {
        3
    } + 20 * scatter as usize
}

/// The override constants of a variant pipeline: the media ones always, `WATER` only for a scene with a transmissive material and
/// `FOAM` only for one that also has foam (the water shader declares it; the foam-only shader has no such override, and a constant
/// for a name the module lacks is an error of the pipeline).
fn variant_constants(media: bool, lighting: bool, water: bool, foam: bool) -> Vec<(&'static str, f64)> {
    let mut constants = vec![("HAS_MEDIA", media as u8 as f64), ("MEDIUM_LIGHTING", lighting as u8 as f64)];
    if water {
        constants.extend([("WATER", 1.0), ("FOAM", foam as u8 as f64)]);
    }
    constants
}

/// The trace shader of a scene, and the override constants its pipeline is given: with volumes (`media`, `lighting` when
/// one needs albedo, `grid` when they are lit by light grids), transmissive materials (`water`), foam and multiple
/// scattering, specialized for those constants.
fn trace_shader(
    media: bool,
    lighting: bool,
    grid: bool,
    water: bool,
    foam: bool,
    scatter: bool,
) -> (String, Vec<(&'static str, f64)>) {
    let base = if media && grid {
        grid_source()
    } else {
        format!("{}\n{}\n{}", include_str!("sampling.wgsl"), include_str!("pathtrace.wgsl"), VOLUME_WGSL)
    };
    let (source, constants) = if water || foam || scatter {
        (variant_source(&base, water, foam, scatter), variant_constants(media, lighting, water, foam))
    } else if media {
        (base, vec![("HAS_MEDIA", 1.0), ("MEDIUM_LIGHTING", lighting as u8 as f64)])
    } else {
        (base, Vec::new())
    };
    (specialize(&source, &constants), constants)
}

/// `source` with the code its false boolean overrides guard taken out: `if (F) {…}`, `if (F && …) {…}` and
/// `else if (F) {…}` for every `override F: bool` that `constants`, or its default, leaves false. The pipeline is given
/// the same constants, so what it computes is unchanged; what changes is the text the driver compiles. NVIDIA's
/// OpenGL compiler inlines and optimizes what an `if (false)` holds before it drops it, so the volume calls under the
/// false overrides of the plain shader cost its compilation as much as the volumes' own shader.
///
/// Panics on a use of a false override it cannot take out exactly (a negation, an `||`, an `else` after the guard).
fn specialize(source: &str, constants: &[(&str, f64)]) -> String {
    let declaration = regex::Regex::new(r"override (\w+): bool = (true|false);").expect("pattern");
    let mut out = source.to_string();
    for flag in declaration.captures_iter(source) {
        let name = &flag[1];
        let on = constants.iter().find(|(k, _)| *k == name).map_or(&flag[2] == "true", |(_, v)| *v != 0.0);
        if on {
            continue;
        }
        let guard = regex::Regex::new(&format!(r"(\}} else )?if \({name}(\)| && )")).expect("pattern");
        while let Some(found) = guard.captures(&out) {
            let whole = found.get(0).expect("match");
            // an `else if` leaves the closing brace of the branch before it
            let mut start = whole.start() + usize::from(found.get(1).is_some());
            let open = whole.end() + out[whole.end()..].find('{').expect("the guarded block");
            let mut end = open + block_length(&out[open..]);
            assert!(
                !out[end..].trim_start().starts_with("else"),
                "an else after a false guard of {name}: specialize keeps no else branch"
            );
            // a guard on lines of its own goes with its indentation and line break
            let line = out[..start].rfind('\n').map_or(0, |i| i + 1);
            if out[line..start].trim().is_empty() && out[end..].starts_with('\n') {
                start = line;
                end += 1;
            }
            out.replace_range(start..end, "");
        }
        let uses = regex::Regex::new(&format!(r"\b{name}\b")).expect("pattern").find_iter(&out).count();
        assert!(uses == 1, "a use of {name} that specialize does not take out ({} besides the declaration)", uses - 1);
    }
    out
}

/// The length of the brace-delimited block `text` starts with.
fn block_length(text: &str) -> usize {
    let mut depth = 0usize;
    for (i, c) in text.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
    }
    panic!("an unclosed block");
}

/// The shader text of a variant: the base with the water additions when `water`, and the foam at the hit when `foam`.
fn variant_source_without_scatter(base: &str, water: bool, foam: bool) -> String {
    let source = if water { water_source(base) } else { base.to_string() };
    if foam {
        foam_source(&source)
    } else {
        source
    }
}

fn needs_scatter(mut bounces: impl Iterator<Item = u32>) -> bool {
    bounces.any(|b| b != 1)
}

/// A trace shader with the walk over the top-level BVH replaced by a ray query of the driver's acceleration structure
/// (`pathtrace_ray_query.wgsl`, bound at group 2), everything else as it was.
fn ray_query_source(source: &str) -> String {
    const HEAD: &str = "fn trace(o: vec3<f32>, d: vec3<f32>, tmax: f32) -> Hit {";
    let start = source.find(HEAD).expect("the trace shader walks the BVH in trace()");
    let end = start + source[start..].find("\n}\n").expect("trace() ends") + 3;
    format!(
        "enable wgpu_ray_query;\n{}{}{}",
        &source[..start],
        include_str!("pathtrace_ray_query.wgsl"),
        &source[end..]
    )
}

/// Keep the single-scattering shader unchanged; the random walk is compiled only when requested.
fn variant_source(base: &str, water: bool, foam: bool, scatter: bool) -> String {
    let source = variant_source_without_scatter(base, water, foam);
    if !scatter {
        return source;
    }
    // the one march (the water's in the shader with water, which marches the media outside the water too) is hooked
    // in once, and so is the media's own light
    let mut source = source;
    for (hook, replacement, sites) in [
        ("var color=vec3(0.0); var trans=1.0; var cursor=0.0;",
            "var color=vec3(0.0); var trans=1.0; var cursor=0.0; var reservoir: ScatterReservoir;", !water as usize),
        ("if (MEDIUM_LIGHTING && any(albedo.rgb>vec3(0.0)))",
            "if (MEDIUM_LIGHTING && tverts[base+13u].w > 0.0 && any(albedo.rgb>vec3(0.0)))", 1),
        ("color+=trans*source*weight; trans*=attenuation;",
            "scatter_offer(&reservoir, point, vec3(trans*weight)); color+=trans*source*weight; trans*=attenuation;", !water as usize),
        ("return vec4(color,trans);", "return vec4(color+scatter_walk(reservoir,-d,vec3(0.0)),trans);", !water as usize),
        ("var color = vec3(0.0); var trans = vec3(1.0); var open = 1.0; var cursor = 0.0;",
            "var color = vec3(0.0); var trans = vec3(1.0); var open = 1.0; var cursor = 0.0; var reservoir: ScatterReservoir;", water as usize),
        ("color += trans * source * weight; trans *= attenuation; open *= exp(-sigma * ds);",
            "scatter_offer(&reservoir, point, trans*weight); color += trans * source * weight; trans *= attenuation; open *= exp(-sigma * ds);", water as usize),
        ("return Transport(color, trans, open);",
            "return Transport(color+scatter_walk(reservoir,-d,sigma_water), trans, open);", water as usize),
    ] {
        assert_eq!(source.matches(hook).count(), sites, "the random walk hooks in at {hook}");
        source = source.replace(hook, replacement);
    }
    format!("{source}\n{}", include_str!("pathtrace_scatter.wgsl"))
}

/// How many of the CPU tracer's built scenes a tracer keeps, and their bytes in all.
const CPU_TOPS: usize = 4;
const CPU_TOP_BYTES: usize = 16 << 20;

/// The path tracer of an engine: the device's pipelines for its output format, and the engine's own ray-query choice,
/// buffer budget, packed geometry and scratch buffers.
pub struct PtGpu {
    pipelines: std::sync::Arc<crate::gpu::OfDevice<Pipelines>>,
    /// Whether passes traverse with ray queries where the device has them.
    rays: bool,
    tile_bytes: u64,
    packed: std::sync::Mutex<packed::Cache>,
    scratch: std::sync::Mutex<Option<Scratch>>,
    /// What the CPU tracer built from the scenes of recent passes, by geometry identity and atlas key, most recent
    /// last; at most [`CPU_TOPS`] of them and [`CPU_TOP_BYTES`] in all.
    cpu_tops: std::sync::Mutex<std::collections::VecDeque<KeptTop>>,
    /// Whether the device is a CPU adapter (the renderer sets it): the trace workgroups are then permuted.
    pub cpu_adapter: bool,
}

/// The shaders, layouts and pipelines of the path tracer for one device and output format (the variants built on first
/// use). They are immutable objects of the device, so the tracers on it share one set while any of them is alive
/// ([`PIPELINES`]): on a software adapter a set holds hundreds of megabytes.
#[derive(Debug)]
struct Pipelines {
    bgl0: wgpu::BindGroupLayout,
    bgl_pair: wgpu::BindGroupLayout,
    bgl_fin: wgpu::BindGroupLayout,
    trace: wgpu::ComputePipeline,
    trace_volume: [std::sync::OnceLock<wgpu::ComputePipeline>; 2],
    /// The same with the `WATER` constant for scenes with transmissive materials: no volumes, volumes
    /// (by whether their lighting needs albedo), and grid-lit volumes (the same).
    trace_water: [std::sync::OnceLock<wgpu::ComputePipeline>; 40],
    module: wgpu::ShaderModule,
    /// The variant of the shader with light grids, its group-1 layout and pipelines (built on first use).
    bgl_grid: wgpu::BindGroupLayout,
    grid: std::sync::OnceLock<GridPipelines>,
    trace_grid: [std::sync::OnceLock<wgpu::ComputePipeline>; 2],
    atrous: wgpu::ComputePipeline,
    /// The kernel that counts the samples an adaptive pass traced, with its group-1 layout (built on first use).
    bgl_count: wgpu::BindGroupLayout,
    count: std::sync::OnceLock<wgpu::ComputePipeline>,
    /// The group-2 layout of the acceleration structure, on a device with ray queries, and the trace pipelines that
    /// traverse it: the software ones' variants, by [`Pipelines::hardware_slot`] (built on first use).
    bgl_rays: Option<wgpu::BindGroupLayout>,
    hardware: [std::sync::OnceLock<wgpu::ComputePipeline>; 45],
    output: wgpu::RenderPipeline,
}

/// The pipelines of the tracers alive, by device and output format.
static PIPELINES: crate::gpu::PerDevice<wgpu::TextureFormat, Pipelines> = crate::gpu::PerDevice::new();

/// What the CPU tracer built from the scene of geometry `id` with the atlas `key`.
struct KeptTop {
    id: u64,
    key: Vec<AtlasKey>,
    top: std::sync::Arc<cpu::Top>,
}

/// The buffers every tile of a pass works in, kept from one pass to the next and grown when a tile needs more: the
/// sums and the guides of each pixel, the denoiser's two halves (made when a pass first denoises) and the stand-in
/// that passes reading the sums bind in their read-write slot. Passes are recorded and submitted in order, and every
/// tile clears the sums and the guides before tracing, so one set serves them all. The bind groups over the halves go
/// with the buffers: the denoiser's three pairs and the output's view of each buffer the last denoise pass can leave
/// the picture in.
#[derive(Clone)]
struct Scratch {
    /// Pixels each buffer holds.
    pixels: u64,
    accum: wgpu::Buffer,
    guide: wgpu::Buffer,
    stand_in: wgpu::Buffer,
    halves: Option<ScratchHalves>,
}
/// The bind groups keep the halves alive.
#[derive(Clone)]
struct ScratchHalves {
    /// sums → ping, ping → pong, pong → ping
    pairs: [wgpu::BindGroup; 3],
    /// the output's view of ping, then of pong
    fin: [wgpu::BindGroup; 2],
}
impl Scratch {
    fn buffer(d: &wgpu::Device, label: &str, len: u64) -> wgpu::Buffer {
        d.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: len,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }
    fn new(d: &wgpu::Device, pixels: u64) -> Self {
        Self {
            pixels,
            accum: Self::buffer(d, "pt-accum", pixels * 32),
            guide: Self::buffer(d, "pt-guides", pixels * 32),
            stand_in: Self::buffer(d, "pt-stand-in", 16),
            halves: None,
        }
    }
    /// Adds the denoiser's halves and their bind groups.
    fn with_halves(mut self, d: &wgpu::Device, pt: &PtGpu) -> Self {
        let ping = Self::buffer(d, "pt-denoise-a", self.pixels * 32);
        let pong = Self::buffer(d, "pt-denoise-b", self.pixels * 32);
        let pair = |src: &wgpu::Buffer, dst: &wgpu::Buffer| {
            d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("pathtrace-denoise"),
                layout: &pt.pipelines.bgl_pair,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: src.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: dst.as_entire_binding() },
                ],
            })
        };
        let pairs = [pair(&self.accum, &ping), pair(&ping, &pong), pair(&pong, &ping)];
        let fin = [pt.fin_group(d, &ping), pt.fin_group(d, &pong)];
        self.halves = Some(ScratchHalves { pairs, fin });
        self
    }
}

impl PtGpu {
    /// The scratch for tiles of `pixels` pixels, with the denoiser's halves when `denoise`, and how many buffers that
    /// took making: none when the kept scratch already served. A GPU keeps it: its device allocations cost
    /// milliseconds a pass. A CPU adapter's buffers are host memory the allocator recycles between passes, so it
    /// makes them anew: keeping them would only raise the peak.
    fn scratch(&self, d: &wgpu::Device, pixels: u64, denoise: bool) -> (Scratch, usize) {
        let mut kept = self.scratch.lock().unwrap_or_else(|e| e.into_inner());
        let mut made = 0;
        let mut scratch = match kept.take().filter(|_| !self.cpu_adapter) {
            Some(s) if s.pixels >= pixels => s,
            _ => {
                made += 3;
                Scratch::new(d, pixels)
            }
        };
        if denoise && scratch.halves.is_none() {
            made += 2;
            scratch = scratch.with_halves(d, self);
        }
        if !self.cpu_adapter {
            *kept = Some(scratch.clone());
        }
        (scratch, made)
    }
    /// What the CPU tracer builds from `data`, the scene of geometry `id` with the atlas `key`: the one kept from an
    /// earlier pass over the same, or else built now, counted in `built`, and kept.
    fn cpu_top(&self, id: u64, key: &[AtlasKey], data: &PtScene, built: &mut usize) -> std::sync::Arc<cpu::Top> {
        let mut tops = self.cpu_tops.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(i) = tops.iter().position(|kept| kept.id == id && kept.key == key) {
            let entry = tops.remove(i).expect("found");
            let top = entry.top.clone();
            tops.push_back(entry);
            return top;
        }
        *built += 1;
        let top = std::sync::Arc::new(cpu::Top::new(data));
        if top.bytes() <= CPU_TOP_BYTES {
            tops.push_back(KeptTop { id, key: key.to_vec(), top: top.clone() });
            while tops.len() > CPU_TOPS || tops.iter().map(|kept| kept.top.bytes()).sum::<usize>() > CPU_TOP_BYTES {
                tops.pop_front();
            }
        }
        top
    }
    /// The bind group the output pass reads the finished picture through.
    fn fin_group(&self, d: &wgpu::Device, fin: &wgpu::Buffer) -> wgpu::BindGroup {
        d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pathtrace-output"),
            layout: &self.pipelines.bgl_fin,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: fin.as_entire_binding() }],
        })
    }
    /// Keeps a `1 / workers` share of the default packed storage budget, for one of `workers` renderers in a process.
    pub fn share_budgets(&self, workers: usize) {
        let mut packed = self.packed.lock().unwrap_or_else(|e| e.into_inner());
        packed.set_budget(packed::BUDGET / workers.max(1) as u64);
    }

    /// Bytes of packed storage this tracer keeps at most.
    pub fn packed_budget(&self) -> u64 {
        self.packed.lock().unwrap_or_else(|e| e.into_inner()).budget()
    }
}

impl PtGpu {
    /// The path tracer of `format` on this device, with the pipelines of any other tracer on it alive.
    pub fn new(d: &std::sync::Arc<wgpu::Device>, format: wgpu::TextureFormat) -> PtGpu {
        PtGpu {
            pipelines: PIPELINES.get(d, format, || Pipelines::new(d, format)),
            rays: true,
            tile_bytes: DEFAULT_TILE_BYTES,
            packed: Default::default(),
            scratch: Default::default(),
            cpu_tops: Default::default(),
            cpu_adapter: false,
        }
    }

    /// Whether passes find their hits with ray queries where the device has them (the default), or always walk the
    /// tracer's own BVH.
    pub fn set_hardware_rays(&mut self, on: bool) {
        self.rays = on;
    }

    /// The group-2 layout of the acceleration structure when passes traverse with ray queries.
    fn rays_layout(&self) -> Option<&wgpu::BindGroupLayout> {
        self.pipelines.bgl_rays.as_ref().filter(|_| self.rays)
    }

    /// Sets the maximum size of each image storage buffer, additionally bounded by device
    /// limits. Useful for constraining memory and verifying that tiled output is identical.
    /// The budget must fit a 64×64 output core and its complete denoising halo
    /// (1,131,008 bytes), bounding the amount of repeated halo work.
    pub fn limit_buffers(&mut self, bytes: u64) -> Result<(), String> {
        if bytes < MIN_TILE_BYTES {
            return Err(format!("path tracing needs at least {MIN_TILE_BYTES} bytes per tile buffer"));
        }
        self.tile_bytes = bytes;
        // the kept scratch was sized for the old budget
        *self.scratch.lock().unwrap_or_else(|e| e.into_inner()) = None;
        Ok(())
    }
}

impl Pipelines {
    fn new(d: &wgpu::Device, format: wgpu::TextureFormat) -> Pipelines {
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pathtrace"),
            source: wgpu::ShaderSource::Wgsl(trace_shader(false, false, false, false, false, false).0.into()),
        });
        let vis = wgpu::ShaderStages::COMPUTE | wgpu::ShaderStages::FRAGMENT;
        let buf = |binding: u32, ty: wgpu::BufferBindingType, dynamic: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: vis,
            ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: dynamic, min_binding_size: None },
            count: None,
        };
        let ro = wgpu::BufferBindingType::Storage { read_only: true };
        let rw = wgpu::BufferBindingType::Storage { read_only: false };
        let tex = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: vis,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let bgl0 = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pathtrace"),
            entries: &[
                buf(0, wgpu::BufferBindingType::Uniform, true),
                buf(1, ro, false),
                buf(4, ro, false),
                buf(5, ro, false),
                buf(6, ro, false),
                buf(7, rw, false),
                buf(8, rw, false),
                tex(10),
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
                    visibility: vis,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                tex(12),
            ],
        });
        let bgl_pair = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pathtrace-denoise"),
            entries: &[buf(0, ro, false), buf(1, rw, false)],
        });
        let bgl_grid = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pathtrace-light-grid"),
            entries: &[buf(0, rw, false)],
        });
        let bgl_fin = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pathtrace-output"),
            entries: &[buf(0, ro, false)],
        });
        let bgl_count = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pathtrace-count"),
            entries: &[buf(0, rw, false)],
        });
        let bgl_rays = d.features().contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY).then(|| {
            d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("pathtrace-rays"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::AccelerationStructure { vertex_return: false },
                        count: None,
                    },
                    buf(1, ro, false),
                ],
            })
        });
        let layout = |groups: &[&wgpu::BindGroupLayout]| {
            let groups: Vec<Option<&wgpu::BindGroupLayout>> = groups.iter().map(|g| Some(*g)).collect();
            d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &groups,
                immediate_size: 0,
            })
        };
        let compute = |entry: &str, l: &wgpu::PipelineLayout| {
            let _creation = crate::gpu::creation_lock();
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(l),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let trace = compute("cs_trace", &layout(&[&bgl0]));
        let atrous = compute("cs_atrous", &layout(&[&bgl0, &bgl_pair]));
        let output = {
            let _creation = crate::gpu::creation_lock();
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("pathtrace-output"),
                layout: Some(&layout(&[&bgl0, &bgl_fin])),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_out"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_out"),
                    compilation_options: Default::default(),
                    targets: &[Some(format.into())],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        Pipelines {
            bgl0,
            bgl_pair,
            bgl_fin,
            trace,
            trace_volume: std::array::from_fn(|_| std::sync::OnceLock::new()),
            trace_water: std::array::from_fn(|_| std::sync::OnceLock::new()),
            module,
            bgl_grid,
            grid: std::sync::OnceLock::new(),
            trace_grid: std::array::from_fn(|_| std::sync::OnceLock::new()),
            atrous,
            bgl_count,
            count: std::sync::OnceLock::new(),
            bgl_rays,
            hardware: std::array::from_fn(|_| std::sync::OnceLock::new()),
            output,
        }
    }

    /// The slot of a scene's hardware trace pipeline: the variant slots, then the plain shader, then with volumes by
    /// grid lighting and medium lighting.
    fn hardware_slot(scene: &Scene3, grid: bool) -> usize {
        let (water, foam) = variant_of(scene.draws.iter().map(|dr| &dr.material));
        let scatter = needs_scatter(scene.volumes.iter().map(|v| v.medium().optical().scatter_bounces));
        let lighting = scene.volumes.iter().any(|v| v.medium().optical().albedo.iter().any(|v| *v > 0.0));
        let media = !scene.volumes.is_empty();
        if water || foam || scatter {
            variant_slot(media, grid, lighting, water, foam, scatter)
        } else if !media {
            40
        } else {
            41 + 2 * grid as usize + lighting as usize
        }
    }

    /// The trace pipeline of `scene` that traverses the acceleration structure in group 2: the shader and constants
    /// of [`Pipelines::trace_pipeline`]'s, with [`ray_query_source`]'s traversal.
    fn hardware_pipeline(
        &self,
        d: &wgpu::Device,
        scene: &Scene3,
        grid: bool,
        rays: &wgpu::BindGroupLayout,
    ) -> &wgpu::ComputePipeline {
        self.hardware[Self::hardware_slot(scene, grid)].get_or_init(|| {
            let (water, foam) = variant_of(scene.draws.iter().map(|dr| &dr.material));
            let scatter = needs_scatter(scene.volumes.iter().map(|v| v.medium().optical().scatter_bounces));
            let lighting = scene.volumes.iter().any(|v| v.medium().optical().albedo.iter().any(|v| *v > 0.0));
            let media = !scene.volumes.is_empty();
            let grid = media && grid;
            let (source, constants) = trace_shader(media, lighting, grid, water, foam, scatter);
            let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("pathtrace-rays"),
                source: wgpu::ShaderSource::Wgsl(ray_query_source(&source).into()),
            });
            let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("pathtrace-rays"),
                bind_group_layouts: &[Some(&self.bgl0), grid.then_some(&self.bgl_grid), Some(rays)],
                immediate_size: 0,
            });
            let _creation = crate::gpu::creation_lock();
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("pathtrace-rays"),
                layout: Some(&layout),
                module: &module,
                entry_point: Some("cs_trace"),
                compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() },
                cache: None,
            })
        })
    }

    /// The pipelines of the shader with light grids, compiled the first time a pass needs them.
    fn grid_pipelines(&self, d: &wgpu::Device) -> &GridPipelines {
        self.grid.get_or_init(|| {
            let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("pathtrace-light-grid"),
                source: wgpu::ShaderSource::Wgsl(specialize(&grid_source(), &[]).into()),
            });
            let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("pathtrace-light-grid"),
                bind_group_layouts: &[Some(&self.bgl0), Some(&self.bgl_grid)],
                immediate_size: 0,
            });
            let kernel = |entry: &str| {
                let _creation = crate::gpu::creation_lock();
                d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&layout),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
            };
            let light = kernel("cs_light_grid");
            let dome = kernel("cs_dome_grid");
            GridPipelines { layout, light, dome }
        })
    }

    fn trace_pipeline(&self, d: &wgpu::Device, scene: &Scene3, grid: bool) -> &wgpu::ComputePipeline {
        if plain_trace(scene.draws.iter().map(|dr| &dr.material), !scene.volumes.is_empty()) {
            return &self.trace;
        }
        let (water, foam) = variant_of(scene.draws.iter().map(|dr| &dr.material));
        let scatter = needs_scatter(scene.volumes.iter().map(|v| v.medium().optical().scatter_bounces));
        if water || foam || scatter {
            return self.variant_pipeline(d, scene, grid, water, foam, scatter);
        }
        let lighting = scene.volumes.iter().any(|v| v.medium().optical().albedo.iter().any(|v| *v > 0.0));
        if grid {
            return self.trace_grid[lighting as usize].get_or_init(|| {
                let g = self.grid_pipelines(d);
                let (source, constants) = trace_shader(true, lighting, true, false, false, false);
                let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("pathtrace-volumes-grid"),
                    source: wgpu::ShaderSource::Wgsl(source.into()),
                });
                let _creation = crate::gpu::creation_lock();
                d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("pathtrace-volumes-grid"),
                    layout: Some(&g.layout),
                    module: &module,
                    entry_point: Some("cs_trace"),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &constants,
                        ..Default::default()
                    },
                    cache: None,
                })
            });
        }
        self.trace_volume[lighting as usize].get_or_init(|| {
            let (source, constants) = trace_shader(true, lighting, false, false, false, false);
            let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("pathtrace-volumes"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("pathtrace-volumes"),
                bind_group_layouts: &[Some(&self.bgl0)],
                immediate_size: 0,
            });
            let _creation = crate::gpu::creation_lock();
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("pathtrace-volumes"),
                layout: Some(&layout),
                module: &module,
                entry_point: Some("cs_trace"),
                compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() },
                cache: None,
            })
        })
    }

    /// The trace pipeline of a scene with transmissive materials (`water`: the shader with `WATER` set), with foam mixed into
    /// some surface's material (`foam`), or both. Scenes with neither keep the pipeline, and the speed, they had; foam alone does
    /// not bring in the refracted shadow rays, which cost several times the plain shader.
    fn variant_pipeline(
        &self,
        d: &wgpu::Device,
        scene: &Scene3,
        grid: bool,
        water: bool,
        foam: bool,
        scatter: bool,
    ) -> &wgpu::ComputePipeline {
        let lighting = scene.volumes.iter().any(|v| v.medium().optical().albedo.iter().any(|v| *v > 0.0));
        let media = !scene.volumes.is_empty();
        let slot = variant_slot(media, grid, lighting, water, foam, scatter);
        self.trace_water[slot].get_or_init(|| {
            let (source, constants) = trace_shader(media, lighting, grid, water, foam, scatter);
            let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("pathtrace-variant"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            let plain_layout;
            let layout = if media && grid {
                &self.grid_pipelines(d).layout
            } else {
                plain_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("pathtrace-variant"),
                    bind_group_layouts: &[Some(&self.bgl0)],
                    immediate_size: 0,
                });
                &plain_layout
            };
            let _creation = crate::gpu::creation_lock();
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("pathtrace-variant"),
                layout: Some(layout),
                module: &module,
                entry_point: Some("cs_trace"),
                compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() },
                cache: None,
            })
        })
    }

    /// The kernel that adds up the samples each pixel of a tile traced.
    fn count_pipeline(&self, d: &wgpu::Device) -> &wgpu::ComputePipeline {
        self.count.get_or_init(|| {
            let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("pathtrace-count"),
                bind_group_layouts: &[Some(&self.bgl0), Some(&self.bgl_count)],
                immediate_size: 0,
            });
            let _creation = crate::gpu::creation_lock();
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("cs_count"),
                layout: Some(&layout),
                module: &self.module,
                entry_point: Some("cs_count"),
                compilation_options: Default::default(),
                cache: None,
            })
        })
    }
}

/// The one texel of the dome a pass without one samples.
const BLACK_DOME: [[f32; 4]; 1] = [[0.0; 4]];

/// Views and samplers the tracer reads that the 3D engine owns.
pub struct PtInputs<'a> {
    pub env: &'a wgpu::TextureView,
    pub backdrop: Option<&'a wgpu::TextureView>,
    pub black: &'a wgpu::TextureView,
    pub sampler: &'a wgpu::Sampler,
    /// Mip 0 of the dome as the shader samples it (f16-rounded), for the CPU tracer.
    pub env_texels: Option<&'a (u32, u32, Vec<[f32; 4]>)>,
}

/// The CPU tracer's view of a pass's uniform block.
fn radiance_params(base: &Params, lights: &[PtLight]) -> cpu::RadianceParams {
    cpu::RadianceParams {
        size: [base.size[0] as u32, base.size[1] as u32],
        inv_view_proj: base.inv_view_proj,
        cam_to_world: base.cam_to_world,
        cam: base.cam,
        env: base.env,
        lights: lights
            .iter()
            .map(|l| cpu::PLight { pos: l.pos, dir: l.dir, color: l.color, size: l.size, right: l.right })
            .collect(),
    }
}

/// Whether a pass runs on the CPU tracer: a software adapter, `SR_PT_CPU` not "0", a scene the plain shader traces
/// (`plain`: no media, water or foam), and none of the features the CPU loop leaves out (adaptive sampling, light
/// grids, counters, procedural sky). It takes the pass ahead of the adapter's ray queries, which are then not built.
/// A backdrop does not count: only paths that have met nothing but glass read it, and the CPU loop has no
/// transmission.
#[allow(clippy::too_many_arguments)]
fn cpu_trace_selected(
    cpu_adapter: bool,
    enabled: bool,
    supported: bool,
    plain: bool,
    base: &Params,
    grid: bool,
    counter: bool,
    env_available: bool,
) -> bool {
    cpu_adapter
        && enabled
        && supported
        && plain
        && base.out[3] <= 0.0
        && !grid
        && !counter
        && base.sky[1][3] <= 0.5
        && (base.env[0] <= 0.5 || env_available)
}

/// Traces `scene` into `out` (premultiplied, the rasteriser's output representation).
#[allow(clippy::too_many_arguments)]
pub fn render(
    pt: &PtGpu,
    d: &wgpu::Device,
    enc: &mut wgpu::CommandEncoder,
    scene: &Scene3,
    data: &PtScene,
    opts: PathOpts,
    inputs: &PtInputs,
    out: &wgpu::TextureView,
) {
    render_timed(pt, d, enc, scene, data, opts, inputs, out, false);
}

/// The buffers of a pass that the light-grid kernels read, besides the grid itself.
struct GridInputs<'a> {
    verts: &'a wgpu::Buffer,
    mats: &'a wgpu::Buffer,
    nodes: &'a wgpu::Buffer,
    lights: &'a wgpu::Buffer,
    guide: &'a wgpu::Buffer,
    /// A buffer for the accumulation binding, which the kernels do not use.
    stand_in: &'a wgpu::Buffer,
}

/// Records the kernels that fill the light grids of `plan` into `group`, once before the tiles:
/// one dispatch for each scalar slot (a light's or a dome direction's) and, for isotropic media
/// under a dome, one for the pre-integrated radiance. `first` carries the pass's parameters, in
/// which only the slot differs from one dispatch to the next.
#[allow(clippy::too_many_arguments)]
fn build_light_grids(
    pt: &PtGpu,
    d: &wgpu::Device,
    enc: &mut wgpu::CommandEncoder,
    timer: &mut Option<crate::fx::Timer>,
    plan: &crate::volume::LightGridPlan,
    group: &wgpu::BindGroup,
    first: Params,
    buffers: &GridInputs,
    inputs: &PtInputs,
) {
    let params: Vec<Params> = (0..plan.scalar_slots)
        .map(|slot| Params { size: [first.size[0], first.size[1], slot as f32, 0.0], ..first })
        .collect();
    let mut bytes = vec![0u8; params.len().max(1) * SLOT as usize];
    for (k, sl) in params.iter().enumerate() {
        let b = bytemuck::bytes_of(sl);
        bytes[k * SLOT as usize..k * SLOT as usize + b.len()].copy_from_slice(b);
    }
    let ubuf = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("pt-grid-params"),
        contents: &bytes,
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let entries = |acc: &wgpu::Buffer| {
        d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pathtrace-grid-build"),
            layout: &pt.pipelines.bgl0,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &ubuf,
                        offset: 0,
                        size: std::num::NonZeroU64::new(std::mem::size_of::<Params>() as u64),
                    }),
                },
                wgpu::BindGroupEntry { binding: 1, resource: buffers.verts.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: buffers.mats.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: buffers.nodes.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: buffers.lights.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: acc.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 8, resource: buffers.guide.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(inputs.env) },
                wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::Sampler(inputs.sampler) },
                wgpu::BindGroupEntry {
                    binding: 12,
                    resource: wgpu::BindingResource::TextureView(inputs.backdrop.unwrap_or(inputs.black)),
                },
            ],
        })
    };
    let build = entries(buffers.stand_in);
    let g = pt.pipelines.grid_pipelines(d);
    let stamp = timer.as_mut().and_then(|t| t.labelled_pair("pathtrace light grid"));
    let mut cp = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("pathtrace-light-grid"),
        timestamp_writes: stamp.zip(timer.as_ref()).map(|(a, t)| wgpu::ComputePassTimestampWrites {
            query_set: &t.set,
            beginning_of_pass_write_index: Some(a),
            end_of_pass_write_index: Some(a + 1),
        }),
    });
    cp.set_bind_group(1, group, &[]);
    cp.set_pipeline(&g.light);
    for slot in 0..plan.scalar_slots {
        cp.set_bind_group(0, &build, &[(u64::from(slot) * SLOT) as u32]);
        cp.dispatch_workgroups(plan.rows_per_slot.div_ceil(64), 1, 1);
    }
    if plan.radiance {
        cp.set_pipeline(&g.dome);
        cp.set_bind_group(0, &build, &[0]);
        cp.dispatch_workgroups(plan.node_count().div_ceil(64), 1, 1);
    }
}

/// [`render`], also reporting the CPU packing time and, with `time_gpu`, timestamp queries
/// around each tile's trace and denoise passes (labels `pathtrace trace` / `pathtrace denoise`).
#[allow(clippy::too_many_arguments)]
pub fn render_timed(
    pt: &PtGpu,
    d: &wgpu::Device,
    enc: &mut wgpu::CommandEncoder,
    scene: &Scene3,
    data: &PtScene,
    opts: PathOpts,
    inputs: &PtInputs,
    out: &wgpu::TextureView,
    time_gpu: bool,
) -> RenderTiming {
    render_cached(pt, d, enc, scene, data, opts, inputs, out, time_gpu, None)
}

/// Only engine-owned geometry tickets may select immutable packed storage.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_cached(
    pt: &PtGpu,
    d: &wgpu::Device,
    enc: &mut wgpu::CommandEncoder,
    scene: &Scene3,
    data: &PtScene,
    opts: PathOpts,
    inputs: &PtInputs,
    out: &wgpu::TextureView,
    time_gpu: bool,
    identity: Option<u64>,
) -> RenderTiming {
    let started = std::time::Instant::now();
    let size = [scene.size[0].max(1), scene.size[1].max(1)];
    let limits = d.limits();
    let budget = pt.tile_bytes.min(limits.max_storage_buffer_binding_size).min(limits.max_buffer_size);
    let tiles = tiles(size, budget, opts.denoise);
    let pixels = tiles.iter().map(|t| u64::from(t.size[0]) * u64::from(t.size[1])).max().unwrap_or(1);
    let mut timer = time_gpu.then(|| crate::fx::Timer::new(d, 2 * tiles.len() as u32 + 1));
    let storage = |label: &str, bytes: &[u8]| {
        d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytes,
            usage: wgpu::BufferUsages::STORAGE,
        })
    };
    let identity = identity.filter(|_| scene.volumes.is_empty());
    let packed_cache = || pt.packed.lock().unwrap_or_else(|e| e.into_inner());
    // the atlas is packed only when the cache holds no storage for its textures, and then still may match an
    // entry's pixels
    let key = identity.map(|_| data.atlas_key());
    let mut cached = identity.zip(key.as_deref()).and_then(|(id, key)| packed_cache().get(id, key));
    let atlas = if cached.is_none() { data.atlas_pixels() } else { Vec::new() };
    if cached.is_none() {
        cached = identity.zip(key.as_deref()).and_then(|(id, key)| packed_cache().get_packed(id, key, &atlas));
    }
    let atlas_texels = atlas.len();
    let geometry_uploads = usize::from(cached.is_none());
    let (geometry, media) = if let Some(buffers) = cached {
        let media = [(buffers.verts.size() / 16) as u32, 0, 0, 0];
        (buffers, media)
    } else {
        let (mut verts, offset) = packed::vertices(data, &atlas);
        let media = crate::volume::pack(&mut verts, &scene.volumes);
        let buffers = packed::Buffers {
            verts: storage("pt-verts", bytemuck::cast_slice(&verts)),
            nodes: storage("pt-nodes", bytemuck::cast_slice(&data.nodes)),
            offset,
        };
        if let Some((id, key)) = identity.zip(key.clone()) {
            packed_cache().insert(id, key, atlas, buffers.clone());
        }
        (buffers, media)
    };
    let packed::Buffers { verts: tverts, nodes, offset } = geometry;
    let mut materials = data.mats.clone();
    for material in &mut materials {
        for map in &mut material.maps {
            map[0] += offset;
        }
    }
    // Light grids: callers check `limit_note` first, which reports the grid sizing errors.
    let grid_plan = crate::volume::plan_light_grid(&scene.volumes, data.lights.len() as u32, scene.env.is_some())
        .unwrap_or_else(|e| panic!("{e}; check pathtrace::limit_note before rendering"));
    let grid_buffer = grid_plan.as_ref().map(|plan| {
        let buffer = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pt-light-grid"),
            size: plan.bytes,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: true,
        });
        buffer
            .slice(..)
            .get_mapped_range_mut()
            .expect("mapped at creation")
            .copy_from_slice(bytemuck::cast_slice(&crate::volume::light_grid_buffer(plan)));
        buffer.unmap();
        buffer
    });
    let grid_group = grid_buffer.as_ref().map(|buffer| {
        d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pathtrace-light-grid"),
            layout: &pt.pipelines.bgl_grid,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
        })
    });
    // built into this encoder ahead of the passes that trace it, so no pass reads a structure still being built
    let mats = storage("pt-mats", bytemuck::cast_slice(&materials));
    let mut lights = data.lights.clone();
    for light in &mut lights {
        if light.size[3] > 0.0 {
            light.size[2] = f32::from_bits(light.size[2].to_bits() + offset);
        }
    }
    let lights = storage("pt-lights", bytemuck::cast_slice(&lights));
    // the sums, then the sample statistics of each pixel, and the denoiser's halves (the colour, then its variance):
    // kept from pass to pass, and reused by every tile in command order, which bounds peak memory independently of
    // frame area. Clear sums and guides before tracing each tile.
    let (scratch, scratch_buffers) = pt.scratch(d, pixels, opts.denoise);
    let Scratch { accum, guide, stand_in, halves, .. } = &scratch;
    // parameter slots: trace dispatches, denoise passes, output
    let samples = opts.samples.max(1);
    let chunks = samples.div_ceil(PER_DISPATCH);
    const PASSES: u32 = DENOISE_PASSES;
    let ambient = scene.lights.iter().filter(|l| l.kind == LightKind::Ambient).fold(Vec3::ZERO, |a, l| a + l.color);
    let lens = scene.dof.map(|f| f.coc_scale / scene.cam.focal_px.max(1e-6) * 0.5).unwrap_or(0.0);
    let noise = opts.noise_target();
    // the counter of traced samples, which only a pass with a noise target needs
    let counter = noise.map(|_| {
        let buffer = |label: &str, usage: wgpu::BufferUsages| {
            d.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: 8, usage, mapped_at_creation: false })
        };
        let counter = buffer("pt-traced", wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC);
        let read = buffer("pt-traced-read", wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ);
        let group = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pathtrace-count"),
            layout: &pt.pipelines.bgl_count,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: counter.as_entire_binding() }],
        });
        (counter, read, group)
    });
    let budget = tiles.iter().map(|t| u64::from(t.size[0]) * u64::from(t.size[1])).sum::<u64>() * u64::from(samples);
    let pack_seconds = started.elapsed().as_secs_f64();
    let make_base = |tile: &Tile| Params {
        cam_to_world: scene.cam.view.inverse().to_cols_array_2d(),
        view_proj: (scene.clip_fix * scene.cam.view_proj()).to_cols_array_2d(),
        inv_view_proj: (scene.clip_fix * scene.cam.view_proj()).inverse().to_cols_array_2d(),
        // the dome's full orientation, as the rasteriser applies it
        env_rot: scene.env.as_ref().map(|e| e.rotation).unwrap_or(Mat4::IDENTITY).to_cols_array_2d(),
        size: [size[0] as f32, size[1] as f32, 0.0, 0.0],
        cam: [scene.lens_k1, lens, scene.dof.map(|f| f.focus).unwrap_or(1.0), opts.bounces as f32],
        env: [
            scene.env.as_ref().map(|e| if e.visible { 2.0 } else { 1.0 }).unwrap_or(0.0),
            scene.env.as_ref().map(|e| e.intensity).unwrap_or(0.0),
            scene.cam.orthographic as u32 as f32,
            (ambient.max_element() > 0.0) as u32 as f32,
        ],
        ambient: [ambient.x, ambient.y, ambient.z, data.lights.len() as f32],
        misc: [inputs.backdrop.is_some() as u32 as f32, samples as f32, 1.0, scene.exposure],
        out: [scene.encode_srgb as u32 as f32, 0.0, 0.0, noise.unwrap_or(0.0)],
        tile: [tile.origin[0], tile.origin[1], tile.size[0], tile.size[1]],
        media,
        perm: [trace_stride_for(pt.cpu_adapter, tile.size[0].div_ceil(8) * tile.size[1].div_ceil(8)), 0, 0, 0],
        sky: scene.env.as_ref().and_then(|e| e.sky).map(|s| s.uniforms()).unwrap_or([[0.0; 4]; 5]),
    };

    // the grids are built once, before the tiles
    if let (Some(plan), Some(group)) = (&grid_plan, &grid_group) {
        let buffers = GridInputs { verts: &tverts, mats: &mats, nodes: &nodes, lights: &lights, guide, stand_in };
        build_light_grids(pt, d, enc, &mut timer, plan, group, make_base(&tiles[0]), &buffers, inputs);
    }
    // whether the CPU tracer takes the pass, which no tile's own position changes; the GPU trace alone reads the
    // driver's acceleration structures, so they are built only for it
    let on_cpu = {
        let base = make_base(&tiles[0]);
        cpu_trace_selected(
            pt.cpu_adapter,
            std::env::var("SR_PT_CPU").map_or(true, |v| v != "0"),
            cpu::supported(data, &radiance_params(&base, &data.lights)),
            plain_trace(scene.draws.iter().map(|dr| &dr.material), !scene.volumes.is_empty()),
            &base,
            grid_plan.is_some(),
            counter.is_some(),
            inputs.env_texels.is_some(),
        )
    };
    let rays = if on_cpu {
        None
    } else {
        pt.rays_layout().and_then(|layout| Some((layout, rays::build(d, enc, data, layout)?)))
    };
    // Records, BVH, atlas and environment texels depend on the scene, not the tile: built once for every tile, and
    // what is built from the scene is kept for the passes over the same geometry and textures.
    let mut cpu_scenes = 0;
    let cpu = on_cpu.then(|| {
        let base = make_base(&tiles[0]);
        let env = match inputs.env_texels {
            Some((w, h, texels)) => cpu::EnvMap::borrowed(*w, *h, texels, base.env_rot, base.env[1]),
            None => cpu::EnvMap::borrowed(1, 1, &BLACK_DOME, base.env_rot, base.env[1]),
        };
        let top = match identity.zip(key.as_deref()) {
            Some((id, key)) => pt.cpu_top(id, key, data, &mut cpu_scenes),
            None => {
                cpu_scenes += 1;
                std::sync::Arc::new(cpu::Top::new(data))
            }
        };
        (cpu::Dense::with(data, top), env)
    });
    for (tile_index, tile) in tiles.iter().enumerate() {
        enc.clear_buffer(accum, 0, None);
        enc.clear_buffer(guide, 0, None);
        let base = make_base(tile);
        let rp = radiance_params(&base, &data.lights);
        if let Some((dense, cpu_env)) = &cpu {
            let (acc, gd) = cpu::trace_tile(
                dense,
                cpu_env,
                &rp,
                [tile.origin[0], tile.origin[1], tile.size[0], tile.size[1]],
                0,
                samples,
            );
            for (dst, src) in [(accum, &acc), (guide, &gd)] {
                let bytes: &[u8] = bytemuck::cast_slice(src);
                let staged = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("pt-cpu-trace"),
                    contents: bytes,
                    usage: wgpu::BufferUsages::COPY_SRC,
                });
                enc.copy_buffer_to_buffer(&staged, 0, dst, 0, bytes.len() as u64);
            }
        }
        let mut slots: Vec<Params> = Vec::new();
        for c in 0..chunks {
            let start = c * PER_DISPATCH;
            slots.push(Params {
                size: [base.size[0], base.size[1], start as f32, (samples - start).min(PER_DISPATCH) as f32],
                ..base
            });
        }
        for k in 0..PASSES {
            slots.push(Params {
                misc: [base.misc[0], base.misc[1], (1u32 << k) as f32, base.misc[3]],
                out: [base.out[0], 0.0, (k == 0) as u32 as f32, (k + 1 == PASSES) as u32 as f32],
                ..base
            });
        }
        slots.push(Params { out: [base.out[0], opts.denoise as u32 as f32, 0.0, 0.0], ..base });
        let mut ubytes = vec![0u8; slots.len() * SLOT as usize];
        for (k, sl) in slots.iter().enumerate() {
            let b = bytemuck::bytes_of(sl);
            ubytes[k * SLOT as usize..k * SLOT as usize + b.len()].copy_from_slice(b);
        }
        let ubuf = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pt-params"),
            contents: &ubytes,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let group0 = |acc: &wgpu::Buffer| {
            d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("pathtrace"),
                layout: &pt.pipelines.bgl0,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &ubuf,
                            offset: 0,
                            size: std::num::NonZeroU64::new(std::mem::size_of::<Params>() as u64),
                        }),
                    },
                    wgpu::BindGroupEntry { binding: 1, resource: tverts.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: mats.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 5, resource: nodes.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 6, resource: lights.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 7, resource: acc.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 8, resource: guide.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(inputs.env) },
                    wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::Sampler(inputs.sampler) },
                    wgpu::BindGroupEntry {
                        binding: 12,
                        resource: wgpu::BindingResource::TextureView(inputs.backdrop.unwrap_or(inputs.black)),
                    },
                ],
            })
        };
        let g_trace = group0(accum);
        // passes that read the accumulation bind a stand-in in its read-write slot
        let g_read = group0(stand_in);
        let groups = [tile.size[0].div_ceil(8), tile.size[1].div_ceil(8)];
        if !on_cpu {
            let stamp = timer.as_mut().and_then(|t| t.labelled_pair("pathtrace trace"));
            let mut cp = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("pathtrace"),
                timestamp_writes: stamp.zip(timer.as_ref()).map(|(a, t)| wgpu::ComputePassTimestampWrites {
                    query_set: &t.set,
                    beginning_of_pass_write_index: Some(a),
                    end_of_pass_write_index: Some(a + 1),
                }),
            });
            if let Some((layout, rays)) = &rays {
                cp.set_pipeline(pt.pipelines.hardware_pipeline(d, scene, grid_group.is_some(), layout));
                cp.set_bind_group(2, &rays.group, &[]);
            } else {
                cp.set_pipeline(pt.pipelines.trace_pipeline(d, scene, grid_group.is_some()));
            }
            if let Some(group) = &grid_group {
                cp.set_bind_group(1, group, &[]);
            }
            for c in 0..chunks {
                cp.set_bind_group(0, &g_trace, &[(c as u64 * SLOT) as u32]);
                cp.dispatch_workgroups(groups[0], groups[1], 1);
            }
            if let Some((_, _, group)) = &counter {
                cp.set_pipeline(pt.pipelines.count_pipeline(d));
                cp.set_bind_group(0, &g_trace, &[0]);
                cp.set_bind_group(1, group, &[]);
                cp.dispatch_workgroups(groups[0], groups[1], 1);
            }
        }
        let mut gfin = None;
        if let Some(halves) = halves.as_ref().filter(|_| opts.denoise) {
            let stamp = timer.as_mut().and_then(|t| t.labelled_pair("pathtrace denoise"));
            let mut cp = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("pathtrace-denoise"),
                timestamp_writes: stamp.zip(timer.as_ref()).map(|(a, t)| wgpu::ComputePassTimestampWrites {
                    query_set: &t.set,
                    beginning_of_pass_write_index: Some(a),
                    end_of_pass_write_index: Some(a + 1),
                }),
            });
            cp.set_pipeline(&pt.pipelines.atrous);
            for k in 0..PASSES {
                // sums → ping, then ping → pong and back
                let pair = if k == 0 { 0 } else { 1 + (k as usize + 1) % 2 };
                cp.set_bind_group(0, &g_read, &[((chunks + k) as u64 * SLOT) as u32]);
                cp.set_bind_group(1, &halves.pairs[pair], &[]);
                cp.dispatch_workgroups(groups[0], groups[1], 1);
                gfin = Some(&halves.fin[usize::from(pair == 1)]);
            }
        }
        let own_fin;
        let gfin = match gfin {
            Some(g) => g,
            None => {
                own_fin = pt.fin_group(d, accum);
                &own_fin
            }
        };
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("pathtrace-output"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: out,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: if tile_index == 0 {
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        rp.set_pipeline(&pt.pipelines.output);
        rp.set_scissor_rect(tile.output_origin[0], tile.output_origin[1], tile.output_size[0], tile.output_size[1]);
        rp.set_bind_group(0, &g_read, &[((chunks + PASSES) as u64 * SLOT) as u32]);
        rp.set_bind_group(1, gfin, &[]);
        rp.draw(0..3, 0..1);
    }
    if let Some(t) = &timer {
        t.resolve(enc);
    }
    let counter = counter.map(|(counter, read, _)| {
        enc.copy_buffer_to_buffer(&counter, 0, &read, 0, 8);
        read
    });
    RenderTiming {
        pack_seconds,
        gpu: timer,
        geometry_uploads,
        atlas_texels,
        cpu_scenes,
        scratch_buffers,
        samples: SampleCount { budget, counter, hardware_rays: rays.is_some() },
    }
}

#[cfg(test)]
mod wgsl_layout_tests {
    //! The Rust structs bound to WGSL structs have their layout (`crate::wgsl_layout`).
    use crate::wgsl_layout::{assert_agree, rust_fields};

    #[test]
    fn the_shader_tests_convergence_from_the_same_minimum() {
        let line = format!("const ADAPTIVE_MIN_SAMPLES: f32 = {}.0;", super::ADAPTIVE_MIN_SAMPLES);
        assert_eq!(include_str!("pathtrace.wgsl").matches(&line).count(), 1, "{line}");
    }

    #[test]
    fn pt_mat_matches_wgsl_mat() {
        let fields = rust_fields!(super::PtMat; base, params, emissive, extra, maps, texture_params, borders, attenuation, sss, sss_color);
        assert_agree(include_str!("pathtrace.wgsl"), "Mat", std::mem::size_of::<super::PtMat>(), &fields);
    }

    #[test]
    fn pt_node_matches_wgsl_node() {
        let fields = rust_fields!(super::PtNode; lo, a, hi, b);
        assert_agree(include_str!("pathtrace.wgsl"), "Node", std::mem::size_of::<super::PtNode>(), &fields);
    }

    #[test]
    fn pt_light_matches_wgsl_p_light() {
        let fields = rust_fields!(super::PtLight; pos, dir, color, spot, size, right);
        assert_agree(include_str!("pathtrace.wgsl"), "PLight", std::mem::size_of::<super::PtLight>(), &fields);
    }

    #[test]
    fn trace_stride_is_a_coprime_bijection_that_cannot_overflow() {
        let gcd = |mut a: u64, mut b: u64| {
            while b != 0 {
                (a, b) = (b, a % b);
            }
            a
        };
        for n in [1u32, 2, 3, 4, 5, 6, 7, 8, 12, 64, 97, 1024, 14400, 14401, 65536, 65537, 129_600, 518_400] {
            let s = super::trace_stride(n);
            assert!(s >= 1, "n={n}");
            assert_eq!(gcd(u64::from(s), u64::from(n)), 1, "n={n} stride={s}");
            assert!(u64::from(n - 1) * u64::from(s) <= u64::from(u32::MAX), "n={n} stride={s} overflows u32");
        }
        for n in [1u32, 2, 3, 6, 7, 64, 97, 1000, 1024, 4096] {
            let s = super::trace_stride(n);
            let mut seen = vec![false; n as usize];
            for lin in 0..n {
                let p = (lin * s % n) as usize;
                assert!(!seen[p], "n={n} stride={s} visits {p} twice");
                seen[p] = true;
            }
        }
    }

    #[test]
    fn only_a_cpu_adapter_permutes_the_trace_workgroups() {
        assert_eq!(super::trace_stride_for(false, 14400), 1);
        assert_eq!(super::trace_stride_for(true, 14400), super::trace_stride(14400));
        assert_ne!(super::trace_stride_for(true, 14400), 1);
    }

    fn zero_params() -> super::Params {
        bytemuck::Zeroable::zeroed()
    }

    #[test]
    fn params_map_field_by_field_to_radiance_params() {
        let mut p = zero_params();
        p.size = [1280.0, 720.0, 0.0, 0.0];
        p.cam = [0.1, 0.0, 3.0, 4.0];
        p.env = [2.0, 1.5, 0.0, 0.0];
        p.inv_view_proj[1][2] = 7.0;
        p.cam_to_world[3][0] = -2.0;
        let light = super::PtLight {
            pos: [0.0, 0.0, 0.0, 1.0],
            dir: [0.0, -1.0, 0.0, 0.0],
            color: [1.0, 2.0, 3.0, 1.0],
            spot: [9.0; 4],
            size: [0.5, 1.0, 0.0, 0.0],
            right: [1.0, 0.0, 0.0, 3.0],
        };
        let r = super::radiance_params(&p, &[light]);
        assert_eq!(r.size, [1280, 720]);
        assert_eq!(r.inv_view_proj, p.inv_view_proj);
        assert_eq!(r.cam_to_world, p.cam_to_world);
        assert_eq!(r.cam, p.cam);
        assert_eq!(r.env, p.env);
        assert_eq!(r.lights.len(), 1);
        let l = &r.lights[0];
        assert_eq!(
            (l.pos, l.dir, l.color, l.size, l.right),
            (light.pos, light.dir, light.color, light.size, light.right)
        );
    }

    #[test]
    fn cpu_trace_is_selected_only_when_nothing_disables_it() {
        let sel =
            |p: &super::Params, f: [bool; 7]| super::cpu_trace_selected(f[0], f[1], f[2], f[6], p, f[3], f[4], f[5]);
        let on = [true, true, true, false, false, true, true];
        let base = zero_params();
        assert!(sel(&base, on));
        // flag 6: the scene traces with the plain shader (no media, water or foam)
        for k in 0..7 {
            let mut f = on;
            f[k] = !f[k];
            // the env is only required when the pass shows or lights with a dome
            assert!(!sel(&base, f) || k == 5, "flag {k}");
        }
        let mut dome = zero_params();
        dome.env[0] = 1.0;
        assert!(sel(&dome, on));
        let mut off = on;
        off[5] = false;
        assert!(!sel(&dome, off));
        let mut p = zero_params();
        p.out[3] = 0.05;
        assert!(!sel(&p, on));
        let mut p = zero_params();
        p.sky[1][3] = 1.0;
        assert!(!sel(&p, on));
        // a backdrop is read only by paths that have met nothing but glass, and the CPU loop has no transmission
        let mut p = zero_params();
        p.misc[0] = 1.0;
        assert!(sel(&p, on));
    }

    #[test]
    fn params_matches_wgsl_params() {
        let fields = rust_fields!(super::Params; cam_to_world, view_proj, inv_view_proj, env_rot, size, cam, env, ambient, misc, out, tile, media, perm, sky);
        assert_agree(include_str!("pathtrace.wgsl"), "Params", std::mem::size_of::<super::Params>(), &fields);
    }
}
