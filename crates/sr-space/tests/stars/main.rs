//! The numeric table of the star-fields draft (sr-core srep/srep-0000-star-fields.md, Conformance), row by row,
//! against `sr_space::stars`, and the failure paths of its rules.
//!
//! The catalogue is `tests/corpus/media/bsc5-compact.csv.gz`, byte-equal to sr-core's
//! conformance/assets/bsc5-compact.csv.gz (the Yale Bright Star Catalogue, 5th edition, compacted by the
//! reference module's `compact_catalogue`).

use std::path::PathBuf;
use std::sync::OnceLock;

use sha2::Digest;
use sr_space::stars::{
    angle, blackbody_xyz, chromaticity, colour, constellation, direction, flux, load_catalogue, magnitude, moved,
    parse_catalogue, planck, precess, radec, scene_direction, spectral_class, star_field, temperature,
    years_since_j2000, Star, BOUNDARIES, CATALOGUE_DIGEST, CLASS_BV_G, JULIAN_YEAR_S, MAX_PROPER_MOTION, MAX_STARS,
};

fn path() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/corpus/media/bsc5-compact.csv.gz"))
}

fn cat() -> &'static [Star] {
    static CAT: OnceLock<Vec<Star>> = OnceLock::new();
    CAT.get_or_init(|| load_catalogue(&path(), Some(&format!("sha256:{CATALOGUE_DIGEST}"))).unwrap())
}

fn hr(n: u32) -> &'static Star {
    cat().iter().find(|s| s.hr == n).unwrap()
}

/// The table prints six significant digits, so a value is within half a unit of its sixth digit.
#[track_caller]
fn near(got: f64, want: f64) {
    assert!((got - want).abs() <= 5e-6 * want.abs() + 1e-12, "{got} != {want}");
}

#[track_caller]
fn near3(got: [f64; 3], want: [f64; 3]) {
    for (g, w) in got.into_iter().zip(want) {
        assert!((g - w).abs() <= 5e-6 * w.abs() + 1e-12, "{got:?} != {want:?}");
    }
}

#[test]
fn catalogue_rows() {
    assert_eq!(CATALOGUE_DIGEST, "cd18b1c9b8c009d922edfed020ba718aa14a577416fb625bf01ddd9cbdaab617");
    assert_eq!(cat().len(), 9096);
    assert_eq!(cat().iter().filter(|s| s.bv.is_none()).count(), 310);
    assert_eq!(cat().iter().filter(|s| s.vmag <= 6.5).count(), 8404);
    assert_eq!(cat().iter().filter(|s| s.vmag <= 6.0).count(), 5080);
    let brightest = cat().iter().min_by(|a, b| a.vmag.total_cmp(&b.vmag).then(a.hr.cmp(&b.hr))).unwrap();
    assert_eq!((brightest.hr, brightest.vmag), (2491, -1.46));
}

#[test]
fn flux_rows() {
    assert_eq!((flux(0.0, 1.0).unwrap() / flux(5.0, 1.0).unwrap() * 1e6).round() / 1e6, 100.0);
    near(flux(0.0, 1.0).unwrap() / flux(1.0, 1.0).unwrap(), 2.51189);
    near(flux(-1.46, 1.0).unwrap() / flux(6.5, 1.0).unwrap(), 1527.57);
    assert_eq!(flux(0.0, 3.0).unwrap(), 3.0);
    near(magnitude(flux(4.2, 1.0).unwrap(), 1.0).unwrap(), 4.2);
    assert!(flux(f64::NAN, 1.0).is_err() && flux(0.0, 0.0).is_err() && flux(0.0, -1.0).is_err());
    assert!(magnitude(0.0, 1.0).is_err());
}

#[test]
fn colour_rows() {
    near(temperature(0.65).unwrap(), 5778.42);
    near(temperature(0.0).unwrap(), 10125.2);
    near(temperature(1.85).unwrap(), 3333.2);
    assert_eq!(temperature(5.74).unwrap(), temperature(2.0).unwrap());
    assert_eq!(temperature(-3.0).unwrap(), temperature(-0.4).unwrap());
    assert!(temperature(f64::NAN).is_err());
    // below about 1900 K the blackbody's blue is negative in sRGB and clipped at 0
    let red = colour(1500.0).unwrap();
    near(red[0], 3.26418);
    near(red[1], 0.434455);
    assert_eq!(red[2], 0.0);
    let wien =
        (300..1000).max_by(|&a, &b| planck(a as f64, 5800.0).unwrap().total_cmp(&planck(b as f64, 5800.0).unwrap()));
    assert_eq!(wien, Some(500));
    assert!(planck(500.0, 0.0).is_err() && planck(0.0, 5800.0).is_err());
    let (x, y) = chromaticity(blackbody_xyz(5778.0).unwrap()).unwrap();
    near(x, 0.32643);
    near(y, 0.335722);
    let (x, y) = chromaticity(blackbody_xyz(6504.0).unwrap()).unwrap();
    near(x, 0.313463);
    near(y, 0.323567);
    near3(colour(5778.0).unwrap(), [1.11194, 0.975481, 0.913851]);
    near3(colour(temperature(0.0).unwrap()).unwrap(), [0.868294, 0.994718, 1.44071]);
    near3(colour(temperature(1.85).unwrap()).unwrap(), [1.62548, 0.878597, 0.361475]);
    let lowest = (-4..21)
        .map(|bv| colour(temperature(bv as f64 / 10.0).unwrap()).unwrap().into_iter().fold(f64::INFINITY, f64::min))
        .fold(f64::INFINITY, f64::min);
    near(lowest, 0.317665);
    assert_eq!((spectral_class("gG9"), CLASS_BV_G), (Some('G'), 0.65));
    assert_eq!(spectral_class("kA2hA5mA7V"), Some('A'));
    assert_eq!(spectral_class("pec"), None);
}

#[test]
fn direction_rows() {
    let s = hr(2491);
    near3(direction(s.ra, s.dec).unwrap(), [-0.187454, 0.939218, -0.28763]);
    let (ra, dec) = radec(direction(101.28708, -16.71611).unwrap()).unwrap();
    near(ra, 101.287);
    near(dec, -16.7161);
    assert!(direction(0.0, 91.0).is_err() && direction(f64::INFINITY, 0.0).is_err());
    let largest = cat()
        .iter()
        .max_by(|a, b| a.pm_ra.hypot(a.pm_dec).total_cmp(&b.pm_ra.hypot(b.pm_dec)).then(b.hr.cmp(&a.hr)))
        .unwrap();
    assert_eq!(largest.hr, 4550);
    near(largest.pm_ra.hypot(largest.pm_dec), 7.05797);
    let g = hr(4550);
    near(angle(direction(g.ra, g.dec).unwrap(), moved(g, 100.0).unwrap()).unwrap().to_degrees(), 0.196054);
    let synthetic = Star {
        hr: 0,
        name: String::new(),
        ra: 10.0,
        dec: 60.0,
        vmag: 5.0,
        bv: Some(0.5),
        pm_ra: 1.0,
        pm_dec: 0.0,
        sptype: "G0".into(),
    };
    near(radec(moved(&synthetic, 1.0).unwrap()).unwrap().0 - 10.0, 0.000555556);
    assert!(moved(g, 10_001.0).is_err() && moved(g, f64::NAN).is_err());
    assert_eq!(moved(g, 0.0).unwrap(), direction(g.ra, g.dec).unwrap());
    assert_eq!(years_since_j2000(JULIAN_YEAR_S), 1.0);
    assert_eq!(scene_direction([1.0, 2.0, 3.0]), [1.0, -3.0, 2.0]);
}

#[test]
fn constellation_rows() {
    let (ra, dec) = precess(0.0, 0.0, 1900.0, 2000.0).unwrap();
    near(ra, 1.28057);
    near(dec, 0.556692);
    let (a, d) = precess(101.28708, -16.71611, 2000.0, 1875.0).unwrap();
    let (ra, dec) = precess(a, d, 1875.0, 2000.0).unwrap();
    near(ra, 101.287);
    near(dec, -16.7161);
    let names: std::collections::BTreeSet<&str> = BOUNDARIES.iter().map(|r| r.3).collect();
    assert_eq!((BOUNDARIES.len(), names.len()), (357, 88));
    assert_eq!(constellation(9.0 * 15.0, 65.0, 1950.0).unwrap(), "UMa");
    assert_eq!(constellation(5.12 * 15.0, 9.12, 1950.0).unwrap(), "Ori");
    assert_eq!(constellation(6.2222 * 15.0, -81.1234, 1950.0).unwrap(), "Men");
    // on a row's lower declination the row holds the place; on its upper right ascension the next row does
    assert_eq!(constellation(0.0, 88.0, 1875.0).unwrap(), "UMi");
    assert_eq!(constellation(345.0, 86.6667, 1875.0).unwrap(), "Cep");
    let of = |n: u32| constellation(hr(n).ra, hr(n).dec, 2000.0).unwrap();
    assert_eq!((of(2491), of(424), of(7001)), ("CMa", "UMi", "Lyr"));
    assert_eq!(
        (constellation(236.0, 7.0, 2000.0).unwrap(), constellation(272.0, -5.0, 2000.0).unwrap()),
        ("Ser", "Ser")
    );
    assert!(constellation(0.0, 90.5, 2000.0).is_err());
    // every Bayer or Flamsteed name ending in a constellation agrees with the lookup
    let (mut named, mut agree) = (0, 0);
    for s in cat() {
        let tail = s.name.get(s.name.len().saturating_sub(3)..).unwrap_or("");
        if names.contains(tail) {
            named += 1;
            agree += usize::from(constellation(s.ra, s.dec, 2000.0).unwrap() == tail);
        }
    }
    assert_eq!((named, agree), (3143, 3143));
}

#[test]
fn field_rows() {
    let field = star_field(cat(), 6.5, 25.0, 1.0).unwrap();
    assert_eq!(field.len(), 8404);
    let mut h = sha2::Sha256::new();
    for s in &field {
        let numbers = [s.direction[0], s.direction[1], s.direction[2], s.flux, s.colour[0], s.colour[1], s.colour[2]];
        let text: Vec<String> = numbers.iter().map(|x| format!("{x:.6}")).collect();
        h.update(format!("{} {} {}\n", s.hr, text.join(" "), s.constellation));
    }
    let digest: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(&digest[..16], "4d9539e19c1f5a15");
    // the seek check: frame 600 at one Julian year per second, sought, equals the field played to it
    let mut played = None;
    for frame in 0..=600 {
        played = Some(star_field(cat(), 4.0, frame as f64 / 24.0, 1.0).unwrap());
    }
    assert_eq!(played.unwrap(), star_field(cat(), 4.0, 600.0 / 24.0, 1.0).unwrap());
    assert!(star_field(cat(), f64::INFINITY, 0.0, 1.0).is_err());
    assert!(star_field(cat(), 6.5, 0.0, 0.0).is_err());
}

fn gz(text: &str) -> Vec<u8> {
    use std::io::Write;
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(text.as_bytes()).unwrap();
    e.finish().unwrap()
}

const HEADER: &str = "HR,Name,RA,Dec,V,B-V,pmRA,pmDE,SpType\n";

#[test]
fn a_catalogue_is_refused_as_rule_1_says() {
    let raw = std::fs::read(path()).unwrap();
    let mut wrong = CATALOGUE_DIGEST.to_string();
    wrong.replace_range(0..1, "0");
    assert!(parse_catalogue(&raw, Some(&format!("sha256:{wrong}"))).is_err());
    assert_eq!(parse_catalogue(&raw, None).unwrap().len(), 9096);
    let ok = gz(&format!("{HEADER}1,a,10.0,5.0,4.0,0.5,0.0,0.0,G0\n2,\"b\",11.0,6.0,5.0,,0.1,0.2,\"C5,4\"\n"));
    let stars = parse_catalogue(&ok, None).unwrap();
    assert_eq!((stars[1].bv, stars[1].sptype.as_str()), (None, "C5,4"));
    // SF1: proper motions at most MAX_PROPER_MOTION arcseconds a year in magnitude
    let fast = gz(&format!("{HEADER}1,a,10,5,4,0.5,100,-100,G0\n"));
    assert_eq!(parse_catalogue(&fast, None).unwrap()[0].pm_dec, -MAX_PROPER_MOTION);
    let quoted = gz(&format!("{HEADER}1,\"a \"\"b\"\"\",10.0,5.0,4.0,0.5,0.0,0.0,G0\n"));
    assert_eq!(parse_catalogue(&quoted, None).unwrap()[0].name, "a \"b\"");
    for bad in [
        gz("HR,Name,RA,Dec,V,B-V,pmRA,pmDE\n"),
        gz(HEADER),
        gz(&format!("{HEADER}2,a,10,5,4,0.5,0,0,G0\n1,b,10,5,4,0.5,0,0,G0\n")),
        gz(&format!("{HEADER}2,a,10,5,4,0.5,0,0,G0\n2,b,10,5,4,0.5,0,0,G0\n")),
        gz(&format!("{HEADER}1,a,10,5,4,0.5,0,0,\"G0\n")),
        gz(&format!("{HEADER}1,a,10,5,4,0.5,nan,0,G0\n")),
        gz(&format!("{HEADER}1,a,10,5,4,0.5,0,nan,G0\n")),
        gz(&format!("{HEADER}1,a,10,5,4,inf,0,0,G0\n")),
        gz(&format!("{HEADER}1,a,10,5,4,0.5,100.001,0,G0\n")),
        gz(&format!("{HEADER}1,a,10,5,4,0.5,0,-100.5,G0\n")),
        gz(&format!("{HEADER}1,a,10,5,4,0.5,-inf,0,G0\n")),
        gz(&format!("{HEADER}1,a,360,5,4,0.5,0,0,G0\n")),
        gz(&format!("{HEADER}1,a,10,90.5,4,0.5,0,0,G0\n")),
        gz(&format!("{HEADER}1,a,10,5,31,0.5,0,0,G0\n")),
        gz(&format!("{HEADER}1,a,10,5,x,0.5,0,0,G0\n")),
        gz(&format!("{HEADER}1,a,10,5,4,0.5,0,0\n")),
        gz(&format!("{HEADER}1,a,nan,5,4,0.5,0,0,G0\n")),
        b"HR,Name\n".to_vec(),
        gz("\u{e9}"),
    ] {
        assert!(parse_catalogue(&bad, None).is_err(), "{:?}", bad.len());
    }
    let many: String = (1..=MAX_STARS as u32 + 1).map(|i| format!("{i},s,10,5,4,0.5,0,0,G0\n")).collect();
    assert!(parse_catalogue(&gz(&format!("{HEADER}{many}")), None).is_err());
}
