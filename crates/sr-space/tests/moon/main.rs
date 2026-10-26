//! Rule 7 of the physical-sky draft (sr-core srep/srep-0000-physical-sky.md): the moon's rows of the draft's
//! numeric table against `sr_space::moon`, the reference module's values at five dates and observers, and a total
//! solar eclipse, where the moon covers the sun.

use sr_space::moon::{moon_from_document, moon_position, MOON_RADIUS_KM};
use sr_space::sun::{sun_from_document, Observer, SUN_RADIUS_DEG};
use sr_space::SpaceError;

fn round(x: f64, digits: i32) -> f64 {
    let s = 10f64.powi(digits);
    (x * s).round() / s
}

fn at(latitude: f64, longitude: f64, altitude: f64, pressure: f64, temperature: f64, delta_t: Option<f64>) -> Observer {
    Observer { latitude, longitude, altitude, pressure, temperature, delta_t }
}

fn equator() -> Observer {
    at(0.0, 0.0, 0.0, 1013.25, 10.0, None)
}

#[test]
fn the_series_of_montenbruck_and_pfleger_at_meeus_example() {
    let m = moon_position(2448724.5);
    assert_eq!(round(m.longitude, 4), 133.1571);
    assert_eq!(round(m.latitude, 4), -3.2268);
    assert_eq!(round(m.distance, 1), 368074.2);
    // Meeus' example 47.a, the full theory: within the series' accuracy
    assert!((m.longitude - 133.162655).abs() <= 0.05);
    assert!((m.latitude + 3.229126).abs() <= 0.02);
    assert!((m.distance - 368409.7).abs() <= 3700.0);
}

#[test]
fn the_moon_s_diameter_and_phase_in_the_draft_s_rows() {
    let golden = at(39.742476, -105.1786, 1830.14, 1013.25, 10.0, Some(67.0));
    let (m, _) = moon_from_document("2003-10-17T12:30:30-07:00", &golden).unwrap();
    assert_eq!(round(m.angular_diameter, 4), 0.4991);
    let (full, _) = moon_from_document("2003-11-09T01:13:00Z", &equator()).unwrap();
    assert_eq!(round(full.illuminated, 4), 1.0);
    let (new, _) = moon_from_document("2003-10-25T12:50:00Z", &equator()).unwrap();
    assert_eq!(round(new.illuminated, 4), 0.0002);
}

/// sr-core physical_sky_reference.moon_from_document: zenith, azimuth, distance, angular diameter, illuminated
/// fraction, phase angle and direction.
#[test]
fn the_reference_module_s_moon_at_five_dates() {
    let cases: [(&str, Observer, [f64; 6], [f64; 3]); 5] = [
        (
            "2003-10-17T12:30:30-07:00",
            at(39.742476, -105.1786, 1830.14, 820.0, 11.0, Some(67.0)),
            [
                84.7348809452794,
                300.0691493704996,
                398871.4864060659,
                0.49913822120502366,
                0.5710084631251169,
                81.83542607236396,
            ],
            [-0.8617699001864847, -0.09176438577501121, 0.49893079343322877],
        ),
        (
            "2003-11-09T01:13:00Z",
            equator(),
            [
                27.71667935356424,
                307.7480276502013,
                405321.6630304071,
                0.4911950248292284,
                0.9999883924889732,
                0.39041187657618254,
            ],
            [-0.36775934272231675, -0.8852582682409651, 0.28472945817294193],
        ),
        (
            "2003-10-25T12:50:00Z",
            equator(),
            [
                19.290966695199856,
                236.08076077627726,
                359726.6813460174,
                0.5534539388103916,
                0.00022257110276746594,
                178.29036606994052,
            ],
            [-0.274145609038411, -0.9438530490532874, -0.18435185607357313],
        ),
        (
            "2024-04-08T18:18:00Z",
            at(25.29, -104.14, 1500.0, 850.0, 20.0, None),
            [
                20.123179523290204,
                149.7733600684316,
                360014.9823733215,
                0.5530107278715395,
                9.258556564584186e-06,
                179.65132168358627,
            ],
            [0.17319700700031845, -0.9389551446261319, -0.29726424801218754],
        ),
        (
            "1969-07-20T20:17:40Z",
            at(0.674, 23.47, 0.0, 1013.25, 10.0, None),
            [
                80.54373483521876,
                265.4293463641146,
                389932.6982597899,
                0.5105805035017902,
                0.3297839223092828,
                109.90320549049282,
            ],
            [-0.983274337118068, -0.1642947094104327, -0.0786055114146926],
        ),
    ];
    for (date, o, values, direction) in cases {
        let (m, d) = moon_from_document(date, &o).unwrap();
        let got = [m.zenith, m.azimuth, m.distance, m.angular_diameter, m.illuminated, m.phase_angle];
        for (i, (g, e)) in got.iter().zip(values).enumerate() {
            assert!((g - e).abs() <= 1e-9 * e.abs().max(1.0), "{date}: value {i} is {g}, the reference {e}");
        }
        for (g, e) in d.iter().zip(direction) {
            assert!((g - e).abs() <= 1e-11, "{date}: direction {d:?}, the reference {direction:?}");
        }
    }
}

/// 2024-04-08 near Nazas, Durango, inside the path of totality: the moon's disc, larger than the sun's, covers it.
#[test]
fn the_moon_covers_the_sun_in_a_total_eclipse() {
    let o = at(25.29, -104.14, 1500.0, 850.0, 20.0, None);
    let (m, md) = moon_from_document("2024-04-08T18:18:00Z", &o).unwrap();
    let (_, sd) = sun_from_document("2024-04-08T18:18:00Z", &o).unwrap();
    let separation = md.iter().zip(sd).map(|(a, b)| a * b).sum::<f64>().clamp(-1.0, 1.0).acos().to_degrees();
    assert!(separation < 0.1, "the centres are {separation}° apart");
    assert!(m.angular_diameter / 2.0 > SUN_RADIUS_DEG);
    assert!(m.illuminated < 1e-4, "the near side is dark: {}", m.illuminated);
    assert_eq!(MOON_RADIUS_KM, 1737.4);
}

#[test]
fn the_moon_s_observer_is_checked_as_the_sun_s() {
    for (date, o) in [
        ("2003-10-17T12:30:30Z", at(91.0, 0.0, 0.0, 1013.25, 10.0, None)),
        ("1800-01-01T00:00:00Z", equator()),
        ("2003-10-17", equator()),
    ] {
        assert!(matches!(moon_from_document(date, &o), Err(SpaceError::Sky(_))), "{date}");
    }
}
