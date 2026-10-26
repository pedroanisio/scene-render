//! The sr-core planets draft (`srep/srep-0000-planets.md`, SP3 of SREP 12): a body's radii and spheroid, its
//! textures' placement on the engine's UV sphere, the limb, the terminator and the phase, the night side and the
//! Sun's limb darkening, the cloud shell's shadow and drift, the rings' shadows, the atmosphere's band from orbit,
//! and the spin, each held to the draft's reference module (`conformance/tools/planets_reference.py`).
//!
//! Lengths are in any one unit (the table's are kilometres); angles are degrees unless a function says radians. The
//! body frame has x toward the prime meridian and z toward the north pole; [`scene_from_body`] turns it into the
//! scene's axes (y down).

type V3 = [f64; 3];

/// The IAU 2009 equatorial and polar radii (km) of `pck00010.tpc`, by NAIF id: the Sun, the eight planets and the
/// Moon.
pub const RADII: [(i32, (f64, f64)); 10] = [
    (10, (696000.0, 696000.0)),
    (199, (2439.7, 2439.7)),
    (299, (6051.8, 6051.8)),
    (399, (6378.1366, 6356.7519)),
    (499, (3396.19, 3376.20)),
    (599, (71492.0, 66854.0)),
    (699, (60268.0, 54364.0)),
    (799, (25559.0, 24973.0)),
    (899, (24764.0, 24341.0)),
    (301, (1737.4, 1737.4)),
];

/// The IAU 2009 prime meridian's rate (degrees per day) of the bodies of [`RADII`].
pub const SPIN: [(i32, f64); 10] = [
    (10, 14.18440),
    (199, 6.1385025),
    (299, -1.4813688),
    (399, 360.9856235),
    (499, 350.89198226),
    (599, 870.5360000),
    (699, 810.7939024),
    (799, -501.1600928),
    (899, 536.3128492),
    (301, 13.17635815),
];

/// The wavelengths (nm) the red, green and blue channels stand for in rule 3's limb darkening.
pub const CHANNEL_WAVELENGTHS_NM: [f64; 3] = [610.0, 550.0, 465.0];

/// The wavelengths (nm) Hestroffer and Magnan (1998) fitted.
const HM98_RANGE_NM: (f64, f64) = (303.0, 1100.0);

const DAY_S: f64 = 86400.0;

/// A body, a value or a geometry the planets draft does not admit.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("planet: {0}")]
pub struct PlanetError(pub String);

fn err<T>(message: &str) -> Result<T, PlanetError> {
    Err(PlanetError(message.into()))
}

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn mul(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

pub fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}

/// `a` over its length; a zero or non-finite vector has no direction.
pub fn unit(a: V3) -> Result<V3, PlanetError> {
    let n = norm(a);
    if !(n > 0.0 && n.is_finite()) {
        return err("a zero vector has no direction");
    }
    Ok(mul(a, 1.0 / n))
}

/// The angle between two vectors, radians.
pub fn angle(a: V3, b: V3) -> f64 {
    (dot(a, b) / (norm(a) * norm(b))).clamp(-1.0, 1.0).acos()
}

/// Two unit vectors perpendicular to the unit `axis` and to each other.
fn basis(axis: V3) -> (V3, V3) {
    let helper = if axis[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
    let e1 = cross(helper, axis);
    let e1 = mul(e1, 1.0 / norm(e1));
    (e1, cross(axis, e1))
}

/// Rule 1: the equatorial and polar radii (km) of `body`.
pub fn radii(body: i32) -> Result<(f64, f64), PlanetError> {
    match RADII.iter().find(|(id, _)| *id == body) {
        Some((_, r)) => Ok(*r),
        None => Err(PlanetError(format!("no radii for body {body}: the draft covers 10, 199 to 899 and 301"))),
    }
}

/// Rule 1: (a − c) / a of the body's IAU spheroid.
pub fn flattening(body: i32) -> Result<f64, PlanetError> {
    let (a, c) = radii(body)?;
    Ok((a - c) / a)
}

/// Rule 1: the body's equatorial radius in scene units under `pixelsPerMeter` (SREP 4).
pub fn scene_radius(body: i32, pixels_per_meter: f64) -> Result<f64, PlanetError> {
    if !(pixels_per_meter > 0.0 && pixels_per_meter.is_finite()) {
        return err("pixelsPerMeter is positive");
    }
    Ok(radii(body)?.0 * 1000.0 * pixels_per_meter)
}

fn check_lon_lat(lon: f64, lat: f64) -> Result<(), PlanetError> {
    if !(lon.is_finite() && (-90.0..=90.0).contains(&lat)) {
        return err("longitude is finite and latitude within [-90, 90]");
    }
    Ok(())
}

/// Rule 2: the texel (u, v) of an equirectangular map at a longitude and latitude: longitude −180 at the left edge,
/// the prime meridian at the centre and the north pole at the top.
pub fn texture_uv(lon: f64, lat: f64) -> Result<[f64; 2], PlanetError> {
    check_lon_lat(lon, lat)?;
    Ok([(lon / 360.0 + 0.5).rem_euclid(1.0), (90.0 - lat) / 180.0])
}

/// Rule 2: the engine's UV-sphere coordinates (u, w) of a longitude and latitude ([`crate::prim`]'s sphere: u = 0
/// at +x, w = 0 at the north pole), where a day texture is read at u + 1/2.
pub fn engine_uv(lon: f64, lat: f64) -> Result<[f64; 2], PlanetError> {
    check_lon_lat(lon, lat)?;
    Ok([(lon / 360.0).rem_euclid(1.0), (90.0 - lat) / 180.0])
}

/// Rule 2: the engine's UV-sphere normal at (u, w): (sin πw cos 2πu, −cos πw, sin πw sin 2πu).
pub fn engine_normal(u: f64, w: f64) -> V3 {
    let (th, ph) = (u * 2.0 * std::f64::consts::PI, w * std::f64::consts::PI);
    [ph.sin() * th.cos(), -ph.cos(), ph.sin() * th.sin()]
}

/// The unit vector of a longitude and latitude in the body frame.
pub fn body_direction(lon: f64, lat: f64) -> Result<V3, PlanetError> {
    check_lon_lat(lon, lat)?;
    let (lo, la) = (lon.to_radians(), lat.to_radians());
    Ok([la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin()])
}

/// The scene's axes of a body-frame vector: the body's pole is the scene's −y and the body's y the scene's z.
pub fn scene_from_body(v: V3) -> V3 {
    [v[0], -v[2], v[1]]
}

/// Rule 1: the point of the spheroid of equatorial radius `a` and flattening `f` at a geodetic longitude and
/// latitude.
pub fn surface_point(a: f64, f: f64, lon: f64, lat: f64) -> Result<V3, PlanetError> {
    if !(a > 0.0 && a.is_finite()) || !(0.0..1.0).contains(&f) {
        return err("the equatorial radius is positive and the flattening within [0, 1)");
    }
    check_lon_lat(lon, lat)?;
    let (lo, la) = (lon.to_radians(), lat.to_radians());
    let e2 = 2.0 * f - f * f;
    let n = a / (1.0 - e2 * la.sin().powi(2)).sqrt();
    Ok([n * la.cos() * lo.cos(), n * la.cos() * lo.sin(), n * (1.0 - e2) * la.sin()])
}

/// Rule 1: the spheroid's outward unit normal at its surface point `p`, the gradient of its implicit equation.
pub fn surface_normal(a: f64, f: f64, p: V3) -> V3 {
    let c = a * (1.0 - f);
    let g = [p[0] / (a * a), p[1] / (a * a), p[2] / (c * c)];
    mul(g, 1.0 / norm(g))
}

/// Rule 7: a sphere's limb seen from an eye: its angular radius (degrees) and the circle in the plane eye · p =
/// radius², its centre and radius.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limb {
    pub angular_radius: f64,
    pub centre: V3,
    pub radius: f64,
}

/// Rule 7: the limb of a sphere of `radius` at the origin from `eye`.
pub fn limb(eye: V3, radius: f64) -> Result<Limb, PlanetError> {
    let d = norm(eye);
    if !(d > radius && radius > 0.0) {
        return err("the eye is outside the sphere");
    }
    Ok(Limb {
        angular_radius: (radius / d).asin().to_degrees(),
        centre: mul(eye, radius * radius / (d * d)),
        radius: radius * (1.0 - (radius / d).powi(2)).sqrt(),
    })
}

/// Rule 7: the lit cap's angular radius (degrees) under a point Sun at `sun_distance`: 90° less the body's angular
/// radius from the Sun.
pub fn terminator_half_angle(radius: f64, sun_distance: f64) -> Result<f64, PlanetError> {
    if !(sun_distance > radius && radius > 0.0) {
        return err("the Sun is farther than the body's radius");
    }
    Ok(90.0 - (radius / sun_distance).asin().to_degrees())
}

/// Rule 7: `n` points of the terminator: the great circle normal to `sun`, or the lit cap's small circle under a
/// point Sun at `sun_distance`.
pub fn terminator_points(sun: V3, radius: f64, n: usize, sun_distance: Option<f64>) -> Result<Vec<V3>, PlanetError> {
    let s = unit(sun)?;
    let half = match sun_distance {
        None => 90.0f64,
        Some(d) => terminator_half_angle(radius, d)?,
    };
    let (e1, e2) = basis(s);
    let (ch, sh) = (half.to_radians().cos(), half.to_radians().sin());
    Ok((0..n)
        .map(|k| {
            let t = 2.0 * std::f64::consts::PI * k as f64 / n as f64;
            let ring = add(mul(e1, t.cos()), mul(e2, t.sin()));
            mul(add(mul(s, ch), mul(ring, sh)), radius)
        })
        .collect())
}

/// Rule 7: whether a surface point (or its normal) faces the Sun.
pub fn lit(p: V3, sun: V3) -> bool {
    dot(p, sun) > 0.0
}

/// Rule 7: the Sun–body–eye angle, degrees.
pub fn phase_angle(eye: V3, sun: V3) -> f64 {
    angle(eye, sun).to_degrees()
}

/// Rule 7: the lit fraction of the disc at a phase angle (degrees), (1 + cos i) / 2.
pub fn illuminated_fraction(phase: f64) -> Result<f64, PlanetError> {
    if !(0.0..=180.0).contains(&phase) {
        return err("the phase angle is within [0, 180]");
    }
    Ok((1.0 + phase.to_radians().cos()) / 2.0)
}

/// Rule 7: the two points where the terminator meets the limb, none when the visible disc is wholly lit or wholly
/// dark (the Sun within the limb's angular radius of the line of sight or of its opposite).
pub fn cusps(eye: V3, sun: V3, radius: f64) -> Result<Option<(V3, V3)>, PlanetError> {
    let d = norm(eye);
    if !(d > radius && radius > 0.0) {
        return err("the eye is outside the sphere");
    }
    let (e, s) = (mul(eye, 1.0 / d), unit(sun)?);
    let c = dot(e, s);
    if 1.0 - c * c < 1e-12 {
        return Ok(None);
    }
    let a = radius * radius / (d * (1.0 - c * c));
    let p0 = add(mul(e, a), mul(s, -a * c));
    let t2 = radius * radius - dot(p0, p0);
    if t2 <= 0.0 {
        return Ok(None);
    }
    let line = unit(cross(e, s))?;
    let t = t2.sqrt();
    Ok(Some((add(p0, mul(line, t)), add(p0, mul(line, -t)))))
}

/// Rule 3: Lambert's cosine of the unit sun direction at a unit normal, clamped at 0.
pub fn day_weight(normal: V3, sun: V3) -> f64 {
    dot(normal, sun).max(0.0)
}

/// Rule 3: the night-lights weight: 1 with the Sun `twilight` degrees or more below the horizon, 0 as far above it,
/// 1/2 on the terminator and linear in the Sun's elevation between.
pub fn night_weight(normal: V3, sun: V3, twilight: f64) -> Result<f64, PlanetError> {
    if !(twilight > 0.0 && twilight.is_finite()) {
        return err("the twilight band is positive");
    }
    let e = dot(normal, sun).clamp(-1.0, 1.0).asin().to_degrees();
    Ok((0.5 - e / (2.0 * twilight)).clamp(0.0, 1.0))
}

/// Rule 3: Hestroffer and Magnan's (1998) eq. 5, the exponent α of I(μ) / I(1) = μ^α at a wavelength in nm, over
/// their 303 to 1100 nm; the gap between their branches (357 to 417 nm) is refused.
pub fn limb_alpha(nm: f64) -> Result<f64, PlanetError> {
    if !(HM98_RANGE_NM.0..=HM98_RANGE_NM.1).contains(&nm) {
        return err("the Hestroffer-Magnan fit covers 303 to 1100 nm");
    }
    let inv = 1000.0 / nm;
    if inv <= 2.4 {
        Ok(-0.023 + 0.292 * inv)
    } else if inv >= 2.8 {
        Ok(-0.507 + 0.441 * inv)
    } else {
        err("no fit between 357 and 417 nm")
    }
}

/// Rule 3: I(μ) / I(1) = μ^α.
pub fn limb_darkening(mu: f64, alpha: f64) -> Result<f64, PlanetError> {
    if !(0.0..=1.0).contains(&mu) {
        return err("mu is within [0, 1]");
    }
    Ok(mu.powf(alpha))
}

/// Rule 3: the disc-averaged intensity over the central one, 2 / (α + 2).
pub fn disc_average(alpha: f64) -> f64 {
    2.0 / (alpha + 2.0)
}

/// Rule 3: the limb-darkened Sun's linear colour at μ relative to its centre, per channel.
pub fn solar_disc(mu: f64) -> Result<V3, PlanetError> {
    let mut out = [0.0; 3];
    for (o, nm) in out.iter_mut().zip(CHANNEL_WAVELENGTHS_NM) {
        *o = limb_darkening(mu, limb_alpha(nm)?)?;
    }
    Ok(out)
}

/// Rule 4: the point of the cloud shell whose texel shadows the surface point `p` of the spheroid of equatorial
/// `radius` and `flattening`: where the ray from p toward the Sun leaves the shell, the body's spheroid scaled by
/// 1 + `height` (semi-axes a(1 + h) and c(1 + h)); none when the Sun is below p's horizon, the plane of its geodetic
/// normal.
pub fn cloud_shadow_point(
    p: V3,
    sun: V3,
    radius: f64,
    height: f64,
    flattening: f64,
) -> Result<Option<V3>, PlanetError> {
    if !(height > 0.0 && height.is_finite()) {
        return err("the cloud height is a positive fraction of the radius");
    }
    if !(radius > 0.0 && radius.is_finite()) || !(0.0..1.0).contains(&flattening) {
        return err("the equatorial radius is positive and the flattening within [0, 1)");
    }
    let s = unit(sun)?;
    if !lit(surface_normal(radius, flattening, p), s) {
        return Ok(None);
    }
    // stretched along the pole by 1 / (1 − f) the shell is the sphere of radius a(1 + h)
    let k = 1.0 / (1.0 - flattening);
    let (q, d) = ([p[0], p[1], p[2] * k], [s[0], s[1], s[2] * k]);
    let shell = radius * (1.0 + height);
    let (a, b) = (dot(d, d), dot(q, d));
    let t = (-b + (b * b - a * (dot(q, q) - shell * shell)).sqrt()) / a;
    Ok(Some(add(p, mul(s, t))))
}

/// Rule 4: the cloud layer's eastward drift (degrees, in [0, 360)) after `seconds` at `rate` degrees per second.
pub fn cloud_longitude(seconds: f64, rate: f64) -> f64 {
    (seconds * rate).rem_euclid(360.0)
}

/// Rule 4: the cloud texel over a longitude and latitude once the layer has drifted eastward.
pub fn clouds_uv(lon: f64, lat: f64, seconds: f64, rate: f64) -> Result<[f64; 2], PlanetError> {
    texture_uv(lon - cloud_longitude(seconds, rate), lat)
}

/// Rule 5: the RGBA of a radial strip at `rho`, linear between its samples spread evenly over [inner, outer]; none
/// outside.
pub fn ring_sample(strip: &[[f64; 4]], rho: f64, inner: f64, outer: f64) -> Result<Option<[f64; 4]>, PlanetError> {
    if !(outer > inner && inner > 0.0) || strip.len() < 2 {
        return err("the ring spans a positive range and the strip has two samples or more");
    }
    if !(inner..=outer).contains(&rho) {
        return Ok(None);
    }
    let x = (rho - inner) / (outer - inner) * (strip.len() - 1) as f64;
    let i = (x as usize).min(strip.len() - 2);
    let f = x - i as f64;
    Ok(Some(std::array::from_fn(|c| strip[i][c] * (1.0 - f) + strip[i + 1][c] * f)))
}

/// Rule 5: whether a point of the ring plane is in the planet's shadow: its ray toward the Sun meets the sphere.
pub fn ring_shadowed_by_planet(q: V3, sun: V3, radius: f64) -> bool {
    let Ok(s) = unit(sun) else { return false };
    let b = dot(q, s);
    let c = dot(q, q) - radius * radius;
    let disc = b * b - c;
    disc >= 0.0 && -b + disc.sqrt() > 0.0
}

/// Rule 5: the semi-axes of the ellipse the planet's shadow cuts in the ring plane (z = 0), along and across the
/// Sun's projected direction; none with the Sun in the plane.
pub fn planet_shadow_on_rings(sun: V3, radius: f64) -> Option<(f64, f64)> {
    let s = unit(sun).ok()?;
    (s[2] != 0.0).then(|| (radius / s[2].abs(), radius))
}

/// Rule 5: the radius at which the ray from a surface point toward the Sun crosses the ring plane ahead of it.
pub fn ring_crossing(p: V3, sun: V3) -> Option<f64> {
    let s = unit(sun).ok()?;
    if s[2] == 0.0 {
        return None;
    }
    let t = -p[2] / s[2];
    if t <= 0.0 {
        return None;
    }
    let q = add(p, mul(s, t));
    Some(q[0].hypot(q[1]))
}

/// Rule 5: the light reaching a surface point through the rings, 1 − alpha of the strip at the crossing radius.
pub fn ring_shadow(p: V3, sun: V3, strip: &[[f64; 4]], inner: f64, outer: f64) -> Result<f64, PlanetError> {
    let Some(rho) = ring_crossing(p, sun) else { return Ok(1.0) };
    Ok(match ring_sample(strip, rho, inner, outer)? {
        Some(s) => 1.0 - s[3].clamp(0.0, 1.0),
        None => 1.0,
    })
}

/// Rule 6: the angles from the nadir (degrees) of the planet's limb and of the atmosphere's top from a point at a
/// distance from the centre; none for an edge the point is inside of.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Band {
    pub planet: Option<f64>,
    pub shell: Option<f64>,
}

/// Rule 6: the atmosphere's band from a point at `distance` from the centre of a planet of `radius` under a shell
/// `height` thick.
pub fn shell_band(distance: f64, radius: f64, height: f64) -> Result<Band, PlanetError> {
    if !(height > 0.0 && radius > 0.0) {
        return err("the radius and the shell height are positive");
    }
    let top = radius + height;
    Ok(Band {
        planet: (distance > radius).then(|| (radius / distance).asin().to_degrees()),
        shell: (distance > top).then(|| (top / distance).asin().to_degrees()),
    })
}

/// Rule 7: the body's turn about its pole after `seconds` (degrees, signed and not wrapped) at the IAU prime
/// meridian's rate.
pub fn spin_degrees(body: i32, seconds: f64) -> Result<f64, PlanetError> {
    match SPIN.iter().find(|(id, _)| *id == body) {
        Some((_, rate)) => Ok(rate * seconds / DAY_S),
        None => Err(PlanetError(format!("no rotation rate for body {body}"))),
    }
}

/// Rule 7: a frame's scene seconds, cloud drift and spin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameState {
    pub seconds: f64,
    pub cloud_longitude: f64,
    pub spin: f64,
}

/// Rule 7: the state of frame `frame` at `fps` under a clock of `rate` scene seconds per second: a pure function of
/// the frame, so a frame rendered alone equals the frame reached by playing.
pub fn frame_state(frame: u64, fps: f64, rate: f64, cloud_rotation: f64, body: i32) -> Result<FrameState, PlanetError> {
    if !(fps > 0.0 && fps.is_finite()) || !(rate >= 0.0 && rate.is_finite()) {
        return err("fps is positive and the rate non-negative");
    }
    let seconds = frame as f64 / fps * rate;
    Ok(FrameState {
        seconds,
        cloud_longitude: cloud_longitude(seconds, cloud_rotation),
        spin: spin_degrees(body, seconds)?,
    })
}
