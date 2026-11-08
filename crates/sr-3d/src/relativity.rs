//! The sr-core relativity draft (`srep/srep-0000-relativity.md`, SP6 of SREP 12): Minkowski diagrams in units of c
//! (the Lorentz boost, the interval it keeps, rapidity, light cones, worldlines and the grid of a boosted frame),
//! the Penrose diamond and the Kruskal–Szekeres map of the Schwarzschild solution, Flamm's paraboloid and the
//! Newtonian potential sheet, and the strain of a gravitational wave with its chirp, each held to the draft's
//! reference module (`conformance/tools/relativity_reference.py`).
//!
//! Events are `(x, t)`. Embeddings use G = c = 1 in scene units; the chirp uses SI units (masses in kilograms).

// The domain checks are negated comparisons so that NaN is refused with the out-of-range values, as the reference's
// `not x > 0` refuses it.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use std::f64::consts::{FRAC_PI_2, PI};

/// The gravitational constant (CODATA 2018), m³ kg⁻¹ s⁻².
pub const G: f64 = 6.67430e-11;
/// The speed of light, m s⁻¹.
pub const C: f64 = 299_792_458.0;
/// The solar mass, kg.
pub const M_SUN: f64 = 1.98841e30;
/// The astronomical unit (IAU 2012), metres.
pub const AU_M: f64 = 149_597_870_700.0;
/// The parsec, 648 000 / π au, metres.
pub const PC_M: f64 = 648_000.0 / PI * AU_M;
/// The megaparsec, metres.
pub const MPC_M: f64 = 1.0e6 * PC_M;
/// Half the diagonal of the Penrose diamond: |X| + |T| < π/2.
pub const PENROSE_EXTENT: f64 = FRAC_PI_2;
/// The largest |h₊| and |hₓ| RD4 allows.
pub const MAX_STRAIN: f64 = 1.0;
/// The frames of the seek check.
pub const SEEK_FRAMES: u64 = 240;
/// Simpson panels of the arc length on Flamm's paraboloid.
pub const SIMPSON_PANELS: usize = 4096;
/// Bisection steps of the radius back from Kruskal coordinates.
pub const KRUSKAL_ITERATIONS: usize = 200;

/// A value outside the draft's domain.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{0}")]
pub struct RelativityError(pub String);

type Result<T> = std::result::Result<T, RelativityError>;

fn err<T>(message: &str) -> Result<T> {
    Err(RelativityError(message.into()))
}

/// An event `(x, t)`.
pub type Event = (f64, f64);

/// The sign of an interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Separation {
    Timelike,
    Lightlike,
    Spacelike,
}

/// The compactification of a diagram (rule 2): none, the Penrose diamond of Minkowski space, or the Penrose
/// diagram of the Schwarzschild solution of the given r_s.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Compact {
    None,
    Minkowski,
    Schwarzschild(f64),
}

/// A corner of the Penrose diamond.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Infinity {
    Spacelike,
    Future,
    Past,
}

/// The kind of a worldline (RD2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldlineKind {
    Inertial,
    Accelerated,
    Parametric,
}

/// A sheet of rule 3: Flamm's paraboloid of the given r_s, or the potential sheet z = −k/r of the given k.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Embedding {
    Flamm(f64),
    Potential(f64),
}

/// The ends of the future and past cone edges of an event.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cone {
    pub future: [Event; 2],
    pub past: [Event; 2],
}

/// The lines x′ = k (`x`) and t′ = k (`t`) of a boosted frame, k = −n ..= n, each as its two ends.
#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    pub x: Vec<[Event; 2]>,
    pub t: Vec<[Event; 2]>,
}

/// The rings and spokes of a sheet, as scene points.
#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddingGrid {
    pub rings: Vec<Vec<[f64; 3]>>,
    pub spokes: Vec<Vec<[f64; 3]>>,
}

/// The plots draft's axes box: its pixel rectangle and its data ranges (y up).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxesBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub x_min: f64,
    pub x_max: f64,
    pub y_min: f64,
    pub y_max: f64,
}

// ------------------------------------------------------------------------------------------- boosts and intervals

fn beta(b: f64) -> Result<f64> {
    if !(-1.0 < b && b < 1.0) {
        return err("beta must lie strictly between -1 and 1");
    }
    Ok(b)
}

/// 1 / √(1 − β²).
pub fn gamma(b: f64) -> Result<f64> {
    let b = beta(b)?;
    Ok(1.0 / (1.0 - b * b).sqrt())
}

/// atanh β.
pub fn rapidity(b: f64) -> Result<f64> {
    Ok(beta(b)?.atanh())
}

/// tanh φ.
pub fn beta_from_rapidity(phi: f64) -> f64 {
    phi.tanh()
}

/// The Lorentz boost of the event (x, t) into the frame moving at β (units of c).
pub fn boost(x: f64, t: f64, b: f64) -> Result<Event> {
    let g = gamma(b)?;
    Ok((g * (x - b * t), g * (t - b * x)))
}

/// [`boost`] with −β.
pub fn inverse_boost(x: f64, t: f64, b: f64) -> Result<Event> {
    boost(x, t, -beta(b)?)
}

/// t² − x² (timelike positive).
pub fn interval(x: f64, t: f64) -> f64 {
    t * t - x * x
}

/// The interval of the separation b − a.
pub fn interval_between(a: Event, b: Event) -> f64 {
    interval(b.0 - a.0, b.1 - a.1)
}

/// (β₁ + β₂) / (1 + β₁β₂).
pub fn compose_beta(b1: f64, b2: f64) -> Result<f64> {
    Ok((beta(b1)? + beta(b2)?) / (1.0 + b1 * b2))
}

/// γ: coordinate time per unit proper time.
pub fn time_dilation(b: f64) -> Result<f64> {
    gamma(b)
}

/// 1 / γ.
pub fn length_contraction(b: f64) -> Result<f64> {
    Ok(1.0 / gamma(b)?)
}

/// The relativistic Doppler factor √((1 + β) / (1 − β)) of an approaching source.
pub fn doppler(b: f64) -> Result<f64> {
    let b = beta(b)?;
    Ok(((1.0 + b) / (1.0 - b)).sqrt())
}

/// Timelike, lightlike or spacelike separation of two events (lightlike within 1e-12 of the larger component²).
pub fn classify(a: Event, b: Event) -> Separation {
    let s = interval_between(a, b);
    let scale = (b.0 - a.0).abs().max((b.1 - a.1).abs()).max(1.0);
    if s.abs() <= 1e-12 * scale * scale {
        Separation::Lightlike
    } else if s > 0.0 {
        Separation::Timelike
    } else {
        Separation::Spacelike
    }
}

/// √ of a timelike interval.
pub fn proper_time(a: Event, b: Event) -> Result<f64> {
    let s = interval_between(a, b);
    if !(s > 0.0) {
        return err("proper time needs a timelike separation");
    }
    Ok(s.sqrt())
}

/// √ of minus a spacelike interval.
pub fn proper_length(a: Event, b: Event) -> Result<f64> {
    let s = interval_between(a, b);
    if !(s < 0.0) {
        return err("proper length needs a spacelike separation");
    }
    Ok((-s).sqrt())
}

/// The ends of the future and past cones of an event, `extent` units of t away.
pub fn light_cone(event: Event, extent: f64) -> Result<Cone> {
    if !(extent > 0.0) {
        return err("extent must be positive");
    }
    let (x, t) = event;
    Ok(Cone {
        future: [(x - extent, t + extent), (x + extent, t + extent)],
        past: [(x - extent, t - extent), (x + extent, t - extent)],
    })
}

/// |dt / dx| of the two future cone edges from the event: (1, 1) for light.
pub fn cone_slopes(event: Event, future: [Event; 2]) -> (f64, f64) {
    let slope = |(x, t): Event| ((t - event.1) / (x - event.0)).abs();
    (slope(future[0]), slope(future[1]))
}

/// Whether `other` lies in the future light cone of `event`.
pub fn inside_future_cone(event: Event, other: Event) -> bool {
    let (dx, dt) = (other.0 - event.0, other.1 - event.1);
    dt > 0.0 && dt * dt - dx * dx > 0.0
}

/// The event at time t on the worldline through (x0, t0) at speed β.
pub fn worldline_inertial(x0: f64, t0: f64, b: f64, t: f64) -> Result<Event> {
    Ok((x0 + beta(b)? * (t - t0), t))
}

/// Rindler motion from rest at the origin: ((cosh aτ − 1)/a, sinh aτ/a).
pub fn worldline_accelerated(a: f64, tau: f64) -> Result<Event> {
    if !(a > 0.0) {
        return err("proper acceleration must be positive");
    }
    Ok((((a * tau).cosh() - 1.0) / a, (a * tau).sinh() / a))
}

/// tanh aτ, the speed on the Rindler worldline at proper time τ.
pub fn accelerated_beta(a: f64, tau: f64) -> Result<f64> {
    if !(a > 0.0) {
        return err("proper acceleration must be positive");
    }
    Ok((a * tau).tanh())
}

/// (x(τ), t(τ)), whose tangent (by central differences of step 1e-6) must be timelike and future-directed.
pub fn worldline_parametric(fx: impl Fn(f64) -> f64, ft: impl Fn(f64) -> f64, tau: f64) -> Result<Event> {
    const H: f64 = 1e-6;
    let dx = (fx(tau + H) - fx(tau - H)) / (2.0 * H);
    let dt = (ft(tau + H) - ft(tau - H)) / (2.0 * H);
    if !(dt > dx.abs()) {
        return err("a worldline's tangent is timelike and future-directed");
    }
    Ok((fx(tau), ft(tau)))
}

/// The event at time t on the line x′ = k of the frame moving at β.
pub fn grid_x_line(b: f64, k: f64, t: f64) -> Result<Event> {
    Ok((k / gamma(b)? + b * t, t))
}

/// The event at position x on the line t′ = k of the frame moving at β.
pub fn grid_t_line(b: f64, k: f64, x: f64) -> Result<Event> {
    Ok((x, k / gamma(b)? + b * x))
}

/// The lines x′ = k and t′ = k, k = −n ..= n, across ±extent.
pub fn boosted_grid(b: f64, n: u32, extent: f64) -> Result<Grid> {
    let n = i64::from(n);
    let (mut x, mut t) = (Vec::new(), Vec::new());
    for k in -n..=n {
        let k = k as f64;
        x.push([grid_x_line(b, k, -extent)?, grid_x_line(b, k, extent)?]);
        t.push([grid_t_line(b, k, -extent)?, grid_t_line(b, k, extent)?]);
    }
    Ok(Grid { x, t })
}

/// The box frame's lines x = k and t = k as a diagram drawn at boost β shows them: from the drawn frame the box
/// frame moves at −β, so they are `boosted_grid(−β, n, extent)` in the drawn frame's own coordinates.
pub fn diagram_grid(b: f64, n: u32, extent: f64) -> Result<Grid> {
    boosted_grid(-beta(b)?, n, extent)
}

// ------------------------------------------------------------------------------------------- compactification

/// The Penrose diamond: U = atan(t − x), V = atan(t + x); (X, T) = ((V − U)/2, (U + V)/2).
pub fn penrose(x: f64, t: f64) -> (f64, f64) {
    let (u, v) = ((t - x).atan(), (t + x).atan());
    ((v - u) / 2.0, (u + v) / 2.0)
}

/// The event of a point of the diamond.
pub fn penrose_inverse(px: f64, pt: f64) -> Result<Event> {
    let (u, v) = (pt - px, pt + px);
    if !(u.abs() < PENROSE_EXTENT && v.abs() < PENROSE_EXTENT) {
        return err("the point lies outside the diamond");
    }
    let (tu, tv) = (u.tan(), v.tan());
    Ok(((tv - tu) / 2.0, (tu + tv) / 2.0))
}

/// dT / dX of a null ray from an event in the diamond: +1 rightwards, −1 leftwards.
pub fn penrose_slope(event: Event, direction: f64) -> Result<f64> {
    if direction != 1.0 && direction != -1.0 {
        return err("direction is +1 or -1");
    }
    let a = penrose(event.0, event.1);
    let b = penrose(event.0 + direction, event.1 + 1.0);
    Ok((b.1 - a.1) / (b.0 - a.0))
}

/// The diamond's corners: spacelike infinity i⁰, future and past timelike infinity.
pub fn penrose_infinity(kind: Infinity) -> (f64, f64) {
    match kind {
        Infinity::Spacelike => (PENROSE_EXTENT, 0.0),
        Infinity::Future => (0.0, PENROSE_EXTENT),
        Infinity::Past => (0.0, -PENROSE_EXTENT),
    }
}

fn positive_rs(rs: f64) -> Result<()> {
    if !(rs > 0.0) {
        return err("the Schwarzschild radius must be positive");
    }
    Ok(())
}

/// Kruskal–Szekeres (T, X) of Schwarzschild (r, t) outside and inside the horizon, G = c = 1.
pub fn kruskal(r: f64, t: f64, rs: f64) -> Result<(f64, f64)> {
    positive_rs(rs)?;
    if !(r > 0.0) {
        return err("r must be positive");
    }
    let e = (r / (2.0 * rs)).exp();
    let (ch, sh) = ((t / (2.0 * rs)).cosh(), (t / (2.0 * rs)).sinh());
    Ok(if r >= rs {
        let p = (r / rs - 1.0).sqrt() * e;
        (p * sh, p * ch)
    } else {
        let p = (1.0 - r / rs).sqrt() * e;
        (p * ch, p * sh)
    })
}

/// T of the singularity r = 0 at X: T² − X² = 1.
pub fn kruskal_singularity(x: f64) -> f64 {
    (1.0 + x * x).sqrt()
}

/// r from X² − T² = (r/r_s − 1) e^(r/r_s), by bisection.
pub fn kruskal_radius(t: f64, x: f64, rs: f64) -> Result<f64> {
    positive_rs(rs)?;
    let q = x * x - t * t;
    if !(q >= -1.0) {
        return err("the point lies beyond the singularity");
    }
    if q == 0.0 {
        return Ok(rs);
    }
    let f = |r: f64| (r / rs - 1.0) * (r / rs).exp();
    let (mut lo, mut hi) = (0.0, rs);
    while f(hi) < q {
        hi *= 2.0;
    }
    for _ in 0..KRUSKAL_ITERATIONS {
        let mid = 0.5 * (lo + hi);
        if f(mid) < q {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Ok(0.5 * (lo + hi))
}

/// The Penrose diagram of the Schwarzschild solution: [`penrose`] of the Kruskal coordinates (X, T).
pub fn penrose_schwarzschild(r: f64, t: f64, rs: f64) -> Result<(f64, f64)> {
    let (kt, kx) = kruskal(r, t, rs)?;
    Ok(penrose(kx, kt))
}

/// A point of the diagram under its compactification.
pub fn compactify(x: f64, t: f64, mode: Compact) -> Result<(f64, f64)> {
    match mode {
        Compact::None => Ok((x, t)),
        Compact::Minkowski => Ok(penrose(x, t)),
        Compact::Schwarzschild(rs) => penrose_schwarzschild(x, t, rs),
    }
}

// ------------------------------------------------------------------------------------------- embeddings

fn flamm_domain(r: f64, rs: f64) -> Result<()> {
    positive_rs(rs)?;
    if !(r >= rs) {
        return err("Flamm's paraboloid starts at the horizon");
    }
    Ok(())
}

/// z = 2√(r_s (r − r_s)).
pub fn flamm(r: f64, rs: f64) -> Result<f64> {
    flamm_domain(r, rs)?;
    Ok(2.0 * (rs * (r - rs)).sqrt())
}

/// dz/dr = √(r_s / (r − r_s)), infinite at the horizon.
pub fn flamm_slope(r: f64, rs: f64) -> Result<f64> {
    flamm_domain(r, rs)?;
    Ok(if r == rs { f64::INFINITY } else { (rs / (r - rs)).sqrt() })
}

fn radial_range(r1: f64, r2: f64, rs: f64) -> Result<()> {
    flamm_domain(r1, rs)?;
    flamm_domain(r2, rs)?;
    if r2 < r1 {
        return err("r2 must not be below r1");
    }
    Ok(())
}

/// The integral of dr / √(1 − r_s/r) from r1 to r2, in closed form.
pub fn proper_radial_distance(r1: f64, r2: f64, rs: f64) -> Result<f64> {
    radial_range(r1, r2, rs)?;
    let f = |r: f64| (r * (r - rs)).sqrt() + rs * (r.sqrt() + (r - rs).sqrt()).ln();
    Ok(f(r2) - f(r1))
}

/// The arc length of a radial line on the paraboloid, by Simpson's rule in s = √(r − r_s), where the integrand
/// 2√(s² + r_s) is smooth.
pub fn flamm_arc_length(r1: f64, r2: f64, rs: f64) -> Result<f64> {
    radial_range(r1, r2, rs)?;
    let (s1, s2) = ((r1 - rs).sqrt(), (r2 - rs).sqrt());
    let n = SIMPSON_PANELS;
    let h = (s2 - s1) / n as f64;
    let total: f64 = (0..=n)
        .map(|i| {
            let s = s1 + i as f64 * h;
            let w = if i == 0 || i == n {
                1.0
            } else if i % 2 == 1 {
                4.0
            } else {
                2.0
            };
            w * 2.0 * (s * s + rs).sqrt()
        })
        .sum();
    Ok(total * h / 3.0)
}

/// The Gaussian curvature of Flamm's paraboloid, −r_s / (2r³).
pub fn flamm_curvature(r: f64, rs: f64) -> Result<f64> {
    flamm_domain(r, rs)?;
    Ok(-rs / (2.0 * r.powi(3)))
}

/// The Newtonian sheet z = −k/r.
pub fn potential(r: f64, k: f64) -> Result<f64> {
    if !(r > 0.0) {
        return err("r must be positive");
    }
    Ok(-k / r)
}

/// k / r².
pub fn potential_slope(r: f64, k: f64) -> Result<f64> {
    if !(r > 0.0) {
        return err("r must be positive");
    }
    Ok(k / (r * r))
}

/// The scene point of a sheet at (r, φ): (r cos φ, −z, r sin φ), z the sheet's height (scene y is down, so both
/// sheets are lowest at their centre: Flamm's throat at the horizon, the potential's well at r_min).
///
/// The reference's `embedding_point` takes the potential sheet's height as +k/r, which turns its well into a hill
/// and its depth at r_min into +k/r_min; rule 3 states z = −k/r and the kit case the depth −k/r_min.
pub fn embedding_point(sheet: Embedding, r: f64, phi: f64) -> Result<[f64; 3]> {
    let z = match sheet {
        Embedding::Flamm(rs) => flamm(r, rs)?,
        Embedding::Potential(k) => potential(r, k)?,
    };
    Ok([r * phi.cos(), -z, r * phi.sin()])
}

/// `rings` rings from r_min to r_max and `spokes` spokes from r_min to r_max, `samples` points each, on the sheet
/// of its own r_s or k (rule 3; the reference's `embedding_grid` takes the same scale as its own parameter).
pub fn embedding_grid(
    sheet: Embedding,
    r_min: f64,
    r_max: f64,
    rings: u32,
    spokes: u32,
    samples: u32,
) -> Result<EmbeddingGrid> {
    if !(r_min > 0.0 && r_max > r_min) {
        return err("r_max must exceed a positive r_min");
    }
    if rings < 1 || spokes < 3 || samples < 2 {
        return err("rings >= 1, spokes >= 3, samples >= 2");
    }
    let tau = 2.0 * PI;
    let ring_r = |i: u32| if rings > 1 { r_min + (r_max - r_min) * f64::from(i) / f64::from(rings - 1) } else { r_min };
    let spoke_r = |i: u32| r_min + (r_max - r_min) * f64::from(i) / f64::from(samples - 1);
    let rings = (0..rings)
        .map(|i| {
            (0..samples).map(|j| embedding_point(sheet, ring_r(i), tau * f64::from(j) / f64::from(samples))).collect()
        })
        .collect::<Result<_>>()?;
    let spokes = (0..spokes)
        .map(|j| {
            let phi = tau * f64::from(j) / f64::from(spokes);
            (0..samples).map(|i| embedding_point(sheet, spoke_r(i), phi)).collect()
        })
        .collect::<Result<_>>()?;
    Ok(EmbeddingGrid { rings, spokes })
}

// ------------------------------------------------------------------------------------------- strain

/// ((h₊x + hₓy)/2, (hₓx − h₊y)/2): the displacement of a plane wave along the viewing axis.
pub fn strain_displacement(x: f64, y: f64, h_plus: f64, h_cross: f64) -> (f64, f64) {
    ((h_plus * x + h_cross * y) / 2.0, (h_cross * x - h_plus * y) / 2.0)
}

/// The point displaced at time t: the displacement times cos(2πft + phase).
pub fn strain_point(x: f64, y: f64, h_plus: f64, h_cross: f64, frequency: f64, t: f64, phase: f64) -> (f64, f64) {
    let (dx, dy) = strain_displacement(x, y, h_plus, h_cross);
    let c = (2.0 * PI * frequency * t + phase).cos();
    (x + dx * c, y + dy * c)
}

/// The semi-axes a unit ring takes at the wave's crest: 1 ± h/2, h = √(h₊² + hₓ²).
pub fn ring_axes(h_plus: f64, h_cross: f64) -> (f64, f64) {
    let h = h_plus.hypot(h_cross);
    (1.0 + h / 2.0, 1.0 - h / 2.0)
}

/// `n` points of a ring of the given radius under the strain.
pub fn strained_ring(
    h_plus: f64,
    h_cross: f64,
    n: u32,
    frequency: f64,
    radius: f64,
    t: f64,
    phase: f64,
) -> Vec<(f64, f64)> {
    (0..n)
        .map(|i| {
            let a = 2.0 * PI * f64::from(i) / f64::from(n);
            strain_point(radius * a.cos(), radius * a.sin(), h_plus, h_cross, frequency, t, phase)
        })
        .collect()
}

/// (m₁m₂)^(3/5) / (m₁ + m₂)^(1/5).
pub fn chirp_mass(m1: f64, m2: f64) -> Result<f64> {
    if !(m1 > 0.0 && m2 > 0.0) {
        return err("masses must be positive");
    }
    Ok((m1 * m2).powf(0.6) / (m1 + m2).powf(0.2))
}

fn chirp_time(mc: f64) -> Result<f64> {
    if !(mc > 0.0) {
        return err("the chirp mass must be positive");
    }
    Ok(G * mc / C.powi(3))
}

/// The quadrupole-formula frequency τ seconds before coalescence, (1/π)(5/(256τ))^(3/8)(GM_c/c³)^(−5/8).
pub fn gw_frequency(tau: f64, mc: f64) -> Result<f64> {
    if !(tau > 0.0) {
        return err("tau must be positive");
    }
    Ok((5.0 / (256.0 * tau)).powf(0.375) * chirp_time(mc)?.powf(-0.625) / PI)
}

/// The inverse of [`gw_frequency`]: (5/256)(GM_c/c³)^(−5/3)(πf)^(−8/3).
pub fn time_to_merger(f: f64, mc: f64) -> Result<f64> {
    if !(f > 0.0) {
        return err("the frequency must be positive");
    }
    Ok(5.0 / 256.0 * chirp_time(mc)?.powf(-5.0 / 3.0) * (PI * f).powf(-8.0 / 3.0))
}

/// (4/d)(GM_c/c²)^(5/3)(πf/c)^(2/3).
pub fn gw_amplitude(d: f64, mc: f64, f: f64) -> Result<f64> {
    if !(d > 0.0) {
        return err("the distance must be positive");
    }
    if !(f > 0.0) {
        return err("the frequency must be positive");
    }
    chirp_time(mc)?;
    Ok(4.0 / d * (G * mc / (C * C)).powf(5.0 / 3.0) * (PI * f / C).powf(2.0 / 3.0))
}

/// Cycles of the wave from τ1 to τ2 ≤ τ1 before coalescence: the integral of f dτ in closed form.
pub fn chirp_cycles(tau1: f64, tau2: f64, mc: f64) -> Result<f64> {
    if tau2 > tau1 {
        return err("tau2 must not exceed tau1");
    }
    if !(tau2 > 0.0) {
        return err("tau must be positive");
    }
    let k = (5.0f64 / 256.0).powf(0.375) * chirp_time(mc)?.powf(-0.625) / PI;
    Ok(k * 1.6 * (tau1.powf(0.625) - tau2.powf(0.625)))
}

// ------------------------------------------------------------------------------------------- animation and mapping

/// The boost at a frame: linear in rapidity from β₀ at t₀ to β₁ at t₁, held outside.
pub fn frame_beta(frame: u64, fps: f64, b0: f64, b1: f64, t0: f64, t1: f64) -> Result<f64> {
    if !(fps > 0.0) {
        return err("fps must be positive");
    }
    if !(t1 > t0) {
        return err("t1 must exceed t0");
    }
    let u = ((frame as f64 / fps - t0) / (t1 - t0)).clamp(0.0, 1.0);
    let (p0, p1) = (rapidity(b0)?, rapidity(b1)?);
    Ok(beta_from_rapidity(p0 + (p1 - p0) * u))
}

/// Composing the per-frame boost increments reproduces the direct frame boost at every frame, and the interval of
/// (3, 5) stays 16.
pub fn seek_test() -> bool {
    let (fps, b0, b1, t0, t1) = (24.0, 0.0, 0.8, 0.0, 10.0);
    let (Ok(p0), Ok(p1)) = (rapidity(b0), rapidity(b1)) else { return false };
    let step = beta_from_rapidity((p1 - p0) / ((t1 - t0) * fps));
    let mut played = b0;
    for frame in 0..=SEEK_FRAMES {
        let Ok(direct) = frame_beta(frame, fps, b0, b1, t0, t1) else { return false };
        if (direct - played).abs() > 1e-12 {
            return false;
        }
        match boost(3.0, 5.0, direct) {
            Ok((x, t)) if (interval(x, t) - 16.0).abs() <= 1e-9 => {}
            _ => return false,
        }
        let Ok(next) = compose_beta(played, step) else { return false };
        played = next;
    }
    true
}

/// The pixel of an event: boost, compactify, then the plots draft's axes mapping (y down).
pub fn diagram_pixel(x: f64, t: f64, b: &AxesBox, beta: f64, compact: Compact) -> Result<(f64, f64)> {
    if !(b.x_max > b.x_min && b.y_max > b.y_min) {
        return err("the axes ranges must be increasing");
    }
    let (xb, tb) = boost(x, t, beta)?;
    let (xc, tc) = compactify(xb, tb, compact)?;
    Ok((b.x + (xc - b.x_min) / (b.x_max - b.x_min) * b.width, b.y + (b.y_max - tc) / (b.y_max - b.y_min) * b.height))
}

// ------------------------------------------------------------------------------------------- validation

/// RD1: c positive, |β| below 1, and a positive r_s for the Schwarzschild diagram.
pub fn validate_spacetime(c: f64, b: f64, compact: Compact) -> Result<()> {
    if !(c > 0.0) {
        return err("RD1: c must be positive");
    }
    if beta(b).is_err() {
        return err("RD1: boost must lie strictly between -1 and 1");
    }
    if let Compact::Schwarzschild(rs) = compact {
        if !(rs > 0.0) {
            return err("RD1: schwarzschild needs a black hole of positive rs");
        }
    }
    Ok(())
}

/// RD2: an inertial worldline has |velocity| below 1; an accelerated one a positive acceleration.
pub fn validate_worldline(kind: WorldlineKind, velocity: Option<f64>, acceleration: Option<f64>) -> Result<()> {
    match kind {
        WorldlineKind::Inertial => match velocity {
            None => err("RD2: an inertial worldline needs a velocity"),
            Some(v) if beta(v).is_err() => err("RD2: velocity must lie strictly between -1 and 1"),
            Some(_) => Ok(()),
        },
        WorldlineKind::Accelerated => match acceleration {
            Some(a) if a > 0.0 => Ok(()),
            _ => err("RD2: an accelerated worldline needs a positive acceleration"),
        },
        WorldlineKind::Parametric => Ok(()),
    }
}

/// RD3: 0 < rMin < rMax, rings ≥ 1, spokes ≥ 3.
pub fn validate_embedding(r_min: f64, r_max: f64, rings: u32, spokes: u32) -> Result<()> {
    if !(r_min > 0.0 && r_max > r_min) {
        return err("RD3: rMax must exceed a positive rMin");
    }
    if rings < 1 || spokes < 3 {
        return err("RD3: rings >= 1 and spokes >= 3");
    }
    Ok(())
}

/// RD4: |h₊| and |hₓ| at most 1; exactly one of a positive frequency and a positive chirp mass, the chirp mass with
/// a merger time.
pub fn validate_strain(
    h_plus: f64,
    h_cross: f64,
    frequency: Option<f64>,
    chirp_mass: Option<f64>,
    merger: Option<f64>,
) -> Result<()> {
    if !(h_plus.abs() <= MAX_STRAIN && h_cross.abs() <= MAX_STRAIN) {
        return err("RD4: |hPlus| and |hCross| are at most 1");
    }
    match (frequency, chirp_mass) {
        (Some(_), Some(_)) | (None, None) => err("RD4: a strain has exactly one of frequency and chirpMass"),
        (Some(f), None) if !(f > 0.0) => err("RD4: frequency must be positive"),
        (None, Some(m)) if !(m > 0.0) => err("RD4: chirpMass must be positive"),
        (None, Some(_)) if merger.is_none() => err("RD4: chirpMass comes with merger"),
        _ => Ok(()),
    }
}
