//! The planets draft (schema/UPSTREAM): `primitive="planet"` and its attributes on `object3D`, the Schematron rules
//! PL1 to PL5 and their clause of V17, and W19 for a flattening that a body's radii override.

const MATERIALS: &str = r##"<materials><material id="earth" baseColorMap="day.png" emissiveMap="night.png"/><material id="photosphere" emissive="#FFF0E0"/><material id="rock" baseColor="#808080"/></materials>"##;

fn codes_at(version: &str, objects: &str) -> Vec<String> {
    let xml = format!(
        r#"<scene version="{version}"><project width="64" height="64" fps="24" duration="1"/>{MATERIALS}<composition><camera id="eye" z="-10"/>{objects}</composition></scene>"#
    );
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
    sr_model::validate_str(&xml, &opts).diagnostics.into_iter().map(|d| d.code).collect()
}

fn codes(objects: &str) -> Vec<String> {
    codes_at("1.8", objects)
}

/// A planet with `attrs`.
fn planet(attrs: &str) -> String {
    format!(r#"<object3D id="p" primitive="planet" {attrs}/>"#)
}

#[track_caller]
fn valid(objects: &str) {
    let got = codes(objects);
    assert!(got.is_empty(), "{objects}: {got:?}");
}

#[track_caller]
fn refused(objects: &str, code: &str) {
    let got = codes(objects);
    assert_eq!(got, vec![code.to_string()], "{objects}");
}

#[test]
fn planets_are_valid_at_1_8() {
    valid(&planet(""));
    valid(&planet(r#"radius="120" flattening="0.0979624" segments="96" material="rock""#));
    valid(&planet(
        r#"body="399" material="earth" nightLights="true" twilight="6" cloudMap="clouds.png" cloudHeight="0.004" cloudRotation="-1.5e-2" cloudShadow="false" atmosphere="earth" atmosphereHeight="100000""#,
    ));
    valid(&planet(r#"body="699" ringMap="rings.png" ringInner="1.239" ringOuter="2.269""#));
    // the Sun lights itself: an emissive colour or an emissive map
    valid(&planet(r#"body="10" material="photosphere" limbDarkening="solar""#));
    valid(&planet(r#"radius="40" material="earth" limbDarkening="solar""#));
    // a body without IAU radii takes its radius from the document
    valid(&planet(r#"body="2000001" radius="4" flattening="0.1""#));
    // signed and padded numbers, the edges of the ranges
    valid(&planet(
        r#"flattening="+0.999" twilight="90" cloudMap="c.png" cloudHeight="1" ringMap="r.png" ringInner="1.0001" ringOuter="+1.0002" nightLights="false" limbDarkening="none" atmosphere="none""#,
    ));
}

#[test]
fn malformed_planet_attributes_are_structure_errors() {
    for attrs in [
        r#"flattening="-0.1""#,
        r#"twilight="-1""#,
        r#"cloudMap="c.png" cloudHeight="0""#,
        r#"ringMap="r.png" ringInner="0""#,
        r#"limbDarkening="quadratic""#,
        r#"atmosphere="mars""#,
        r#"atmosphereHeight="0""#,
        r#"nightLights="maybe""#,
        r#"cloudMap="c.png" cloudRotation="east""#,
    ] {
        let got = codes(&planet(attrs));
        // the structure stage reports the value; a rule that reads it may report it too (ringInner 0 is PL3's)
        assert!(got.iter().any(|c| c.starts_with('S')), "{attrs}: {got:?}");
        assert!(got.iter().all(|c| c.starts_with('S') || c == "PL3"), "{attrs}: {got:?}");
    }
}

#[test]
fn a_body_has_iau_radii_or_a_radius_pl1() {
    for b in ["10", "199", "299", "399", "499", "599", "699", "799", "899", "301", "+0399"] {
        valid(&planet(&format!(r#"body="{b}""#)));
    }
    for b in ["0", "3", "401", "2000001"] {
        refused(&planet(&format!(r#"body="{b}""#)), "PL1");
        valid(&planet(&format!(r#"body="{b}" radius="10""#)));
    }
}

#[test]
fn the_planet_attributes_are_written_only_on_a_planet_pl2() {
    for attrs in [
        r#"flattening="0.1""#,
        r#"nightLights="false""#,
        r#"twilight="6""#,
        r#"cloudMap="c.png""#,
        r#"cloudHeight="0.1""#,
        r#"cloudRotation="1""#,
        r#"cloudShadow="true""#,
        r#"ringMap="r.png""#,
        r#"ringInner="1.5""#,
        r#"ringOuter="2""#,
        r#"limbDarkening="none""#,
        r#"atmosphere="earth""#,
        r#"atmosphereHeight="1000""#,
    ] {
        refused(&format!(r#"<object3D id="s" primitive="sphere" {attrs}/>"#), "PL2");
    }
    // body alone is the time-frames draft's, on any primitive
    valid(r#"<object3D id="s" primitive="sphere" body="399"/>"#);
}

#[test]
fn the_planet_numbers_are_in_range_pl3() {
    refused(&planet(r#"flattening="1""#), "PL3");
    refused(&planet(r#"twilight="90.5""#), "PL3");
    refused(&planet(r#"cloudMap="c.png" cloudHeight="1.01""#), "PL3");
    refused(&planet(r#"ringMap="r.png" ringInner="1""#), "PL3");
    refused(&planet(r#"ringMap="r.png" ringInner="2" ringOuter="2""#), "PL3");
    // against the defaults: ringOuter 2.269 is not above ringInner 3, ringOuter 1.2 not above 1.239
    refused(&planet(r#"ringMap="r.png" ringInner="3""#), "PL3");
    refused(&planet(r#"ringMap="r.png" ringOuter="1.2""#), "PL3");
    // a zero height is the structure stage's and PL3's, as the Schematron also asserts it
    let mut got = codes(&planet(r#"atmosphereHeight="0""#));
    got.sort();
    assert_eq!(got, ["PL3", "S06"]);
}

#[test]
fn clouds_and_rings_need_their_maps_pl4() {
    for attrs in [r#"cloudHeight="0.01""#, r#"cloudRotation="1""#, r#"cloudShadow="false""#] {
        refused(&planet(attrs), "PL4");
    }
    for attrs in [r#"ringInner="1.5""#, r#"ringOuter="2""#] {
        refused(&planet(attrs), "PL4");
    }
}

#[test]
fn solar_limb_darkening_needs_an_emissive_sun_pl5() {
    refused(&planet(r#"limbDarkening="solar""#), "PL5");
    refused(&planet(r#"material="rock" limbDarkening="solar""#), "PL5");
    refused(&planet(r#"body="399" material="earth" limbDarkening="solar""#), "PL5");
    valid(&planet(r#"body="399" material="earth" limbDarkening="none""#));
}

#[test]
fn before_1_8_planets_are_refused_v17() {
    for objects in [planet(""), planet(r#"radius="10" flattening="0.1""#)] {
        assert_eq!(codes_at("1.7", &objects), vec!["V17".to_string()], "{objects}");
    }
    // the attributes alone, on another primitive: PL2 and the gate
    let got = codes_at("1.7", r#"<object3D id="s" primitive="sphere" cloudMap="c.png"/>"#);
    assert!(got.contains(&"V17".to_string()) && got.contains(&"PL2".to_string()), "{got:?}");
}

#[test]
fn a_body_with_radii_ignores_the_flattening_w19() {
    assert_eq!(codes(&planet(r#"body="399" flattening="0.5""#)), vec!["W19".to_string()]);
    // without IAU radii the flattening is read
    valid(&planet(r#"body="2000001" radius="4" flattening="0.5""#));
}
