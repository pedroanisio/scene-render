//! Video on the GPU: decoded planes become premultiplied working-space
//! textures (YUV matrix and range, chroma upsampling, rotation, input
//! transfer, primaries), and in-between source frames are synthesised by
//! frame mixing or DIS optical flow.

use std::collections::HashMap;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use sr_media::{PixelLayout, VideoFrame};
use sr_model::model::{AlphaMode, ColorSpace, Transfer};

use crate::color::{self, Working};
use crate::resources::{self, Pool, Tex};
use crate::types;

/// How to interpret a decoded stream.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Interpretation {
    /// Primaries of the samples.
    pub space: ColorSpace,
    /// Transfer of the samples (resolved, not `auto`).
    pub transfer: Transfer,
    /// Alpha handling.
    pub alpha: AlphaMode,
    /// Clockwise quarter turns to display orientation.
    pub quarter_turns: u32,
    /// YUV matrix coefficients (Kr, Kb).
    pub matrix: (f32, f32),
    /// Full-range samples.
    pub full_range: bool,
}

impl Interpretation {
    /// Matrix coefficients from an FFmpeg `color_space` tag, defaulting by height.
    pub fn matrix_for(tag: &str, height: u32) -> (f32, f32) {
        match tag {
            "bt2020nc" | "bt2020c" | "bt2020_ncl" => (0.2627, 0.0593),
            "smpte170m" | "bt470bg" | "bt601" => (0.299, 0.114),
            "bt709" => (0.2126, 0.0722),
            _ if height >= 720 => (0.2126, 0.0722),
            _ => (0.299, 0.114),
        }
    }

    /// The transfer an FFmpeg `color_transfer` tag names, if it is one of the schema's.
    pub fn transfer_for(tag: &str) -> Option<Transfer> {
        Some(match tag {
            "smpte2084" => Transfer::Pq,
            "arib-std-b67" => Transfer::Hlg,
            "iec61966-2-1" => Transfer::Srgb,
            "bt709" | "bt1361e" | "smpte170m" | "bt2020-10" | "bt2020-12" => Transfer::Bt1886,
            "gamma22" => Transfer::Gamma22,
            "linear" => Transfer::Linear,
            _ => return None,
        })
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Conv {
    to_working: [[f32; 4]; 3],
    size_out: [u32; 2],
    coded: [u32; 2],
    chroma_shift: [u32; 2],
    packing: u32,
    rotation: u32,
    transfer: u32,
    alpha_mode: u32,
    full_range: u32,
    deep: u32,
    linear_light: u32,
    store_transfer: u32,
    kr: f32,
    kb: f32,
    pad: [u32; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct FlowParams {
    size: [u32; 2],
    grid: [u32; 2],
    coarse: [u32; 2],
    t: f32,
    iterations: u32,
}

#[derive(Debug)]
struct ConvPipe {
    bgl: wgpu::BindGroupLayout,
    pipe: wgpu::RenderPipeline,
}

/// The shaders, layouts and pipelines of video processing on one device. Immutable objects of the device, so the
/// engines on it share one set while any of them holds it ([`KITS`]).
#[derive(Debug)]
struct Kit {
    conv_f: ConvPipe,
    conv_u: ConvPipe,
    mix: wgpu::RenderPipeline,
    warp: wgpu::RenderPipeline,
    reduce: wgpu::ComputePipeline,
    patch: wgpu::ComputePipeline,
    densify: wgpu::ComputePipeline,
    bgl0: wgpu::BindGroupLayout,
    bgl1: wgpu::BindGroupLayout,
    bgl2: wgpu::BindGroupLayout,
    bgl3: wgpu::BindGroupLayout,
}

/// The kits of the video engines alive, by device.
static KITS: crate::gpu::PerDevice<(), Kit> = crate::gpu::PerDevice::new();

/// Pipelines for video processing, and the resources its passes reuse between frames.
pub struct VideoEngine {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    /// The device's kit, taken when the engine first converts, mixes or interpolates a frame.
    kit: std::sync::OnceLock<Arc<crate::gpu::OfDevice<Kit>>>,
    samp: wgpu::Sampler,
    /// A 1×1 flow of zeros: what a frame mix warps by.
    no_flow: wgpu::TextureView,
    /// Plane textures and conversion bindings by decoded size and layout.
    planes: Ring<(u32, u32, PixelLayout), PlaneSet>,
    /// Luma pyramids, flow levels and patch bindings by frame size.
    flows: Ring<[u32; 2], FlowSet>,
    /// The parameters of the frame's blend passes.
    blends: Ring<(), Params>,
}

fn entry(binding: u32, vis: wgpu::ShaderStages, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: vis, ty, count: None }
}

fn tex_ty(sample: wgpu::TextureSampleType) -> wgpu::BindingType {
    wgpu::BindingType::Texture {
        sample_type: sample,
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: false,
    }
}

fn uniform() -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Uniform,
        has_dynamic_offset: false,
        min_binding_size: None,
    }
}

fn storage_tex(f: wgpu::TextureFormat) -> wgpu::BindingType {
    wgpu::BindingType::StorageTexture {
        access: wgpu::StorageTextureAccess::WriteOnly,
        format: f,
        view_dimension: wgpu::TextureViewDimension::D2,
    }
}

const F: wgpu::ShaderStages = wgpu::ShaderStages::FRAGMENT;
const C: wgpu::ShaderStages = wgpu::ShaderStages::COMPUTE;
const UNFILTERED: wgpu::TextureSampleType = wgpu::TextureSampleType::Float { filterable: false };
const FILTERED: wgpu::TextureSampleType = wgpu::TextureSampleType::Float { filterable: true };

impl Kit {
    fn new(d: &wgpu::Device) -> Kit {
        let transfer = include_str!("transfer.wgsl");
        let conv = |tex_t: &str, scale: &str, sample: wgpu::TextureSampleType| {
            let src = format!(
                "{transfer}\n{}",
                include_str!("video.wgsl").replace("TEX_T", tex_t).replace("LOAD_SCALE", scale)
            );
            let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("video"),
                source: wgpu::ShaderSource::Wgsl(src.into()),
            });
            let bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("video planes"),
                entries: &[
                    entry(0, F, uniform()),
                    entry(1, F, tex_ty(sample)),
                    entry(2, F, tex_ty(sample)),
                    entry(3, F, tex_ty(sample)),
                ],
            });
            let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[Some(&bgl)],
                immediate_size: 0,
            });
            let pipe = full_pipeline(d, &module, &layout, "fs_convert");
            ConvPipe { bgl, pipe }
        };
        let conv_f = conv("f32", "1.0", FILTERED);
        let conv_u = conv("u32", "(1.0 / 65535.0)", wgpu::TextureSampleType::Uint);
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flow"),
            source: wgpu::ShaderSource::Wgsl(include_str!("flow.wgsl").into()),
        });
        let bgl0 = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("flow params"),
            entries: &[entry(0, C | F, uniform())],
        });
        let bgl1 = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("reduce"),
            entries: &[
                entry(0, C, tex_ty(UNFILTERED)),
                entry(1, C, tex_ty(UNFILTERED)),
                entry(2, C, storage_tex(wgpu::TextureFormat::R32Float)),
                entry(3, C, storage_tex(wgpu::TextureFormat::R32Float)),
            ],
        });
        let bgl2 = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("patches"),
            entries: &[
                entry(0, C, tex_ty(UNFILTERED)),
                entry(1, C, tex_ty(UNFILTERED)),
                entry(2, C, tex_ty(UNFILTERED)),
                entry(
                    3,
                    C,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                entry(4, C, storage_tex(wgpu::TextureFormat::Rgba32Float)),
            ],
        });
        let bgl3 = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("warp"),
            entries: &[
                entry(0, F, tex_ty(FILTERED)),
                entry(1, F, tex_ty(FILTERED)),
                entry(2, F, tex_ty(UNFILTERED)),
                entry(3, F, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
            ],
        });
        let compute = |groups: &[Option<&wgpu::BindGroupLayout>], ep: &str| {
            let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: groups,
                immediate_size: 0,
            });
            {
                let _creation = crate::gpu::creation_lock();
                d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(ep),
                    layout: Some(&layout),
                    module: &module,
                    entry_point: Some(ep),
                    compilation_options: Default::default(),
                    cache: None,
                })
            }
        };
        let reduce = compute(&[Some(&bgl0), Some(&bgl1)], "cs_reduce");
        let patch = compute(&[Some(&bgl0), None, Some(&bgl2)], "cs_patch");
        let densify = compute(&[Some(&bgl0), None, Some(&bgl2)], "cs_densify");
        let frag_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bgl0), None, None, Some(&bgl3)],
            immediate_size: 0,
        });
        let mix = full_pipeline(d, &module, &frag_layout, "fs_mix");
        let warp = full_pipeline(d, &module, &frag_layout, "fs_warp");
        Kit { conv_f, conv_u, mix, warp, reduce, patch, densify, bgl0, bgl1, bgl2, bgl3 }
    }
}

impl VideoEngine {
    /// An engine whose pipelines are those of every other engine on the device, built when one first needs them.
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> VideoEngine {
        let d = &*device;
        let samp = d.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let no_flow = flow_texture(d, [1, 1]).create_view(&Default::default());
        VideoEngine {
            device,
            queue,
            kit: std::sync::OnceLock::new(),
            samp,
            no_flow,
            planes: Ring::new(),
            flows: Ring::new(),
            blends: Ring::new(),
        }
    }

    /// The device's video pipelines.
    fn kit(&self) -> Arc<crate::gpu::OfDevice<Kit>> {
        self.kit.get_or_init(|| KITS.get(&self.device, (), || Kit::new(&self.device))).clone()
    }

    /// Records `f` into a command encoder of its own and submits it: for work outside a frame, like tests and
    /// diagnostics. A frame records into its own encoder and calls [`VideoEngine::submitted`] after submitting it.
    pub fn run<R>(&mut self, f: impl FnOnce(&mut VideoEngine, &mut wgpu::CommandEncoder) -> R) -> R {
        let mut enc = self.device.create_command_encoder(&Default::default());
        let r = f(self, &mut enc);
        self.queue.submit([enc.finish()]);
        self.submitted();
        r
    }

    /// The queue took every pass recorded so far: the next frame takes the plane sets, flow sets and blend
    /// parameters from the first again.
    pub fn submitted(&mut self) {
        self.planes.rewind();
        self.flows.rewind();
        self.blends.rewind();
    }

    /// Ends a frame: drops the resources of sizes and layouts no frame used for a while.
    pub fn trim(&mut self) {
        self.planes.trim();
        self.flows.trim();
        self.blends.trim();
    }

    /// Uploads `frame` and records its conversion to a premultiplied working-space texture from `pool`, into `enc`.
    /// The planes go to textures kept by size and layout; the uniforms and bind group with them.
    pub fn convert(
        &mut self,
        enc: &mut wgpu::CommandEncoder,
        pool: &mut Pool,
        frame: &VideoFrame,
        it: &Interpretation,
        working: &Working,
        src_layout: &wgpu::BindGroupLayout,
    ) -> Arc<Tex> {
        let kit = self.kit();
        let d = self.device.clone();
        let deep = matches!(frame.layout, PixelLayout::Yuv16(..) | PixelLayout::Rgba16);
        let rgba = matches!(frame.layout, PixelLayout::Rgba8 | PixelLayout::Rgba16);
        let fmt = match (rgba, deep) {
            (false, false) => wgpu::TextureFormat::R8Unorm,
            (false, true) => wgpu::TextureFormat::R16Uint,
            (true, false) => wgpu::TextureFormat::Rgba8Unorm,
            (true, true) => wgpu::TextureFormat::Rgba16Uint,
        };
        let cp = if deep { &kit.conv_u } else { &kit.conv_f };
        let set = self.planes.take((frame.width, frame.height, frame.layout), || {
            let planes: Vec<wgpu::Texture> = frame
                .planes
                .iter()
                .map(|p| {
                    d.create_texture(&wgpu::TextureDescriptor {
                        label: Some("video plane"),
                        size: wgpu::Extent3d { width: p.width, height: p.height, depth_or_array_layers: 1 },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: fmt,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    })
                })
                .collect();
            let views: Vec<wgpu::TextureView> = planes.iter().map(|t| t.create_view(&Default::default())).collect();
            let uniforms = uniform_buffer(&d, std::mem::size_of::<Conv>(), "video conversion");
            let view = |i: usize| &views[i.min(views.len() - 1)];
            let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("video planes"),
                layout: &cp.bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(view(0)) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(view(1)) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(view(2)) },
                ],
            });
            PlaneSet { planes, uniforms, bind }
        });
        let bpt = fmt.block_copy_size(None).unwrap_or(1);
        for (t, p) in set.planes.iter().zip(&frame.planes) {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: t,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &p.data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(p.width * bpt),
                    rows_per_image: Some(p.height),
                },
                wgpu::Extent3d { width: p.width, height: p.height, depth_or_array_layers: 1 },
            );
        }
        let (w, h) = if it.quarter_turns % 2 == 1 { (frame.height, frame.width) } else { (frame.width, frame.height) };
        let shift = match frame.layout {
            PixelLayout::Yuv8(x, y) | PixelLayout::Yuv16(x, y) => [x, y],
            _ => [0, 0],
        };
        let store = color::default_transfer(working.space);
        let u = Conv {
            to_working: types::mat3(&color::convert(it.space, working.space)),
            size_out: [w, h],
            coded: [frame.width, frame.height],
            chroma_shift: shift,
            packing: rgba as u32,
            rotation: it.quarter_turns % 4,
            transfer: color::transfer_id(it.transfer),
            alpha_mode: match it.alpha {
                AlphaMode::Premultiplied => 1,
                AlphaMode::None => 2,
                _ => 0,
            },
            full_range: it.full_range as u32,
            deep: deep as u32,
            linear_light: working.linear as u32,
            store_transfer: color::transfer_id(if store == Transfer::Linear { Transfer::Srgb } else { store }),
            kr: it.matrix.0,
            kb: it.matrix.1,
            pad: [0; 4],
        };
        self.queue.write_buffer(&set.uniforms, 0, bytemuck::bytes_of(&u));
        let out = pool.get_as(&d, src_layout, resources::FORMAT, [w, h]);
        {
            let mut pass = begin(enc, &out.view);
            pass.set_pipeline(&cp.pipe);
            pass.set_bind_group(0, &set.bind, &[]);
            pass.draw(0..3, 0..1);
        }
        out
    }

    /// Records a blend of the two `frames` at phase `t` into a texture from `pool`: a warp along `flow`, or a mix
    /// without one.
    fn blend_pass(
        &mut self,
        enc: &mut wgpu::CommandEncoder,
        pool: &mut Pool,
        frames: [&Tex; 2],
        flow: Option<&wgpu::TextureView>,
        t: f32,
        src_layout: &wgpu::BindGroupLayout,
    ) -> Arc<Tex> {
        let kit = self.kit();
        let d = self.device.clone();
        let [a, b] = frames;
        let warp = flow.is_some();
        let flow = flow.unwrap_or(&self.no_flow);
        let out = pool.get_as(&d, src_layout, resources::FORMAT, a.size);
        let bgl0 = &kit.bgl0;
        let params = self.blends.take((), || Params::new(&d, bgl0));
        let p = FlowParams { size: a.size, grid: [0; 2], coarse: [0; 2], t, iterations: 0 };
        self.queue.write_buffer(&params.buf, 0, bytemuck::bytes_of(&p));
        let g3 = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("video blend"),
            layout: &kit.bgl3,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&a.view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&b.view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(flow) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.samp) },
            ],
        });
        {
            let mut pass = begin(enc, &out.view);
            pass.set_pipeline(if warp { &kit.warp } else { &kit.mix });
            pass.set_bind_group(0, &params.bind, &[]);
            pass.set_bind_group(3, &g3, &[]);
            pass.draw(0..3, 0..1);
        }
        out
    }

    /// Linear blend of two frames: `(1 − t)·a + t·b`, recorded into `enc`.
    pub fn mix(
        &mut self,
        enc: &mut wgpu::CommandEncoder,
        pool: &mut Pool,
        a: &Tex,
        b: &Tex,
        t: f32,
        src_layout: &wgpu::BindGroupLayout,
    ) -> Arc<Tex> {
        self.blend_pass(enc, pool, [a, b], None, t, src_layout)
    }

    /// Frame at phase `t` between `a` and `b`, warped along the optical flow, recorded into `enc`.
    pub fn interpolate(
        &mut self,
        enc: &mut wgpu::CommandEncoder,
        pool: &mut Pool,
        a: &Tex,
        b: &Tex,
        t: f32,
        src_layout: &wgpu::BindGroupLayout,
    ) -> Arc<Tex> {
        let flow = self.flow(enc, a, b).create_view(&Default::default());
        self.blend_pass(enc, pool, [a, b], Some(&flow), t, src_layout)
    }

    /// Records the dense flow from `a` to `b` at half resolution (pixels of that level) into `enc`. The texture
    /// returned belongs to a set kept by frame size: it holds its flow until the queue takes the frame and a later
    /// frame records into it again.
    pub fn flow(&mut self, enc: &mut wgpu::CommandEncoder, a: &Tex, b: &Tex) -> wgpu::Texture {
        let kit = self.kit();
        let d = self.device.clone();
        let (queue, bgl0, bgl1, bgl2) = (&self.queue, &kit.bgl0, &kit.bgl1, &kit.bgl2);
        let set = self.flows.take(a.size, || FlowSet::new(&d, queue, bgl0, bgl1, bgl2, a.size));
        // level 0 reduces the frames themselves; the levels below reduce the level above
        let g1 = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("reduce"),
            layout: bgl1,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&a.view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&b.view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&set.la[0]) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&set.lb[0]) },
            ],
        });
        for (l, &size) in set.sizes.iter().enumerate() {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(&kit.reduce);
            pass.set_bind_group(0, &set.reduce_params[l].bind, &[]);
            pass.set_bind_group(1, if l == 0 { &g1 } else { &set.reduce_binds[l - 1] }, &[]);
            pass.dispatch_workgroups(size[0].div_ceil(8), size[1].div_ceil(8), 1);
        }
        // coarse to fine
        for l in (0..set.sizes.len()).rev() {
            let size = set.sizes[l];
            let gs = flow_grid(size);
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_bind_group(0, &set.patch_params[l].bind, &[]);
            pass.set_bind_group(2, &set.patch_binds[l], &[]);
            pass.set_pipeline(&kit.patch);
            pass.dispatch_workgroups(gs[0].div_ceil(8), gs[1].div_ceil(8), 1);
            pass.set_pipeline(&kit.densify);
            pass.dispatch_workgroups(size[0].div_ceil(8), size[1].div_ceil(8), 1);
        }
        set.flows[0].clone()
    }

    /// Reads a flow texture back (x, y per pixel), for tests and diagnostics; the work that wrote it is submitted.
    pub fn read_flow(&self, t: &wgpu::Texture) -> (u32, u32, Vec<[f32; 2]>) {
        let (w, h) = (t.width(), t.height());
        let row = (w * 16).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: t,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        crate::gpu::map_read(&self.device, buf.slice(..))
            .unwrap_or_else(|e| panic!("reading back a {w}x{h} texture: {e}"));
        let data = buf.slice(..).get_mapped_range().expect("mapped");
        let mut out = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                let o = (y * row + x * 16) as usize;
                let f = |k: usize| f32::from_le_bytes([data[o + k], data[o + k + 1], data[o + k + 2], data[o + k + 3]]);
                out.push([f(0), f(4)]);
            }
        }
        (w, h, out)
    }
}

/// Resources of one kind kept between frames by a key (a size, or a size and layout): a frame takes a key's
/// entries in order, making one when none is left, so two uses of a key in one frame share nothing the queue has
/// not taken yet. Once the queue took the frame's work the entries are taken from the first again, and the entries
/// of a key no frame used for a while are dropped.
pub(crate) struct Ring<K, T> {
    entries: HashMap<K, RingEntry<T>>,
    /// Frames ended so far.
    frame: u64,
}

struct RingEntry<T> {
    items: Vec<Arc<T>>,
    /// Entries taken since the queue last took the work.
    used: usize,
    /// The frame the key was last taken in.
    wanted: u64,
}

/// Frames a ring keeps a key's entries after the key was last taken (the texture pool's span).
const RING_KEEP: u64 = 8;

impl<K: Copy + Eq + std::hash::Hash, T> Ring<K, T> {
    pub(crate) fn new() -> Ring<K, T> {
        Ring { entries: HashMap::new(), frame: 0 }
    }

    /// The next entry of `key`, made by `make` when the frame used every one there is.
    pub(crate) fn take(&mut self, key: K, make: impl FnOnce() -> T) -> Arc<T> {
        let e = self.entries.entry(key).or_insert_with(|| RingEntry { items: Vec::new(), used: 0, wanted: 0 });
        e.wanted = self.frame;
        if e.used == e.items.len() {
            e.items.push(Arc::new(make()));
        }
        e.used += 1;
        e.items[e.used - 1].clone()
    }

    pub(crate) fn rewind(&mut self) {
        for e in self.entries.values_mut() {
            e.used = 0;
        }
    }

    pub(crate) fn trim(&mut self) {
        self.frame += 1;
        let now = self.frame;
        self.entries.retain(|_, e| now - e.wanted <= RING_KEEP);
    }
}

/// The plane textures of one decoded size and layout, and the conversion's uniforms and bindings.
struct PlaneSet {
    planes: Vec<wgpu::Texture>,
    uniforms: wgpu::Buffer,
    bind: wgpu::BindGroup,
}

/// A [`FlowParams`] uniform buffer and its bind group.
struct Params {
    buf: wgpu::Buffer,
    bind: wgpu::BindGroup,
}

impl Params {
    fn new(d: &wgpu::Device, bgl0: &wgpu::BindGroupLayout) -> Params {
        let buf = uniform_buffer(d, std::mem::size_of::<FlowParams>(), "flow params");
        let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("flow params"),
            layout: bgl0,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }],
        });
        Params { buf, bind }
    }

    fn of(d: &wgpu::Device, q: &wgpu::Queue, bgl0: &wgpu::BindGroupLayout, p: FlowParams) -> Params {
        let params = Params::new(d, bgl0);
        q.write_buffer(&params.buf, 0, bytemuck::bytes_of(&p));
        params
    }
}

/// The patch grid of a flow level.
fn flow_grid(s: [u32; 2]) -> [u32; 2] {
    [(s[0].saturating_sub(8) / 4 + 1).max(1), (s[1].saturating_sub(8) / 4 + 1).max(1)]
}

/// The luma pyramids, flow levels, patch buffer and bindings of flows between frames of one size. Level 0 is at
/// half resolution; the bindings of level 0's reduction, which reads the frames, are made per flow.
struct FlowSet {
    sizes: Vec<[u32; 2]>,
    /// Luma of frame `a` and `b` per level.
    la: Vec<wgpu::TextureView>,
    lb: Vec<wgpu::TextureView>,
    /// Flow per level; the texture of level 0 is the result.
    flows: Vec<wgpu::Texture>,
    reduce_params: Vec<Params>,
    /// The reduction of level `l + 1` from level `l`.
    reduce_binds: Vec<wgpu::BindGroup>,
    patch_params: Vec<Params>,
    patch_binds: Vec<wgpu::BindGroup>,
}

impl FlowSet {
    fn new(
        d: &wgpu::Device,
        q: &wgpu::Queue,
        bgl0: &wgpu::BindGroupLayout,
        bgl1: &wgpu::BindGroupLayout,
        bgl2: &wgpu::BindGroupLayout,
        size: [u32; 2],
    ) -> FlowSet {
        let mut sizes = vec![[size[0].div_ceil(2), size[1].div_ceil(2)]];
        while sizes.len() < 6 && sizes.last().unwrap().iter().all(|&s| s >= 32) {
            let l = *sizes.last().unwrap();
            sizes.push([l[0].div_ceil(2), l[1].div_ceil(2)]);
        }
        let v = |t: &wgpu::Texture| t.create_view(&Default::default());
        let la: Vec<wgpu::TextureView> = sizes.iter().map(|s| v(&luma_texture(d, *s))).collect();
        let lb: Vec<wgpu::TextureView> = sizes.iter().map(|s| v(&luma_texture(d, *s))).collect();
        let flows: Vec<wgpu::Texture> = sizes.iter().map(|s| flow_texture(d, *s)).collect();
        let none = v(&flow_texture(d, [1, 1]));
        let g0 = flow_grid(sizes[0]);
        let patches = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("patches"),
            size: (g0[0] * g0[1] * 8) as u64,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let reduce_params = sizes
            .iter()
            .map(|&size| {
                Params::of(d, q, bgl0, FlowParams { size, grid: [0; 2], coarse: [0; 2], t: 0.0, iterations: 0 })
            })
            .collect();
        let reduce_binds = (1..sizes.len())
            .map(|l| {
                d.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("reduce"),
                    layout: bgl1,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&la[l - 1]) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&lb[l - 1]) },
                        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&la[l]) },
                        wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&lb[l]) },
                    ],
                })
            })
            .collect();
        let patch_params = (0..sizes.len())
            .map(|l| {
                let coarse = if l + 1 < sizes.len() { sizes[l + 1] } else { [0, 0] };
                Params::of(
                    d,
                    q,
                    bgl0,
                    FlowParams { size: sizes[l], grid: flow_grid(sizes[l]), coarse, t: 0.0, iterations: 12 },
                )
            })
            .collect();
        let flow_views: Vec<wgpu::TextureView> = flows.iter().map(v).collect();
        let patch_binds = (0..sizes.len())
            .map(|l| {
                let coarse = if l + 1 < sizes.len() { &flow_views[l + 1] } else { &none };
                d.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("patches"),
                    layout: bgl2,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&la[l]) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&lb[l]) },
                        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(coarse) },
                        wgpu::BindGroupEntry { binding: 3, resource: patches.as_entire_binding() },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: wgpu::BindingResource::TextureView(&flow_views[l]),
                        },
                    ],
                })
            })
            .collect();
        FlowSet { sizes, la, lb, flows, reduce_params, reduce_binds, patch_params, patch_binds }
    }
}

fn flow_texture(d: &wgpu::Device, size: [u32; 2]) -> wgpu::Texture {
    d.create_texture(&wgpu::TextureDescriptor {
        label: Some("flow"),
        size: wgpu::Extent3d { width: size[0].max(1), height: size[1].max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

fn luma_texture(d: &wgpu::Device, size: [u32; 2]) -> wgpu::Texture {
    d.create_texture(&wgpu::TextureDescriptor {
        label: Some("luma"),
        size: wgpu::Extent3d { width: size[0].max(1), height: size[1].max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
        view_formats: &[],
    })
}

/// A uniform buffer of `size` bytes the queue writes into.
fn uniform_buffer(d: &wgpu::Device, size: usize, label: &str) -> wgpu::Buffer {
    d.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn full_pipeline(
    d: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    fs: &str,
) -> wgpu::RenderPipeline {
    {
        let _creation = crate::gpu::creation_lock();
        d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(fs),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module,
                entry_point: Some("vs_full"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: Some(fs),
                targets: &[Some(wgpu::ColorTargetState {
                    format: resources::FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        })
    }
}

fn begin<'e>(enc: &'e mut wgpu::CommandEncoder, view: &'e wgpu::TextureView) -> wgpu::RenderPass<'e> {
    enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: None,
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

#[cfg(test)]
mod shared_pipeline_tests {
    use super::*;

    #[test]
    fn video_engines_on_one_device_share_pipelines_built_on_first_use() {
        let gpu = sr_testkit::require_gpu!(
            crate::gpu::test_gpu(),
            "video_engines_on_one_device_share_pipelines_built_on_first_use"
        );
        let a = VideoEngine::new(gpu.device.clone(), gpu.queue.clone());
        let b = VideoEngine::new(gpu.device.clone(), gpu.queue.clone());
        assert!(a.kit.get().is_none(), "an engine that converts nothing compiles nothing");
        assert!(Arc::ptr_eq(&a.kit(), &b.kit()), "one set per device");
        assert_eq!(a.kit().mix, b.kit().mix);
    }
}

#[cfg(test)]
mod wgsl_layout_tests {
    //! The Rust structs bound to WGSL structs have their layout (`crate::wgsl_layout`).
    use crate::wgsl_layout::{assert_agree, rust_fields};

    #[test]
    fn conv_matches_wgsl_conv() {
        let fields = rust_fields!(super::Conv; to_working, size_out, coded, chroma_shift, packing, rotation, transfer, alpha_mode, full_range, deep, linear_light, store_transfer, kr, kb, pad);
        assert_agree(include_str!("video.wgsl"), "Conv", std::mem::size_of::<super::Conv>(), &fields);
    }

    #[test]
    fn flow_params_matches_wgsl_flow_params() {
        let fields = rust_fields!(super::FlowParams; size, grid, coarse, t, iterations);
        assert_agree(include_str!("flow.wgsl"), "FlowParams", std::mem::size_of::<super::FlowParams>(), &fields);
    }
}
