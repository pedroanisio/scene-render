//! Renderers created from several threads at once, as a parallel delivery does: on a software adapter, creating pipelines
//! concurrently crashed the Vulkan loader (`vkSetDebugUtilsObjectNameEXT` in the loader, SIGSEGV), and so did listing the
//! adapters while another thread did (`vkEnumerateInstanceExtensionProperties` called through a null pointer while the
//! OpenGL backend loaded libEGL, SIGSEGV).

use super::common;

use sr_gpu::Renderer;

fn program() -> sr_eval::Evaluator {
    let xml = r##"<scene version="1.1"><project width="64" height="36" fps="10" duration="1" background="#000000"/><composition>
        <shape id="r" shape="rect" width="8" height="8" fill="#FF0000"/></composition></scene>"##;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    sr_eval::Evaluator::new(&doc, &Default::default()).unwrap()
}

#[test]
fn renderers_can_be_created_from_many_threads_without_crashing() {
    let Some(gpu) = common::gpu() else { return };
    // the crash is a software-adapter (llvmpipe) race: elsewhere there is nothing to stress
    if !gpu.is_software() {
        return;
    }
    let iterations: usize = std::env::var("SR_STRESS_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(50);
    let threads = 4;
    let ev = program();
    for round in 0..iterations {
        // devices are opened one after the other, as a parallel delivery does; the renderers are created together
        let gpus: Vec<_> = (0..threads).map(|_| gpu.open_like().unwrap()).collect();
        std::thread::scope(|s| {
            for g in gpus {
                let program = ev.program();
                s.spawn(move || {
                    let renderer = Renderer::new(g, program);
                    drop(renderer);
                });
            }
        });
        eprintln!("round {round} ok");
    }
}

#[test]
fn adapters_can_be_listed_from_many_threads_without_crashing() {
    // what test threads and a parallel delivery do when each opens its own device: every listing creates an instance,
    // which scans the Vulkan drivers and loads EGL
    use sr_gpu::gpu::{adapters, GpuOptions};
    let iterations: usize = std::env::var("SR_STRESS_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(20);
    let backends = [wgpu::Backends::VULKAN | wgpu::Backends::GL, wgpu::Backends::GL, wgpu::Backends::VULKAN];
    let counts = std::thread::scope(|s| {
        let workers: Vec<_> = (0..8)
            .map(|t| {
                let opts = GpuOptions { backends: backends[t % backends.len()], adapter: None };
                s.spawn(move || (0..iterations).map(|_| adapters(&opts).0.len()).collect::<Vec<_>>())
            })
            .collect();
        workers.into_iter().map(|w| w.join().expect("a listing thread panicked")).collect::<Vec<_>>()
    });
    for (t, c) in counts.iter().enumerate() {
        assert!(c.windows(2).all(|w| w[0] == w[1]), "thread {t} saw the adapter count change: {c:?}");
    }
}
