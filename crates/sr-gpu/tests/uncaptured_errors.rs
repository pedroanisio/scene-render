//! A validation error the device reports outside an error scope is an error of the run, never a panic: wgpu's
//! default handler panics, so the device keeps the error until `Gpu::take_error` (or the next frame) collects it.
//! A crate of its own, so the error it provokes cannot reach another test's frame.

mod common;
use common::*;

#[test]
fn an_invalid_operation_is_an_error_not_a_panic() {
    let Some(gpu) = gpu() else { return };
    let side = gpu.device.limits().max_texture_dimension_2d + 1;
    let invalid = || {
        gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("larger than the device allows"),
            size: wgpu::Extent3d { width: side, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
    };

    let _texture = invalid();
    match gpu.take_error() {
        Err(sr_gpu::gpu::GpuError::Uncaptured(e)) => assert!(!e.is_empty()),
        other => panic!("expected the device's error, got {other:?}"),
    }
    assert!(gpu.take_error().is_ok(), "an error is reported once");

    // the next frame reports it as one of its errors
    let _texture = invalid();
    let r = render(&doc(r#"width="4" height="4""#, "", r#"<layer id="l" asset="red"/>"#)).unwrap();
    assert!(r.stats.errors.iter().any(|e| e.contains("the GPU device reported an error")), "{:?}", r.stats.errors);
    assert!(gpu.take_error().is_ok());
}
