//! The typed model of a Schwarzschild black hole, its disk and the camera that traces geodesics: what a renderer reads
//! is in the model with the defaults the schema gives, and the rules refuse what the renderer cannot draw.

use sr_model::model::{AccretionDisk, BlackHole, Camera};

const SCENE: &str = r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/><composition>
  <shape id="sky" shape="rect" x="0" y="0" width="64" height="64" fill="#000000"/>
  <camera id="eye" x="0" y="0" z="-60" geodesics="true"/>
  <blackHole id="hole" mass="1.5" x="1" y="2" z="3"/>
  <accretionDisk id="disk" blackHole="hole" outerRadius="30" temperatureScale="6000"/>
</composition></scene>"##;

fn find<T: 'static + Clone>(doc: &sr_model::Document) -> T {
    let mut found = None;
    sr_model::element::walk(&doc.scene, &mut |e| {
        if let Some(x) = e.as_any().downcast_ref::<T>() {
            found = Some(x.clone());
        }
    });
    found.expect("the element is in the model")
}

fn codes(xml: &str) -> Vec<String> {
    sr_model::validate_str(xml, &sr_model::LoadOptions::without_assets())
        .diagnostics
        .into_iter()
        .map(|d| d.code)
        .collect()
}

#[test]
fn the_model_carries_what_a_renderer_reads_with_the_schemas_defaults() {
    let doc = sr_model::load_str(SCENE, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
    let hole: BlackHole = find(&doc);
    assert_eq!((hole.mass.get(), hole.x, hole.y, hole.z), (1.5, 1.0, 2.0, 3.0));
    let disk: AccretionDisk = find(&doc);
    assert_eq!(disk.black_hole, "hole");
    assert_eq!((disk.outer_radius.get(), disk.temperature_scale.get()), (30.0, 6000.0));
    // the inner radius is the innermost stable circular orbit when absent: the renderer takes 6 mass
    assert!(disk.inner_radius.is_none());
    assert_eq!(disk.seed, 0);
    assert_eq!(disk.angular_pattern.as_str(), "clumps");
    assert_eq!((disk.contrast.get(), disk.intensity.get(), disk.time_scale.get()), (0.5, 1.0, 1.0));
    assert_eq!((disk.rotation_x, disk.rotation_y, disk.rotation), (0.0, 0.0, 0.0));
    // a disk has no place of its own: it is centred on its hole
    assert!(!sr_model::validate_str(
        &SCENE.replace("temperatureScale", "x=\"1\" temperatureScale"),
        &sr_model::LoadOptions::without_assets()
    )
    .diagnostics
    .is_empty());
    let camera: Camera = find(&doc);
    assert!(camera.geodesics);
}

#[test]
fn a_camera_without_geodesics_is_the_camera_there_always_was() {
    let xml = SCENE.replace(r#" geodesics="true""#, "");
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    assert!(!find::<Camera>(&doc).geodesics);
    // and the hole and the disk are warned about, never silently dropped (W11, SREP 76; W03 is SREP 57's)
    assert_eq!(codes(&xml), ["W11", "W11"]);
}

#[test]
fn denoise_on_a_geodesic_camera_is_warned_as_w12() {
    let xml = SCENE.replace(r#"geodesics="true""#, r#"geodesics="true" denoise="true""#);
    assert_eq!(codes(&xml), ["W12"]);
}

#[test]
fn black_holes_need_version_1_3_or_later() {
    // SREP 76, open issue 2: the gate is "1.3 or later", so a black hole can sit in a 1.6 document (V15, V16)
    for later in ["1.3", "1.4", "1.5", "1.6"] {
        let xml = SCENE.replace(r#"version="1.3""#, &format!(r#"version="{later}""#));
        assert!(codes(&xml).is_empty(), "{later}: {:?}", codes(&xml));
    }
    for earlier in ["1.0", "1.1", "1.2"] {
        let xml = SCENE.replace(r#"version="1.3""#, &format!(r#"version="{earlier}""#));
        assert!(codes(&xml).contains(&"BH1".to_string()), "{earlier}");
    }
}

#[test]
fn every_rule_names_what_it_refuses() {
    for (what, from, to, code) in [
        ("a version before 1.3", r#"version="1.3""#, r#"version="1.2""#, "BH1"),
        ("two holes", "<accretionDisk", r#"<blackHole id="b" mass="1"/><accretionDisk"#, "BH2"),
        ("a disk of nothing", r#"blackHole="hole""#, r#"blackHole="eye""#, "BH3"),
        // 8 is below the last stable orbit of a hole of mass 1.5, 6 * 1.5 = 9
        (
            "a disk inside the last stable orbit",
            r#"<accretionDisk id="disk""#,
            r#"<accretionDisk id="disk" innerRadius="8""#,
            "BH4",
        ),
        // the hole is at (1, 2, 3): this is 2 from it, inside 3 * 1.5
        ("a camera inside the photon sphere", r#"x="0" y="0" z="-60""#, r#"x="1" y="2" z="1""#, "BH7"),
        ("another object", "<blackHole", r#"<object3D id="ball" primitive="sphere" radius="1"/><blackHole"#, "BH6"),
        (
            "a second geodesic camera",
            "<blackHole",
            r#"<camera id="eye2" x="0" y="0" z="-90" geodesics="true"/><blackHole"#,
            "BH8",
        ),
    ] {
        let got = codes(&SCENE.replace(from, to));
        assert!(got.contains(&code.to_string()), "{what}: {got:?}");
    }
    let no_hole = SCENE
        .replace(r#"<blackHole id="hole" mass="1.5" x="1" y="2" z="3"/>"#, "")
        .replace(r#"<accretionDisk id="disk" blackHole="hole" outerRadius="30" temperatureScale="6000"/>"#, "");
    assert!(codes(&no_hole).contains(&"BH5".to_string()));
}

/// The lexical forms of `xs:boolean` true and false: XSD 1.0 Part 2, 3.2.2.1, with its whitespace collapsed
/// (3.2.2.2). The model reads every one of them (`parse.rs`), so every rule must too (SREP 76, open issue 4).
const TRUE_FORMS: [&str; 4] = ["true", "1", " true ", "\t1\n"];
const FALSE_FORMS: [&str; 4] = ["false", "0", " false ", "\t0\n"];

fn with_geodesics(xml: &str, form: &str) -> String {
    xml.replace(r#"geodesics="true""#, &format!(r#"geodesics="{form}""#))
}

#[test]
fn every_lexical_form_of_true_is_checked_by_bh5_to_bh8() {
    for on in TRUE_FORMS {
        let doc = sr_model::load_str(&with_geodesics(SCENE, on), &sr_model::LoadOptions::without_assets()).unwrap();
        assert!(find::<Camera>(&doc).geodesics, "{on:?} is true for the model, so the renderer draws the pass");
        assert!(codes(&with_geodesics(SCENE, on)).is_empty(), "{on:?}: {:?}", codes(&with_geodesics(SCENE, on)));
        for (what, from, to, code) in [
            ("no hole", r#"<blackHole id="hole" mass="1.5" x="1" y="2" z="3"/>"#, "", "BH5"),
            ("another object", "<blackHole", r#"<object3D id="ball" primitive="sphere" radius="1"/><blackHole"#, "BH6"),
            ("a camera inside the photon sphere", r#"x="0" y="0" z="-60""#, r#"x="1" y="2" z="1""#, "BH7"),
            // the second camera is written "true": a camera of any form counts against one of any other form
            (
                "a second geodesic camera",
                "<blackHole",
                r#"<camera id="eye2" x="0" y="0" z="-90" geodesics="true"/><blackHole"#,
                "BH8",
            ),
        ] {
            let xml = with_geodesics(SCENE, on).replace(from, to);
            let xml = if code == "BH5" {
                xml.replace(
                    r#"<accretionDisk id="disk" blackHole="hole" outerRadius="30" temperatureScale="6000"/>"#,
                    "",
                )
            } else {
                xml
            };
            let got = codes(&xml);
            assert!(got.contains(&code.to_string()), "geodesics={on:?}, {what}: {got:?}");
        }
        // the warnings of a geodesic camera read the same forms
        let denoise = with_geodesics(&SCENE.replace(r#"geodesics="true""#, r#"geodesics="true" denoise="1""#), on);
        assert_eq!(codes(&denoise), ["W12"], "geodesics={on:?} denoise=\"1\"");
        let lights = with_geodesics(
            &SCENE.replace("</composition>", r#"</composition><lights><light id="sun" type="directional"/></lights>"#),
            on,
        );
        assert_eq!(codes(&lights), ["W05"], "geodesics={on:?} with a light");
    }
}

#[test]
fn every_lexical_form_of_false_is_the_camera_there_always_was() {
    for off in FALSE_FORMS {
        let xml = with_geodesics(SCENE, off);
        let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
        assert!(!find::<Camera>(&doc).geodesics, "{off:?}");
        // nothing draws the hole and the disk (W11), and no rule of a geodesic camera applies, not even with no hole
        assert_eq!(codes(&xml), ["W11", "W11"], "{off:?}");
        let no_hole = xml
            .replace(r#"<blackHole id="hole" mass="1.5" x="1" y="2" z="3"/>"#, "")
            .replace(r#"<accretionDisk id="disk" blackHole="hole" outerRadius="30" temperatureScale="6000"/>"#, "");
        assert!(codes(&no_hole).is_empty(), "{off:?}: {:?}", codes(&no_hole));
        // a geodesic camera of another form draws the hole: no W11, and only that camera counts for BH8
        let other = xml.replace("<blackHole", r#"<camera id="eye2" x="0" y="0" z="-90" geodesics="1"/><blackHole"#);
        assert!(codes(&other).is_empty(), "{off:?} beside geodesics=\"1\": {:?}", codes(&other));
        // denoise="1" on a camera without geodesics is not W12
        let denoise = xml.replace(&format!(r#"geodesics="{off}""#), &format!(r#"geodesics="{off}" denoise="1""#));
        assert_eq!(codes(&denoise), ["W11", "W11"], "{off:?} denoise=\"1\"");
    }
}

#[test]
fn the_distance_to_the_hole_is_the_distance_from_its_place() {
    // the camera 4.4 from a hole of mass 1.5 (3 mass = 4.5) is inside, 4.6 is outside
    let xml = |d: f64| SCENE.replace(r#"x="0" y="0" z="-60""#, &format!(r#"x="1" y="2" z="{}""#, 3.0 - d));
    assert!(codes(&xml(4.4)).contains(&"BH7".to_string()));
    assert!(codes(&xml(4.6)).is_empty(), "{:?}", codes(&xml(4.6)));
}

#[test]
fn a_scene_that_only_draws_the_hole_is_valid_and_the_disk_is_bounded_by_the_orbit() {
    assert!(codes(SCENE).is_empty(), "{:?}", codes(SCENE));
    // 9 is exactly 6 * 1.5: the last stable orbit is allowed; outer radius must be beyond it
    let at = SCENE.replace("<accretionDisk id=\"disk\"", "<accretionDisk id=\"disk\" innerRadius=\"9\"");
    assert!(codes(&at).is_empty(), "{:?}", codes(&at));
    assert!(codes(&at.replace(r#"outerRadius="30""#, r#"outerRadius="9""#)).contains(&"BH4".to_string()));
}

#[test]
fn the_black_hole_warnings_have_codes_of_their_own() {
    // SREP 76, open issue 1: W03 and W04 are SREP 57's generated-marker warnings
    let summary = |code: &str| sr_model::codes::lookup(code).unwrap_or_else(|| panic!("{code} is explained")).summary;
    assert!(summary("W11").contains("blackHole"));
    assert!(summary("W12").contains("denoise"));
    assert!(!summary("W03").contains("blackHole"));
    assert!(!summary("W04").contains("geodesics"));
}
