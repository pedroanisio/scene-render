//! The scale draft (schema/UPSTREAM): a camera's `depth` and `scaleBar` child, the `geometric` value of `curveType`,
//! the Schematron rules SC1 to SC4 and their clause of V17.

fn codes_at(version: &str, camera: &str, kids: &str, composition: &str) -> Vec<String> {
    let xml = format!(
        r#"<scene version="{version}"><project width="640" height="360" fps="24" duration="1"/><composition><camera id="eye" z="-10" {camera}>{kids}</camera><object3D id="ball" primitive="sphere"/>{composition}</composition></scene>"#
    );
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
    let mut got: Vec<String> = sr_model::validate_str(&xml, &opts).diagnostics.into_iter().map(|d| d.code).collect();
    got.sort();
    got
}

fn codes(camera: &str, kids: &str) -> Vec<String> {
    codes_at("1.8", camera, kids, "")
}

/// An animation of the camera's z through keys of `values`, the first `interpolation`.
fn keyed(interpolation: &str, values: &[&str]) -> String {
    let keys: String = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let curve = if i == 0 { format!(r#" interpolation="{interpolation}""#) } else { String::new() };
            format!(r#"<key time="{i}" value="{v}"{curve}/>"#)
        })
        .collect();
    format!(r#"<animate property="z">{keys}</animate>"#)
}

#[track_caller]
fn valid(camera: &str, kids: &str) {
    let got = codes(camera, kids);
    assert!(got.is_empty(), "{camera} {kids}: {got:?}");
}

#[track_caller]
fn refused(camera: &str, kids: &str, code: &str) {
    assert_eq!(codes(camera, kids), vec![code.to_string()], "{camera} {kids}");
}

#[test]
fn depth_scale_bars_and_geometric_keys_are_valid_at_1_8() {
    valid(r#"depth="logarithmic" near="0.1" far="1e19""#, "");
    valid(r#"depth="reversed""#, "");
    valid("", "<scaleBar/>");
    valid(
        r#"near="1" far="1e6""#,
        r##"<scaleBar maxWidth="640" units="si" distance="1000" x="10" y="+5.5" color="#FFCC00" thickness="3" label="false"/>"##,
    );
    valid(r#"near="10" far="1e21""#, &keyed("geometric", &["10", "1e20"]));
    // two negative values, and a geometric key whose next key is reached by another curve
    valid("", &keyed("geometric", &["-1", "-100", "0"]));
    // values that are not plain numbers are left to the build (E04), which sees their magnitudes
    valid("", &keyed("geometric", &["1", "x"]));
    valid("", &keyed("geometric", &["1vw", "-100vw"]));
    valid("", &keyed("geometric", &["1", "1e400"]));
    // a last key's curve governs no segment
    valid(
        "",
        r#"<animate property="z"><key time="0" value="0"/><key time="1" value="10" interpolation="geometric"/></animate>"#,
    );
    // geometric as the animation's default
    valid(
        "",
        r#"<animate property="z" defaultInterpolation="geometric"><key time="0" value="1"/><key time="1" value="10" interpolation="linear"/><key time="2" value="0"/></animate>"#,
    );
}

#[test]
fn sc1_near_is_positive_far_exceeds_it_and_their_ratio_is_at_most_1e20() {
    refused(r#"near="0.1" far="2e19""#, "", "SC1");
    // far exceeds near
    refused(r#"near="10" far="10""#, "", "SC1");
    refused(r#"near="10" far="5""#, "", "SC1");
    // the defaults: near 0.1 and far 10000
    refused(r#"far="1.1e19""#, "", "SC1");
    refused(r#"near="1e-17""#, "", "SC1");
    valid(r#"near="2e-16""#, "");
}

#[test]
fn sc2_a_geometric_segment_has_non_zero_values_of_one_sign() {
    refused("", &keyed("geometric", &["-1", "10"]), "SC2");
    refused("", &keyed("geometric", &["0", "10"]), "SC2");
    refused("", &keyed("geometric", &["10", "0"]), "SC2");
    // from the animation's default, on the key that does not name its own curve
    refused(
        "",
        r#"<animate property="z" defaultInterpolation="geometric"><key time="0" value="1" interpolation="linear"/><key time="1" value="-1"/><key time="2" value="10"/></animate>"#,
        "SC2",
    );
    // a time remap's keys (the corpus document whose first segment remaps from 12 s to 10 s)
    let doc =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/corpus/valid/c15-loop-00.scene.xml"))
            .unwrap()
            .replace(r#"<scene version="1.2">"#, r#"<scene version="1.8">"#)
            .replacen("<timeRemap>", r#"<timeRemap defaultInterpolation="geometric">"#, 1);
    let remap = |xml: &str| {
        let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
        sr_model::validate_str(xml, &opts).diagnostics.into_iter().map(|d| d.code).collect::<Vec<_>>()
    };
    assert!(remap(&doc).is_empty(), "{:?}", remap(&doc));
    assert_eq!(remap(&doc.replacen(r#"<key time="2" value="10"/>"#, r#"<key time="2" value="-10"/>"#, 1)), vec!["SC2"]);
}

#[test]
fn sc2_refuses_geometric_on_an_easing_of_progress() {
    // these curves ease a progress from 0 to 1, whose first value is zero
    for (composition, what) in [
        (
            r#"<shape id="s" shape="rect" width="10" height="10"><motionPath path="M0 0 L10 0" interpolation="geometric"/></shape>"#,
            "motionPath",
        ),
        (r#"<particleEmitter id="p" sizeCurve="geometric"/>"#, "sizeCurve"),
        (r#"<particleEmitter id="p" colorCurve="geometric"/>"#, "colorCurve"),
    ] {
        let got = codes_at("1.8", "", "", composition);
        assert!(got.contains(&"SC2".to_string()), "{what}: {got:?}");
    }
}

#[test]
fn sc3_one_scale_bar_no_wider_than_the_scene() {
    refused("", r#"<scaleBar maxWidth="641"/>"#, "SC3");
    refused("", "<scaleBar/><scaleBar/>", "SC3");
    valid("", r#"<scaleBar maxWidth="640"/>"#);
}

#[test]
fn sc4_a_scale_bar_distance_lies_between_near_and_far() {
    refused(r#"near="1" far="100""#, r#"<scaleBar distance="101"/>"#, "SC4");
    refused(r#"near="1" far="100""#, r#"<scaleBar distance="0.5"/>"#, "SC4");
    // the default far of 10000
    refused("", r#"<scaleBar distance="20000"/>"#, "SC4");
    valid(r#"near="1" far="100""#, r#"<scaleBar distance="100"/>"#);
}

#[test]
fn malformed_attributes_are_structure_errors() {
    for (camera, kids) in [
        (r#"depth="linear""#, ""),
        ("", r#"<scaleBar maxWidth="0"/>"#),
        ("", r#"<scaleBar units="imperial"/>"#),
        ("", r#"<scaleBar thickness="0"/>"#),
        ("", r#"<scaleBar distance="-1"/>"#),
        ("", r#"<scaleBar label="maybe"/>"#),
        ("", r#"<scaleBar unknown="1"/>"#),
        ("", &keyed("logarithmic", &["1", "10"])),
    ] {
        let got = codes(camera, kids);
        assert!(got.iter().any(|c| c.starts_with('S') && !c.starts_with("SC")), "{camera} {kids}: {got:?}");
    }
}

#[test]
fn v17_gates_depth_scale_bars_and_geometric_before_1_8() {
    for (camera, kids) in [
        (r#"depth="reversed""#, ""),
        ("", "<scaleBar/>"),
        ("", &keyed("geometric", &["1", "10"])),
        ("", r#"<animate property="z" defaultInterpolation="geometric"><key time="0" value="1"/></animate>"#),
    ] {
        let got = codes_at("1.7", camera, kids, "");
        assert!(got.contains(&"V17".to_string()), "{camera} {kids}: {got:?}");
    }
    assert!(codes_at("1.7", "", "", "").is_empty());
}
