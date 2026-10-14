//! Adapter choice without a GPU: hardware before software, an explicit name wins, software
//! is recognised by name when the driver reports another type, and WSL2 is set up to reach
//! its GPU through Mesa's D3D12 driver.

use sr_gpu::gpu::{
    can_run_3d, check_gl_version, choose_adapter, choose_adapter_for, is_software, software_warning, wsl_environment,
    WslProbe,
};
use sr_gpu::GpuError;
use wgpu::{AdapterInfo, Backend, DeviceType};

fn info(name: &str, t: DeviceType, b: Backend) -> AdapterInfo {
    AdapterInfo { name: name.into(), ..AdapterInfo::new(t, b) }
}

fn lavapipe() -> AdapterInfo {
    info("llvmpipe (LLVM 19.1.7, 256 bits)", DeviceType::Cpu, Backend::Vulkan)
}

#[test]
fn hardware_is_chosen_before_software() {
    // what WSL2 offers once the D3D12 driver is enabled: lavapipe first, the GPU behind GL second
    let found = [lavapipe(), info("D3D12 (NVIDIA GeForce RTX 4050 Laptop GPU)", DeviceType::Other, Backend::Gl)];
    assert_eq!(choose_adapter(&found, None).unwrap(), 1);
}

#[test]
fn device_classes_rank_discrete_integrated_other_virtual_then_software() {
    let found = [
        lavapipe(),
        info("virtio", DeviceType::VirtualGpu, Backend::Vulkan),
        info("unknown", DeviceType::Other, Backend::Vulkan),
        info("Radeon 780M", DeviceType::IntegratedGpu, Backend::Vulkan),
        info("RTX 4050", DeviceType::DiscreteGpu, Backend::Vulkan),
    ];
    assert_eq!(choose_adapter(&found, None).unwrap(), 4);
    assert_eq!(choose_adapter(&found[..4], None).unwrap(), 3);
    assert_eq!(choose_adapter(&found[..3], None).unwrap(), 2);
    assert_eq!(choose_adapter(&found[..2], None).unwrap(), 1);
    assert_eq!(choose_adapter(&found[..1], None).unwrap(), 0);
}

#[test]
fn software_is_recognised_by_name_when_the_driver_reports_another_type() {
    for name in [
        "llvmpipe (LLVM 19.1.7, 256 bits)",
        "lavapipe",
        "softpipe",
        "SwiftShader Device (Subzero)",
        "Microsoft Basic Render Driver",
    ] {
        assert!(is_software(&info(name, DeviceType::Other, Backend::Gl)), "{name}");
    }
    assert!(is_software(&info("anything", DeviceType::Cpu, Backend::Vulkan)));
    assert!(!is_software(&info("D3D12 (NVIDIA GeForce RTX 4050 Laptop GPU)", DeviceType::Other, Backend::Gl)));
    // GL on llvmpipe is software even when it reports itself as an unknown device
    let found = [info("llvmpipe (LLVM 19.1.7, 256 bits)", DeviceType::Other, Backend::Gl), lavapipe()];
    assert_eq!(choose_adapter(&found, None).unwrap(), 1, "a native backend wins between two software devices");
}

#[test]
fn a_native_backend_wins_a_tie_with_gl() {
    let found =
        [info("RTX (GL)", DeviceType::DiscreteGpu, Backend::Gl), info("RTX", DeviceType::DiscreteGpu, Backend::Vulkan)];
    assert_eq!(choose_adapter(&found, None).unwrap(), 1);
}

#[test]
fn a_named_adapter_is_chosen_even_when_it_is_software() {
    let found = [info("RTX 4050", DeviceType::DiscreteGpu, Backend::Vulkan), lavapipe()];
    assert_eq!(choose_adapter(&found, Some("LLVMpipe")).unwrap(), 1);
    assert_eq!(choose_adapter(&found, Some("rtx")).unwrap(), 0);
}

#[test]
fn an_unknown_name_lists_what_exists() {
    let found = [info("RTX 4050", DeviceType::DiscreteGpu, Backend::Vulkan), lavapipe()];
    match choose_adapter(&found, Some("radeon")) {
        Err(e @ GpuError::NoMatch { .. }) => {
            let msg = e.to_string();
            assert!(msg.contains("radeon") && msg.contains("RTX 4050") && msg.contains("llvmpipe"), "{msg}");
        }
        other => panic!("expected NoMatch, got {other:?}"),
    }
}

#[test]
fn nothing_found_is_an_error() {
    assert!(matches!(choose_adapter(&[], None), Err(GpuError::NoAdapter(_))));
}

#[test]
fn software_warning_names_the_adapter_and_the_way_out() {
    let w = software_warning(&lavapipe()).expect("software adapters warn");
    assert!(w.contains("llvmpipe") && w.contains("scene-render gpus"), "{w}");
    assert!(software_warning(&info("RTX 4050", DeviceType::DiscreteGpu, Backend::Vulkan)).is_none());
}

fn wsl() -> WslProbe {
    WslProbe { dxg: true, d3d12_driver: true, nvidia: true, gallium_driver: None, d3d12_adapter: None, opt_out: false }
}

fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
    v.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

#[test]
fn wsl2_routes_gl_through_d3d12_to_the_nvidia_gpu() {
    assert_eq!(
        wsl_environment(&wsl()),
        pairs(&[("GALLIUM_DRIVER", "d3d12"), ("MESA_D3D12_DEFAULT_ADAPTER_NAME", "NVIDIA")])
    );
}

#[test]
fn wsl2_without_nvidia_keeps_the_default_d3d12_adapter() {
    assert_eq!(wsl_environment(&WslProbe { nvidia: false, ..wsl() }), pairs(&[("GALLIUM_DRIVER", "d3d12")]));
}

#[test]
fn settings_the_user_made_are_never_overridden() {
    assert_eq!(wsl_environment(&WslProbe { gallium_driver: Some("llvmpipe".into()), ..wsl() }), pairs(&[]));
    assert_eq!(
        wsl_environment(&WslProbe { d3d12_adapter: Some("AMD".into()), ..wsl() }),
        pairs(&[("GALLIUM_DRIVER", "d3d12")])
    );
}

#[test]
fn nothing_changes_outside_wsl2_without_the_driver_or_when_opted_out() {
    assert_eq!(wsl_environment(&WslProbe { dxg: false, ..wsl() }), pairs(&[]));
    assert_eq!(wsl_environment(&WslProbe { d3d12_driver: false, ..wsl() }), pairs(&[]));
    assert_eq!(wsl_environment(&WslProbe { opt_out: true, ..wsl() }), pairs(&[]));
}

#[test]
fn a_device_reopens_on_exactly_its_own_adapter() {
    use sr_gpu::gpu::same_adapter;
    let gpu = |name: &str, device: u32, b: Backend| AdapterInfo { device, ..info(name, DeviceType::DiscreteGpu, b) };
    let found = [
        gpu("NVIDIA GeForce RTX 3080 Ti", 0x2208, Backend::Vulkan),
        gpu("NVIDIA GeForce RTX 3080", 0x2206, Backend::Dx12),
        gpu("NVIDIA GeForce RTX 3080", 0x2206, Backend::Vulkan),
        gpu("NVIDIA GeForce RTX 3080", 0x2216, Backend::Vulkan),
    ];
    for (k, a) in found.iter().enumerate() {
        assert_eq!(same_adapter(&found, a), Some(k), "{}", a.name);
    }
    assert_eq!(same_adapter(&found, &gpu("NVIDIA GeForce RTX 308", 0x2206, Backend::Vulkan)), None);
    assert_eq!(same_adapter(&found, &gpu("nvidia geforce rtx 3080", 0x2206, Backend::Vulkan)), None);
}

#[test]
fn every_backend_draws_3d_and_none_warns_about_it() {
    use sr_gpu::gpu::three_d_warning;
    for a in [
        rtx_through_gl(),
        info("llvmpipe (LLVM 19.1.7, 256 bits)", DeviceType::Cpu, Backend::Gl),
        lavapipe(),
        info("RTX 4050", DeviceType::DiscreteGpu, Backend::Dx12),
        info("Apple M2", DeviceType::IntegratedGpu, Backend::Metal),
    ] {
        assert!(can_run_3d(&a), "{} ({:?})", a.name, a.backend);
        assert_eq!(three_d_warning(&a), None, "{} ({:?})", a.name, a.backend);
        assert!(sr_gpu::gpu::require_3d(&a).is_ok(), "{} ({:?})", a.name, a.backend);
    }
}

fn rtx_through_gl() -> AdapterInfo {
    info("D3D12 (NVIDIA GeForce RTX 4050 Laptop GPU)", DeviceType::Other, Backend::Gl)
}

#[test]
fn a_document_with_3d_takes_the_gpu_behind_opengl_before_software_vulkan() {
    // WSL2 with Mesa's D3D12 driver: the GPU is reachable only through OpenGL, and it draws the 3D pass, so a 3D
    // document takes it like a flat one rather than lavapipe
    let found = [rtx_through_gl(), lavapipe()];
    assert_eq!(choose_adapter_for(&found, None, false).unwrap(), 0);
    assert_eq!(choose_adapter_for(&found, None, true).unwrap(), 0);
}

#[test]
fn a_3d_document_picks_the_best_ranked_adapter() {
    let found = [rtx_through_gl(), lavapipe(), info("Radeon 780M", DeviceType::IntegratedGpu, Backend::Vulkan)];
    assert_eq!(choose_adapter_for(&found, None, true).unwrap(), 2, "an integrated GPU beats an unknown one");
    assert_eq!(choose_adapter_for(&found, None, true).unwrap(), choose_adapter(&found, None).unwrap());
}

#[test]
fn a_3d_document_with_only_opengl_adapters_takes_one() {
    assert_eq!(choose_adapter_for(&[rtx_through_gl()], None, true).unwrap(), 0);
    assert!(matches!(choose_adapter_for(&[], None, true), Err(GpuError::NoAdapter(_))));
}

#[test]
fn naming_an_opengl_adapter_for_a_3d_document_takes_it() {
    let found = [rtx_through_gl(), lavapipe()];
    assert_eq!(choose_adapter_for(&found, Some("RTX"), true).unwrap(), 0);
    assert_eq!(choose_adapter_for(&found, Some("llvmpipe"), true).unwrap(), 1);
    assert_eq!(choose_adapter_for(&found, Some("RTX"), false).unwrap(), 0);
    // an unknown name is still reported as unknown
    assert!(matches!(choose_adapter_for(&found, Some("nope"), true), Err(GpuError::NoMatch { .. })));
}

#[test]
fn can_run_3d_agrees_with_the_3d_warning() {
    use sr_gpu::gpu::three_d_warning;
    for a in [rtx_through_gl(), lavapipe(), info("RTX 4050", DeviceType::DiscreteGpu, Backend::Dx12)] {
        assert_eq!(can_run_3d(&a), three_d_warning(&a).is_none(), "{}", a.name);
    }
}

/// Format features with every usage and flag, except `usages` and `flags`, which `format` lacks.
fn features_without(
    format: wgpu::TextureFormat,
    usages: wgpu::TextureUsages,
    flags: wgpu::TextureFormatFeatureFlags,
) -> impl Fn(wgpu::TextureFormat) -> wgpu::TextureFormatFeatures {
    move |f| {
        let mut all = wgpu::TextureFormatFeatures {
            allowed_usages: wgpu::TextureUsages::all(),
            flags: wgpu::TextureFormatFeatureFlags::all(),
        };
        if f == format {
            all.allowed_usages.remove(usages);
            all.flags.remove(flags);
        }
        all
    }
}

#[test]
fn an_adapter_with_every_format_feature_can_run_the_3d_pass() {
    use sr_gpu::gpu::check_3d_formats;
    let none = (wgpu::TextureUsages::empty(), wgpu::TextureFormatFeatureFlags::empty());
    assert!(check_3d_formats(features_without(wgpu::TextureFormat::Depth32Float, none.0, none.1)).is_ok());
    // a format the 3D pass does not multisample is not asked for anything
    let unused = features_without(
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureUsages::all(),
        wgpu::TextureFormatFeatureFlags::all(),
    );
    assert!(check_3d_formats(unused).is_ok());
}

#[test]
fn an_adapter_without_the_multisampled_depth_or_colour_the_3d_pass_needs_is_refused() {
    use sr_gpu::gpu::check_3d_formats;
    use wgpu::{TextureFormat as F, TextureFormatFeatureFlags as Flag, TextureUsages as U};
    let cases = [
        (F::Depth32Float, U::empty(), Flag::MULTISAMPLE_X4),
        (F::Depth32Float, U::TEXTURE_BINDING, Flag::empty()),
        (F::Depth32Float, U::RENDER_ATTACHMENT, Flag::empty()),
        (F::Rgba16Float, U::empty(), Flag::MULTISAMPLE_X4),
        (F::Rgba16Float, U::empty(), Flag::MULTISAMPLE_RESOLVE),
        (F::Rgba16Float, U::RENDER_ATTACHMENT, Flag::empty()),
    ];
    for (format, usages, flags) in cases {
        match check_3d_formats(features_without(format, usages, flags)) {
            Err(e @ GpuError::Format { format: refused, .. }) => {
                assert_eq!(refused, format, "{format:?} without {usages:?} {flags:?}");
                let message = e.to_string();
                assert!(message.contains(&format!("{format:?}")), "{message}");
                assert!(message.contains("3D"), "{message}");
            }
            other => panic!("{format:?} without {usages:?} {flags:?}: {other:?}"),
        }
    }
}

#[test]
fn an_opengl_driver_older_than_4_3_or_es_3_1_is_refused_naming_what_its_shaders_lack() {
    let gl = |version: &str| AdapterInfo {
        driver_info: version.into(),
        ..info("GPU", DeviceType::DiscreteGpu, Backend::Gl)
    };
    // the first is what the NVIDIA driver gives an EGL request for a desktop 3.3 context
    for version in [
        "3.3.0 NVIDIA 615.71.09",
        "4.2.0 NVIDIA 615.71.09",
        "3.3 (Core Profile) Mesa 25.0.7",
        "OpenGL ES 3.0 Mesa 25.0.7",
        "OpenGL ES 3.0.0 (ANGLE 2.1.0)",
    ] {
        match check_gl_version(&gl(version)) {
            Err(e @ GpuError::OpenGlVersion { .. }) => {
                let message = e.to_string();
                assert!(message.contains(version), "{message}");
                assert!(message.contains("storage buffers") && message.contains("compute shaders"), "{message}");
                assert!(message.contains("SR_GPU_ADAPTER"), "{message}");
            }
            other => panic!("{version}: {other:?}"),
        }
    }
    for version in [
        "4.5 (Core Profile) Mesa 25.0.7",
        "4.6.0 NVIDIA 615.71.09",
        "4.3.0",
        "OpenGL ES 3.1 Mesa 25.0.7",
        "OpenGL ES 3.2 NVIDIA 615.71.09",
    ] {
        assert!(check_gl_version(&gl(version)).is_ok(), "{version}");
    }
    // another backend's driver version says nothing of OpenGL, and a version this cannot read is left to the probe of
    // the device
    let vulkan = AdapterInfo { driver_info: "3.3.0".into(), ..info("RTX", DeviceType::DiscreteGpu, Backend::Vulkan) };
    assert!(check_gl_version(&vulkan).is_ok());
    assert!(check_gl_version(&gl("")).is_ok());
}
