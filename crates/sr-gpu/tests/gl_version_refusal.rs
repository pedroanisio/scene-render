//! An OpenGL adapter too old for the shaders is refused before its device opens.
//! One test per binary: should the refusal regress, the device it opens is dropped at the test's end, and no other
//! test thread may be creating or destroying a device beside it (see `tools/tests/test_shared_test_devices.py`).

#[test]
fn an_opengl_adapter_too_old_for_the_shaders_is_refused_before_its_device_opens() {
    // a driver whose newest context is older than OpenGL 4.3 (or OpenGL ES 3.1) has a shading language without storage
    // buffers or compute shaders: opening it is a typed refusal that names them, not a device that fails its first
    // pipeline
    use sr_gpu::gpu::{adapters, check_gl_version, GpuOptions};
    let (found, _) = adapters(&GpuOptions { backends: wgpu::Backends::GL, adapter: None });
    for info in found.iter().filter(|i| check_gl_version(i).is_err()) {
        let opts = GpuOptions { backends: wgpu::Backends::GL, adapter: Some(info.name.clone()) };
        match sr_gpu::Gpu::with_options(&opts) {
            Err(e @ sr_gpu::GpuError::OpenGlVersion { .. }) => eprintln!("refused {}: {e}", info.name),
            Err(e) => panic!("{} ({}): another refusal: {e}", info.name, info.driver_info),
            Ok(_) => panic!("{} ({}) opened", info.name, info.driver_info),
        }
    }
}
