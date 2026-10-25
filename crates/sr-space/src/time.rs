//! Rules 1 and 2: the proleptic Gregorian calendar, UTC to TAI to TT to TDB, and the frame clock of a `space`.

use super::{rotation_known, Frame, SpaceError};

/// The Julian day of 2000-01-01T12:00:00, the origin of ephemeris time.
pub const J2000_JD: f64 = 2451545.0;
/// TT − TAI in seconds.
pub const TT_MINUS_TAI: f64 = 32.184;
const TDB_K: f64 = 1.657e-3;
const TDB_EB: f64 = 1.671e-2;
const TDB_M0: f64 = 6.239996;
const TDB_M1: f64 = 1.99096871e-7;

/// NAIF `naif0012.tls`, DELTET/DELTA_AT: TAI − UTC after each leap second, from 1972-01-01.
pub const LEAP_SECONDS: [(i32, (i32, u32, u32)); 28] = [
    (10, (1972, 1, 1)),
    (11, (1972, 7, 1)),
    (12, (1973, 1, 1)),
    (13, (1974, 1, 1)),
    (14, (1975, 1, 1)),
    (15, (1976, 1, 1)),
    (16, (1977, 1, 1)),
    (17, (1978, 1, 1)),
    (18, (1979, 1, 1)),
    (19, (1980, 1, 1)),
    (20, (1981, 7, 1)),
    (21, (1982, 7, 1)),
    (22, (1983, 7, 1)),
    (23, (1985, 7, 1)),
    (24, (1988, 1, 1)),
    (25, (1990, 1, 1)),
    (26, (1991, 1, 1)),
    (27, (1992, 7, 1)),
    (28, (1993, 7, 1)),
    (29, (1994, 7, 1)),
    (30, (1996, 1, 1)),
    (31, (1997, 7, 1)),
    (32, (1999, 1, 1)),
    (33, (2006, 1, 1)),
    (34, (2009, 1, 1)),
    (35, (2012, 7, 1)),
    (36, (2015, 7, 1)),
    (37, (2017, 1, 1)),
];

/// A UTC date and time: year, month, day, hour, minute, second.
pub type Civil = (i32, u32, u32, u32, u32, f64);

/// The Julian day of a proleptic Gregorian date (Meeus, chapter 7).
pub fn julian_day(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: f64) -> f64 {
    let (mut y, mut m) = (year as f64, month as f64);
    if month <= 2 {
        y -= 1.0;
        m += 12.0;
    }
    let a = (y / 100.0).floor();
    let b = 2.0 - a + (a / 4.0).floor();
    (365.25 * (y + 4716.0)).floor() + (30.6001 * (m + 1.0)).floor() + day as f64 + b - 1524.5
        + (hour as f64 + minute as f64 / 60.0 + second / 3600.0) / 24.0
}

/// The proleptic Gregorian date of a Julian day.
pub fn calendar(jd: f64) -> Civil {
    let z = (jd + 0.5).floor();
    let f = jd + 0.5 - z;
    let alpha = ((z - 1867216.25) / 36524.25).floor();
    let a = z + 1.0 + alpha - (alpha / 4.0).floor();
    let b = a + 1524.0;
    let c = ((b - 122.1) / 365.25).floor();
    let d = (365.25 * c).floor();
    let e = ((b - d) / 30.6001).floor();
    let day = (b - d - (30.6001 * e).floor()) as u32;
    let month = if e < 14.0 { e - 1.0 } else { e - 13.0 } as u32;
    let year = if month > 2 { c - 4716.0 } else { c - 4715.0 } as i32;
    let seconds = f * 86400.0;
    let hour = (seconds / 3600.0).floor();
    let minute = ((seconds - hour * 3600.0) / 60.0).floor();
    (year, month, day, hour as u32, minute as u32, seconds - hour * 3600.0 - minute * 60.0)
}

/// Days from 1970-01-01 to a proleptic Gregorian date (Hinnant's `days_from_civil`).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn digits(s: &str, n: usize) -> Option<u32> {
    (s.len() == n && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse().ok()).flatten()
}

/// `YYYY-MM-DDThh:mm:ss[.f](Z|±hh:mm)` into the UTC date and time it names.
pub fn parse_utc(text: &str) -> Result<Civil, SpaceError> {
    let bad = || SpaceError::Time(format!("a UTC epoch is YYYY-MM-DDThh:mm:ss with Z or a +hh:mm offset: {text:?}"));
    let (stamp, offset) = if let Some(s) = text.strip_suffix('Z') {
        (s, 0i64)
    } else {
        let at = text.len().checked_sub(6).ok_or_else(bad)?;
        let (s, z) = text.split_at_checked(at).ok_or_else(bad)?;
        let sign = match z.as_bytes()[0] {
            b'+' => 1,
            b'-' => -1,
            _ => return Err(bad()),
        };
        let h = digits(z.get(1..3).ok_or_else(bad)?, 2).ok_or_else(bad)?;
        let m = digits(z.get(4..6).ok_or_else(bad)?, 2).ok_or_else(bad)?;
        if z.get(3..4) != Some(":") || h > 23 || m > 59 {
            return Err(bad());
        }
        (s, sign * (h as i64 * 60 + m as i64))
    };
    let (date, clock) = stamp.split_once('T').ok_or_else(bad)?;
    let mut d = date.split('-');
    let (y, mo, da) = (d.next(), d.next(), d.next());
    if d.next().is_some() {
        return Err(bad());
    }
    let year = digits(y.ok_or_else(bad)?, 4).ok_or_else(bad)? as i32;
    let month = digits(mo.ok_or_else(bad)?, 2).ok_or_else(bad)?;
    let day = digits(da.ok_or_else(bad)?, 2).ok_or_else(bad)?;
    let mut c = clock.splitn(3, ':');
    let hour = digits(c.next().ok_or_else(bad)?, 2).ok_or_else(bad)?;
    let minute = digits(c.next().ok_or_else(bad)?, 2).ok_or_else(bad)?;
    let sec_text = c.next().ok_or_else(bad)?;
    let (whole, frac) = sec_text.split_once('.').unwrap_or((sec_text, ""));
    digits(whole, 2).ok_or_else(bad)?;
    if sec_text.contains('.') && (frac.is_empty() || !frac.bytes().all(|b| b.is_ascii_digit())) {
        return Err(bad());
    }
    let second: f64 = sec_text.parse().map_err(|_| bad())?;
    let range = || SpaceError::Time(format!("the epoch has a field out of range: {text:?}"));
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || !(0.0..60.0).contains(&second)
    {
        return Err(range());
    }
    if year < 1 || day > days_in_month(year, month) {
        return Err(SpaceError::Time(format!("the epoch is not a calendar date: {text:?}")));
    }
    let local = (days_from_civil(year as i64, month, day) * 24 + hour as i64) * 60 + minute as i64;
    let utc = local - offset;
    let (days, mins) = (utc.div_euclid(1440), utc.rem_euclid(1440));
    let (uy, um, ud) = civil_from_days(days);
    Ok((uy as i32, um, ud, (mins / 60) as u32, (mins % 60) as u32, second))
}

/// A Julian day written as a decimal number (`xs:decimal`'s lexical form, TF2): XML whitespace around an optional
/// sign and digits with at most one point; no exponent, infinity or NaN.
fn julian_day_text(text: &str) -> Option<f64> {
    let t = text.trim_matches([' ', '\t', '\r', '\n']);
    let digits = t.strip_prefix(['+', '-']).unwrap_or(t);
    let lexical = digits.bytes().any(|c| c.is_ascii_digit())
        && digits.bytes().all(|c| c.is_ascii_digit() || c == b'.')
        && digits.bytes().filter(|&c| c == b'.').count() <= 1;
    lexical.then(|| t.parse().ok()).flatten()
}

/// TAI − UTC in whole seconds at a UTC Julian day; dates before 1972 are refused.
pub fn leap_seconds(jd_utc: f64) -> Result<i32, SpaceError> {
    let start = |(_, (y, m, d)): (i32, (i32, u32, u32))| julian_day(y, m, d, 0, 0, 0.0);
    if jd_utc < start(LEAP_SECONDS[0]) {
        return Err(SpaceError::Time("UTC with leap seconds starts on 1972-01-01".into()));
    }
    Ok(LEAP_SECONDS.iter().filter(|&&e| jd_utc >= start(e)).map(|e| e.0).next_back().unwrap_or(LEAP_SECONDS[0].0))
}

/// NAIF's formula (naif0012.tls): K sin(E), E = M + EB sin M, M = M0 + M1 t, t seconds of TT past J2000.
fn tdb_minus_tt_seconds(tt_seconds: f64) -> f64 {
    let m = TDB_M0 + TDB_M1 * tt_seconds;
    TDB_K * (m + TDB_EB * m.sin()).sin()
}

/// TDB − TT in seconds at a TT Julian day.
pub fn tdb_minus_tt(jd_tt: f64) -> f64 {
    tdb_minus_tt_seconds((jd_tt - J2000_JD) * 86400.0)
}

/// Ephemeris time (TDB seconds past J2000) of a UTC epoch string, carried in seconds so it is exact to the float.
pub fn et_from_utc(text: &str) -> Result<f64, SpaceError> {
    let (y, mo, d, h, mi, s) = parse_utc(text)?;
    let leap = leap_seconds(julian_day(y, mo, d, h, mi, s))?;
    let days = days_from_civil(y as i64, mo, d) - days_from_civil(2000, 1, 1);
    let tt = days as f64 * 86400.0 + h as f64 * 3600.0 + mi as f64 * 60.0 + s - 43200.0 + leap as f64 + TT_MINUS_TAI;
    Ok(tt + tdb_minus_tt_seconds(tt))
}

/// The time scale of a `space` epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeScale {
    Utc,
    Tt,
    Tdb,
}

/// The `space` element: the frame clock's epoch as ephemeris time, its rate in seconds of ephemeris time per
/// second of scene time, the origin body and the frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Space {
    pub et0: f64,
    pub rate: f64,
    pub origin: i32,
    pub frame: Frame,
}

impl Space {
    /// A UTC epoch is ISO 8601 text; a TT or TDB epoch is a Julian day.
    pub fn new(epoch: &str, scale: TimeScale, rate: f64, origin: i32, frame: Frame) -> Result<Space, SpaceError> {
        let et0 = match scale {
            TimeScale::Utc => et_from_utc(epoch)?,
            TimeScale::Tt | TimeScale::Tdb => {
                let jd = julian_day_text(epoch)
                    .ok_or_else(|| SpaceError::Time(format!("a TT or TDB epoch is a Julian day: {epoch:?}")))?;
                let tt = (jd - J2000_JD) * 86400.0;
                if scale == TimeScale::Tt {
                    tt + tdb_minus_tt_seconds(tt)
                } else {
                    tt
                }
            }
        };
        if !(rate >= 0.0 && rate.is_finite()) {
            return Err(SpaceError::Time(format!("rate is a finite non-negative number, not {rate}")));
        }
        if let Frame::BodyFixed(body) = frame {
            if !rotation_known(body) {
                return Err(SpaceError::Frame(format!(
                    "bodyFixed needs a frameBody with rotation elements, not {body}"
                )));
            }
        }
        Ok(Space { et0, rate, origin, frame })
    }

    /// Ephemeris time of a frame: epoch + frame / fps × rate.
    pub fn frame_time(&self, frame: u64, fps: f64) -> Result<f64, SpaceError> {
        if !(fps > 0.0 && fps.is_finite()) {
            return Err(SpaceError::Time(format!("fps is positive, not {fps}")));
        }
        Ok(self.et0 + frame as f64 / fps * self.rate)
    }
}
