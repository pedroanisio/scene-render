//! The orbits draft (`srep/srep-0000-orbits.md`, SP4 of SREP 12), rules 1 and 2: Kepler's problem and the
//! `orbit` element. Positions are metres and velocities metres per second in the space frame; angles are
//! degrees unless a function says radians.
//!
//! * [`state_from_elements`] and [`elements_from_state`]: the classical elements and their inverse, with the
//!   angles taken by atan2 about ĥ so retrograde and equatorial orbits come back unchanged.
//! * [`kepler_elliptic`] and [`kepler_hyperbolic`]: Kepler's equation by Newton's iterations from Danby's start.
//! * [`propagate`]: the universal variable χ (Vallado, algorithm 8) with the Stumpff functions and the Lagrange
//!   f and g functions, one formula for ellipses and hyperbolas; an ellipse closes after a period to 1e-12.
//! * [`orbit`] and [`orbit_state`]: an object's state at any time of the space clock from its elements at an
//!   epoch, with no state carried between frames.

use super::SpaceError;

/// A vector of the space frame: metres or metres per second.
pub type Vec3 = [f64; 3];

/// The constant of gravitation, m³ kg⁻¹ s⁻² (CODATA 2018).
pub const G: f64 = 6.67430e-11;
/// The astronomical unit in metres (IAU 2012).
pub const AU_M: f64 = 149597870700.0;
/// The day in seconds.
pub const DAY_S: f64 = 86400.0;
/// The Sun's GM, m³/s² (DE440).
pub const GM_SUN: f64 = 1.327_124_400_412_794_2e20;
/// The Earth's GM, m³/s² (DE440).
pub const GM_EARTH: f64 = 3.98600435507e14;
/// The DE440 header GMs (Park et al. 2021), m³/s², of the ten bodies of the planets draft by NAIF id.
pub const GM: [(i32, f64); 10] = [
    (10, GM_SUN),
    (199, 2.2031868551e13),
    (299, 3.24858592e14),
    (399, GM_EARTH),
    (499, 4.2828375816e13),
    (599, 1.26712764100000e17),
    (699, 3.7940584841800e16),
    (799, 5.7945564e15),
    (899, 6.83652710058e15),
    (301, 4.902800118e12),
];
/// Kepler's equation's iteration bound.
pub const MAX_KEPLER_ITERATIONS: usize = 60;
/// The universal Kepler equation's iteration bound.
pub const MAX_UNIVERSAL_ITERATIONS: usize = 60;

fn err<T>(m: impl Into<String>) -> Result<T, SpaceError> {
    Err(SpaceError::Orbit(m.into()))
}

/// a + b.
pub fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// a − b.
pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// a s.
pub fn mul(a: Vec3, s: f64) -> Vec3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

/// a · b.
pub fn dot(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// a × b.
pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// |a|.
pub fn norm(a: Vec3) -> f64 {
    dot(a, a).sqrt()
}

/// 2π √(a³ / μ) of an ellipse, seconds.
pub fn period(mu: f64, a: f64) -> Result<f64, SpaceError> {
    if !(mu > 0.0 && a > 0.0) {
        return err("the period needs a positive mu and a positive semi-major axis");
    }
    Ok(2.0 * std::f64::consts::PI * (a.powi(3) / mu).sqrt())
}

/// v² / 2 − μ / r.
pub fn specific_energy(mu: f64, r: Vec3, v: Vec3) -> f64 {
    dot(v, v) / 2.0 - mu / norm(r)
}

/// E − e sin E = M, radians, with M reduced to (−π, π].
pub fn kepler_elliptic(mean_anomaly: f64, e: f64) -> Result<f64, SpaceError> {
    if !(0.0..1.0).contains(&e) {
        return err("the elliptic eccentricity is in [0, 1)");
    }
    if !mean_anomaly.is_finite() {
        return err("the mean anomaly is finite");
    }
    let tau = 2.0 * std::f64::consts::PI;
    let m = mean_anomaly - tau * (mean_anomaly / tau).round();
    let mut big_e = m + 0.85 * e * if m.sin() >= 0.0 { 1.0 } else { -1.0 };
    for _ in 0..MAX_KEPLER_ITERATIONS {
        let f = big_e - e * big_e.sin() - m;
        let de = f / (1.0 - e * big_e.cos());
        big_e -= de;
        if de.abs() <= 1e-15 * big_e.abs().max(1.0) {
            return Ok(big_e);
        }
    }
    err("Kepler's equation did not converge")
}

/// e sinh H − H = M, radians.
pub fn kepler_hyperbolic(mean_anomaly: f64, e: f64) -> Result<f64, SpaceError> {
    if e.is_nan() || e <= 1.0 {
        return err("the hyperbolic eccentricity is above 1");
    }
    if !mean_anomaly.is_finite() {
        return err("the mean anomaly is finite");
    }
    let mut h = (mean_anomaly / e).asinh();
    for _ in 0..MAX_KEPLER_ITERATIONS {
        let f = e * h.sinh() - h - mean_anomaly;
        let dh = f / (e * h.cosh() - 1.0);
        h -= dh;
        if dh.abs() <= 1e-15 * h.abs().max(1.0) {
            return Ok(h);
        }
    }
    err("the hyperbolic Kepler equation did not converge")
}

/// OR2: the eccentricity is non-negative and not within 1e-12 of 1; a is positive below 1 and negative above.
pub fn check_conic(a: f64, e: f64) -> Result<(), SpaceError> {
    if !e.is_finite() || e < 0.0 {
        return err("the eccentricity is non-negative and finite");
    }
    if (e - 1.0).abs() < 1e-12 {
        return err("the eccentricity is not 1 (the parabola is excluded)");
    }
    if e < 1.0 && !(a > 0.0 && a.is_finite()) {
        return err("a is positive for an ellipse");
    }
    if e > 1.0 && !(a < 0.0 && a.is_finite()) {
        return err("a is negative for a hyperbola");
    }
    Ok(())
}

fn perifocal_to_inertial(i: f64, raan: f64, argp: f64) -> [Vec3; 3] {
    let (si, ci) = i.to_radians().sin_cos();
    let (so, co) = raan.to_radians().sin_cos();
    let (sw, cw) = argp.to_radians().sin_cos();
    [
        [co * cw - so * sw * ci, -co * sw - so * cw * ci, so * si],
        [so * cw + co * sw * ci, -so * sw + co * cw * ci, -co * si],
        [sw * si, cw * si, ci],
    ]
}

fn apply(m: &[Vec3; 3], v: Vec3) -> Vec3 {
    [dot(m[0], v), dot(m[1], v), dot(m[2], v)]
}

/// Position (m) and velocity (m/s) from the classical elements: a in metres, the angles in degrees, ν the true
/// anomaly. The perifocal state rotated by R₃(Ω) R₁(i) R₃(ω).
pub fn state_from_elements(
    mu: f64,
    a: f64,
    e: f64,
    i: f64,
    raan: f64,
    argp: f64,
    nu: f64,
) -> Result<(Vec3, Vec3), SpaceError> {
    if !(mu > 0.0 && mu.is_finite()) {
        return err("mu is positive");
    }
    check_conic(a, e)?;
    if ![i, raan, argp, nu].iter().all(|x| x.is_finite()) {
        return err("the orbit's angles are finite");
    }
    let p = a * (1.0 - e * e);
    let (sn, cn) = nu.to_radians().sin_cos();
    let denom = 1.0 + e * cn;
    if denom.is_nan() || denom <= 0.0 {
        return err("the true anomaly is outside the hyperbola's asymptotes");
    }
    let r = p / denom;
    let k = (mu / p).sqrt();
    let m = perifocal_to_inertial(i, raan, argp);
    Ok((apply(&m, [r * cn, r * sn, 0.0]), apply(&m, [-k * sn, k * (e + cn), 0.0])))
}

/// The classical elements of a state (angles in degrees on [0, 360)).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Elements {
    /// The semi-major axis, metres (negative for a hyperbola).
    pub a: f64,
    /// The eccentricity.
    pub e: f64,
    /// The inclination.
    pub i: f64,
    /// The longitude of the ascending node (0 for an equatorial orbit, the node taken as x̂).
    pub raan: f64,
    /// The argument of pericentre (0 for a circular orbit).
    pub argp: f64,
    /// The true anomaly.
    pub nu: f64,
    /// The mean anomaly.
    pub mean: f64,
    /// The period, seconds; none for a hyperbola.
    pub period: Option<f64>,
    /// The specific energy, m²/s².
    pub energy: f64,
    /// The specific angular momentum, m²/s.
    pub h: Vec3,
}

fn deg360(x: f64) -> f64 {
    x.to_degrees().rem_euclid(360.0)
}

/// The elements of a state, from h = r × v, the node ẑ × h and the eccentricity vector.
pub fn elements_from_state(mu: f64, r: Vec3, v: Vec3) -> Result<Elements, SpaceError> {
    if !(mu > 0.0 && mu.is_finite()) {
        return err("mu is positive");
    }
    if !r.iter().chain(v.iter()).all(|x| x.is_finite()) {
        return err("the state is finite");
    }
    let rn = norm(r);
    let h = cross(r, v);
    let hn = norm(h);
    if rn == 0.0 || hn == 0.0 || hn <= 1e-12 * rn * norm(v) {
        return err("a radial or degenerate state has no orbit plane");
    }
    let energy = dot(v, v) / 2.0 - mu / rn;
    if energy.abs() < 1e-14 * mu / rn {
        return err("a parabolic state has no semi-major axis");
    }
    let a = -mu / (2.0 * energy);
    let e_vec = mul(sub(mul(r, dot(v, v) - mu / rn), mul(v, dot(r, v))), 1.0 / mu);
    let e = norm(e_vec);
    let h_hat = mul(h, 1.0 / hn);
    let inc = h_hat[2].clamp(-1.0, 1.0).acos();
    let node = cross([0.0, 0.0, 1.0], h);
    let (n_hat, raan) = if norm(node) <= 1e-12 * hn {
        ([1.0, 0.0, 0.0], 0.0)
    } else {
        (mul(node, 1.0 / norm(node)), node[1].atan2(node[0]))
    };
    let (e_hat, argp) = if e <= 1e-12 {
        (n_hat, 0.0)
    } else {
        let e_hat = mul(e_vec, 1.0 / e);
        (e_hat, dot(cross(n_hat, e_hat), h_hat).atan2(dot(n_hat, e_hat)))
    };
    let r_hat = mul(r, 1.0 / rn);
    let nu = dot(cross(e_hat, r_hat), h_hat).atan2(dot(e_hat, r_hat));
    let (mean, per) = if e < 1.0 {
        let big_e = 2.0 * ((1.0 - e).sqrt() * (nu / 2.0).sin()).atan2((1.0 + e).sqrt() * (nu / 2.0).cos());
        (big_e - e * big_e.sin(), Some(period(mu, a)?))
    } else {
        let big_h = 2.0 * (((e - 1.0) / (e + 1.0)).sqrt() * (nu / 2.0).tan()).atanh();
        (e * big_h.sinh() - big_h, None)
    };
    Ok(Elements {
        a,
        e,
        i: inc.to_degrees(),
        raan: deg360(raan),
        argp: deg360(argp),
        nu: deg360(nu),
        mean: deg360(mean),
        period: per,
        energy,
        h,
    })
}

/// (1 − cos √z) / z, continued through z = 0 by its series and to z < 0.
pub fn stumpff_c2(z: f64) -> f64 {
    if z.abs() < 1e-3 {
        return 0.5 - z / 24.0 + z * z / 720.0 - z.powi(3) / 40320.0 + z.powi(4) / 3628800.0;
    }
    if z > 0.0 {
        (1.0 - z.sqrt().cos()) / z
    } else {
        ((-z).sqrt().cosh() - 1.0) / -z
    }
}

/// (√z − sin √z) / √z³, continued through z = 0 by its series and to z < 0.
pub fn stumpff_c3(z: f64) -> f64 {
    if z.abs() < 1e-3 {
        return 1.0 / 6.0 - z / 120.0 + z * z / 5040.0 - z.powi(3) / 362880.0 + z.powi(4) / 39916800.0;
    }
    if z > 0.0 {
        let s = z.sqrt();
        (s - s.sin()) / s.powi(3)
    } else {
        let s = (-z).sqrt();
        (s.sinh() - s) / s.powi(3)
    }
}

/// The state `dt` seconds later (or earlier) by the universal variable χ, Newton's iterations stopping at
/// rounding, and the Lagrange f and g functions.
pub fn propagate(mu: f64, r0: Vec3, v0: Vec3, dt: f64) -> Result<(Vec3, Vec3), SpaceError> {
    if !(mu > 0.0 && mu.is_finite()) {
        return err("mu is positive");
    }
    if !dt.is_finite() || !r0.iter().chain(v0.iter()).all(|x| x.is_finite()) {
        return err("the state and the interval are finite");
    }
    let r0n = norm(r0);
    if r0n == 0.0 {
        return err("the state is at the centre");
    }
    if dt == 0.0 {
        return Ok((r0, v0));
    }
    let sqmu = mu.sqrt();
    let rv = dot(r0, v0);
    let alpha = 2.0 / r0n - dot(v0, v0) / mu; // 1 / a
    let mut chi = if alpha > 1e-12 / r0n {
        sqmu * alpha * dt
    } else if alpha < -1e-12 / r0n {
        let a = 1.0 / alpha;
        let sign = if dt > 0.0 { 1.0 } else { -1.0 };
        let num = -2.0 * mu * alpha * dt;
        let den = rv + sign * (-mu * a).sqrt() * (1.0 - r0n * alpha);
        sign * (-a).sqrt() * (num / den).ln()
    } else {
        let h = norm(cross(r0, v0));
        let p = h * h / mu;
        let s = 0.5 * (1.0 / (3.0 * (mu / p.powi(3)).sqrt() * dt)).atan();
        let w = s.tan().cbrt().atan();
        p.sqrt() * 2.0 / (2.0 * w).tan()
    };
    let mut last = f64::INFINITY;
    let mut converged = false;
    for _ in 0..MAX_UNIVERSAL_ITERATIONS {
        let z = alpha * chi * chi;
        let (c2, c3) = (stumpff_c2(z), stumpff_c3(z));
        let r = chi * chi * c2 + rv / sqmu * chi * (1.0 - z * c3) + r0n * (1.0 - z * c2);
        let f = rv / sqmu * chi * chi * c2 + (1.0 - alpha * r0n) * chi.powi(3) * c3 + r0n * chi - sqmu * dt;
        let d = f / r;
        chi -= d;
        let step = d.abs() / chi.abs().max(1.0);
        // At rounding, or the steps have stopped shrinking.
        if step <= 4e-16 || (step <= 1e-12 && step >= last) {
            converged = true;
            break;
        }
        last = step;
    }
    if !converged || !chi.is_finite() {
        return err("the universal Kepler equation did not converge");
    }
    let z = alpha * chi * chi;
    let (c2, c3) = (stumpff_c2(z), stumpff_c3(z));
    let f = 1.0 - chi * chi * c2 / r0n;
    let g = dt - chi.powi(3) * c3 / sqmu;
    let r = add(mul(r0, f), mul(v0, g));
    let rn = norm(r);
    let gdot = 1.0 - chi * chi * c2 / rn;
    let fdot = sqmu / (r0n * rn) * chi * (z * c3 - 1.0);
    Ok((r, add(mul(r0, fdot), mul(v0, gdot))))
}

/// The largest relative deviation of position and velocity after one period of an ellipse.
pub fn closure(mu: f64, r: Vec3, v: Vec3) -> Result<f64, SpaceError> {
    let Some(p) = elements_from_state(mu, r, v)?.period else {
        return err("only an ellipse closes");
    };
    let (r1, v1) = propagate(mu, r, v, p)?;
    Ok((norm(sub(r, r1)) / norm(r)).max(norm(sub(v, v1)) / norm(v)))
}

/// The GM of a NAIF body in [`GM`].
pub fn body_gm(body: i32) -> Option<f64> {
    GM.iter().find(|(id, _)| *id == body).map(|(_, gm)| *gm)
}

/// OR1: the central object's gravitational parameter, G × its mass, else the GM of its body.
pub fn orbit_mu(mass: Option<f64>, body: Option<i32>) -> Result<f64, SpaceError> {
    if let Some(m) = mass {
        if !(m > 0.0 && m.is_finite()) {
            return err("the central mass is positive");
        }
        return Ok(G * m);
    }
    match body {
        Some(b) => match body_gm(b) {
            Some(gm) => Ok(gm),
            None => {
                let ids: Vec<String> = GM.iter().map(|(id, _)| id.to_string()).collect();
                err(format!("no GM for body {b}; the draft covers {}", ids.join(", ")))
            }
        },
        None => err("the orbit's central object needs a mass or a body"),
    }
}

/// The `orbit` element: the state at its epoch, seconds of the space clock past the space epoch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Orbit {
    /// The central gravitational parameter, m³/s².
    pub mu: f64,
    /// The position at the epoch, metres relative to the central object.
    pub r0: Vec3,
    /// The velocity at the epoch, m/s.
    pub v0: Vec3,
    /// The epoch, seconds of the space clock.
    pub epoch: f64,
}

/// Rule 2: the orbit from its elements (angles in degrees, `anomaly` the true anomaly at `epoch`).
#[allow(clippy::too_many_arguments)]
pub fn orbit(
    mu: f64,
    a: f64,
    e: f64,
    i: f64,
    raan: f64,
    argp: f64,
    anomaly: f64,
    epoch: f64,
) -> Result<Orbit, SpaceError> {
    if !epoch.is_finite() {
        return err("the orbit's epoch is finite");
    }
    let (r0, v0) = state_from_elements(mu, a, e, i, raan, argp, anomaly)?;
    Ok(Orbit { mu, r0, v0, epoch })
}

/// The position and velocity at time `t` of the space clock, forward or back from the epoch.
pub fn orbit_state(orbit: &Orbit, t: f64) -> Result<(Vec3, Vec3), SpaceError> {
    propagate(orbit.mu, orbit.r0, orbit.v0, t - orbit.epoch)
}

/// Rule 5: the `count` sample times of a trail ending at `t`, oldest first, evenly spaced over the last `length`
/// seconds of the space clock and clamped at its epoch (time 0).
pub fn trail_times(t: f64, length: f64, count: usize) -> Result<Vec<f64>, SpaceError> {
    if count < 2 || length.is_nan() || length <= 0.0 {
        return err("a trail has two samples or more over a positive length");
    }
    let last = (count - 1) as f64;
    Ok((0..count).map(|k| (t - length * (last - k as f64) / last).max(0.0)).collect())
}

/// Rule 5: the opacity of a trail sample `age` seconds old, 1 − age / length when fading (within 0 and 1), else 1.
pub fn trail_alpha(age: f64, length: f64, fade: bool) -> f64 {
    if !fade {
        return 1.0;
    }
    (1.0 - age / length).clamp(0.0, 1.0)
}

/// Rule 5: an orbit's trail, its positions relative to its central object at the trail times ending at `t`.
pub fn orbit_trail(orbit: &Orbit, t: f64, length: f64, count: usize) -> Result<Vec<Vec3>, SpaceError> {
    trail_times(t, length, count)?.into_iter().map(|tk| orbit_state(orbit, tk).map(|(r, _)| r)).collect()
}
