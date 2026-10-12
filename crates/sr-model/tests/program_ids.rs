//! SREP 87: the ids a build-mode program emits are scoped by the program. An id `E` that program `P` emits is
//! `P.E` in the document; references inside the output resolve to the output's own ids first; a bare reference from
//! elsewhere resolves when exactly one program emitted the id (W13) and fails when several did (PRG17).
//!
//! The modules are `srep66-rect.wasm` of the sr-core kit (it emits a rect `g1` at x 200) and constant modules
//! written here from WebAssembly text, as sr-core's `wasm_min.py` writes the kit's.

use std::path::PathBuf;

use sr_model::element::{children, AttrValue, Element};
use sr_model::model::Node;
use sr_model::{load_str, Document, LoadOptions};

/// A directory of modules for one test.
struct Kit {
    dir: PathBuf,
}

impl Kit {
    fn new(tag: &str) -> Kit {
        // every test runs without the on-disk cache (tests/program_cache.rs tests it)
        std::env::set_var("SR_PROGRAM_CACHE", "off");
        let dir = std::env::temp_dir().join(format!("sr-srep87-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let rect = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sr-wasm/tests/modules/srep66-rect.wasm");
        std::fs::copy(rect, dir.join("srep66-rect.wasm")).unwrap();
        Kit { dir }
    }

    /// Writes a module whose output is `fragment`.
    fn constant(&self, name: &str, fragment: &str) {
        let bytes: String = fragment.bytes().map(|b| format!("\\{b:02x}")).collect();
        let wasm = wat::parse_str(format!(
            r#"(module (memory (export "memory") 1) (data (i32.const 0) "{bytes}") (func (export "generate") (result i64) i64.const {}))"#,
            fragment.len()
        ))
        .unwrap();
        std::fs::write(self.dir.join(format!("{name}.wasm")), wasm).unwrap();
    }

    fn sha(&self, name: &str) -> String {
        use sha2::Digest;
        let b = std::fs::read(self.dir.join(format!("{name}.wasm"))).unwrap();
        sha2::Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
    }

    /// A program `id` running module `name`.
    fn program(&self, id: &str, name: &str, attrs: &str) -> String {
        format!(r#"<program id="{id}" src="{name}.wasm" sha256="{}"{attrs}/>"#, self.sha(name))
    }

    fn load(&self, before: &str, body: &str) -> Result<Document, Vec<(String, String)>> {
        let xml = format!(
            r#"<scene version="1.6"><project width="640" height="360" fps="24" duration="1" seed="1"/>{before}<composition>{body}</composition></scene>"#
        );
        load_str(&xml, &LoadOptions { verify_assets: true, base_dir: Some(self.dir.clone()) }).map_err(|e| {
            e.report()
                .map(|r| r.diagnostics.iter().map(|d| (d.code.clone(), d.message.clone())).collect())
                .unwrap_or_default()
        })
    }
}

impl Drop for Kit {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn codes(r: &Result<Document, Vec<(String, String)>>) -> Vec<String> {
    match r {
        Ok(_) => Vec::new(),
        Err(d) => d.iter().map(|(c, _)| c.clone()).collect(),
    }
}

fn str_attr(d: &Document, id: &str, name: &str) -> Option<String> {
    let n = d.node(id).unwrap_or_else(|| panic!("no node {id}"));
    match (n as &dyn Element).get_attr(name) {
        Some(AttrValue::Str(s)) => Some(s),
        other => other.map(|v| format!("{v:?}")),
    }
}

fn expression(d: &Document, id: &str) -> String {
    let n = d.node(id).unwrap_or_else(|| panic!("no node {id}"));
    let e = children(n as &dyn Element).into_iter().find(|c| c.element_name() == "expression").expect("expression");
    e.as_any().downcast_ref::<sr_model::model::Expression>().expect("expression").value.clone()
}

fn w13(d: &Document) -> Vec<String> {
    d.warnings().iter().filter(|w| w.code == "W13").map(|w| w.message.clone()).collect()
}

const RECT: &str = r##"width="10" height="10" fill="#FF0000""##;

#[test]
fn one_module_run_by_two_programs_loads_as_two_namespaced_ids() {
    // Semantics 2; kit srep-NNNN-program-ids-reuse
    let k = Kit::new("reuse");
    let d = k
        .load("", &(k.program("a", "srep66-rect", r#" x="0""#) + &k.program("b", "srep66-rect", r#" x="300""#)))
        .expect("two programs that emit the same id load");
    assert!(matches!(d.node("a.g1"), Some(Node::Shape(_))));
    assert!(matches!(d.node("b.g1"), Some(Node::Shape(_))));
    assert!(d.node("g1").is_none(), "the local id is not a document id");
    assert!(w13(&d).is_empty());
}

#[test]
fn the_program_keeps_its_id_as_the_group_that_holds_the_namespaced_ids() {
    // Semantics 5: the group P is the parent of P.E
    let k = Kit::new("group");
    let d = k.load("", &k.program("p", "srep66-rect", "")).unwrap();
    let g = d.node("p").expect("p");
    assert!(matches!(g, Node::Group(_)), "p is a group");
    let kids: Vec<Option<&str>> = g.child_nodes().map(|n| n.id()).collect();
    assert_eq!(kids, [Some("p.g1")]);
}

#[test]
fn a_reference_inside_an_output_names_the_outputs_own_element() {
    // Semantics 3.1; kit srep-NNNN-program-ids-local-ref
    let k = Kit::new("local-ref");
    k.constant(
        "matted",
        &format!(r#"<shape id="m" shape="rect" {RECT}/><shape id="s" shape="rect" {RECT} matte="m"/>"#),
    );
    let d = k.load("", &(k.program("a", "matted", "") + &k.program("b", "matted", ""))).unwrap();
    assert_eq!(str_attr(&d, "a.s", "matte").as_deref(), Some("a.m"));
    assert_eq!(str_attr(&d, "b.s", "matte").as_deref(), Some("b.m"));
    assert!(w13(&d).is_empty());
}

#[test]
fn the_outputs_own_id_wins_over_a_document_id() {
    // Semantics 3.1; kit srep-NNNN-program-ids-local-wins
    let k = Kit::new("local-wins");
    k.constant(
        "matted",
        &format!(r#"<shape id="m" shape="rect" {RECT}/><shape id="s" shape="rect" {RECT} matte="m"/>"#),
    );
    let d =
        k.load("", &(format!(r#"<shape id="m" shape="ellipse" {RECT}/>"#) + &k.program("p", "matted", ""))).unwrap();
    assert_eq!(str_attr(&d, "p.s", "matte").as_deref(), Some("p.m"));
    assert!(matches!(d.node("m"), Some(Node::Shape(_))), "the document's m is still there");
    assert!(w13(&d).is_empty());
}

#[test]
fn a_prop_at_time_inside_an_output_names_the_outputs_own_element() {
    // Semantics 3.2
    let k = Kit::new("local-prop");
    k.constant(
        "follow",
        &format!(
            r#"<shape id="m" shape="rect" {RECT} x="40"/><shape id="s" shape="rect" {RECT}><expression property="x">propAtTime("m.x", 0) + prop('m.y')</expression></shape>"#
        ),
    );
    let d = k.load("", &k.program("p", "follow", "")).unwrap();
    assert_eq!(expression(&d, "p.s"), r#"propAtTime("p.m.x", 0) + prop('p.m.y')"#);
    assert!(w13(&d).is_empty());
}

#[test]
fn a_qualified_prop_from_outside_resolves_without_a_warning() {
    // Semantics 4 (a full id is a document id); kit srep-NNNN-program-ids-qualified-prop
    let k = Kit::new("qualified");
    let f =
        format!(r#"<shape id="f" shape="rect" {RECT}><expression property="x">prop("a.g1.x")</expression></shape>"#);
    let d = k.load("", &(k.program("a", "srep66-rect", "") + &f)).unwrap();
    assert_eq!(expression(&d, "f"), r#"prop("a.g1.x")"#);
    assert!(w13(&d).is_empty(), "{:?}", w13(&d));
}

#[test]
fn a_bare_prop_from_outside_resolves_to_the_one_program_that_emitted_it_with_w13() {
    // Semantics 4; kit srep-NNNN-program-ids-bare-prop
    let k = Kit::new("bare-prop");
    let f =
        format!(r#"<shape id="f" shape="rect" {RECT}><expression property="x">prop("g1.x") + 10</expression></shape>"#);
    let d = k.load("", &(k.program("a", "srep66-rect", "") + &f)).unwrap();
    assert_eq!(expression(&d, "f"), r#"prop("a.g1.x") + 10"#);
    let w = w13(&d);
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].contains("\"g1\"") && w[0].contains("\"a.g1\""), "{w:?}");
}

#[test]
fn a_bare_layout_override_target_resolves_with_w13() {
    // Semantics 4; kit srep-NNNN-program-ids-bare-override
    let k = Kit::new("bare-override");
    let layouts = r##"<layouts><layout id="tall" width="360" height="640"><override target="g1" property="fill" value="#00FF00"/></layout></layouts>"##;
    let d = k.load(layouts, &k.program("a", "srep66-rect", "")).unwrap();
    let Some(l) = d.scene.layouts.as_ref().and_then(|l| l.layouts.first()) else { panic!() };
    assert_eq!(l.overrides.first().map(|o| o.target.as_str()), Some("a.g1"));
    assert_eq!(w13(&d).len(), 1, "{:?}", w13(&d));
}

#[test]
fn a_bare_idref_from_outside_resolves_with_w13() {
    // Semantics 4: an xs:IDREF outside every output; validation without the programs cannot resolve it, loading can
    let k = Kit::new("bare-idref");
    let d = k
        .load("", &(k.program("a", "srep66-rect", "") + &format!(r#"<shape id="f" shape="rect" {RECT} matte="g1"/>"#)));
    let d = d.expect("loads");
    assert_eq!(str_attr(&d, "f", "matte").as_deref(), Some("a.g1"));
    assert_eq!(w13(&d).len(), 1, "{:?}", w13(&d));
    // written in full it needs no warning
    let d = k
        .load(
            "",
            &(k.program("a", "srep66-rect", "") + &format!(r#"<shape id="f" shape="rect" {RECT} matte="a.g1"/>"#)),
        )
        .expect("loads");
    assert!(w13(&d).is_empty());
}

#[test]
fn an_idref_that_no_program_emits_is_still_s10() {
    // Semantics 4: with M empty, the reference's own error applies
    let k = Kit::new("s10");
    let r = k
        .load("", &(k.program("a", "srep66-rect", "") + &format!(r#"<shape id="f" shape="rect" {RECT} matte="g9"/>"#)));
    let mut c = codes(&r);
    c.sort();
    assert_eq!(c, ["R8", "S10"], "the reference names nothing (S10), so no node (R8)");
}

#[test]
fn a_bare_reference_that_two_programs_emitted_fails_prg17() {
    // Semantics 4; kit srep-NNNN-program-ids-ambiguous
    let k = Kit::new("ambiguous");
    let two = k.program("a", "srep66-rect", "") + &k.program("b", "srep66-rect", "");
    let f = format!(r#"<shape id="f" shape="rect" {RECT}><expression property="x">prop("g1.x")</expression></shape>"#);
    let r = k.load("", &(two.clone() + &f));
    assert_eq!(codes(&r), ["PRG17"]);
    let Err(d) = r else { unreachable!() };
    assert!(d[0].1.contains("\"a\"") && d[0].1.contains("\"b\""), "{d:?}");
    // the same for an xs:IDREF and a layout override
    assert_eq!(
        codes(&k.load("", &(two.clone() + &format!(r#"<shape id="f" shape="rect" {RECT} matte="g1"/>"#)))),
        ["PRG17"]
    );
    let layouts = r##"<layouts><layout id="tall" width="360" height="640"><override target="g1" property="fill" value="#00FF00"/></layout></layouts>"##;
    assert_eq!(codes(&k.load(layouts, &two)), ["PRG17"]);
}

#[test]
fn a_namespaced_id_equal_to_another_fails_prg14() {
    // Semantics 2; kit srep-NNNN-program-ids-composed-collision and the amended srep-0066-program-id-collision
    let k = Kit::new("collision");
    k.constant("bc", &format!(r#"<shape id="b.c" shape="rect" {RECT}/>"#));
    k.constant("c", &format!(r#"<shape id="c" shape="rect" {RECT}/>"#));
    let r = k.load("", &(k.program("a", "bc", "") + &k.program("a.b", "c", "")));
    assert_eq!(codes(&r), ["PRG14"], "a emitting b.c beside a.b emitting c both make a.b.c");
    let r = k.load("", &(k.program("p", "srep66-rect", "") + &format!(r#"<shape id="p.g1" shape="rect" {RECT}/>"#)));
    assert_eq!(codes(&r), ["PRG14"], "a written id equal to a namespaced one");
    // the local value alone does not collide
    let d = k.load("", &(k.program("p", "srep66-rect", "") + &format!(r#"<shape id="g1" shape="rect" {RECT}/>"#)));
    assert!(d.is_ok(), "{:?}", codes(&d));
}

#[test]
fn two_equal_ids_in_one_output_fail_prg14() {
    // Semantics 1; kit srep-NNNN-program-ids-duplicate-local
    let k = Kit::new("duplicate");
    k.constant(
        "twice",
        &format!(r#"<shape id="g1" shape="rect" {RECT}/><group id="h"><shape id="g1" shape="rect" {RECT}/></group>"#),
    );
    assert_eq!(codes(&k.load("", &k.program("p", "twice", ""))), ["PRG14"]);
}

#[test]
fn the_output_digest_is_over_the_bytes_the_module_returned() {
    // Semantics 6; kit srep-NNNN-program-ids-output-sha256
    use sha2::Digest;
    let k = Kit::new("digest");
    let out = sr_wasm::generate(
        &std::fs::read(k.dir.join("srep66-rect.wasm")).unwrap(),
        &sr_wasm::Inputs { project_seed: 1, ..Default::default() },
        sr_wasm::Limits::default(),
    )
    .unwrap();
    let digest: String = sha2::Sha256::digest(&out).iter().map(|x| format!("{x:02x}")).collect();
    let pin = format!(r#" outputSha256="{digest}""#);
    let d = k.load("", &(k.program("a", "srep66-rect", &pin) + &k.program("b", "srep66-rect", &pin)));
    assert!(d.is_ok(), "{:?}", codes(&d));
}

#[test]
fn a_reference_in_one_output_to_an_id_another_program_emitted_resolves_with_w13() {
    // Semantics 4, third kind (the case the draft leaves without a kit case: it needs two cooperating modules)
    let k = Kit::new("cross");
    k.constant("user", &format!(r#"<shape id="s" shape="rect" {RECT} matte="g1"/>"#));
    let d = k.load("", &(k.program("a", "srep66-rect", "") + &k.program("u", "user", ""))).unwrap();
    assert_eq!(str_attr(&d, "u.s", "matte").as_deref(), Some("a.g1"));
    let w = w13(&d);
    assert!(w.len() == 1 && w[0].contains("program \"u\""), "{w:?}");
}

#[test]
fn a_link_source_names_its_node_as_prop_does() {
    // beyond the SREP's list of positions: link/@source "nodeId.property" resolves like prop("nodeId.property")
    let k = Kit::new("link");
    let f = format!(r#"<shape id="f" shape="rect" {RECT}><link property="x" source="g1.x"/></shape>"#);
    let d = k.load("", &(k.program("a", "srep66-rect", "") + &f)).unwrap();
    let n = d.node("f").unwrap();
    let l = children(n as &dyn Element).into_iter().find(|c| c.element_name() == "link").unwrap();
    assert_eq!(l.get_attr("source"), Some(AttrValue::Str("a.g1.x".into())));
    assert_eq!(w13(&d).len(), 1);
}

#[test]
fn a_document_without_node_programs_is_read_as_before() {
    // Semantics: nothing changes without a program in the composition
    let k = Kit::new("none");
    let d = k
        .load("", &format!(r#"<shape id="g1" shape="rect" {RECT}/><shape id="f" shape="rect" {RECT} matte="g1"/>"#))
        .unwrap();
    assert_eq!(str_attr(&d, "f", "matte").as_deref(), Some("g1"));
    assert!(d.warnings().is_empty());
    let mut c = codes(&k.load("", &format!(r#"<shape id="f" shape="rect" {RECT} matte="g9"/>"#)));
    c.sort();
    assert_eq!(c, ["R8", "S10"]);
}

#[test]
fn w13_and_prg17_are_catalogued() {
    for c in ["W13", "PRG17"] {
        let code = sr_model::codes::lookup(c).unwrap_or_else(|| panic!("{c} is not in the code table"));
        assert!(code.summary.contains("SREP 87"), "{c}: {}", code.summary);
    }
    assert!(sr_model::codes::lookup("PRG14").unwrap().summary.contains("87)"));
}

#[test]
fn another_error_of_an_element_holding_a_bare_idref_is_reported_as_itself() {
    // the holder's own error waits with its S10 until the programs run; once the reference resolves, the error is
    // the element's (S06 at the shape), not the output's (PRG14 at a program)
    let k = Kit::new("holder-error");
    let r = k.load(
        "",
        &(k.program("p", "srep66-rect", "")
            + &format!(r#"<shape id="s" shape="rect" {RECT} opacity="7" matte="g1"/>"#)),
    );
    let Err(d) = r else { panic!("opacity 7 is invalid") };
    let errors: Vec<&String> = d.iter().map(|(c, _)| c).filter(|c| !c.starts_with('W')).collect();
    assert_eq!(errors, ["S06"], "{d:?}");
}

#[test]
fn a_parenthesised_prop_literal_from_outside_resolves_with_w13() {
    // the expression language reads `prop(("g1.x"))` and `(prop)("g1.x")` as `prop("g1.x")`
    let k = Kit::new("paren-prop");
    for (written, scoped) in [
        (r#"prop(("g1.x")) + 1"#, r#"prop(("a.g1.x")) + 1"#),
        (r#"(prop)("g1.x") + 1"#, r#"(prop)("a.g1.x") + 1"#),
        (r#"propAtTime((("g1.x")), time) + 1"#, r#"propAtTime((("a.g1.x")), time) + 1"#),
    ] {
        let f = format!(r#"<shape id="f" shape="rect" {RECT}><expression property="x">{written}</expression></shape>"#);
        let d = k.load("", &(k.program("a", "srep66-rect", "") + &f)).unwrap_or_else(|e| panic!("{written}: {e:?}"));
        assert_eq!(expression(&d, "f"), scoped);
        assert_eq!(w13(&d).len(), 1, "{written}");
    }
}

#[test]
fn build_programs_beside_a_step_and_a_data_program_are_scoped_alone() {
    // a step program (SREP 69) emits pixels and a data program JSON: neither emits ids, so neither is scoped nor
    // counted among the emitters of rule 4
    let k = Kit::new("mixed");
    let modules = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sr-wasm/tests/modules");
    for m in ["srep66-rows", "srep69-counter"] {
        std::fs::copy(modules.join(format!("{m}.wasm")), k.dir.join(format!("{m}.wasm"))).unwrap();
    }
    let parameters = format!("<parameters>{}</parameters>", k.program("rows", "srep66-rows", ""));
    let step = k
        .program("s", "srep69-counter", r#" mode="step" width="10" height="10""#)
        .replace("/>", r#"><param name="target" value="3"/></program>"#);
    let body = k.program("a", "srep66-rect", "")
        + &step
        + &k.program("b", "srep66-rect", "")
        + r#"<repeat id="r" over="rows"><shape id="c" shape="rect" width="4" height="4"/></repeat>"#
        + &format!(r#"<shape id="f" shape="rect" {RECT}><expression property="x">prop("b.g1.x")</expression></shape>"#);
    let d = k.load(&parameters, &body).unwrap_or_else(|e| panic!("{e:?}"));
    assert!(matches!(d.node("a.g1"), Some(Node::Shape(_))));
    assert!(matches!(d.node("b.g1"), Some(Node::Shape(_))));
    assert!(d.node("s").is_some(), "the step program keeps its place and id");
    assert!(d.node("s.g1").is_none() && d.node("rows.g1").is_none());
    assert!(d.data_source("rows").is_some(), "the data program is a data source");
    assert_eq!(expression(&d, "f"), r#"prop("b.g1.x")"#);
    assert!(w13(&d).is_empty(), "{:?}", w13(&d));
}
