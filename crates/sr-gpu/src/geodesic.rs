//! Rendering a Schwarzschild black hole with a thin accretion disk by tracing null geodesics.
//!
//! The units are G = c = 1: lengths are in the scene's units, the hole's radius is 2 `mass`, the
//! innermost stable circular orbit 6 `mass`, and the geometric time is a length. See
//! `geodesic.wgsl` for the method. The pass draws the whole frame (a hole, its disk and a star
//! field behind them), in tiles when asked, and nothing else of a 3D scene.

use glam::Vec3;
use rayon::prelude::*;
use wgpu::util::DeviceExt;

/// How the brightness of the disk varies with radius and azimuth, apart from its temperature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    /// A smooth disk.
    None,
    /// Seeded clumps carried round by the disk's Keplerian rotation.
    Clumps,
    /// Seeded logarithmic spiral arms carried round by the same rotation.
    Spiral,
}

/// A thin, opaque, Keplerian disk in the plane through the hole's centre perpendicular to `axis`.
#[derive(Clone, Copy, Debug)]
pub struct Disk {
    /// Inner radius; 6 `mass` is the innermost stable orbit.
    pub inner: f32,
    pub outer: f32,
    /// Highest local temperature of the disk (kelvin), reached at 49/36 of the inner radius.
    pub peak_kelvin: f32,
    /// Scale of the emitted radiance.
    pub intensity: f32,
    /// Strength of the pattern, 0 to 1.
    pub contrast: f32,
    pub pattern: Pattern,
    pub seed: u32,
    /// Direction of the spin: the disk turns counter-clockwise seen from where this points.
    pub axis: Vec3,
    /// Geometric time of the frame: the pattern turns by `sqrt(mass / r^3)` radians per unit of it.
    pub time: f32,
}

/// The frame to draw.
#[derive(Clone, Debug)]
pub struct GeodesicScene {
    pub size: [u32; 2],
    /// The observer, at rest at a distance of more than 3 `mass` from the hole.
    pub eye: Vec3,
    pub right: Vec3,
    /// Toward the bottom of the image.
    pub down: Vec3,
    pub forward: Vec3,
    pub focal_px: f32,
    pub hole: Vec3,
    pub mass: f32,
    pub disk: Option<Disk>,
    /// Rays per pixel, spread over it.
    pub samples: u32,
    pub star_seed: u32,
    /// Factor on the radiance.
    pub exposure: f32,
    /// Encode the picture with the sRGB curve (working spaces that blend on encoded values).
    pub encode_srgb: bool,
}

/// What a pass writes: the picture, or per-pixel numbers for checking the tracing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    /// The picture, each sample read from the frame's [`OrbitTable`].
    Picture,
    /// The picture with every sample's geodesic integrated on its own: the reference the table is checked against.
    TracedPicture,
    /// (outcome 0 captured / 1 escaped, escape angle, radius of crossing 0, of crossing 1), one ray
    /// through each pixel's centre, crossings of the disk's plane at any radius.
    Crossings,
    /// (radius of crossing 2, of crossing 3, crossings counted, angle of the first crossing).
    MoreCrossings,
    /// (g, radius, azimuth, order) of the first crossing inside the disk, or (0, -1, 0, -1).
    FirstHit,
}

const BLACKBODY_ENTRIES: usize = sr_volume::thermal::TABLE_INTERVALS + 1;

/// Angles at which a row of the orbit table holds its orbit, evenly over [0, [`ORBIT_SPAN`]].
pub const ORBIT_COLUMNS: usize = 256;
/// The angles a row covers: the disk's plane is crossed at phi0 + k pi with phi0 in (0, pi] and k < 4.
pub const ORBIT_SPAN: f64 = 4.0 * std::f64::consts::PI;
/// Impact parameters the table of a frame holds for each direction.
pub const ORBIT_ROWS: usize = 2048;
/// RK4 steps per column of a row.
const ORBIT_SUBSTEPS: usize = 4;
/// The angle a traced ray gives up at (`STEP` x `MAX_STEPS` of `geodesic.wgsl`): it counts as captured.
const TRACE_LIMIT: f64 = 0.02 * 4096.0;
/// Width of the rows' crowding round the critical impact parameter, as a fraction of it (see [`OrbitTable`]).
const ORBIT_WARP: f64 = 0.01;

/// Every orbit a frame's rays follow. In the Schwarzschild metric a ray from an observer at `r_o` follows
/// u(phi) = 1 / r fixed by its impact parameter b and by whether it starts toward the hole, so one row per
/// (b, direction) serves every sample of the frame; the rows of rays that start away from the hole are filled
/// only when the frame has such rays.
///
/// Near the critical impact parameter b_c = 3 sqrt(3) M an orbit winds round the photon sphere a number of
/// times that grows like -ln(b - b_c), so its crossings change faster with b the closer b is to b_c. The rows
/// are therefore even in x = asinh((b - b_c) / w), w = [`ORBIT_WARP`] b_c, from the smallest b to the largest
/// the frame needs: crowded where the orbits change fast, sparse where they change slowly.
///
/// The texels are laid out as the texture the pass reads: one line per row, the ingoing orbit's
/// [`ORBIT_COLUMNS`] (u, du/dphi) and its (end angle, 1 escaped or 0 captured), then the outgoing orbit's.
/// The end angle of an escaping orbit is where u reaches 0, the direction it leaves in.
pub struct OrbitTable {
    pub rows: usize,
    pub mass: f64,
    /// x of the first and of the last row.
    pub x_range: [f64; 2],
    /// w of the rows' spacing.
    pub width: f64,
    pub data: Vec<[f32; 2]>,
}

impl OrbitTable {
    const LINE: usize = 2 * (ORBIT_COLUMNS + 1);

    fn index(&self, row: usize, ingoing: bool, column: usize) -> usize {
        row * Self::LINE + usize::from(!ingoing) * (ORBIT_COLUMNS + 1) + column
    }

    /// The impact parameter of row `row`.
    pub fn impact(&self, row: usize) -> f64 {
        let [lo, hi] = self.x_range;
        let x = lo + (hi - lo) * row as f64 / (self.rows - 1) as f64;
        (27.0f64.sqrt() * self.mass + self.width * x.sinh()).max(1e-5)
    }

    /// (u, du/dphi) of a row's orbit at column `column`.
    pub fn at(&self, row: usize, ingoing: bool, column: usize) -> [f32; 2] {
        self.data[self.index(row, ingoing, column)]
    }

    /// The angle a row's orbit ends at and whether it escapes there (else it falls in).
    pub fn end(&self, row: usize, ingoing: bool) -> (f32, bool) {
        let [phi, escaped] = self.data[self.index(row, ingoing, ORBIT_COLUMNS)];
        (phi, escaped > 0.5)
    }
}

/// The orbits of `rows` impact parameters (`rows` at least 2) for an observer at `r_o` from a hole of `mass`,
/// up to the b of a ray at sin(alpha) = `s_max` from the direction to the hole, with the outgoing half filled
/// when `outgoing`. Each row is integrated in f64 by RK4 at [`ORBIT_SUBSTEPS`] steps per column.
pub fn orbit_table(mass: f64, r_o: f64, s_max: f64, rows: usize, outgoing: bool) -> OrbitTable {
    let rows = rows.max(2);
    let critical = 27.0f64.sqrt() * mass;
    let width = ORBIT_WARP * critical;
    let widest = (r_o * s_max / (1.0 - 2.0 * mass / r_o).sqrt()).max(2e-5);
    let x_range = [((1e-5 - critical) / width).asinh(), ((widest - critical) / width).asinh()];
    let mut table = OrbitTable { rows, mass, x_range, width, data: vec![[0.0f32; 2]; rows * OrbitTable::LINE] };
    let impacts: Vec<f64> = (0..rows).map(|row| table.impact(row)).collect();
    table.data.par_chunks_mut(OrbitTable::LINE).zip(impacts).for_each(|(line, b)| {
        let (ingoing, rest) = line.split_at_mut(ORBIT_COLUMNS + 1);
        orbit_row(mass, r_o, b, true, ingoing);
        if outgoing {
            orbit_row(mass, r_o, b, false, rest);
        }
    });
    table
}

/// One orbit into `out`: (u, du/dphi) at each column while the ray is outside the horizon and u > 0, and
/// at the first column past its end (so that the last interval before the end interpolates), then the end.
fn orbit_row(m: f64, r_o: f64, b: f64, ingoing: bool, out: &mut [[f32; 2]]) {
    let accel = |u: f64| -u + 3.0 * m * u * u;
    let rk4 = |u: f64, w: f64, h: f64| {
        let (k1u, k1w) = (w, accel(u));
        let (k2u, k2w) = (w + 0.5 * h * k1w, accel(u + 0.5 * h * k1u));
        let (k3u, k3w) = (w + 0.5 * h * k2w, accel(u + 0.5 * h * k2u));
        let (k4u, k4w) = (w + h * k3w, accel(u + h * k3u));
        (u + h / 6.0 * (k1u + 2.0 * k2u + 2.0 * k3u + k4u), w + h / 6.0 * (k1w + 2.0 * k2w + 2.0 * k3w + k4w))
    };
    let horizon = 0.5 / m;
    let u0 = 1.0 / r_o;
    let mut u = u0;
    let mut w = (1.0 / (b * b) - u0 * u0 + 2.0 * m * u0 * u0 * u0).max(0.0).sqrt();
    if !ingoing {
        w = -w;
    }
    // past the horizon the orbit soon diverges; beyond 1 / m (r = M, inside every disk) only "no crossing" matters
    let store = |u: f64, w: f64| {
        if u.is_finite() && w.is_finite() && u <= 1.0 / m {
            [u as f32, w as f32]
        } else {
            [(1.0 / m) as f32, 0.0]
        }
    };
    let (mut phi, mut end) = (0.0f64, None::<(f64, bool)>);
    let h = ORBIT_SPAN / ((ORBIT_COLUMNS - 1) * ORBIT_SUBSTEPS) as f64;
    out[0] = store(u, w);
    let mut column = 1;
    while column < ORBIT_COLUMNS {
        for _ in 0..ORBIT_SUBSTEPS {
            let (un, wn) = rk4(u, w, h);
            if end.is_none() {
                if un >= horizon {
                    end = Some((phi + h * (horizon - u) / (un - u), false));
                } else if un <= 0.0 {
                    end = Some((phi + h * u / (u - un), true));
                }
            }
            (u, w, phi) = (un, wn, phi + h);
        }
        out[column] = store(u, w);
        column += 1;
        if end.is_some() {
            break;
        }
    }
    let last = out[column - 1];
    out[column..ORBIT_COLUMNS].fill(last);
    let (phi_end, escaped) = end.unwrap_or_else(|| {
        // still bound after the span: follow it at the traced step to where it escapes, falls in or gives up
        let h = 0.02;
        while phi < TRACE_LIMIT {
            let (un, wn) = rk4(u, w, h);
            if un >= horizon {
                return (phi + h * (horizon - u) / (un - u), false);
            }
            if un <= 0.0 {
                return (phi + h * u / (u - un), true);
            }
            (u, w, phi) = (un, wn, phi + h);
        }
        (TRACE_LIMIT, false)
    });
    out[ORBIT_COLUMNS] = [phi_end as f32, if escaped { 1.0 } else { 0.0 }];
}

/// The rows a frame needs: the largest sin(alpha) of its rays, alpha the angle between a ray and the direction
/// to the hole, and whether some ray starts away from the hole (alpha above 90 degrees). The angle to a point
/// has no maximum inside the image other than at the antipode, which is beyond 90 degrees, so the image's
/// border bounds it; the bound carries one pixel of margin for the spacing of the border's samples.
fn frame_extent(scene: &GeodesicScene) -> (f64, bool) {
    let to_hole = (scene.hole - scene.eye).normalize().as_dvec3();
    let (fwd, right, down) =
        (scene.forward.normalize().as_dvec3(), scene.right.normalize().as_dvec3(), scene.down.normalize().as_dvec3());
    let focal = f64::from(scene.focal_px);
    let (w, h) = (f64::from(scene.size[0]), f64::from(scene.size[1]));
    let angle = |x: f64, y: f64| {
        let d = (fwd * focal + right * (x - w * 0.5) + down * (y - h * 0.5)).normalize();
        d.dot(to_hole).clamp(-1.0, 1.0).acos()
    };
    let mut widest = 0.0f64;
    for x in 0..=scene.size[0] {
        widest = widest.max(angle(f64::from(x), 0.0)).max(angle(f64::from(x), h));
    }
    for y in 0..=scene.size[1] {
        widest = widest.max(angle(0.0, f64::from(y))).max(angle(w, f64::from(y)));
    }
    let widest = widest + 1.0 / focal.max(1e-6);
    let away = widest >= std::f64::consts::FRAC_PI_2;
    (if away { 1.0 } else { widest.sin() }, away)
}

/// The pipeline of the pass.
pub struct GeodesicGpu {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    blackbody: wgpu::Buffer,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    eye: [f32; 4],
    hole: [f32; 4],
    right: [f32; 4],
    down: [f32; 4],
    fwd: [f32; 4],
    dx: [f32; 4],
    dy: [f32; 4],
    dz: [f32; 4],
    size: [f32; 4],
    misc: [u32; 4],
    extra: [f32; 4],
    table: [f32; 4],
}

fn v4(v: Vec3, w: f32) -> [f32; 4] {
    [v.x, v.y, v.z, w]
}

/// Two unit vectors that with `axis` (made a unit vector) form a right-handed frame.
fn plane_axes(axis: Vec3) -> (Vec3, Vec3, Vec3) {
    let z = axis.normalize_or(Vec3::NEG_Y);
    let helper = if z.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let x = (helper - z * helper.dot(z)).normalize();
    (x, z.cross(x), z)
}

/// The most rows a pass draws at once; a taller frame is drawn in bands.
const BAND_ROWS: u32 = 128;

impl GeodesicGpu {
    pub fn new(d: &wgpu::Device, format: wgpu::TextureFormat) -> GeodesicGpu {
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("geodesic"),
            source: wgpu::ShaderSource::Wgsl(include_str!("geodesic.wgsl").into()),
        });
        let uniform = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let orbits = wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("geodesic"),
            entries: &[uniform(0), uniform(1), orbits],
        });
        let pipeline_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("geodesic"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("geodesic"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let table: Vec<[f32; 4]> = sr_volume::thermal::blackbody_table()
            .iter()
            .map(|rgb| [rgb[0] as f32, rgb[1] as f32, rgb[2] as f32, 0.0])
            .collect();
        debug_assert_eq!(table.len(), BLACKBODY_ENTRIES);
        let blackbody = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("geodesic-blackbody"),
            contents: bytemuck::cast_slice(&table),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        GeodesicGpu { pipeline, layout, blackbody }
    }

    /// Draws `scene` into `out`, a texture of the scene's size, in bands of at most `rows` rows
    /// (all of them when `None`): every pixel is computed from its own coordinates alone, so the
    /// bands change nothing but the time one submission takes.
    pub fn render(
        &self,
        d: &wgpu::Device,
        enc: &mut wgpu::CommandEncoder,
        scene: &GeodesicScene,
        output: Output,
        rows: Option<u32>,
        out: &wgpu::TextureView,
    ) {
        let (pattern, disk) = match &scene.disk {
            Some(disk) => (
                match disk.pattern {
                    Pattern::None => 0,
                    Pattern::Clumps => 1,
                    Pattern::Spiral => 2,
                },
                *disk,
            ),
            None => (
                3,
                Disk {
                    inner: 6.0 * scene.mass,
                    outer: 7.0 * scene.mass,
                    peak_kelvin: 0.0,
                    intensity: 0.0,
                    contrast: 0.0,
                    pattern: Pattern::None,
                    seed: 0,
                    axis: Vec3::NEG_Y,
                    time: 0.0,
                },
            ),
        };
        let (x, y, z) = plane_axes(disk.axis);
        let mode = match output {
            Output::Picture => 0.0,
            Output::Crossings => 1.0,
            Output::MoreCrossings => 2.0,
            Output::FirstHit => 3.0,
            Output::TracedPicture => 4.0,
        };
        // the rays of every crossing are wanted for checks; the picture stops at the first in the disk
        let flags = u32::from(matches!(output, Output::Crossings | Output::MoreCrossings | Output::FirstHit));
        let (s_max, away) = frame_extent(scene);
        let r_o = f64::from((scene.eye - scene.hole).length());
        let table = orbit_table(f64::from(scene.mass), r_o, s_max, ORBIT_ROWS, away);
        let orbits = upload_orbits(d, enc, &table);
        let params = Params {
            eye: v4(scene.eye, scene.mass),
            hole: v4(scene.hole, disk.inner),
            right: v4(scene.right.normalize(), scene.focal_px),
            down: v4(scene.down.normalize(), scene.samples.max(1) as f32),
            fwd: v4(scene.forward.normalize(), mode),
            dx: v4(x, disk.outer),
            dy: v4(y, disk.peak_kelvin),
            dz: v4(z, disk.intensity),
            size: [scene.size[0] as f32, scene.size[1] as f32, disk.contrast, disk.time],
            misc: [pattern, disk.seed, flags, scene.star_seed],
            extra: [scene.exposure, f32::from(u8::from(scene.encode_srgb)), 0.0, 0.0],
            table: [table.x_range[0] as f32, table.x_range[1] as f32, table.width as f32, table.rows as f32],
        };
        let uniforms = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("geodesic-params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let group = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("geodesic"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: self.blackbody.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&orbits) },
            ],
        });
        let band = rows.unwrap_or(BAND_ROWS.max(scene.size[1])).max(1);
        let mut first = 0;
        let mut y0 = 0;
        while y0 < scene.size[1] {
            let h = band.min(scene.size[1] - y0);
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("geodesic"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: out,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if first == 0 { wgpu::LoadOp::Clear(wgpu::Color::BLACK) } else { wgpu::LoadOp::Load },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.set_scissor_rect(0, y0, scene.size[0], h);
            pass.draw(0..3, 0..1);
            drop(pass);
            first += 1;
            y0 += h;
        }
    }
}

/// The table as the texture the pass reads, copied in by `enc` ahead of the pass.
fn upload_orbits(d: &wgpu::Device, enc: &mut wgpu::CommandEncoder, table: &OrbitTable) -> wgpu::TextureView {
    let (width, height) = (OrbitTable::LINE as u32, table.rows as u32);
    let line = OrbitTable::LINE * std::mem::size_of::<[f32; 2]>();
    let stride = line.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize);
    let mut bytes = vec![0u8; stride * table.rows];
    for (dst, src) in bytes.chunks_exact_mut(stride).zip(table.data.as_chunks::<{ OrbitTable::LINE }>().0) {
        dst[..line].copy_from_slice(bytemuck::cast_slice(src));
    }
    let staging = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("geodesic-orbits-staging"),
        contents: &bytes,
        usage: wgpu::BufferUsages::COPY_SRC,
    });
    let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
    let texture = d.create_texture(&wgpu::TextureDescriptor {
        label: Some("geodesic-orbits"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rg32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    enc.copy_buffer_to_texture(
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride as u32),
                rows_per_image: Some(height),
            },
        },
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        size,
    );
    texture.create_view(&Default::default())
}

#[cfg(test)]
mod tests {
    /// The thin-disk profile f(x) = x^(-3/4) (1 - x^(-1/2))^(1/4), x = r / r_in.
    fn profile(x: f64) -> f64 {
        if x <= 1.0 {
            0.0
        } else {
            x.powf(-0.75) * (1.0 - x.powf(-0.5)).powf(0.25)
        }
    }

    use super::{orbit_table, ORBIT_COLUMNS, ORBIT_SPAN};

    /// u = 1 / r along the orbit of impact parameter `b` from `r_o`, by RK4 at a step far finer than
    /// the table's, at `phi` (before the orbit ends).
    fn fine_u(m: f64, r_o: f64, b: f64, ingoing: bool, phi: f64) -> f64 {
        let u0 = 1.0 / r_o;
        let mut w = (1.0 / (b * b) - u0 * u0 + 2.0 * m * u0 * u0 * u0).max(0.0).sqrt();
        if !ingoing {
            w = -w;
        }
        let (mut u, steps) = (u0, (phi / 1e-4).ceil() as usize);
        let h = phi / steps as f64;
        let a = |u: f64| -u + 3.0 * m * u * u;
        for _ in 0..steps {
            let (k1u, k1w) = (w, a(u));
            let (k2u, k2w) = (w + 0.5 * h * k1w, a(u + 0.5 * h * k1u));
            let (k3u, k3w) = (w + 0.5 * h * k2w, a(u + 0.5 * h * k2u));
            let (k4u, k4w) = (w + h * k3w, a(u + h * k3u));
            u += h / 6.0 * (k1u + 2.0 * k2u + 2.0 * k3u + k4u);
            w += h / 6.0 * (k1w + 2.0 * k2w + 2.0 * k3w + k4w);
        }
        u
    }

    #[test]
    fn the_rows_hold_the_orbit_at_every_column_until_it_ends() {
        let (m, r_o) = (1.0, 80.0);
        let t = orbit_table(m, r_o, 0.4, 64, true);
        let h = ORBIT_SPAN / (ORBIT_COLUMNS - 1) as f64;
        let mut compared = 0;
        for row in (1..64).step_by(3) {
            let b = t.impact(row);
            for ingoing in [true, false] {
                let (end, _) = t.end(row, ingoing);
                for c in (0..ORBIT_COLUMNS).step_by(5) {
                    let phi = c as f64 * h;
                    if phi >= end as f64 {
                        break;
                    }
                    let want = fine_u(m, r_o, b, ingoing, phi);
                    let got = t.at(row, ingoing, c)[0] as f64;
                    assert!((got - want).abs() <= 1e-6 + 1e-5 * want.abs(), "b={b} phi={phi}: {got} vs {want}");
                    compared += 1;
                }
            }
        }
        assert!(compared > 200, "{compared} points compared");
    }

    #[test]
    fn rows_below_the_critical_impact_parameter_fall_in_and_rows_above_it_escape() {
        let (m, r_o) = (1.0, 60.0);
        let t = orbit_table(m, r_o, 0.2, 512, false);
        let critical = 27.0f64.sqrt() * m;
        for row in 0..512 {
            let b = t.impact(row);
            let (_, escaped) = t.end(row, true);
            // a row within its own spacing of b_c may fall either way
            let spacing = (t.impact(row.min(510) + 1) - t.impact(row.max(1) - 1)).abs();
            if (b - critical).abs() > spacing {
                assert_eq!(escaped, b > critical, "b = {b}, critical {critical}");
            }
        }
    }

    #[test]
    fn a_distant_ray_escapes_deflected_by_four_mass_over_b() {
        // weak field: an ingoing ray from far away leaves at pi + 4 M / b (plus the angle it starts at)
        let (m, r_o) = (1.0, 5000.0);
        let t = orbit_table(m, r_o, 0.05, 101, false);
        let weak: Vec<usize> = (0..101).filter(|&row| t.impact(row) >= 100.0 * m).collect();
        assert!(weak.len() >= 3, "rows far outside the photon sphere: {weak:?}");
        for row in weak {
            let b = t.impact(row);
            let (phi_inf, escaped) = t.end(row, true);
            assert!(escaped);
            // the straight line from r_o reaches infinity at pi - asin(b / r_o)
            let want = std::f64::consts::PI - (b / r_o).asin() + 4.0 * m / b;
            assert!((phi_inf as f64 - want).abs() < 0.08 * 4.0 * m / b, "b={b}: {phi_inf} vs {want}");
        }
    }

    #[test]
    fn every_entry_of_the_table_is_finite() {
        let t = orbit_table(1.0, 7.0, 1.0, 128, true);
        assert!(t.data.iter().flatten().all(|v| v.is_finite()));
    }

    #[test]
    fn the_disk_profile_is_normalised_by_its_exact_maximum() {
        // SREP 76, Semantics 8 and open issue 7: f(49/36) = (6/7)^(3/2) 7^(-1/4) = 0.4878713..., not 0.4880
        let exact = (6.0f64 / 7.0).powf(1.5) * 7.0f64.powf(-0.25);
        assert!((profile(49.0 / 36.0) - exact).abs() < 1e-15);
        // and it is the maximum: f' = 0 at x = 49/36, f below it on a fine grid
        assert!((1..20000).map(|i| 1.0 + f64::from(i) * 1e-3).all(|x| profile(x) <= exact + 1e-15));
        let src = include_str!("geodesic.wgsl");
        let line = src
            .lines()
            .find_map(|l| l.trim().strip_prefix("const DISK_PROFILE_PEAK: f32 = "))
            .expect("geodesic.wgsl names the profile's maximum DISK_PROFILE_PEAK");
        let peak: f32 = line.trim_end_matches(';').trim().parse().expect("a number");
        assert_eq!(peak, exact as f32, "the shader divides by f(49/36) rounded to f32");
        assert!(src.contains("/ DISK_PROFILE_PEAK"), "profile() divides by it");
    }
}

#[cfg(test)]
mod wgsl_layout_tests {
    //! The Rust structs bound to WGSL structs have their layout (`crate::wgsl_layout`).
    use crate::wgsl_layout::{assert_agree, rust_fields};

    #[test]
    fn params_matches_wgsl_params() {
        let fields = rust_fields!(super::Params; eye, hole, right, down, fwd, dx, dy, dz, size, misc, extra, table);
        assert_agree(include_str!("geodesic.wgsl"), "Params", std::mem::size_of::<super::Params>(), &fields);
    }
}
