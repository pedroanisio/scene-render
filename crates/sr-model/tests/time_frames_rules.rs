//! The time-frames draft (schema/UPSTREAM): the `space` element, the `ephemeris` asset, the `object3D` attributes
//! body, positionFrom and orientation, the Schematron rules TF2 to TF5 and their clause of V17, the one version gate
//! of the 1.8 drafts. TF1 and the clauses of TF4 and TF5 that read the file are the asset stage's (assets.rs).

const DIGEST: &str = "sha256:d113bb5117946244cf63de9d06287d5aac008acd1a561d28b760bcc7d4c3d1fd";

fn codes_at(version: &str, assets: &str, composition: &str, space: &str) -> Vec<String> {
    let xml = format!(
        r#"<scene version="{version}"><project width="64" height="64" fps="24" duration="1"/><assets>{assets}</assets><composition>{composition}</composition>{space}</scene>"#
    );
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
    sr_model::validate_str(&xml, &opts).diagnostics.into_iter().map(|d| d.code).collect()
}

fn file() -> String {
    format!(r#"<ephemeris id="de" src="de440s-excerpt.bsp" digest="{DIGEST}"/>"#)
}

fn codes(space: &str, composition: &str) -> Vec<String> {
    codes_at("1.8", &file(), composition, space)
}

#[track_caller]
fn valid(space: &str, composition: &str) {
    let got = codes(space, composition);
    assert!(got.is_empty(), "{space} {composition}: {got:?}");
}

#[track_caller]
fn refused(space: &str, composition: &str, code: &str) {
    let got = codes(space, composition);
    assert!(got.iter().any(|c| c == code), "{space} {composition}: want {code}, got {got:?}");
}

fn space(epoch: &str, rest: &str) -> String {
    format!(r#"<space epoch="{epoch}"{rest}/>"#)
}

fn body(attrs: &str) -> String {
    format!(r#"<object3D id="b" primitive="sphere" radius="4"{attrs}/>"#)
}

#[test]
fn spaces_ephemerides_and_placed_bodies_are_valid_at_1_8() {
    valid(&space("2026-10-24T00:00:00Z", ""), "");
    valid(
        &space("2451545.0", r#" timeScale="tdb" rate="3600" origin="399" frame="eclipticJ2000" ephemeris="de""#),
        &body(r#" body="301" positionFrom="ephemeris" orientation="bodyFixed""#),
    );
    valid(&space("2451545.0", r#" timeScale="tt" frame="bodyFixed" frameBody="399" ephemeris="de""#), "");
    valid(
        &space("2025-06-01T00:00:00Z", r#" ephemeris="approximate""#),
        &body(r#" body="4" positionFrom="ephemeris""#),
    );
    // a body without positionFrom or orientation is only a label
    valid(&space("2025-06-01T00:00:00Z", ""), &body(r#" body="499""#));
}

#[test]
fn malformed_spaces_and_ephemerides_are_structure_errors() {
    for (bad, assets) in [
        (space("2451545.0", r#" timeScale="tai""#), file()),
        (space("2451545.0", r#" frame="galactic""#), file()),
        (space("2451545.0", r#" rate="-1""#), file()),
        (space("2451545.0", r#" origin="earth""#), file()),
        (space("2451545.0", r#" frameBody="3.5""#), file()),
        (space("2026-10-24T00:00:00Z", ""), r#"<ephemeris id="de" src="a.bsp" digest="sha256:XYZ"/>"#.into()),
        (space("2026-10-24T00:00:00Z", ""), format!(r#"<ephemeris id="de" src="a.bsp" digest="{}"/>"#, &DIGEST[7..])),
    ] {
        let got = codes_at("1.8", &assets, "", &bad);
        assert!(got.iter().any(|c| c == "S06"), "{bad} {assets}: {got:?}");
    }
    let got = codes_at("1.8", "", "", r#"<space/>"#);
    assert!(got.iter().any(|c| c == "S05"), "{got:?}");
    let got = codes_at("1.8", r#"<ephemeris id="de" src="a.bsp"/>"#, "", "");
    assert!(got.iter().any(|c| c == "S05"), "{got:?}");
    for attrs in [r#" body="earth""#, r#" positionFrom="file""#, r#" orientation="sky""#] {
        let got = codes(&space("2026-10-24T00:00:00Z", ""), &body(attrs));
        assert!(got.iter().any(|c| c == "S06"), "{attrs}: {got:?}");
    }
    let one = space("2026-10-24T00:00:00Z", "");
    let got = codes_at("1.8", "", "", &format!("{one}{one}"));
    assert!(!got.is_empty(), "two spaces: {got:?}");
}

#[test]
fn an_epoch_parses_as_rule_1_says_tf2() {
    for epoch in [
        "2026-10-24 00:00:00Z",
        "2026-10-24T00:00:00",
        "2026-10-24T00:00Z",
        "2026-10-24T00:00:00.Z",
        "2026-10-24T00:00:00,5Z",
        "26-10-24T00:00:00Z",
        "2026-13-01T00:00:00Z",
        "2026-00-01T00:00:00Z",
        "2026-02-29T00:00:00Z",
        "2100-02-29T00:00:00Z",
        "2026-04-31T00:00:00Z",
        "2026-10-24T24:00:00Z",
        "2026-10-24T00:60:00Z",
        "2026-10-24T00:00:60Z",
        "2026-10-24T00:00:00+24:00",
        "2026-10-24T00:00:00+05:60",
        "2026-10-24T00:00:00+0530",
        "1971-12-31T23:59:59Z",
        "1972-01-01T00:30:00+01:00",
        " 2026-10-24T00:00:00Z",
        "2026-10-24T00:00:00+0é:00",
        "2026-10-24T00:00:00.5éZ",
        "２026-10-24T00:00:00Z",
        "",
    ] {
        refused(&space(epoch, r#" ephemeris="de""#), "", "TF2");
    }
    for epoch in [
        "2024-02-29T12:00:00.25Z",
        "2000-02-29T00:00:00Z",
        "1972-01-01T00:00:00Z",
        "1972-01-01T00:30:00-01:00",
        "2026-10-24T00:00:00+05:30",
        "2026-10-24T23:59:59.999999Z",
    ] {
        valid(&space(epoch, r#" ephemeris="de""#), "");
    }
    for epoch in ["J2000", "2451545.0.1", "1e6", "", ".", "+", "2451545 0", "INF", "NaN"] {
        refused(&space(epoch, r#" timeScale="tdb" ephemeris="de""#), "", "TF2");
    }
    for epoch in ["2451545.0", " 2451545 ", "+2451545", "-100.5", ".5", "2451545."] {
        valid(&space(epoch, r#" timeScale="tt" ephemeris="de""#), "");
    }
}

#[test]
fn the_approximate_elements_bound_the_epoch_tf2() {
    // the long table (rule 5): years -3000 to 3000, Julian days 625332.5 to 2817152.5
    refused(&space("3001-01-01T00:00:00Z", ""), "", "TF2");
    valid(&space("3000-12-31T23:59:59Z", ""), "");
    valid(&space("3001-01-01T00:00:00Z", r#" ephemeris="de""#), "");
    for (jd, ok) in [("625332.4", false), ("625332.5", true), ("2817152.4", true), ("2817152.5", false)] {
        let got = codes(&space(jd, r#" timeScale="tdb""#), "");
        assert_eq!(!got.iter().any(|c| c == "TF2"), ok, "{jd}: {got:?}");
    }
}

#[test]
fn a_body_fixed_frame_needs_a_body_with_rotation_elements_tf3() {
    refused(&space("2451545.0", r#" timeScale="tdb" frame="bodyFixed""#), "", "TF3");
    refused(&space("2451545.0", r#" timeScale="tdb" frame="bodyFixed" frameBody="4""#), "", "TF3");
    for b in ["10", "199", "299", "399", "499", "599", "699", "799", "899", "301", "+399", "0399", " 399 "] {
        valid(&space("2451545.0", &format!(r#" timeScale="tdb" frame="bodyFixed" frameBody="{b}""#)), "");
    }
}

#[test]
fn a_placed_object_needs_a_body_and_a_space_tf4() {
    let approx = space("2025-06-01T00:00:00Z", "");
    let de = space("2025-06-01T00:00:00Z", r#" ephemeris="de""#);
    refused(&approx, &body(r#" positionFrom="ephemeris""#), "TF4");
    refused(&approx, &body(r#" orientation="bodyFixed""#), "TF4");
    let got = codes_at("1.8", &file(), &body(r#" body="399" positionFrom="ephemeris""#), "");
    assert!(got.iter().any(|c| c == "TF4"), "no space: {got:?}");
    // the approximate tables hold the planets 1 to 8
    for b in ["0", "9", "10", "301", "399"] {
        refused(&approx, &body(&format!(r#" body="{b}" positionFrom="ephemeris""#)), "TF4");
    }
    for b in ["1", "3", "8", "+5"] {
        valid(&approx, &body(&format!(r#" body="{b}" positionFrom="ephemeris""#)));
    }
    valid(&de, &body(r#" body="301" positionFrom="ephemeris""#));
    // orientation needs rotation elements, whatever the ephemeris
    refused(&de, &body(r#" body="3" orientation="bodyFixed""#), "TF4");
    valid(&de, &body(r#" body="399" orientation="bodyFixed""#));
    valid(&approx, &body(r#" body="399" orientation="bodyFixed""#));
}

#[test]
fn the_rate_and_the_ephemeris_reference_are_checked_tf5() {
    refused(&space("2451545.0", r#" timeScale="tdb" rate="INF" ephemeris="de""#), "", "TF5");
    valid(&space("2451545.0", r#" timeScale="tdb" rate="0" ephemeris="de""#), "");
    valid(&space("2451545.0", r#" timeScale="tdb" rate="+60" ephemeris="de""#), "");
    refused(&space("2451545.0", r#" timeScale="tdb" ephemeris="nope""#), "", "TF5");
    // the approximate elements are heliocentric: the origin is the barycentre (taken as the Sun), the Sun or a planet
    for o in ["0", "10", "1", "8"] {
        valid(&space("2451545.0", &format!(r#" timeScale="tdb" origin="{o}""#)), "");
    }
    for o in ["399", "301", "9"] {
        refused(&space("2451545.0", &format!(r#" timeScale="tdb" origin="{o}""#)), "", "TF5");
        valid(&space("2451545.0", &format!(r#" timeScale="tdb" origin="{o}" ephemeris="de""#)), "");
    }
}

#[test]
fn before_1_8_spaces_ephemerides_and_placed_bodies_are_refused_v17() {
    let v17 = |assets: &str, composition: &str, sp: &str| {
        for v in ["1.0", "1.3", "1.6", "1.7"] {
            let got = codes_at(v, assets, composition, sp);
            assert!(got.iter().any(|k| k == "V17"), "{v} {assets} {composition} {sp}: {got:?}");
        }
    };
    v17("", "", &space("2026-10-24T00:00:00Z", ""));
    v17(&file(), "", "");
    for attrs in [r#" body="399""#, r#" positionFrom="document""#, r#" orientation="document""#] {
        v17("", &body(attrs), "");
    }
    let got = codes_at("1.7", "", &body(""), "");
    assert!(!got.iter().any(|k| k == "V17"), "{got:?}");
}
