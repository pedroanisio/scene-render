//! Chunks of a video output rendered by remote workers (`sr_deliver::remote`). The workers here are in this
//! process: each renders its chunk with `Options::segment`, as `scene-render serve` does, on a device of its own.
//! It runs in its own test binary, like the other tests whose workers open devices, and its tests run one after
//! the other ([`alone`]): two deliveries that each open several devices are not safe at once in one process (run
//! together, about one run in ten stops for good; in separate processes, none of 120 did).

mod common;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex};

use sr_deliver::remote::{Part, PartDone, RemoteWorkers, Segment};

/// Held by each test while it runs.
fn alone() -> Option<std::sync::MutexGuard<'static, ()>> {
    static ONE: Mutex<()> = Mutex::new(());
    // SR_TEST_TOGETHER lets them run at once, to look at what stops
    std::env::var_os("SR_TEST_TOGETHER").is_none().then(|| ONE.lock().unwrap_or_else(|e| e.into_inner()))
}

fn gpu() -> Option<sr_gpu::Gpu> {
    common::delivery("remote worker tests")
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr-remote-workers-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Six seconds at 10 fps, lossless: a square that crosses the frame and a background that changes each second, so
/// every frame differs from the others and a chunk out of place or rendered at the wrong time shows.
fn scene(d: &Path) -> PathBuf {
    let xml = format!(
        r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="6" background="#000000"/>
          <output id="v" path="{}/v.mkv" codec="ffv1" audio="false"/>
          <composition>
            <shape id="back" shape="rect" width="64" height="36" x="0" y="0" fill="#200000">
              <animate property="fill"><key time="0" value="#200000"/><key time="6" value="#0000E0"/></animate>
            </shape>
            <shape id="w" shape="rect" width="8" height="8" y="14" fill="#FFFFFF">
              <animate property="x"><key time="0" value="0"/><key time="6" value="56"/></animate>
            </shape>
          </composition></scene>"##,
        d.display()
    );
    let path = d.join("scene.xml");
    std::fs::write(&path, xml).unwrap();
    path
}

fn decode(path: &Path) -> Vec<u8> {
    let out = std::process::Command::new(sr_media::ffmpeg())
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .unwrap();
    out.stdout
}

/// Workers in this process. Each call loads the document anew, as a worker on another machine has its own copy.
#[derive(Debug)]
struct InProcess {
    scene: PathBuf,
    gpus: Vec<sr_gpu::Gpu>,
    /// Calls that fail before a worker starts to succeed.
    failures: AtomicUsize,
    /// The chunks asked for, by worker.
    asked: Mutex<Vec<(usize, Part)>>,
}

impl RemoteWorkers for InProcess {
    fn names(&self) -> Vec<String> {
        (0..self.gpus.len()).map(|i| format!("in-process-{i}")).collect()
    }

    fn render(
        &self,
        worker: usize,
        output: &sr_model::model::Output,
        part: Part,
        to: &Path,
        cancelled: &AtomicBool,
        frame: &dyn Fn(),
    ) -> Result<PartDone, String> {
        self.asked.lock().unwrap().push((worker, part));
        if self.failures.load(Relaxed) > 0 {
            self.failures.fetch_sub(1, Relaxed);
            // a few frames were announced before the worker was lost: the coordinator must not count them
            frame();
            frame();
            return Err("the worker was lost".into());
        }
        let doc = sr_model::load_file(&self.scene, &sr_model::LoadOptions::default()).map_err(|e| format!("{e:?}"))?;
        let own = sr_model::model::Output { path: to.to_string_lossy().into_owned(), ..output.clone() };
        let cancel = Arc::new(AtomicBool::new(cancelled.load(Relaxed)));
        let opts = sr_deliver::Options {
            start: Some(output.start),
            end: output.end,
            upload: false,
            parallel: sr_deliver::Parallel::Count(1),
            segment: Some(Segment { part, cancel: Some(cancel) }),
            ..Default::default()
        };
        let r = sr_deliver::deliver(&doc, &own, Some(&self.gpus[worker]), &opts, &mut |_, _| frame())
            .map_err(|e| e.to_string())?;
        assert_eq!(r.frames, (part.last - part.first) as u64);
        Ok(PartDone { encoder: r.encoder, unsupported: r.unsupported })
    }
}

fn serial(name: &str, gpu: &sr_gpu::Gpu) -> Vec<u8> {
    let d = dir(name);
    let path = scene(&d);
    let doc = sr_model::load_file(&path, &sr_model::LoadOptions::default()).unwrap_or_else(|e| panic!("{e:?}"));
    let opts = sr_deliver::Options { parallel: sr_deliver::Parallel::Count(1), ..Default::default() };
    let r = sr_deliver::deliver(&doc, &doc.scene.outputs[0], Some(gpu), &opts, &mut |_, _| {}).unwrap();
    assert!(r.remote.is_empty());
    decode(&r.path)
}

#[test]
fn chunks_rendered_by_remote_workers_join_into_the_serial_frames() {
    let _alone = alone();
    let Some(gpu) = gpu() else { return };
    let reference = serial("reference", &gpu);
    assert_eq!(reference.len(), 60 * 64 * 36 * 3);

    // one worker here and two remote: three chunks of two seconds, taken from one queue
    let d = dir("shared");
    let path = scene(&d);
    let doc = sr_model::load_file(&path, &sr_model::LoadOptions::default()).unwrap_or_else(|e| panic!("{e:?}"));
    let workers = Arc::new(InProcess {
        scene: path.clone(),
        gpus: vec![gpu.open_like().unwrap(), gpu.open_like().unwrap()],
        failures: AtomicUsize::new(0),
        asked: Mutex::new(Vec::new()),
    });
    let opts = sr_deliver::Options {
        parallel: sr_deliver::Parallel::Count(1),
        remote: Some(workers.clone()),
        ..Default::default()
    };
    let mut last = (0, 0);
    let r = sr_deliver::deliver(&doc, &doc.scene.outputs[0], Some(&gpu), &opts, &mut |done, total| {
        assert!(done <= total && done >= last.0, "progress goes forward: {done} after {last:?}");
        last = (done, total);
    })
    .unwrap();
    assert_eq!(last, (60, 60));
    assert_eq!(r.frames, 60);
    assert!(decode(&r.path) == reference, "the frames joined from remote chunks differ from the serial ones");
    let asked = workers.asked.lock().unwrap().clone();
    assert!(!asked.is_empty(), "no remote worker was asked for a chunk");
    for (_, part) in &asked {
        assert_eq!((part.frames, part.first % 20, part.last - part.first), (60, 0, 20), "{part:?}");
    }
    let rendered: u64 = r.remote.iter().map(|s| s.frames).sum();
    assert_eq!(rendered, 20 * asked.len() as u64);
    assert_eq!(r.remote.len(), 2);
    assert!(r.remote.iter().all(|s| s.failed.is_none()), "{:?}", r.remote);
    assert_eq!(r.segments as usize, 1 + r.remote.iter().filter(|s| s.chunks > 0).count());
}

#[test]
fn a_chunk_a_remote_worker_fails_is_rendered_by_another() {
    let _alone = alone();
    let Some(gpu) = gpu() else { return };
    let reference = serial("reference-failed", &gpu);
    let d = dir("failed");
    let path = scene(&d);
    let doc = sr_model::load_file(&path, &sr_model::LoadOptions::default()).unwrap_or_else(|e| panic!("{e:?}"));
    let workers = Arc::new(InProcess {
        scene: path.clone(),
        gpus: vec![gpu.open_like().unwrap()],
        failures: AtomicUsize::new(1),
        asked: Mutex::new(Vec::new()),
    });
    let opts = sr_deliver::Options {
        parallel: sr_deliver::Parallel::Count(1),
        remote: Some(workers.clone()),
        ..Default::default()
    };
    let mut most = 0;
    let r = sr_deliver::deliver(&doc, &doc.scene.outputs[0], Some(&gpu), &opts, &mut |done, total| {
        assert!(done <= total);
        most = most.max(done);
    })
    .unwrap();
    assert_eq!(most, 60);
    assert!(decode(&r.path) == reference, "the frames differ after a worker failed");
    // the worker was asked once, failed, and took no more: every chunk was rendered here
    assert_eq!(workers.asked.lock().unwrap().len(), 1);
    assert_eq!((r.remote.len(), r.remote[0].chunks, r.remote[0].frames), (1, 0, 0));
    assert_eq!(r.remote[0].failed.as_deref(), Some("the worker was lost"));
    assert!(
        r.warnings.iter().any(|w| w.contains("in-process-0") && w.contains("the worker was lost")),
        "{:?}",
        r.warnings
    );
    assert_eq!(r.segments, 1);
}

#[test]
fn a_segment_of_another_frame_count_is_refused() {
    let _alone = alone();
    let Some(gpu) = gpu() else { return };
    let d = dir("refused");
    let path = scene(&d);
    let doc = sr_model::load_file(&path, &sr_model::LoadOptions::default()).unwrap_or_else(|e| panic!("{e:?}"));
    for part in [
        Part { first: 0, last: 20, frames: 61 },
        Part { first: 20, last: 20, frames: 60 },
        Part { first: 40, last: 61, frames: 60 },
    ] {
        let opts = sr_deliver::Options { segment: Some(Segment { part, cancel: None }), ..Default::default() };
        let e = sr_deliver::deliver(&doc, &doc.scene.outputs[0], Some(&gpu), &opts, &mut |_, _| {}).unwrap_err();
        assert!(e.to_string().contains("not the coordinator's"), "{part:?}: {e}");
    }
    // and one that is cancelled stops with an error, leaving no report of a finished file
    let cancel = Arc::new(AtomicBool::new(true));
    let part = Part { first: 0, last: 20, frames: 60 };
    let opts = sr_deliver::Options { segment: Some(Segment { part, cancel: Some(cancel) }), ..Default::default() };
    let e = sr_deliver::deliver(&doc, &doc.scene.outputs[0], Some(&gpu), &opts, &mut |_, _| {}).unwrap_err();
    assert!(e.to_string().contains("cancelled"), "{e}");
}
