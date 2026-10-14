//! The file formats of the byte-level targets, cut and flipped at fixed places: every seed is truncated at
//! every length up to 128 bytes and at 384 more lengths spread to its end, and has single bits flipped (every
//! bit of its first 32 bytes, then 256 more spread over the rest). Each input must come back as a typed error
//! or a valid value (the targets assert what a valid value is), never as a panic, and the parse may hold at
//! most [`PEAK_LIMIT`] bytes at once. A dedicated test process: the allocator that measures this is global.
//! The count is kept per thread, so what other tests allocate and free meanwhile does not move it; the parsers
//! measured run on the calling thread.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::{Mutex, MutexGuard};

use sr_fuzz::targets;

thread_local! {
    /// Bytes this thread allocated and has not freed (negative after freeing what another thread allocated), and
    /// the most there were since the last reset. Const-initialised and without a destructor: reading them never
    /// allocates, so the allocator may.
    static LIVE: Cell<isize> = const { Cell::new(0) };
    static PEAK: Cell<isize> = const { Cell::new(0) };
}

fn grew(n: usize) {
    let _ = LIVE.try_with(|live| {
        let now = live.get().saturating_add_unsigned(n);
        live.set(now);
        let _ = PEAK.try_with(|peak| peak.set(peak.get().max(now)));
    });
}

fn shrank(n: usize) {
    let _ = LIVE.try_with(|live| live.set(live.get().saturating_sub_unsigned(n)));
}

struct Counting;

// SAFETY: every operation forwards its arguments unchanged to System and returns its result. The bookkeeping
// only updates thread-local cells, after a successful call; it never allocates and never touches the allocated
// memory.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        shrank(layout.size());
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, size) };
        if !p.is_null() {
            if size >= layout.size() {
                grew(size - layout.size());
            } else {
                shrank(layout.size() - size);
            }
        }
        p
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The most a parse of one input of at most 64 KiB may hold at once.
const PEAK_LIMIT: usize = 64 << 20;

/// One probe at a time: the panic hook that records where a target panicked is global.
fn exclusive() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// The most bytes `f` held at once on this thread beyond what was live when it started.
fn held(f: impl FnOnce()) -> usize {
    let base = LIVE.with(Cell::get);
    PEAK.with(|peak| peak.set(base));
    f();
    usize::try_from(PEAK.with(Cell::get).saturating_sub(base)).unwrap_or(0)
}

/// Lengths to cut a seed of `len` bytes to: each up to 128, then 384 spread evenly to `len`.
fn cuts(len: usize) -> Vec<usize> {
    let mut out: Vec<usize> = (0..=len.min(128)).collect();
    if len > 128 {
        out.extend((1..=384).map(|k| 128 + (len - 128) * k / 384));
    }
    out.dedup();
    out
}

/// (byte, bit) places to flip in a seed of `len` bytes: every bit of the first 32 bytes, then 256 spread evenly.
fn flips(len: usize) -> Vec<(usize, u32)> {
    let mut out: Vec<(usize, u32)> = (0..len.min(32)).flat_map(|i| (0..8).map(move |b| (i, b))).collect();
    if len > 32 {
        out.extend((0..256).map(|k| (32 + (len - 32) * k / 256, k as u32 % 8)));
    }
    out
}

/// The panic messages, with their places, of the inputs run since the last take.
static PANICS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Runs the target on `input`: `Err` says how it failed (a panic, or too much held).
fn probe(t: &targets::Target, input: &[u8]) -> Result<(), String> {
    let mut crashed = None;
    let peak = held(|| crashed = targets::guard(t.name, input, t.run));
    let place = std::mem::take(&mut *PANICS.lock().unwrap_or_else(|e| e.into_inner()));
    if let Some(k) = crashed {
        return Err(format!("panic: {} ({})", k.message, place.join("; ")));
    }
    if peak > PEAK_LIMIT {
        return Err(format!("held {peak} bytes at once, over {PEAK_LIMIT}"));
    }
    Ok(())
}

/// The seeds of target `name`, at least `min_seeds` of them, and every cut and flip of each: all must pass [`probe`].
fn cut_and_flip(name: &str, min_seeds: usize) {
    let _held = exclusive();
    let t = targets::target(name).unwrap_or_else(|| panic!("no target {name}"));
    let seeds = (t.seeds)();
    assert!(seeds.len() >= min_seeds, "{name}: {} seeds, expected at least {min_seeds}", seeds.len());
    std::panic::set_hook(Box::new(|info| PANICS.lock().unwrap_or_else(|e| e.into_inner()).push(info.to_string())));
    let mut failures = Vec::new();
    let mut inputs = 0;
    for (s, seed) in seeds.iter().enumerate() {
        let mut run = |what: String, input: &[u8]| {
            inputs += 1;
            if let Err(e) = probe(t, input) {
                failures.push(format!("seed {s} ({} bytes) {what}: {e}", seed.len()));
            }
        };
        run("as it is".into(), seed);
        for n in cuts(seed.len()) {
            run(format!("cut to {n} bytes"), &seed[..n]);
        }
        for (i, b) in flips(seed.len()) {
            let mut m = seed.clone();
            m[i] ^= 1 << b;
            run(format!("with bit {b} of byte {i} flipped"), &m);
        }
    }
    let _ = std::panic::take_hook();
    assert!(
        failures.is_empty(),
        "{name}: {} of {inputs} inputs failed:\n{}",
        failures.len(),
        failures.iter().take(8).cloned().collect::<Vec<_>>().join("\n")
    );
}

/// How many seeds of target `name` its parser reads to a value, not an error.
fn read(name: &str) -> usize {
    let t = targets::target(name).unwrap_or_else(|| panic!("no target {name}"));
    let seeds = (t.seeds)();
    let ok = |s: &&Vec<u8>| -> bool {
        match name {
            "lut" => {
                let text = String::from_utf8_lossy(s);
                ["cube", "3dl", "clf"].iter().any(|e| sr_gpu::fx::parse_lut(&text, e, "seed").is_ok())
            }
            "openvdb" => {
                sr_volume::openvdb::read(std::io::Cursor::new(s.as_slice()), sr_volume::CacheLimits::default()).is_ok()
            }
            "srvol" => sr_volume::Volume::read(s.as_slice(), sr_volume::CacheLimits::default()).is_ok(),
            "vox" => sr_3d::voxel::vox::parse(s, &Default::default()).is_ok(),
            "stroke-font" => sr_eval::stroke_font::parse_jhf(&String::from_utf8_lossy(s)).is_ok(),
            "ies" => sr_3d::light::Ies::parse(&String::from_utf8_lossy(s)).is_ok(),
            "icc" => {
                sr_media::icc::parse(s).is_ok()
                    || sr_media::heif::colour(s)
                        .iter()
                        .any(|c| matches!(c, sr_media::heif::Colr::Icc(p) if sr_media::icc::parse(p).is_ok()))
            }
            other => panic!("no reader for {other}"),
        }
    };
    seeds.iter().filter(ok).count()
}

#[test]
fn the_measure_sees_an_allocation_over_the_limit() {
    let big = held(|| drop(std::hint::black_box(vec![1u8; PEAK_LIMIT + 1])));
    assert!(big > PEAK_LIMIT, "a {}-byte vector measured {big}", PEAK_LIMIT + 1);
    let small = held(|| drop(std::hint::black_box(vec![1u8; 1 << 10])));
    assert!(small < 1 << 20, "a 1 KiB vector measured {small}");
}

#[test]
fn what_another_thread_frees_meanwhile_does_not_count_against_this_one() {
    let freed_elsewhere = std::hint::black_box(vec![1u8; 1 << 20]);
    let big = held(|| {
        std::thread::spawn(move || drop(freed_elsewhere)).join().unwrap();
        drop(std::hint::black_box(vec![1u8; PEAK_LIMIT + 1]));
    });
    assert!(big > PEAK_LIMIT, "a {}-byte vector measured {big} after another thread freed 1 MiB", PEAK_LIMIT + 1);
}

#[test]
fn the_probe_reports_a_panic_and_a_large_hold() {
    fn boom(_: &[u8]) {
        panic!("boom");
    }
    fn hog(_: &[u8]) {
        drop(std::hint::black_box(vec![0u8; PEAK_LIMIT * 2]));
    }
    let _held = exclusive();
    std::panic::set_hook(Box::new(|info| PANICS.lock().unwrap_or_else(|e| e.into_inner()).push(info.to_string())));
    let panics = probe(&targets::Target { name: "boom", text: false, seeds: Vec::new, run: boom }, b"x");
    let hogs = probe(&targets::Target { name: "hog", text: false, seeds: Vec::new, run: hog }, b"x");
    let _ = std::panic::take_hook();
    let panics = panics.unwrap_err();
    assert!(panics.contains("boom") && panics.contains("parsers.rs"), "{panics}");
    assert!(hogs.unwrap_err().contains("held"));
}

#[test]
fn cuts_and_flips_cover_short_seeds_whole_and_long_seeds_to_the_end() {
    assert_eq!(cuts(3), vec![0, 1, 2, 3]);
    let long = cuts(10_000);
    assert_eq!((long[..129].to_vec(), long.last().copied()), ((0..=128).collect(), Some(10_000)));
    assert!(long.windows(2).all(|w| w[0] < w[1]));
    assert_eq!(
        flips(2),
        vec![
            (0, 0),
            (0, 1),
            (0, 2),
            (0, 3),
            (0, 4),
            (0, 5),
            (0, 6),
            (0, 7),
            (1, 0),
            (1, 1),
            (1, 2),
            (1, 3),
            (1, 4),
            (1, 5),
            (1, 6),
            (1, 7)
        ]
    );
    let f = flips(10_000);
    assert_eq!(f.len(), 32 * 8 + 256);
    assert!(f.iter().all(|&(i, b)| i < 10_000 && b < 8));
}

#[test]
fn the_seeds_are_read_to_values() {
    for (name, at_least) in
        [("lut", 4), ("openvdb", 10), ("srvol", 3), ("vox", 2), ("stroke-font", 2), ("icc", 4), ("ies", 3)]
    {
        let n = read(name);
        assert!(n >= at_least, "{name}: {n} seeds read, expected at least {at_least}");
    }
}

#[test]
fn lut_inputs_cut_and_flipped() {
    cut_and_flip("lut", 4);
}

#[test]
fn openvdb_inputs_cut_and_flipped() {
    cut_and_flip("openvdb", 10);
}

#[test]
fn srvol_inputs_cut_and_flipped() {
    cut_and_flip("srvol", 3);
}

#[test]
fn vox_inputs_cut_and_flipped() {
    cut_and_flip("vox", 2);
}

#[test]
fn font_inputs_cut_and_flipped() {
    cut_and_flip("font", 2);
}

#[test]
fn stroke_font_inputs_cut_and_flipped() {
    cut_and_flip("stroke-font", 2);
}

#[test]
fn icc_inputs_cut_and_flipped() {
    cut_and_flip("icc", 4);
}

#[test]
fn ies_inputs_cut_and_flipped() {
    cut_and_flip("ies", 3);
}
