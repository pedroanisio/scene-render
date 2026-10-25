//! The plots draft (schema/UPSTREAM): the axes node, the plot element, primitive="axes" with axes3D and plot3D, the
//! Schematron rules PLT1 to PLT3 and the version gate V17.

fn codes(version: &str, composition: &str) -> Vec<String> {
    let xml = format!(
        r#"<scene version="{version}"><project width="64" height="64" fps="30" duration="1"/><composition>{composition}</composition></scene>"#
    );
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
    sr_model::validate_str(&xml, &opts).diagnostics.into_iter().map(|d| d.code).collect()
}

fn axes(attrs: &str, children: &str) -> String {
    format!(r#"<axes id="ax" width="40" height="30" {attrs}>{children}</axes>"#)
}

const SINE: &str = r#"<plot kind="explicit" y="Math.sin(x)"/>"#;

fn object(attrs: &str, children: &str) -> String {
    format!(r#"<object3D id="o" {attrs}>{children}</object3D>"#)
}

const AXES3D: &str = r#"<axes3D width="2" height="2" depth="2"/>"#;
const HELIX: &str = r#"<plot3D x="Math.cos(t)" y="Math.sin(t)" z="t" t1="12.56"/>"#;

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
fn axes_with_plots_of_every_kind_are_valid_at_1_8() {
    valid(&axes("", ""));
    valid(&axes(
        r##"kind="complex" xMin="-2" xMax="2" yMin="-2" yMax="2" xStep="0.5" grid="true" gridStroke="#FFFFFF40" clip="false""##,
        &format!(
            r##"{SINE}<plot kind="parametric" x="Math.cos(t)" y="Math.sin(t)" t1="6.283" closed="true" fill="#FF000080"/><plot kind="polar" r="1 + Math.cos(theta)" samples="512"/><plot kind="implicit" f="x*x + y*y" level="1" resolution="128" trimEnd="0.5" markerEnd="arrow"/><shape id="dot" shape="ellipse" width="8" height="8" x="0.5" y="0.25"/>"##
        ),
    ));
    valid(&axes(r#"kind="number-line" xMin="0" xMax="10""#, ""));
    valid(&axes(
        r#"kind="polar""#,
        r#"<animate property="xMax"><key time="0" value="1"/><key time="1" value="2"/></animate>"#,
    ));
    valid(&object(r#"primitive="axes""#, &format!("{AXES3D}{HELIX}")));
    valid(&object(
        r#"primitive="axes""#,
        r#"<axes3D xMin="0" xMax="4" zMin="0" zMax="1" width="2" height="1" depth="3" zStep="0.25" lineWidth="0.01" tickLength="0.05"/><plot3D x="t" y="t" z="t" tubeRadius="0.02" tubeSegments="3" closed="true"/>"#,
    ));
}

#[test]
fn a_plot_has_the_expressions_of_its_kind_plt1() {
    for bad in [
        r#"<plot kind="explicit"/>"#,
        r#"<plot kind="explicit" y="x" x="x"/>"#,
        r#"<plot kind="explicit" y="x" f="x"/>"#,
        r#"<plot kind="parametric" x="t"/>"#,
        r#"<plot kind="parametric" x="t" y="t" r="t"/>"#,
        r#"<plot kind="polar" r="1" y="1"/>"#,
        r#"<plot kind="polar"/>"#,
        r#"<plot kind="implicit" f="x" y="1"/>"#,
        r#"<plot kind="implicit"/>"#,
    ] {
        refused(&axes("", bad), "PLT1");
    }
}

#[test]
fn each_min_is_below_its_max_plt2() {
    for bad in [r#"xMin="1" xMax="1""#, r#"xMin="2""#, r#"xMax="-1.5""#, r#"yMin="3" yMax="-3""#, r#"yMin="1""#] {
        refused(&axes(bad, ""), "PLT2");
    }
    valid(&axes(r#"xMin="0.5""#, ""));
    valid(&axes(r#"yMax="-0.5""#, ""));
    for bad in [r#"zMin="1""#, r#"zMin="0" zMax="-1""#, r#"xMin="5" xMax="4""#] {
        refused(&object(r#"primitive="axes""#, &format!(r#"<axes3D width="1" height="1" depth="1" {bad}/>"#)), "PLT2");
    }
}

#[test]
fn only_primitive_axes_has_one_axes3d_and_plot3d_children_plt3() {
    refused(&object(r#"primitive="axes""#, ""), "PLT3");
    refused(&object(r#"primitive="axes""#, &format!("{AXES3D}{AXES3D}")), "PLT3");
    refused(&object(r#"primitive="axes""#, HELIX), "PLT3");
    refused(&object(r#"primitive="sphere""#, AXES3D), "PLT3");
    refused(&object(r#"primitive="sphere""#, HELIX), "PLT3");
}

#[test]
fn the_schema_types_refuse_values_outside_their_ranges() {
    for bad in [
        axes(r#"kind="log""#, ""),
        axes(r#"xStep="0""#, ""),
        axes(r#"tickLength="-1""#, ""),
        axes("", r#"<plot kind="cubic" y="x"/>"#),
        axes("", r#"<plot kind="explicit" y="x" samples="1"/>"#),
        axes("", r#"<plot kind="implicit" f="x" resolution="0"/>"#),
        axes("", r#"<plot kind="implicit" f="x" resolution="4097"/>"#),
        axes("", r#"<plot kind="explicit" y="x" strokeWidth="0"/>"#),
        r#"<axes id="ax" width="40"/>"#.into(),
        object(
            r#"primitive="axes""#,
            r#"<axes3D width="2" height="2" depth="2"/><plot3D x="t" y="t" z="t" tubeSegments="2"/>"#,
        ),
        object(r#"primitive="axes""#, r#"<axes3D width="2" height="2" depth="2"/><plot3D x="t" y="t"/>"#),
        object(r#"primitive="axes""#, r#"<axes3D width="2" height="2"/>"#),
    ] {
        let got = codes("1.8", &bad);
        assert!(got.iter().any(|c| c.starts_with('S')), "{bad}: {got:?}");
    }
}

#[test]
fn axes_and_plots_need_version_1_8_v17() {
    let uses = [axes("", SINE), object(r#"primitive="axes""#, &format!("{AXES3D}{HELIX}"))];
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
