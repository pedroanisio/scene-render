//! The sun of a dome light with `sky="physical"`: rules 1 to 6 of the sr-core draft "Add the physical sun, moon and
//! sky to dome lights" (`srep/srep-0000-physical-sky.md`, A1 of SREP 12). The solar position algorithm of Reda and
//! Andreas (2004, NREL/TP-560-34302), written from the report's equations and tables, gives the topocentric zenith
//! and azimuth of a civil date for an observer; [`sun_direction`] turns them into scene axes. Angles are degrees.

mod tables;

pub use tables::{B0, B1, L0, L1, L2, L3, L4, L5, NUTATION_ABCD, NUTATION_Y, R0, R1, R2, R3, R4};

use crate::SpaceError;

/// The sun's apparent radius, degrees (the report's 3.14.1).
pub const SUN_RADIUS_DEG: f64 = 0.26667;
/// The atmospheric refraction at sunrise and sunset, degrees: refraction applies while the sun's elevation is above
/// −(SUN_RADIUS_DEG + REFRACTION).
pub const REFRACTION: f64 = 0.5667;
/// TT − UT when a document gives no `deltaT`, seconds; allowed for the years [`DELTA_T_FREE_YEARS`].
pub const DEFAULT_DELTA_T: f64 = 69.0;
/// The years in which `deltaT` may be left out (SKY2).
pub const DELTA_T_FREE_YEARS: (i32, i32) = (1900, 2100);
/// The bounds of `deltaT`, seconds (SKY3).
pub const DELTA_T_RANGE: (f64, f64) = (-8000.0, 8000.0);
/// The years the algorithm covers (SKY2).
pub const SPA_YEARS: (i32, i32) = (-2000, 6000);

/// A civil date and time with its offset from UTC, as a document's `date` writes it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DateTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: f64,
    /// Hours east of UTC.
    pub timezone: f64,
}

/// A ground observer and the air the sun is refracted through.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Observer {
    /// Degrees north, −90 to 90.
    pub latitude: f64,
    /// Degrees east, −180 to 180.
    pub longitude: f64,
    /// Metres above the ellipsoid.
    pub altitude: f64,
    /// hPa, at least 0; 0 turns refraction off.
    pub pressure: f64,
    /// °C, above −273.
    pub temperature: f64,
    /// TT − UT, seconds; `None` is [`DEFAULT_DELTA_T`] within [`DELTA_T_FREE_YEARS`].
    pub delta_t: Option<f64>,
}

/// The report's time scales of a Julian day (3.1.2 to 3.1.5).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeScales {
    pub jd: f64,
    /// The ephemeris day, jd + ΔT.
    pub jde: f64,
    /// Julian centuries of jd from J2000.
    pub jc: f64,
    /// Julian ephemeris centuries.
    pub jce: f64,
    /// Julian ephemeris millennia.
    pub jme: f64,
}

/// Every intermediate of the algorithm the report's example prints, and the topocentric place of the sun.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SunPosition {
    pub jd: f64,
    pub jde: f64,
    pub heliocentric_longitude: f64,
    pub heliocentric_latitude: f64,
    /// The Earth's radius vector, astronomical units.
    pub radius: f64,
    pub geocentric_longitude: f64,
    pub geocentric_latitude: f64,
    pub nutation_longitude: f64,
    pub nutation_obliquity: f64,
    pub obliquity: f64,
    pub aberration: f64,
    pub apparent_longitude: f64,
    pub sidereal_time: f64,
    pub right_ascension: f64,
    pub declination: f64,
    pub hour_angle: f64,
    pub topocentric_right_ascension: f64,
    pub topocentric_declination: f64,
    pub topocentric_hour_angle: f64,
    /// The topocentric zenith angle after refraction.
    pub zenith: f64,
    /// The topocentric azimuth from north eastward, [0, 360).
    pub azimuth: f64,
    /// The astronomers' azimuth, from south westward, [0, 360).
    pub astronomers_azimuth: f64,
}

impl SunPosition {
    /// The report's 3.16: the incidence angle on a surface of `slope` degrees from the horizontal whose normal turns
    /// `azimuth_rotation` degrees from south toward west.
    pub fn incidence(&self, slope: f64, azimuth_rotation: f64) -> f64 {
        let (z, w) = (self.zenith.to_radians(), slope.to_radians());
        let c =
            z.cos() * w.cos() + w.sin() * z.sin() * (self.astronomers_azimuth - azimuth_rotation).to_radians().cos();
        c.clamp(-1.0, 1.0).acos().to_degrees()
    }
}

fn limit360(x: f64) -> f64 {
    x.rem_euclid(360.0)
}

fn sky(message: impl Into<String>) -> SpaceError {
    SpaceError::Sky(message.into())
}

/// Reads `n` ASCII digits of `s` from `at`.
fn digits(s: &[u8], at: usize, n: usize) -> Option<u32> {
    let part = s.get(at..at + n)?;
    part.iter().all(u8::is_ascii_digit).then(|| part.iter().fold(0, |v, c| v * 10 + (c - b'0') as u32))
}

/// Rule 1: `YYYY-MM-DDThh:mm:ss[.f](Z|±hh:mm)`, the year with an optional minus.
pub fn parse_datetime(text: &str) -> Result<DateTime, SpaceError> {
    let form = || sky(format!("date is YYYY-MM-DDThh:mm:ss with a Z or +hh:mm offset: {text:?}"));
    let b = text.as_bytes();
    let neg = b.first() == Some(&b'-');
    let s = &b[neg as usize..];
    let year = digits(s, 0, 4).ok_or_else(form)? as i32 * if neg { -1 } else { 1 };
    let sep = |at: usize, c: u8| (s.get(at) == Some(&c)).then_some(()).ok_or_else(form);
    sep(4, b'-')?;
    let month = digits(s, 5, 2).ok_or_else(form)?;
    sep(7, b'-')?;
    let day = digits(s, 8, 2).ok_or_else(form)?;
    sep(10, b'T')?;
    let hour = digits(s, 11, 2).ok_or_else(form)?;
    sep(13, b':')?;
    let minute = digits(s, 14, 2).ok_or_else(form)?;
    sep(16, b':')?;
    digits(s, 17, 2).ok_or_else(form)?;
    let mut end = 19;
    if s.get(end) == Some(&b'.') {
        end += 1;
        let start = end;
        while s.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
        if end == start {
            return Err(form());
        }
    }
    // the digits are ASCII, so the slice is text
    let second: f64 = std::str::from_utf8(&s[17..end]).map_err(|_| form())?.parse().map_err(|_| form())?;
    let timezone = match &s[end..] {
        b"Z" => 0.0,
        [sign @ (b'+' | b'-'), ..] if s.len() == end + 6 && s[end + 3] == b':' => {
            let h = digits(s, end + 1, 2).ok_or_else(form)?;
            let m = digits(s, end + 4, 2).ok_or_else(form)?;
            (if *sign == b'+' { 1.0 } else { -1.0 }) * (h as f64 + m as f64 / 60.0)
        }
        _ => return Err(form()),
    };
    if !((1..=12).contains(&month)
        && (1..=31).contains(&day)
        && hour <= 23
        && minute <= 59
        && (0.0..61.0).contains(&second))
    {
        return Err(sky(format!("date has a field out of range: {text:?}")));
    }
    Ok(DateTime { year, month, day, hour, minute, second, timezone })
}

/// Rule 1, the report's equation 4: the Julian day of a civil date, Gregorian after 1582-10-15.
pub fn julian_day(d: &DateTime) -> f64 {
    let day = d.day as f64 + (d.hour as f64 + d.minute as f64 / 60.0 + d.second / 3600.0 - d.timezone) / 24.0;
    let (mut y, mut m) = (d.year as f64, d.month as f64);
    if m <= 2.0 {
        y -= 1.0;
        m += 12.0;
    }
    let mut jd = (365.25 * (y + 4716.0)).floor() + (30.6001 * (m + 1.0)).floor() + day - 1524.5;
    if jd > 2299160.0 {
        let a = (y / 100.0).floor();
        jd += 2.0 - a + (a / 4.0).floor();
    }
    jd
}

/// Rule 1, the report's 3.1.2 to 3.1.5.
pub fn time_scales(jd: f64, delta_t: f64) -> TimeScales {
    let jde = jd + delta_t / 86400.0;
    let jc = (jd - 2451545.0) / 36525.0;
    let jce = (jde - 2451545.0) / 36525.0;
    TimeScales { jd, jde, jc, jce, jme: jce / 10.0 }
}

fn series(table: &[(f64, f64, f64)], x: f64) -> f64 {
    table.iter().map(|&(a, b, c)| a * (b + c * x).cos()).sum()
}

fn polynomial(tables: &[&[(f64, f64, f64)]], x: f64) -> f64 {
    tables.iter().zip(0..).map(|(t, i)| series(t, x) * x.powi(i)).sum::<f64>() / 1e8
}

/// Rule 2, the report's 3.2.1 to 3.2.4: degrees in [0, 360).
pub fn heliocentric_longitude(jme: f64) -> f64 {
    limit360(polynomial(&[&L0, &L1, &L2, &L3, &L4, &L5], jme).to_degrees())
}

/// Rule 2, the report's 3.2.7.
pub fn heliocentric_latitude(jme: f64) -> f64 {
    polynomial(&[&B0, &B1], jme).to_degrees()
}

/// Rule 2, the report's 3.2.8: astronomical units.
pub fn earth_radius_vector(jme: f64) -> f64 {
    polynomial(&[&R0, &R1, &R2, &R3, &R4], jme)
}

/// Rule 3, the report's 3.4: the nutation in longitude and in obliquity.
pub fn nutation(jce: f64) -> (f64, f64) {
    let (t2, t3) = (jce * jce, jce * jce * jce);
    let x = [
        297.85036 + 445267.111480 * jce - 0.0019142 * t2 + t3 / 189474.0,
        357.52772 + 35999.050340 * jce - 0.0001603 * t2 - t3 / 300000.0,
        134.96298 + 477198.867398 * jce + 0.0086972 * t2 + t3 / 56250.0,
        93.27191 + 483202.017538 * jce - 0.0036825 * t2 + t3 / 327270.0,
        125.04452 - 1934.136261 * jce + 0.0020708 * t2 + t3 / 450000.0,
    ];
    let (mut dpsi, mut deps) = (0.0, 0.0);
    for (y, &(a, b, c, d)) in NUTATION_Y.iter().zip(&NUTATION_ABCD) {
        let arg = y.iter().zip(&x).map(|(&yi, xi)| yi as f64 * xi).sum::<f64>().to_radians();
        dpsi += (a + b * jce) * arg.sin();
        deps += (c + d * jce) * arg.cos();
    }
    (dpsi / 36000000.0, deps / 36000000.0)
}

/// Rule 3, the report's 3.5: the mean obliquity polynomial plus the nutation in obliquity.
pub fn true_obliquity(jme: f64, deps: f64) -> f64 {
    let u = jme / 10.0;
    let c = [84381.448, -4680.93, -1.55, 1999.25, -51.38, -249.67, -39.05, 7.12, 27.87, 5.79, 2.45];
    c.iter().rev().fold(0.0, |acc, k| acc * u + k) / 3600.0 + deps
}

/// Rule 4, the report's 3.8: the apparent sidereal time at Greenwich.
pub fn sidereal_time(jd: f64, jc: f64, dpsi: f64, eps: f64) -> f64 {
    let nu0 =
        limit360(280.46061837 + 360.98564736629 * (jd - 2451545.0) + 0.000387933 * jc * jc - jc * jc * jc / 38710000.0);
    nu0 + dpsi * eps.to_radians().cos()
}

/// Rule 4, the report's 3.9 and 3.10: right ascension in [0, 360) and declination of ecliptic coordinates.
pub fn equatorial(lambda: f64, beta: f64, eps: f64) -> (f64, f64) {
    let (l, b, e) = (lambda.to_radians(), beta.to_radians(), eps.to_radians());
    let alpha = (l.sin() * e.cos() - b.tan() * e.sin()).atan2(l.cos());
    let delta = (b.sin() * e.cos() + b.cos() * e.sin() * l.sin()).asin();
    (limit360(alpha.to_degrees()), delta.to_degrees())
}

/// Rule 4, the report's 3.12: right ascension, declination and hour angle shifted for the parallax `xi` of a body
/// seen from `latitude` at `altitude` metres on the flattened Earth.
pub fn topocentric(alpha: f64, delta: f64, h: f64, latitude: f64, altitude: f64, xi: f64) -> (f64, f64, f64) {
    let phi = latitude.to_radians();
    let u = (0.99664719 * phi.tan()).atan();
    let x = u.cos() + altitude / 6378140.0 * phi.cos();
    let y = 0.99664719 * u.sin() + altitude / 6378140.0 * phi.sin();
    let (xr, hr, dr) = (xi.to_radians(), h.to_radians(), delta.to_radians());
    let dalpha = (-x * xr.sin() * hr.sin()).atan2(dr.cos() - x * xr.sin() * hr.cos());
    let delta_t = ((dr.sin() - y * xr.sin()) * dalpha.cos()).atan2(dr.cos() - x * xr.sin() * hr.cos());
    (alpha + dalpha.to_degrees(), delta_t.to_degrees(), h - dalpha.to_degrees())
}

fn refraction_with(zenith: f64, pressure: f64, temperature: f64, refraction: f64) -> f64 {
    let e0 = 90.0 - zenith;
    if pressure <= 0.0 || e0 < -(SUN_RADIUS_DEG + refraction) {
        return 0.0;
    }
    (pressure / 1010.0) * (283.0 / (273.0 + temperature)) * 1.02 / (60.0 * (e0 + 10.3 / (e0 + 5.11)).to_radians().tan())
}

/// Rule 5, the report's 3.14.1: the refraction added to the elevation of a body at the geometric `zenith`; zero
/// while the sun's top limb is below the horizon and at zero pressure.
pub fn refraction_correction(zenith: f64, pressure: f64, temperature: f64) -> f64 {
    refraction_with(zenith, pressure, temperature, REFRACTION)
}

/// Rule 5, the report's 3.14 and 3.15: the zenith after refraction, the azimuth from north eastward and the
/// astronomers' azimuth of a topocentric declination and hour angle.
pub fn horizontal(delta: f64, h: f64, latitude: f64, pressure: f64, temperature: f64) -> (f64, f64, f64) {
    let (phi, dr, hr) = (latitude.to_radians(), delta.to_radians(), h.to_radians());
    let e0 = (phi.sin() * dr.sin() + phi.cos() * dr.cos() * hr.cos()).asin().to_degrees();
    let e = e0 + refraction_correction(90.0 - e0, pressure, temperature);
    let gamma = limit360(hr.sin().atan2(hr.cos() * phi.sin() - dr.tan() * phi.cos()).to_degrees());
    (90.0 - e, limit360(gamma + 180.0), gamma)
}

/// Rules 1 to 5 end to end for a date the observer has checked (`delta_t` given).
pub fn sun_position(date: &DateTime, o: &Observer, delta_t: f64) -> SunPosition {
    let jd = julian_day(date);
    let t = time_scales(jd, delta_t);
    let l_h = heliocentric_longitude(t.jme);
    let b_h = heliocentric_latitude(t.jme);
    let r = earth_radius_vector(t.jme);
    let theta = limit360(l_h + 180.0);
    let beta = -b_h;
    let (dpsi, deps) = nutation(t.jce);
    let eps = true_obliquity(t.jme, deps);
    let dtau = -20.4898 / (3600.0 * r);
    let lambda = theta + dpsi + dtau;
    let nu = sidereal_time(jd, t.jc, dpsi, eps);
    let (alpha, delta) = equatorial(lambda, beta, eps);
    let h = limit360(nu + o.longitude - alpha);
    let xi = 8.794 / (3600.0 * r);
    let (alpha_t, delta_t_, h_t) = topocentric(alpha, delta, h, o.latitude, o.altitude, xi);
    let (zenith, azimuth, gamma) = horizontal(delta_t_, h_t, o.latitude, o.pressure, o.temperature);
    SunPosition {
        jd,
        jde: t.jde,
        heliocentric_longitude: l_h,
        heliocentric_latitude: b_h,
        radius: r,
        geocentric_longitude: theta,
        geocentric_latitude: beta,
        nutation_longitude: dpsi,
        nutation_obliquity: deps,
        obliquity: eps,
        aberration: dtau,
        apparent_longitude: lambda,
        sidereal_time: nu,
        right_ascension: alpha,
        declination: delta,
        hour_angle: h,
        topocentric_right_ascension: alpha_t,
        topocentric_declination: delta_t_,
        topocentric_hour_angle: h_t,
        zenith,
        azimuth,
        astronomers_azimuth: gamma,
    }
}

/// Rule 6: the unit direction toward a body of `zenith` and `azimuth` (from north eastward) in scene axes, up −y,
/// azimuth 0 along +z and 90° along +x.
pub fn sun_direction(zenith: f64, azimuth: f64) -> [f64; 3] {
    let (z, a) = (zenith.to_radians(), azimuth.to_radians());
    [z.sin() * a.sin(), -z.cos(), z.sin() * a.cos()]
}

/// SKY2 and SKY3 for a document's `date` and observer: the parsed date and the ΔT it is read with.
pub fn check_observer(date: &str, o: &Observer) -> Result<(DateTime, f64), SpaceError> {
    let d = parse_datetime(date)?;
    if !((-90.0..=90.0).contains(&o.latitude) && (-180.0..=180.0).contains(&o.longitude)) {
        return Err(sky("latitude is -90 to 90 and longitude -180 to 180"));
    }
    if !(o.pressure >= 0.0 && o.pressure.is_finite() && o.temperature.is_finite() && o.temperature > -273.0) {
        return Err(sky("pressure is at least 0 hPa and temperature above absolute zero"));
    }
    if !(SPA_YEARS.0..=SPA_YEARS.1).contains(&d.year) {
        return Err(sky("the solar position algorithm covers the years -2000 to 6000"));
    }
    let delta_t = match o.delta_t {
        Some(v) => v,
        None if (DELTA_T_FREE_YEARS.0..=DELTA_T_FREE_YEARS.1).contains(&d.year) => DEFAULT_DELTA_T,
        None => return Err(sky("deltaT must be given for dates outside 1900 to 2100")),
    };
    if !(DELTA_T_RANGE.0..=DELTA_T_RANGE.1).contains(&delta_t) {
        return Err(sky("deltaT is -8000 to 8000 seconds"));
    }
    if !o.altitude.is_finite() {
        return Err(sky("altitude is a finite number of metres"));
    }
    Ok((d, delta_t))
}

/// Rule 6: the sun of a dome light with `sky="physical"` and its direction in scene axes.
pub fn sun_from_document(date: &str, o: &Observer) -> Result<(SunPosition, [f64; 3]), SpaceError> {
    let (d, delta_t) = check_observer(date, o)?;
    let s = sun_position(&d, o, delta_t);
    let dir = sun_direction(s.zenith, s.azimuth);
    Ok((s, dir))
}
