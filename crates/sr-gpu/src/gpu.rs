//! Headless GPU device, and the choice of adapter behind it.
//!
//! Every adapter the enabled backends expose is ranked: discrete GPUs first, then integrated,
//! unknown and virtual ones, and software rasterisers (llvmpipe, SwiftShader, WARP) last, so a
//! machine with a GPU never renders on the CPU unless asked to. Between equal devices a native
//! backend (Vulkan, Metal, DirectX 12) beats OpenGL, whose submission costs more.
//! `SR_GPU_ADAPTER` names an adapter explicitly and `SR_GPU_BACKEND` restricts the backends.
//! Where the adapter has ray queries the device asks for them, and the path tracer traverses the driver's
//! acceleration structures (the GPU's ray-tracing units where it has them); `SR_GPU_RAY_QUERY=0` leaves them off.

use std::sync::Arc;

/// Why no device could be created.
#[derive(Debug, thiserror::Error)]
pub enum GpuError {
    /// No adapter matched.
    #[error("no GPU adapter is available ({0}); install a Vulkan, Metal or DirectX 12 driver, or Mesa lavapipe for software rendering")]
    NoAdapter(String),
    /// No adapter has the requested name.
    #[error("no GPU adapter matches \"{wanted}\"; available: {}", if .available.is_empty() { "none".to_string() } else { .available.join(", ") })]
    NoMatch {
        /// The name asked for (`SR_GPU_ADAPTER`).
        wanted: String,
        /// Names of the adapters found.
        available: Vec<String>,
    },
    /// The adapter refused the device.
    #[error("the GPU adapter refused to create a device: {0}")]
    Device(String),
    /// The document has 3D objects, and no adapter that can run the 3D pass was chosen or found.
    #[error(
        "the document has 3D objects, but {why}; `scene-render gpus` lists the adapters found and SR_GPU_ADAPTER \
         names another"
    )]
    NoThreeD {
        /// What was wrong with the choice.
        why: String,
    },
    /// The adapter lacks a capability of a texture format the 3D pass draws into: 4× multisampled depth or colour.
    #[error(
        "the GPU adapter cannot give {format:?} textures {missing}, which the 3D pass needs; \
         `scene-render gpus` lists the adapters found and SR_GPU_ADAPTER names another"
    )]
    Format {
        /// The texture format.
        format: wgpu::TextureFormat,
        /// The capabilities it lacks.
        missing: String,
    },
    /// The adapter's OpenGL driver is older than the renderer's shaders: they read storage buffers and run compute shaders,
    /// which OpenGL has from 4.3 and OpenGL ES from 3.1.
    #[error(
        "the GPU adapter's OpenGL driver is version \"{version}\", which lacks the storage buffers and compute shaders \
         that the renderer's shaders need (OpenGL 4.3 or OpenGL ES 3.1); `scene-render gpus` lists the adapters found and \
         SR_GPU_ADAPTER names another"
    )]
    OpenGlVersion {
        /// The version the driver reports (`GL_VERSION`).
        version: String,
    },
    /// The device reported an error outside an error scope (a validation error, or memory it could not allocate):
    /// the work that raised it was not done.
    #[error("the GPU device reported an error: {0}")]
    Uncaptured(String),
}

/// Which adapters to consider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuOptions {
    /// Backends to enumerate.
    pub backends: wgpu::Backends,
    /// Case-insensitive part of the adapter name to use instead of the best-ranked one.
    pub adapter: Option<String>,
}

impl Default for GpuOptions {
    fn default() -> Self {
        GpuOptions { backends: wgpu::Backends::PRIMARY | wgpu::Backends::GL, adapter: None }
    }
}

impl GpuOptions {
    /// Options from `SR_GPU_BACKEND` (`vulkan`, `metal`, `dx12`, `gl`) and `SR_GPU_ADAPTER`.
    pub fn from_env() -> Self {
        let backends = match std::env::var("SR_GPU_BACKEND").ok().as_deref() {
            Some("vulkan") => wgpu::Backends::VULKAN,
            Some("metal") => wgpu::Backends::METAL,
            Some("dx12") => wgpu::Backends::DX12,
            Some("gl") => wgpu::Backends::GL,
            _ => GpuOptions::default().backends,
        };
        let adapter = std::env::var("SR_GPU_ADAPTER").ok().filter(|s| !s.trim().is_empty());
        GpuOptions { backends, adapter }
    }
}

/// Names software rasterisers report; some report a device type other than `Cpu`
/// (llvmpipe through OpenGL, for one).
const SOFTWARE_NAMES: &[&str] = &["llvmpipe", "lavapipe", "softpipe", "swiftshader", "microsoft basic render driver"];

/// True when the adapter rasterises on the CPU.
pub fn is_software(info: &wgpu::AdapterInfo) -> bool {
    let name = info.name.to_lowercase();
    info.device_type == wgpu::DeviceType::Cpu || SOFTWARE_NAMES.iter().any(|s| name.contains(s))
}

/// Ranking key: lower is better.
fn rank(info: &wgpu::AdapterInfo) -> (u8, u8) {
    let class = if is_software(info) {
        4
    } else {
        match info.device_type {
            wgpu::DeviceType::DiscreteGpu => 0,
            wgpu::DeviceType::IntegratedGpu => 1,
            wgpu::DeviceType::Other => 2,
            wgpu::DeviceType::VirtualGpu => 3,
            wgpu::DeviceType::Cpu => 4,
        }
    };
    (class, (info.backend == wgpu::Backend::Gl) as u8)
}

/// Index of the adapter to use among `found`: the first whose name contains `name`
/// (ignoring case) when one is given, otherwise the best-ranked, the earliest on a tie.
pub fn choose_adapter(found: &[wgpu::AdapterInfo], name: Option<&str>) -> Result<usize, GpuError> {
    if found.is_empty() {
        return Err(GpuError::NoAdapter("no adapter was found".into()));
    }
    match name {
        Some(want) => {
            let w = want.to_lowercase();
            found.iter().position(|a| a.name.to_lowercase().contains(&w)).ok_or_else(|| GpuError::NoMatch {
                wanted: want.to_string(),
                available: found.iter().map(|a| a.name.clone()).collect(),
            })
        }
        None => Ok((0..found.len()).min_by_key(|&i| rank(&found[i])).unwrap_or(0)),
    }
}

/// True when the 3D pass can run on the adapter: on every backend, Vulkan, Metal, DirectX 12 and OpenGL alike. An
/// adapter without the texture formats the pass draws into is refused when its device is opened ([`GpuError::Format`]).
pub fn can_run_3d(info: &wgpu::AdapterInfo) -> bool {
    let _ = info;
    true
}

/// Like [`choose_adapter`], for a document that does (`needs_3d`) or does not need the 3D pass.
///
/// A document with 3D objects takes the best-ranked adapter that can run them ([`can_run_3d`]; so, on WSL2 where the GPU
/// is reachable only through OpenGL, the GPU before lavapipe) instead of one that would drop the objects without a word.
/// When none can, or the adapter named cannot, that is [`GpuError::NoThreeD`].
pub fn choose_adapter_for(found: &[wgpu::AdapterInfo], name: Option<&str>, needs_3d: bool) -> Result<usize, GpuError> {
    if !needs_3d {
        return choose_adapter(found, name);
    }
    match name {
        Some(_) => {
            let i = choose_adapter(found, name)?;
            if can_run_3d(&found[i]) {
                Ok(i)
            } else {
                Err(GpuError::NoThreeD {
                    why: format!("the adapter asked for, {}, cannot run them", describe(&found[i])),
                })
            }
        }
        None => {
            if found.is_empty() {
                return Err(GpuError::NoAdapter("no adapter was found".into()));
            }
            (0..found.len()).filter(|&i| can_run_3d(&found[i])).min_by_key(|&i| rank(&found[i])).ok_or_else(|| {
                GpuError::NoThreeD {
                    why: format!(
                        "none of the adapters found can run them ({})",
                        found.iter().map(describe).collect::<Vec<_>>().join(", ")
                    ),
                }
            })
        }
    }
}

/// `Ok` when the adapter can run the 3D pass, else [`GpuError::NoThreeD`]: for a device chosen by the caller.
pub fn require_3d(info: &wgpu::AdapterInfo) -> Result<(), GpuError> {
    if can_run_3d(info) {
        Ok(())
    } else {
        Err(GpuError::NoThreeD { why: format!("the adapter in use, {}, cannot run them", describe(info)) })
    }
}

/// `Ok` unless `info` is an OpenGL adapter whose driver reports a version below OpenGL 4.3 or OpenGL ES 3.1, which is
/// [`GpuError::OpenGlVersion`]. wgpu gives OpenGL's `GL_VERSION` as the driver info, and from OpenGL 3.3 and OpenGL ES 3.0
/// on the shading language a driver compiles has the version of OpenGL itself, which is the one wgpu writes the shaders
/// for: below 4.3 (ES 3.1) its GLSL has no storage buffers or compute shaders, and the device opens and then fails its
/// first pipeline. The OpenGL backend (vendor/wgpu-hal) asks EGL for the newest desktop context first, because the NVIDIA
/// driver gives exactly the version asked for; this refuses what a driver without 4.3 still gives. A version this cannot
/// read is left to the probe of the device.
pub fn check_gl_version(info: &wgpu::AdapterInfo) -> Result<(), GpuError> {
    if info.backend != wgpu::Backend::Gl {
        return Ok(());
    }
    let (rest, needed) = match info.driver_info.strip_prefix("OpenGL ES ") {
        Some(rest) => (rest, (3, 1)),
        None => (info.driver_info.as_str(), (4, 3)),
    };
    let mut numbers = rest.split_whitespace().next().unwrap_or("").split('.').map(str::parse::<u32>);
    match (numbers.next(), numbers.next()) {
        (Some(Ok(major)), Some(Ok(minor))) if (major, minor) < needed => {
            Err(GpuError::OpenGlVersion { version: info.driver_info.clone() })
        }
        _ => Ok(()),
    }
}

/// `Ok` when `features` (an adapter's, `|f| adapter.get_texture_format_features(f)`) gives the texture formats the 3D
/// pass draws into everything it asks of them, else [`GpuError::Format`] for the first that lacks something.
pub fn check_3d_formats(features: impl Fn(wgpu::TextureFormat) -> wgpu::TextureFormatFeatures) -> Result<(), GpuError> {
    use wgpu::{TextureFormatFeatureFlags as Flag, TextureUsages as U};
    let names = [
        (Flag::MULTISAMPLE_X4, U::empty(), "4x multisampling"),
        (Flag::MULTISAMPLE_RESOLVE, U::empty(), "multisample resolve"),
        (Flag::empty(), U::RENDER_ATTACHMENT, "use as a render attachment"),
        (Flag::empty(), U::TEXTURE_BINDING, "binding to a shader"),
    ];
    for (format, usages, flags) in crate::three::MULTISAMPLED {
        let have = features(format);
        let missing = names
            .iter()
            .filter(|(flag, usage, _)| {
                (flags.contains(*flag) && !have.flags.contains(*flag))
                    || (usages.contains(*usage) && !have.allowed_usages.contains(*usage))
            })
            .map(|(_, _, name)| *name)
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(GpuError::Format { format, missing: missing.join(", ") });
        }
    }
    Ok(())
}

/// A warning for renders on a software adapter, `None` on a GPU.
pub fn software_warning(info: &wgpu::AdapterInfo) -> Option<String> {
    is_software(info).then(|| {
        format!(
            "rendering on a software adapter ({}, {:?}): expect renders many times slower than on a GPU; \
             `scene-render gpus` lists the adapters found and SR_GPU_ADAPTER picks one",
            info.name, info.backend
        )
    })
}

/// Why 3D objects are left out on this adapter and how to get them drawn; `None` where they draw, which is on every
/// adapter ([`can_run_3d`]).
pub fn three_d_warning(info: &wgpu::AdapterInfo) -> Option<String> {
    (!can_run_3d(info)).then(|| {
        format!(
            "3D objects are not drawn: the adapter in use ({}) cannot run the 3D pass; \
             `scene-render gpus` lists the adapters found and SR_GPU_ADAPTER names another",
            info.name
        )
    })
}

/// Index among `found` of the adapter `want` describes: the same name, backend, vendor, device,
/// type and bus address, not merely a name that contains it ("RTX 3080" is not "RTX 3080 Ti").
pub fn same_adapter(found: &[wgpu::AdapterInfo], want: &wgpu::AdapterInfo) -> Option<usize> {
    found.iter().position(|a| {
        a.name == want.name
            && a.backend == want.backend
            && a.vendor == want.vendor
            && a.device == want.device
            && a.device_type == want.device_type
            && a.device_pci_bus_id == want.device_pci_bus_id
    })
}

/// The limits a device is created with: the downlevel set every backend meets, at the
/// adapter's texture sizes, with the adapter's own buffer, binding and layer limits where
/// those are larger, so large frames, splat sets and shadow arrays fit where the hardware has room.
pub fn device_limits(adapter: &wgpu::Limits) -> wgpu::Limits {
    let base = wgpu::Limits { max_storage_buffers_per_shader_stage: 8, ..wgpu::Limits::downlevel_defaults() }
        .using_resolution(adapter.clone());
    wgpu::Limits {
        max_buffer_size: base.max_buffer_size.max(adapter.max_buffer_size),
        max_storage_buffer_binding_size: base
            .max_storage_buffer_binding_size
            .max(adapter.max_storage_buffer_binding_size),
        max_uniform_buffer_binding_size: base
            .max_uniform_buffer_binding_size
            .max(adapter.max_uniform_buffer_binding_size),
        max_texture_array_layers: base.max_texture_array_layers.max(adapter.max_texture_array_layers),
        ..base
    }
}

/// The ray-query feature when `adapter` has it and `setting` (`SR_GPU_RAY_QUERY`) does not switch it off with `0`,
/// `off` or `false`.
pub fn ray_query_features(adapter: wgpu::Features, setting: Option<&str>) -> wgpu::Features {
    let off =
        setting.map(|v| v.trim().to_ascii_lowercase()).is_some_and(|v| ["0", "off", "false"].contains(&v.as_str()));
    if off {
        return wgpu::Features::empty();
    }
    adapter & wgpu::Features::EXPERIMENTAL_RAY_QUERY
}

/// The acknowledgement wgpu asks for before it enables a feature it calls experimental, which ray queries are.
#[allow(unsafe_code)]
fn experimental_features() -> wgpu::ExperimentalFeatures {
    // SAFETY: the token only acknowledges that wgpu's experimental features may still have bugs; it has no
    // precondition for the caller to uphold. The only such feature requested is EXPERIMENTAL_RAY_QUERY, used through
    // wgpu's validated API: the structures are built in the encoder that records the passes reading them, and the
    // shaders pass naga's validation. SR_GPU_RAY_QUERY=0 leaves the feature, and this token, out of the device.
    unsafe { wgpu::ExperimentalFeatures::enabled() }
}

/// A one-line description: name, backend and device type.
pub fn describe(info: &wgpu::AdapterInfo) -> String {
    format!("{} ({:?}, {:?})", info.name, info.backend, info.device_type)
}

/// Every adapter the backends in `opts` expose, in enumeration order, and the index
/// `Gpu::with_options` would choose (or why none would be).
pub fn adapters(opts: &GpuOptions) -> (Vec<wgpu::AdapterInfo>, Result<usize, GpuError>) {
    let found = {
        let creation = creation_lock();
        enumerate(opts.backends, &creation).into_iter().map(|a| a.get_info()).collect::<Vec<_>>()
    };
    let chosen = choose_adapter(&found, opts.adapter.as_deref());
    (found, chosen)
}

/// Every adapter the backends in `opts` expose, in enumeration order, with the features it offers a device.
pub fn adapter_features(opts: &GpuOptions) -> Vec<(wgpu::AdapterInfo, wgpu::Features)> {
    let creation = creation_lock();
    enumerate(opts.backends, &creation).into_iter().map(|a| (a.get_info(), a.features())).collect()
}

/// Instance flags: the debug and validation layers the build config would turn on (debug builds) only when asked for with
/// `SR_GPU_DEBUG=1` or `--debug-gpu`. They name every object through the Vulkan loader, which crashed under concurrent
/// pipeline creation on llvmpipe, and a delivery does not need them.
pub fn instance_flags() -> wgpu::InstanceFlags {
    if debug_layers() {
        wgpu::InstanceFlags::from_build_config()
    } else {
        wgpu::InstanceFlags::empty()
    }
}

/// Whether the debug and validation layers are on (`SR_GPU_DEBUG` set to anything but empty or `0`; `--debug-gpu` sets it).
/// With them on, object naming reaches the Vulkan loader from every buffer and texture creation, which is not serialised:
/// a delivery therefore renders with one worker.
pub fn debug_layers() -> bool {
    std::env::var("SR_GPU_DEBUG").is_ok_and(|v| !v.is_empty() && v != "0")
}

static CREATION: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The devices opened by this test binary: the unit tests of the crate share one.
#[cfg(test)]
static OPENED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The one device that the unit tests of this crate use, opened the first time it is asked for (the reason when there is none). Tests
/// run on the threads of one process, and several devices used at once by those threads have blocked the process for good inside the
/// Vulkan driver (a thread waiting on the creation lock, others on the driver's own, with a group of its threads for each device: 7
/// devices open at once in the dump of the hang, found once in 15 to 45 runs of the unit tests with the host loaded); creation is
/// serialised by [`creation_lock`] but the use of devices is not, so a process has one.
#[cfg(test)]
pub(crate) fn test_gpu() -> Result<Gpu, String> {
    static SHARED: std::sync::OnceLock<Result<Gpu, String>> = std::sync::OnceLock::new();
    SHARED.get_or_init(|| Gpu::new().map_err(|e| e.to_string())).clone()
}

/// The process-wide lock held while an instance lists its adapters, a device opens or a pipeline is created. Several
/// renderers are created at once by a parallel delivery, and creating pipelines concurrently on a software adapter crashed
/// the Vulkan loader; so did creating instances concurrently (each scans the Vulkan drivers and loads EGL). Rendering takes
/// no lock.
pub fn creation_lock() -> std::sync::MutexGuard<'static, ()> {
    CREATION.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The adapters of a new instance. The caller holds [`creation_lock`], and still holds it when the adapters it does not
/// keep are dropped (the last one drops the instance).
fn enumerate(backends: wgpu::Backends, _creation: &std::sync::MutexGuard<'static, ()>) -> Vec<wgpu::Adapter> {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = backends;
    desc.flags = instance_flags();
    let instance = wgpu::Instance::new(desc);
    pollster::block_on(instance.enumerate_adapters(backends))
}

/// What decides how WSL2 can reach its GPU.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WslProbe {
    /// `/dev/dxg`, the paravirtualised GPU of WSL2, exists.
    pub dxg: bool,
    /// Mesa's D3D12 Gallium driver (`d3d12_dri.so`) is installed.
    pub d3d12_driver: bool,
    /// The NVIDIA user-mode driver is mapped into WSL (`/usr/lib/wsl/lib`).
    pub nvidia: bool,
    /// `GALLIUM_DRIVER` as set by the user.
    pub gallium_driver: Option<String>,
    /// `MESA_D3D12_DEFAULT_ADAPTER_NAME` as set by the user.
    pub d3d12_adapter: Option<String>,
    /// `SR_GPU_WSL=off`.
    pub opt_out: bool,
}

impl WslProbe {
    /// Looks at this machine.
    pub fn detect() -> Self {
        let exists = |p: &str| std::path::Path::new(p).exists();
        let d3d12_driver =
            ["/usr/lib/x86_64-linux-gnu/dri", "/usr/lib/aarch64-linux-gnu/dri", "/usr/lib64/dri", "/usr/lib/dri"]
                .iter()
                .any(|d| exists(&format!("{d}/d3d12_dri.so")));
        let nvidia = std::fs::read_dir("/usr/lib/wsl/lib")
            .map(|r| r.flatten().any(|e| e.file_name().to_string_lossy().starts_with("libnvidia")))
            .unwrap_or(false);
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        WslProbe {
            dxg: exists("/dev/dxg"),
            d3d12_driver,
            nvidia,
            gallium_driver: var("GALLIUM_DRIVER"),
            d3d12_adapter: var("MESA_D3D12_DEFAULT_ADAPTER_NAME"),
            opt_out: var("SR_GPU_WSL")
                .is_some_and(|v| matches!(v.to_lowercase().as_str(), "off" | "0" | "false" | "no")),
        }
    }
}

/// Environment variables that route OpenGL through Mesa's D3D12 driver on WSL2, where no
/// hardware Vulkan driver exists and the Vulkan loader only finds lavapipe; on an NVIDIA
/// machine they also pick the NVIDIA GPU over an integrated one. Nothing the user set is changed.
pub fn wsl_environment(p: &WslProbe) -> Vec<(String, String)> {
    let mut v = Vec::new();
    if !p.dxg || !p.d3d12_driver || p.opt_out || p.gallium_driver.is_some() {
        return v;
    }
    v.push(("GALLIUM_DRIVER".to_string(), "d3d12".to_string()));
    if p.nvidia && p.d3d12_adapter.is_none() {
        v.push(("MESA_D3D12_DEFAULT_ADAPTER_NAME".to_string(), "NVIDIA".to_string()));
    }
    v
}

/// Applies [`wsl_environment`] to this process. Mesa reads these variables when a GL
/// display is first opened, so call it at program start, before any other thread runs.
pub fn prepare_environment() -> Vec<(String, String)> {
    let set = wsl_environment(&WslProbe::detect());
    for (k, v) in &set {
        std::env::set_var(k, v);
    }
    set
}

/// A device and queue shared by renderers.
#[derive(Debug, Clone)]
pub struct Gpu {
    /// Device.
    pub device: Arc<wgpu::Device>,
    /// Queue.
    pub queue: Arc<wgpu::Queue>,
    /// Adapter description.
    pub info: wgpu::AdapterInfo,
    /// The device has timestamp queries (GPU time of passes can be measured).
    pub timestamps: bool,
    /// The widest float format the device renders to that holds 3D view depth and reflectance
    /// (32-bit float where it can; OpenGL may not render to RGBA32F).
    pub gbuffer_depth: wgpu::TextureFormat,
    /// The single-channel float format the device renders to for depth and blur-size maps
    /// (R32Float where it can, else R16Float).
    pub scalar_target: wgpu::TextureFormat,
    /// The first error the device reported outside an error scope since it was last taken (see [`Gpu::take_error`]).
    uncaptured: Arc<std::sync::Mutex<Option<String>>>,
    /// Sample counts supported by both the raster colour and depth targets, with colour resolve available.
    pub raster_sample_counts: wgpu::TextureFormatFeatureFlags,
    /// The compositor of each working format, built by the first renderer on this device that needs it and shared by
    /// every other (see [`Gpu::compositor`]).
    compositors: Arc<std::sync::Mutex<CompositorSlots>>,
}

type CompositorSlots = Vec<(wgpu::TextureFormat, Arc<std::sync::OnceLock<Arc<crate::render::Compositor>>>)>;

/// What the renderer's shaders need of a device, tried on a minimal pipeline: a fragment shader that reads a runtime-sized
/// storage buffer. An OpenGL adapter older than 4.3 (the one a WSL2 host without a D3D12 driver falls back to) opens a
/// device and then fails the first pipeline the renderer creates, which wgpu treats as fatal; made inside an error scope it
/// is an error here, so that the adapter is refused with its reason, as a caller (a test of the OpenGL backend, the
/// adapter choice) can act on.
fn probe(device: &wgpu::Device) -> Result<(), GpuError> {
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("scene-render-probe"),
        source: wgpu::ShaderSource::Wgsl(
            "@group(0) @binding(0) var<storage, read> data: array<u32>;\n\
             @vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }\n\
             @fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(f32(data[0]) + f32(arrayLength(&data))); }"
                .into(),
        ),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("scene-render-probe"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("scene-render-probe"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let _pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("scene-render-probe"),
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
            targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
        }),
        multiview_mask: None,
        cache: None,
    });
    // the scopes are a stack: both are popped, the one pushed last first, whatever the first one caught
    let from_validation = pollster::block_on(validation.pop());
    let from_internal = pollster::block_on(internal.pop());
    match from_validation.or(from_internal) {
        None => Ok(()),
        Some(e) => Err(GpuError::Device(format!("the device cannot run the renderer's shaders: {e}"))),
    }
}

impl Gpu {
    /// Opens the best adapter without a window, following `SR_GPU_BACKEND` and `SR_GPU_ADAPTER`.
    pub fn new() -> Result<Gpu, GpuError> {
        Gpu::with_options(&GpuOptions::from_env())
    }

    /// Like [`Gpu::new`], for a document that does or does not need the 3D pass
    /// (see [`choose_adapter_for`]).
    pub fn new_for(needs_3d: bool) -> Result<Gpu, GpuError> {
        Gpu::with_options_for(&GpuOptions::from_env(), needs_3d)
    }

    /// Opens the adapter `opts` selects.
    pub fn with_options(opts: &GpuOptions) -> Result<Gpu, GpuError> {
        Gpu::with_options_for(opts, false)
    }

    /// Opens the adapter `opts` selects for a document that does or does not need the 3D pass.
    pub fn with_options_for(opts: &GpuOptions, needs_3d: bool) -> Result<Gpu, GpuError> {
        let creation = creation_lock();
        let mut found = enumerate(opts.backends, &creation);
        let infos = found.iter().map(|a| a.get_info()).collect::<Vec<_>>();
        let i = choose_adapter_for(&infos, opts.adapter.as_deref(), needs_3d).map_err(|e| match e {
            GpuError::NoAdapter(_) => GpuError::NoAdapter(format!("none on {:?}", opts.backends)),
            e => e,
        })?;
        let adapter = found.swap_remove(i);
        drop(found);
        Gpu::open(creation, adapter)
    }

    /// Opens a device on `adapter` with [`creation_lock`] held throughout. Parameters drop in reverse order of
    /// declaration, so the adapter is dropped before the lock is released.
    fn open(_creation: std::sync::MutexGuard<'static, ()>, adapter: wgpu::Adapter) -> Result<Gpu, GpuError> {
        #[cfg(test)]
        assert!(
            OPENED.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0,
            "a unit test of sr-gpu opened a second device: they share one (crate::gpu::test_gpu()), because several devices used at \
             once by the threads of one process have deadlocked the Vulkan driver"
        );
        let info = adapter.get_info();
        check_gl_version(&info)?;
        check_3d_formats(|f| adapter.get_texture_format_features(f))?;
        let rays = ray_query_features(adapter.features(), std::env::var("SR_GPU_RAY_QUERY").ok().as_deref());
        let limits = if rays.is_empty() {
            device_limits(&adapter.limits())
        } else {
            device_limits(&adapter.limits()).using_acceleration_structure_values(adapter.limits())
        };
        // timestamp queries, where the adapter has them, so renders can report GPU time; and blending into and
        // filtering 32-bit float textures for project/@precision="f32" (SREP 72), and adapter-specific raster sample counts.
        let features = adapter.features()
            & (wgpu::Features::TIMESTAMP_QUERY
                | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS
                | crate::resources::F32_FEATURES
                | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES)
            | rays;
        let format_flags = |format: wgpu::TextureFormat| {
            if features.contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES) {
                adapter.get_texture_format_features(format).flags
            } else {
                format.guaranteed_format_features(features).flags
            }
        };
        let color = format_flags(crate::resources::FORMAT);
        let depth = format_flags(wgpu::TextureFormat::Depth32Float);
        use wgpu::TextureFormatFeatureFlags as F;
        let raster_sample_counts = if color.contains(F::MULTISAMPLE_RESOLVE) {
            color & depth & (F::MULTISAMPLE_X2 | F::MULTISAMPLE_X4)
        } else {
            F::empty()
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("scene-render"),
            required_features: features,
            required_limits: limits,
            experimental_features: if rays.is_empty() {
                wgpu::ExperimentalFeatures::disabled()
            } else {
                experimental_features()
            },
            ..Default::default()
        }))
        .map_err(|e| GpuError::Device(e.to_string()))?;
        probe(&device)?;
        // wgpu's default handler panics on any error outside a scope, such as a texture over the device's limits that
        // input asked for; the first one is kept instead and returned at the next checkpoint (`take_error`). The ones
        // after it are mostly its consequences (objects made from the invalid one), so they are not kept.
        let uncaptured = Arc::new(std::sync::Mutex::new(None::<String>));
        let slot = uncaptured.clone();
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
            slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get_or_insert_with(|| e.to_string());
        }));
        let timestamps = features.contains(wgpu::Features::TIMESTAMP_QUERY);
        let renderable = |f: wgpu::TextureFormat| {
            adapter.get_texture_format_features(f).allowed_usages.contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
        };
        let gbuffer_depth = [wgpu::TextureFormat::Rg32Float, wgpu::TextureFormat::Rgba32Float]
            .into_iter()
            .find(|f| renderable(*f))
            .unwrap_or(wgpu::TextureFormat::Rgba16Float);
        let scalar_target = if renderable(wgpu::TextureFormat::R32Float) {
            wgpu::TextureFormat::R32Float
        } else {
            wgpu::TextureFormat::R16Float
        };
        Ok(Gpu {
            device: Arc::new(device),
            queue: Arc::new(queue),
            info,
            timestamps,
            gbuffer_depth,
            scalar_target,
            uncaptured,
            raster_sample_counts,
            compositors: Default::default(),
        })
    }

    /// The compositor of `format` on this device: built on first use, and the same for every renderer after. Two
    /// renderers asking at once for one format wait for a single build; other formats build alongside it.
    pub(crate) fn compositor(&self, format: wgpu::TextureFormat) -> Arc<crate::render::Compositor> {
        let slot = {
            let mut slots = self.compositors.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            match slots.iter().find(|(f, _)| *f == format) {
                Some((_, slot)) => slot.clone(),
                None => {
                    let slot = Arc::<std::sync::OnceLock<_>>::default();
                    slots.push((format, slot.clone()));
                    slot
                }
            }
        };
        slot.get_or_init(|| Arc::new(crate::render::Compositor::new(&self.device, format))).clone()
    }

    /// The error the device reported outside an error scope since the last call, if any, and clears it. A renderer
    /// checks after each frame (it is one of the frame's [`errors`](crate::RenderStats::errors)); code that drives
    /// the device directly checks after its work.
    pub fn take_error(&self) -> Result<(), GpuError> {
        match self.uncaptured.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take() {
            Some(e) => Err(GpuError::Uncaptured(e)),
            None => Ok(()),
        }
    }

    /// Selects a supported raster sample count and reports any fallback from the requested count.
    pub fn raster_samples(&self, requested: u64) -> (u32, Option<String>) {
        let (wanted, selected) = select_raster_samples(requested, self.raster_sample_counts);
        let warning = (selected != wanted).then(|| format!(
            "antialias3d={requested} requests {wanted} raster samples, but adapter {} does not support them; using {selected}",
            self.info.name
        ));
        (selected, warning)
    }

    /// A new device on the same adapter, for work that needs a device of its own.
    pub fn open_like(&self) -> Result<Gpu, GpuError> {
        let creation = creation_lock();
        let mut found = enumerate(wgpu::Backends::from(self.info.backend), &creation);
        let infos = found.iter().map(|a| a.get_info()).collect::<Vec<_>>();
        let i = same_adapter(&infos, &self.info).ok_or_else(|| GpuError::NoMatch {
            wanted: describe(&self.info),
            available: infos.iter().map(describe).collect(),
        })?;
        let adapter = found.swap_remove(i);
        drop(found);
        Gpu::open(creation, adapter)
    }

    /// True when this device rasterises on the CPU.
    pub fn is_software(&self) -> bool {
        is_software(&self.info)
    }

    /// Blocks until submitted work completes.
    pub fn wait(&self) {
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
    }

    /// Waits for one submission (and everything before it), leaving later submissions in flight.
    pub fn wait_for(&self, idx: wgpu::SubmissionIndex) {
        let _ = self.device.poll(wgpu::PollType::Wait { submission_index: Some(idx), timeout: None });
    }
}

fn select_raster_samples(requested: u64, flags: wgpu::TextureFormatFeatureFlags) -> (u32, u32) {
    let requested = requested.clamp(1, 4) as u32;
    let wanted = if requested == 3 { 4 } else { requested };
    let selected = [4, 2, 1].into_iter().find(|&n| n <= wanted && flags.sample_count_supported(n)).unwrap_or(1);
    (wanted, selected)
}

#[cfg(test)]
mod raster_sample_tests {
    #[test]
    fn a6_raster_samples_round_up_three_and_fall_back_only_to_supported_counts() {
        use wgpu::TextureFormatFeatureFlags as F;
        for (flags, expected) in [
            (F::MULTISAMPLE_X2 | F::MULTISAMPLE_X4, [1, 2, 4, 4]),
            (F::MULTISAMPLE_X2, [1, 2, 2, 2]),
            (F::MULTISAMPLE_X4, [1, 1, 4, 4]),
            (F::empty(), [1, 1, 1, 1]),
        ] {
            for (requested, selected) in (1..=4).zip(expected) {
                let wanted = if requested == 3 { 4 } else { requested as u32 };
                assert_eq!(super::select_raster_samples(requested, flags), (wanted, selected));
            }
        }
    }
}
