//! GPU fine stage of the vector rasteriser and paint conversion. The tile
//! encoder (`sr_vector::tile`) runs on the CPU; this module turns its
//! output into buffers, resolves its paints into the frame's paint table,
//! and dispatches `raster.wgsl` into a storage texture.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use sr_vector::scene::{GradientKind, Paint};
use sr_vector::tile::Encoded;

use crate::color::{self, Working};
use crate::paint::PaintTable;
use crate::resources::Tex;
use crate::types::{PaintDesc, Stop};

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, Default)]
struct RCmd {
    kind: u32,
    piece_off: u32,
    piece_count: u32,
    backdrop: u32,
    backdrop_val: f32,
    paint: u32,
    flags: u32,
    param: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, Default)]
struct RParams {
    tiles_x: u32,
    tiles_y: u32,
    width: u32,
    height: u32,
    active_count: u32,
    pad: [u32; 3],
}

/// A rasterisation job: encoded tiles with paints resolved, and its target.
pub struct RasterJob {
    pub target: Arc<Tex>,
    size: [u32; 2],
    tiles: [u32; 2],
    ranges: Vec<[u32; 2]>,
    cmds: Vec<RCmd>,
    pieces: Vec<[f32; 4]>,
    backdrops: Vec<f32>,
    /// No layer stack or gradient evaluation is needed by any command.
    solid_fills: bool,
    active_tiles: Option<Vec<u32>>,
    /// `active_tiles` are the tiles that changed since the pixels `target` holds: they are not cleared first.
    partial: bool,
    state: TileState,
}

/// What a rasterisation of a cached target was made from: every job-level input of its pixels and a hash of each
/// tile's inputs. A later job into the same target rasterises only the tiles whose hash differs, provided its
/// job-level inputs equal these.
#[derive(Debug, Clone, PartialEq)]
pub struct TileState {
    size: [u32; 2],
    tiles: [u32; 2],
    solid_fills: bool,
    format: wgpu::TextureFormat,
    hashes: Vec<u64>,
}

impl TileState {
    /// Whether tile hashes of `self` and `other` are comparable: the same size, tile grid, pipeline variant and format.
    fn comparable(&self, other: &TileState) -> bool {
        self.size == other.size
            && self.tiles == other.tiles
            && self.solid_fills == other.solid_fills
            && self.format == other.format
            && self.hashes.len() == other.hashes.len()
    }
}

/// A hash per tile of everything its pixels depend on: its commands, their pieces and backdrops, and the resolved
/// descriptor and stops of each fill's paint (never the paint's index, which depends on the frame's paint table).
fn tile_hashes(e: &Encoded, map: &[u32], t: &PaintTable) -> Vec<u64> {
    use std::hash::{Hash, Hasher};
    e.ranges
        .iter()
        .map(|r| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            let cmds = &e.cmds[r[0] as usize..(r[0] + r[1]) as usize];
            cmds.len().hash(&mut h);
            for c in cmds {
                (c.kind, c.piece_count, c.backdrop_val.to_bits(), c.flags, c.param.to_bits(), c.backdrop == u32::MAX)
                    .hash(&mut h);
                for p in &e.pieces[c.piece_off as usize..(c.piece_off + c.piece_count) as usize] {
                    p.map(f32::to_bits).hash(&mut h);
                }
                if c.backdrop != u32::MAX {
                    for b in &e.backdrops[c.backdrop as usize..c.backdrop as usize + 16] {
                        b.to_bits().hash(&mut h);
                    }
                }
                if c.kind == sr_vector::tile::kind::FILL {
                    let pd = t.paints[map.get(c.paint as usize).copied().unwrap_or(0) as usize];
                    // the stops' position in this frame's table is not an input of the pixels, their values are
                    bytemuck::bytes_of(&PaintDesc { stop_off: 0, ..pd }).hash(&mut h);
                    let stops = pd.stop_off as usize..(pd.stop_off + pd.stop_count) as usize;
                    for s in &t.stops[stops] {
                        bytemuck::bytes_of(s).hash(&mut h);
                    }
                }
            }
            h.finish()
        })
        .collect()
}

/// An sRGB-encoded straight colour into the stored working representation.
pub fn srgb_to_working(w: &Working, c: [f64; 4]) -> [f64; 4] {
    let d = |v: f64| color::decode(sr_model::model::Transfer::Srgb, v);
    w.from_linear_srgb([d(c[0]), d(c[1]), d(c[2]), c[3]])
}

fn xf_desc(x: &sr_vector::Xf) -> ([f32; 4], [f32; 4]) {
    let [a, b, c, d, e, f] = x.0;
    ([a as f32, b as f32, c as f32, d as f32], [e as f32, f as f32, 0.0, 0.0])
}

/// Adds a vector paint to the frame's paint table and returns its index.
pub fn resolve_paint(t: &mut PaintTable, w: &Working, p: &Paint) -> u32 {
    match p {
        Paint::Solid { rgba, srgb } => {
            let stored = if *srgb { srgb_to_working(w, *rgba) } else { w.from_literal(*rgba) };
            t.stops.push(Stop { color: stored.map(|x| x as f32), offset: 0.0, midpoint: 0.5, pad: [0.0; 2] });
            t.paints.push(PaintDesc {
                kind: 0,
                stop_off: (t.stops.len() - 1) as u32,
                stop_count: 1,
                xform0: [1.0, 0.0, 0.0, 1.0],
                ..Default::default()
            });
        }
        Paint::Gradient(g) => {
            let off = t.stops.len() as u32;
            for (o, c) in &g.stops {
                t.stops.push(Stop {
                    color: srgb_to_working(w, *c).map(|x| x as f32),
                    offset: *o as f32,
                    midpoint: 0.5,
                    pad: [0.0; 2],
                });
            }
            let (xform0, xform1) = xf_desc(&g.to_gradient);
            let (kind, p0, p1) = match g.kind {
                GradientKind::Linear { a, b } => (1, [a.x, a.y, b.x, b.y], [0.0; 4]),
                GradientKind::Radial { c, r, f, fr } => (2, [c.x, c.y, r, 1.0], [f.x, f.y, fr, 0.0]),
                GradientKind::Conic { c, start } => (3, [c.x, c.y, start, 0.0], [0.0; 4]),
            };
            t.paints.push(PaintDesc {
                xform0,
                xform1,
                p0: p0.map(|v| v as f32),
                p1: p1.map(|v| v as f32),
                kind,
                spread: g.spread,
                // imported gradients interpolate in sRGB, as SVG and Lottie do
                space: 1,
                stop_off: off,
                stop_count: g.stops.len() as u32,
                ..Default::default()
            });
        }
        Paint::External { index, to_local } => {
            let mut d = t.paints.get(*index as usize).copied().unwrap_or_default();
            // compose: target pixel → paint-local → paint space
            let [a, b, c, dd] = d.xform0.map(|v| v as f64);
            let [e, f, _, _] = d.xform1.map(|v| v as f64);
            let inner = sr_vector::Xf([a, b, c, dd, e, f]).mul(to_local);
            (d.xform0, d.xform1) = xf_desc(&inner);
            t.paints.push(d);
        }
    }
    (t.paints.len() - 1) as u32
}

impl RasterJob {
    /// The size of the target it rasterises into.
    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    /// Bytes of the buffers `Raster::record` creates for it.
    pub fn buffer_bytes(&self) -> u64 {
        (std::mem::size_of_val(&self.cmds[..])
            + std::mem::size_of_val(&self.ranges[..])
            + std::mem::size_of_val(&self.pieces[..])
            + std::mem::size_of_val(&self.backdrops[..])
            + self.active_tiles.as_ref().map_or(0, |a| std::mem::size_of_val(&a[..]))
            + std::mem::size_of::<RParams>()) as u64
    }

    /// Resolves an encoded scene's paints and prepares its buffers.
    pub fn new(e: Encoded, target: Arc<Tex>, t: &mut PaintTable, w: &Working) -> RasterJob {
        Self::build(e, target, t, w, None, false)
    }

    /// Like [`RasterJob::new`] for a target that stays across frames. `prev` is the state of the job that last
    /// rasterised `target`, if its pixels are still there: only the tiles whose inputs changed are then rasterised,
    /// over the existing pixels. Without `prev`, or when a job-level input differs, every tile is.
    pub fn new_cached(
        e: Encoded,
        target: Arc<Tex>,
        t: &mut PaintTable,
        w: &Working,
        prev: Option<&TileState>,
    ) -> RasterJob {
        Self::build(e, target, t, w, prev, true)
    }

    /// The state of this job, for the next job into the same target.
    pub fn tile_state(&self) -> &TileState {
        &self.state
    }

    /// Whether this job leaves the tiles of `active_tiles` only to be rasterised, over the target's existing pixels.
    pub fn is_partial(&self) -> bool {
        self.partial
    }

    /// The number of tiles this job rasterises.
    pub fn dispatched_tiles(&self) -> usize {
        self.active_tiles.as_ref().map_or((self.tiles[0] * self.tiles[1]) as usize, Vec::len)
    }

    fn build(
        e: Encoded,
        target: Arc<Tex>,
        t: &mut PaintTable,
        w: &Working,
        prev: Option<&TileState>,
        track: bool,
    ) -> RasterJob {
        let map: Vec<u32> = e.paints.iter().map(|p| resolve_paint(t, w, p)).collect();
        let solid_fills = e.cmds.iter().all(|c| c.kind == sr_vector::tile::kind::FILL)
            && map.iter().all(|&i| t.paints[i as usize].kind == 0);
        let cmds = e
            .cmds
            .iter()
            .map(|c| RCmd {
                kind: c.kind,
                piece_off: c.piece_off,
                piece_count: c.piece_count,
                backdrop: c.backdrop,
                backdrop_val: c.backdrop_val,
                paint: if c.kind == sr_vector::tile::kind::FILL {
                    map.get(c.paint as usize).copied().unwrap_or(0)
                } else {
                    0
                },
                flags: c.flags,
                param: c.param,
            })
            .collect();
        let nonempty = e.ranges.iter().filter(|r| r[1] != 0).count();
        let active_tiles = (nonempty * 2 < e.ranges.len())
            .then(|| e.ranges.iter().enumerate().filter_map(|(i, r)| (r[1] != 0).then_some(i as u32)).collect());
        let state = TileState {
            size: e.size,
            tiles: e.tiles,
            solid_fills,
            format: target.tex.format(),
            hashes: if track { tile_hashes(&e, &map, t) } else { Vec::new() },
        };
        let changed: Option<Vec<u32>> = prev.filter(|p| track && p.comparable(&state)).map(|p| {
            let same = |i: &usize| p.hashes[*i] == state.hashes[*i];
            (0..state.hashes.len()).filter(|i| !same(i)).map(|i| i as u32).collect()
        });
        // every tile changed: the whole-target path is cheaper than a list of all of them
        let changed = changed.filter(|c| c.len() < state.hashes.len());
        let partial = changed.is_some();
        let active_tiles = changed.or(active_tiles);
        RasterJob {
            target,
            size: e.size,
            tiles: e.tiles,
            ranges: e.ranges,
            cmds,
            pieces: e.pieces,
            backdrops: e.backdrops,
            solid_fills,
            active_tiles,
            partial,
            state,
        }
    }
}

/// The fine-stage pipelines, each compiled the first time a job needs it.
pub struct Raster {
    device: wgpu::Device,
    module: wgpu::ShaderModule,
    layout: wgpu::PipelineLayout,
    pipelines: [std::sync::OnceLock<wgpu::ComputePipeline>; 4],
    bgl: wgpu::BindGroupLayout,
    /// The jobs' parameters, commands, ranges, pieces, backdrops and active tiles since the last submit.
    arenas: [Arena; 6],
    /// Buffers made so far for jobs (a job that fits the arenas makes none).
    pub buffers_made: usize,
}

/// Where the jobs' records of one kind live: a buffer the jobs since the last submit append to at aligned offsets,
/// replaced by a larger one when a job does not fit (the bind groups of the jobs before keep the old one alive),
/// and rewound when the queue takes the jobs. A `queue.write_buffer` lands before the next submit's first command,
/// so two jobs between submits never share bytes.
struct Arena {
    buf: Option<wgpu::Buffer>,
    cap: u64,
    cursor: u64,
    usage: wgpu::BufferUsages,
    label: &'static str,
}

/// Storage and uniform offsets within an arena are multiples of this (what the device limits ask at most).
const ARENA_ALIGN: u64 = 256;

impl Arena {
    fn new(usage: wgpu::BufferUsages, label: &'static str) -> Arena {
        Arena { buf: None, cap: 0, cursor: 0, usage, label }
    }

    /// Writes `bytes` after the jobs before and returns where they are bound: at least 64 bytes, zero-padded, as
    /// an empty scene still binds one zeroed element of every record type. `made` counts the buffers this makes.
    fn place(&mut self, d: &wgpu::Device, q: &wgpu::Queue, bytes: &[u8], made: &mut usize) -> Place {
        let size = (bytes.len() as u64).max(64).div_ceil(4) * 4;
        if self.buf.is_none() || self.cursor + size > self.cap {
            let cap = size.max(self.cap * 2).max(64 << 10).div_ceil(ARENA_ALIGN) * ARENA_ALIGN;
            self.buf = Some(d.create_buffer(&wgpu::BufferDescriptor {
                label: Some(self.label),
                size: cap,
                usage: self.usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
            self.cap = cap;
            self.cursor = 0;
            *made += 1;
        }
        let buf = self.buf.clone().expect("made");
        let at = self.cursor;
        if (bytes.len() as u64) < size {
            let mut padded = bytes.to_vec();
            padded.resize(size as usize, 0);
            q.write_buffer(&buf, at, &padded);
        } else {
            q.write_buffer(&buf, at, bytes);
        }
        self.cursor = (at + size).div_ceil(ARENA_ALIGN) * ARENA_ALIGN;
        Place { buf, at, size }
    }
}

/// A job's records of one kind: where an arena placed them.
struct Place {
    buf: wgpu::Buffer,
    at: u64,
    size: u64,
}

impl Place {
    fn binding(&self) -> wgpu::BindingResource<'_> {
        wgpu::BindingResource::Buffer(wgpu::BufferBinding {
            buffer: &self.buf,
            offset: self.at,
            size: std::num::NonZeroU64::new(self.size),
        })
    }
}

impl Raster {
    /// Builds the raster stage for [`crate::resources::FORMAT`] targets.
    pub fn new(d: &wgpu::Device) -> Raster {
        Self::new_as(d, crate::resources::FORMAT)
    }

    /// Builds the raster stage for targets of `format` (a working format of SREP 72).
    pub fn new_as(d: &wgpu::Device, format: wgpu::TextureFormat) -> Raster {
        let st = |b: u32| wgpu::BindGroupLayoutEntry {
            binding: b,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let un = |b: u32| wgpu::BindGroupLayoutEntry {
            binding: b,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("raster"),
            entries: &[
                st(0),
                st(1),
                st(2),
                st(3),
                st(4),
                st(5),
                st(9),
                un(6),
                un(7),
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("raster"),
            source: wgpu::ShaderSource::Wgsl({
                let src = concat!(include_str!("common.wgsl"), include_str!("raster.wgsl"));
                if format == crate::resources::FORMAT_F32 {
                    src.replace("texture_storage_2d<rgba16float, write>", "texture_storage_2d<rgba32float, write>")
                        .into()
                } else {
                    src.into()
                }
            }),
        });
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("raster"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let (uniform, storage) = (wgpu::BufferUsages::UNIFORM, wgpu::BufferUsages::STORAGE);
        let arenas = [
            Arena::new(uniform, "raster params"),
            Arena::new(storage, "rcmds"),
            Arena::new(storage, "ranges"),
            Arena::new(storage, "pieces"),
            Arena::new(storage, "backdrops"),
            Arena::new(storage, "active tiles"),
        ];
        Raster { device: d.clone(), module, layout, pipelines: Default::default(), bgl, arenas, buffers_made: 0 }
    }

    /// The pipeline for variant `i` (bit 0 `SOLID_FILLS`, bit 1 `SPARSE_TILES`).
    fn pipeline(&self, i: usize) -> &wgpu::ComputePipeline {
        self.pipelines[i].get_or_init(|| {
            let _creation = crate::gpu::creation_lock();
            self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("raster"),
                layout: Some(&self.layout),
                module: &self.module,
                entry_point: Some("raster_main"),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &[("SOLID_FILLS", (i & 1) as f64), ("SPARSE_TILES", (i >> 1) as f64)],
                    ..Default::default()
                },
                cache: None,
            })
        })
    }

    /// Records a job into `enc`, reading the frame's paint buffers.
    /// The queue took every job recorded so far: the arenas serve the jobs recorded next from their start.
    pub fn submitted(&mut self) {
        for arena in &mut self.arenas {
            arena.cursor = 0;
        }
    }

    pub fn record(
        &mut self,
        q: &wgpu::Queue,
        enc: &mut wgpu::CommandEncoder,
        job: &RasterJob,
        paints: &wgpu::Buffer,
        stops: &wgpu::Buffer,
        globals: &wgpu::Buffer,
    ) {
        let d = &self.device;
        let view =
            job.target.tex.create_view(&wgpu::TextureViewDescriptor { mip_level_count: Some(1), ..Default::default() });
        if let Some(active) = &job.active_tiles {
            if job.partial {
                // The other tiles keep the pixels of the job before; every tile of the list is overwritten.
                if active.is_empty() {
                    return;
                }
            } else {
                // A clear covers empty tiles more cheaply than launching a pixel
                // invocation for each of them. Every populated tile is overwritten.
                drop(enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("clear sparse raster"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                }));
                if active.is_empty() {
                    return;
                }
            }
        }
        let params = RParams {
            tiles_x: job.tiles[0],
            tiles_y: job.tiles[1],
            width: job.size[0],
            height: job.size[1],
            active_count: job.active_tiles.as_ref().map_or(0, |a| a.len() as u32),
            pad: [0; 3],
        };
        let made = &mut self.buffers_made;
        let [pa, ca, ra, pia, ba, aa] = &mut self.arenas;
        let pb = pa.place(d, q, bytemuck::bytes_of(&params), made);
        let (c, r, p, b) = (
            ca.place(d, q, bytemuck::cast_slice(&job.cmds), made),
            ra.place(d, q, bytemuck::cast_slice(&job.ranges), made),
            pia.place(d, q, bytemuck::cast_slice(&job.pieces), made),
            ba.place(d, q, bytemuck::cast_slice(&job.backdrops), made),
        );
        let active = job.active_tiles.as_ref().map(|tiles| aa.place(d, q, bytemuck::cast_slice(tiles), made));
        let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("raster"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: c.binding() },
                wgpu::BindGroupEntry { binding: 1, resource: r.binding() },
                wgpu::BindGroupEntry { binding: 2, resource: p.binding() },
                wgpu::BindGroupEntry { binding: 3, resource: b.binding() },
                wgpu::BindGroupEntry { binding: 4, resource: paints.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: stops.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: globals.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: pb.binding() },
                wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 9, resource: active.as_ref().unwrap_or(&r).binding() },
            ],
        });
        let mut pass =
            enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("raster"), timestamp_writes: None });
        pass.set_pipeline(self.pipeline(job.solid_fills as usize + 2 * job.active_tiles.is_some() as usize));
        pass.set_bind_group(0, &bind, &[]);
        if job.active_tiles.is_some() {
            let n = params.active_count;
            pass.dispatch_workgroups(n.min(65535), n.div_ceil(65535), 1);
        } else {
            pass.dispatch_workgroups(job.tiles[0], job.tiles[1], 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A storage buffer of `data` for a hand-made job's paints and stops.
    fn storage<T: Pod>(d: &wgpu::Device, data: &[T], label: &str) -> wgpu::Buffer {
        use wgpu::util::DeviceExt;
        d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::cast_slice(data),
            usage: wgpu::BufferUsages::STORAGE,
        })
    }

    #[test]
    fn solid_fill_specialization_matches_general_rasterization_exactly() {
        let gpu = sr_testkit::require_gpu!(
            crate::gpu::test_gpu(),
            "solid_fill_specialization_matches_general_rasterization_exactly"
        );
        let d = &gpu.device;
        let layout = crate::resources::source_layout(d);
        // Partial edge tiles, an empty tile, fractional edge coverage, overlapping
        // translucent paints, and both winding rules exercise the shared arithmetic.
        let target = Arc::new(crate::resources::create(d, &layout, [31, 19], 1, "raster oracle"));
        let mut job = RasterJob {
            target: target.clone(),
            size: target.size,
            tiles: [2, 2],
            ranges: vec![[0, 3], [0, 0], [1, 2], [0, 3]],
            cmds: vec![
                RCmd { backdrop: u32::MAX, backdrop_val: 0.25, param: 0.75, ..Default::default() },
                RCmd { backdrop: 0, piece_count: 1, paint: 1, param: 0.5, ..Default::default() },
                RCmd { backdrop: u32::MAX, backdrop_val: 1.4, flags: 1, param: 0.9, ..Default::default() },
            ],
            pieces: vec![[0.3, 0.1, 12.7, 15.9]],
            backdrops: (0..16).map(|y| y as f32 / 32.0).collect(),
            solid_fills: false,
            active_tiles: None,
            partial: false,
            state: TileState {
                size: target.size,
                tiles: [2, 2],
                solid_fills: false,
                format: target.tex.format(),
                hashes: Vec::new(),
            },
        };
        let paints = storage(d, &[PaintDesc::default(), PaintDesc { stop_off: 1, ..Default::default() }], "paints");
        let stops = storage(
            d,
            &[
                Stop { color: [0.1, 0.7, 0.3, 0.65], ..Default::default() },
                Stop { color: [0.9, 0.2, 0.4, 0.35], ..Default::default() },
            ],
            "stops",
        );
        let globals = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<crate::types::Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });
        let document = sr_model::load_str(
            r#"<scene version="1.2"><project width="31" height="19" fps="1" duration="1"/><composition/></scene>"#,
            &sr_model::LoadOptions::default(),
        )
        .unwrap();
        let evaluator = sr_eval::Evaluator::new(&document, &sr_eval::EvalOptions::default()).unwrap();
        let reader = crate::Renderer::new(gpu.clone(), evaluator.program());
        let mut raster = Raster::new(d);
        let mut render = |job: &RasterJob| {
            let mut encoder = d.create_command_encoder(&Default::default());
            raster.record(&gpu.queue, &mut encoder, job, &paints, &stops, &globals);
            gpu.queue.submit([encoder.finish()]);
            raster.submitted();
            reader.read(&job.target)
        };
        let general = render(&job);
        job.solid_fills = true;
        let specialized = render(&job);
        assert!(general.iter().any(|p| p[3] > 0.0));
        assert!(general.contains(&[0.0; 4]));
        assert_eq!(specialized, general);
        job.active_tiles = Some(vec![0, 2, 3]);
        assert_eq!(render(&job), general, "sparse solid tiles");
        job.solid_fills = false;
        assert_eq!(render(&job), general, "sparse general tiles");
        job.active_tiles = Some(Vec::new());
        assert!(render(&job).iter().all(|p| *p == [0.0; 4]), "empty jobs clear stale texels");
    }

    /// A GPU, the raster pipeline and a reader, for rasterising hand-made tiles.
    struct Rig {
        gpu: crate::gpu::Gpu,
        raster: Raster,
        reader: crate::Renderer,
        layout: wgpu::BindGroupLayout,
    }

    impl Rig {
        fn new(test: &str) -> Option<Rig> {
            let gpu = sr_testkit::gpu(crate::gpu::test_gpu(), test)?;
            let document = sr_model::load_str(
                r#"<scene version="1.2"><project width="31" height="19" fps="1" duration="1"/><composition/></scene>"#,
                &sr_model::LoadOptions::default(),
            )
            .unwrap();
            let evaluator = sr_eval::Evaluator::new(&document, &sr_eval::EvalOptions::default()).unwrap();
            Some(Rig {
                raster: Raster::new(&gpu.device),
                reader: crate::Renderer::new(gpu.clone(), evaluator.program()),
                layout: crate::resources::source_layout(&gpu.device),
                gpu,
            })
        }

        fn target(&self, size: [u32; 2]) -> Arc<Tex> {
            Arc::new(crate::resources::create(&self.gpu.device, &self.layout, size, 1, "dirty target"))
        }

        /// Records and submits `job`, with the paints `table` holds, and reads its target.
        fn render(&mut self, job: &RasterJob, table: &PaintTable) -> Vec<[f32; 4]> {
            let d = &self.gpu.device;
            let (paints, stops) = (storage(d, &table.paints, "paints"), storage(d, &table.stops, "stops"));
            let globals = d.create_buffer(&wgpu::BufferDescriptor {
                label: Some("globals"),
                size: std::mem::size_of::<crate::types::Globals>() as u64,
                usage: wgpu::BufferUsages::UNIFORM,
                mapped_at_creation: false,
            });
            let mut encoder = d.create_command_encoder(&Default::default());
            self.raster.record(&self.gpu.queue, &mut encoder, job, &paints, &stops, &globals);
            self.gpu.queue.submit([encoder.finish()]);
            self.raster.submitted();
            self.reader.read(&job.target)
        }
    }

    fn working() -> Working {
        Working { space: sr_model::model::ColorSpace::Srgb, linear: false }
    }

    /// A 48x32 scene of six tiles; tile `i` is one even-odd-free fill of `colors[i]` at coverage `cover[i]`.
    fn six_tiles(colors: [[f64; 4]; 6], cover: [f32; 6]) -> Encoded {
        Encoded {
            size: [48, 32],
            tiles: [3, 2],
            ranges: (0..6).map(|i| [i, 1]).collect(),
            cmds: (0..6)
                .map(|i| sr_vector::tile::GpuCmd {
                    kind: sr_vector::tile::kind::FILL,
                    backdrop: u32::MAX,
                    backdrop_val: cover[i as usize],
                    paint: i,
                    param: 1.0,
                    ..Default::default()
                })
                .collect(),
            paints: colors.iter().map(|&rgba| Paint::Solid { rgba, srgb: true }).collect(),
            ..Default::default()
        }
    }

    const COLORS: [[f64; 4]; 6] = [
        [1.0, 0.0, 0.0, 1.0],
        [0.0, 1.0, 0.0, 1.0],
        [0.0, 0.0, 1.0, 1.0],
        [1.0, 1.0, 0.0, 0.5],
        [0.0, 1.0, 1.0, 1.0],
        [1.0, 0.0, 1.0, 0.75],
    ];

    /// A full raster of `e` into a fresh target: the oracle of a partial one.
    fn full(rig: &mut Rig, e: Encoded) -> Vec<[f32; 4]> {
        let mut table = PaintTable::default();
        let size = e.size;
        let job = RasterJob::new_cached(e, rig.target(size), &mut table, &working(), None);
        assert!(!job.is_partial());
        rig.render(&job, &table)
    }

    #[test]
    fn changing_one_tile_rasterises_only_that_tile_and_matches_a_full_raster() {
        let Some(mut rig) = Rig::new("changing_one_tile_rasterises_only_that_tile_and_matches_a_full_raster") else {
            return;
        };
        let target = rig.target([48, 32]);
        let mut table = PaintTable::default();
        let first = RasterJob::new_cached(six_tiles(COLORS, [1.0; 6]), target.clone(), &mut table, &working(), None);
        assert!(!first.is_partial());
        rig.render(&first, &table);

        let mut cover = [1.0; 6];
        cover[4] = 0.5;
        let mut table = PaintTable::default();
        let second =
            RasterJob::new_cached(six_tiles(COLORS, cover), target, &mut table, &working(), Some(first.tile_state()));
        assert!(second.is_partial());
        assert_eq!(second.dispatched_tiles(), 1);
        let partial = rig.render(&second, &table);
        let oracle = full(&mut rig, six_tiles(COLORS, cover));
        assert_ne!(partial, rig.render(&first, &table), "the frames differ");
        assert_eq!(partial, oracle, "a partial raster is bit-identical to a full one");
    }

    #[test]
    fn an_unchanged_frame_dispatches_nothing_and_keeps_the_pixels() {
        let Some(mut rig) = Rig::new("an_unchanged_frame_dispatches_nothing_and_keeps_the_pixels") else {
            return;
        };
        let target = rig.target([48, 32]);
        let mut table = PaintTable::default();
        let first = RasterJob::new_cached(six_tiles(COLORS, [1.0; 6]), target.clone(), &mut table, &working(), None);
        let before = rig.render(&first, &table);
        let mut table = PaintTable::default();
        let second = RasterJob::new_cached(
            six_tiles(COLORS, [1.0; 6]),
            target,
            &mut table,
            &working(),
            Some(first.tile_state()),
        );
        assert!(second.is_partial());
        assert_eq!(second.dispatched_tiles(), 0);
        assert!(before.iter().any(|p| p[3] > 0.0));
        assert_eq!(rig.render(&second, &table), before);
    }

    #[test]
    fn a_colour_change_with_identical_geometry_invalidates_the_tile() {
        let Some(mut rig) = Rig::new("a_colour_change_with_identical_geometry_invalidates_the_tile") else {
            return;
        };
        let target = rig.target([48, 32]);
        let mut table = PaintTable::default();
        let first = RasterJob::new_cached(six_tiles(COLORS, [1.0; 6]), target.clone(), &mut table, &working(), None);
        rig.render(&first, &table);

        let mut colors = COLORS;
        colors[2] = [0.2, 0.4, 0.6, 1.0];
        let mut table = PaintTable::default();
        let second = RasterJob::new_cached(
            six_tiles(colors, [1.0; 6]),
            target,
            &mut table,
            &working(),
            Some(first.tile_state()),
        );
        assert!(second.is_partial());
        assert_eq!(second.dispatched_tiles(), 1);
        assert_eq!(rig.render(&second, &table), full(&mut rig, six_tiles(colors, [1.0; 6])));
    }

    #[test]
    fn the_same_colours_at_other_paint_indices_keep_their_tiles() {
        let Some(mut rig) = Rig::new("the_same_colours_at_other_paint_indices_keep_their_tiles") else {
            return;
        };
        let target = rig.target([48, 32]);
        let mut table = PaintTable::default();
        let first = RasterJob::new_cached(six_tiles(COLORS, [1.0; 6]), target.clone(), &mut table, &working(), None);
        rig.render(&first, &table);
        // The table of a later frame already holds other paints: the same tiles resolve to other indices.
        let mut table = PaintTable::default();
        for i in 0..5 {
            resolve_paint(&mut table, &working(), &Paint::Solid { rgba: [i as f64 / 5.0; 4], srgb: true });
        }
        let second = RasterJob::new_cached(
            six_tiles(COLORS, [1.0; 6]),
            target,
            &mut table,
            &working(),
            Some(first.tile_state()),
        );
        assert_eq!(second.dispatched_tiles(), 0);
    }

    #[test]
    fn job_level_changes_force_a_full_raster() {
        let Some(rig) = Rig::new("job_level_changes_force_a_full_raster") else {
            return;
        };
        let mut table = PaintTable::default();
        let first =
            RasterJob::new_cached(six_tiles(COLORS, [1.0; 6]), rig.target([48, 32]), &mut table, &working(), None);
        let prev = first.tile_state();

        // the target size, with the tile grid unchanged
        let mut e = six_tiles(COLORS, [1.0; 6]);
        e.size = [47, 32];
        let job = RasterJob::new_cached(e, rig.target([47, 32]), &mut PaintTable::default(), &working(), Some(prev));
        assert!(!job.is_partial(), "size");

        // the tile grid
        let mut e = six_tiles(COLORS, [1.0; 6]);
        e.tiles = [2, 3];
        let job = RasterJob::new_cached(e, rig.target([48, 32]), &mut PaintTable::default(), &working(), Some(prev));
        assert!(!job.is_partial(), "tile grid");

        // the solid-fill pipeline variant: an unused gradient paint leaves every tile's content as it was
        let mut e = six_tiles(COLORS, [1.0; 6]);
        e.paints.push(Paint::Gradient(Box::new(sr_vector::scene::Gradient {
            kind: GradientKind::Linear { a: sr_vector::P { x: 0.0, y: 0.0 }, b: sr_vector::P { x: 1.0, y: 0.0 } },
            stops: vec![(0.0, [0.0, 0.0, 0.0, 1.0]), (1.0, [1.0, 1.0, 1.0, 1.0])],
            to_gradient: sr_vector::Xf([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
            spread: 0,
        })));
        let job = RasterJob::new_cached(e, rig.target([48, 32]), &mut PaintTable::default(), &working(), Some(prev));
        assert!(!job.is_partial(), "solid_fills");

        // the working format
        let mut other = prev.clone();
        other.format = wgpu::TextureFormat::Rgba32Float;
        let job = RasterJob::new_cached(
            six_tiles(COLORS, [1.0; 6]),
            rig.target([48, 32]),
            &mut PaintTable::default(),
            &working(),
            Some(&other),
        );
        assert!(!job.is_partial(), "format");

        // control: nothing changed
        let job = RasterJob::new_cached(
            six_tiles(COLORS, [1.0; 6]),
            rig.target([48, 32]),
            &mut PaintTable::default(),
            &working(),
            Some(prev),
        );
        assert!(job.is_partial());
    }
}

#[cfg(test)]
mod wgsl_layout_tests {
    //! The Rust structs bound to WGSL structs have their layout (`crate::wgsl_layout`).
    use crate::wgsl_layout::{assert_agree, rust_fields};

    #[test]
    fn r_cmd_matches_wgsl_r_cmd() {
        let fields =
            rust_fields!(super::RCmd; kind, piece_off, piece_count, backdrop, backdrop_val, paint, flags, param);
        assert_agree(include_str!("raster.wgsl"), "RCmd", std::mem::size_of::<super::RCmd>(), &fields);
    }

    #[test]
    fn r_params_matches_wgsl_r_params() {
        let fields = rust_fields!(super::RParams; tiles_x, tiles_y, width, height, active_count, pad);
        assert_agree(include_str!("raster.wgsl"), "RParams", std::mem::size_of::<super::RParams>(), &fields);
    }
}
