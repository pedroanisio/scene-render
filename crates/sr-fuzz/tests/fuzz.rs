//! A short deterministic fuzzing run on every test pass (the long run is `sr-fuzz --seconds N`).

use std::sync::Mutex;

use sr_fuzz::targets;

/// Runs `f` with panic messages off the terminal; one campaign at a time, since the hook is global.
fn quiet<T>(f: impl FnOnce() -> T) -> T {
    static LOCK: Mutex<()> = Mutex::new(());
    let _held = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    sr_fuzz::quiet_panics();
    let out = f();
    let _ = std::panic::take_hook();
    out
}

/// A crash says where it panicked, not only what the panic said: an index out of bounds found by
/// a ten-minute run is otherwise a message with no place to start from.
#[test]
fn a_crash_names_the_place_that_panicked() {
    fn boom(input: &str) {
        panic!("boom on {input}");
    }
    let k = quiet(|| sr_fuzz::guard("document", "x", boom)).expect("the panic is a crash");
    assert!(k.message.starts_with("boom on x at "), "{}", k.message);
    assert!(k.message.contains("fuzz.rs:"), "{}", k.message);
    // a crash after it does not carry the place of the one before
    let quiet_input = quiet(|| sr_fuzz::guard("document", "y", |_| {}));
    assert!(quiet_input.is_none());
}

/// A panic on a worker thread (rayon re-raises it on the caller) still says where it happened: the
/// crash of CI run 37985001294 (`min > max, or either was NaN`) came with no place at all.
#[test]
fn a_crash_on_a_worker_thread_names_the_place_that_panicked() {
    fn boom(input: &str) {
        let input = input.to_string();
        let worker = std::thread::spawn(move || panic!("boom on {input}"));
        if let Err(payload) = worker.join() {
            std::panic::resume_unwind(payload);
        }
    }
    let k = quiet(|| sr_fuzz::guard("document", "x", boom)).expect("the panic is a crash");
    assert!(k.message.starts_with("boom on x at "), "{}", k.message);
    assert!(k.message.contains("fuzz.rs:"), "{}", k.message);
}

/// The crash found by the ten-minute run of CI run 37985001294 on main baf0db6: a 3D rigid body at
/// `y="INF"` coupled to an ocean panicked in `f64::clamp` (`min = 0.0, max = NaN`).
#[test]
fn an_infinite_body_on_an_ocean_is_no_crash() {
    let input = include_str!("crashes/ocean-body-at-infinity.scene.xml");
    let k = quiet(|| sr_fuzz::guard("document", input, sr_fuzz::run_doc));
    assert!(k.is_none(), "{}", k.map(|k| k.message).unwrap_or_default());
}

#[test]
fn a_flock_with_huge_bounds_is_no_crash() {
    let input = include_str!("crashes/flock-huge-bounds.scene.xml");
    let k = quiet(|| sr_fuzz::guard("document", input, sr_fuzz::run_doc));
    assert!(k.is_none(), "{}", k.map(|k| k.message).unwrap_or_default());
}

#[test]
fn no_panics_in_a_short_campaign() {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/corpus"));
    let seeds = sr_fuzz::corpus(root);
    assert!(seeds.len() > 100, "corpus found: {}", seeds.len());
    let c = quiet(|| sr_fuzz::fuzz(42, &seeds, |c| c.docs + c.exprs >= 3000));
    let report: Vec<String> = c
        .crashes
        .iter()
        .take(5)
        .map(|k| format!("{}: {}\n{}", k.target, k.message, &k.input[..k.input.len().min(400)]))
        .collect();
    assert!(c.crashes.is_empty(), "{} crashes:\n{}", c.crashes.len(), report.join("\n---\n"));
}

/// `inputs` inputs of the byte-level target `name` from a fixed seed.
fn smoke(name: &str, inputs: u64) {
    let t = targets::target(name).expect(name);
    let c = quiet(|| targets::fuzz(42, t, |c| c.inputs >= inputs));
    assert_eq!(c.inputs, inputs, "{name} has no seeds");
    let report: Vec<String> = c
        .crashes
        .iter()
        .take(5)
        .map(|k| format!("{}\n{}", k.message, k.input[..k.input.len().min(400)].escape_ascii()))
        .collect();
    assert!(c.crashes.is_empty(), "{name}: {} crashes:\n{}", c.crashes.len(), report.join("\n---\n"));
}

#[test]
fn every_target_has_a_smoke_test() {
    let here = include_str!("fuzz.rs");
    for t in targets::TARGETS {
        assert!(here.contains(&format!("smoke(\"{}\",", t.name)), "{}", t.name);
    }
}

#[test]
fn captions_smoke() {
    smoke("captions", 600);
}

#[test]
fn lottie_smoke() {
    smoke("lottie", 300);
}

#[test]
fn dotlottie_smoke() {
    smoke("dotlottie", 300);
}

#[test]
fn svg_smoke() {
    smoke("svg", 300);
}

#[test]
fn path_smoke() {
    smoke("path", 1000);
}

#[test]
fn formula_smoke() {
    smoke("formula", 600);
}

#[test]
fn text_smoke() {
    smoke("text", 200);
}

#[test]
fn pmtiles_smoke() {
    smoke("pmtiles", 300);
}

#[test]
fn mvt_smoke() {
    smoke("mvt", 1000);
}

#[test]
fn geodata_smoke() {
    smoke("geodata", 600);
}

#[test]
fn ply_smoke() {
    smoke("ply", 600);
}

#[test]
fn usd_smoke() {
    smoke("usd", 300);
}

#[test]
fn gltf_smoke() {
    smoke("gltf", 300);
}

#[test]
fn document_assets_smoke() {
    smoke("document-assets", 300);
}

#[test]
fn campaign_partitions_preserve_the_serial_work_and_crashes() {
    let seeds =
        vec![r#"<scene version="1.0"><project width="4" height="4" fps="1" duration="1"/><composition/></scene>"#
            .to_string()];
    // Under `quiet`, like every other campaign here: `guard` clears the process-wide place of a
    // worker-thread panic, so a campaign running beside another test would erase that test's place.
    let (serial, parts) = quiet(|| {
        let serial = sr_fuzz::fuzz(0x5eed, &seeds, |c| c.docs + c.exprs >= 120);
        let parts: Vec<_> = (0..4).map(|index| sr_fuzz::fuzz_partition(0x5eed, &seeds, 120, index, 4)).collect();
        (serial, parts)
    });
    assert_eq!(parts.iter().map(|c| c.docs).sum::<u64>(), serial.docs);
    assert_eq!(parts.iter().map(|c| c.exprs).sum::<u64>(), serial.exprs);
    assert_eq!(parts.iter().map(|c| c.crashes.len()).sum::<usize>(), serial.crashes.len());
}

#[test]
fn fixed_campaign_cli_reports_work_and_rejects_invalid_partitions() {
    let binary = env!("CARGO_BIN_EXE_sr-fuzz");
    let help = std::process::Command::new(binary).args(["--seconds", "0", "--help"]).output().unwrap();
    assert!(help.status.success());
    let help = String::from_utf8_lossy(&help.stdout);
    assert!(help.contains("--iterations") && help.contains("--partition") && help.contains("--report"));
    let report = std::env::temp_dir().join(format!("sr-fuzz-report-{}.json", std::process::id()));
    let output = std::process::Command::new(binary)
        .args(["--iterations", "11", "--partition", "1/3", "--report"])
        .arg(&report)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("4 inputs"));
    let result: serde_json::Value = serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
    std::fs::remove_file(report).unwrap();
    assert_eq!(result["seed"], 0x5eed);
    assert_eq!(result["iterations"], 11);
    assert_eq!(result["index"], 1);
    assert_eq!(result["count"], 3);
    assert_eq!(result["docs"].as_u64().unwrap() + result["exprs"].as_u64().unwrap(), 4);
    assert_eq!(result["crashes"], 0);
    for args in [
        vec!["--partition", "1/3"],
        vec!["--iterations", "11", "--partition", "3/3"],
        vec!["--iterations", "11", "--partition", "0/0"],
        vec!["--iterations", "0"],
        vec!["--iterations", "11", "--seconds", "1"],
        vec!["--iterations", "11", "--target", "all"],
    ] {
        let output = std::process::Command::new(binary).args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2));
    }
}
