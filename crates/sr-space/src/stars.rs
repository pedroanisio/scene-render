//! Star fields from the Bright Star Catalogue: the sr-core draft "Add star fields from the Bright Star Catalogue"
//! (`srep/srep-0000-star-fields.md`, SP2 of SREP 12), rules 1 to 7. The catalogue is a gzipped CSV pinned by its
//! sha256; a star's flux follows its V magnitude, its colour the blackbody of its B−V temperature, its direction
//! its J2000 place moved by its proper motion, and its constellation Roman's table of boundaries. Every function
//! is pure, so a frame rendered alone equals the frame reached by playing.

use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::frames::SCENE_FROM_ICRF_AXES;
use crate::SpaceError;

mod tables;
pub use tables::{BOUNDARIES, CMF};

/// The sha256 of the reference catalogue, sr-core's `conformance/assets/bsc5-compact.csv.gz`.
pub const CATALOGUE_DIGEST: &str = "cd18b1c9b8c009d922edfed020ba718aa14a577416fb625bf01ddd9cbdaab617";
/// The catalogue's columns, in order (SF1).
pub const COLUMNS: [&str; 9] = ["HR", "Name", "RA", "Dec", "V", "B-V", "pmRA", "pmDE", "SpType"];
/// The most stars a catalogue holds and a field draws (rule 6).
pub const MAX_STARS: usize = 16384;
/// The largest proper motion in either coordinate, arcseconds a Julian year (SF1; Barnard's star moves 10.4″).
pub const MAX_PROPER_MOTION: f64 = 100.0;
/// The span of linear proper motion about J2000, Julian years (SF4, rule 4).
pub const MAX_YEARS: f64 = 10_000.0;
/// Seconds of a Julian year.
pub const JULIAN_YEAR_S: f64 = 31_557_600.0;
/// B−V is clamped to this range before its temperature (rule 3).
pub const BV_RANGE: (f64, f64) = (-0.4, 2.0);
/// The B−V of a star whose spectral type names class G, or no class (the Sun's).
pub const CLASS_BV_G: f64 = 0.65;
/// The largest decompressed catalogue read: 64 bytes a row is far above the compact CSV's.
const MAX_CSV_BYTES: u64 = (MAX_STARS as u64 + 1) * 256;

const PLANCK_C1: f64 = 2.0 * 6.62607015e-34 * 299792458.0 * 299792458.0;
const PLANCK_C2: f64 = 6.62607015e-34 * 299792458.0 / 1.380649e-23;
/// CIE XYZ to linear sRGB (IEC 61966-2-1), rows first.
const XYZ_TO_SRGB: [[f64; 3]; 3] = [[3.2406, -1.5372, -0.4986], [-0.9689, 1.8758, 0.0415], [0.0557, -0.2040, 1.0570]];

fn err(message: impl Into<String>) -> SpaceError {
    SpaceError::Stars(message.into())
}

/// One catalogue star: J2000 place in degrees, V, B−V when known, proper motion in arcseconds per Julian year
/// (pmRA already × cos δ) and spectral type.
#[derive(Debug, Clone, PartialEq)]
pub struct Star {
    pub hr: u32,
    pub name: String,
    pub ra: f64,
    pub dec: f64,
    pub vmag: f64,
    pub bv: Option<f64>,
    pub pm_ra: f64,
    pub pm_dec: f64,
    pub sptype: String,
}

/// Rule 1: reads a catalogue file and checks it against `digest` (`sha256:` and 64 hex digits; None skips the pin).
pub fn load_catalogue(path: &Path, digest: Option<&str>) -> Result<Vec<Star>, SpaceError> {
    let raw = std::fs::read(path).map_err(|e| err(format!("{}: {e}", path.display())))?;
    parse_catalogue(&raw, digest)
}

/// Rule 1 on a catalogue's bytes: the digest, the gzip member, the nine columns, HR strictly increasing and every
/// value in range (SF1).
pub fn parse_catalogue(raw: &[u8], digest: Option<&str>) -> Result<Vec<Star>, SpaceError> {
    if let Some(d) = digest {
        let want = d.strip_prefix("sha256:").unwrap_or(d);
        let got: String = Sha256::digest(raw).iter().map(|b| format!("{b:02x}")).collect();
        if got != want {
            return Err(err(format!("catalogue digest {got} is not the pinned {want}")));
        }
    }
    let mut text = String::new();
    flate2::read::GzDecoder::new(raw)
        .take(MAX_CSV_BYTES)
        .read_to_string(&mut text)
        .map_err(|e| err(format!("catalogue is not a gzipped ASCII CSV: {e}")))?;
    if !text.is_ascii() {
        return Err(err("catalogue is not ASCII"));
    }
    let mut lines = text.lines();
    let header = csv_fields(lines.next().unwrap_or(""))?;
    if header != COLUMNS {
        return Err(err(format!("catalogue columns {header:?} are not {COLUMNS:?}")));
    }
    let mut stars: Vec<Star> = Vec::new();
    for line in lines.filter(|l| !l.is_empty()) {
        let row = stars.len() + 1;
        let f = csv_fields(line)?;
        if f.len() != COLUMNS.len() {
            return Err(err(format!("catalogue row {row} has {} fields", f.len())));
        }
        let num = |s: &str| s.parse::<f64>().map_err(|e| err(format!("catalogue row {row}: {s:?}: {e}")));
        let hr = f[0].parse::<u32>().map_err(|e| err(format!("catalogue row {row}: HR {:?}: {e}", f[0])))?;
        let (ra, dec, vmag) = (num(&f[2])?, num(&f[3])?, num(&f[4])?);
        let bv = if f[5].is_empty() { None } else { Some(num(&f[5])?) };
        let (pm_ra, pm_dec) = (num(&f[6])?, num(&f[7])?);
        let in_order = stars.last().is_none_or(|s| hr > s.hr) && hr > 0;
        if !in_order || !(0.0..360.0).contains(&ra) || !(-90.0..=90.0).contains(&dec) || !(-5.0..=30.0).contains(&vmag)
        {
            return Err(err(format!("catalogue row {row}: HR {hr} out of order or out of range")));
        }
        if !(pm_ra.is_finite() && pm_dec.is_finite() && bv.is_none_or(f64::is_finite)) {
            return Err(err(format!("catalogue row {row}: HR {hr} has a value that is not finite")));
        }
        if pm_ra.abs().max(pm_dec.abs()) > MAX_PROPER_MOTION {
            return Err(err(format!("catalogue row {row}: HR {hr} moves more than {MAX_PROPER_MOTION}″ a year")));
        }
        if stars.len() >= MAX_STARS {
            return Err(err(format!("catalogue has more than {MAX_STARS} stars")));
        }
        stars.push(Star { hr, name: f[1].clone(), ra, dec, vmag, bv, pm_ra, pm_dec, sptype: f[8].clone() });
    }
    if stars.is_empty() {
        return Err(err("catalogue is empty"));
    }
    Ok(stars)
}

/// The fields of one CSV line as Python's csv module writes them: a field holding a comma or a quote is quoted, a
/// quote inside doubled.
fn csv_fields(line: &str) -> Result<Vec<String>, SpaceError> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars().peekable();
    let mut quoted = false;
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            (true, '"') => quoted = false,
            (true, c) => cur.push(c),
            (false, '"') if cur.is_empty() => quoted = true,
            (false, ',') => out.push(std::mem::take(&mut cur)),
            (false, c) => cur.push(c),
        }
    }
    if quoted {
        return Err(err(format!("catalogue line {line:?} has an unclosed quote")));
    }
    out.push(cur);
    Ok(out)
}

/// Rule 2: the flux of magnitude m, `zero` being the flux of magnitude 0.
pub fn flux(m: f64, zero: f64) -> Result<f64, SpaceError> {
    if !(zero > 0.0 && zero.is_finite()) || !m.is_finite() {
        return Err(err("the zero point is positive and the magnitude finite"));
    }
    Ok(zero * 10f64.powf(-0.4 * m))
}

/// Rule 2 inverted: the magnitude of flux f.
pub fn magnitude(f: f64, zero: f64) -> Result<f64, SpaceError> {
    if !(f > 0.0 && zero > 0.0) {
        return Err(err("a magnitude needs a positive flux"));
    }
    Ok(-2.5 * (f / zero).log10())
}

/// Rule 3: Ballesteros (2012), B−V clamped to [`BV_RANGE`].
pub fn temperature(bv: f64) -> Result<f64, SpaceError> {
    if !bv.is_finite() {
        return Err(err("B-V is finite"));
    }
    let bv = bv.clamp(BV_RANGE.0, BV_RANGE.1);
    Ok(4600.0 * (1.0 / (0.92 * bv + 1.7) + 1.0 / (0.92 * bv + 0.62)))
}

/// Planck's law, W sr⁻¹ m⁻³, at a wavelength in nanometres.
pub fn planck(wavelength_nm: f64, t: f64) -> Result<f64, SpaceError> {
    if !(t > 0.0 && wavelength_nm > 0.0) {
        return Err(err("Planck's law needs a positive temperature and wavelength"));
    }
    let lam = wavelength_nm * 1e-9;
    Ok(PLANCK_C1 / (lam.powi(5) * (PLANCK_C2 / (lam * t)).exp_m1()))
}

/// CIE XYZ of the Planck spectrum, summed over [`CMF`].
pub fn blackbody_xyz(t: f64) -> Result<[f64; 3], SpaceError> {
    let mut xyz = [0.0; 3];
    for &(nm, xb, yb, zb) in &CMF {
        let b = planck(nm, t)?;
        xyz[0] += b * xb;
        xyz[1] += b * yb;
        xyz[2] += b * zb;
    }
    Ok(xyz)
}

/// The (x, y) chromaticity of a colour.
pub fn chromaticity(xyz: [f64; 3]) -> Result<(f64, f64), SpaceError> {
    let s = xyz[0] + xyz[1] + xyz[2];
    if s.is_nan() || s <= 0.0 {
        return Err(err("a black colour has no chromaticity"));
    }
    Ok((xyz[0] / s, xyz[1] / s))
}

/// Rule 3: the linear sRGB of a blackbody at t kelvin, normalised to Y = 1 and clipped at 0.
pub fn colour(t: f64) -> Result<[f64; 3], SpaceError> {
    let xyz = blackbody_xyz(t)?;
    if xyz[1].is_nan() || xyz[1] <= 0.0 {
        return Err(err("the spectrum has no luminance"));
    }
    let n = [xyz[0] / xyz[1], 1.0, xyz[2] / xyz[1]];
    Ok(XYZ_TO_SRGB.map(|m| (m[0] * n[0] + m[1] * n[1] + m[2] * n[2]).max(0.0)))
}

/// The first class letter O B A F G K M of a spectral type (gG9 is G, kA2hA5mA7V is A); None when it names none.
pub fn spectral_class(sptype: &str) -> Option<char> {
    sptype.chars().find(|c| "OBAFGKM".contains(*c))
}

/// The B−V a star without one takes from its class (`CLASS_BV`); the Sun's for no class.
pub fn class_bv(class: Option<char>) -> f64 {
    match class {
        Some('O') => -0.3,
        Some('B') => -0.2,
        Some('A') => 0.0,
        Some('F') => 0.4,
        Some('K') => 1.0,
        Some('M') => 1.5,
        _ => CLASS_BV_G,
    }
}

/// A star's temperature: its B−V's, else its class's.
pub fn star_temperature(star: &Star) -> Result<f64, SpaceError> {
    temperature(star.bv.unwrap_or_else(|| class_bv(spectral_class(&star.sptype))))
}

fn norm(a: [f64; 3]) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

fn unit(a: [f64; 3]) -> Result<[f64; 3], SpaceError> {
    let n = norm(a);
    if !(n > 0.0 && n.is_finite()) {
        return Err(err("a zero vector has no direction"));
    }
    Ok([a[0] / n, a[1] / n, a[2] / n])
}

/// The angle in radians between two directions, stable for small angles.
pub fn angle(a: [f64; 3], b: [f64; 3]) -> Result<f64, SpaceError> {
    let (c, d) = (unit(a)?, unit(b)?);
    let cross = [c[1] * d[2] - c[2] * d[1], c[2] * d[0] - c[0] * d[2], c[0] * d[1] - c[1] * d[0]];
    Ok(norm(cross).atan2(c[0] * d[0] + c[1] * d[1] + c[2] * d[2]))
}

/// Rule 4: the ICRF unit vector of a right ascension and declination in degrees.
pub fn direction(ra_deg: f64, dec_deg: f64) -> Result<[f64; 3], SpaceError> {
    if !(-90.0..=90.0).contains(&dec_deg) || !ra_deg.is_finite() {
        return Err(err("declination is within [-90, 90] and right ascension finite"));
    }
    let (ra, dec) = (ra_deg.to_radians(), dec_deg.to_radians());
    Ok([dec.cos() * ra.cos(), dec.cos() * ra.sin(), dec.sin()])
}

/// The right ascension in [0, 360) and declination of a direction, degrees.
pub fn radec(d: [f64; 3]) -> Result<(f64, f64), SpaceError> {
    let u = unit(d)?;
    Ok((u[1].atan2(u[0]).to_degrees().rem_euclid(360.0), u[2].clamp(-1.0, 1.0).asin().to_degrees()))
}

/// Julian years since J2000 of an ephemeris time in seconds.
pub fn years_since_j2000(et_seconds: f64) -> f64 {
    et_seconds / JULIAN_YEAR_S
}

/// Rule 4: the star's direction `years` Julian years after J2000, its J2000 direction displaced along the local
/// east and north by pmRA and pmDE times the years and renormalised (great-circle motion to second order).
pub fn moved(star: &Star, years: f64) -> Result<[f64; 3], SpaceError> {
    if !years.is_finite() || years.abs() > MAX_YEARS {
        return Err(err(format!("the proper-motion span is within {MAX_YEARS} years of J2000, not {years}")));
    }
    let d = direction(star.ra, star.dec)?;
    if years == 0.0 || (star.pm_ra == 0.0 && star.pm_dec == 0.0) {
        return Ok(d);
    }
    let (ra, dec) = (star.ra.to_radians(), star.dec.to_radians());
    let east = [-ra.sin(), ra.cos(), 0.0];
    let north = [-dec.sin() * ra.cos(), -dec.sin() * ra.sin(), dec.cos()];
    let e = (star.pm_ra / 3600.0 * years).to_radians();
    let n = (star.pm_dec / 3600.0 * years).to_radians();
    unit([0, 1, 2].map(|k| d[k] + e * east[k] + n * north[k]))
}

/// The scene direction of an ICRF direction: (x, −z, y), the time-frames draft's axes.
pub fn scene_direction(d: [f64; 3]) -> [f64; 3] {
    SCENE_FROM_ICRF_AXES.map(|m| m[0] * d[0] + m[1] * d[1] + m[2] * d[2])
}

/// Rule 5: Herget's precession (as in Roman's program) of a mean place in degrees from epoch1 to epoch2, years AD.
pub fn precess(ra_deg: f64, dec_deg: f64, epoch1: f64, epoch2: f64) -> Result<(f64, f64), SpaceError> {
    if epoch1 == epoch2 {
        return Ok((ra_deg, dec_deg));
    }
    let x1 = direction(ra_deg, dec_deg)?;
    let csr = 1f64.to_radians() / 3600.0;
    let t = 0.001 * (epoch2 - epoch1);
    let st = 0.001 * (epoch1 - 1900.0);
    let a = csr * t * (23042.53 + st * (139.75 + 0.06 * st) + t * (30.23 - 0.27 * st + 18.0 * t));
    let b = csr * t * t * (79.27 + 0.66 * st + 0.32 * t) + a;
    let c = csr * t * (20046.85 - st * (85.33 + 0.37 * st) + t * (-42.67 - 0.37 * st - 41.8 * t));
    let (sa, sb, sc, ca, cb, cc) = (a.sin(), b.sin(), c.sin(), a.cos(), b.cos(), c.cos());
    let r = [
        [ca * cb * cc - sa * sb, -ca * sb - sa * cb * cc, -cb * sc],
        [sa * cb + ca * sb * cc, ca * cb - sa * sb * cc, -sb * sc],
        [ca * sc, -sa * sc, cc],
    ];
    radec(r.map(|row| row[0] * x1[0] + row[1] * x1[1] + row[2] * x1[2]))
}

/// Rule 5: Roman's lookup. The place is precessed to 1875; the first boundary row at or below its declination whose
/// arc holds its right ascension names the constellation (Octans closes the table at −90°).
pub fn constellation(ra_deg: f64, dec_deg: f64, epoch: f64) -> Result<&'static str, SpaceError> {
    if !(-90.0..=90.0).contains(&dec_deg) {
        return Err(err("declination is within [-90, 90]"));
    }
    let (ra, dec) = precess(ra_deg.rem_euclid(360.0), dec_deg, epoch, 1875.0)?;
    let ra_h = (ra / 15.0).rem_euclid(24.0);
    BOUNDARIES
        .iter()
        .find(|&&(low, up, dec_low, _)| dec_low <= dec && low <= ra_h && ra_h < up)
        .map(|r| r.3)
        .ok_or_else(|| err(format!("no constellation holds RA {ra_deg} Dec {dec_deg}")))
}

/// One star of a field (rule 6): its ICRF direction at the field's epoch, flux, linear colour (Y = 1) and
/// constellation.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldStar {
    pub hr: u32,
    pub direction: [f64; 3],
    pub flux: f64,
    pub colour: [f64; 3],
    pub constellation: &'static str,
}

/// Rule 6: the stars to V ≤ `limit_magnitude`, in catalogue order, `years` after J2000, magnitude 0 having flux
/// `magnitude_zero`. Proper motion is the caller's to leave out by passing 0 years.
pub fn star_field(
    catalogue: &[Star],
    limit_magnitude: f64,
    years: f64,
    magnitude_zero: f64,
) -> Result<Vec<FieldStar>, SpaceError> {
    if !(magnitude_zero > 0.0 && magnitude_zero.is_finite()) {
        return Err(err("magnitudeZero is positive"));
    }
    if !limit_magnitude.is_finite() {
        return Err(err("limitMagnitude is finite"));
    }
    if catalogue.len() > MAX_STARS {
        return Err(err(format!("catalogue has more than {MAX_STARS} stars")));
    }
    catalogue
        .iter()
        .filter(|s| s.vmag <= limit_magnitude)
        .map(|s| {
            Ok(FieldStar {
                hr: s.hr,
                direction: moved(s, years)?,
                flux: flux(s.vmag, magnitude_zero)?,
                colour: colour(star_temperature(s)?)?,
                constellation: constellation(s.ra, s.dec, 2000.0)?,
            })
        })
        .collect()
}
