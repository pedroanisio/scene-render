//! What the evaluator reads from an included document's own definitions (an include merges its document's assets, effects and
//! materials are merged under the namespace `includeId/` and every reference to them is renamed): an expression's
//! `prop()` of a material and `geo()` of a map, and an ocean's material, are the included document's own even when the
//! including document declares the same id.

use std::path::{Path, PathBuf};

/// A fresh directory for one test's documents.
fn dir(test: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr-eval-include-scope-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn evaluator(path: &Path, text: &str) -> sr_eval::Evaluator {
    std::fs::write(path, text).unwrap();
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: Some(path.parent().unwrap().to_path_buf()) };
    let doc = sr_model::load_str(text, &opts).unwrap_or_else(|e| panic!("{e:?}\n{text}"));
    sr_eval::Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e:?}"))
}

/// The value of property `prop` of node `id` at time `t`.
fn num(ev: &sr_eval::Evaluator, id: &str, prop: &str, t: f64) -> f64 {
    let g = ev.evaluate(t);
    let n = g.nodes.iter().find(|n| &*n.id == id).unwrap_or_else(|| panic!("no node {id}"));
    n.props.get(prop).and_then(sr_eval::Value::as_num).unwrap_or_else(|| panic!("{id} has no {prop}"))
}

/// The including document of version `version`: `defs` before its composition, which holds the include of
/// `B.scene.xml` and `body`.
fn including(version: &str, defs: &str, body: &str) -> String {
    format!(
        r##"<scene version="{version}"><project width="200" height="200" fps="10" duration="2"/>{defs}<composition><include id="i" src="B.scene.xml"/>{body}</composition></scene>"##
    )
}

#[test]
fn an_included_expression_reads_its_documents_material() {
    let d = dir("prop");
    let inner = r##"<scene version="1.6"><project width="200" height="200" fps="10" duration="2"/>
<materials><material id="m" roughness="0.2"><animate property="roughness"><key time="0" value="0.2"/><key time="1" value="0.8"/></animate></material></materials>
<composition><shape id="g" shape="rect" width="10" height="10" fill="#FFFFFF"><expression property="opacity">prop("m.roughness")</expression></shape></composition></scene>"##;
    let direct = evaluator(&d.join("B.scene.xml"), inner);
    let parent = r##"<materials><material id="m" roughness="0.1"><animate property="roughness"><key time="0" value="0.1"/><key time="1" value="0.3"/></animate></material></materials>"##;
    let a = evaluator(&d.join("A.scene.xml"), &including("1.6", parent, ""));
    let alone = evaluator(&d.join("C.scene.xml"), &including("1.6", "", ""));
    let want = num(&direct, "g", "opacity", 0.5);
    assert!((want - 0.5).abs() < 1e-9, "the document by itself: {want}");
    assert_eq!(num(&a, "i/g", "opacity", 0.5), want, "beside a material of the same id, the include reads its own");
    assert_eq!(num(&alone, "i/g", "opacity", 0.5), want, "the include alone reads its own, animated");
}

#[test]
fn an_included_expression_places_by_its_documents_map() {
    let d = dir("geo");
    let map = |lon: f64| {
        format!(
            r#"<assets><map id="m" width="200" height="200" projection="orthographic" centerLon="{lon}" centerLat="0"/></assets>"#
        )
    };
    let inner = format!(
        r##"<scene version="1.6"><project width="200" height="200" fps="10" duration="2"/>{}
<composition><shape id="mark" shape="rect" width="4" height="4" fill="#FFFFFF"><expression property="x">geo('m', 30, 0)[0]</expression></shape></composition></scene>"##,
        map(0.0)
    );
    let direct = evaluator(&d.join("B.scene.xml"), &inner);
    let a = evaluator(&d.join("A.scene.xml"), &including("1.6", &map(60.0), ""));
    let want = num(&direct, "mark", "x", 0.0);
    // 30°E on an orthographic globe of radius 100 centred on 0°E: 100 + 100 sin 30°
    assert!((want - 150.0).abs() < 1e-6, "the document by itself: {want}");
    assert_eq!(num(&a, "i/mark", "x", 0.0), want, "beside a map of the same id, the include places by its own");
}

#[test]
fn an_included_ocean_takes_its_documents_material() {
    // the foam coverage of a basin of 601 x 601 vertices does not fit in 108 MiB (ocean_foam_albedo.rs): it is made
    // only for a water that takes foam, which an unlit one does not. The include's water is unlit, the including
    // document's of the same id lit: the include's surface is made, without the coverage.
    let d = dir("ocean");
    // an ocean is a document of version 1.3 exactly (OCN1)
    let inner = r##"<scene version="1.3"><project width="64" height="64" fps="10" duration="4"/>
<materials><material id="w" unlit="true"/></materials><composition>
  <ocean id="sea" width="600" depth="600" cellSize="1" bottomDepth="2" dt="0.1" surfaceMemoryMiB="108" material="w">
    <waterImpulse time="0.1" radius="3" amplitude="0.3"/>
    <whitewater emissionRate="60" threshold="0.05" lifetime="3" seed="5" foamMode="albedo"/>
  </ocean></composition></scene>"##;
    std::fs::write(d.join("B.scene.xml"), inner).unwrap();
    let a = evaluator(
        &d.join("A.scene.xml"),
        &including(
            "1.3",
            r##"<materials><material id="w" baseColor="#102040"/></materials>"##,
            r#"<camera id="cam" renderer="pathtrace"/>"#,
        ),
    );
    let f = a.evaluate(1.0);
    assert!(f.failures.is_empty(), "the unlit water of the include has no coverage to fail: {:?}", f.failures);
    let sea = f.nodes.iter().find(|n| &*n.id == "i/sea").and_then(|n| n.sim_ocean.clone());
    let sea = sea.expect("the include's surface is made");
    assert!(sea.mesh.vertices.iter().all(|v| v.color == [1.0; 4]), "no coverage is made");
}

/// The value of property `prop` of the element outside the composition keyed `key` at time `t`.
fn elem(ev: &sr_eval::Evaluator, key: &str, prop: &str, t: f64) -> f64 {
    let g = ev.evaluate(t);
    let e = g.elements.iter().find(|e| &*e.key == key).unwrap_or_else(|| panic!("no element {key}"));
    e.props.get(prop).and_then(sr_eval::Value::as_num).unwrap_or_else(|| panic!("{key} has no {prop}"))
}

#[test]
fn an_included_definition_runs_on_the_includes_clock() {
    // an include runs on its own clock, as an instance does: its material's keys and its expressions' `time` are the included
    // document's time, which starts with the include
    let d = dir("clock");
    let inner = r##"<scene version="1.6"><project width="200" height="200" fps="10" duration="2"/>
<materials><material id="m" roughness="0.2"><animate property="roughness"><key time="0" value="0.2"/><key time="1" value="0.8"/></animate><expression property="metallic">time / 2</expression></material></materials>
<composition><shape id="g" shape="rect" width="10" height="10" fill="#FFFFFF"><expression property="opacity">prop("m.roughness")</expression></shape></composition></scene>"##;
    let direct = evaluator(&d.join("B.scene.xml"), inner);
    assert!((elem(&direct, "m", "roughness", 0.5) - 0.5).abs() < 1e-9);
    assert!((elem(&direct, "m", "metallic", 0.5) - 0.25).abs() < 1e-9);
    let a = evaluator(
        &d.join("A.scene.xml"),
        r##"<scene version="1.6"><project width="200" height="200" fps="10" duration="4"/><composition><include id="i" src="B.scene.xml" start="1"/></composition></scene>"##,
    );
    // the include starts at 1 s: its time 0.5 is the frame at 1.5 s
    for (prop, want) in [("roughness", 0.5), ("metallic", 0.25)] {
        let got = elem(&a, "i/m", prop, 1.5);
        assert!((got - want).abs() < 1e-9, "{prop} of the include's material at its time 0.5: {got}, want {want}");
    }
    let got = num(&a, "i/g", "opacity", 1.5);
    assert!((got - 0.5).abs() < 1e-9, "the include's expression reads its material at its own time: {got}");
}

#[test]
fn an_included_expression_does_not_read_the_including_documents_definition() {
    // the included document declares no material m: prop("m.roughness") is an error there, as it is in the document
    // by itself, and the including document's m (animated or not) is not read in its place
    let d = dir("no-leak");
    let inner = r##"<scene version="1.6"><project width="200" height="200" fps="10" duration="2"/>
<composition><shape id="g" shape="rect" width="10" height="10" fill="#FFFFFF"><expression property="opacity">prop("m.roughness")</expression></shape></composition></scene>"##;
    let build = |path: &Path, text: &str| {
        std::fs::write(path, text).unwrap();
        let opts = sr_model::LoadOptions { verify_assets: false, base_dir: Some(path.parent().unwrap().to_path_buf()) };
        let doc = sr_model::load_str(text, &opts).unwrap_or_else(|e| panic!("{e:?}\n{text}"));
        sr_eval::Evaluator::new(&doc, &Default::default()).map(|_| ()).map_err(|e| format!("{e:?}"))
    };
    let direct = build(&d.join("B.scene.xml"), inner);
    assert!(direct.as_ref().is_err_and(|e| e.contains("no element has id")), "the document by itself: {direct:?}");
    for parent in [
        r##"<materials><material id="m" roughness="0.1"><animate property="roughness"><key time="0" value="0.1"/><key time="1" value="0.3"/></animate></material></materials>"##,
        r##"<materials><material id="m" roughness="0.1"/></materials>"##,
    ] {
        let a = build(&d.join("A.scene.xml"), &including("1.6", parent, ""));
        assert!(a.as_ref().is_err_and(|e| e.contains("no element has id")), "through the include: {a:?}");
    }
}

#[test]
fn an_included_definitions_expression_places_by_its_documents_map() {
    // an expression on a material (not on a node) of the included document reads that document's map
    let d = dir("geo-definition");
    let map = |lon: f64| {
        format!(
            r#"<assets><map id="mp" width="200" height="200" projection="orthographic" centerLon="{lon}" centerLat="0"/></assets>"#
        )
    };
    let inner = format!(
        r##"<scene version="1.6"><project width="200" height="200" fps="10" duration="2"/>{}
<materials><material id="m" roughness="0.2"><expression property="roughness">geo('mp', 30, 0)[0] / 1000</expression></material></materials>
<composition><shape id="g" shape="rect" width="4" height="4" fill="#FFFFFF"/></composition></scene>"##,
        map(0.0)
    );
    let direct = evaluator(&d.join("B.scene.xml"), &inner);
    let want = elem(&direct, "m", "roughness", 0.0);
    // 30°E on an orthographic globe of radius 100 centred on 0°E: 100 + 100 sin 30° = 150
    assert!((want - 0.15).abs() < 1e-9, "the document by itself: {want}");
    let a = evaluator(&d.join("A.scene.xml"), &including("1.6", &map(60.0), ""));
    assert_eq!(elem(&a, "i/m", "roughness", 0.0), want, "beside a map of the same id, the material reads its own");
    let alone = evaluator(&d.join("C.scene.xml"), &including("1.6", "", ""));
    assert_eq!(elem(&alone, "i/m", "roughness", 0.0), want, "with no map of that id beside it, it reads its own");
}

#[test]
fn an_included_documents_sections_that_are_not_merged_are_reported() {
    // an include merges assets, effects, symbols and materials; any other section of the included document is
    // reported. Its lights do not light the include's content (the including document's do).
    let d = dir("reported");
    let inner = r##"<scene version="1.6"><project width="200" height="200" fps="10" duration="2"/>
<composition><shape id="g" shape="rect" width="10" height="10" fill="#FFFFFF"/></composition>
<lights><light id="sun" type="directional" intensity="2" yaw="35" pitch="-48"/></lights></scene>"##;
    let direct = evaluator(&d.join("B.scene.xml"), inner);
    assert!(!direct.warnings().iter().any(|w| w.code == "E12"), "the document by itself: {:?}", direct.warnings());
    let a = evaluator(&d.join("A.scene.xml"), &including("1.6", "", ""));
    let found: Vec<_> = a.warnings().iter().filter(|w| w.code == "E12").collect();
    assert_eq!(found.len(), 1, "{:?}", a.warnings());
    assert!(!found[0].is_error() && !found[0].is_info(), "a warning: {:?}", found[0]);
    assert!(found[0].message.contains("<lights>") && found[0].message.contains("\"i\""), "{}", found[0].message);
}

// ------------------------------------------------------------------ node references inside an include
//
// The ids of an included document's nodes carry the include's namespace (`i/knee`), while the references written in
// it do not (`pole="knee"`). Each reference names the node (or bone, crater, body) of its own document: the include
// evaluates as its document evaluates by itself, and a node of the including document with the same id is never taken
// in its place.

/// The document `<scene version="{version}">` with `project` (the project element's attributes), `body` as its
/// composition and `post` after it.
fn document(version: &str, project: &str, body: &str, post: &str) -> String {
    format!(r##"<scene version="{version}"><project {project}/><composition>{body}</composition>{post}</scene>"##)
}

/// The world transform of node `id` at `t`, and of each of its bones.
fn worlds(ev: &sr_eval::Evaluator, id: &str, t: f64) -> Vec<[f64; 6]> {
    let g = ev.evaluate(t);
    assert!(g.problems.is_empty() && g.failures.is_empty(), "{:?} {:?}", g.problems, g.failures);
    let n = g.nodes.iter().find(|n| &*n.id == id).unwrap_or_else(|| panic!("no node {id}"));
    std::iter::once(n.world.0).chain(n.bones.iter().map(|b| b.world.0)).collect()
}

/// An IK constraint's `pole` names a node of its own document, on a skeleton's chain of two bones and on a node.
const POLE_BODY: &str = r##"<shape id="goal" shape="ellipse" x="15" y="5" width="6" height="6"/>
<shape id="knee" shape="ellipse" x="0" y="-100" width="6" height="6"/>
<skeleton id="rig"><bone id="a" length="10"/><bone id="b" parent="a" x="10" length="10"/><transformConstraint type="ik" target="goal" pole="knee"/></skeleton>
<shape id="goal2" shape="ellipse" x="130" y="80" width="6" height="6"/>
<group id="arm" x="100" y="100"><shape id="fore" shape="rect" x="20" width="20" height="4" fill="#FFFFFF"><transformConstraint type="ik" target="goal2" pole="knee" offsetX="20"/></shape></group>"##;

#[test]
fn an_included_ik_pole_is_its_documents_node() {
    let d = dir("pole");
    let project = r#"width="400" height="400" fps="10" duration="1""#;
    let direct = evaluator(&d.join("B.scene.xml"), &document("1.2", project, POLE_BODY, ""));
    // the pole decides the bend: with the knee below instead of above, both chains bend the other way
    let below = evaluator(
        &d.join("C.scene.xml"),
        &document("1.2", project, &POLE_BODY.replace(r#"y="-100""#, r#"y="300""#), ""),
    );
    for id in ["rig", "fore", "arm"] {
        assert_ne!(worlds(&direct, id, 0.0), worlds(&below, id, 0.0), "{id}: the pole bends the chain");
    }
    let include = r#"<include id="i" src="B.scene.xml"/>"#;
    let alone = evaluator(&d.join("A.scene.xml"), &document("1.2", project, include, ""));
    // the including document declares a knee (and goals) of the same ids on the other side, before and after the include
    let knee = r##"<shape id="knee" shape="ellipse" x="0" y="300" width="6" height="6"/>"##;
    let goals = r##"<shape id="goal" shape="ellipse" x="-15" y="5" width="6" height="6"/><shape id="goal2" shape="ellipse" x="60" y="60" width="6" height="6"/>"##;
    let before = evaluator(&d.join("P.scene.xml"), &document("1.2", project, &format!("{knee}{goals}{include}"), ""));
    let after = evaluator(&d.join("Q.scene.xml"), &document("1.2", project, &format!("{include}{knee}{goals}"), ""));
    for id in ["rig", "fore", "arm"] {
        let want = worlds(&direct, id, 0.0);
        assert_eq!(worlds(&alone, &format!("i/{id}"), 0.0), want, "{id}: the include bends as its document does");
        assert_eq!(worlds(&before, &format!("i/{id}"), 0.0), want, "{id}: beside a knee of the same id (before it)");
        assert_eq!(worlds(&after, &format!("i/{id}"), 0.0), want, "{id}: beside a knee of the same id (after it)");
    }
}

#[test]
fn an_included_constraint_on_a_bone_names_its_documents_bone() {
    // a constraint's target (and an IK pole) may name a bone: the bone of the included document's own skeleton, not
    // the bone of the same id of a skeleton of the including document that comes first
    let d = dir("bone-target");
    let project = r#"width="400" height="400" fps="10" duration="1""#;
    let body = r##"<group id="g" x="50" y="50"><skeleton id="rig"><bone id="b" length="10" rotation="30"/><bone id="tip" parent="b" x="10" length="5"/></skeleton></group>
<shape id="dot" shape="rect" width="4" height="4" fill="#FFFFFF"><transformConstraint type="copy-position" target="tip"/></shape>
<shape id="goal" shape="ellipse" x="215" y="5" width="6" height="6"/>
<group id="lg" x="200"><skeleton id="leg"><bone id="thigh" length="10"/><bone id="shin" parent="thigh" x="10" length="10"/><transformConstraint type="ik" target="goal" pole="tip"/></skeleton></group>"##;
    let direct = evaluator(&d.join("B.scene.xml"), &document("1.2", project, body, ""));
    let dot = worlds(&direct, "dot", 0.0);
    assert!((dot[0][4] - 50.0).abs() > 1.0, "the dot is moved to the bone's tip: {dot:?}");
    // with the pole on the other side of the leg, the leg bends the other way: the pole is read
    let flipped =
        evaluator(&d.join("C.scene.xml"), &document("1.2", project, &body.replace(r#"y="50""#, r#"y="-300""#), ""));
    assert_ne!(worlds(&direct, "leg", 0.0), worlds(&flipped, "leg", 0.0), "the leg bends toward the bone it names");
    let include = r#"<include id="i" src="B.scene.xml"/>"#;
    let alone = evaluator(&d.join("A.scene.xml"), &document("1.2", project, include, ""));
    let theirs = r##"<group id="g" x="300" y="-300"><skeleton id="rig"><bone id="b" length="40" rotation="-60"/><bone id="tip" parent="b" x="40" length="5"/></skeleton></group>"##;
    let before = evaluator(&d.join("P.scene.xml"), &document("1.2", project, &format!("{theirs}{include}"), ""));
    for id in ["dot", "leg"] {
        let want = worlds(&direct, id, 0.0);
        assert_eq!(worlds(&alone, &format!("i/{id}"), 0.0), want, "{id}: through the include");
        assert_eq!(
            worlds(&before, &format!("i/{id}"), 0.0),
            want,
            "{id}: beside a skeleton with a bone of the same id"
        );
    }
}

#[test]
fn an_include_does_not_reach_the_including_documents_nodes() {
    // `x` is a node of the included document only inside an instance of its symbol: at the top level the reference
    // names nothing (the constraint does not apply), and the including document's top-level `x` is not taken instead
    let d = dir("boundary");
    let project = r#"width="400" height="400" fps="10" duration="1""#;
    let post = r##"<symbols><symbol id="s" width="10" height="10"><shape id="x" shape="rect" x="5" width="4" height="4"/></symbol></symbols>"##;
    let body = r##"<instance id="inst" symbol="s" x="100" y="100"/><shape id="dot" shape="rect" x="20" y="20" width="4" height="4"><transformConstraint type="copy-position" target="x"/></shape>"##;
    let inner =
        format!(r##"<scene version="1.2"><project {project}/>{post}<composition>{body}</composition></scene>"##);
    let direct = evaluator(&d.join("B.scene.xml"), &inner);
    let want = worlds(&direct, "dot", 0.0);
    let x = r##"<shape id="x" shape="rect" x="300" y="300" width="4" height="4"/>"##;
    let a = evaluator(
        &d.join("A.scene.xml"),
        &document("1.2", project, &format!(r#"{x}<include id="i" src="B.scene.xml"/>"#), ""),
    );
    assert_eq!(worlds(&a, "i/dot", 0.0), want, "the include's reference does not name the including document's x");
}

/// A rock that a ball breaks, in the physics of `physics`: `fracture source="ball"`.
const FRACTURE_BODY: &str = r##"<object3D id="ball" primitive="sphere" radius="1" x="-8">
  <rigidBody mass="1" velocityX="20" linearDamping="0" angularDamping="0" restitution="0"/>
</object3D>
<object3D id="rock" primitive="box" width="4" height="4" depth="4">
  <rigidBody mass="8" linearDamping="0" angularDamping="0" restitution="0"/>
  <fracture pieces="4" seed="7" interiorMaterial="inside" source="ball"/>
</object3D>"##;
const FRACTURE_PHYSICS: &str = r#"<physics gravityY="0" pixelsPerMeter="1" fixedStep="0.005"/>"#;

/// The fracture of node `id` at `t`: none before it breaks, else its pieces' poses.
fn fracture(ev: &sr_eval::Evaluator, id: &str, t: f64) -> Option<String> {
    let g = ev.evaluate(t);
    assert!(g.problems.is_empty() && g.failures.is_empty(), "t = {t}: {:?} {:?}", g.problems, g.failures);
    let n = g.nodes.iter().find(|n| &*n.id == id).unwrap_or_else(|| panic!("no node {id}"));
    n.fracture.as_ref().map(|f| format!("{:?}", f.poses))
}

#[test]
fn an_included_fracture_is_broken_by_its_documents_body() {
    let d = dir("fracture");
    let project = r#"width="64" height="64" fps="20" duration="4""#;
    let inner = format!(
        r##"<scene version="1.3"><project {project}/><materials><material id="inside" baseColor="#ff0000"/></materials><composition>{FRACTURE_BODY}</composition>{FRACTURE_PHYSICS}</scene>"##
    );
    let direct = evaluator(&d.join("B.scene.xml"), &inner);
    // the ball reaches the rock at 0.25 s
    assert!(fracture(&direct, "rock", 0.2).is_none(), "not broken before the ball arrives");
    let want = fracture(&direct, "rock", 1.0).expect("broken by the ball");
    let include = r#"<include id="i" src="B.scene.xml"/>"#;
    let alone = evaluator(&d.join("A.scene.xml"), &document("1.3", project, include, FRACTURE_PHYSICS));
    assert!(fracture(&alone, "i/rock", 0.2).is_none());
    assert_eq!(fracture(&alone, "i/rock", 1.0).as_ref(), Some(&want), "through the include");
    // the including document's ball of the same id is a static body far away: it breaks nothing
    let ball =
        r#"<object3D id="ball" primitive="sphere" radius="1" x="500" y="500"><rigidBody type="static"/></object3D>"#;
    let beside =
        evaluator(&d.join("P.scene.xml"), &document("1.3", project, &format!("{ball}{include}"), FRACTURE_PHYSICS));
    assert!(fracture(&beside, "i/rock", 0.2).is_none());
    assert_eq!(fracture(&beside, "i/rock", 1.0).as_ref(), Some(&want), "beside a ball of the same id");
}

/// The impact scene of the splash tests: a rock that makes the crater `pit` in the ground, an emitter whose burst
/// throws the crater's ejecta (`crater="pit"`) and an ocean the ejecta fall into (`splash="debris"`).
fn impact_body() -> String {
    const GRAVITY: f64 = 9.80665;
    const FLIGHT: f64 = 1.4888;
    let (vx, vy) = (100.0 * 60f64.to_radians().cos(), 100.0 * 60f64.to_radians().sin());
    let launch = vy - GRAVITY * FLIGHT;
    let (x, y) = (-vx * FLIGHT, -2.0 - launch * FLIGHT - 0.5 * GRAVITY * FLIGHT * FLIGHT);
    format!(
        r##"<object3D id="rock" primitive="sphere" radius="2" x="{x}" y="{y}">
  <rigidBody shape="sphere" mass="90478" velocityX="{vx}" velocityY="{launch}" restitution="0" linearDamping="0" angularDamping="0"/>
</object3D>
<object3D id="ground" primitive="plane" width="160" height="160" segments="160" y="0" rotationX="-90">
  <crater id="pit" source="rock" targetMaterial="softRock"/>
  <rigidBody type="static" shape="auto"/>
</object3D>
<particles3D id="debris" rate="0" lifetime="8" dt="0.0416666666666667" gravityY="9.80665" maxParticles="400">
  <burst crater="pit" count="400"/>
</particles3D>
<ocean id="sea" y="1" width="400" depth="400" cellSize="4" dt="0.0416666666666667" boundary="open" splash="debris"/>"##
    )
}
const IMPACT_PHYSICS: &str = r#"<physics gravityY="-9.80665" pixelsPerMeter="1" fixedStep="0.008333333333333333" bounds="none" fixInternalEdges="true"/>"#;

/// The particles of emitter `debris` at `t` and what ocean `sea` was given in its canonical steps up to `steps`.
fn ejecta(ev: &sr_eval::Evaluator, debris: &str, sea: &str, t: f64, steps: u64) -> (String, String) {
    let g = ev.evaluate(t);
    assert!(g.problems.is_empty() && g.failures.is_empty(), "t = {t}: {:?} {:?}", g.problems, g.failures);
    let n = g.nodes.iter().find(|n| &*n.id == debris).unwrap_or_else(|| panic!("no node {debris}"));
    let particles = format!("{:?}", n.particles3d.as_ref().expect("simulated").frame.particles);
    let given: Vec<_> =
        (0..=steps).map(|s| ev.splash_into(sea, s).unwrap_or_else(|e| panic!("step {s}: {e}"))).collect();
    (particles, format!("{given:?}"))
}

#[test]
fn an_included_burst_and_splash_name_their_documents_crater_and_emitter() {
    let d = dir("crater-splash");
    let project = r#"width="64" height="64" fps="24" duration="8""#;
    let direct = evaluator(&d.join("B.scene.xml"), &document("1.3", project, &impact_body(), IMPACT_PHYSICS));
    // the impact is at 1.49 s; by 7 s some ejecta have fallen into the water (160 steps of 1/24 s)
    let want = ejecta(&direct, "debris", "sea", 7.0, 160);
    assert!(
        want.1.contains("volume"),
        "ejecta fall into the water: {} {}",
        &want.0[..want.0.len().min(300)],
        want.1.len()
    );
    let include = r#"<include id="i" src="B.scene.xml"/>"#;
    let alone = evaluator(&d.join("A.scene.xml"), &document("1.3", project, include, IMPACT_PHYSICS));
    assert_eq!(ejecta(&alone, "i/debris", "i/sea", 7.0, 160), want, "through the include");
    // the including document has a crater `pit` (whose body never hits) and an emitter `debris` of its own
    let theirs = r##"<object3D id="prock" primitive="sphere" radius="1" x="2000" y="-100"><rigidBody shape="sphere" mass="1"/></object3D>
<object3D id="pground" primitive="plane" width="10" height="10" x="1000" y="-50" rotationX="-90"><crater id="pit" source="prock" targetMaterial="softRock"/><rigidBody type="static" shape="auto"/></object3D>
<particles3D id="debris" rate="0" lifetime="1" maxParticles="10"/>"##;
    let beside =
        evaluator(&d.join("P.scene.xml"), &document("1.3", project, &format!("{theirs}{include}"), IMPACT_PHYSICS));
    assert_eq!(ejecta(&beside, "i/debris", "i/sea", 7.0, 160), want, "beside a crater and an emitter of the same ids");
}

#[test]
fn a_constraint_naming_a_bone_hidden_in_an_instance_or_a_repeat_is_reported() {
    // ids inside a symbol instance or a repeat copy are scoped to it (effective id `instanceId/innerId`): from outside
    // it, `tip` names no bone, as `x` names no node in `an_include_does_not_reach_the_including_documents_nodes`. The
    // constraint has no effect, and that is reported, not silent.
    let d = dir("hidden-bone");
    let project = r#"width="400" height="400" fps="10" duration="1""#;
    let rig = r##"<skeleton id="rig"><bone id="b" rotation="30"/><bone id="tip" parent="b" x="20"/></skeleton>"##;
    let dot = |constraint: &str| {
        format!(r##"<shape id="dot" shape="rect" width="4" height="4" fill="#FFFFFF">{constraint}</shape>"##)
    };
    let copy = r#"<transformConstraint type="copy-position" target="tip"/>"#;
    let free = worlds(&evaluator(&d.join("F.scene.xml"), &document("1.2", project, &dot(""), "")), "dot", 0.0);
    let symbols = format!(r#"<symbols><symbol id="s">{rig}</symbol></symbols>"#);
    let cases = [
        ("instance", format!(r#"<instance id="hero" symbol="s" x="100" y="100"/>{}"#, dot(copy)), symbols.as_str()),
        ("repeat", format!(r#"<repeat id="r" count="2">{rig}</repeat>{}"#, dot(copy)), ""),
    ];
    for (name, body, defs) in cases {
        let text =
            format!(r##"<scene version="1.2"><project {project}/>{defs}<composition>{body}</composition></scene>"##);
        let ev = evaluator(&d.join(format!("{name}.scene.xml")), &text);
        assert_eq!(worlds(&ev, "dot", 0.0), free, "{name}: the bone inside is not the constraint's target");
        let found: Vec<_> = ev.warnings().iter().filter(|w| w.code == "E11" && w.message.contains("\"tip\"")).collect();
        assert_eq!(found.len(), 1, "{name}: {:?}", ev.warnings());
        assert!(!found[0].is_error() && found[0].message.contains("\"dot\""), "{name}: {:?}", found[0]);
    }
    // a bone of a skeleton in the constraint's own scope is its target, and nothing is reported
    let ev = evaluator(&d.join("V.scene.xml"), &document("1.2", project, &format!("{rig}{}", dot(copy)), ""));
    assert_ne!(worlds(&ev, "dot", 0.0), free, "the visible bone moves the dot");
    assert!(!ev.warnings().iter().any(|w| w.code == "E11"), "{:?}", ev.warnings());
}

#[test]
fn a_resolved_reference_names_its_node_beside_a_nested_include_of_the_same_id() {
    // B includes C as `a`, and B is included as `a`. B's cloud is parented to B's carrier, which the compiler names
    // `a/carrier`; C's carrier is `a/a/carrier`. The resolved id names B's carrier, never C's.
    let d = dir("nested-same-id");
    let project = r#"<project width="32" height="32" fps="10" duration="1"/>"#;
    let physics = r#"<physics pixelsPerMeter="10" gravityY="0" bounds="none"><forceField id="wind" type="directional" forceX="1" affects="particles"/></physics>"#;
    std::fs::write(
        d.join("C.scene.xml"),
        format!(
            r#"<scene version="1.3">{project}<composition><object3D id="carrier" primitive="box"/></composition></scene>"#
        ),
    )
    .unwrap();
    let inner = |turn: &str| {
        format!(
            r#"<scene version="1.3">{project}<composition><include id="a" src="C.scene.xml"/><object3D id="carrier" primitive="box" rotationY="{turn}"/>
<object3D id="cloud" primitive="volume" parent="carrier"><pyro width="8" height="8" depth="8" voxelSize="1" dt="0.1" boundary="open"/></object3D></composition>{physics}</scene>"#
        )
    };
    let velocity = |g: &sr_eval::FrameGraph, id: &str| -> Vec<f64> {
        assert!(g.problems.is_empty() && g.failures.is_empty(), "{:?} {:?}", g.problems, g.failures);
        let n = g.nodes.iter().find(|n| &*n.id == id).unwrap_or_else(|| panic!("no node {id}"));
        let v = n.sim_volume.as_ref().expect("simulated");
        ["velocity.x", "velocity.y", "velocity.z"].map(|c| v.data.grid(c).unwrap().sample_world([0.5; 3]) as f64).into()
    };
    let direct = evaluator(&d.join("B.scene.xml"), &inner("90")).evaluate(0.1);
    let want = velocity(&direct, "cloud");
    let close = |a: &[f64], b: &[f64]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6);
    let unturned = evaluator(&d.join("U.scene.xml"), &inner("0")).evaluate(0.1);
    let other = velocity(&unturned, "cloud");
    assert!(!close(&other, &want), "the carrier's turn reaches the cloud: {other:?} and {want:?}");
    let a = evaluator(
        &d.join("A.scene.xml"),
        &format!(
            r#"<scene version="1.3">{project}<composition><include id="a" src="B.scene.xml"/></composition>{physics}</scene>"#
        ),
    );
    let g = a.evaluate(0.1);
    let cloud = g.nodes.iter().position(|n| &*n.id == "a/cloud").expect("a/cloud");
    let carrier = g.nodes.iter().position(|n| &*n.id == "a/carrier").expect("a/carrier");
    assert_eq!(sr_eval::resolve_in(&g.nodes, &g.nodes[cloud].scope, "a/carrier"), Some(carrier));
    let got = velocity(&g, "a/cloud");
    assert!(close(&got, &want), "through the include, the cloud turns with its own carrier: {got:?}, not {want:?}");
}

#[test]
fn an_included_document_reads_its_own_parameters() {
    // a repeat's @over, a link from `param:`, an expression's param() and a text placeholder written in the included
    // document read its parameters (their default values), never the including document's of the same name
    let d = dir("params");
    let project = r#"width="200" height="200" fps="10" duration="1""#;
    let inner = format!(
        r##"<scene version="1.2"><project {project}/>
<parameters><param id="items" type="list" default="a,b,c"/><param id="k" type="number" default="3"/></parameters>
<assets><text id="txt" text="{{{{item}}}} {{{{who}}}}" width="100" height="20" size="10"/></assets>
<composition><repeat id="r" over="items" var="item"><layer id="t" asset="txt"/></repeat>
<shape id="s" shape="rect" width="4" height="4" fill="#FFFFFF"><link property="rotation" source="param:k" scale="10"/><expression property="x">param("k")</expression></shape></composition></scene>"##
    );
    let read = |ev: &sr_eval::Evaluator, ns: &str| {
        let g = ev.evaluate(0.0);
        assert!(g.problems.is_empty() && g.failures.is_empty(), "{:?} {:?}", g.problems, g.failures);
        let copies: Vec<String> = g
            .nodes
            .iter()
            .filter(|n| n.id.starts_with(&format!("{ns}r[")) && n.id.ends_with("/t"))
            .map(|n| format!("{} {:?}", &n.id[ns.len()..], n.text))
            .collect();
        let s = g.nodes.iter().find(|n| *n.id == *format!("{ns}s")).expect("s");
        (copies, format!("{:?} {:?}", s.props.get("rotation"), s.props.get("x")))
    };
    let direct = evaluator(&d.join("B.scene.xml"), &inner);
    let want = read(&direct, "");
    assert_eq!(want.0.len(), 3, "{want:?}");
    assert!(want.0[0].contains("a {{who}}") && want.1.contains("30"), "the document by itself: {want:?}");
    let include = r#"<include id="i" src="B.scene.xml"/>"#;
    let alone = evaluator(&d.join("A.scene.xml"), &document("1.2", project, include, ""));
    assert_eq!(read(&alone, "i/"), want, "through the include");
    let theirs = r#"<parameters><param id="items" type="list" default="z"/><param id="k" type="number" default="7"/><param id="who" type="string" default="Z"/></parameters>"#;
    let beside = evaluator(
        &d.join("P.scene.xml"),
        &format!(r##"<scene version="1.2"><project {project}/>{theirs}<composition>{include}</composition></scene>"##),
    );
    assert_eq!(read(&beside, "i/"), want, "beside parameters of the same names");
}
