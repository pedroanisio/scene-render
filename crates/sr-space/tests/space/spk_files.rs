//! The reader on both byte orders and the failure paths of rules 1 to 6: every malformed file, foreign value
//! and out-of-range request is a typed error, never a default.

use sr_space::{
    apparent_position, approximate_position, leap_seconds, mat_apply, parse_utc, place_body, pole_icrf,
    rotation_elements, solve_kepler, ElementTable, Frame, Space, SpaceError, Spk, TimeScale, C_KM_S,
    MAX_KEPLER_ITERATIONS, SCENE_FROM_ICRF_AXES,
};

use crate::reference::{excerpt, EXCERPT, EXCERPT_DIGEST};

const RECORD: usize = 1024;

fn bytes() -> Vec<u8> {
    std::fs::read(EXCERPT).unwrap()
}

fn i32_at(b: &[u8], off: usize) -> i32 {
    i32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}

fn f64_at(b: &[u8], off: usize) -> f64 {
    f64::from_le_bytes(b[off..off + 8].try_into().unwrap())
}

fn put_i32(b: &mut [u8], off: usize, v: i32) {
    b[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

/// The summary record numbers of a little-endian file, in chain order.
fn summary_records(b: &[u8]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut rec = i32_at(b, 76) as usize;
    while rec > 0 {
        out.push(rec);
        rec = f64_at(b, (rec - 1) * RECORD) as usize;
    }
    out
}

/// The same file in BIG-IEEE: every double and integer word swapped, names kept.
fn big_endian(le: &[u8]) -> Vec<u8> {
    let mut b = le.to_vec();
    for off in [8, 12, 76, 80, 84] {
        b[off..off + 4].reverse();
    }
    b[88..96].copy_from_slice(b"BIG-IEEE");
    let summaries = summary_records(le);
    for &rec in &summaries {
        let base = (rec - 1) * RECORD;
        let n = f64_at(le, base + 16) as usize;
        for w in 0..3 {
            b[base + w * 8..base + w * 8 + 8].reverse();
        }
        for i in 0..n {
            let off = base + 24 + i * 40;
            b[off..off + 8].reverse();
            b[off + 8..off + 16].reverse();
            for k in 0..6 {
                b[off + 16 + k * 4..off + 20 + k * 4].reverse();
            }
        }
    }
    let skip: Vec<usize> = summaries.iter().flat_map(|&r| [r, r + 1]).collect();
    for rec in 2..=le.len() / RECORD {
        if skip.contains(&rec) {
            continue;
        }
        for w in (0..RECORD).step_by(8) {
            b[(rec - 1) * RECORD + w..(rec - 1) * RECORD + w + 8].reverse();
        }
    }
    b
}

fn parse_err(b: &[u8]) -> String {
    match Spk::parse(b.to_vec()) {
        Err(SpaceError::Ephemeris(m)) => m,
        other => panic!("expected an ephemeris error, got {:?}", other.map(|s| s.segments.len())),
    }
}

#[test]
fn the_excerpt_spans_more_than_one_summary_record() {
    assert_eq!(summary_records(&bytes()).len(), 3);
}

#[test]
fn a_big_endian_file_reads_to_the_same_positions() {
    let le = excerpt();
    let be = Spk::parse(big_endian(&bytes())).unwrap();
    assert_eq!(be.format, "BIG-IEEE");
    assert_eq!(be.segments.len(), le.segments.len());
    for body in [399, 301, 10, 5] {
        assert_eq!(be.position(body, 0, 1234.5).unwrap(), le.position(body, 0, 1234.5).unwrap());
    }
}

#[test]
fn a_foreign_or_short_file_is_refused() {
    let mut b = bytes();
    b[0..8].copy_from_slice(b"DAF/PCK ");
    assert!(parse_err(&b).contains("not an SPK file"));
    assert!(parse_err(&bytes()[..RECORD]).contains("shorter"));
    let mut b = bytes();
    b[88..96].copy_from_slice(b"VAX-GFLT");
    assert!(parse_err(&b).contains("byte order"));
    let mut b = bytes();
    put_i32(&mut b, 8, 3);
    assert!(parse_err(&b).contains("2 doubles and 6 integers"));
}

#[test]
fn a_segment_of_another_type_frame_or_address_is_refused() {
    let first = (summary_records(&bytes())[0] - 1) * RECORD + 24;
    let mut b = bytes();
    put_i32(&mut b, first + 16 + 3 * 4, 3);
    assert!(parse_err(&b).contains("type 3"));
    let mut b = bytes();
    put_i32(&mut b, first + 16 + 2 * 4, 17);
    assert!(parse_err(&b).contains("frame 17"));
    let mut b = bytes();
    put_i32(&mut b, first + 16 + 5 * 4, i32::MAX / 8);
    assert!(parse_err(&b).contains("outside the file"));
    let mut b = bytes();
    let rec = summary_records(&b)[0];
    b[(rec - 1) * RECORD + 16..(rec - 1) * RECORD + 24].copy_from_slice(&26.0f64.to_le_bytes());
    assert!(parse_err(&b).contains("0 to 25 summaries"));
}

#[test]
fn a_summary_chain_that_loops_or_leaves_the_file_is_refused() {
    let mut b = bytes();
    let last = *summary_records(&b).last().unwrap();
    let first = summary_records(&b)[0];
    b[(last - 1) * RECORD..(last - 1) * RECORD + 8].copy_from_slice(&(first as f64).to_le_bytes());
    assert!(parse_err(&b).contains("loop"));
    let mut b = bytes();
    put_i32(&mut b, 76, 10_000);
    assert!(parse_err(&b).contains("beyond the file"));
}

#[test]
fn the_digest_pins_the_file() {
    let path = std::path::Path::new(EXCERPT);
    assert!(Spk::open(path, Some(&format!("sha256:{EXCERPT_DIGEST}"))).is_ok());
    let mut wrong = EXCERPT_DIGEST.to_string();
    wrong.replace_range(0..1, if wrong.starts_with('0') { "1" } else { "0" });
    match Spk::open(path, Some(&format!("sha256:{wrong}"))) {
        Err(SpaceError::Ephemeris(m)) => assert!(m.contains("digest mismatch"), "{m}"),
        other => panic!("{:?}", other.map(|_| ())),
    }
    for bad in ["sha256:xyz", "md5:00", "sha256:", &format!("sha256:{}", EXCERPT_DIGEST.to_uppercase())] {
        assert!(Spk::open(path, Some(bad)).is_err(), "{bad}");
    }
    assert!(Spk::open(std::path::Path::new("/nonexistent.bsp"), None).is_err());
}

#[test]
fn an_absent_body_or_an_uncovered_time_is_an_error() {
    let spk = excerpt();
    let err = |r: Result<[f64; 3], SpaceError>| match r {
        Err(SpaceError::Ephemeris(m)) => m,
        other => panic!("{other:?}"),
    };
    assert!(err(spk.position(499, 0, 0.0)).contains("not in the ephemeris"));
    assert!(err(spk.position(399, 0, 1.0e9)).contains("no segment"));
    assert_eq!(spk.position(0, 0, 0.0).unwrap(), [0.0; 3]);
    assert!(spk.position(42, 42, 0.0).is_err());
}

#[test]
fn time_scale_inputs_out_of_range_are_errors() {
    for bad in [
        "2003-10-17 12:30:30Z",
        "2003-13-17T12:30:30Z",
        "2003-02-30T12:30:30Z",
        "2003-10-17T12:30:60Z",
        "",
        "2003-10-17T12:30:30+é:00",
        "2003-10-17T12:30:30+0é:0",
        "2003-10-17T12:30:30.Z",
        "0000-01-01T00:00:00Z",
    ] {
        assert!(matches!(parse_utc(bad), Err(SpaceError::Time(_))), "{bad}");
    }
    assert!(leap_seconds(2441317.4).is_err());
    assert!(Space::new("1971-12-31T23:59:59Z", TimeScale::Utc, 1.0, 0, Frame::Icrf).is_err());
    assert!(Space::new("J2000", TimeScale::Tt, 1.0, 0, Frame::Icrf).is_err());
    for rate in [-1.0, f64::NAN, f64::INFINITY] {
        assert!(Space::new("2451545.0", TimeScale::Tdb, rate, 0, Frame::Icrf).is_err(), "{rate}");
    }
    assert!(Space::new("2451545.0", TimeScale::Tdb, 1.0, 0, Frame::BodyFixed(2)).is_err());
    let s = Space::new("2451545.0", TimeScale::Tdb, 1.0, 0, Frame::Icrf).unwrap();
    assert!(s.frame_time(0, 0.0).is_err());
    assert!(s.scene_from_icrf([0.0; 3], [0.0; 3], 0.0, 0.0).is_err());
}

#[test]
fn the_approximate_elements_refuse_what_their_tables_do_not_cover() {
    assert!(approximate_position(9, 0.0, ElementTable::Y1800To2050).is_err());
    assert!(approximate_position(3, 100.0 * 365.25 * 86400.0, ElementTable::Y1800To2050).is_err());
    assert!(approximate_position(3, 100.0 * 365.25 * 86400.0, ElementTable::Bc3000ToAd3000).is_ok());
    assert!(solve_kepler(10.0, 1.0).is_err());
    assert!(solve_kepler(10.0, -0.1).is_err());
    const { assert!(MAX_KEPLER_ITERATIONS <= 50) };
    assert!(rotation_elements(4, 0.0).is_err());
}

#[test]
fn placing_a_body_absent_from_the_file_is_an_error() {
    let s = Space::new("2451545.0", TimeScale::Tdb, 1.0, 0, Frame::Icrf).unwrap();
    assert!(place_body(&excerpt(), &s, 499, 0, 24.0, 1e-6).is_err());
}

#[test]
fn the_light_time_solves_its_own_equation() {
    let spk = excerpt();
    for (target, observer) in [(10, 399), (301, 399), (5, 399)] {
        let (p, lt) = apparent_position(&spk, target, observer, 0.0).unwrap();
        let tgt = spk.position(target, 0, -lt).unwrap();
        let obs = spk.position(observer, 0, 0.0).unwrap();
        let d = [tgt[0] - obs[0], tgt[1] - obs[1], tgt[2] - obs[2]];
        assert_eq!(p, d);
        let residual = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() / C_KM_S - lt;
        assert!(residual.abs() < 1e-7, "{target}: {residual}");
    }
}

#[test]
fn a_body_with_rotation_elements_turns_its_pole_onto_the_scene_up_axis() {
    let spk = excerpt();
    let s = Space::new("2451545.0", TimeScale::Tdb, 1.0, 0, Frame::Icrf).unwrap();
    for body in [399, 301] {
        let o = place_body(&spk, &s, body, 0, 24.0, 1e-6).unwrap().orientation;
        // the body's +z (its pole) in scene axes is (0, -1, 0); the orientation takes it to the pole's scene image
        let got = mat_apply(&o, [0.0, -1.0, 0.0]);
        let want = mat_apply(&SCENE_FROM_ICRF_AXES, pole_icrf(body, 0.0).unwrap());
        for k in 0..3 {
            assert!((got[k] - want[k]).abs() < 1e-12, "{body}: {got:?} != {want:?}");
        }
    }
    // a body without rotation elements keeps the space's axes
    let bary = place_body(&spk, &s, 3, 0, 24.0, 1e-6).unwrap().orientation;
    assert_eq!(bary, [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
}
