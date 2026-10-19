//! The shipped example documents are valid documents of the schema this engine implements. A rule or schema change
//! that refuses one of them fails here, not when a user first runs it.

use std::path::{Path, PathBuf};

use sr_model::{validate_file, LoadOptions};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every `*.scene.xml` under `examples/`, sorted.
fn examples() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.to_string_lossy().ends_with(".scene.xml") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(&root().join("examples"), &mut out);
    out.sort();
    out
}

/// Examples whose input files are produced by a script and not kept in the repository (`examples/splat/.gitignore`):
/// they validate without the existence and digest check of their assets.
const GENERATED_INPUTS: &[&str] = &["examples/splat/turntable.scene.xml"];

fn relative(path: &Path) -> String {
    path.strip_prefix(root().join("examples")).unwrap().to_string_lossy().replace('\\', "/")
}

#[test]
fn every_example_document_has_no_validation_errors() {
    let found = examples();
    assert!(found.len() >= 8, "the examples were not found: {found:?}");
    let mut failures = Vec::new();
    for path in found {
        let generated = GENERATED_INPUTS.contains(&format!("examples/{}", relative(&path)).as_str());
        let opts = if generated { LoadOptions::without_assets() } else { LoadOptions::default() };
        // the program example runs its module at load time; its on-disk cache is not the subject here
        std::env::set_var("SR_PROGRAM_CACHE", "off");
        match validate_file(&path, &opts) {
            Err(e) => failures.push(format!("{}: {e}", relative(&path))),
            Ok(report) if report.has_errors() => failures.push(format!("{}:\n{report}", relative(&path))),
            Ok(_) => {}
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn the_generated_input_list_names_examples_that_exist() {
    for name in GENERATED_INPUTS {
        assert!(root().join(name).is_file(), "{name} is listed as having generated inputs and does not exist");
    }
}

/// `(element path, attribute)` pairs in which `hero-hires.scene.xml` is allowed to differ from `hero.scene.xml`: its
/// resolution, solver and memory and work budgets, and the follow window of the plume, which only the standard
/// scene sets.
const HIRES_DIFFERENCES: &[(&str, &str)] = &[
    ("scene/composition/ocean", "cellSize"),
    ("scene/composition/ocean", "checkpointMemoryMiB"),
    ("scene/composition/ocean", "maxMemoryMiB"),
    ("scene/composition/ocean", "maxWork"),
    ("scene/composition/ocean", "order"),
    ("scene/composition/ocean", "surfaceMemoryMiB"),
    ("scene/composition/ocean/whitewater", "maxWork"),
    ("scene/composition/particles3D", "maxMemoryMiB"),
    ("scene/composition/particles3D", "maxParticles"),
    ("scene/composition/particles3D/burst", "count"),
    ("scene/composition/object3D/pyro", "advection"),
    ("scene/composition/object3D/pyro", "checkpointMemoryMiB"),
    ("scene/composition/object3D/pyro", "follow"),
    ("scene/composition/object3D/pyro", "followLoss"),
    ("scene/composition/object3D/pyro", "followMargin"),
    ("scene/composition/object3D/pyro", "maxMemoryMiB"),
    ("scene/composition/object3D/pyro", "solver"),
    ("scene/composition/object3D/pyro", "voxelSize"),
    ("scene/composition/object3D/medium", "stepSize"),
];

fn differences(
    a: roxmltree::Node,
    b: roxmltree::Node,
    path: &str,
    out: &mut Vec<(String, String)>,
    shape: &mut Vec<String>,
) {
    let here = format!("{path}/{}", a.tag_name().name());
    if a.tag_name().name() != b.tag_name().name() {
        shape.push(format!("{path}: <{}> against <{}>", a.tag_name().name(), b.tag_name().name()));
        return;
    }
    let names: std::collections::BTreeSet<&str> = a.attributes().chain(b.attributes()).map(|x| x.name()).collect();
    for name in names {
        if a.attribute(name) != b.attribute(name) {
            out.push((here.trim_start_matches('/').to_string(), name.to_string()));
        }
    }
    let (ca, cb): (Vec<_>, Vec<_>) =
        (a.children().filter(|n| n.is_element()).collect(), b.children().filter(|n| n.is_element()).collect());
    if ca.len() != cb.len() {
        shape.push(format!("{here}: {} children against {}", ca.len(), cb.len()));
        return;
    }
    for (x, y) in ca.into_iter().zip(cb) {
        differences(x, y, &here, out, shape);
    }
}

#[test]
fn hero_hires_differs_from_hero_only_in_the_documented_attributes() {
    let dir = root().join("examples/cinematic-impact");
    let (xa, xb) = (
        std::fs::read_to_string(dir.join("hero.scene.xml")).unwrap(),
        std::fs::read_to_string(dir.join("hero-hires.scene.xml")).unwrap(),
    );
    let (da, db) = (roxmltree::Document::parse(&xa).unwrap(), roxmltree::Document::parse(&xb).unwrap());
    let (mut diff, mut shape) = (Vec::new(), Vec::new());
    differences(da.root_element(), db.root_element(), "", &mut diff, &mut shape);
    assert!(shape.is_empty(), "the two scenes differ in structure: {shape:?}");
    let allowed: Vec<(String, String)> =
        HIRES_DIFFERENCES.iter().map(|(p, a)| (p.to_string(), a.to_string())).collect();
    let extra: Vec<_> = diff.iter().filter(|d| !allowed.contains(d)).collect();
    assert!(extra.is_empty(), "attributes that differ and are not in HIRES_DIFFERENCES: {extra:?}");
    let unused: Vec<_> = allowed.iter().filter(|d| !diff.contains(d)).collect();
    assert!(unused.is_empty(), "HIRES_DIFFERENCES entries that no longer differ: {unused:?}");
}

/// `examples/glass-of-water` shows a drop falling into a glass of water with the engine's own 3D elements: the glass
/// and the water are transmissive materials, the ripples an ocean the drop's impulse drives, and the splash the ocean's
/// own spray. Nothing in it is a shader effect or a program, so an author can start from it without writing GLSL.
#[test]
fn the_glass_of_water_example_is_built_from_native_elements_only() {
    let path = root().join("examples/glass-of-water/glass-of-water.scene.xml");
    let xml = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let doc = roxmltree::Document::parse(&xml).unwrap();
    let all = |tag: &str| doc.descendants().filter(move |n| n.has_tag_name(tag)).collect::<Vec<_>>();
    let num = |n: &roxmltree::Node, a: &str| n.attribute(a).and_then(|v| v.parse::<f64>().ok());
    let material = |id: &str| {
        all("material")
            .into_iter()
            .find(|m| m.attribute("id") == Some(id))
            .unwrap_or_else(|| panic!("no material {id}"))
    };
    let glass = material("glass");
    assert!(num(&glass, "transmission").is_some_and(|t| t > 0.9), "the glass transmits");
    assert!(num(&glass, "ior").is_some_and(|i| (1.45..=1.55).contains(&i)), "soda-lime glass, n ≈ 1.5");
    let water = material("water");
    assert!(num(&water, "transmission").is_some_and(|t| t > 0.9), "the water transmits");
    assert_eq!(num(&water, "ior"), Some(1.333), "water, n = 1.333");
    let ocean = all("ocean");
    assert_eq!(ocean.len(), 1, "one water body");
    assert_eq!(ocean[0].attribute("material"), Some("water"));
    assert_eq!(ocean[0].attribute("boundary"), Some("closed"), "the glass walls reflect the ripples");
    let drop = ocean[0].children().find(|n| n.has_tag_name("waterImpulse")).and_then(|w| w.attribute("source"));
    let drop = drop.expect("the drop drives the water through a waterImpulse");
    let body = all("object3D").into_iter().find(|o| o.attribute("id") == Some(drop)).expect("the drop is an object3D");
    assert!(body.children().any(|n| n.has_tag_name("rigidBody")), "the drop falls under gravity");
    assert!(
        ocean[0].children().any(|n| n.has_tag_name("whitewater")),
        "the splash is the water's own spray, which the ripples raise where they are steep"
    );
    for tag in ["effect", "program", "compute", "shader"] {
        assert!(all(tag).is_empty(), "<{tag}> is not a native scene element for this example");
    }
    assert!(doc.descendants().any(|n| n.has_tag_name("camera") && n.attribute("renderer") == Some("pathtrace")));
}
