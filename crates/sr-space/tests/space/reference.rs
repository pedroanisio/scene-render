//! The draft's numeric table. Floats are rounded as the table's expression rounds them.

use sr_space::{
    apparent_position, approximate_position, approximate_relative, body_fixed_from_icrf, calendar, chebyshev,
    chebyshev_derivative, determinant, ecliptic_from_icrf, element_table, et_from_utc, julian_day, leap_seconds,
    mat_apply, parse_utc, place_at, place_body, pole_icrf, rotation_elements, solve_kepler, tdb_minus_tt, ElementTable,
    Frame, Positions, Space, Spk, TimeScale, LEAP_SECONDS, ROTATION_EARTH_W1, SCENE_FROM_ICRF_AXES, TT_MINUS_TAI,
};

pub const EXCERPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/space/fixtures/de440s-excerpt.bsp");
pub const EXCERPT_DIGEST: &str = "d113bb5117946244cf63de9d06287d5aac008acd1a561d28b760bcc7d4c3d1fd";
const HORIZONS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/space/fixtures/horizons-vectors.json");
const J2000: f64 = 2451545.0;

pub fn excerpt() -> Spk {
    Spk::open(std::path::Path::new(EXCERPT), Some(&format!("sha256:{EXCERPT_DIGEST}"))).expect("the kit's excerpt")
}

fn round(v: f64, d: i32) -> f64 {
    let s = 10f64.powi(d);
    (v * s).round() / s
}

fn dist(a: [f64; 3]) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

fn angle_deg(a: [f64; 3], b: [f64; 3]) -> f64 {
    let c = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2]) / (dist(a) * dist(b));
    c.clamp(-1.0, 1.0).acos().to_degrees()
}

fn utc(epoch: &str, rate: f64) -> Space {
    Space::new(epoch, TimeScale::Utc, rate, 0, Frame::Icrf).unwrap()
}

#[test]
fn calendar_rows() {
    assert_eq!(julian_day(2000, 1, 1, 12, 0, 0.0), 2451545.0);
    assert_eq!(calendar(2451545.0), (2000, 1, 1, 12, 0, 0.0));
    assert_eq!(julian_day(1972, 1, 1, 0, 0, 0.0), 2441317.5);
    assert_eq!(parse_utc("2003-10-17T12:30:30-07:00").unwrap(), (2003, 10, 17, 19, 30, 30.0));
}

#[test]
fn time_scale_rows() {
    assert_eq!(leap_seconds(julian_day(1972, 1, 1, 0, 0, 0.0)).unwrap(), 10);
    assert_eq!(leap_seconds(julian_day(2017, 1, 1, 0, 0, 0.0)).unwrap(), 37);
    assert_eq!(LEAP_SECONDS.len(), 28);
    assert_eq!(TT_MINUS_TAI, 32.184);
    assert_eq!(round(tdb_minus_tt(J2000), 9), -7.2737e-05);
    let worst = (0..366).step_by(5).map(|d| tdb_minus_tt(J2000 + d as f64).abs()).fold(0.0, f64::max);
    assert!(worst < 0.0017, "{worst}");
    let et = et_from_utc("2000-01-01T11:58:55.816Z").unwrap();
    assert_eq!(round(et - tdb_minus_tt(J2000), 6), 0.0);
}

#[test]
fn frame_clock_rows() {
    let s = utc("2026-10-24T00:00:00Z", 60.0);
    assert_eq!(s.frame_time(24, 24.0).unwrap() - s.frame_time(0, 24.0).unwrap(), 60.0);
    let frozen = Space::new("2451545.0", TimeScale::Tdb, 0.0, 0, Frame::Icrf).unwrap();
    assert_eq!(frozen.frame_time(500, 24.0).unwrap(), 0.0);
}

#[test]
fn excerpt_rows() {
    let spk = excerpt();
    assert_eq!(spk.digest_hex(), EXCERPT_DIGEST);
    assert_eq!((spk.id.as_str(), spk.format.as_str(), spk.nd, spk.ni), ("DAF/SPK", "LTL-IEEE", 2, 6));
    assert_eq!(spk.segments.len(), 56);
    let mut bodies: Vec<i32> = spk.segments.iter().map(|s| s.target).collect();
    bodies.sort_unstable();
    bodies.dedup();
    assert_eq!(bodies, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 199, 299, 301, 399]);
}

#[test]
fn chebyshev_rows() {
    assert_eq!(round(chebyshev(&[1.0, -0.5, 0.25], 0.5).unwrap(), 12), 0.625);
    assert_eq!(round(chebyshev_derivative(&[1.0, -0.5, 0.25], 0.5).unwrap(), 12), 0.0);
}

#[test]
fn position_rows() {
    let spk = excerpt();
    let p = spk.position(399, 0, 0.0).unwrap();
    assert_eq!(p.map(|x| round(x, 3)), [-27566740.483, 132361381.154, 57418653.286]);
    assert_eq!(round(dist(spk.position(301, 399, 0.0).unwrap()), 3), 402448.64);
    assert_eq!(round(dist(spk.velocity(399, 0, 0.0).unwrap()), 6), 30.285256);
    assert_eq!(round(apparent_position(&spk, 10, 399, 0.0).unwrap().1, 3), 490.685);
}

#[test]
fn horizons_rows() {
    let spk = excerpt();
    let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(HORIZONS).unwrap()).unwrap();
    let mut worst = 0.0f64;
    let mut rows = 0;
    for (body, list) in doc["bodies"].as_object().unwrap() {
        let target = if body == "499" { 4 } else { body.parse::<i32>().unwrap() };
        for r in list.as_array().unwrap() {
            let f = |k: &str| r[k].as_f64().unwrap();
            let p = spk.position(target, 0, (f("jd") - J2000) * 86400.0).unwrap();
            worst = worst.max(dist([p[0] - f("x"), p[1] - f("y"), p[2] - f("z")]));
            rows += 1;
        }
    }
    assert_eq!(rows, 56);
    assert_eq!(round(worst, 6), 0.003229);
    // the draft's horizons test: within 1 km, and within 10 m
    assert!(worst < 0.01);
}

fn elements_error_arcsec(spk: &Spk, body: i32, et: f64, table: ElementTable) -> f64 {
    3600.0 * angle_deg(approximate_position(body, et, table).unwrap(), spk.position(body, 10, et).unwrap())
}

#[test]
fn element_rows() {
    let spk = excerpt();
    assert_eq!(round(solve_kepler(45.0, 0.2).unwrap(), 9), 54.306556927);
    let em = elements_error_arcsec(&spk, 3, 0.0, ElementTable::Y1800To2050);
    assert_eq!(round(em, 3), 2.76);
    let mars = elements_error_arcsec(&spk, 4, (2460977.5 - J2000) * 86400.0, ElementTable::Y1800To2050);
    assert_eq!(round(mars, 3), 12.933);
    let jupiter = elements_error_arcsec(&spk, 5, 0.0, ElementTable::Bc3000ToAd3000);
    assert_eq!(round(jupiter, 3), 485.963);
    assert!(em < 60.0 && jupiter < 1800.0);
}

#[test]
fn approximate_relative_rows() {
    // Mars about the Earth-Moon barycentre at J2000, against the excerpt's position(4, 3)
    let p = approximate_relative(4, 3, 0.0).unwrap();
    assert_eq!(round(dist(p), 3), 276691116.515);
    let e = excerpt().position(4, 3, 0.0).unwrap();
    // the reference's row: 6933 km nearer than the excerpt's distance, 0.9″ from its direction
    assert_eq!(round(dist(e) - dist(p), 0), 6933.0);
    assert_eq!(round(3600.0 * angle_deg(p, e), 1), 0.9);
    // the Sun and the barycentre are the elements' origin; a planet about itself is at zero
    assert_eq!(
        approximate_relative(4, 0, 0.0).unwrap(),
        approximate_position(4, 0.0, ElementTable::Y1800To2050).unwrap()
    );
    assert_eq!(approximate_relative(4, 10, 0.0).unwrap(), approximate_relative(4, 0, 0.0).unwrap());
    assert_eq!(approximate_relative(10, 0, 0.0).unwrap(), [0.0; 3]);
    assert_eq!(approximate_relative(4, 4, 0.0).unwrap(), [0.0; 3]);
    // Table 1 inside 1800 to 2050, the long tables elsewhere to 3000, an error beyond
    let year = |y: f64| (y - 2000.0) * 365.25 * 86400.0;
    assert_eq!(element_table(0.0).unwrap(), ElementTable::Y1800To2050);
    assert_eq!(element_table(year(2100.0)).unwrap(), ElementTable::Bc3000ToAd3000);
    assert_eq!(element_table(year(1700.0)).unwrap(), ElementTable::Bc3000ToAd3000);
    assert!(element_table(year(3100.0)).is_err());
    assert_eq!(
        approximate_relative(5, 0, year(2100.0)).unwrap(),
        approximate_position(5, year(2100.0), ElementTable::Bc3000ToAd3000).unwrap()
    );
    for (body, origin) in [(9, 0), (301, 0), (4, 399), (4, 9)] {
        assert!(approximate_relative(body, origin, 0.0).is_err(), "{body} about {origin}");
    }
}

#[test]
fn frame_rows() {
    let (ra, dec, w) = rotation_elements(399, 0.0).unwrap();
    assert_eq!((round(ra, 6), round(dec, 6), round(w, 6)), (0.0, 90.0, 190.147));
    assert_eq!(round(determinant(&body_fixed_from_icrf(399, 12345.0).unwrap()), 12), 1.0);
    assert_eq!(pole_icrf(301, 0.0).unwrap().map(|x| round(x, 6)), [-0.022609, -0.411831, 0.91098]);
    assert_eq!(round(360.0 / ROTATION_EARTH_W1 * 86400.0, 4), 86164.0962);
    let pole = mat_apply(&ecliptic_from_icrf(), [0.0, 0.0, 1.0]);
    assert_eq!(pole.map(|x| round(x, 9)), [0.0, 0.397776978, 0.917482139]);
}

#[test]
fn scene_rows() {
    let s = Space::new("2451545.0", TimeScale::Tdb, 1.0, 0, Frame::Icrf).unwrap();
    let got = s.scene_from_icrf([1.0, 2.0, 3.0], [0.0; 3], 1e-6, 0.0).unwrap();
    for (g, w) in got.iter().zip([0.001, -0.003, 0.002]) {
        assert!((g - w).abs() < 1e-15, "{got:?}");
    }
    assert_eq!(round(determinant(&SCENE_FROM_ICRF_AXES), 12), 1.0);

    let spk = excerpt();
    let fixed = Space::new("2451545.0", TimeScale::Tdb, 1.0, 399, Frame::BodyFixed(399)).unwrap();
    let [x, _, z] = place_body(&spk, &fixed, 10, 0, 24.0, 1e-6).unwrap().position;
    assert_eq!(round(z.atan2(x).to_degrees(), 3), 1.141);
}

#[test]
fn seek_row() {
    let spk = excerpt();
    let s = Space::new("2025-10-28T00:00:00Z", TimeScale::Utc, 3600.0, 0, Frame::EclipticJ2000).unwrap();
    let first = place_body(&spk, &s, 4, 40, 24.0, 1e-9).unwrap();
    place_body(&spk, &s, 4, 1000, 24.0, 1e-9).unwrap();
    let again = place_body(&spk, &s, 4, 40, 24.0, 1e-9).unwrap();
    assert_eq!(first, again);
    assert_eq!(first.et, s.et0 + 40.0 / 24.0 * 3600.0);
}

#[test]
fn place_at_rows() {
    // a scene time in seconds is the frame clock's time of frame / fps
    let s = Space::new("2451545.0", TimeScale::Tdb, 3600.0, 399, Frame::Icrf).unwrap();
    assert_eq!(s.time_at(30.0 / 24.0).unwrap(), s.frame_time(30, 24.0).unwrap());
    assert!(s.time_at(f64::NAN).is_err());
    assert!(s.time_at(f64::INFINITY).is_err());
    // the file's placement is place_body's at the frame's time
    let spk = excerpt();
    let et = s.frame_time(30, 24.0).unwrap();
    assert_eq!(
        place_at(Positions::File(&spk), &s, 301, et, 1e-6).unwrap(),
        place_body(&spk, &s, 301, 30, 24.0, 1e-6).unwrap()
    );
    // the approximate elements place a planet about a planet or the Sun
    let helio = Space::new("2451545.0", TimeScale::Tdb, 1.0, 3, Frame::EclipticJ2000).unwrap();
    let got = place_at(Positions::Approximate, &helio, 4, 0.0, 1e-9).unwrap();
    let want = helio.scene_from_icrf(approximate_relative(4, 3, 0.0).unwrap(), [0.0; 3], 1e-9, 0.0).unwrap();
    assert_eq!(got.position, want);
    assert_eq!(got.et, 0.0);
    assert_eq!(Positions::Approximate.position(4, 3, 0.0).unwrap(), approximate_relative(4, 3, 0.0).unwrap());
    assert_eq!(Positions::File(&spk).position(301, 399, 0.0).unwrap(), spk.position(301, 399, 0.0).unwrap());
    assert!(place_at(Positions::Approximate, &helio, 301, 0.0, 1e-9).is_err());
    assert!(place_at(Positions::File(&spk), &s, 499, 0.0, 1e-6).is_err());
}
