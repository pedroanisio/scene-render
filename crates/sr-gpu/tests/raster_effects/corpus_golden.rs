//! Corpus golden frames: documents from tests/corpus/valid, spanning 2D vector art, text, 3D, volumes, colour
//! management and compositing, are each rendered at a fixed time, reduced to a thumbnail of at most
//! [`THUMBNAIL_SIDE`] px on the long side, and compared channel by channel with a PNG in tests/golden/corpus under
//! [`CORPUS_TOLERANCE`] (max, mean and 99th-percentile error per channel).
//!
//! Regenerating the references is always explicit and local, after reviewing the frames:
//! `SR_GPU_ADAPTER=llvmpipe SR_BLESS=1 cargo test -p sr-gpu --test raster_effects corpus_golden` writes the current
//! frames as the references, and their hashes then go into provenance.json. It is refused under `CI`, and a missing
//! or mismatching reference fails without being written. The references come from llvmpipe;
//! tests/golden/corpus/provenance.json records each frame's decoded pixels and the adapters (name, backend and
//! driver) whose output was verified against the set. On any other adapter a scene still renders and must report
//! only its declared gaps, and its metrics are written, but the comparison is skipped with a `SKIPPED` line naming
//! the adapter (a failure under `SR_REQUIRE_GOLDEN_ADAPTER=1`). Each comparison leaves `<scene>.actual.png`,
//! `<scene>.expected.png`, `<scene>.diff-x16.png` and `<scene>.report.json` in `SR_GOLDEN_OUT` (default
//! target/golden-output), under `corpus/`.

use super::common;
use super::golden::{bless_requested, diff_x16, golden_dir, in_ci, output_dir, reference};
use sr_gpu::golden::{channel_errors, distinctness, spread, thumbnail, CORPUS_TOLERANCE};
use std::path::{Path, PathBuf};

/// Long side of a corpus golden, in pixels. Large enough that a misplaced element, a wrong colour transform or a
/// missing layer moves many pixels; small enough that a reference set stays reviewable at a glance.
const THUMBNAIL_SIDE: u32 = 64;

/// Largest PNG accepted anywhere under tests/golden, in bytes. A 64×64 RGBA8 thumbnail is 16 KiB uncompressed, so
/// every corpus golden fits even when PNG cannot compress it at all, and the 128×72 kitchen frames (about 13 KB)
/// fit too. A frame committed at full resolution does not: a 480×270 frame is about 506 KiB uncompressed and
/// rarely compresses below this cap, so a bless that skipped the thumbnail fails here rather than bloating history.
const MAX_GOLDEN_BYTES: u64 = 32 * 1024;

/// Smallest [`distinctness`] allowed between two corpus goldens of one set, in code values. A frame that passes
/// [`CORPUS_TOLERANCE`] is within a mean error of 0.5 of its golden in every channel, and averaging over the
/// distinctness grid cannot increase a difference, so one frame could pass against two goldens closer than 1.0: such
/// a pair cannot tell a regression that swaps them. 2.0 keeps twice that margin and still sits below the closest pair
/// of legitimately different scenes (dark frames whose content is small at thumbnail size).
const MIN_DISTINCTNESS: f64 = 2.0;

/// Smallest [`spread`] allowed in a corpus golden, in code values: a blank or flat frame fails.
const MIN_SPREAD: u8 = 16;

/// What a corpus scene is in the suite to cover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Area {
    Vector,
    Text,
    ThreeD,
    Volume,
    Colour,
    Compositing,
}

const AREAS: [Area; 6] = [Area::Vector, Area::Text, Area::ThreeD, Area::Volume, Area::Colour, Area::Compositing];

struct Scene {
    /// The test's name, and the golden's file stem.
    name: &'static str,
    /// File stem in tests/corpus/valid, without `.scene.xml`.
    stem: &'static str,
    /// Time rendered, seconds.
    time: f64,
    covers: &'static [Area],
    /// Unsupported content the renderer is known to report for this scene, each the start of one message (the node
    /// and the start of its reason). The frame still renders and is compared; any other report fails.
    gaps: &'static [&'static str],
    /// Render errors known for this scene, matched like `gaps`: only the corpus's stand-in media fixtures cause them.
    errors: &'static [&'static str],
}

/// The corpus reference set.
fn corpus_dir() -> PathBuf {
    golden_dir().join("corpus")
}

fn scene_path(stem: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/valid").join(format!("{stem}.scene.xml"))
}

/// Renders corpus scene `stem` at `time`, at the frame size the document declares, and compares its thumbnail with
/// golden `name`.
fn check(name: &str, stem: &str, time: f64, gaps: &[&str], errors: &[&str]) {
    // Without an adapter, `gpu` has skipped (or failed under `SR_REQUIRE_GPU=1`).
    let Some(gpu) = common::gpu() else { return };
    let doc = sr_model::load_file(scene_path(stem), &sr_model::LoadOptions::default())
        .unwrap_or_else(|e| panic!("{stem}: {e:?}"));
    let ev =
        sr_eval::Evaluator::new(&doc, &sr_eval::EvalOptions::default()).unwrap_or_else(|e| panic!("{stem}: {e:?}"));
    let mut r = sr_gpu::Renderer::new(gpu.clone(), ev.program());
    let g = ev.evaluate(time);
    let mut sub = |st: f64| ev.evaluate(st);
    let f = r.render_with(&g, ev.program(), Some(&mut sub));
    // Undeclared unsupported content or render errors, and stale declarations, fail even when blessing.
    let mut undeclared = known_problems("unsupported content", &f.stats.unsupported, gaps);
    undeclared.extend(known_problems("render error", &f.stats.errors, errors));
    assert!(undeclared.is_empty(), "{stem} at {time} s: {undeclared:#?}");
    let (px, size) = thumbnail(&r.read(&f.texture), f.texture.size, THUMBNAIL_SIDE).unwrap();
    let actual = image::RgbaImage::from_raw(size[0], size[1], r.to_srgb8(&px)).unwrap();
    let out = output_dir("corpus");
    actual.save(out.join(format!("{name}.actual.png"))).unwrap();

    let set = corpus_dir();
    let bless = bless_requested();
    if bless && !in_ci() {
        std::fs::create_dir_all(&set).unwrap();
    }
    let golden = match reference(&set.join(format!("{name}.png")), &actual, bless, in_ci()) {
        Ok(Some(golden)) => golden,
        Ok(None) => return,
        Err(e) => panic!("{name}: {e}"),
    };
    let adapter = AdapterId::of(&gpu.info);
    let verified = verified_adapters(&set).unwrap_or_else(|e| panic!("{name}: {e}")).iter().any(|v| adapter.is(v));
    golden.save(out.join(format!("{name}.expected.png"))).unwrap();
    diff_x16(&actual, &golden).save(out.join(format!("{name}.diff-x16.png"))).unwrap();
    let channels = channel_errors(actual.as_raw(), golden.as_raw()).unwrap();
    let breaches = CORPUS_TOLERANCE.breaches(&channels);
    let report = serde_json::json!({"golden": name, "scene": stem, "time": time, "size": size, "channels": channels,
        "tolerance": CORPUS_TOLERANCE, "breaches": breaches, "gaps": gaps, "errors": errors,
        "adapter": format!("{:?}", gpu.info), "verified_adapter": verified});
    std::fs::write(out.join(format!("{name}.report.json")), serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    if !verified {
        // The frame rendered and reported only its declared gaps; whether its pixels match is unknown on this adapter.
        sr_testkit::GOLDEN_ADAPTER.skip(
            &format!("corpus_golden::{name}"),
            format!(
                "{adapter} is not among the adapters verified against {}; metrics in {}",
                set.join("provenance.json").display(),
                out.join(format!("{name}.report.json")).display()
            ),
        );
        return;
    }
    assert!(breaches.is_empty(), "{name} ({stem} at {time} s): {breaches:?}; images and metrics: {}", out.display());
}

/// An adapter as provenance.json records it: wgpu's adapter name, backend and driver information.
#[derive(Debug, PartialEq, Eq)]
struct AdapterId {
    name: String,
    backend: String,
    driver: String,
}

impl AdapterId {
    fn of(info: &wgpu::AdapterInfo) -> Self {
        AdapterId { name: info.name.clone(), backend: format!("{:?}", info.backend), driver: info.driver_info.clone() }
    }

    /// Whether `v` records this adapter: name, backend and driver all equal, since a driver release can change
    /// rasterisation and filtering as much as a change of hardware can.
    fn is(&self, v: &VerifiedAdapter) -> bool {
        self.name == v.adapter && self.backend == v.backend && self.driver == v.driver
    }
}

impl std::fmt::Display for AdapterId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "adapter {:?} ({}, driver {:?})", self.name, self.backend, self.driver)
    }
}

/// The adapters `set`'s provenance.json records as verified against it.
fn verified_adapters(set: &Path) -> Result<Vec<VerifiedAdapter>, String> {
    let path = set.join("provenance.json");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let record: Provenance = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(record.verified_adapters)
}

#[test]
fn only_a_recorded_name_backend_and_driver_is_a_verified_adapter() {
    let recorded = VerifiedAdapter {
        adapter: "llvmpipe (LLVM 19.1.7, 256 bits)".into(),
        backend: "Vulkan".into(),
        driver: "Mesa 25.0.7-2+deb13u1 (LLVM 19.1.7)".into(),
    };
    let id = |name: &str, backend: &str, driver: &str| AdapterId {
        name: name.into(),
        backend: backend.into(),
        driver: driver.into(),
    };
    assert!(id(&recorded.adapter, "Vulkan", &recorded.driver).is(&recorded));
    // the same name on another Mesa release, another backend, or another LLVM width is not verified
    assert!(!id(&recorded.adapter, "Vulkan", "Mesa 24.2.8-1ubuntu1~24.04.1 (LLVM 19.1.1)").is(&recorded));
    assert!(!id(&recorded.adapter, "Gl", &recorded.driver).is(&recorded));
    assert!(!id("llvmpipe (LLVM 19.1.7, 128 bits)", "Vulkan", &recorded.driver).is(&recorded));
    let shown = id("llvmpipe", "Vulkan", "Mesa").to_string();
    assert_eq!(shown, r#"adapter "llvmpipe" (Vulkan, driver "Mesa")"#);
}

/// A scene's declared gaps or errors: none, or the constant that lists them.
macro_rules! declared {
    () => {
        &[]
    };
    ($list:ident) => {
        $list
    };
}

// Each scene is its own libtest case, so CI distributes the suite across workers and a failure names its scene.
macro_rules! corpus_goldens {
    ($($test:ident: $stem:literal at $time:literal covers [$($area:ident)+] $(gaps $gaps:ident)? $(errors $errors:ident)?;)*) => {
        const SCENES: &[Scene] = &[$(Scene {
            name: stringify!($test),
            stem: $stem,
            time: $time,
            covers: &[$(Area::$area),+],
            gaps: declared!($($gaps)?),
            errors: declared!($($errors)?),
        }),*];
        $(#[test] fn $test() {
            let s = SCENES.iter().find(|s| s.name == stringify!($test)).unwrap();
            check(s.name, s.stem, s.time, s.gaps, s.errors);
        })*
    };
}

/// Unsupported content the kitchen sink reports at both of its times: its font fixture stands in for a real font
/// (a few bytes, so text falls back), and four of its nodes need map data or bodies the corpus does not ship.
const KITCHEN_GAPS: &[&str] = &[
    "atlas: the raster basemap is drawn beneath",
    "font asset inter: ",
    "ground: terrain and buildings need a web-mercator map",
    "pin1: constraint body sparks has no rigidBody",
    "socket: constraint body earth has no 3D rigidBody",
];
/// The render error the kitchen sink reports at both of its times: its transcript fixture is not JSON, so the
/// automatic captions draw nothing. Its video fixture is not a video either, which is why neither time is one where
/// the video is on screen (decoding it would fail, and how it fails depends on the host's ffprobe).
const KITCHEN_ERRORS: &[&str] = &["auto: transcript line 1: "];

corpus_goldens! {
    gm_key: "gm-key" at 2.0 covers [Vector];
    stroke_text: "stroke-text" at 1.5 covers [Text Vector];
    connector: "connector" at 1.0 covers [Vector Text];
    repeat_points: "repeat-points" at 1.0 covers [Vector];
    fractal: "fractal" at 0.5 covers [Vector Colour];
    pdf_region: "pdf-region" at 1.0 covers [Vector Compositing];
    flock_upright: "flock-upright" at 0.5 covers [Vector];
    text3d_tracking: "text3d-tracking" at 1.0 covers [Text ThreeD];
    metal_without_environment: "metal-without-environment" at 0.5 covers [ThreeD];
    subsurface_sky_maps: "srep71-subsurface-sky-maps" at 0.5 covers [ThreeD Colour];
    viewport3d: "viewport3d" at 0.5 covers [ThreeD Compositing];
    black_hole_isco: "black-hole-disk-at-isco" at 1.0 covers [ThreeD];
    globe_relief: "globe-relief" at 1.0 covers [ThreeD];
    mesh_sequence: "mesh-sequence" at 1.0 covers [ThreeD];
    parametric: "parametric" at 0.5 covers [ThreeD];
    voxel_fracture: "voxel-fracture" at 2.0 covers [ThreeD];
    particles3d_gas: "particles3d-gas" at 1.0 covers [ThreeD Volume];
    volume: "volume" at 0.5 covers [Volume];
    advected_volume: "advected-volume" at 0.5 covers [Volume];
    thermal_volume: "thermal-volume" at 0.5 covers [Volume Colour];
    volume_scatter_bounces: "volume-scatter-bounces" at 0.5 covers [Volume];
    kitchen_sink_early: "kitchen-sink" at 1.0 covers [Colour Compositing] gaps KITCHEN_GAPS errors KITCHEN_ERRORS;
    kitchen_sink_late: "kitchen-sink" at 11.0 covers [Colour Compositing] gaps KITCHEN_GAPS errors KITCHEN_ERRORS;
}

/// Problems with one reference set: missing or extra goldens, oversized thumbnails, flat frames, and pairs of
/// frames too alike to tell a regression that swaps or blanks them.
fn set_problems(set: &Path, names: &[&str]) -> Vec<String> {
    let mut problems = Vec::new();
    let mut frames = Vec::new();
    for name in names {
        let path = set.join(format!("{name}.png"));
        let Ok(img) = image::open(&path) else {
            problems.push(format!("{}: missing or unreadable", path.display()));
            continue;
        };
        let img = img.to_rgba8();
        let size = [img.width(), img.height()];
        if size[0].max(size[1]) > THUMBNAIL_SIDE {
            problems.push(format!("{}: {size:?} exceeds {THUMBNAIL_SIDE} px on its long side", path.display()));
        }
        match spread(img.as_raw()) {
            Ok(s) if s >= MIN_SPREAD => {}
            Ok(s) => problems.push(format!("{}: spread {s} < {MIN_SPREAD}, the frame is flat", path.display())),
            Err(e) => problems.push(format!("{}: {e}", path.display())),
        }
        frames.push((name, img, size));
    }
    for (i, (a, ia, sa)) in frames.iter().enumerate() {
        for (b, ib, sb) in &frames[i + 1..] {
            match distinctness(ia.as_raw(), *sa, ib.as_raw(), *sb) {
                Ok(d) if d > MIN_DISTINCTNESS => {}
                Ok(d) => problems.push(format!("{}: {a} and {b} differ by {d:.2} ≤ {MIN_DISTINCTNESS}", set.display())),
                Err(e) => problems.push(format!("{}: {a} and {b}: {e}", set.display())),
            }
        }
    }
    let expected: Vec<String> = names.iter().map(|s| format!("{s}.png")).collect();
    if let Ok(entries) = std::fs::read_dir(set) {
        for name in entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()) {
            if name.ends_with(".png") && !expected.contains(&name) {
                problems.push(format!("{}: {name} belongs to no corpus scene", set.display()));
            }
        }
    }
    problems
}

/// Every PNG under `dir`, recursively, larger than `cap` bytes.
fn oversized(dir: &Path, cap: u64) -> Vec<String> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(d) = pending.pop() {
        for entry in std::fs::read_dir(&d).unwrap_or_else(|e| panic!("{}: {e}", d.display())).flatten() {
            let path = entry.path();
            let meta = entry.metadata().unwrap();
            if meta.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|x| x == "png") && meta.len() > cap {
                found.push(format!("{}: {} bytes > {cap}", path.display(), meta.len()));
            }
        }
    }
    found.sort();
    found
}

fn names() -> Vec<&'static str> {
    SCENES.iter().map(|s| s.name).collect()
}

#[test]
fn the_corpus_reference_set_is_complete_small_and_distinct() {
    let problems = set_problems(&corpus_dir(), &names());
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// tests/golden/corpus/provenance.json: where and how the references were made, and the pixels each one holds.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)] // the descriptive fields are read by people; the test checks their presence and the frames
struct Provenance {
    review_date: String,
    adapter: String,
    os: String,
    method: String,
    regenerate: String,
    verified_adapters: Vec<VerifiedAdapter>,
    frames: Vec<ProvenanceFrame>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct VerifiedAdapter {
    adapter: String,
    backend: String,
    driver: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ProvenanceFrame {
    file: String,
    scene: String,
    time: f64,
    decoded_rgba_sha256: String,
}

/// SHA-256 of a PNG's decoded RGBA8 pixels, in lower-case hex: the same picture hashes alike however it is encoded.
fn decoded_rgba_sha256(path: &Path) -> Result<String, String> {
    use sha2::Digest;
    let img = image::open(path).map_err(|e| format!("{}: {e}", path.display()))?.to_rgba8();
    Ok(sha2::Sha256::digest(img.as_raw()).iter().map(|b| format!("{b:02x}")).collect())
}

/// Disagreements between a provenance record and the reference set it describes: every golden is recorded once,
/// with its scene, time and decoded pixels, and nothing else is.
fn provenance_problems(set: &Path, scenes: &[Scene]) -> Vec<String> {
    let path = set.join("provenance.json");
    let record: Provenance = match std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|text| serde_json::from_str(&text).map_err(|e| e.to_string()))
    {
        Ok(record) => record,
        Err(e) => return vec![format!("{}: {e}", path.display())],
    };
    let mut problems = Vec::new();
    if record.verified_adapters.is_empty() {
        problems.push(format!("{}: no verified adapter", path.display()));
    }
    for s in scenes {
        let file = format!("{}.png", s.name);
        let entries: Vec<_> = record.frames.iter().filter(|f| f.file == file).collect();
        let [entry] = entries[..] else {
            problems.push(format!("{file}: recorded {} times", entries.len()));
            continue;
        };
        if entry.scene != s.stem || entry.time != s.time {
            problems.push(format!(
                "{file}: recorded as {} at {} s, listed as {} at {} s",
                entry.scene, entry.time, s.stem, s.time
            ));
        }
        match decoded_rgba_sha256(&set.join(&file)) {
            Ok(sha) if sha == entry.decoded_rgba_sha256 => {}
            Ok(sha) => {
                problems.push(format!("{file}: pixels hash to {sha}, provenance records {}", entry.decoded_rgba_sha256))
            }
            Err(e) => problems.push(e),
        }
    }
    for f in &record.frames {
        if !scenes.iter().any(|s| f.file == format!("{}.png", s.name)) {
            problems.push(format!("{}: recorded but belongs to no corpus scene", f.file));
        }
    }
    problems
}

#[test]
fn the_corpus_provenance_records_every_golden() {
    let problems = provenance_problems(&corpus_dir(), SCENES);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn a_provenance_that_disagrees_with_its_set_is_reported() {
    let set = scratch_set("provenance");
    ramp([255, 0, 0]).save(set.join("red.png")).unwrap();
    ramp([0, 0, 255]).save(set.join("blue.png")).unwrap();
    let scene = |name| Scene { name, stem: "gm-key", time: 1.0, covers: &[Area::Vector], gaps: &[], errors: &[] };
    let scenes = [scene("red"), scene("blue"), scene("absent")];
    let red = decoded_rgba_sha256(&set.join("red.png")).unwrap();
    let write = |frames: &str| {
        let text = format!(
            r#"{{"review_date": "d", "adapter": "a", "os": "o", "method": "m",
            "regenerate": "r", "verified_adapters": [{{"adapter": "a", "backend": "Vulkan", "driver": "d"}}],
            "frames": [{frames}]}}"#
        );
        std::fs::write(set.join("provenance.json"), text).unwrap();
    };
    let frame = |file: &str, stem: &str, sha: &str| {
        format!(r#"{{"file": "{file}", "scene": "{stem}", "time": 1.0, "decoded_rgba_sha256": "{sha}"}}"#)
    };
    write(
        &[frame("red.png", "gm-key", &red), frame("blue.png", "gm-bar", &red), frame("stray.png", "gm-key", &red)]
            .join(","),
    );
    let problems = provenance_problems(&set, &scenes);
    let has = |needle: &str| problems.iter().any(|p| p.contains(needle));
    assert!(has("blue.png: recorded as gm-bar at 1 s"), "{problems:#?}");
    assert!(has("blue.png: pixels hash to"), "{problems:#?}");
    assert!(has("absent.png: recorded 0 times"), "{problems:#?}");
    assert!(has("stray.png: recorded but belongs to no corpus scene"), "{problems:#?}");
    assert!(!has("red.png"), "{problems:#?}");
    assert_eq!(problems.len(), 4, "{problems:#?}");
    // an unknown field, a missing field and a record that is not JSON are each refused, never read around
    write(&format!(r#"{}, "extra": 1"#, frame("red.png", "gm-key", &red)).replace(r#""}, "extra""#, r#"", "extra""#));
    assert!(provenance_problems(&set, &scenes[..1])[0].contains("unknown field `extra`"));
    std::fs::write(set.join("provenance.json"), r#"{"frames": []}"#).unwrap();
    assert!(provenance_problems(&set, &scenes[..1])[0].contains("missing field"));
    std::fs::write(set.join("provenance.json"), "frames: none").unwrap();
    assert_eq!(provenance_problems(&set, &scenes[..1]).len(), 1);
    // a record with no verified adapter
    write(&frame("red.png", "gm-key", &red));
    let text = std::fs::read_to_string(set.join("provenance.json")).unwrap();
    let start = text.find("[{\"adapter\"").unwrap();
    let end = text[start..].find("}]").unwrap() + start + 2;
    std::fs::write(set.join("provenance.json"), format!("{}[]{}", &text[..start], &text[end..])).unwrap();
    assert_eq!(
        provenance_problems(&set, &scenes[..1]),
        [format!("{}: no verified adapter", set.join("provenance.json").display())]
    );
}

#[test]
fn every_golden_png_stays_under_the_size_cap() {
    let found = oversized(&golden_dir(), MAX_GOLDEN_BYTES);
    assert!(found.is_empty(), "{}", found.join("\n"));
}

#[test]
fn corpus_scenes_are_distinct_and_cover_every_area() {
    let mut seen: Vec<(&str, u64)> = SCENES.iter().map(|s| (s.stem, s.time.to_bits())).collect();
    seen.sort_unstable();
    let before = seen.len();
    seen.dedup();
    assert_eq!(seen.len(), before, "a corpus scene is listed twice at the same time");
    for area in AREAS {
        assert!(SCENES.iter().any(|s| s.covers.contains(&area)), "no corpus scene covers {area:?}");
    }
    for s in SCENES {
        assert!(scene_path(s.stem).is_file(), "{} is not in the corpus", s.stem);
        assert!(s.time.is_finite() && s.time >= 0.0, "{}: time {}", s.stem, s.time);
        // a declaration names a node and the start of its reason, so it cannot excuse unrelated reports
        assert!(s.gaps.iter().chain(s.errors).all(|g| g.contains(": ")), "{}: {:?} {:?}", s.name, s.gaps, s.errors);
    }
}

/// Reports of `kind` that no declaration in `known` starts, and declarations that no report matches (a gap that has
/// been closed must leave the list, so the scene is again held to rendering everything).
fn known_problems(kind: &str, reports: &[String], known: &[&str]) -> Vec<String> {
    let mut problems: Vec<String> = reports
        .iter()
        .filter(|m| !known.iter().any(|g| m.starts_with(g)))
        .map(|m| format!("undeclared {kind}: {m}"))
        .collect();
    problems.extend(
        known
            .iter()
            .filter(|g| !reports.iter().any(|m| m.starts_with(*g)))
            .map(|g| format!("declared {kind} not reported: {g}")),
    );
    problems
}

#[test]
fn undeclared_reports_and_stale_declarations_are_reported() {
    let reports = |r: &[&str]| r.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let gaps = ["atlas: the raster basemap", "pin1: constraint body"];
    let k = "unsupported content";
    assert!(known_problems(k, &reports(&[]), &[]).is_empty());
    assert!(known_problems(
        k,
        &reports(&["atlas: the raster basemap is drawn beneath", "pin1: constraint body x"]),
        &gaps
    )
    .is_empty());
    // a new report, though its node already has a declared gap with another reason
    let p = known_problems(
        k,
        &reports(&["atlas: the raster basemap is drawn", "pin1: constraint body x", "atlas: no tiles"]),
        &gaps,
    );
    assert_eq!(p, ["undeclared unsupported content: atlas: no tiles"]);
    // a closed gap
    let p = known_problems(k, &reports(&["pin1: constraint body x"]), &gaps);
    assert_eq!(p, ["declared unsupported content not reported: atlas: the raster basemap"]);
    // nothing declared: every report fails, and the kind names what was reported
    let p = known_problems("render error", &reports(&["bg: ffprobe failed on clip.mp4"]), &[]);
    assert_eq!(p, ["undeclared render error: bg: ffprobe failed on clip.mp4"]);
    // a declaration matches the start of a message, never its middle
    let p = known_problems("render error", &reports(&["bg: transcript line 1: x"]), &["transcript line 1: "]);
    assert_eq!(p.len(), 2, "{p:?}");
}

fn scratch_set(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sr-corpus-golden-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A 64×64 frame: a horizontal ramp of `colour`'s channels over black.
fn ramp(colour: [u8; 3]) -> image::RgbaImage {
    image::RgbaImage::from_fn(64, 64, |x, _| {
        let k = |c: u8| (c as u32 * x / 63) as u8;
        image::Rgba([k(colour[0]), k(colour[1]), k(colour[2]), 255])
    })
}

#[test]
fn identical_flat_oversized_missing_and_stray_goldens_are_all_reported() {
    let set = scratch_set("checks");
    ramp([255, 0, 0]).save(set.join("red.png")).unwrap();
    ramp([255, 0, 0]).save(set.join("copy.png")).unwrap();
    ramp([0, 0, 255]).save(set.join("blue.png")).unwrap();
    image::RgbaImage::from_pixel(64, 64, image::Rgba([0, 0, 0, 0])).save(set.join("blank.png")).unwrap();
    image::RgbaImage::from_fn(65, 8, |x, _| image::Rgba([x as u8 * 3, 0, 0, 255])).save(set.join("wide.png")).unwrap();
    ramp([0, 255, 0]).save(set.join("stray.png")).unwrap();
    let problems = set_problems(&set, &["red", "copy", "blue", "blank", "wide", "absent"]);
    let has = |needle: &str| problems.iter().any(|p| p.contains(needle));
    assert!(has("red and copy differ by 0.00"), "{problems:#?}");
    assert!(has("blank.png: spread 0"), "{problems:#?}");
    assert!(has("wide.png: [65, 8] exceeds 64 px"), "{problems:#?}");
    assert!(has("absent.png: missing"), "{problems:#?}");
    assert!(has("stray.png belongs to no corpus scene"), "{problems:#?}");
    // the distinct ramps, and the blank frame against them, pass
    assert!(!has("red and blue") && !has("blue and blank") && !has("red and blank"), "{problems:#?}");
    assert_eq!(problems.len(), 5, "{problems:#?}");
}

/// `img` with `d` added to every colour channel.
fn lifted(img: &image::RgbaImage, d: u8) -> image::RgbaImage {
    let mut out = img.clone();
    out.pixels_mut().for_each(|p| (0..3).for_each(|c| p.0[c] += d));
    out
}

#[test]
fn a_near_identical_pair_is_reported_and_the_smallest_safe_margin_passes() {
    // no channel of these ramps reaches 255 - 3, so every lift is exact
    let base = ramp([200, 120, 40]);
    let set = scratch_set("near");
    base.save(set.join("base.png")).unwrap();
    lifted(&base, 1).save(set.join("nudged.png")).unwrap();
    let problems = set_problems(&set, &["base", "nudged"]);
    assert_eq!(problems, [format!("{}: base and nudged differ by 1.00 ≤ 2", set.display())]);
    // one code value more than the margin is a distinct frame
    let set = scratch_set("margin");
    base.save(set.join("base.png")).unwrap();
    lifted(&base, 3).save(set.join("lifted.png")).unwrap();
    assert_eq!(set_problems(&set, &["base", "lifted"]), Vec::<String>::new());
}

#[test]
fn a_full_resolution_frame_breaks_the_size_cap() {
    let dir = scratch_set("size");
    std::fs::create_dir_all(dir.join("nested")).unwrap();
    ramp([255, 255, 255]).save(dir.join("thumbnail.png")).unwrap();
    // noise does not compress: a 480×270 frame of it is far over the cap
    let mut state = 0x2545_f491u32;
    let noise = image::RgbaImage::from_fn(480, 270, |_, _| {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        image::Rgba(state.to_le_bytes())
    });
    noise.save(dir.join("nested/full.png")).unwrap();
    let found = oversized(&dir, MAX_GOLDEN_BYTES);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("full.png"), "{found:?}");
}
