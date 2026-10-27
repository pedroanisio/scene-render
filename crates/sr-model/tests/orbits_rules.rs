//! The orbits draft (schema/UPSTREAM): the `orbit` and `trail` children of `object3D`, the `nbody` child of `space`
//! with its `member` and `cluster` children, the Schematron rules OR1 to OR5 and their clause of V17.

fn codes_at(version: &str, objects: &str, nbody: &str) -> Vec<String> {
    codes_with(version, "24", r#"rate="3600""#, objects, nbody)
}

fn codes_with(version: &str, fps: &str, rate: &str, objects: &str, nbody: &str) -> Vec<String> {
    let xml = format!(
        r#"<scene version="{version}"><project width="64" height="64" fps="{fps}" duration="1"/><composition><camera id="eye" z="-10"/><object3D id="sun" primitive="sphere" body="10"/><object3D id="star" primitive="sphere"/>{objects}</composition><space epoch="2451545.0" timeScale="tdb" {rate}>{nbody}</space></scene>"#
    );
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
    let mut got: Vec<String> = sr_model::validate_str(&xml, &opts).diagnostics.into_iter().map(|d| d.code).collect();
    got.sort();
    got
}

fn codes(objects: &str, nbody: &str) -> Vec<String> {
    codes_at("1.8", objects, nbody)
}

/// An object `earth` with the children `kids`.
fn earth(kids: &str) -> String {
    format!(r#"<object3D id="earth" primitive="sphere">{kids}</object3D>"#)
}

fn orbit(attrs: &str) -> String {
    earth(&format!(r#"<orbit around="sun" {attrs}/>"#))
}

const PAIR: &str = r#"<nbody step="150"><member ref="sun" body="10"/><member ref="earth" mass="5.97e24" x="1.496e11" vy="29780"/></nbody>"#;

#[track_caller]
fn valid(objects: &str, nbody: &str) {
    let got = codes(objects, nbody);
    assert!(got.is_empty(), "{objects} {nbody}: {got:?}");
}

#[track_caller]
fn refused(objects: &str, nbody: &str, code: &str) {
    assert_eq!(codes(objects, nbody), vec![code.to_string()], "{objects} {nbody}");
}

#[test]
fn orbits_nbody_and_trails_are_valid_at_1_8() {
    valid(&orbit(r#"a="1.496e11" e="0.0167" argp="102.9""#), "");
    valid(&orbit(r#"a="+1.496E11" e="0.3" i="7" raan="10" argp="20" anomaly="30" epoch="-86400""#), "");
    // a hyperbola inside its asymptotes, and an orbit about a centre named by mu alone
    valid(&orbit(r#"a="-0.5e11" e="1.5" anomaly="120""#), "");
    valid(&earth(r#"<orbit around="star" mu="3.986e14" a="7e6"/>"#), "");
    // about a member with a mass, and about a member with a body
    valid(&earth(r#"<orbit around="star" a="7e6"/>"#), r#"<nbody step="150"><member ref="star" mass="2e30"/></nbody>"#);
    valid(&earth(r#"<orbit around="star" a="7e6"/>"#), r#"<nbody step="150"><member ref="star" body="+399"/></nbody>"#);
    valid(&earth(r##"<trail length="86400" samples="2" width="3" color="#FFCC00" fade="false"/>"##), PAIR);
    valid(&earth(r#"<orbit around="sun" a="1e11"/><trail length="86400"/>"#), "");
    // 3600 / 24 = 150 s, a whole number of steps of 150, 75 or 1.5; with a tolerance the step is the first one
    valid(&earth(""), &PAIR.replace(r#"step="150""#, r#"step="1.5""#));
    valid(&earth(""), &PAIR.replace(r#"step="150""#, r#"integrator="ias15" step="7" tolerance="1e-9""#));
    valid(&earth(""), &PAIR.replace(r#"step="150""#, r#"integrator="wisdomHolman" step="75" softening="0""#));
    // the default step of 3600 s divides 3600 s per frame at 1 fps
    assert!(codes_with("1.8", "1", r#"rate="3600""#, &earth(""), r#"<nbody><member ref="earth" mass="1"/></nbody>"#)
        .is_empty());
    // a rational frame rate: 1001 / 30000 × 30000 s
    assert!(codes_with(
        "1.8",
        "30000/1001",
        r#"rate="30000""#,
        &earth(""),
        r#"<nbody step="1.001"><member ref="earth" mass="1"/></nbody>"#
    )
    .is_empty());
    valid(
        &earth(""),
        r#"<nbody step="150" softening="1e9"><member ref="earth" mass="1"/><cluster count="400" mass="2e30" scale="1e12" seed="-42" prefix="c" radius="0.5" vx="1"/></nbody>"#,
    );
}

#[test]
fn or1_an_orbit_needs_a_centre_and_its_gravitational_parameter() {
    // the star has no body and no member entry
    refused(&earth(r#"<orbit around="star" a="7e6"/>"#), "", "OR1");
    // a member without mass whose body has no GM (OR3 for the member, OR1 for the orbit)
    assert_eq!(
        codes(&earth(r#"<orbit around="star" a="7e6"/>"#), r#"<nbody><member ref="star" body="401"/></nbody>"#),
        vec!["OR1", "OR3"]
    );
    // around names the object itself, or something that is not an object3D
    refused(
        r#"<object3D id="earth" primitive="sphere" body="399"><orbit around="earth" a="7e6"/></object3D>"#,
        "",
        "OR1",
    );
    refused(&earth(r#"<orbit around="eye" mu="1" a="7e6"/>"#), "", "OR1");
    // an orbit without around has no centre, even beside an object with no id and a GM
    let got = codes(&format!(r#"<object3D primitive="sphere" body="10"/>{}"#, earth(r#"<orbit a="7e6"/>"#)), "");
    assert!(got.contains(&"OR1".to_string()), "{got:?}");
}

#[test]
fn or2_conics_are_refused_at_the_parabola_beyond_the_asymptotes_and_with_the_wrong_sign() {
    for attrs in [
        r#"a="1e11" e="1""#,
        r#"a="1e11" e="1.0000000000001""#,
        // within 1e-12 of the parabola on the hyperbola's side
        r#"a="-1e11" e="1.0000000000005""#,
        r#"a="-1e11" e="0.5""#,
        r#"a="1e11" e="1.5""#,
        r#"a="-1e11" e="1.5" anomaly="150""#,
        r#"a="-1e11" e="2" anomaly="-121""#,
        r#"a="INF""#,
        r#"a="1e11" anomaly="NaN""#,
        r#"a="1e11" epoch="INF""#,
        r#"a="1e11" e="INF""#,
    ] {
        // (W01 warns of a non-finite number besides)
        let errors: Vec<String> = codes(&orbit(attrs), "").into_iter().filter(|c| c != "W01").collect();
        assert_eq!(errors, vec!["OR2"], "{attrs}");
    }
    // 1 + 2 cos(119°) is positive
    valid(&orbit(r#"a="-1e11" e="2" anomaly="-119""#), "");
}

#[test]
fn or3_an_nbodys_members_and_step() {
    // a repeated member
    // (the object then has two member entries: OR4 as well)
    assert_eq!(
        codes(&earth(""), r#"<nbody step="150"><member ref="earth" mass="1"/><member ref="earth" mass="2"/></nbody>"#),
        vec!["OR3", "OR4"]
    );
    // a member naming a camera
    refused(&earth(""), r#"<nbody step="150"><member ref="eye" mass="1"/></nbody>"#, "OR3");
    // a member with neither a mass nor a body with a GM
    refused(&earth(""), r#"<nbody step="150"><member ref="earth"/></nbody>"#, "OR3");
    // 150 s is not a whole number of steps of 7 s, nor of the default 3600 s
    refused(&earth(""), &PAIR.replace(r#"step="150""#, r#"step="7""#), "OR3");
    refused(&earth(""), &PAIR.replace(r#" step="150""#, ""), "OR3");
    // tolerance without ias15, softening with wisdomHolman
    refused(&earth(""), &PAIR.replace(r#"step="150""#, r#"step="150" tolerance="1e-9""#), "OR3");
    refused(&earth(""), &PAIR.replace(r#"step="150""#, r#"step="150" integrator="wisdomHolman" softening="1""#), "OR3");
    // the default rate of 1 s per second: 1 / 24 s per frame is not a whole number of steps of 150 s
    assert_eq!(codes_with("1.8", "24", "", &earth(""), PAIR), vec!["OR3"]);
    // at a zero rate every frame is at the epoch: zero steps
    assert!(
        codes_with("1.8", "24", r#"rate="0""#, &earth(""), &PAIR.replace(r#"step="150""#, r#"step="7""#)).is_empty()
    );
}

#[test]
fn or4_one_source_of_position() {
    refused(&earth(r#"<orbit around="sun" a="1e11"/><orbit around="sun" a="2e11"/>"#), "", "OR4");
    refused(&earth(r#"<orbit around="sun" a="1e11"/>"#), PAIR, "OR4");
    refused(
        r#"<object3D id="earth" primitive="sphere" body="3" positionFrom="ephemeris"><orbit around="sun" a="1e11"/></object3D>"#,
        "",
        "OR4",
    );
}

#[test]
fn or5_a_trail_follows_an_orbit_or_a_member() {
    refused(&earth(r#"<trail length="10"/>"#), "", "OR5");
    refused(&earth(r#"<trail length="10"/><trail length="20"/>"#), PAIR, "OR5");
    refused(&earth(r#"<trail length="10" samples="1"/>"#), PAIR, "OR5");
}

#[test]
fn malformed_attributes_are_structure_errors() {
    for kids in [
        r#"<orbit around="sun" a="1e11" e="-0.1"/>"#,
        r#"<orbit around="sun" a="1e11" mu="0"/>"#,
        r#"<orbit around="sun"/>"#,
        r#"<orbit a="1e11"/>"#,
        r#"<orbit around="sun" a="x"/>"#,
        r#"<trail/>"#,
        r#"<trail length="0"/>"#,
        r#"<trail length="1" samples="0"/>"#,
        r#"<trail length="1" fade="maybe"/>"#,
    ] {
        let got = codes(&earth(kids), "");
        assert!(got.iter().any(|c| c.starts_with('S')), "{kids}: {got:?}");
    }
    for nbody in [
        r#"<nbody integrator="rk4"><member ref="earth" mass="1"/></nbody>"#,
        r#"<nbody step="0"><member ref="earth" mass="1"/></nbody>"#,
        r#"<nbody/>"#,
        r#"<nbody><member mass="1"/></nbody>"#,
        r#"<nbody><cluster count="1" mass="1" scale="1" prefix="c"/></nbody>"#,
        r#"<nbody><cluster count="4097" mass="1" scale="1" prefix="c"/></nbody>"#,
        r#"<nbody><cluster count="2" mass="1" scale="1"/></nbody>"#,
        r#"<nbody><member ref="earth" mass="1"/></nbody><nbody><member ref="earth" mass="1"/></nbody>"#,
    ] {
        let got = codes(&earth(""), nbody);
        assert!(got.iter().any(|c| c.starts_with('S')), "{nbody}: {got:?}");
    }
}

#[test]
fn v17_gates_orbits_nbody_and_trails_before_1_8() {
    let at17 = codes_at("1.7", &orbit(r#"a="1e11""#), "");
    assert!(at17.contains(&"V17".to_string()), "{at17:?}");
    let at17 = codes_at("1.7", &earth(""), PAIR);
    assert!(at17.contains(&"V17".to_string()), "{at17:?}");
}

#[test]
fn v17_gates_an_orbit_without_a_space() {
    let xml = r#"<scene version="1.7"><project width="64" height="64" fps="24" duration="1"/><composition><camera id="eye" z="-10"/><object3D id="star" primitive="sphere"/><object3D id="earth" primitive="sphere"><orbit around="star" mu="1" a="1"/><trail length="1"/></object3D></composition></scene>"#;
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
    let got: Vec<String> = sr_model::validate_str(xml, &opts).diagnostics.into_iter().map(|d| d.code).collect();
    assert!(got.contains(&"V17".to_string()), "{got:?}");
}
