//! Rules 6 and 7: the ICRF, the ecliptic of J2000 and the body-fixed frames of the IAU 2009 rotation elements
//! (NAIF `pck00010.tpc`), the mapping into scene pixels (SREP 14 rule 4), and bodies placed at a frame's time.

use super::elements::approximate_relative;
use super::spk::Spk;
use super::time::Space;
use super::{SpaceError, OBLIQUITY_J2000};

/// A 3 × 3 matrix, rows first.
pub type Mat3 = [[f64; 3]; 3];

/// (α₀, α₁), (δ₀, δ₁), (W₀, W₁, W₂) in degrees, T Julian centuries and d days of TDB past J2000.
type Elements = (i32, [f64; 2], [f64; 2], [f64; 3]);

const ROTATION: [Elements; 10] = [
    (10, [286.13, 0.0], [63.87, 0.0], [84.176, 14.18440, 0.0]),
    (199, [281.0097, -0.0328], [61.4143, -0.0049], [329.5469, 6.1385025, 0.0]),
    (299, [272.76, 0.0], [67.16, 0.0], [160.20, -1.4813688, 0.0]),
    (399, [0.0, -0.641], [90.0, -0.557], [190.147, ROTATION_EARTH_W1, 0.0]),
    (499, [317.68143, -0.1061], [52.88650, -0.0609], [176.630, 350.89198226, 0.0]),
    (599, [268.056595, -0.006499], [64.495303, 0.002413], [284.95, 870.5360000, 0.0]),
    (699, [40.589, -0.036], [83.537, -0.004], [38.90, 810.7939024, 0.0]),
    (799, [257.311, 0.0], [-15.175, 0.0], [203.81, -501.1600928, 0.0]),
    (899, [299.36, 0.0], [43.46, 0.0], [253.18, 536.3128492, 0.0]),
    (301, [269.9949, 0.0031], [66.5392, 0.0130], [38.3213, 13.17635815, -1.4e-12]),
];

/// The Earth's W₁, degrees per day.
pub const ROTATION_EARTH_W1: f64 = 360.9856235;

/// The Moon's nutation-precession angles E1 to E13 (degrees and degrees per century) and their coefficients in α,
/// δ and W.
const MOON_ANGLES: [[f64; 2]; 13] = [
    [125.045, -1935.5364525],
    [250.089, -3871.072905],
    [260.008, 475263.3328725],
    [176.625, 487269.629985],
    [357.529, 35999.0509575],
    [311.589, 964468.49931],
    [134.963, 477198.869325],
    [276.617, 12006.300765],
    [34.226, 63863.5132425],
    [15.134, -5806.6093575],
    [119.743, 131.84064],
    [239.961, 6003.1503825],
    [25.053, 473327.79642],
];
const MOON_RA: [f64; 13] = [-3.8787, -0.1204, 0.0700, -0.0172, 0.0, 0.0072, 0.0, 0.0, 0.0, -0.0052, 0.0, 0.0, 0.0043];
const MOON_DEC: [f64; 13] =
    [1.5419, 0.0239, -0.0278, 0.0068, 0.0, -0.0029, 0.0009, 0.0, 0.0, 0.0008, 0.0, 0.0, -0.0009];
const MOON_PM: [f64; 13] =
    [3.5610, 0.1208, -0.0642, 0.0158, 0.0252, -0.0066, -0.0047, -0.0046, 0.0028, 0.0052, 0.0040, 0.0019, -0.0044];

const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
/// ICRF (x, y, z) to scene axes: scene x = x, scene y = −z (up is −y), scene z = y.
pub const SCENE_FROM_ICRF_AXES: Mat3 = [[1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]];

/// The frame of a `space`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frame {
    Icrf,
    EclipticJ2000,
    /// The body-fixed frame of a body with rotation elements.
    BodyFixed(i32),
}

/// Whether rule 6 has rotation elements for a body.
pub fn rotation_known(body: i32) -> bool {
    ROTATION.iter().any(|r| r.0 == body)
}

/// (right ascension, declination, prime meridian angle W) in degrees at ephemeris time et.
pub fn rotation_elements(body: i32, et: f64) -> Result<(f64, f64, f64), SpaceError> {
    let &(_, ra_, dec_, w_) = ROTATION
        .iter()
        .find(|r| r.0 == body)
        .ok_or_else(|| SpaceError::Frame(format!("no rotation elements for body {body}")))?;
    let d = et / 86400.0;
    let t = d / 36525.0;
    let (mut ra, mut dec, mut w) = (ra_[0] + ra_[1] * t, dec_[0] + dec_[1] * t, w_[0] + w_[1] * d + w_[2] * d * d);
    if body == 301 {
        for k in 0..13 {
            let ang = (MOON_ANGLES[k][0] + MOON_ANGLES[k][1] * t).to_radians();
            ra += MOON_RA[k] * ang.sin();
            dec += MOON_DEC[k] * ang.cos();
            w += MOON_PM[k] * ang.sin();
        }
    }
    Ok((ra, dec, w))
}

/// The unit vector of a body's north pole in the ICRF.
pub fn pole_icrf(body: i32, et: f64) -> Result<[f64; 3], SpaceError> {
    let (ra, dec, _) = rotation_elements(body, et)?;
    let (r, d) = (ra.to_radians(), dec.to_radians());
    Ok([d.cos() * r.cos(), d.cos() * r.sin(), d.sin()])
}

fn rz(deg: f64) -> Mat3 {
    let (s, c) = deg.to_radians().sin_cos();
    [[c, s, 0.0], [-s, c, 0.0], [0.0, 0.0, 1.0]]
}

fn rx(deg: f64) -> Mat3 {
    let (s, c) = deg.to_radians().sin_cos();
    [[1.0, 0.0, 0.0], [0.0, c, s], [0.0, -s, c]]
}

pub fn mat_mul(a: &Mat3, b: &Mat3) -> Mat3 {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}

pub fn transpose(m: &Mat3) -> Mat3 {
    std::array::from_fn(|i| std::array::from_fn(|j| m[j][i]))
}

pub fn mat_apply(m: &Mat3, v: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| (0..3).map(|k| m[i][k] * v[k]).sum())
}

pub fn determinant(m: &Mat3) -> f64 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}

/// ICRF components to the body-fixed frame: Rz(W) Rx(90° − δ₀) Rz(90° + α₀).
pub fn body_fixed_from_icrf(body: i32, et: f64) -> Result<Mat3, SpaceError> {
    let (ra, dec, w) = rotation_elements(body, et)?;
    Ok(mat_mul(&rz(w), &mat_mul(&rx(90.0 - dec), &rz(90.0 + ra))))
}

/// ICRF to the ecliptic and equinox of J2000: a rotation about x by the obliquity.
pub fn ecliptic_from_icrf() -> Mat3 {
    rx(OBLIQUITY_J2000)
}

impl Space {
    /// ICRF components to the space's frame at ephemeris time et.
    pub fn frame_matrix(&self, et: f64) -> Result<Mat3, SpaceError> {
        match self.frame {
            Frame::Icrf => Ok(IDENTITY),
            Frame::EclipticJ2000 => Ok(ecliptic_from_icrf()),
            Frame::BodyFixed(body) => body_fixed_from_icrf(body, et),
        }
    }

    /// A position in km (ICRF axes) about the origin's position, in scene pixels through the space's frame and
    /// `physics/@pixelsPerMeter`: (x, y, z) to (x, −z, y) × 1000 × pixelsPerMeter.
    pub fn scene_from_icrf(
        &self,
        p_km: [f64; 3],
        origin_km: [f64; 3],
        pixels_per_meter: f64,
        et: f64,
    ) -> Result<[f64; 3], SpaceError> {
        if !(pixels_per_meter > 0.0 && pixels_per_meter.is_finite()) {
            return Err(SpaceError::Frame(format!("pixelsPerMeter is positive, not {pixels_per_meter}")));
        }
        let q = mat_apply(
            &self.frame_matrix(et)?,
            [p_km[0] - origin_km[0], p_km[1] - origin_km[1], p_km[2] - origin_km[2]],
        );
        let scale = 1000.0 * pixels_per_meter;
        Ok([q[0] * scale, -q[2] * scale, q[1] * scale])
    }
}

/// A body's place in the scene at a frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// The frame's ephemeris time.
    pub et: f64,
    /// Scene pixels about the space's origin.
    pub position: [f64; 3],
    /// The body-fixed axes in scene axes (rows first); the space's frame for a body without rotation elements.
    pub orientation: Mat3,
}

/// Where a space's positions come from: an ephemeris file (rule 4) or the approximate elements (rule 5).
#[derive(Debug, Clone, Copy)]
pub enum Positions<'a> {
    File(&'a Spk),
    Approximate,
}

impl Positions<'_> {
    /// A body's position about the origin in km, ICRF axes, at ephemeris time et.
    pub fn position(&self, body: i32, origin: i32, et: f64) -> Result<[f64; 3], SpaceError> {
        match self {
            Positions::File(spk) => spk.position(body, origin, et),
            Positions::Approximate => approximate_relative(body, origin, et),
        }
    }
}

/// Rule 7: a body's position and body-fixed orientation in the space at a frame.
pub fn place_body(
    spk: &Spk,
    space: &Space,
    body: i32,
    frame: u64,
    fps: f64,
    pixels_per_meter: f64,
) -> Result<Placement, SpaceError> {
    place_at(Positions::File(spk), space, body, space.frame_time(frame, fps)?, pixels_per_meter)
}

/// Rule 7 at ephemeris time et, with the positions of either source.
pub fn place_at(
    positions: Positions<'_>,
    space: &Space,
    body: i32,
    et: f64,
    pixels_per_meter: f64,
) -> Result<Placement, SpaceError> {
    let p = positions.position(body, space.origin, et)?;
    let position = space.scene_from_icrf(p, [0.0; 3], pixels_per_meter, et)?;
    let f = space.frame_matrix(et)?;
    let axes = if rotation_known(body) { mat_mul(&f, &transpose(&body_fixed_from_icrf(body, et)?)) } else { f };
    let orientation = mat_mul(&SCENE_FROM_ICRF_AXES, &mat_mul(&axes, &transpose(&SCENE_FROM_ICRF_AXES)));
    Ok(Placement { et, position, orientation })
}
