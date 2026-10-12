//! `sr-fuzz [--seconds N] [--seed S] [--corpus DIR] [--out DIR] [--target NAME|all]`: fuzzes the
//! document pipeline and the expression compiler for N seconds (default 60)
//! and writes every crashing input to the output directory. `--target` fuzzes
//! a byte-level target instead (`all`: each in turn, sharing the time).
//! `--iterations N --partition I/K` executes a deterministic share of N inputs;
//! `--report FILE` writes its counters as JSON for CI coverage verification.

use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let get = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let present = |k: &str| args.iter().any(|a| a == k);
    if present("--help") || present("-h") {
        println!("Fuzz document parsing, evaluation, expressions and asset parsers.\n\nExamples:\n  sr-fuzz --seconds 600\n  sr-fuzz --iterations 2400000 --partition 0/20 --report campaign.json\n\nOptions:\n  --seconds N       Time budget (default: 60); conflicts with --iterations\n  --iterations N    Positive number of serial campaign inputs\n  --partition I/K   Zero-based partition; requires --iterations and 0 <= I < K\n  --report FILE     Write JSON counters; requires --iterations\n  --seed S          Integer RNG seed (default: 24301)\n  --corpus DIR      Seed corpus directory\n  --out DIR         Crash output directory (default: fuzz-crashes)\n  --target NAME     Byte-level target, or all; conflicts with --iterations\n\nExit codes: 0 success, 1 crash or I/O failure, 2 invalid arguments.");
        return;
    }
    let iterations = if present("--iterations") {
        Some(
            get("--iterations")
                .and_then(|s| s.parse::<u64>().ok())
                .filter(|n| *n > 0)
                .unwrap_or_else(|| usage_error("--iterations requires a positive integer")),
        )
    } else {
        None
    };
    if iterations.is_some() && (present("--seconds") || present("--target")) {
        usage_error("use --iterations on its own; it cannot be combined with --seconds or --target");
    }
    if iterations.is_none() && (present("--partition") || present("--report")) {
        usage_error("--partition and --report require --iterations N");
    }
    let (index, count) = if present("--partition") {
        get("--partition")
            .and_then(|s| {
                let (index, count) = s.split_once('/')?;
                Some((index.parse::<u64>().ok()?, count.parse::<u64>().ok()?))
            })
            .filter(|(index, count)| *count > 0 && index < count)
            .unwrap_or_else(|| usage_error("--partition must be I/K with K > 0 and 0 <= I < K (for example 0/20)"))
    } else {
        (0, 1)
    };
    let report = get("--report").map(std::path::PathBuf::from);
    if present("--report") && report.is_none() {
        usage_error("--report requires an output file path");
    }
    let seconds: u64 = get("--seconds").and_then(|s| s.parse().ok()).unwrap_or(60);
    let seed: u64 = get("--seed").and_then(|s| s.parse().ok()).unwrap_or(0x5eed);
    let corpus = get("--corpus").unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/corpus").into());
    let out = std::path::PathBuf::from(get("--out").unwrap_or_else(|| "fuzz-crashes".into()));
    // panics are caught; keep their messages off the terminal, and each crash says where it panicked
    sr_fuzz::quiet_panics();
    if let Some(name) = get("--target") {
        // each place that panics, once
        static SEEN: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
        std::panic::set_hook(Box::new(|info| {
            let at = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
            let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
            if !seen.contains(&at) {
                eprintln!("panic at {at}");
                seen.push(at);
            }
        }));
        std::process::exit(targets(&name, seconds, seed, &out));
    }
    let seeds = sr_fuzz::corpus(std::path::Path::new(&corpus));
    let start = Instant::now();
    let limit = Duration::from_secs(seconds);
    let mut last = Instant::now();
    let c = if let Some(total) = iterations {
        eprintln!("Running partition {index}/{count} of {total} deterministic inputs");
        sr_fuzz::fuzz_partition(seed, &seeds, total, index, count)
    } else {
        sr_fuzz::fuzz(seed, &seeds, |c| {
            if last.elapsed() > Duration::from_secs(60) {
                eprintln!(
                    "{:>6.0} s: {} documents, {} expressions, {} crashes",
                    start.elapsed().as_secs_f64(),
                    c.docs,
                    c.exprs,
                    c.crashes.len()
                );
                last = Instant::now();
            }
            start.elapsed() >= limit
        })
    };
    if let Some(total) = iterations {
        println!("{} inputs in partition {index}/{count}", c.docs + c.exprs);
        if let Some(path) = report {
            let report = serde_json::json!({"seed": seed, "seeds": seeds.len(), "iterations": total,
                "index": index, "count": count, "docs": c.docs, "exprs": c.exprs, "crashes": c.crashes.len()});
            let write = || -> std::io::Result<()> {
                if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&path, report.to_string())
            };
            if let Err(error) = write() {
                eprintln!("error: cannot write campaign report {}: {error}", path.display());
                std::process::exit(1);
            }
        }
    }
    let secs = start.elapsed().as_secs_f64();
    println!(
        "{} seeds; {secs:.0} s: {} documents ({:.0}/s), {} expressions ({:.0}/s), {} crashes",
        seeds.len(),
        c.docs,
        c.docs as f64 / secs,
        c.exprs,
        c.exprs as f64 / secs,
        c.crashes.len()
    );
    if !c.crashes.is_empty() {
        std::fs::create_dir_all(&out).ok();
        for (k, crash) in c.crashes.iter().enumerate() {
            let p = out.join(format!("{k:04}-{}.txt", crash.target));
            std::fs::write(&p, &crash.input).ok();
            println!("{}: {} ({})", p.display(), crash.message, crash.target);
        }
        std::process::exit(1);
    }
}

fn usage_error(message: &str) -> ! {
    eprintln!("error: {message}\nRun sr-fuzz --help for examples.");
    std::process::exit(2);
}

/// Fuzzes the byte-level target `name` (or all of them); the exit code.
fn targets(name: &str, seconds: u64, seed: u64, out: &std::path::Path) -> i32 {
    use sr_fuzz::targets::{fuzz, target, TARGETS};
    let list: Vec<_> = match name {
        "all" => TARGETS.iter().collect(),
        n => match target(n) {
            Some(t) => vec![t],
            None => {
                let names: Vec<&str> = TARGETS.iter().map(|t| t.name).collect();
                eprintln!("unknown target {n}; targets: all, {}", names.join(", "));
                return 2;
            }
        },
    };
    let each = Duration::from_secs_f64(seconds as f64 / list.len() as f64);
    let mut found = 0;
    for t in list {
        let start = Instant::now();
        let c = fuzz(seed, t, |_| start.elapsed() >= each);
        let secs = start.elapsed().as_secs_f64();
        println!("{}: {} inputs ({:.0}/s), {} crashes", t.name, c.inputs, c.inputs as f64 / secs, c.crashes.len());
        if !c.crashes.is_empty() {
            std::fs::create_dir_all(out).ok();
        }
        for crash in &c.crashes {
            let p = out.join(format!("{found:04}-{}.bin", crash.target));
            std::fs::write(&p, &crash.input).ok();
            println!("{}: {}", p.display(), crash.message);
            found += 1;
        }
    }
    (found > 0) as i32
}
