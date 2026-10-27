//! The orbits draft (sr-core `srep/srep-0000-orbits.md`, SP4 of SREP 12), rules 3, 4 and 6: an N-body system
//! under mutual gravitation, its three integrators, the Plummer cluster and the state at a frame.
//!
//! Unlike the rest of the crate this module works in SI units in the space frame (metres, seconds, kilograms);
//! the scene maps its positions to pixels through the time-frames axes.
//!
//! * [`step_leapfrog`]: kick-drift-kick, second order, symplectic and time-symmetric.
//! * [`step_wisdom_holman`]: Wisdom and Holman (1991) in Jacobi coordinates, the exact Kepler drift of
//!   [`sr_space::orbits::propagate`] between two half kicks of the interaction Hamiltonian.
//! * [`step_ias15`] and [`integrate_adaptive`]: Everhart's Gauss–Radau scheme as Rein and Spiegel (2015) describe
//!   it, with the Radau nodes found by bisection ([`radau`]), not copied.
//! * [`plummer`]: Aarseth, Hénon and Wielen's sampling drawn from the F1 random streams, one body per index.
//! * [`frame_state`]: the system at a frame, integrated from the space epoch by a step that divides the time, so a
//!   frame rendered alone equals the frame reached by playing.

use sr_space::orbits::{add, cross, dot, mul, norm, propagate, sub, Vec3};
use sr_space::SpaceError;
use sr_vector::d24::d24_unit;
use std::sync::OnceLock;

/// The most bodies a system or a cluster holds.
pub const MAX_BODIES: usize = 4096;
/// The deterministic bound on the steps of one integration, fixed or adaptive.
pub const MAX_STEPS: usize = 1_000_000;
/// The predictor–corrector passes of one IAS15 step.
pub const MAX_RADAU_ITERATIONS: usize = 12;
/// The rejection draws of one Plummer speed.
pub const MAX_TRIES: usize = 64;
/// A time is a whole number of steps within this fraction of a step (OR3).
pub const STEP_TOLERANCE: f64 = 1e-9;

const ZERO: Vec3 = [0.0; 3];

fn err<T>(m: impl Into<String>) -> Result<T, SpaceError> {
    Err(SpaceError::Orbit(m.into()))
}

/// An N-body state: masses (kg), positions (m) and velocities (m/s), the constant of gravitation, the Plummer
/// softening (m) and the time (s past the space epoch).
#[derive(Debug, Clone, PartialEq)]
pub struct System {
    /// The masses.
    pub masses: Vec<f64>,
    /// The positions.
    pub positions: Vec<Vec3>,
    /// The velocities.
    pub velocities: Vec<Vec3>,
    /// The constant of gravitation, m³ kg⁻¹ s⁻².
    pub g: f64,
    /// The Plummer softening ε, metres.
    pub softening: f64,
    /// The time, seconds.
    pub t: f64,
}

impl System {
    fn with(&self, positions: Vec<Vec3>, velocities: Vec<Vec3>, t: f64) -> System {
        System { masses: self.masses.clone(), positions, velocities, g: self.g, softening: self.softening, t }
    }
}

/// A validated system at time 0: 1 to [`MAX_BODIES`] bodies with positive finite masses and finite states, a
/// positive G, a non-negative softening, and no two coincident bodies without softening.
pub fn system(
    masses: &[f64],
    positions: &[Vec3],
    velocities: &[Vec3],
    g: f64,
    softening: f64,
) -> Result<System, SpaceError> {
    let n = masses.len();
    if n == 0 || n > MAX_BODIES || positions.len() != n || velocities.len() != n {
        return err(format!("1 to {MAX_BODIES} bodies, each with a mass, a position and a velocity"));
    }
    if !masses.iter().all(|m| *m > 0.0 && m.is_finite()) {
        return err("masses are positive and finite");
    }
    if !(g > 0.0 && g.is_finite() && softening >= 0.0 && softening.is_finite()) {
        return err("G is positive and the softening non-negative");
    }
    if !positions.iter().chain(velocities.iter()).flatten().all(|x| x.is_finite()) {
        return err("positions and velocities are finite");
    }
    if softening == 0.0 {
        for i in 0..n {
            for j in i + 1..n {
                if positions[i] == positions[j] {
                    return err(format!("bodies {i} and {j} coincide and the softening is zero"));
                }
            }
        }
    }
    Ok(System {
        masses: masses.to_vec(),
        positions: positions.to_vec(),
        velocities: velocities.to_vec(),
        g,
        softening,
        t: 0.0,
    })
}

/// a_i = Σ_j G m_j (r_j − r_i) / (|r_j − r_i|² + ε²)^(3/2) at the given positions.
pub fn accelerations(s: &System, positions: &[Vec3]) -> Vec<Vec3> {
    let (m, eps2) = (&s.masses, s.softening * s.softening);
    let mut acc = vec![ZERO; m.len()];
    for i in 0..m.len() {
        for j in i + 1..m.len() {
            let d = sub(positions[j], positions[i]);
            let d2 = dot(d, d) + eps2;
            if d2 == 0.0 {
                continue;
            }
            let inv = 1.0 / (d2 * d2.sqrt());
            acc[i] = add(acc[i], mul(d, s.g * m[j] * inv));
            acc[j] = sub(acc[j], mul(d, s.g * m[i] * inv));
        }
    }
    acc
}

/// Σ m v² / 2.
pub fn kinetic(s: &System) -> f64 {
    s.masses.iter().zip(&s.velocities).map(|(m, v)| 0.5 * m * dot(*v, *v)).sum()
}

/// −Σ_pairs G m_i m_j / √(|r_j − r_i|² + ε²).
pub fn potential(s: &System) -> f64 {
    let (m, p, eps2) = (&s.masses, &s.positions, s.softening * s.softening);
    let mut total = 0.0;
    for i in 0..m.len() {
        for j in i + 1..m.len() {
            let d = sub(p[j], p[i]);
            total -= s.g * m[i] * m[j] / (dot(d, d) + eps2).sqrt();
        }
    }
    total
}

/// The kinetic plus the potential energy.
pub fn energy(s: &System) -> f64 {
    kinetic(s) + potential(s)
}

/// Σ m v.
pub fn momentum(s: &System) -> Vec3 {
    s.masses.iter().zip(&s.velocities).fold(ZERO, |t, (m, v)| add(t, mul(*v, *m)))
}

/// Σ m r / Σ m.
pub fn centre_of_mass(s: &System) -> Vec3 {
    let total = s.masses.iter().zip(&s.positions).fold(ZERO, |t, (m, p)| add(t, mul(*p, *m)));
    mul(total, 1.0 / s.masses.iter().sum::<f64>())
}

/// Σ m r × v.
pub fn angular_momentum(s: &System) -> Vec3 {
    s.masses.iter().zip(&s.positions).zip(&s.velocities).fold(ZERO, |t, ((m, p), v)| add(t, mul(cross(*p, *v), *m)))
}

/// The same system in its barycentric frame: the centre of mass at the origin and at rest.
pub fn at_rest(s: &System) -> System {
    let com = centre_of_mass(s);
    let vcom = mul(momentum(s), 1.0 / s.masses.iter().sum::<f64>());
    s.with(
        s.positions.iter().map(|p| sub(*p, com)).collect(),
        s.velocities.iter().map(|v| sub(*v, vcom)).collect(),
        s.t,
    )
}

/// The integrator of an `nbody` element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Integrator {
    /// Kick-drift-kick.
    Leapfrog,
    /// Wisdom–Holman in Jacobi coordinates.
    WisdomHolman,
    /// Everhart's Gauss–Radau scheme of 15th order.
    Ias15,
}

impl std::str::FromStr for Integrator {
    type Err = SpaceError;
    fn from_str(s: &str) -> Result<Self, SpaceError> {
        match s {
            "leapfrog" => Ok(Integrator::Leapfrog),
            "wisdomHolman" => Ok(Integrator::WisdomHolman),
            "ias15" => Ok(Integrator::Ias15),
            _ => err("the integrator is one of leapfrog, wisdomHolman, ias15"),
        }
    }
}

/// One kick-drift-kick step.
pub fn step_leapfrog(s: &System, dt: f64) -> System {
    let a = accelerations(s, &s.positions);
    let v_half: Vec<Vec3> = s.velocities.iter().zip(&a).map(|(v, ai)| add(*v, mul(*ai, dt / 2.0))).collect();
    let x: Vec<Vec3> = s.positions.iter().zip(&v_half).map(|(p, v)| add(*p, mul(*v, dt))).collect();
    let a = accelerations(s, &x);
    let v = v_half.iter().zip(&a).map(|(vh, ai)| add(*vh, mul(*ai, dt / 2.0))).collect();
    s.with(x, v, s.t + dt)
}

/// Jacobi coordinates r_i = x_i − X_{i−1} (r_0 unused), the last centre of mass X and the cumulative masses M_i.
fn jacobi(masses: &[f64], vectors: &[Vec3]) -> (Vec<Vec3>, Vec3, Vec<f64>) {
    let (mut big_x, mut big_m) = (vectors[0], masses[0]);
    let mut r = vec![ZERO; masses.len()];
    let mut cum = vec![big_m];
    for i in 1..masses.len() {
        r[i] = sub(vectors[i], big_x);
        big_x = mul(add(mul(big_x, big_m), mul(vectors[i], masses[i])), 1.0 / (big_m + masses[i]));
        big_m += masses[i];
        cum.push(big_m);
    }
    (r, big_x, cum)
}

fn from_jacobi(masses: &[f64], r: &[Vec3], big_x: Vec3, cum: &[f64]) -> Vec<Vec3> {
    let mut out = vec![ZERO; masses.len()];
    let mut x = big_x;
    for i in (1..masses.len()).rev() {
        x = sub(x, mul(r[i], masses[i] / cum[i]));
        out[i] = add(x, r[i]);
    }
    out[0] = x;
    out
}

/// The Cartesian accelerations of H_int = Σ_i G m_i M_{i−1} / |r_i| − Σ_pairs G m_i m_j / |x_i − x_j|.
fn interaction_accelerations(s: &System, positions: &[Vec3], r: &[Vec3], cum: &[f64]) -> Vec<Vec3> {
    let n = s.masses.len();
    let a = accelerations(s, positions);
    let kep: Vec<Vec3> = (0..n).map(|i| if i == 0 { ZERO } else { mul(r[i], s.g / norm(r[i]).powi(3)) }).collect();
    (0..n)
        .map(|j| {
            let mut aj = a[j];
            if j >= 1 {
                aj = add(aj, mul(kep[j], cum[j - 1]));
            }
            for (i, k) in kep.iter().enumerate().skip(j + 1) {
                aj = sub(aj, mul(*k, s.masses[i]));
            }
            aj
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn kick(s: &System, r: &[Vec3], rdot: &mut [Vec3], big_x: Vec3, cum: &[f64], dt: f64) {
    let positions = from_jacobi(&s.masses, r, big_x, cum);
    let a = interaction_accelerations(s, &positions, r, cum);
    let (a_j, _, _) = jacobi(&s.masses, &a);
    for i in 1..s.masses.len() {
        rdot[i] = add(rdot[i], mul(a_j[i], dt));
    }
}

/// One Wisdom–Holman step: the interaction kick for dt / 2, the Kepler drift of every Jacobi coordinate about the
/// mass inside it for dt, the kick for dt / 2. Body 0 is the central mass; softening is not admitted (OR3).
pub fn step_wisdom_holman(s: &System, dt: f64) -> Result<System, SpaceError> {
    if s.softening != 0.0 {
        return err("the Wisdom-Holman splitting has no softening");
    }
    let n = s.masses.len();
    if n == 1 {
        return Ok(s.with(vec![add(s.positions[0], mul(s.velocities[0], dt))], s.velocities.clone(), s.t + dt));
    }
    let (mut r, mut big_x, cum) = jacobi(&s.masses, &s.positions);
    let (mut rdot, big_v, _) = jacobi(&s.masses, &s.velocities);
    kick(s, &r, &mut rdot, big_x, &cum, dt / 2.0);
    for i in 1..n {
        (r[i], rdot[i]) = propagate(s.g * cum[i], r[i], rdot[i], dt)?;
    }
    big_x = add(big_x, mul(big_v, dt));
    kick(s, &r, &mut rdot, big_x, &cum, dt / 2.0);
    Ok(s.with(from_jacobi(&s.masses, &r, big_x, &cum), from_jacobi(&s.masses, &rdot, big_v, &cum), s.t + dt))
}

/// The Gauss–Radau tables of IAS15.
#[derive(Debug, Clone, PartialEq)]
pub struct Radau {
    /// The nodes on [0, 1]: 0 and the seven roots of P₇ + P₈ on (−1, 1) mapped by (x + 1) / 2.
    pub h: [f64; 8],
    /// The quadrature weights of the nodes, exact to degree 14.
    pub w: [f64; 8],
    /// c[j][k]: the coefficient of τ^k in ∏_{i=1..j} (τ − h_i), j = 0..6.
    pub c: [[f64; 7]; 7],
    /// r[i][j] = 1 / (h_i − h_j) for j < i: the divided differences' denominators.
    pub r: [[f64; 8]; 8],
}

fn legendre(n: usize, x: f64) -> f64 {
    let (mut p0, mut p1) = (1.0, x);
    if n == 0 {
        return p0;
    }
    for k in 2..=n {
        let kf = k as f64;
        (p0, p1) = (p1, ((2.0 * kf - 1.0) * x * p1 - (kf - 1.0) * p0) / kf);
    }
    p1
}

fn radau_nodes() -> [f64; 8] {
    let f = |x: f64| legendre(7, x) + legendre(8, x);
    let grid = 4000;
    let xs: Vec<f64> = (0..=grid).map(|k| -1.0 + 1e-9 + (2.0 - 2e-9) * k as f64 / grid as f64).collect();
    let mut roots = Vec::new();
    for w in xs.windows(2) {
        let (mut a, mut b) = (w[0], w[1]);
        let mut fa = f(a);
        if fa == 0.0 {
            roots.push(a);
            continue;
        }
        if fa * f(b) < 0.0 {
            for _ in 0..200 {
                let c = (a + b) / 2.0;
                let fc = f(c);
                if fc == 0.0 || b - a < 1e-17 {
                    break;
                }
                if fa * fc < 0.0 {
                    b = c;
                } else {
                    (a, fa) = (c, fc);
                }
            }
            roots.push((a + b) / 2.0);
        }
    }
    // P₇ + P₈ has exactly seven roots on (−1, 1); the grid separates them.
    assert_eq!(roots.len(), 7, "the Radau polynomial's roots");
    let mut h = [0.0; 8];
    for (k, x) in roots.iter().enumerate() {
        h[k + 1] = (x + 1.0) / 2.0;
    }
    h
}

fn radau_tables() -> Radau {
    let h = radau_nodes();
    // Radau's weights on [−1, 1], halved for [0, 1]: 2 / n² at the fixed node and (1 − x) / (n² P₇(x)²) at the
    // others; a Vandermonde solve for them is ill-conditioned and loses three digits.
    let w = std::array::from_fn(|i| {
        if i == 0 {
            1.0 / 64.0
        } else {
            let x = 2.0 * h[i] - 1.0;
            (1.0 - x) / (64.0 * legendre(7, x).powi(2)) / 2.0
        }
    });
    let mut c = [[0.0; 7]; 7];
    let mut poly = vec![1.0];
    for (j, row) in c.iter_mut().enumerate() {
        if j > 0 {
            let mut shifted = vec![0.0];
            shifted.extend_from_slice(&poly);
            poly = (0..shifted.len())
                .map(|k| shifted[k] - if k + 1 < shifted.len() { h[j] * shifted[k + 1] } else { 0.0 })
                .collect();
        }
        row[..poly.len()].copy_from_slice(&poly);
    }
    let r = std::array::from_fn(|i| std::array::from_fn(|j| if j < i { 1.0 / (h[i] - h[j]) } else { 0.0 }));
    Radau { h, w, c, r }
}

/// The Gauss–Radau tables, computed once.
pub fn radau() -> &'static Radau {
    static TABLES: OnceLock<Radau> = OnceLock::new();
    TABLES.get_or_init(radau_tables)
}

/// The seven b coefficients of the acceleration polynomial, each over the 3n components.
pub type Coefficients = [Vec<f64>; 7];

fn flat(v: &[Vec3]) -> Vec<f64> {
    v.iter().flatten().copied().collect()
}

fn unflat(f: &[f64]) -> Vec<Vec3> {
    f.as_chunks::<3>().0.to_vec()
}

/// One Gauss–Radau step from a guess of the b: the positions, velocities, converged b and the error estimate
/// max|b₆| / max|a₀|.
fn radau_step(s: &System, dt: f64, b_start: Option<&Coefficients>) -> (Vec<Vec3>, Vec<Vec3>, Coefficients, f64) {
    let rd = radau();
    let n3 = 3 * s.masses.len();
    let (x0, v0) = (flat(&s.positions), flat(&s.velocities));
    let a0 = flat(&accelerations(s, &s.positions));
    let mut b: Coefficients = match b_start {
        Some(b) => b.clone(),
        None => std::array::from_fn(|_| vec![0.0; n3]),
    };
    let mut g: Coefficients = std::array::from_fn(|_| vec![0.0; n3]);
    let a_scale = a0.iter().fold(0.0f64, |m, a| m.max(a.abs()));
    let a_scale = if a_scale == 0.0 { 1.0 } else { a_scale };
    let mut xn = vec![0.0; n3];
    for _ in 0..MAX_RADAU_ITERATIONS {
        let mut change = 0.0f64;
        for nn in 1..8 {
            let tau = rd.h[nn];
            for c in 0..n3 {
                let poly: f64 = (0..7).map(|k| b[k][c] * tau.powi(k as i32 + 3) / ((k + 2) * (k + 3)) as f64).sum();
                xn[c] = x0[c] + v0[c] * dt * tau + dt * dt * (a0[c] * tau * tau / 2.0 + poly);
            }
            let an = flat(&accelerations(s, &unflat(&xn)));
            for c in 0..n3 {
                let mut temp = (an[c] - a0[c]) * rd.r[nn][0];
                for j in 1..nn {
                    temp = (temp - g[j - 1][c]) * rd.r[nn][j];
                }
                g[nn - 1][c] = temp;
            }
            for c in 0..n3 {
                for (k, bk) in b.iter_mut().enumerate() {
                    let new: f64 = (k..7).map(|j| rd.c[j][k] * g[j][c]).sum();
                    change = change.max((new - bk[c]).abs());
                    bk[c] = new;
                }
            }
        }
        if change <= 1e-16 * a_scale {
            break;
        }
    }
    let x1: Vec<f64> = (0..n3)
        .map(|c| {
            let poly: f64 = (0..7).map(|k| b[k][c] / ((k + 2) * (k + 3)) as f64).sum();
            x0[c] + v0[c] * dt + dt * dt * (a0[c] / 2.0 + poly)
        })
        .collect();
    let v1: Vec<f64> =
        (0..n3).map(|c| v0[c] + dt * (a0[c] + (0..7).map(|k| b[k][c] / (k + 2) as f64).sum::<f64>())).collect();
    let err = b[6].iter().fold(0.0f64, |m, x| m.max(x.abs())) / a_scale;
    (unflat(&x1), unflat(&v1), b, err)
}

/// One IAS15 step of a fixed size from the previous step's coefficients (the predictor): the new system and the
/// converged coefficients.
pub fn step_ias15(s: &System, dt: f64, b: Option<&Coefficients>) -> (System, Coefficients) {
    let (x, v, b, _) = radau_step(s, dt, b);
    (s.with(x, v, s.t + dt), b)
}

/// The whole number of steps in a time, or an error when the time is not one within [`STEP_TOLERANCE`] (OR3).
pub fn steps_to(t: f64, step: f64) -> Result<usize, SpaceError> {
    if !(step > 0.0 && step.is_finite() && t >= 0.0 && t.is_finite()) {
        return err("the step is positive and the time non-negative");
    }
    let n = (t / step).round();
    if (t / step - n).abs() > STEP_TOLERANCE {
        return err("the time is not a whole number of steps");
    }
    Ok(n as usize)
}

fn step(s: &System, integrator: Integrator, dt: f64, b: &mut Option<Coefficients>) -> Result<System, SpaceError> {
    Ok(match integrator {
        Integrator::Leapfrog => step_leapfrog(s, dt),
        Integrator::WisdomHolman => step_wisdom_holman(s, dt)?,
        Integrator::Ias15 => {
            let (out, nb) = step_ias15(s, dt, b.as_ref());
            *b = Some(nb);
            out
        }
    })
}

/// The system `t` seconds later by a fixed step that divides t, at most [`MAX_STEPS`] steps.
pub fn integrate(s: &System, integrator: Integrator, dt: f64, t: f64) -> Result<System, SpaceError> {
    let n = steps_to(t, dt)?;
    if n > MAX_STEPS {
        return err(format!("more than {MAX_STEPS} steps"));
    }
    if integrator == Integrator::WisdomHolman && s.softening != 0.0 {
        return err("the Wisdom-Holman splitting has no softening");
    }
    let (mut out, mut b) = (s.clone(), None);
    for _ in 0..n {
        out = step(&out, integrator, dt, &mut b)?;
    }
    Ok(out)
}

/// The steps an adaptive integration took: the count (rejected ones included) and the smallest and largest
/// accepted step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AdaptiveStats {
    /// Steps attempted, accepted or rejected.
    pub count: usize,
    /// The smallest accepted step, seconds.
    pub min: f64,
    /// The largest accepted step, seconds.
    pub max: f64,
}

/// IAS15 with the adaptive step of Rein and Spiegel: a step whose error estimate max|b₆| / max|a| exceeds ε is
/// rejected and retried at 0.99 dt (ε / error)^(1/7); an accepted step grows by at most 1.4; the last step lands on
/// t; more than `max_steps` attempts is an error.
pub fn integrate_adaptive(
    s: &System,
    t: f64,
    dt0: f64,
    epsilon: f64,
    max_steps: usize,
) -> Result<(System, AdaptiveStats), SpaceError> {
    if !(epsilon > 0.0 && dt0 > 0.0 && dt0.is_finite() && t >= 0.0 && t.is_finite()) {
        return err("the tolerance, the first step and the time are positive");
    }
    let end = s.t + t;
    let mut out = s.clone();
    let mut dt = if t > 0.0 { dt0.min(t) } else { dt0 };
    let mut b: Option<Coefficients> = None;
    let mut stats = AdaptiveStats { count: 0, min: f64::INFINITY, max: 0.0 };
    while out.t < end - 1e-15 * end.abs().max(1.0) {
        if stats.count >= max_steps {
            return err(format!("more than {max_steps} steps"));
        }
        dt = dt.min(end - out.t);
        let (x, v, nb, error) = radau_step(&out, dt, b.as_ref());
        let ratio = if error > 0.0 { (epsilon / error).powf(1.0 / 7.0) } else { 1.4 };
        stats.count += 1;
        if ratio < 1.0 {
            dt *= 0.99 * ratio;
            continue;
        }
        out = out.with(x, v, out.t + dt);
        b = Some(nb);
        stats.min = stats.min.min(dt);
        stats.max = stats.max.max(dt);
        dt *= ratio.min(1.4);
    }
    out.t = end;
    Ok((out, stats))
}

/// The largest |E − E₀| / |E₀| over the steps.
pub fn energy_drift(s: &System, integrator: Integrator, dt: f64, t: f64) -> Result<f64, SpaceError> {
    let e0 = energy(s);
    let n = steps_to(t, dt)?;
    if n > MAX_STEPS {
        return err(format!("more than {MAX_STEPS} steps"));
    }
    let (mut out, mut b, mut worst) = (s.clone(), None, 0.0f64);
    for _ in 0..n {
        out = step(&out, integrator, dt, &mut b)?;
        worst = worst.max(((energy(&out) - e0) / e0).abs());
    }
    Ok(worst)
}

/// Forward for t, velocities reversed, forward for t again: the largest position error over the largest initial
/// distance from the origin.
pub fn reversibility(s: &System, integrator: Integrator, dt: f64, t: f64) -> Result<f64, SpaceError> {
    let forward = integrate(s, integrator, dt, t)?;
    let flipped =
        forward.with(forward.positions.clone(), forward.velocities.iter().map(|v| mul(*v, -1.0)).collect(), forward.t);
    let back = integrate(&flipped, integrator, dt, t)?;
    let scale = s.positions.iter().fold(0.0f64, |m, p| m.max(norm(*p)));
    let scale = if scale == 0.0 { 1.0 } else { scale };
    let worst = back.positions.iter().zip(&s.positions).fold(0.0f64, |m, (a, b)| m.max(norm(sub(*a, *b))));
    Ok(worst / scale)
}

/// For two bodies: the distance between the integrated relative position and Kepler's exact one.
pub fn position_error(s: &System, integrator: Integrator, dt: f64, t: f64) -> Result<f64, SpaceError> {
    if s.masses.len() != 2 {
        return err("the exact solution is the two-body one");
    }
    let mu = s.g * (s.masses[0] + s.masses[1]);
    let r0 = sub(s.positions[1], s.positions[0]);
    let v0 = sub(s.velocities[1], s.velocities[0]);
    let (exact, _) = propagate(mu, r0, v0, t)?;
    let out = integrate(s, integrator, dt, t)?;
    Ok(norm(sub(sub(out.positions[1], out.positions[0]), exact)))
}

/// The inverse of the Plummer cumulative mass: r = a / √(u^(−2/3) − 1), u floored at 1e-10.
pub fn plummer_radius(u: f64, scale: f64) -> f64 {
    scale / (u.max(1e-10).powf(-2.0 / 3.0) - 1.0).sqrt()
}

/// √(2GM / a) (1 + r² / a²)^(−1/4).
pub fn plummer_escape_speed(r: f64, total_mass: f64, scale: f64, g: f64) -> f64 {
    (2.0 * g * total_mass / scale).sqrt() * (1.0 + (r / scale).powi(2)).powf(-0.25)
}

/// Aarseth, Hénon and Wielen (1974): q in [0, 1) with density q² (1 − q²)^(7/2) by rejection against 0.1 U, the
/// draws of one body [`MAX_TRIES`] consecutive indices of `channel` and `channel + 1`; q and the draws it took.
pub fn plummer_q(seed: u64, channel: u64, index: u64) -> Result<(f64, usize), SpaceError> {
    for t in 0..MAX_TRIES {
        let k = index * MAX_TRIES as u64 + t as u64;
        let q = d24_unit(seed, channel, k);
        let y = 0.1 * d24_unit(seed, channel + 1, k);
        if y <= q * q * (1.0 - q * q).powf(3.5) {
            return Ok((q, t + 1));
        }
    }
    err(format!("no Plummer speed accepted in {MAX_TRIES} draws"))
}

fn direction(u: f64, v: f64, length: f64) -> Vec3 {
    let ct = 2.0 * u - 1.0;
    let ph = std::f64::consts::TAU * v;
    let st = (1.0 - ct * ct).max(0.0).sqrt();
    [length * st * ph.cos(), length * st * ph.sin(), length * ct]
}

/// One body of the cluster before recentring: channels 0 (radius), 1–2 (direction), 3–4 (speed), 5–6
/// (velocity direction).
pub fn plummer_sample(seed: u64, index: u64, total_mass: f64, scale: f64, g: f64) -> Result<(Vec3, Vec3), SpaceError> {
    let r = plummer_radius(d24_unit(seed, 0, index), scale);
    let p = direction(d24_unit(seed, 1, index), d24_unit(seed, 2, index), r);
    let (q, _) = plummer_q(seed, 3, index)?;
    let speed = q * plummer_escape_speed(r, total_mass, scale, g);
    Ok((p, direction(d24_unit(seed, 5, index), d24_unit(seed, 6, index), speed)))
}

/// A Plummer sphere of n equal masses drawn from the random streams, recentred and at rest. A signed `seed`
/// (`xs:long`) is taken as its two's-complement bits.
pub fn plummer(seed: u64, n: usize, total_mass: f64, scale: f64, g: f64) -> Result<System, SpaceError> {
    if !(2..=MAX_BODIES).contains(&n) {
        return err(format!("a cluster has 2 to {MAX_BODIES} bodies"));
    }
    if !(total_mass > 0.0 && total_mass.is_finite() && scale > 0.0 && scale.is_finite()) {
        return err("the cluster's mass and scale radius are positive");
    }
    let samples =
        (0..n as u64).map(|i| plummer_sample(seed, i, total_mass, scale, g)).collect::<Result<Vec<_>, _>>()?;
    let (p, v): (Vec<Vec3>, Vec<Vec3>) = samples.into_iter().unzip();
    Ok(at_rest(&system(&vec![total_mass / n as f64; n], &p, &v, g, 0.0)?))
}

/// The system at a frame: integrated from the space epoch over frame / fps × rate seconds, which OR3 makes a whole
/// number of steps.
pub fn frame_state(
    s: &System,
    integrator: Integrator,
    dt: f64,
    frame: u32,
    fps: f64,
    rate: f64,
) -> Result<System, SpaceError> {
    if !(fps > 0.0 && fps.is_finite() && rate >= 0.0 && rate.is_finite()) {
        return err("the frame is a non-negative integer, fps positive and the rate non-negative");
    }
    integrate(s, integrator, dt, f64::from(frame) / fps * rate)
}
