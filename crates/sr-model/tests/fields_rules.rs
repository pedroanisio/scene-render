//! The fields draft (schema/UPSTREAM): the field node with its sources and presentations, the Schematron rules FLD1
//! to FLD6, and its clause of V17, the one version gate of the 1.8 drafts.

fn codes(version: &str, composition: &str) -> Vec<String> {
    let xml = format!(
        r#"<scene version="{version}"><project width="64" height="64" fps="30" duration="1"/><parameters><data id="flow" src="flow.csv" format="csv"/></parameters><composition>{composition}</composition><physics><forceField id="wind" type="wind" forceX="1"/></physics></scene>"#
    );
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
    sr_model::validate_str(&xml, &opts).diagnostics.into_iter().map(|d| d.code).collect()
}

fn field(attrs: &str, children: &str) -> String {
    format!(r#"<field id="f" width="40" height="30" {attrs}>{children}</field>"#)
}

const ROTATION: &str = r#"u="-y" v="x""#;

/// Shot 2: a loop of two wires of opposite charge carrying the current round it, with Poynting arrows.
const SHOT2: &str = r#"<wire x1="-1" y1="0.5" x2="1" y2="0.5" q="1" i="1"/><wire x1="1" y1="-0.5" x2="-1" y2="-0.5" q="-1" i="1"/><arrows cols="9" rows="9" show="poynting"/>"#;

#[track_caller]
fn valid(composition: &str) {
    let got = codes("1.8", composition);
    assert!(got.is_empty(), "{composition}: {got:?}");
}

#[track_caller]
fn refused(composition: &str, code: &str) {
    let got = codes("1.8", composition);
    assert!(got.iter().any(|c| c == code), "{composition}: want {code}, got {got:?}");
}

#[test]
fn fields_of_every_source_and_presentation_are_valid_at_1_8() {
    valid(&field(ROTATION, ""));
    valid(&field(
        &format!(r##"{ROTATION} xMin="-2" xMax="2" yMin="-1" yMax="1" stroke="#FFFFFFFF" strokeWidth="1""##),
        r##"<arrows cols="12" rows="8" scale="0.5" maxLength="20" headLength="0.25" headWidth="0.4"/><streamlines separation="16" test="6" step="2" maxSteps="2000" maxLines="300" seedX="0.5" seedY="0" markerEnd="arrow"/><lic length="12" step="0.5" resolution="128" seed="7" opacity="0.8"/>"##,
    ));
    valid(&field(r#"kind="scalar" u="x*x - y*y""#, r#"<contours levels="-0.5 0 0.5" resolution="96" dash="4 2"/>"#));
    valid(&field(r#"kind="complex" function="sqrt""#, r#"<portrait modulus="false" base="3" opacity="0.5"/>"#));
    valid(&field(r#"kind="complex" w="x*x - y*y; 2*x*y""#, "<portrait/>"));
    valid(&field(r#"grid="flow""#, "<arrows/>"));
    valid(&field(r#"forceField="wind""#, "<streamlines/>"));
    valid(&field(
        "",
        r#"<charge x="-0.5" y="0" q="1"/><charge x="0.5" y="0" q="-1"/><lines perCharge="12" radius="0.02" step="0.005" maxSteps="4000"/><arrows/>"#,
    ));
    valid(&field("", r#"<current x="0" y="0" i="2"/><dipole x="0.3" y="0" mx="0" my="1"/><streamlines/>"#));
    valid(&field("", SHOT2));
    valid(&field("", r#"<wire x1="0" y1="0" x2="1" y2="0" q="2"/><arrows show="field"/>"#));
    // in an axes, the box is the axes' and the field needs no width or height
    valid(&format!(
        r#"<axes id="ax" width="40" height="30" xMin="-2" xMax="2"><field id="g" {ROTATION}><arrows/></field></axes>"#
    ));
    valid(&field(
        ROTATION,
        r#"<animate property="opacity"><key time="0" value="0"/><key time="1" value="1"/></animate>"#,
    ));
}

#[test]
fn a_field_has_exactly_one_source_fld1() {
    for bad in [
        field("", ""),
        field("", "<arrows/>"),
        field(r#"u="x" function="sqrt""#, ""),
        field(r#"function="exp" w="x; y""#, ""),
        field(r#"grid="flow" forceField="wind""#, ""),
        field(r#"u="x""#, r#"<charge x="0" y="0" q="1"/>"#),
        field(r#"w="x; y""#, r#"<dipole x="0" y="0" mx="1" my="0"/>"#),
        // v only with u, and only in a vector field
        field(r#"v="x""#, ""),
        field(r#"kind="scalar" u="x" v="y""#, ""),
        field(r#"kind="complex" function="exp" v="y""#, ""),
    ] {
        refused(&bad, "FLD1");
    }
    valid(&field(r#"kind="vector" u="x" v="y""#, ""));
    valid(&field("", r#"<charge x="0" y="0" q="1"/><current x="1" y="0" i="1"/>"#));
    // a wire is a source child like the others
    refused(&field(ROTATION, r#"<wire x1="0" y1="0" x2="1" y2="0" i="1"/>"#), "FLD1");
    refused(&field(r#"kind="scalar""#, r#"<wire x1="0" y1="0" x2="1" y2="0" q="1"/>"#), "FLD1");
    valid(&field("", r#"<wire x1="0" y1="0" x2="1" y2="0"/>"#));
}

#[test]
fn poynting_arrows_need_a_current_carrying_wire_and_a_charge_fld6() {
    let wire = |q: &str, i: &str| format!(r#"<wire x1="0" y1="0" x2="1" y2="0" q="{q}" i="{i}"/>"#);
    let poynting = r#"<arrows show="poynting"/>"#;
    for bad in [
        // no wire carries a current
        format!(r#"<charge x="0" y="1" q="1"/>{}{poynting}"#, wire("1", "0")),
        format!(r#"<charge x="0" y="1" q="1"/><current x="0" y="0" i="1"/>{poynting}"#),
        // the current flows but nothing is charged
        format!("{}{poynting}", wire("0", "1")),
        format!(r#"<current x="0" y="2" i="1"/>{}{poynting}"#, wire("0", "1")),
    ] {
        refused(&field("", &bad), "FLD6");
    }
    valid(&field("", &format!("{}{poynting}", wire("1", "1"))));
    valid(&field("", &format!(r#"<charge x="0" y="1" q="1"/>{}{poynting}"#, wire("0", "1"))));
    valid(&field("", &format!(r#"<dipole x="0" y="1" mx="1" my="0"/>{}{poynting}"#, wire("0", "-2"))));
    // arrows of the field itself need neither
    valid(&field(ROTATION, r#"<arrows show="field"/>"#));
}

#[test]
fn the_source_fits_the_kind_fld1() {
    for bad in [
        // a vector field takes u with v, grid, forceField or the source children
        field(r#"u="x""#, ""),
        field(r#"function="exp""#, ""),
        field(r#"w="x; y""#, ""),
        // a scalar field takes u alone or grid
        field(r#"kind="scalar" function="exp""#, ""),
        field(r#"kind="scalar" forceField="wind""#, ""),
        field(r#"kind="scalar""#, r#"<charge x="0" y="0" q="1"/>"#),
        // a complex field takes function or w
        field(r#"kind="complex" u="x""#, ""),
        field(r#"kind="complex" grid="flow""#, ""),
        field(r#"kind="complex""#, r#"<dipole x="0" y="0" mx="1" my="0"/>"#),
    ] {
        refused(&bad, "FLD1");
    }
    valid(&field(r#"kind="scalar" u="x""#, ""));
    valid(&field(r#"kind="scalar" grid="flow""#, ""));
    valid(&field(r#"grid="flow""#, ""));
    valid(&field(r#"forceField="wind""#, ""));
    valid(&field(r#"kind="complex" w="x; y""#, ""));
}

#[test]
fn coefficients_go_with_mobius_and_are_eight_fld1() {
    for bad in [
        field(r#"kind="complex" function="mobius""#, ""),
        field(r#"kind="complex" function="mobius" coefficients="1 0 0 0 0 0 1""#, ""),
        field(r#"kind="complex" function="mobius" coefficients="1 0 0 0 0 0 1 0 0""#, ""),
        field(r#"kind="complex" function="exp" coefficients="1 0 0 0 0 0 1 0""#, ""),
        field(r#"kind="complex" w="x; y" coefficients="1 0 0 0 0 0 1 0""#, ""),
    ] {
        refused(&bad, "FLD1");
    }
    valid(&field(r#"kind="complex" function="mobius" coefficients=" 1 0  0 0 0 0 1 0 ""#, "<portrait/>"));
}

#[test]
fn a_field_outside_an_axes_states_its_box_fld5() {
    for bad in [
        r#"<field id="g" u="-y" v="x"/>"#,
        r#"<field id="g" u="-y" v="x" width="40"/>"#,
        r#"<group id="k"><field id="g" u="-y" v="x" height="30"/></group>"#,
    ] {
        refused(bad, "FLD5");
    }
    valid(r#"<axes id="ax" width="40" height="30"><group id="k"><field id="g" u="-y" v="x"/></group></axes>"#);
}

#[test]
fn each_given_min_is_below_its_max_fld2() {
    for bad in [r#"xMin="1" xMax="1""#, r#"xMin="2" xMax="-2""#, r#"yMin="0.5" yMax="0""#] {
        refused(&field(&format!("{ROTATION} {bad}"), ""), "FLD2");
    }
    valid(&field(&format!(r#"{ROTATION} xMin="-3" xMax="-2" yMin="5" yMax="6""#), ""));
}

#[test]
fn a_presentation_fits_its_field_fld3() {
    refused(&field(ROTATION, "<portrait/>"), "FLD3");
    refused(&field(r#"kind="scalar" u="x""#, "<portrait/>"), "FLD3");
    refused(&field(ROTATION, r#"<contours levels="0"/>"#), "FLD3");
    refused(&field(r#"kind="complex" function="exp""#, r#"<contours levels="0"/>"#), "FLD3");
    refused(&field(ROTATION, "<lines/>"), "FLD3");
    refused(&field("", r#"<dipole x="0" y="0" mx="1" my="0"/><lines/>"#), "FLD3");
}

#[test]
fn streamline_bounds_hold_fld4() {
    refused(&field(ROTATION, r#"<streamlines separation="10" test="11"/>"#), "FLD4");
    // the default separation is 20 pixels
    refused(&field(ROTATION, r#"<streamlines test="21"/>"#), "FLD4");
    refused(&field(ROTATION, r#"<streamlines maxSteps="1000001"/>"#), "FLD4");
    refused(&field(ROTATION, r#"<streamlines maxLines="1000001"/>"#), "FLD4");
    refused(&field("", r#"<charge x="0" y="0" q="1"/><lines maxSteps="1000001"/>"#), "FLD4");
    valid(&field(ROTATION, r#"<streamlines test="20" maxSteps="1000000" maxLines="1000000"/>"#));
    valid(&field(ROTATION, r#"<streamlines separation="10" test="10"/>"#));
}

#[test]
fn the_schema_types_refuse_values_outside_their_ranges() {
    for bad in [
        field(&format!(r#"{ROTATION} kind="tensor""#), ""),
        field(r#"function="tan""#, ""),
        field(r#"grid="nowhere""#, ""),
        field(ROTATION, r#"<arrows cols="0"/>"#),
        field(ROTATION, r#"<arrows rows="1001"/>"#),
        field(ROTATION, r#"<arrows scale="0"/>"#),
        field(ROTATION, r#"<streamlines separation="-1"/>"#),
        field(ROTATION, r#"<lic length="0"/>"#),
        field(ROTATION, r#"<lic resolution="4097"/>"#),
        field(ROTATION, r#"<lic opacity="2"/>"#),
        field(ROTATION, r#"<lic seed="-1"/>"#),
        field(r#"kind="complex" function="exp""#, r#"<portrait base="0"/>"#),
        // a base above 1: log_base needs it
        field(r#"kind="complex" function="exp""#, r#"<portrait base="1"/>"#),
        field(r#"kind="complex" function="exp""#, r#"<portrait base="0.5"/>"#),
        field(r#"kind="scalar" u="x""#, r#"<contours levels="0" resolution="0"/>"#),
        field("", r#"<charge x="0" y="0"/>"#),
        field("", r#"<current x="0" i="1"/>"#),
        field("", r#"<dipole x="0" y="0" mx="1"/>"#),
        field("", r#"<wire x1="0" y1="0" x2="1" i="1"/>"#),
        field("", r#"<wire x1="0" y1="0" x2="1" y2="0" q="a"/>"#),
        field(ROTATION, r#"<arrows show="flux"/>"#),
        field("", r#"<charge x="0" y="0" q="1"/><lines perCharge="0"/>"#),
        // sources come before presentations
        field("", r#"<arrows/><charge x="0" y="0" q="1"/>"#),
        field(ROTATION, r#"<arrows strokeWidth="0"/>"#),
    ] {
        let got = codes("1.8", &bad);
        assert!(got.iter().any(|c| c.starts_with('S')), "{bad}: {got:?}");
    }
}

#[test]
fn before_1_8_fields_are_refused_v17() {
    let uses = [
        field(ROTATION, "<arrows/>"),
        format!(r#"<group id="g">{}</group>"#, field("", r#"<charge x="0" y="0" q="1"/>"#)),
    ];
    for v in ["1.0", "1.1", "1.2", "1.3", "1.4", "1.5", "1.6", "1.7"] {
        for c in &uses {
            let got = codes(v, c);
            assert!(got.iter().any(|c| c == "V17"), "{v} {c}: {got:?}");
        }
    }
    for c in &uses {
        valid(c);
    }
}
