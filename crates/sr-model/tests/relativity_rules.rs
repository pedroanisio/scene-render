//! The relativity draft (schema/UPSTREAM): the `spacetime` child of `axes` with its `event`, `worldline` and
//! `lightRay` children, `primitive="embedding"` with its `embedding` child, the modifier type `strain`, the
//! Schematron rules RD1 to RD4 and their clause of V17.

/// A document at `version` with a diagram (`spacetime` attributes `st`, children `kids`), an embedding (attributes
/// `emb` of a Flamm sheet of rs 2 to 10) and a strained ring (attributes `strain` of a modifier with hPlus 0.2).
fn document(version: &str, st: &str, kids: &str, emb: &str, strain: &str) -> String {
    format!(
        r#"<scene version="{version}"><project width="640" height="360" fps="24" duration="10"/><composition><axes id="ax" width="400" height="300" x="100" y="50" xMin="-2" xMax="2" yMin="-1.5" yMax="1.5"><spacetime {st}>{kids}</spacetime></axes><object3D id="sheet" primitive="embedding"><embedding kind="flamm" rs="2" rMax="10" {emb}/></object3D><shape id="ring" shape="ellipse" width="100" height="100"><deform><modifier type="strain" hPlus="0.2" frequency="1" {strain}/></deform></shape></composition></scene>"#
    )
}

/// `xml` with the black hole of mass 1 (r_s 2) that the diagrams and sheets name.
fn with_hole(xml: &str) -> String {
    xml.replacen("<composition>", r#"<composition><blackHole id="hole" mass="1"/>"#, 1)
}

fn codes_of(xml: &str) -> Vec<String> {
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
    let mut got: Vec<String> = sr_model::validate_str(xml, &opts).diagnostics.into_iter().map(|d| d.code).collect();
    got.sort();
    got.dedup();
    got
}

fn codes(st: &str, kids: &str, emb: &str, strain: &str) -> Vec<String> {
    codes_of(&document("1.8", st, kids, emb, strain))
}

#[track_caller]
fn valid(st: &str, kids: &str, emb: &str, strain: &str) {
    let got = codes(st, kids, emb, strain);
    assert!(got.is_empty(), "{st} {kids} {emb} {strain}: {got:?}");
}

#[track_caller]
fn refused(st: &str, kids: &str, emb: &str, strain: &str, code: &str) {
    assert_eq!(codes(st, kids, emb, strain), vec![code.to_string()], "{st} {kids} {emb} {strain}");
}

const EVENTS: &str = r##"<event x="0" t="0" label="O" lightCone="true" extent="1" future="#FFCC0040" past="#4080FF40" stroke="#FFFFFF" strokeWidth="2" markerSize="8"/><event x="3" t="5"/>"##;
const LINES: &str = r##"<worldline kind="inertial" velocity="0.5" label="ship"/><worldline kind="accelerated" acceleration="2" x0="0.1" t0="-0.2" tau0="-1" tau1="1" samples="64"/><worldline kind="parametric" x="0.5 * Math.sin(tau)" t="tau" tau0="0" tau1="3" dash="4 2"/><lightRay x="0" t="0" direction="-1" length="2" stroke="#FFCC00"/>"##;

#[test]
fn diagrams_embeddings_and_strain_are_valid_at_1_8() {
    valid("", "", "", "");
    valid(r#"c="2" boost="0.6" grid="true""#, &format!("{EVENTS}{LINES}"), "", "");
    valid(r#"boost="-0.999" compact="minkowski""#, EVENTS, "", "");
    let schwarzschild = with_hole(&document("1.8", r#"compact="schwarzschild" blackHole="hole""#, "", "", ""));
    assert!(codes_of(&schwarzschild).is_empty(), "{:?}", codes_of(&schwarzschild));
    // a keyed boost: the literal is checked, the keys are the build's
    valid("", r#"<animate property="boost"><key time="0" value="0"/><key time="10" value="0.8"/></animate>"#, "", "");
    valid("", "", r##"rMin="3" rings="4" spokes="3" samples="2" lineWidth="0.05" color="#FFCC00" surface="true""##, "");
    valid("", "", r#"rMin="2""#, "");
    valid("", "", "", r#"hCross="-1" phase="1.5" centerX="50" centerY="50""#);
}

#[test]
fn the_embedding_s_other_forms_are_valid() {
    let xml =
        |emb: &str| document("1.8", "", "", "", "").replace(r#"<embedding kind="flamm" rs="2" rMax="10" />"#, emb);
    for emb in [
        r#"<embedding kind="flamm" blackHole="hole" rMax="10"/>"#,
        r#"<embedding kind="potential" k="4" rMin="0.5" rMax="10"/>"#,
        r#"<embedding kind="potential" k="4" rs="2" rMax="10"/>"#,
        r#"<embedding kind="potential" k="4" blackHole="hole" rMax="10"/>"#,
        // a flamm sheet starts at its throat rs: a sheet of rs 2 reaches out to 3
        r#"<embedding kind="flamm" rs="2" rMax="3"/>"#,
    ] {
        let got = codes_of(&with_hole(&xml(emb)));
        let want: &[&str] = if emb.contains("hole") { &[] } else { &["W11"] };
        assert_eq!(got, want, "{emb}");
    }
    // a chirping strain
    let chirp = document("1.8", "", "", "", "").replace(r#"frequency="1" "#, r#"chirpMass="28" merger="1" "#);
    assert!(codes_of(&chirp).is_empty(), "{:?}", codes_of(&chirp));
}

#[test]
fn rd1_the_diagram() {
    for st in [r#"boost="1""#, r#"boost="-1""#, r#"boost="1.5""#, r#"compact="schwarzschild""#] {
        refused(st, "", "", "", "RD1");
    }
    // at most one diagram in an axes
    let two = document("1.8", "", "", "", "").replace("<spacetime ></spacetime>", "<spacetime/><spacetime/>");
    assert_eq!(codes_of(&two), vec!["RD1"]);
    // c is a positive decimal: the structure refuses it too
    assert_eq!(codes(r#"c="0""#, "", "", ""), vec!["RD1", "S06"]);
}

#[test]
fn rd2_worldlines() {
    for line in [
        r#"<worldline kind="inertial"/>"#,
        r#"<worldline kind="inertial" velocity="1"/>"#,
        r#"<worldline kind="inertial" velocity="-1.2"/>"#,
        r#"<worldline kind="accelerated"/>"#,
        r#"<worldline kind="parametric" x="tau"/>"#,
        r#"<worldline kind="parametric" t="tau"/>"#,
        // attributes of another kind
        r#"<worldline kind="inertial" velocity="0.5" acceleration="2"/>"#,
        r#"<worldline kind="inertial" velocity="0.5" tau0="1"/>"#,
        r#"<worldline kind="accelerated" acceleration="2" velocity="0.5"/>"#,
        r#"<worldline kind="accelerated" acceleration="2" x="tau"/>"#,
        r#"<worldline kind="parametric" x="tau" t="tau" velocity="0.5"/>"#,
        r#"<worldline kind="parametric" x="tau" t="tau" acceleration="1"/>"#,
    ] {
        refused("", line, "", "", "RD2");
    }
    // a zero acceleration is the type's (S06) and RD2's: a Rindler hyperbola needs a positive one
    let zero = codes("", r#"<worldline kind="accelerated" acceleration="0"/>"#, "", "");
    assert_eq!(zero, vec!["RD2", "S06"]);
}

#[test]
fn rd3_embeddings() {
    // rMin above rMax, inside the horizon, at rMax, and too few spokes
    for emb in [r#"rMin="12""#, r#"rMin="1""#, r#"rMin="10""#, r#"spokes="2""#] {
        refused("", "", emb, "", "RD3");
    }
    let xml =
        |emb: &str| document("1.8", "", "", "", "").replace(r#"<embedding kind="flamm" rs="2" rMax="10" />"#, emb);
    for emb in [
        // flamm without rs or blackHole, potential without k, and a potential sheet with no rMin to default from
        r#"<embedding kind="flamm" rMax="10"/>"#,
        r#"<embedding kind="potential" rMin="1" rMax="10"/>"#,
        r#"<embedding kind="potential" k="4" rMax="10"/>"#,
        // the horizon of the black hole the sheet names is at 2, above rMax
        r#"<embedding kind="flamm" blackHole="hole" rMax="1.5"/>"#,
        // a blackHole that names no black hole
        r#"<embedding kind="flamm" blackHole="ring" rMax="10"/>"#,
        // two embeddings
        r#"<embedding kind="flamm" rs="2" rMax="10"/><embedding kind="flamm" rs="2" rMax="10"/>"#,
        "",
    ] {
        let xml = if emb.contains(r#"blackHole="hole""#) { with_hole(&xml(emb)) } else { xml(emb) };
        assert_eq!(codes_of(&xml), vec!["RD3"], "{emb}");
    }
    // a zero rMin is the type's (S06) and RD3's
    let zero = xml(r#"<embedding kind="potential" k="4" rMin="0" rMax="10"/>"#);
    assert_eq!(codes_of(&zero), vec!["RD3", "S06"]);
    // an embedding on another primitive
    let other = document("1.8", "", "", "", "").replace(r#"primitive="embedding""#, r#"primitive="sphere""#);
    assert_eq!(codes_of(&other), vec!["RD3"]);
}

#[test]
fn rd4_strain() {
    for strain in [r#"hCross="1.5""#, r#"hCross="-2""#, r#"chirpMass="28" merger="1""#] {
        refused("", "", "", strain, "RD4");
    }
    let xml = |m: &str| document("1.8", "", "", "", "").replace(r#"hPlus="0.2" frequency="1" "#, m);
    for m in [
        r#"hPlus="2" frequency="1" "#,
        r#"hPlus="0.2" "#,
        r#"hPlus="0.2" chirpMass="28" "#,
        r#"hPlus="0.2" frequency="0" "#,
        r#"hPlus="0.2" frequency="-1" "#,
    ] {
        assert_eq!(codes_of(&xml(m)), vec!["RD4"], "{m}");
    }
    // the strain attributes on another modifier type are the other type's business
    let wave = document("1.8", "", "", "", "")
        .replace(r#"type="strain" hPlus="0.2" frequency="1" "#, r#"type="wave" amount="3" "#);
    assert!(codes_of(&wave).is_empty(), "{:?}", codes_of(&wave));
}

#[test]
fn w11_keeps_quiet_about_a_black_hole_a_diagram_or_a_sheet_names() {
    // a black hole nothing names and no camera renders is still reported
    assert_eq!(codes_of(&with_hole(&document("1.8", "", "", "", ""))), vec!["W11"]);
    let named = with_hole(&document("1.8", r#"blackHole="hole""#, "", "", ""));
    assert!(codes_of(&named).is_empty(), "{:?}", codes_of(&named));
    let named = with_hole(&document("1.8", "", "", "", "").replace(r#"rs="2" rMax"#, r#"blackHole="hole" rMax"#));
    assert!(codes_of(&named).is_empty(), "{:?}", codes_of(&named));
    // a diagram that names something else lends nothing
    let elsewhere = with_hole(&document("1.8", r#"blackHole="ring""#, "", "", ""));
    assert_eq!(codes_of(&elsewhere), vec!["W11"]);
    // its disk is still reported
    let disk = named
        .replace("<axes", r#"<accretionDisk id="disk" blackHole="hole" outerRadius="20" temperatureScale="1"/><axes"#);
    assert_eq!(codes_of(&disk), vec!["W11"]);
}

#[test]
fn v17_gates_the_syntax_before_1_8() {
    // every piece alone at 1.7
    let at_1_7 = |xml: String| codes_of(&xml.replace(r#"version="1.8""#, r#"version="1.7""#));
    let base = document("1.8", "", "", "", "");
    let without_diagram = base.replace(
        r#"<axes id="ax" width="400" height="300" x="100" y="50" xMin="-2" xMax="2" yMin="-1.5" yMax="1.5"><spacetime ></spacetime></axes>"#,
        "",
    );
    let without_sheet = |x: &str| {
        x.replace(
            r#"<object3D id="sheet" primitive="embedding"><embedding kind="flamm" rs="2" rMax="10" /></object3D>"#,
            "",
        )
    };
    let without_ring = |x: &str| {
        x.replace(r#"<shape id="ring" shape="ellipse" width="100" height="100"><deform><modifier type="strain" hPlus="0.2" frequency="1" /></deform></shape>"#, "")
    };
    // the strain alone (a group's deform is 1.8 syntax too, so the ring's modifier sits on a shape)
    assert_eq!(at_1_7(without_sheet(&without_diagram)), vec!["V17"]);
    assert_eq!(at_1_7(without_ring(&without_diagram)), vec!["V17"]);
    // the diagram alone: axes are 1.8 already, so the version gate fires either way
    assert_eq!(at_1_7(without_ring(&without_sheet(&base))), vec!["V17"]);
    // the strain's attributes on another modifier type are 1.8 syntax too
    let wave = without_sheet(&without_diagram)
        .replace(r#"type="strain" hPlus="0.2" frequency="1" "#, r#"type="wave" amount="3" hCross="0.1" "#);
    assert_eq!(at_1_7(wave), vec!["V17"]);
    // and the version 1.8 document of all three is valid
    assert!(codes_of(&base).is_empty());
}

#[test]
fn the_structure_refuses_unknown_kinds_and_out_of_range_counts() {
    for (st, kids, emb, strain) in [
        (r#"compact="kerr""#, "", "", ""),
        ("", r#"<worldline kind="geodesic"/>"#, "", ""),
        ("", r#"<lightRay x="0" t="0" direction="2"/>"#, "", ""),
        ("", r#"<lightRay x="0"/>"#, "", ""),
        ("", r#"<event x="0"/>"#, "", ""),
        ("", r#"<worldline kind="parametric" x="tau" t="tau" samples="1"/>"#, "", ""),
        ("", "", r#"samples="4097""#, ""),
        ("", "", r#"rings="0""#, ""),
        ("", "", "", r#"chirpMass="-1" merger="1""#),
    ] {
        let got = codes(st, kids, emb, strain);
        assert!(
            ["S01", "S05", "S06"].iter().any(|c| got.contains(&c.to_string())),
            "{st} {kids} {emb} {strain}: {got:?}"
        );
    }
}
