//! The adapters can be listed while the document loads: `Gpu::start` begins on a thread, `finish` picks the adapter for the
//! document and opens it. One test per binary: it opens devices one after the other, and no other test thread may be
//! creating or destroying a device beside it (see `tools/tests/test_shared_test_devices.py`).

use sr_gpu::gpu::GpuOptions;

#[test]
fn a_started_open_chooses_the_adapter_that_a_direct_open_chooses_and_an_unused_start_frees_the_lock() {
    let opts = GpuOptions::from_env();
    let Ok(direct) = sr_gpu::Gpu::with_options_for(&opts, true) else {
        eprintln!("no usable adapter: skipped");
        return;
    };
    let started = sr_gpu::Gpu::start(&opts).finish(true).expect("the started open");
    assert_eq!(started.info.name, direct.info.name);
    assert_eq!(started.info.backend, direct.info.backend);
    drop((started, direct));

    // a start nobody finishes (a document with no picture) gives the creation lock back
    drop(sr_gpu::Gpu::start(&opts));
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        drop(sr_gpu::gpu::creation_lock());
        tx.send(()).unwrap();
    });
    rx.recv_timeout(std::time::Duration::from_secs(60)).expect("the creation lock was released");
}
