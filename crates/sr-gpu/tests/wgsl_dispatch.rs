//! The numbers a shader and the Rust that drives it must agree on, checked from the sources with no GPU: the
//! `@workgroup_size` of every compute entry point against the `div_ceil(N)` that sizes its dispatch, and the tile and
//! step constants written once in WGSL and once in Rust.
//!
//! A divisor that no longer matches dispatches too few groups, or too many threads, and the picture is wrong without
//! an error; only a GPU golden image would notice. Every dispatch site and every compute entry point must be listed
//! here, so a new one is a failing test until someone states what it must equal.

use std::collections::BTreeMap;
use std::path::PathBuf;

use regex::Regex;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let path = crate_dir().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The `@workgroup_size` of entry point `entry` in `text`, padded to three axes.
fn workgroup(text: &str, entry: &str) -> Result<[u32; 3], String> {
    let re = Regex::new(&format!(r"@workgroup_size\(([^)]*)\)\s*fn\s+{}\b", regex::escape(entry))).unwrap();
    let m = re.captures(text).ok_or_else(|| format!("no compute entry `{entry}`"))?;
    let mut dims = [1u32; 3];
    for (i, d) in m[1].split(',').enumerate() {
        dims[i] = d.trim().parse().map_err(|_| format!("`{entry}`: workgroup size `{}` is not a literal", &m[1]))?;
    }
    Ok(dims)
}

/// The lines of `text` that contain `anchor`.
fn lines_with<'a>(text: &'a str, anchor: &str) -> Vec<&'a str> {
    text.lines().filter(|l| l.contains(anchor)).collect()
}

/// The literal divisors `div_ceil(N)` of the line, in order.
fn divisors(line: &str) -> Vec<u32> {
    let re = Regex::new(r"div_ceil\((\d+)\)").unwrap();
    re.captures_iter(line).map(|c| c[1].parse().unwrap()).collect()
}

/// What a dispatch site must equal: `rust` is a source file, `anchor` a substring of the line(s) that size the
/// dispatch, `lines` how many such lines there are, `entries` the compute entry points (file, name) it launches.
struct Site {
    rust: &'static str,
    anchor: &'static str,
    lines: usize,
    entries: &'static [(&'static str, &'static str)],
}

const SITES: &[Site] = &[
    Site {
        rust: "src/compute.rs",
        anchor: "(job.invocations as u64).div_ceil(64)",
        lines: 1,
        entries: &[("src/compute.rs", "sr_main")],
    },
    Site {
        rust: "src/pathtrace.rs",
        anchor: "plan.rows_per_slot.div_ceil(64)",
        lines: 1,
        entries: &[("src/volume_grid.wgsl", "cs_light_grid")],
    },
    Site {
        rust: "src/pathtrace.rs",
        anchor: "plan.node_count().div_ceil(64)",
        lines: 1,
        entries: &[("src/volume_grid.wgsl", "cs_dome_grid")],
    },
    Site {
        rust: "src/pathtrace.rs",
        anchor: "tile.size[0].div_ceil(8), tile.size[1].div_ceil(8)",
        lines: 1,
        entries: &[
            ("src/pathtrace.wgsl", "cs_trace"),
            ("src/pathtrace.wgsl", "cs_count"),
            ("src/pathtrace.wgsl", "cs_atrous"),
        ],
    },
    Site {
        rust: "src/output.rs",
        anchor: "invocations.div_ceil(64)",
        lines: 1,
        entries: &[("src/output.wgsl", "cs_pack"), ("src/output.wgsl", "cs_pack_nv12_blocks")],
    },
    Site {
        rust: "src/output.rs",
        anchor: "dispatch_workgroups(small[0].div_ceil(8)",
        lines: 1,
        entries: &[("src/output_blur.wgsl", "cs_blur")],
    },
    Site {
        rust: "src/output.rs",
        anchor: "dispatch_workgroups(into.size[0].div_ceil(8)",
        lines: 1,
        entries: &[("src/output.wgsl", "cs_place")],
    },
    Site {
        rust: "src/video.rs",
        anchor: "dispatch_workgroups(size[0].div_ceil(8), size[1].div_ceil(8), 1)",
        lines: 2,
        entries: &[("src/flow.wgsl", "cs_reduce"), ("src/flow.wgsl", "cs_densify")],
    },
    Site {
        rust: "src/video.rs",
        anchor: "dispatch_workgroups(gs[0].div_ceil(8)",
        lines: 1,
        entries: &[("src/flow.wgsl", "cs_patch")],
    },
    Site {
        rust: "src/three.rs",
        anchor: "let blocks = n.div_ceil(256)",
        lines: 1,
        entries: &[
            ("src/three_sort.wgsl", "cs_keys"),
            ("src/three_sort.wgsl", "cs_hist"),
            ("src/three_sort.wgsl", "cs_scan"),
            ("src/three_sort.wgsl", "cs_scatter"),
        ],
    },
];

/// How many `.dispatch_workgroups(` calls each source file makes, so that one added or removed fails until the table
/// above, or the reason it needs none, is brought up to date:
/// - `compute.rs`, `output.rs`, `video.rs`, `pathtrace.rs`: every call is sized by a `SITES` row (pathtrace's trace,
///   sample-count and a-trous passes share one `groups`; its light and dome calls have their own rows);
/// - `raster.rs`: one group per 16×16 tile, `job.tiles` counts tiles (`raster_group_is_one_tile`), or one per listed
///   active tile;
/// - `three.rs`: the sort passes are sized by `blocks` (a `SITES` row), the scan and the CPU scatter are one group, and
///   the aperture kernel is (1, 4, 1) groups of 32 (`aperture_groups_fill_the_array`).
const DISPATCH_CALLS: &[(&str, usize)] = &[
    ("src/compute.rs", 1),
    ("src/output.rs", 3),
    ("src/pathtrace.rs", 5),
    ("src/raster.rs", 2),
    ("src/three.rs", 5),
    ("src/video.rs", 3),
];

/// Every compute entry point of the crate, with the sites above that check it or the reason none does.
const ENTRIES: &[(&str, &str)] = &[
    ("src/compute.rs", "sr_main"),
    ("src/flow.wgsl", "cs_reduce"),
    ("src/flow.wgsl", "cs_patch"),
    ("src/flow.wgsl", "cs_densify"),
    ("src/output.wgsl", "cs_pack"),
    ("src/output.wgsl", "cs_pack_nv12_blocks"),
    ("src/output.wgsl", "cs_place"),
    ("src/output_blur.wgsl", "cs_blur"),
    ("src/pathtrace.wgsl", "cs_trace"),
    ("src/pathtrace.wgsl", "cs_count"),
    ("src/pathtrace.wgsl", "cs_atrous"),
    ("src/raster.wgsl", "raster_main"),
    ("src/three_dof_kernel.wgsl", "cs_aperture"),
    ("src/three_sort.wgsl", "cs_keys"),
    ("src/three_sort.wgsl", "cs_hist"),
    ("src/three_sort.wgsl", "cs_scan"),
    ("src/three_sort.wgsl", "cs_scatter"),
    ("src/three_sort.wgsl", "cs_scatter_cpu"),
    ("src/volume_grid.wgsl", "cs_light_grid"),
    ("src/volume_grid.wgsl", "cs_dome_grid"),
];

/// Compare the divisors on `lines` with the workgroup dimensions of `entries`, as text: the problems found.
fn site_problems(rust_text: &str, anchor: &str, expected_lines: usize, entries: &[(&str, [u32; 3])]) -> Vec<String> {
    let lines = lines_with(rust_text, anchor);
    let mut out = Vec::new();
    if lines.len() != expected_lines {
        out.push(format!("`{anchor}`: {} lines, expected {expected_lines}", lines.len()));
    }
    for line in lines {
        let found = divisors(line);
        if found.is_empty() {
            out.push(format!("`{anchor}`: no div_ceil literal on `{}`", line.trim()));
        }
        for (name, dims) in entries {
            // a 2-D dispatch divides x and y by the same literal; a 1-D one divides x
            let wanted: Vec<u32> = if dims[1] > 1 { vec![dims[0], dims[1]] } else { vec![dims[0]] };
            if found.len() < wanted.len() || found[..wanted.len()] != wanted[..] {
                out.push(format!("`{anchor}`: divides by {found:?}, `{name}` is {wanted:?}"));
            }
        }
    }
    out
}

#[test]
fn every_dispatch_divisor_is_the_workgroup_size_of_its_shader() {
    let mut problems = Vec::new();
    for s in SITES {
        let rust = read(s.rust);
        let mut dims = Vec::new();
        for (file, entry) in s.entries {
            match workgroup(&read(file), entry) {
                Ok(d) => dims.push((*entry, d)),
                Err(e) => problems.push(format!("{file}: {e}")),
            }
        }
        problems.extend(site_problems(&rust, s.anchor, s.lines, &dims).into_iter().map(|p| format!("{}: {p}", s.rust)));
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_listed_compute_entry_points_are_all_of_them() {
    let mut found: Vec<(String, String)> = Vec::new();
    let re = Regex::new(r"@compute\s+@workgroup_size\([^)]*\)\s*fn\s+(\w+)").unwrap();
    let mut files: Vec<_> = std::fs::read_dir(crate_dir().join("src"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("wgsl" | "rs")))
        .collect();
    files.sort();
    for p in files {
        let text = std::fs::read_to_string(&p).unwrap();
        let rel = format!("src/{}", p.file_name().unwrap().to_string_lossy());
        for c in re.captures_iter(&text) {
            found.push((rel.clone(), c[1].to_string()));
        }
    }
    let mut listed: Vec<(String, String)> = ENTRIES.iter().map(|(f, e)| (f.to_string(), e.to_string())).collect();
    listed.sort();
    found.sort();
    assert_eq!(found, listed, "a compute entry point was added or removed: list it in ENTRIES");
}

#[test]
fn every_dispatch_call_is_accounted_for() {
    let mut calls: BTreeMap<String, usize> = BTreeMap::new();
    for e in std::fs::read_dir(crate_dir().join("src")).unwrap() {
        let p = e.unwrap().path();
        if p.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let n = std::fs::read_to_string(&p).unwrap().matches(".dispatch_workgroups(").count();
        if n > 0 {
            calls.insert(format!("src/{}", p.file_name().unwrap().to_string_lossy()), n);
        }
    }
    let expected: BTreeMap<String, usize> = DISPATCH_CALLS.iter().map(|(f, n)| (f.to_string(), *n)).collect();
    assert_eq!(calls, expected, "a dispatch call was added or removed: say what its size must equal");
}

#[test]
fn raster_group_is_one_tile() {
    let raster = workgroup(&read("src/raster.wgsl"), "raster_main").unwrap();
    let tile =
        Regex::new(r"pub const TILE: u32 = (\d+);").unwrap().captures(&read("../sr-vector/src/tile.rs")).unwrap()[1]
            .parse::<u32>()
            .unwrap();
    assert_eq!(raster, [tile, tile, 1], "raster.wgsl's workgroup is one sr-vector tile");
    let wgsl = read("src/raster.wgsl");
    assert!(wgsl.contains(&format!("* {tile}u")), "raster.wgsl indexes pixels as `tile * {tile}u`");
}

#[test]
fn aperture_groups_fill_the_array() {
    let wgsl = read("src/three_dof_kernel.wgsl");
    let size = workgroup(&wgsl, "cs_aperture").unwrap()[0];
    let len: u32 = Regex::new(r"array<vec4<f32>, (\d+)>").unwrap().captures(&wgsl).unwrap()[1].parse().unwrap();
    let rows: u32 = Regex::new(r"dispatch_workgroups\(1, (\d+), 1\)").unwrap().captures(&read("src/three.rs")).unwrap()
        [1]
    .parse()
    .unwrap();
    assert_eq!(size * rows, len, "the aperture kernel writes {size} per row, {rows} rows, into {len} entries");
    assert!(wgsl.contains(&format!("id.y * {size}u + k")), "the row stride is the workgroup size");
}

/// A numeric constant of `text` named `name`: `const NAME: ty = 16u;`, `pub const NAME: u32 = 16;` or `= 0.02;`.
fn constant(text: &str, name: &str) -> Option<String> {
    let re = Regex::new(&format!(r"\bconst {}\s*:\s*[\w<>]+\s*=\s*([0-9][0-9_.]*)[a-z0-9]*\s*;", regex::escape(name)))
        .unwrap();
    re.captures(text).map(|c| c[1].replace('_', ""))
}

#[test]
fn shared_constants_have_one_value() {
    // (WGSL file, name, Rust file, name)
    let pairs = [
        ("src/three_types.wgsl", "TILE", "src/three.rs", "TILE"),
        ("src/three_types.wgsl", "TILE", "../sr-vector/src/tile.rs", "TILE"),
        ("src/three_types.wgsl", "MAX_PER_TILE", "src/three.rs", "MAX_PER_TILE"),
        ("src/geodesic.wgsl", "STEP", "../sr-sim/src/gr.rs", "STEP"),
        ("src/geodesic.wgsl", "MAX_STEPS", "../sr-sim/src/gr.rs", "MAX_STEPS"),
    ];
    let mut problems = Vec::new();
    for (wf, wn, rf, rn) in pairs {
        let (w, r) = (constant(&read(wf), wn), constant(&read(rf), rn));
        match (w, r) {
            (Some(w), Some(r)) if w == r => {}
            (w, r) => problems.push(format!("{wf} {wn} = {w:?}, {rf} {rn} = {r:?}")),
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_dof_tile_texture_is_one_wgsl_tile_per_texel() {
    let wgsl = constant(&read("src/three_post.wgsl"), "DOF_TILE").expect("DOF_TILE in three_post.wgsl");
    let rust = read("src/three.rs");
    let line = lines_with(&rust, "let tsize = [size[0].div_ceil(")[0];
    assert_eq!(divisors(line), vec![wgsl.parse::<u32>().unwrap(); 2], "{line}");
}

#[test]
fn the_checker_finds_a_divisor_that_no_longer_matches() {
    let wgsl = "@compute @workgroup_size(8, 8)\nfn cs_a(@builtin(global_invocation_id) g: vec3<u32>) {}\n\
                @compute @workgroup_size(64)\nfn cs_b() {}\n";
    let a = workgroup(wgsl, "cs_a").unwrap();
    let b = workgroup(wgsl, "cs_b").unwrap();
    assert_eq!((a, b), ([8, 8, 1], [64, 1, 1]));
    assert!(workgroup(wgsl, "cs_missing").is_err());
    let ok = "p.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);";
    assert!(site_problems(ok, "dispatch_workgroups(w", 1, &[("cs_a", a)]).is_empty());
    // the shader moved to 16×16 and the call did not
    let moved = [("cs_a", [16, 16, 1])];
    assert_eq!(site_problems(ok, "dispatch_workgroups(w", 1, &moved).len(), 1);
    // the call moved and the shader did not
    let bad = "p.dispatch_workgroups(w.div_ceil(16), h.div_ceil(16), 1);";
    assert_eq!(site_problems(bad, "dispatch_workgroups(w", 1, &[("cs_a", a)]).len(), 1);
    // a second line that carries the anchor, or none
    assert_eq!(site_problems(&format!("{ok}\n{ok}"), "dispatch_workgroups(w", 1, &[("cs_a", a)]).len(), 1);
    assert_eq!(site_problems("", "dispatch_workgroups(w", 1, &[("cs_a", a)]).len(), 1);
    // a 1-D kernel divides once
    assert!(site_problems("g.div_ceil(64)", "g.div", 1, &[("cs_b", b)]).is_empty());
    assert_eq!(site_problems("g.div_ceil(32)", "g.div", 1, &[("cs_b", b)]).len(), 1);
}

#[test]
fn constants_are_read_in_the_forms_the_sources_use() {
    assert_eq!(constant("const TILE: u32 = 16u;", "TILE").as_deref(), Some("16"));
    assert_eq!(constant("pub const TILE: u32 = 16;", "TILE").as_deref(), Some("16"));
    assert_eq!(constant("pub const STEP: f64 = 0.02;", "STEP").as_deref(), Some("0.02"));
    assert_eq!(constant("const STEP: f32 = 0.02;", "STEP").as_deref(), Some("0.02"));
    assert_eq!(constant("pub const MAX_STEPS: usize = 4096;", "MAX_STEPS").as_deref(), Some("4096"));
    assert_eq!(constant("const MAX_STEPS: u32 = 4096u;", "MAX_STEPS").as_deref(), Some("4096"));
    assert_eq!(constant("const TILE2: u32 = 16u;", "TILE"), None);
}
