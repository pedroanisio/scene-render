//! Dedicated process: the bytes held at once while a long source is decoded to planes.

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};

static TRACK: AtomicBool = AtomicBool::new(false);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
struct Observed;
fn grew(size: usize) {
    let live = LIVE.fetch_add(size, Relaxed) + size;
    if TRACK.load(Relaxed) {
        PEAK.fetch_max(live, Relaxed);
    }
}
// SAFETY: all allocation operations delegate unchanged to System. Observation
// uses only atomics, never allocates, and does not access allocated memory.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for Observed {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        grew(layout.size());
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // a block that grows may be moved, the old one held until the copy is made; one that shrinks is not
        if size > layout.size() {
            grew(size);
            LIVE.fetch_sub(layout.size(), Relaxed);
        } else {
            LIVE.fetch_sub(layout.size() - size, Relaxed);
        }
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: Observed = Observed;

#[test]
fn a_source_decoded_to_planes_is_held_once() {
    if !common::ffmpeg_available("audio memory") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("sr-media-audio-memory-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    struct Clean(std::path::PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(dir.clone());
    let wav = dir.join("tone.wav");
    let made = std::process::Command::new(sr_media::ffmpeg())
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000:duration=20"])
        .args(["-ac", "2", "-c:a", "pcm_s16le"])
        .arg(&wav)
        .status()
        .unwrap();
    assert!(made.success());
    let decoded = 20 * 48_000 * 2 * std::mem::size_of::<f32>();

    let before = LIVE.load(Relaxed);
    PEAK.store(before, Relaxed);
    TRACK.store(true, Relaxed);
    let planar = sr_media::decode_audio_planar(&wav, 0, 48_000).unwrap();
    TRACK.store(false, Relaxed);
    let held = PEAK.load(Relaxed) - before;
    assert!(held <= decoded + decoded / 8 + (1 << 20), "held {held} bytes at once for {decoded} decoded");

    assert_eq!((planar.sample_rate, planar.channels, planar.planes.len()), (48_000, 2, 2));
    let interleaved = sr_media::decode_audio(&wav, 0, 48_000).unwrap();
    assert_eq!(planar.planes[0].len(), interleaved.frames());
    for (c, plane) in planar.planes.iter().enumerate() {
        assert!(plane.iter().zip(interleaved.samples.iter().skip(c).step_by(2)).all(|(a, b)| a == b), "channel {c}");
    }
}
