//! Delivery conversion on the GPU: the working-space frame becomes the
//! bytes an encoder reads (NV12, P010, RGBA8, RGBA16 or planar float), in
//! the output colour space, transfer, matrix and range, scaled to the
//! output size with letterboxing, then read back through a staging ring so
//! the next frame renders and converts while this one maps.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use sr_media::encode::InputFormat;
use sr_model::model::{ColorSpace, Transfer};

use crate::color::{self, Working};
use crate::resources::Tex;
use crate::types;

/// Checks a frame's dimensions before conversion or texture allocation.
pub fn frame_size(size: [f64; 2], limits: &wgpu::Limits) -> Result<[u32; 2], String> {
    let limit = limits.max_texture_dimension_2d as f64;
    if size.iter().any(|v| !v.is_finite() || *v <= 0.0 || v.round() > limit) {
        return Err(format!(
            "frame dimensions {}x{} must be finite, positive and at most {} per axis",
            size[0], size[1], limits.max_texture_dimension_2d
        ));
    }
    Ok(size.map(|v| v.round().max(1.0) as u32))
}

/// The colour of a deliverable.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputColor {
    /// Primaries.
    pub space: ColorSpace,
    /// Transfer (resolved).
    pub transfer: Transfer,
    /// YUV matrix (Kr, Kb).
    pub matrix: (f32, f32),
    /// Full-range codes.
    pub full_range: bool,
}

impl OutputColor {
    /// Output colour of `space`/`transfer` attributes (`auto` resolves by space).
    pub fn new(space: ColorSpace, transfer: Transfer, full_range: bool) -> OutputColor {
        let transfer = color::resolve(space, transfer);
        let matrix = if space == ColorSpace::Rec2020 { (0.2627, 0.0593) } else { (0.2126, 0.0722) };
        OutputColor { space, transfer, matrix, full_range }
    }

    /// FFmpeg colour tags: primaries, transfer, matrix.
    pub fn ffmpeg_tags(&self) -> (String, String, String) {
        let prim = match self.space {
            ColorSpace::DisplayP3 => "smpte432",
            ColorSpace::DciP3 => "smpte431",
            ColorSpace::Rec2020 => "bt2020",
            _ => "bt709",
        };
        let trc = match self.transfer {
            Transfer::Srgb => "iec61966-2-1",
            Transfer::Pq => "smpte2084",
            Transfer::Hlg => "arib-std-b67",
            Transfer::Linear => "linear",
            Transfer::Gamma22 => "gamma22",
            Transfer::Gamma26 => "unknown",
            _ => "bt709",
        };
        let mat = if self.space == ColorSpace::Rec2020 { "bt2020nc" } else { "bt709" };
        (prim.into(), trc.into(), mat.into())
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Params {
    to_out: [[f32; 4]; 3],
    fsize: [u32; 2],
    osize: [u32; 2],
    offset: [f32; 2],
    scale: f32,
    format: u32,
    transfer: u32,
    full_range: u32,
    linear_light: u32,
    store_transfer: u32,
    keep_alpha: u32,
    kr: f32,
    kb: f32,
    words: u32,
    seed: u32,
    mode: u32,
    bg_offset: [f32; 2],
    bg_scale: f32,
    overlay: u32,
    /// First word of the piece being packed; `words` is one past its last.
    base: u32,
    pad: u32,
}

/// How a frame is placed in an output of another shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Placement {
    /// Scaled to fit inside, transparent (black when opaque) around it.
    Fit { focus: [f64; 2] },
    /// Scaled to cover, the excess cut away.
    Crop { focus: [f64; 2] },
    /// Scaled to fit inside, over a blurred copy scaled to cover.
    FitBlur { focus: [f64; 2] },
}

impl Default for Placement {
    fn default() -> Placement {
        Placement::Fit { focus: [0.5, 0.5] }
    }
}

impl Placement {
    /// Scale and offset (output pixels) of a `frame`-sized image placed in `size`: `cover` or fit, with
    /// the free space shared out by `focus` (0 puts the image's left or top edge on the output's).
    fn place(frame: [f32; 2], size: [u32; 2], cover: bool, focus: [f64; 2]) -> (f32, [f32; 2]) {
        let (sx, sy) = (size[0] as f32 / frame[0], size[1] as f32 / frame[1]);
        let scale = if cover { sx.max(sy) } else { sx.min(sy) };
        let free = [size[0] as f32 - frame[0] * scale, size[1] as f32 - frame[1] * scale];
        (scale, [free[0] * focus[0] as f32, free[1] * focus[1] as f32])
    }
}

/// A conversion in flight: its staging buffers (out of the ring until [`OutputStage::wait`]
/// returns them; one unless the frame is larger than a buffer), the last submission that fills
/// them, and their pending mappings.
pub struct Pending {
    slot: usize,
    buffers: Vec<wgpu::Buffer>,
    bytes: usize,
    submitted: wgpu::SubmissionIndex,
    ready: Vec<crate::gpu::MapWait>,
}

/// The pack shader's number for an input format (`Out.format` in output.wgsl).
fn format_id(format: InputFormat) -> u32 {
    match format {
        InputFormat::Nv12 => 0,
        InputFormat::P010 => 1,
        InputFormat::Rgba8 => 2,
        InputFormat::Rgba16 => 3,
        InputFormat::Gbrapf32 => 4,
        InputFormat::Yuv420p => 5,
        InputFormat::Yuv420p10 => 6,
        InputFormat::Yuv422p => 7,
        InputFormat::Yuv422p10 => 8,
        InputFormat::Yuv444p => 9,
        InputFormat::Yuv444p10 => 10,
        InputFormat::Yuva444p10 => 11,
    }
}

/// Staging buffers in the ring: one mapping, one converting, one spare.
const RING: usize = 3;

/// The most dropped frame buffers a stage keeps for the frames after them.
const FREE_FRAMES: usize = 4;

/// Frame buffers that read back frames return to when they are dropped, for the next [`OutputStage::wait`] to refill.
#[derive(Clone, Default)]
struct FramePool {
    free: Arc<std::sync::Mutex<Vec<Vec<u8>>>>,
    allocated: Arc<std::sync::atomic::AtomicUsize>,
}

impl FramePool {
    /// An empty buffer holding at least `bytes`: a returned one when there is one.
    fn take(&self, bytes: usize) -> Vec<u8> {
        let returned = self.free.lock().unwrap_or_else(|e| e.into_inner()).pop();
        match returned {
            Some(mut v) if v.capacity() >= bytes => {
                v.clear();
                v
            }
            _ => {
                self.allocated.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Vec::with_capacity(bytes)
            }
        }
    }

    fn give(&self, v: Vec<u8>) {
        let mut free = self.free.lock().unwrap_or_else(|e| e.into_inner());
        if free.len() < FREE_FRAMES && v.capacity() > 0 {
            free.push(v);
        }
    }
}

/// A read back frame's bytes. Dropped, on whatever thread, its buffer returns to the stage that filled it.
pub struct FrameBytes {
    bytes: Vec<u8>,
    pool: FramePool,
}

impl FrameBytes {
    /// The bytes, kept from the stage's reuse.
    pub fn into_vec(mut self) -> Vec<u8> {
        std::mem::take(&mut self.bytes)
    }
}

impl From<Vec<u8>> for FrameBytes {
    /// Bytes of no stage: dropped, they are freed.
    fn from(bytes: Vec<u8>) -> FrameBytes {
        FrameBytes { bytes, pool: FramePool::default() }
    }
}

impl std::ops::Deref for FrameBytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.bytes
    }
}

impl Drop for FrameBytes {
    fn drop(&mut self) {
        self.pool.give(std::mem::take(&mut self.bytes));
    }
}

impl PartialEq for FrameBytes {
    fn eq(&self, other: &FrameBytes) -> bool {
        self.bytes == other.bytes
    }
}

impl std::fmt::Debug for FrameBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.bytes.fmt(f)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PackKey {
    format: u32,
    transfer: u32,
    linear_light: u32,
    store_transfer: u32,
    blocks: bool,
    /// The frame sits unmoved and unscaled over the whole output, so a pixel samples itself.
    whole: bool,
}

impl PackKey {
    fn of(p: &Params, blocks: bool) -> Self {
        Self {
            format: p.format,
            transfer: p.transfer,
            linear_light: p.linear_light,
            store_transfer: p.store_transfer,
            blocks,
            whole: p.offset == [0.0; 2] && p.scale == 1.0 && p.osize[0] <= p.fsize[0] && p.osize[1] <= p.fsize[1],
        }
    }
}

/// The converter and its staging ring.
pub struct OutputStage {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    pack_module: wgpu::ShaderModule,
    pack_layout: wgpu::PipelineLayout,
    /// Most recently used colour/format programs, capped at eight configurations.
    pack_pipes: std::collections::VecDeque<(PackKey, wgpu::ComputePipeline)>,
    bgl: wgpu::BindGroupLayout,
    /// Placement into a working-space texture, without conversion (transitions between outputs' frames).
    place_pipe: std::sync::OnceLock<wgpu::ComputePipeline>,
    place_layout: wgpu::PipelineLayout,
    place_bgl: wgpu::BindGroupLayout,
    samp: wgpu::Sampler,
    /// Parameters, rewritten before every submission (queue writes land after earlier work).
    params: wgpu::Buffer,
    /// The fit-blur background: downsample and Gaussian passes, their direction uniforms (down,
    /// across, along), the two reduced textures they ping-pong between, and a 1×1 stand-in.
    blur_pipe: std::sync::OnceLock<wgpu::ComputePipeline>,
    blur_module: wgpu::ShaderModule,
    blur_layout: wgpu::PipelineLayout,
    blur_bgl: wgpu::BindGroupLayout,
    blur_dirs: [wgpu::Buffer; 3],
    blur_tex: Option<([u32; 2], [wgpu::TextureView; 2])>,
    none_view: wgpu::TextureView,
    /// Packed output, grown when a larger frame needs it.
    packed: Option<wgpu::Buffer>,
    /// The most bytes one packed or staging buffer holds: the device's storage binding and
    /// buffer limits. A larger frame is packed and read back in pieces of this size.
    piece: u64,
    /// Staging buffers by slot; a slot is empty while its buffers are out in a [`Pending`].
    ring: [Option<Vec<wgpu::Buffer>>; RING],
    /// The slot the next submission takes.
    head: usize,
    frame_index: u32,
    /// Buffers of read back frames, back from their consumers.
    frames: FramePool,
}

impl OutputStage {
    /// Builds the stage; its pipelines are compiled as frames first need them.
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> OutputStage {
        let d = &*device;
        let limits = d.limits();
        let src = format!("{}\n{}", include_str!("transfer.wgsl"), include_str!("output.wgsl"));
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("output"),
            source: wgpu::ShaderSource::Wgsl(src.into()),
        });
        let c = wgpu::ShaderStages::COMPUTE;
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: c,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("output"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: c,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: c,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: c,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture(4),
                texture(5),
            ],
        });
        let place_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("output place"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: c,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: c,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                texture(4),
                texture(5),
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: c,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let blur_module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("output blur"),
            source: wgpu::ShaderSource::Wgsl(include_str!("output_blur.wgsl").into()),
        });
        let blur_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("output blur"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: c,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: c,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: c,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let blur_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&blur_bgl)],
            immediate_size: 0,
        });
        let blur_dirs = [[0i32, 0, 0, 0], [1, 0, 0, 0], [0, 1, 0, 0]].map(|v| {
            let b = d.create_buffer(&wgpu::BufferDescriptor {
                label: Some("output blur direction"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&b, 0, bytemuck::cast_slice(&v));
            b
        });
        let none_view = d
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("no blur"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let place_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&place_bgl)],
            immediate_size: 0,
        });
        let samp = d.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let params = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("output params"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        OutputStage {
            device,
            queue,
            pack_module: module,
            pack_layout: layout,
            pack_pipes: Default::default(),
            bgl,
            place_pipe: Default::default(),
            place_layout,
            place_bgl,
            samp,
            params,
            blur_pipe: Default::default(),
            blur_module,
            blur_layout,
            blur_bgl,
            blur_dirs,
            blur_tex: None,
            none_view,
            packed: None,
            piece: limits.max_storage_buffer_binding_size.min(limits.max_buffer_size),
            ring: [None, None, None],
            head: 0,
            frame_index: 0,
            frames: FramePool::default(),
        }
    }

    /// Placement without conversion, compiled when a transition first needs it.
    fn place_pipeline(&self) -> &wgpu::ComputePipeline {
        self.place_pipe.get_or_init(|| {
            let _creation = crate::gpu::creation_lock();
            self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("place"),
                layout: Some(&self.place_layout),
                module: &self.pack_module,
                entry_point: Some("cs_place"),
                compilation_options: Default::default(),
                cache: None,
            })
        })
    }

    /// The fit-blur pass, compiled when a frame first needs a blurred background.
    fn blur_pipeline(&self) -> &wgpu::ComputePipeline {
        self.blur_pipe.get_or_init(|| {
            let _creation = crate::gpu::creation_lock();
            self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("output blur"),
                layout: Some(&self.blur_layout),
                module: &self.blur_module,
                entry_point: Some("cs_blur"),
                compilation_options: Default::default(),
                cache: None,
            })
        })
    }

    fn compile_pack(&self, blocks: bool, constants: &[(&str, f64)]) -> wgpu::ComputePipeline {
        {
            let _creation = crate::gpu::creation_lock();
            self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("pack output"),
                layout: Some(&self.pack_layout),
                module: &self.pack_module,
                entry_point: Some(if blocks { "cs_pack_nv12_blocks" } else { "cs_pack" }),
                compilation_options: wgpu::PipelineCompilationOptions { constants, ..Default::default() },
                cache: None,
            })
        }
    }

    fn pack_pipeline(&mut self, key: PackKey) -> wgpu::ComputePipeline {
        if let Some(at) = self.pack_pipes.iter().position(|(k, _)| *k == key) {
            let item = self.pack_pipes.remove(at).expect("found");
            let pipe = item.1.clone();
            self.pack_pipes.push_back(item);
            return pipe;
        }
        let pipe = self.compile_pack(
            key.blocks,
            &[
                ("OUTPUT_FORMAT", key.format as f64),
                ("OUTPUT_TRANSFER", key.transfer as f64),
                ("WORKING_LINEAR", key.linear_light as f64),
                ("WORKING_TRANSFER", key.store_transfer as f64),
                ("PLACED_WHOLE", key.whole as u32 as f64),
            ],
        );
        if self.pack_pipes.len() == 8 {
            self.pack_pipes.pop_front();
        }
        self.pack_pipes.push_back((key, pipe.clone()));
        pipe
    }

    /// Makes the next submitted frame frame number `frame` of the programme: it seeds that
    /// frame's dither, so a frame converts the same whichever frame a render started from.
    pub fn seek(&mut self, frame: u32) {
        self.frame_index = frame;
    }

    /// Lowers the size of the buffers frames are packed and read back in (never above the
    /// device's limits); frames larger than `bytes` go through in pieces.
    pub fn limit_buffers(&mut self, bytes: u64) {
        self.piece = self.piece.min(bytes);
        self.packed = None;
    }

    /// Starts converting `frame` to `size` in `format`; returns a pending readback.
    pub fn submit(
        &mut self,
        frame: &Tex,
        working: &Working,
        out: &OutputColor,
        format: InputFormat,
        size: [u32; 2],
        keep_alpha: bool,
    ) -> Pending {
        self.submit_placed(frame, working, out, format, size, keep_alpha, Placement::default(), None)
    }

    /// [`submit`](Self::submit), with the frame placed in the output by `placement` and an
    /// output-sized `overlay` (working representation, premultiplied) composited over it.
    #[allow(clippy::too_many_arguments)]
    pub fn submit_placed(
        &mut self,
        frame: &Tex,
        working: &Working,
        out: &OutputColor,
        format: InputFormat,
        size: [u32; 2],
        keep_alpha: bool,
        placement: Placement,
        overlay: Option<&Tex>,
    ) -> Pending {
        let d = self.device.clone();
        let bytes = format.frame_bytes(size[0], size[1]);
        let words = (bytes.div_ceil(4) as u32).max(1);
        // (first word, words) of each piece: one, unless the frame is larger than a buffer
        let per = (self.piece / 4).clamp(1, u32::MAX as u64) as u32;
        let pieces: Vec<(u32, u32)> = (0..words.div_ceil(per)).map(|k| (k * per, per.min(words - k * per))).collect();
        let mut p = Self::params(frame, working, out, size, placement);
        p.format = format_id(format);
        p.keep_alpha = keep_alpha as u32;
        p.seed = self.frame_index;
        if let Some(o) = overlay {
            assert_eq!(o.size, size, "the overlay is output-sized");
            p.overlay = 1;
        }
        let blocks = p.format == 0 && size[0].is_multiple_of(4) && size[1].is_multiple_of(2) && pieces.len() == 1;
        let pipe = self.pack_pipeline(PackKey::of(&p, blocks));
        let mode = p.mode;
        self.frame_index = self.frame_index.wrapping_add(1);
        let mut enc = d.create_command_encoder(&Default::default());
        if mode == 1 {
            self.blur(&mut enc, frame);
        }
        let size_bytes = (pieces[0].1 as u64) * 4;
        // The shader stops at `words`, so a packed buffer left over from a larger frame is fine.
        if self.packed.as_ref().is_none_or(|b| b.size() < size_bytes) {
            self.packed = Some(d.create_buffer(&wgpu::BufferDescriptor {
                label: Some("packed"),
                size: size_bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }));
        }
        let storage = self.packed.as_ref().expect("packed buffer");
        // Round-robin through the ring. A slot is only empty while its buffers are out in a
        // `Pending` (still mapping, or dropped unwaited), so a buffer is never handed out
        // twice; an empty or wrongly sized slot gets fresh buffers.
        let slot = self.head;
        self.head = (self.head + 1) % RING;
        let fits = |b: &Vec<wgpu::Buffer>| {
            b.len() == pieces.len() && b.iter().zip(&pieces).all(|(b, (_, n))| b.size() == *n as u64 * 4)
        };
        let staging = match self.ring[slot].take() {
            Some(b) if fits(&b) => b,
            _ => pieces
                .iter()
                .map(|(_, n)| {
                    d.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("staging"),
                        size: *n as u64 * 4,
                        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                        mapped_at_creation: false,
                    })
                })
                .collect(),
        };
        let blurred = match (&self.blur_tex, mode) {
            (Some((_, v)), 1) => &v[0],
            _ => &self.none_view,
        };
        let bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&frame.view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.samp) },
                wgpu::BindGroupEntry { binding: 3, resource: storage.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(blurred) },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(overlay.map_or(&self.none_view, |o| &o.view)),
                },
            ],
        });
        let mut ready = Vec::with_capacity(pieces.len());
        let mut enc = Some(enc);
        let mut submitted = None;
        for ((base, n), staging) in pieces.iter().zip(&staging) {
            p.base = *base;
            p.words = base + n;
            // The queue orders this write after every earlier submission, so the previous
            // piece's (or frame's) pack pass reads its own parameters before these land.
            self.queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&p));
            let mut enc = enc.take().unwrap_or_else(|| d.create_command_encoder(&Default::default()));
            {
                let mut pass = enc.begin_compute_pass(&Default::default());
                // A 4×2 block owns two complete Y words and one UV word. Odd
                // dimensions and split readbacks retain the word-at-a-time path.
                pass.set_pipeline(&pipe);
                pass.set_bind_group(0, &bg, &[]);
                let invocations = if blocks { size[0] * size[1] / 8 } else { *n };
                let groups = invocations.div_ceil(64);
                let (x, y) = if groups > 65535 { (65535, groups.div_ceil(65535)) } else { (groups, 1) };
                pass.dispatch_workgroups(x, y, 1);
            }
            enc.copy_buffer_to_buffer(storage, 0, staging, 0, *n as u64 * 4);
            submitted = Some(self.queue.submit([enc.finish()]));
            ready.push(crate::gpu::begin_map_read(staging.slice(..)));
        }
        Pending { slot, buffers: staging, bytes, submitted: submitted.expect("a frame has a piece"), ready }
    }

    /// Conversion parameters for `frame` placed in `size`; the caller sets the packing fields.
    fn params(frame: &Tex, working: &Working, out: &OutputColor, size: [u32; 2], placement: Placement) -> Params {
        let fsz = [frame.size[0] as f32, frame.size[1] as f32];
        let (mode, (scale, offset), (bg_scale, bg_offset)) = match placement {
            Placement::Fit { focus } => (0, Placement::place(fsz, size, false, focus), (1.0, [0.0; 2])),
            Placement::Crop { focus } => (0, Placement::place(fsz, size, true, focus), (1.0, [0.0; 2])),
            Placement::FitBlur { focus } => {
                (1, Placement::place(fsz, size, false, focus), Placement::place(fsz, size, true, focus))
            }
        };
        let store = color::default_transfer(working.space);
        Params {
            to_out: types::mat3(&color::convert(working.space, out.space)),
            fsize: frame.size,
            osize: size,
            offset,
            scale,
            format: 0,
            transfer: color::transfer_id(out.transfer),
            full_range: out.full_range as u32,
            linear_light: working.linear as u32,
            store_transfer: color::transfer_id(if store == Transfer::Linear { Transfer::Srgb } else { store }),
            keep_alpha: 0,
            kr: out.matrix.0,
            kb: out.matrix.1,
            words: 0,
            seed: 0,
            base: 0,
            mode,
            bg_offset,
            bg_scale,
            overlay: 0,
            pad: 0,
        }
    }

    /// Records the fit-blur background of `frame`: reduced 8× and blurred, down into [0], across into
    /// [1], down again into [0].
    fn blur(&mut self, enc: &mut wgpu::CommandEncoder, frame: &Tex) {
        let d = &*self.device;
        let small = [frame.size[0].div_ceil(8).max(1), frame.size[1].div_ceil(8).max(1)];
        if self.blur_tex.as_ref().is_none_or(|(s, _)| *s != small) {
            let make = || {
                d.create_texture(&wgpu::TextureDescriptor {
                    label: Some("fit-blur"),
                    size: wgpu::Extent3d { width: small[0], height: small[1], depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba16Float,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
            };
            self.blur_tex = Some((small, [make(), make()]));
        }
        let (_, views) = self.blur_tex.as_ref().expect("blur textures");
        let steps: [(&wgpu::TextureView, &wgpu::TextureView, usize); 3] =
            [(&frame.view, &views[0], 0), (&views[0], &views[1], 1), (&views[1], &views[0], 2)];
        for (src, dst, k) in steps {
            let g = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.blur_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(src) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(dst) },
                    wgpu::BindGroupEntry { binding: 2, resource: self.blur_dirs[k].as_entire_binding() },
                ],
            });
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(self.blur_pipeline());
            pass.set_bind_group(0, &g, &[]);
            pass.dispatch_workgroups(small[0].div_ceil(8), small[1].div_ceil(8), 1);
        }
    }

    /// Places `frame` in `into` (the output's size) as `placement` does, in the working
    /// representation, without converting it: both sides of a transition between an output's
    /// segments are placed with their own focus before they are combined.
    pub fn place(&mut self, frame: &Tex, working: &Working, into: &Tex, placement: Placement) {
        self.place_with_overlay(frame, working, into, placement, None);
    }

    /// Places and composites the picture exactly as delivery does, before output encoding.
    /// Accessibility checks use this to inspect the output's frame, including overlays and captions.
    pub fn place_with_overlay(
        &mut self,
        frame: &Tex,
        working: &Working,
        into: &Tex,
        placement: Placement,
        overlay: Option<&Tex>,
    ) {
        let d = self.device.clone();
        let same = OutputColor::new(working.space, Transfer::Linear, true);
        let mut p = Self::params(frame, working, &same, into.size, placement);
        if let Some(o) = overlay {
            assert_eq!(o.size, into.size, "the overlay is output-sized");
            p.overlay = 1;
        }
        self.queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&p));
        let mut enc = d.create_command_encoder(&Default::default());
        if p.mode == 1 {
            self.blur(&mut enc, frame);
        }
        let blurred = match (&self.blur_tex, p.mode) {
            (Some((_, v)), 1) => &v[0],
            _ => &self.none_view,
        };
        let bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.place_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&frame.view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.samp) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(blurred) },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(overlay.map_or(&self.none_view, |o| &o.view)),
                },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&into.view) },
            ],
        });
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(self.place_pipeline());
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups(into.size[0].div_ceil(8), into.size[1].div_ceil(8), 1);
        }
        self.queue.submit([enc.finish()]);
    }

    /// Waits for a conversion and returns its bytes; the staging buffer returns to the ring.
    ///
    /// Only this conversion's submission is waited for, so work submitted after it (the
    /// next frame's render and pack) keeps running on the GPU meanwhile. The bytes fill a buffer an
    /// earlier frame returned when dropped, if one is free.
    pub fn wait(&mut self, p: Pending) -> FrameBytes {
        let Pending { slot, buffers, bytes, submitted, ready } = p;
        let _ = self.device.poll(wgpu::PollType::Wait { submission_index: Some(submitted), timeout: None });
        for map in ready {
            map.wait(&self.device, crate::gpu::MAP_READ_TIMEOUT)
                .unwrap_or_else(|e| panic!("reading back a converted frame: {e}"));
        }
        let mut out = self.frames.take(bytes);
        for b in &buffers {
            let view = b.slice(..).get_mapped_range().expect("mapped");
            out.extend_from_slice(&view[..view.len().min(bytes - out.len())]);
            drop(view);
            b.unmap();
        }
        self.ring[slot] = Some(buffers);
        FrameBytes { bytes: out, pool: self.frames.clone() }
    }

    /// Frame buffers [`OutputStage::wait`] has allocated: the rest of its frames reused one an earlier frame returned.
    pub fn frames_allocated(&self) -> usize {
        self.frames.allocated.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a6_a_fitted_frame_shares_the_free_space_by_the_focus() {
        // 64 x 36 into 36 x 64: scale 0.5625, a 36 x 20.25 band with 43.75 rows free
        let frame = [64.0, 36.0];
        for (focus, top) in [([0.5, 0.5], 21.875), ([0.0, 0.0], 0.0), ([0.0, 1.0], 43.75), ([1.0, 0.25], 10.9375)] {
            let (scale, offset) = Placement::place(frame, [36, 64], false, focus);
            assert!((scale - 0.5625).abs() < 1e-6 && offset[0].abs() < 1e-4, "{focus:?}: {scale} {offset:?}");
            assert!((offset[1] - top).abs() < 1e-4, "{focus:?}: {offset:?}");
        }
    }

    #[test]
    fn only_an_unmoved_unscaled_frame_takes_the_whole_frame_path() {
        let gpu = sr_testkit::require_gpu!(
            crate::gpu::test_gpu(),
            "only_an_unmoved_unscaled_frame_takes_the_whole_frame_path"
        );
        let layout = crate::resources::source_layout(&gpu.device);
        let frame = crate::resources::create(&gpu.device, &layout, [8, 6], 1, "whole-frame key");
        let working = Working { space: ColorSpace::Srgb, linear: false };
        let out = OutputColor::new(ColorSpace::Rec709, Transfer::Auto, false);
        for (size, placement, whole) in [
            ([8, 6], Placement::default(), true),
            ([8, 6], Placement::Crop { focus: [0.75, 0.25] }, true),
            ([8, 6], Placement::FitBlur { focus: [0.5, 0.5] }, true),
            ([12, 6], Placement::FitBlur { focus: [0.25, 0.75] }, false),
            ([4, 6], Placement::Crop { focus: [0.5, 0.5] }, false),
            ([4, 2], Placement::Crop { focus: [0.75, 0.25] }, false),
            ([5, 3], Placement::default(), false),
        ] {
            let params = OutputStage::params(&frame, &working, &out, size, placement);
            assert_eq!(PackKey::of(&params, true).whole, whole, "size={size:?} placement={placement:?}");
        }
    }

    #[test]
    fn nv12_blocks_match_piecewise_packing_byte_for_byte() {
        let gpu = sr_testkit::require_gpu!(crate::gpu::test_gpu(), "nv12_blocks_match_piecewise_packing_byte_for_byte");
        let layout = crate::resources::source_layout(&gpu.device);
        let frame = crate::resources::create(&gpu.device, &layout, [8, 6], 1, "NV12 oracle");
        let bytes: Vec<u8> = (0..48)
            .flat_map(|i| {
                let alpha = (i % 7) as f32 / 6.0;
                [i as f32 / 20.0 * alpha, (i % 9) as f32 / 8.0 * alpha, 0.7 * alpha, alpha]
                    .into_iter()
                    .flat_map(|v| half::f16::from_f32(v).to_le_bytes())
            })
            .collect();
        gpu.queue.write_texture(
            frame.tex.as_image_copy(),
            &bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(64), rows_per_image: Some(6) },
            wgpu::Extent3d { width: 8, height: 6, depth_or_array_layers: 1 },
        );
        let mut block = OutputStage::new(gpu.device.clone(), gpu.queue.clone());
        let mut piecewise = OutputStage::new(gpu.device.clone(), gpu.queue.clone());
        piecewise.limit_buffers(4);
        let generic = piecewise.compile_pack(false, &[]);
        let force_general = |stage: &mut OutputStage, params: &Params| {
            stage.pack_pipes.clear();
            stage.pack_pipes.push_back((PackKey::of(params, false), generic.clone()));
        };
        for linear in [false, true] {
            let working = Working { space: ColorSpace::Srgb, linear };
            for (size, placement) in [
                ([8, 6], Placement::default()),
                ([12, 6], Placement::FitBlur { focus: [0.25, 0.75] }),
                ([4, 2], Placement::Crop { focus: [0.75, 0.25] }),
                ([5, 3], Placement::default()),
            ] {
                for full_range in [false, true] {
                    let out = OutputColor::new(ColorSpace::Rec709, Transfer::Auto, full_range);
                    for stage in [&mut block, &mut piecewise] {
                        stage.seek(137);
                    }
                    force_general(&mut piecewise, &OutputStage::params(&frame, &working, &out, size, placement));
                    let a =
                        block.submit_placed(&frame, &working, &out, InputFormat::Nv12, size, false, placement, None);
                    let b = piecewise.submit_placed(
                        &frame,
                        &working,
                        &out,
                        InputFormat::Nv12,
                        size,
                        false,
                        placement,
                        None,
                    );
                    assert_eq!(block.wait(a), piecewise.wait(b), "size={size:?} linear={linear} full={full_range}");
                }
            }
        }
        // Overlay composition must be shared by luma and chroma too.
        let working = Working { space: ColorSpace::Rec2020, linear: true };
        let out = OutputColor::new(ColorSpace::Rec2020, Transfer::Pq, false);
        for stage in [&mut block, &mut piecewise] {
            stage.seek(u32::MAX);
        }
        force_general(&mut piecewise, &OutputStage::params(&frame, &working, &out, [8, 6], Placement::default()));
        let a = block.submit_placed(
            &frame,
            &working,
            &out,
            InputFormat::Nv12,
            [8, 6],
            false,
            Placement::default(),
            Some(&frame),
        );
        let b = piecewise.submit_placed(
            &frame,
            &working,
            &out,
            InputFormat::Nv12,
            [8, 6],
            false,
            Placement::default(),
            Some(&frame),
        );
        assert_eq!(block.wait(a), piecewise.wait(b));
        // Changing colour/format configuration on one stage must not reuse a
        // conversion for the previous output. The split path is the reference.
        for linear in [false, true] {
            let working = Working { space: ColorSpace::Srgb, linear };
            for transfer in [Transfer::Srgb, Transfer::Bt1886, Transfer::Pq, Transfer::Hlg] {
                let out = OutputColor::new(ColorSpace::Rec2020, transfer, false);
                for format in [
                    InputFormat::Nv12,
                    InputFormat::P010,
                    InputFormat::Rgba8,
                    InputFormat::Rgba16,
                    InputFormat::Gbrapf32,
                    InputFormat::Yuv420p,
                    InputFormat::Yuv420p10,
                    InputFormat::Yuv422p,
                    InputFormat::Yuv422p10,
                    InputFormat::Yuv444p,
                    InputFormat::Yuv444p10,
                    InputFormat::Yuva444p10,
                ] {
                    for keep_alpha in [false, true] {
                        for stage in [&mut block, &mut piecewise] {
                            stage.seek(911);
                        }
                        let mut params = OutputStage::params(&frame, &working, &out, [8, 6], Placement::default());
                        params.format = format_id(format);
                        force_general(&mut piecewise, &params);
                        let a = block.submit(&frame, &working, &out, format, [8, 6], keep_alpha);
                        let b = piecewise.submit(&frame, &working, &out, format, [8, 6], keep_alpha);
                        assert!(block.pack_pipes.len() <= 8);
                        assert_eq!(
                            block.wait(a),
                            piecewise.wait(b),
                            "linear={linear} transfer={transfer:?} format={format:?} alpha={keep_alpha}"
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod planar_tests {
    use super::*;

    fn words(bytes: &[u8]) -> Vec<u16> {
        bytes.as_chunks::<2>().0.iter().map(|p| u16::from_le_bytes(*p)).collect()
    }

    /// A code at `bits` as output.wgsl's `code` makes it, from a luma or chroma value.
    fn code(v: f64, luma: bool, bits: u32, full_range: bool) -> i64 {
        let m = (1u32 << (bits - 8)) as f64;
        let x = if full_range {
            (if luma { v } else { v + 0.5 }) * 255.0 * m
        } else if luma {
            (16.0 + 219.0 * v) * m
        } else {
            (128.0 + 224.0 * v) * m
        };
        (x + 0.5).floor().clamp(0.0, 255.0 * m + (m - 1.0)) as i64
    }

    #[test]
    fn planar_packs_agree_with_the_interleaved_and_rgb_packs() {
        let gpu =
            sr_testkit::require_gpu!(crate::gpu::test_gpu(), "planar_packs_agree_with_the_interleaved_and_rgb_packs");
        let layout = crate::resources::source_layout(&gpu.device);
        let (w, h) = (8usize, 6usize);
        let frame = crate::resources::create(&gpu.device, &layout, [w as u32, h as u32], 1, "planar oracle");
        let alphas: Vec<f32> = (0..w * h).map(|i| half::f16::from_f32((i % 7) as f32 / 6.0).to_f32()).collect();
        let bytes: Vec<u8> = (0..w * h)
            .flat_map(|i| {
                let alpha = alphas[i];
                [i as f32 / 20.0 * alpha, (i % 9) as f32 / 8.0 * alpha, 0.7 * alpha, alpha]
                    .into_iter()
                    .flat_map(|v| half::f16::from_f32(v).to_le_bytes())
            })
            .collect();
        gpu.queue.write_texture(
            frame.tex.as_image_copy(),
            &bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(64), rows_per_image: Some(6) },
            wgpu::Extent3d { width: 8, height: 6, depth_or_array_layers: 1 },
        );
        let mut stage = OutputStage::new(gpu.device.clone(), gpu.queue.clone());
        let working = Working { space: ColorSpace::Srgb, linear: false };
        let n = w * h;
        let (cw, ch) = (w / 2, h / 2);
        for full_range in [false, true] {
            let out = OutputColor::new(ColorSpace::Rec709, Transfer::Auto, full_range);
            let mut pack = |format: InputFormat, keep_alpha: bool| {
                stage.seek(29);
                let p = stage.submit(&frame, &working, &out, format, [8, 6], keep_alpha);
                let bytes = stage.wait(p).into_vec();
                assert_eq!(bytes.len(), format.frame_bytes(8, 6), "{format:?}");
                bytes
            };
            // 4:2:0: the planes of the interleaved formats, with the same dither
            let nv12 = pack(InputFormat::Nv12, false);
            let i420 = pack(InputFormat::Yuv420p, false);
            assert_eq!(&i420[..n], &nv12[..n], "full={full_range}: luma plane");
            let (u, v): (Vec<u8>, Vec<u8>) = nv12[n..].as_chunks::<2>().0.iter().map(|p| (p[0], p[1])).unzip();
            assert_eq!(&i420[n..n + cw * ch], &u[..], "full={full_range}: Cb plane");
            assert_eq!(&i420[n + cw * ch..], &v[..], "full={full_range}: Cr plane");
            let p010 = words(&pack(InputFormat::P010, false));
            let i420_10 = words(&pack(InputFormat::Yuv420p10, false));
            assert_eq!(i420_10[..n], p010[..n].iter().map(|c| c >> 6).collect::<Vec<_>>()[..]);
            let (u, v): (Vec<u16>, Vec<u16>) =
                p010[n..].as_chunks::<2>().0.iter().map(|p| (p[0] >> 6, p[1] >> 6)).unzip();
            assert_eq!(i420_10[n..n + cw * ch], u[..]);
            assert_eq!(i420_10[n + cw * ch..], v[..]);
            // 4:4:4 and 4:2:2 against the output-encoded RGB of the RGBA16 pack
            let rgb = words(&pack(InputFormat::Rgba16, false));
            let (kr, kb) = (out.matrix.0 as f64, out.matrix.1 as f64);
            let yuv_of = |rgb: [f64; 3]| {
                let y = kr * rgb[0] + (1.0 - kr - kb) * rgb[1] + kb * rgb[2];
                [y, (rgb[2] - y) / (2.0 * (1.0 - kb)), (rgb[0] - y) / (2.0 * (1.0 - kr))]
            };
            let px = |i: usize| [rgb[4 * i], rgb[4 * i + 1], rgb[4 * i + 2]].map(|c| c as f64 / 65535.0);
            let yuv444_10 = words(&pack(InputFormat::Yuv444p10, false));
            let yuv444 = pack(InputFormat::Yuv444p, false);
            for i in 0..n {
                let [y, u, v] = yuv_of(px(i));
                for (k, (value, luma)) in [(y, true), (u, false), (v, false)].into_iter().enumerate() {
                    let want = code(value, luma, 10, full_range);
                    let got = yuv444_10[k * n + i] as i64;
                    assert!(
                        (got - want).abs() <= 1,
                        "full={full_range} 4:4:4 10-bit plane {k} sample {i}: {got} vs {want}"
                    );
                    // 8-bit: dithered, so a code away from the 10-bit one quartered
                    let got8 = yuv444[k * n + i] as i64;
                    assert!(
                        (got8 - (want + 2) / 4).abs() <= 1,
                        "full={full_range} 4:4:4 8-bit plane {k} sample {i}: {got8} vs {want}"
                    );
                }
            }
            let yuv422_10 = words(&pack(InputFormat::Yuv422p10, false));
            let yuv422 = pack(InputFormat::Yuv422p, false);
            assert_eq!(yuv422_10[..n], yuv444_10[..n], "full={full_range}: 4:2:2 luma is 4:4:4 luma");
            for cy in 0..h {
                for cx in 0..cw {
                    let (a, b) = (px(cy * w + 2 * cx), px(cy * w + 2 * cx + 1));
                    let [_, u, v] = yuv_of([(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0, (a[2] + b[2]) / 2.0]);
                    let at = cy * cw + cx;
                    for (k, value) in [u, v].into_iter().enumerate() {
                        let want = code(value, false, 10, full_range);
                        let got = yuv422_10[n + k * cw * h + at] as i64;
                        assert!(
                            (got - want).abs() <= 1,
                            "full={full_range} 4:2:2 10-bit chroma {k} at {at}: {got} vs {want}"
                        );
                        let got8 = yuv422[n + k * cw * h + at] as i64;
                        assert!(
                            (got8 - (want + 2) / 4).abs() <= 1,
                            "full={full_range} 4:2:2 8-bit chroma {k} at {at}: {got8} vs {want}"
                        );
                    }
                }
            }
            // alpha: a fourth plane, full range whatever the output's range
            let yuva = words(&pack(InputFormat::Yuva444p10, true));
            for i in 0..n {
                let want = (alphas[i] as f64 * 1023.0).round() as i64;
                assert!((yuva[3 * n + i] as i64 - want).abs() <= 1, "alpha {i}: {} vs {want}", yuva[3 * n + i]);
            }
        }
    }
}

#[cfg(test)]
mod wgsl_layout_tests {
    //! The Rust structs bound to WGSL structs have their layout (`crate::wgsl_layout`).
    use crate::wgsl_layout::{assert_agree, rust_fields};

    #[test]
    fn params_matches_wgsl_out() {
        let fields = rust_fields!(super::Params; to_out, fsize, osize, offset, scale, format, transfer, full_range, linear_light, store_transfer, keep_alpha, kr, kb, words, seed, mode, bg_offset, bg_scale, overlay, base, pad);
        assert_agree(include_str!("output.wgsl"), "Out", std::mem::size_of::<super::Params>(), &fields);
    }
}
