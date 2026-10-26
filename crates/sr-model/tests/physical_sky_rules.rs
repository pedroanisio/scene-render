//! The physical-sky draft (schema/UPSTREAM): `sky="physical"` on a dome light and its attributes, the Schematron
//! rules PSKY2 to PSKY4 and their clause of V17, and W18 (the draft's PSKY1) for the physical sky's attributes written
//! under another sky.

const GOLDEN: &str = r#"date="2003-10-17T12:30:30-07:00" latitude="39.742476" longitude="-105.1786" altitude="1830.14" pressure="820" temperature="11" deltaT="67""#;

fn codes_at(version: &str, attrs: &str) -> Vec<String> {
    let xml = format!(
        r#"<scene version="{version}"><project width="64" height="64" fps="24" duration="1"/><composition/><lights><light id="sky" type="dome" {attrs}/></lights></scene>"#
    );
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
    sr_model::validate_str(&xml, &opts).diagnostics.into_iter().map(|d| d.code).collect()
}

fn codes(attrs: &str) -> Vec<String> {
    codes_at("1.8", attrs)
}

/// The physical sky of the report's example with `date` replaced.
fn dated(date: &str, rest: &str) -> String {
    format!(r#"sky="physical" date="{date}" {rest}"#)
}

#[track_caller]
fn valid(attrs: &str) {
    let got = codes(attrs);
    assert!(got.is_empty(), "{attrs}: {got:?}");
}

#[track_caller]
fn refused(attrs: &str, code: &str) {
    let got = codes(attrs);
    assert_eq!(got, vec![code.to_string()], "{attrs}");
}

#[test]
fn physical_skies_are_valid_at_1_8() {
    valid(&format!(r#"sky="physical" {GOLDEN}"#));
    valid(r#"sky="physical" date="2024-04-08T18:17:00Z""#);
    // a fraction of a second, a leap second, the last day of the free deltaT range
    valid(r#"sky="physical" date="2100-12-31T23:59:60.25Z""#);
    // the SPA's first year with its deltaT, a + offset, signed doubles
    valid(r#"sky="physical" date="-2000-03-21T06:00:00+05:30" deltaT="+8000" latitude="+90" longitude="-180""#);
    valid(
        r##"sky="physical" date="1900-01-01T00:00:00Z" temperature="-272.9" groundAlbedo="0" sunIntensity="2" moon="false" moonIntensity="0.5" atmosphere="none" aerosol="2" ozone="0" planetRadius="3389500" atmosphereHeight="11.1e3" sunIrradiance="#FFE0C0FF" aerialPerspective="false" fogDensity="0.001" fogHeight="250""##,
    );
}

#[test]
fn v17_gates_the_physical_sky() {
    let got = codes_at("1.7", &format!(r#"sky="physical" {GOLDEN}"#));
    assert_eq!(got, vec!["V17".to_string()]);
    // the attributes alone, under a gradient, are not 1.8 syntax the gate refuses: W18 says they are ignored
    let got = codes_at("1.7", &format!(r#"sky="gradient" {GOLDEN}"#));
    assert_eq!(got, vec!["W18".to_string()]);
}

#[test]
fn psky2_reads_the_date_as_rule_1_does() {
    let rest = r#"deltaT="67""#;
    refused(r#"sky="physical""#, "PSKY2");
    for bad in [
        "2003-10-17T12:30-07:00",
        "2003-10-17T12:30:30",
        "2003-10-17T12:30:30-0700",
        "2003-10-17T12:30:30.Z",
        "2003-10-17T12:30:30.5.Z",
        "2003-10-17 12:30:30Z",
        "03-10-17T12:30:30Z",
        "+2003-10-17T12:30:30Z",
        "2003-13-17T12:30:30Z",
        "2003-00-17T12:30:30Z",
        "2003-10-32T12:30:30Z",
        "2003-10-00T12:30:30Z",
        "2003-10-17T24:30:30Z",
        "2003-10-17T12:60:30Z",
        "2003-10-17T12:30:61Z",
        "6001-10-17T12:30:30Z",
        "-2001-10-17T12:30:30Z",
        "",
    ] {
        refused(&dated(bad, rest), "PSKY2");
    }
    // deltaT is required outside 1900 to 2100
    refused(r#"sky="physical" date="1899-12-31T23:59:59Z""#, "PSKY2");
    refused(r#"sky="physical" date="2101-01-01T00:00:00Z""#, "PSKY2");
    valid(r#"sky="physical" date="1899-12-31T23:59:59Z" deltaT="-2.5""#);
}

#[test]
fn psky3_bounds_the_observer_and_the_ground() {
    let base = r#"sky="physical" date="2003-10-17T12:30:30Z""#;
    for bad in [
        r#"latitude="90.5""#,
        r#"latitude="-91""#,
        r#"longitude="180.01""#,
        r#"longitude="-181""#,
        r#"temperature="-273""#,
        r#"deltaT="8000.5""#,
        r#"deltaT="-8001""#,
        r#"groundAlbedo="1.25""#,
        r#"groundAlbedo="-0.1""#,
    ] {
        refused(&format!("{base} {bad}"), "PSKY3");
    }
    // NaN is an xs:double the structure warns of (W01) and outside every range
    assert_eq!(codes(&format!(r#"{base} latitude="NaN""#)), vec!["PSKY3".to_string(), "W01".to_string()]);
}

#[test]
fn psky4_refuses_a_hand_placed_sun() {
    let base = r#"sky="physical" date="2003-10-17T12:30:30Z""#;
    refused(&format!(r#"{base} sunElevation="30""#), "PSKY4");
    refused(&format!(r#"{base} sunAzimuth="180""#), "PSKY4");
    // SREP 71's disc attributes keep their meaning
    valid(&format!(r##"{base} sunSize="0.6" sunColor="#FFEEDDFF" sunIntensity="0""##));
}

#[test]
fn w18_names_the_attributes_another_sky_ignores() {
    for sky in ["none", "gradient"] {
        let got = codes(&format!(r#"sky="{sky}" date="2003-10-17T12:30:30Z" fogDensity="0.1""#));
        assert_eq!(got, vec!["W18".to_string()], "{sky}");
    }
    // no sky written is the default none
    assert_eq!(codes(r#"moon="false""#), vec!["W18".to_string()]);
    // none of them, no warning
    valid(r#"sky="gradient" sunElevation="10""#);
    let d = sr_model::validate_str(
        r#"<scene version="1.8"><project width="64" height="64" fps="24" duration="1"/><composition/><lights><light id="sky" type="dome" sky="gradient" latitude="10" fogHeight="5"/></lights></scene>"#,
        &sr_model::LoadOptions { verify_assets: false, base_dir: None },
    );
    let w = d.diagnostics.iter().find(|d| d.code == "W18").unwrap();
    assert!(w.message.contains("latitude") && w.message.contains("fogHeight"), "{}", w.message);
}

#[test]
fn the_model_reads_the_physical_sky() {
    let xml = format!(
        r#"<scene version="1.8"><project width="64" height="64" fps="24" duration="1"/><composition/><lights><light id="sky" type="dome" sky="physical" {GOLDEN} atmosphere="none"/></lights></scene>"#
    );
    let d = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let l = &d.scene.lights.as_ref().unwrap().lights[0];
    assert_eq!(l.sky, sr_model::model::Sky::Physical);
    assert_eq!(l.date.as_deref(), Some("2003-10-17T12:30:30-07:00"));
    assert_eq!((l.latitude, l.longitude, l.altitude, l.temperature), (39.742476, -105.1786, 1830.14, 11.0));
    assert_eq!(l.pressure.get(), 820.0);
    assert_eq!(l.delta_t, Some(67.0));
    assert!(l.moon && l.aerial_perspective);
    assert_eq!((l.moon_intensity.get(), l.aerosol.get(), l.ozone.get()), (1.0, 1.0, 1.0));
    assert_eq!((l.planet_radius.get(), l.atmosphere_height.get(), l.ground_albedo), (6360000.0, 100000.0, 0.3));
    assert_eq!((l.fog_density.get(), l.fog_height.get()), (0.0, 100.0));
}

#[test]
fn an_absent_sun_intensity_stays_absent() {
    // the draft's schema diff gives sunIntensity no default: absent reads as 0 under none and gradient and as 1 under
    // physical, which only the renderer knows
    let load = |attrs: &str| {
        let xml = format!(
            r#"<scene version="1.8"><project width="64" height="64" fps="24" duration="1"/><composition/><lights><light id="sky" type="dome" {attrs}/></lights></scene>"#
        );
        sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap().scene.lights.unwrap().lights[0]
            .sun_intensity
            .map(|v| v.get())
    };
    assert_eq!(load(&format!(r#"sky="physical" {GOLDEN}"#)), None);
    assert_eq!(load(r#"sky="gradient""#), None);
    assert_eq!(load(r#"sky="gradient" sunIntensity="0""#), Some(0.0));
}
