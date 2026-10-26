//! The numeric table of the physical-sky draft (sr-core srep/srep-0000-physical-sky.md, Conformance), rules 1 to
//! 6, row by row against `sr_space::sun`, the reference module's values at dates the report's example does not
//! reach, and the failure paths of SKY2 and SKY3.

use sr_space::sun::{
    earth_radius_vector, heliocentric_latitude, heliocentric_longitude, julian_day, nutation, parse_datetime,
    refraction_correction, sun_direction, sun_from_document, sun_position, time_scales, DateTime, Observer,
    SunPosition, DEFAULT_DELTA_T,
};
use sr_space::SpaceError;

fn round(x: f64, digits: i32) -> f64 {
    let s = 10f64.powi(digits);
    (x * s).round() / s
}

const EXAMPLE_DATE: DateTime =
    DateTime { year: 2003, month: 10, day: 17, hour: 12, minute: 30, second: 30.0, timezone: -7.0 };

/// The report's example observer: Golden, Colorado.
fn golden() -> Observer {
    Observer {
        latitude: 39.742476,
        longitude: -105.1786,
        altitude: 1830.14,
        pressure: 820.0,
        temperature: 11.0,
        delta_t: Some(67.0),
    }
}

fn example() -> SunPosition {
    sun_position(&EXAMPLE_DATE, &golden(), 67.0)
}

fn jme() -> f64 {
    time_scales(julian_day(&EXAMPLE_DATE), 67.0).jme
}

#[test]
fn the_julian_day_of_the_example_and_of_j2000() {
    assert_eq!(round(julian_day(&EXAMPLE_DATE), 6), 2452930.312847);
    let j2000 = DateTime { year: 2000, month: 1, day: 1, hour: 12, minute: 0, second: 0.0, timezone: 0.0 };
    assert_eq!(julian_day(&j2000), 2451545.0);
}

#[test]
fn a_date_parses_into_its_fields_and_offset() {
    assert_eq!(parse_datetime("2003-10-17T12:30:30-07:00").unwrap(), EXAMPLE_DATE);
    let z = parse_datetime("-1500-03-21T06:00:00.25Z").unwrap();
    assert_eq!((z.year, z.second, z.timezone), (-1500, 0.25, 0.0));
    assert_eq!(parse_datetime("2024-01-01T00:00:00+05:30").unwrap().timezone, 5.5);
}

#[test]
fn a_date_out_of_its_form_is_refused() {
    for bad in [
        "",
        "2003-10-17",
        "2003-10-17T12:30:30",
        "2003-10-17 12:30:30Z",
        "2003-13-17T12:30:30Z",
        "2003-10-32T12:30:30Z",
        "2003-10-17T24:00:00Z",
        "2003-10-17T12:60:00Z",
        "2003-10-17T12:30:61Z",
        "2003-10-17T12:30:30+0700",
        "03-10-17T12:30:30Z",
        "2003-10-17T12:30:30Z trailing",
    ] {
        assert!(matches!(parse_datetime(bad), Err(SpaceError::Sky(_))), "{bad:?}");
    }
}

#[test]
fn the_earth_terms_of_the_example() {
    assert_eq!(round(heliocentric_longitude(jme()), 7), 24.0182617);
    assert_eq!(round(heliocentric_latitude(jme()), 9), -0.000101122);
    assert_eq!(round(earth_radius_vector(jme()), 9), 0.996542297);
    let jce = time_scales(julian_day(&EXAMPLE_DATE), 67.0).jce;
    let (dpsi, deps) = nutation(jce);
    assert_eq!((round(dpsi, 8), round(deps, 8)), (-0.0039984, 0.00166657));
}

#[test]
fn every_intermediate_of_the_example() {
    let s = example();
    assert_eq!(round(s.obliquity, 6), 23.440465);
    assert_eq!(round(s.apparent_longitude, 7), 204.0085519);
    assert_eq!(round(s.right_ascension, 5), 202.22741);
    assert_eq!(round(s.declination, 5), -9.31434);
    assert_eq!(round(s.hour_angle, 6), 11.105902);
    assert_eq!(
        (
            round(s.topocentric_right_ascension, 5),
            round(s.topocentric_declination, 5),
            round(s.topocentric_hour_angle, 5)
        ),
        (202.22704, -9.31618, 11.10627)
    );
    assert_eq!(round(s.zenith, 5), 50.11162);
    assert_eq!(round(s.azimuth, 5), 194.34024);
    assert_eq!(round(s.incidence(30.0, -10.0), 5), 25.187);
}

#[test]
fn the_report_s_printed_example_within_a_thousandth_of_a_degree() {
    let s = example();
    assert!((s.zenith - 50.11162).abs() <= 1e-3);
    assert!((s.azimuth - 194.34024).abs() <= 1e-3);
    assert!((s.incidence(30.0, -10.0) - 25.18700).abs() <= 1e-3);
}

#[test]
fn refraction_scales_with_the_air_and_stops_below_the_limb() {
    assert_eq!(round(refraction_correction(89.0, 1013.25, 10.0), 6), 0.363564);
    assert_eq!(refraction_correction(89.0, 0.0, 10.0), 0.0);
    // the sun's top limb is 0.26667° + 0.5667° below the horizon at this zenith
    assert!(refraction_correction(90.0 + 0.26667 + 0.5667 - 1e-6, 1013.25, 10.0) > 0.0);
    assert_eq!(refraction_correction(90.0 + 0.26667 + 0.5667 + 1e-6, 1013.25, 10.0), 0.0);
    let thin = refraction_correction(80.0, 506.625, 10.0);
    assert!((thin * 2.0 - refraction_correction(80.0, 1013.25, 10.0)).abs() < 1e-15);
}

#[test]
fn the_sun_in_scene_axes() {
    let d = sun_direction(120.0, 0.0);
    assert_eq!(d.map(|c| round(c, 12)), [0.0, 0.5, 0.866025403784]);
    // up is -y, azimuth 90° is +x
    let up = sun_direction(0.0, 37.0);
    assert!((up[1] + 1.0).abs() < 1e-15 && up[0].abs() < 1e-15 && up[2].abs() < 1e-15);
    let east = sun_direction(90.0, 90.0);
    assert!((east[0] - 1.0).abs() < 1e-15 && east[1].abs() < 1e-15);
}

/// sr-core physical_sky_reference.sun_from_document at four dates: the Julian calendar, a year before 0, the
/// pole at the solstice with the default deltaT, and the far future south of the antarctic circle.
#[test]
fn the_reference_module_s_values_away_from_the_example() {
    struct Case {
        date: &'static str,
        observer: Observer,
        values: [f64; 17],
        direction: [f64; 3],
    }
    let observer = |latitude, longitude, altitude, pressure, temperature, delta_t| Observer {
        latitude,
        longitude,
        altitude,
        pressure,
        temperature,
        delta_t,
    };
    let cases = [
        Case {
            date: "1066-10-14T09:00:00Z",
            observer: observer(50.9, 0.5, 0.0, 1013.25, 10.0, Some(1200.0)),
            values: [
                2110700.875,
                26.706350685970392,
                0.00019257094288243127,
                0.9913119569399093,
                -0.0007973599278876774,
                -0.002480596244118361,
                23.557692419368408,
                206.69981183258903,
                163.3795690044664,
                204.75091691757402,
                -10.345428277560778,
                319.1286520868924,
                204.75195278226258,
                -10.34751219346219,
                319.1276162222038,
                70.69873504928492,
                137.00989293848352,
            ],
            direction: [0.6435465339003503, -0.3305352294499514, -0.6903581105464853],
        },
        Case {
            date: "-1500-03-21T06:00:00+02:00",
            observer: observer(29.97, 31.13, 20.0, 1000.0, 25.0, Some(7000.0)),
            values: [
                1173262.6666666667,
                167.02830541692674,
                0.00026439181862198145,
                1.0097665317187134,
                -0.0030153510727413957,
                -0.0017133310493975284,
                23.868907556316724,
                347.0196535043991,
                225.31889233062734,
                348.09684339047857,
                -5.215062133030483,
                268.3520489401488,
                348.09894867406217,
                -5.216253066355071,
                268.3499436565652,
                94.02878783747714,
                93.70350410290462,
            ],
            direction: [0.995445705978132, 0.07025768354178726, -0.06443372062081078],
        },
        Case {
            date: "2024-06-21T00:00:00Z",
            observer: observer(89.9, 0.0, 0.0, 1013.25, -20.0, None),
            values: [
                2460482.5,
                270.13185044631064,
                0.00017741138820831162,
                1.0162033903444678,
                -0.0009915486371344252,
                0.0022934669541270457,
                23.4384029108259,
                90.12525803945438,
                269.6830680797232,
                90.13652255139392,
                23.438166141125087,
                179.54654552832932,
                90.13652251508242,
                23.435966340795872,
                179.5465455646408,
                66.62056791176562,
                359.54688768312326,
            ],
            direction: [-0.007258931901205202, -0.3968184115339803, 0.9178684307542678],
        },
        Case {
            date: "5500-12-31T23:59:59-11:30",
            observer: observer(-77.85, 166.67, 2800.0, 700.0, -30.0, Some(4000.0)),
            values: [
                3730258.9791550925,
                100.39515073038638,
                7.433457838464143e-05,
                0.9930619084638377,
                -0.004774124276144946,
                -0.00032232419816160597,
                23.00693842900808,
                280.38464523018837,
                274.4922112560513,
                281.26000261857075,
                -22.609085728151207,
                159.9022086374805,
                281.25980920269427,
                -22.60668497033564,
                159.90240205335698,
                78.7874109955585,
                198.86368697786486,
            ],
            direction: [-0.31714639942782824, -0.1944498815815823, -0.9282280995977622],
        },
    ];
    for c in cases {
        let (s, d) = sun_from_document(c.date, &c.observer).unwrap();
        let got = [
            s.jd,
            s.heliocentric_longitude,
            s.heliocentric_latitude,
            s.radius,
            s.nutation_longitude,
            s.nutation_obliquity,
            s.obliquity,
            s.apparent_longitude,
            s.sidereal_time,
            s.right_ascension,
            s.declination,
            s.hour_angle,
            s.topocentric_right_ascension,
            s.topocentric_declination,
            s.topocentric_hour_angle,
            s.zenith,
            s.azimuth,
        ];
        for (i, (g, e)) in got.iter().zip(c.values).enumerate() {
            assert!((g - e).abs() <= 1e-9 * e.abs().max(1.0), "{}: value {i} is {g}, the reference {e}", c.date);
        }
        // the angles agree to rounding, a few 1e-12 rad: the long series sum terms of up to 6e11
        for (g, e) in d.iter().zip(c.direction) {
            assert!((g - e).abs() <= 1e-11, "{}: direction {d:?}, the reference {:?}", c.date, c.direction);
        }
    }
}

#[test]
fn deltat_defaults_within_1900_to_2100_and_is_required_outside() {
    let mut o = golden();
    o.delta_t = None;
    let (s, _) = sun_from_document("2003-10-17T12:30:30-07:00", &o).unwrap();
    let t = time_scales(s.jd, DEFAULT_DELTA_T);
    assert_eq!(s.jde, t.jde);
    assert_eq!(DEFAULT_DELTA_T, 69.0);
    for date in ["1899-12-31T23:59:59Z", "2101-01-01T00:00:00Z"] {
        assert!(matches!(sun_from_document(date, &o), Err(SpaceError::Sky(m)) if m.contains("deltaT")), "{date}");
    }
    assert!(sun_from_document("1900-01-01T00:00:00Z", &o).is_ok());
    assert!(sun_from_document("2100-12-31T23:59:59Z", &o).is_ok());
}

#[test]
fn an_observer_out_of_sky3_or_a_year_out_of_sky2_is_refused() {
    let base = Observer { delta_t: Some(67.0), ..golden() };
    let bad = [
        ("2003-10-17T12:30:30Z", Observer { latitude: 90.0001, ..base }),
        ("2003-10-17T12:30:30Z", Observer { latitude: -90.0001, ..base }),
        ("2003-10-17T12:30:30Z", Observer { longitude: 180.0001, ..base }),
        ("2003-10-17T12:30:30Z", Observer { longitude: f64::NAN, ..base }),
        ("2003-10-17T12:30:30Z", Observer { pressure: -1.0, ..base }),
        ("2003-10-17T12:30:30Z", Observer { pressure: f64::INFINITY, ..base }),
        ("2003-10-17T12:30:30Z", Observer { temperature: -273.0, ..base }),
        ("2003-10-17T12:30:30Z", Observer { temperature: f64::NAN, ..base }),
        ("2003-10-17T12:30:30Z", Observer { altitude: f64::INFINITY, ..base }),
        ("2003-10-17T12:30:30Z", Observer { delta_t: Some(8000.1), ..base }),
        ("2003-10-17T12:30:30Z", Observer { delta_t: Some(f64::NAN), ..base }),
        ("-2001-01-01T00:00:00Z", base),
        ("6001-01-01T00:00:00Z", base),
        ("2003-10-17", base),
    ];
    for (date, o) in bad {
        assert!(matches!(sun_from_document(date, &o), Err(SpaceError::Sky(_))), "{date} {o:?}");
    }
    for (date, o) in [
        ("-2000-01-01T00:00:00Z", base),
        ("6000-12-31T23:59:59Z", base),
        ("2003-10-17T12:30:30Z", Observer { latitude: 90.0, longitude: -180.0, pressure: 0.0, ..base }),
        ("2003-10-17T12:30:30Z", Observer { delta_t: Some(-8000.0), ..base }),
    ] {
        assert!(sun_from_document(date, &o).is_ok(), "{date} {o:?}");
    }
}

#[test]
fn the_sun_s_direction_is_its_zenith_and_azimuth() {
    let (s, d) = sun_from_document("2003-10-17T12:30:30-07:00", &golden()).unwrap();
    assert_eq!(d, sun_direction(s.zenith, s.azimuth));
    assert!((d.iter().map(|c| c * c).sum::<f64>() - 1.0).abs() < 1e-15);
}
