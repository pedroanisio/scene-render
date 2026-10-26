//! The moon of a dome light with `sky="physical"`: rule 7 of the sr-core physical-sky draft
//! (`srep/srep-0000-physical-sky.md`). The low-precision lunar theory of Montenbruck and Pfleger gives the
//! geocentric ecliptic place of the date (better than 0.1° over the 20th and 21st centuries) and the Almanac's
//! horizontal parallax the distance; the observer, the nutation, the obliquity and the sidereal time are the sun's
//! ([`crate::sun`]). Angles are degrees.

use std::f64::consts::TAU;

use crate::sun::{check_observer, equatorial, horizontal, sun_direction, sun_position, topocentric, Observer};
use crate::{SpaceError, AU_KM};

/// The moon's mean radius, km.
pub const MOON_RADIUS_KM: f64 = 1737.4;
/// The Earth's equatorial radius of the parallax, km.
pub const EARTH_RADIUS_KM: f64 = 6378.14;

const ARCSECONDS: f64 = 3600.0 * 180.0 / std::f64::consts::PI;

/// The geocentric moon of the series.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoonPosition {
    /// Ecliptic longitude of the date, [0, 360).
    pub longitude: f64,
    /// Ecliptic latitude.
    pub latitude: f64,
    /// km from the Earth's centre.
    pub distance: f64,
    /// The equatorial horizontal parallax.
    pub parallax: f64,
}

/// The moon an observer sees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Moon {
    /// The topocentric zenith angle after refraction.
    pub zenith: f64,
    /// The azimuth from north eastward, [0, 360).
    pub azimuth: f64,
    /// km from the Earth's centre.
    pub distance: f64,
    /// The disc's angular diameter from [`MOON_RADIUS_KM`].
    pub angular_diameter: f64,
    /// The illuminated fraction of the disc, 0 (new) to 1 (full).
    pub illuminated: f64,
    /// The phase angle, sun–moon–Earth.
    pub phase_angle: f64,
}

fn frac(x: f64) -> f64 {
    x - x.floor()
}

/// Rule 7: the moon of the series at ephemeris day `jde`.
pub fn moon_position(jde: f64) -> MoonPosition {
    let t = (jde - 2451545.0) / 36525.0;
    let l0 = frac(0.606433 + 1336.855225 * t);
    let l = TAU * frac(0.374897 + 1325.552410 * t);
    let ls = TAU * frac(0.993133 + 99.997361 * t);
    let d = TAU * frac(0.827361 + 1236.853086 * t);
    let f = TAU * frac(0.259086 + 1342.227825 * t);
    let sin = f64::sin;
    let dl = 22640.0 * sin(l) - 4586.0 * sin(l - 2.0 * d) + 2370.0 * sin(2.0 * d) + 769.0 * sin(2.0 * l)
        - 668.0 * sin(ls)
        - 412.0 * sin(2.0 * f)
        - 212.0 * sin(2.0 * l - 2.0 * d)
        - 206.0 * sin(l + ls - 2.0 * d)
        + 192.0 * sin(l + 2.0 * d)
        - 165.0 * sin(ls - 2.0 * d)
        - 125.0 * sin(d)
        - 110.0 * sin(l + ls)
        + 148.0 * sin(l - ls)
        - 55.0 * sin(2.0 * f - 2.0 * d);
    let s = f + (dl + 412.0 * sin(2.0 * f) + 541.0 * sin(ls)) / ARCSECONDS;
    let h = f - 2.0 * d;
    let n = -526.0 * sin(h) + 44.0 * sin(l + h) - 31.0 * sin(-l + h) - 23.0 * sin(ls + h) + 11.0 * sin(-ls + h)
        - 25.0 * sin(-2.0 * l + f)
        + 21.0 * sin(-l + f);
    let longitude = TAU * frac(l0 + dl / 1296000.0);
    let latitude = (18520.0 * sin(s) + n) / ARCSECONDS;
    let parallax =
        0.9508 + 0.0518 * l.cos() + 0.0095 * (l - 2.0 * d).cos() + 0.0078 * (2.0 * d).cos() + 0.0028 * (2.0 * l).cos();
    MoonPosition {
        longitude: longitude.to_degrees(),
        latitude: latitude.to_degrees(),
        distance: EARTH_RADIUS_KM / parallax.to_radians().sin(),
        parallax,
    }
}

/// Rule 7: the moon of a dome light's `date` for its observer (SKY2 and SKY3 as for the sun) and its direction in
/// scene axes.
pub fn moon_from_document(date: &str, o: &Observer) -> Result<(Moon, [f64; 3]), SpaceError> {
    let (d, delta_t) = check_observer(date, o)?;
    let sun = sun_position(&d, o, delta_t);
    let m = moon_position(sun.jde);
    let (alpha, delta) = equatorial(m.longitude + sun.nutation_longitude, m.latitude, sun.obliquity);
    let hour = (sun.sidereal_time + o.longitude - alpha).rem_euclid(360.0);
    let (_, delta_t_, h_t) = topocentric(alpha, delta, hour, o.latitude, o.altitude, m.parallax);
    let (zenith, azimuth, _) = horizontal(delta_t_, h_t, o.latitude, o.pressure, o.temperature);
    // the elongation from the sun and the phase angle with the two distances (Meeus 48.2 and 48.3)
    let psi = (m.latitude.to_radians().cos() * (m.longitude - sun.geocentric_longitude).to_radians().cos())
        .clamp(-1.0, 1.0)
        .acos();
    let r_sun = sun.radius * AU_KM;
    let i = (r_sun * psi.sin()).atan2(m.distance - r_sun * psi.cos());
    let moon = Moon {
        zenith,
        azimuth,
        distance: m.distance,
        angular_diameter: (2.0 * (MOON_RADIUS_KM / m.distance).asin()).to_degrees(),
        illuminated: (1.0 + i.cos()) / 2.0,
        phase_angle: i.to_degrees(),
    };
    Ok((moon, sun_direction(zenith, azimuth)))
}
