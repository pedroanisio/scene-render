//! SREP 87 as the evaluator sees it: references to program-emitted ids reach the namespaced element, scopes compose
//! with instances, generated marker ids are untouched, and values derived from an element's id use its local id.
//! The modules are `srep66-rect.wasm` of the sr-core kit (a rect `g1` at x 200) and constant modules written here.

use std::path::PathBuf;

use sr_eval::{EvalOptions, Evaluator};
use sr_model::{load_str, LoadOptions};

struct Kit {
    dir: PathBuf,
}

impl Kit {
    fn new(tag: &str) -> Kit {
        std::env::set_var("SR_PROGRAM_CACHE", "off");
        let dir = std::env::temp_dir().join(format!("sr-eval-srep87-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let rect = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sr-wasm/tests/modules/srep66-rect.wasm");
        std::fs::copy(rect, dir.join("srep66-rect.wasm")).unwrap();
        Kit { dir }
    }

    fn constant(&self, name: &str, fragment: &str) {
        let bytes: String = fragment.bytes().map(|b| format!("\\{b:02x}")).collect();
        let wasm = wat::parse_str(format!(
            r#"(module (memory (export "memory") 1) (data (i32.const 0) "{bytes}") (func (export "generate") (result i64) i64.const {}))"#,
            fragment.len()
        ))
        .unwrap();
        std::fs::write(self.dir.join(format!("{name}.wasm")), wasm).unwrap();
    }

    fn program(&self, id: &str, name: &str) -> String {
        use sha2::Digest;
        let b = std::fs::read(self.dir.join(format!("{name}.wasm"))).unwrap();
        let sha: String = sha2::Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect();
        format!(r#"<program id="{id}" src="{name}.wasm" sha256="{sha}"/>"#)
    }

    fn evaluator(&self, before: &str, body: &str, layout: Option<&str>) -> Evaluator {
        let xml = format!(
            r#"<scene version="1.6"><project width="640" height="360" fps="24" duration="4" seed="1"/>{before}<composition>{body}</composition></scene>"#
        );
        let doc = load_str(&xml, &LoadOptions { verify_assets: true, base_dir: Some(self.dir.clone()) })
            .unwrap_or_else(|e| panic!("{e}"));
        let opts = EvalOptions { layout: layout.map(str::to_string), ..Default::default() };
        Evaluator::new(&doc, &opts).unwrap_or_else(|r| panic!("{r}"))
    }
}

impl Drop for Kit {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn index(e: &Evaluator, id: &str) -> usize {
    let p = e.program();
    p.nodes
        .iter()
        .position(|n| &*n.id == id)
        .unwrap_or_else(|| panic!("no node {id}; have {:?}", p.nodes.iter().map(|n| &n.id).collect::<Vec<_>>()))
}

fn x_at(e: &Evaluator, id: &str, t: f64) -> Option<sr_eval::Value> {
    let g = e.evaluate(t);
    g.nodes.iter().find(|n| &*n.id == id).and_then(|n| n.props.get("x").cloned())
}

const RECT: &str = r##"width="10" height="10" fill="#FF0000""##;

#[test]
fn an_emitted_matte_reference_reaches_the_programs_own_element() {
    // Semantics 3.1: each copy of the module is matted by its own rect
    let k = Kit::new("matte");
    k.constant(
        "matted",
        &format!(r#"<shape id="m" shape="rect" {RECT}/><shape id="s" shape="rect" {RECT} matte="m"/>"#),
    );
    let e = k.evaluator("", &(k.program("a", "matted") + &k.program("b", "matted")), None);
    let p = e.program();
    assert_eq!(p.nodes[index(&e, "a.s")].matte, Some(index(&e, "a.m") as u32));
    assert_eq!(p.nodes[index(&e, "b.s")].matte, Some(index(&e, "b.m") as u32));
}

#[test]
fn a_bare_prop_follows_the_one_emitter_as_the_qualified_one_does() {
    // Semantics 4; kits srep-NNNN-program-ids-bare-prop and -qualified-prop
    let k = Kit::new("prop");
    let follower = |id: &str| {
        format!(
            r#"<shape id="f" shape="rect" {RECT}><expression property="x">prop("{id}.x") + 10</expression></shape>"#
        )
    };
    let bare = k.evaluator("", &(k.program("a", "srep66-rect") + &follower("g1")), None);
    let qualified = k.evaluator("", &(k.program("a", "srep66-rect") + &follower("a.g1")), None);
    let x = x_at(&bare, "f", 0.5);
    assert!(x.is_some(), "the expression drives x");
    assert_eq!(x, x_at(&qualified, "f", 0.5));
}

#[test]
fn a_bare_layout_override_applies_to_the_namespaced_element() {
    // Semantics 4; kit srep-NNNN-program-ids-bare-override
    let k = Kit::new("override");
    let layouts = r##"<layouts><layout id="tall" width="360" height="640"><override target="g1" property="width" value="77"/></layout></layouts>"##;
    use sr_model::element::Element;
    let width = |layout: Option<&str>| {
        let e = k.evaluator(layouts, &k.program("a", "srep66-rect"), layout);
        let n = &e.program().nodes[index(&e, "a.g1")];
        format!("{:?}", (&*n.elem as &dyn Element).get_attr("width"))
    };
    let (tall, plain) = (width(Some("tall")), width(None));
    assert!(tall.contains("77") && tall != plain, "{tall} / {plain}");
}

#[test]
fn ids_derived_values_use_the_local_id() {
    // Semantics 7: the per-node seed of an emitted node is the seed of the same node written by hand with its local id;
    // kit srep-NNNN-program-ids-seed-local
    let k = Kit::new("seed");
    let emitted = k.evaluator("", &k.program("p", "srep66-rect"), None);
    let by_hand = k.evaluator(
        "",
        r##"<group id="p"><shape id="g1" shape="rect" width="40" height="20" anchorX="20" anchorY="10" x="200" y="120" fill="#FF0000FF"/></group>"##,
        None,
    );
    let a = &emitted.program().nodes[index(&emitted, "p.g1")];
    let b = &by_hand.program().nodes[index(&by_hand, "g1")];
    assert_eq!(a.seed, b.seed);
    // and a wiggle without a seed moves the same
    k.constant(
        "wiggly",
        &format!(
            r#"<shape id="g1" shape="rect" {RECT} x="100"><expression property="x">wiggle(2, 30)</expression></shape>"#
        ),
    );
    let emitted = k.evaluator("", &k.program("p", "wiggly"), None);
    let by_hand = k.evaluator(
        "",
        &format!(r#"<group id="p"><shape id="g1" shape="rect" {RECT} x="100"><expression property="x">wiggle(2, 30)</expression></shape></group>"#),
        None,
    );
    for t in [0.3, 1.1, 2.7] {
        assert_eq!(x_at(&emitted, "p.g1", t), x_at(&by_hand, "g1", t), "t = {t}");
    }
}

#[test]
fn a_program_named_beat_does_not_disturb_generated_beat_markers() {
    // Hazards of the "." separator; kit srep-NNNN-program-ids-beat-program
    let k = Kit::new("beat");
    k.constant("x1", &format!(r#"<shape id="x1" shape="rect" {RECT}/>"#));
    let markers = r#"<markers><beatGrid bpm="120"/></markers>"#;
    let starter = format!(r#"<shape id="s" shape="rect" {RECT} startMarker="beat.3"/>"#);
    let named_beat = k.evaluator(markers, &(k.program("beat", "x1") + &starter), None);
    let named_other = k.evaluator(markers, &(k.program("q", "x1") + &starter), None);
    let start = |e: &Evaluator| e.program().nodes[index(e, "s")].start;
    assert!(start(&named_beat) > 0.0);
    assert_eq!(start(&named_beat), start(&named_other));
    index(&named_beat, "beat.x1");
}

#[test]
fn a_program_inside_a_symbol_has_its_ids_scoped_by_each_instance() {
    // Semantics 8: the effective id is instanceId/P.E
    let k = Kit::new("symbol");
    let symbols = format!(
        r#"<symbols><symbol id="sym" width="64" height="64">{}</symbol></symbols>"#,
        k.program("p", "srep66-rect")
    );
    let e = k.evaluator(&symbols, r#"<instance id="i1" symbol="sym"/><instance id="i2" symbol="sym" x="100"/>"#, None);
    index(&e, "i1/p.g1");
    index(&e, "i2/p.g1");
}

impl Kit {
    /// The build of the document, or the codes of its errors.
    fn build(&self, before: &str, body: &str) -> Result<Evaluator, Vec<String>> {
        let xml = format!(
            r#"<scene version="1.6"><project width="640" height="360" fps="24" duration="4" seed="1"/>{before}<composition>{body}</composition></scene>"#
        );
        let doc = load_str(&xml, &LoadOptions { verify_assets: true, base_dir: Some(self.dir.clone()) })
            .unwrap_or_else(|e| panic!("{e}"));
        Evaluator::new(&doc, &EvalOptions::default())
            .map_err(|r| r.diagnostics.iter().filter(|d| d.is_error()).map(|d| d.code.clone()).collect())
    }

    /// Writes the document `name` holding `body` in its composition.
    fn document(&self, name: &str, body: &str) {
        let xml = format!(
            r#"<scene version="1.6"><project width="640" height="360" fps="24" duration="4" seed="1"/><composition>{body}</composition></scene>"#
        );
        std::fs::write(self.dir.join(name), xml).unwrap();
    }
}

fn width(e: &Evaluator, id: &str) -> String {
    use sr_model::element::Element;
    format!("{:?}", (&*e.program().nodes[index(e, id)].elem as &dyn Element).get_attr("width"))
}

fn w13(e: &Evaluator) -> Vec<String> {
    e.warnings().iter().filter(|w| w.code == "W13").map(|w| w.message.clone()).collect()
}

#[test]
fn an_instance_override_naming_a_local_id_reaches_the_namespaced_element() {
    // Backwards compatibility: SREP 66 accepted an instance's override naming an id a program in the symbol emits;
    // it resolves in the instance's scope as a bare reference does (Semantics 4: the one emitter, W13)
    let k = Kit::new("inst-override");
    let symbols = format!(
        r#"<symbols><symbol id="sym" width="64" height="64">{}</symbol></symbols>"#,
        k.program("p", "srep66-rect")
    );
    let body = r#"<instance id="i1" symbol="sym"><override target="g1" property="width" value="77"/></instance><instance id="i2" symbol="sym"/>"#;
    let e = k.build(&symbols, body).unwrap_or_else(|c| panic!("{c:?}"));
    assert!(width(&e, "i1/p.g1").contains("77"), "{}", width(&e, "i1/p.g1"));
    assert!(!width(&e, "i2/p.g1").contains("77"));
    assert_eq!(w13(&e).len(), 1, "{:?}", e.warnings());
    // written in full, the same override warns nothing
    let full = body.replace(r#"target="g1""#, r#"target="p.g1""#);
    let e = k.build(&symbols, &full).unwrap_or_else(|c| panic!("{c:?}"));
    assert!(width(&e, "i1/p.g1").contains("77"));
    assert!(w13(&e).is_empty());
}

#[test]
fn an_instance_override_naming_an_id_two_programs_of_the_symbol_emit_is_prg17() {
    let k = Kit::new("inst-ambiguous");
    let symbols = format!(
        r#"<symbols><symbol id="sym" width="64" height="64">{}{}</symbol></symbols>"#,
        k.program("p", "srep66-rect"),
        k.program("q", "srep66-rect")
    );
    let body = r#"<instance id="i1" symbol="sym"><override target="g1" property="width" value="77"/></instance>"#;
    let Err(codes) = k.build(&symbols, body) else { panic!("an ambiguous override fails") };
    assert!(codes.contains(&"PRG17".to_string()), "{codes:?}");
    assert!(!codes.contains(&"E05".to_string()), "{codes:?}");
}

#[test]
fn an_include_override_naming_a_local_id_of_the_included_document_reaches_it() {
    let k = Kit::new("inc-override");
    k.document("B.scene.xml", &k.program("p", "srep66-rect"));
    let body = r#"<include id="i" src="B.scene.xml"><override target="g1" property="width" value="77"/></include>"#;
    let e = k.build("", body).unwrap_or_else(|c| panic!("{c:?}"));
    assert!(width(&e, "i/p.g1").contains("77"), "{}", width(&e, "i/p.g1"));
    assert_eq!(w13(&e).len(), 1, "{:?}", e.warnings());
}

#[test]
fn skin_weights_name_the_bones_of_an_emitted_skeleton_by_their_local_ids() {
    // the module writes the weights file for its own output, which knows only local ids
    let k = Kit::new("skin");
    std::fs::write(k.dir.join("w.json"), r#"{"points":[[0,0]],"weights":[{"b0":1.0}]}"#).unwrap();
    k.constant("rig", r#"<skeleton id="rig" weights="w.json"><bone id="b0" length="10"/></skeleton>"#);
    let e = k.build("", &k.program("p", "rig")).unwrap_or_else(|c| panic!("{c:?}"));
    assert!(e.program().skins.contains_key("p.rig"));
}

#[test]
fn a_written_id_of_one_document_is_not_taken_for_an_emitted_id_of_another() {
    // Semantics 7 maps the ids a program emitted, in the document that holds the program: the included document's
    // own `p.g1` keeps its seed when the including document's program `p` emits `g1`
    let k = Kit::new("seed-docs");
    k.document("B.scene.xml", &format!(r#"<shape id="p.g1" shape="rect" {RECT}/>"#));
    let include = r#"<include id="i" src="B.scene.xml"/>"#;
    let with_program = k.build("", &(k.program("p", "srep66-rect") + include)).unwrap_or_else(|c| panic!("{c:?}"));
    let without = k.build("", include).unwrap_or_else(|c| panic!("{c:?}"));
    let node = |e: &Evaluator| {
        let n = &e.program().nodes[index(e, "i/p.g1")];
        (n.seed, n.seed_id.to_string())
    };
    assert_eq!(node(&with_program), node(&without));
    assert_eq!(node(&without).1, "i/p.g1");
    // while the emitted one maps
    assert_eq!(&*with_program.program().nodes[index(&with_program, "p.g1")].seed_id, "g1");
}
