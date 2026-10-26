//! The star-fields draft (schema/UPSTREAM): the `catalogue` asset, a dome light's `starField`, the Schematron rules
//! SF2 to SF4 and their clause of V17, and W17 for an epoch a space overrides. SF1 reads the file and is the asset
//! stage's (assets.rs).

const DIGEST: &str = "sha256:cd18b1c9b8c009d922edfed020ba718aa14a577416fb625bf01ddd9cbdaab617";

fn codes_at(version: &str, assets: &str, lights: &str, space: &str) -> Vec<String> {
    let xml = format!(
        r#"<scene version="{version}"><project width="64" height="64" fps="24" duration="1"/><assets>{assets}</assets><composition/><lights>{lights}</lights>{space}</scene>"#
    );
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
    sr_model::validate_str(&xml, &opts).diagnostics.into_iter().map(|d| d.code).collect()
}

fn file() -> String {
    format!(r#"<catalogue id="bsc" src="bsc5-compact.csv.gz" digest="{DIGEST}"/>"#)
}

/// A light of `kind` holding `children`.
fn light(kind: &str, children: &str) -> String {
    format!(r#"<light id="sky" type="{kind}">{children}</light>"#)
}

fn field(attrs: &str) -> String {
    format!(r#"<starField catalogue="bsc"{attrs}/>"#)
}

fn codes(lights: &str) -> Vec<String> {
    codes_at("1.8", &file(), lights, "")
}

#[track_caller]
fn valid(lights: &str) {
    let got = codes(lights);
    assert!(got.is_empty(), "{lights}: {got:?}");
}

#[track_caller]
fn refused(lights: &str, code: &str) {
    let got = codes(lights);
    assert!(got.iter().any(|c| c == code), "{lights}: want {code}, got {got:?}");
}

#[test]
fn catalogues_and_star_fields_are_valid_at_1_8() {
    valid(&light("dome", &field("")));
    valid(&light(
        "dome",
        &field(
            r#" limitMagnitude="4.5" magnitudeZero="0.25" pointSize="3" properMotion="false" epoch="-7999.5" constellations="boundaries""#,
        ),
    ));
    valid(&light("dome", &field(r#" constellations="labels" epoch="12000""#)));
    // a field beside a light's animation children
    valid(&light(
        "dome",
        &format!(
            r#"{}<animate property="intensity"><key time="0" value="0"/><key time="1" value="1"/></animate>"#,
            field("")
        ),
    ));
    // a catalogue no light reads
    valid(&light("dome", ""));
}

#[test]
fn malformed_catalogues_and_fields_are_structure_errors() {
    for attrs in [
        r#" magnitudeZero="0""#,
        r#" pointSize="-1""#,
        r#" properMotion="yes""#,
        r#" constellations="lines""#,
        r#" epoch="J2000""#,
        r#" limitMagnitude="bright""#,
        r#" colour="red""#,
    ] {
        let got = codes(&light("dome", &field(attrs)));
        assert!(got.iter().any(|c| c == "S06" || c == "S04"), "{attrs}: {got:?}");
    }
    let got = codes(&light("dome", "<starField/>"));
    assert!(got.iter().any(|c| c == "S05"), "{got:?}");
    for bad in [
        r#"<catalogue id="bsc" src="a.csv.gz" digest="sha256:XYZ"/>"#.to_string(),
        format!(r#"<catalogue id="bsc" src="a.csv.gz" digest="{}"/>"#, &DIGEST[7..]),
    ] {
        let got = codes_at("1.8", &bad, &light("dome", &field("")), "");
        assert!(got.iter().any(|c| c == "S06"), "{bad}: {got:?}");
    }
    let got = codes_at("1.8", r#"<catalogue id="bsc" src="a.csv.gz"/>"#, &light("dome", &field("")), "");
    assert!(got.iter().any(|c| c == "S05"), "{got:?}");
}

#[test]
fn a_star_field_is_one_on_a_dome_light_naming_a_catalogue_sf2() {
    for kind in ["directional", "ambient", "point"] {
        refused(&light(kind, &field("")), "SF2");
    }
    refused(&light("dome", &format!("{}{}", field(""), field(r#" pointSize="2""#))), "SF2");
    // an IDREF to another kind of asset
    let got = codes_at(
        "1.8",
        &format!(r#"{}<value id="v" value="1"/>"#, file()),
        &light("dome", r#"<starField catalogue="v"/>"#),
        "",
    );
    assert!(got.iter().any(|c| c == "SF2"), "{got:?}");
}

#[test]
fn the_limit_magnitude_is_finite_sf3() {
    for limit in ["INF", "-INF", "NaN"] {
        refused(&light("dome", &field(&format!(r#" limitMagnitude="{limit}""#))), "SF3");
    }
    valid(&light("dome", &field(r#" limitMagnitude="-1.5""#)));
    valid(&light("dome", &field(r#" limitMagnitude="+30""#)));
}

#[test]
fn the_epoch_is_within_ten_thousand_years_of_j2000_sf4() {
    for epoch in ["12000.001", "-8000.5", "1e5", "INF"] {
        refused(&light("dome", &field(&format!(r#" epoch="{epoch}""#))), "SF4");
    }
    for epoch in ["12000", "-8000", "+2026.8"] {
        valid(&light("dome", &field(&format!(r#" epoch="{epoch}""#))));
    }
}

#[test]
fn a_space_gives_the_epoch_and_a_written_one_is_ignored_w17() {
    let sp = r#"<space epoch="2451545.0" timeScale="tdb"/>"#;
    let got = codes_at("1.8", &file(), &light("dome", &field(r#" epoch="2100""#)), sp);
    assert_eq!(got, ["W17"]);
    let got = codes_at("1.8", &file(), &light("dome", &field("")), sp);
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn before_1_8_catalogues_and_star_fields_are_refused_v17() {
    for v in ["1.0", "1.4", "1.7"] {
        for (assets, lights) in
            [(file(), light("dome", "")), (String::new(), light("dome", r#"<starField catalogue="x"/>"#))]
        {
            let got = codes_at(v, &assets, &lights, "");
            assert!(got.iter().any(|k| k == "V17"), "{v} {assets} {lights}: {got:?}");
        }
    }
    let got = codes_at("1.7", "", &light("dome", ""), "");
    assert!(!got.iter().any(|k| k == "V17"), "{got:?}");
}
