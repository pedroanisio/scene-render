//! The numeric table of the orbits draft (sr-core srep/srep-0000-orbits.md, Conformance), rules 3, 4 and 6, row
//! by row against `sr_sim::nbody`, with the conservation laws, the step rule, the adaptive IAS15 and the failure
//! paths of OR3 and of the system.

use sr_sim::nbody::{
    accelerations, angular_momentum, at_rest, centre_of_mass, energy, energy_drift, frame_state, integrate,
    integrate_adaptive, kinetic, momentum, plummer, plummer_escape_speed, plummer_q, plummer_radius, plummer_sample,
    position_error, potential, radau, reversibility, step_ias15, step_wisdom_holman, steps_to, system, Integrator,
    System, MAX_BODIES, MAX_STEPS, MAX_TRIES,
};
use sr_space::orbits::{elements_from_state, norm, propagate, sub, Vec3, AU_M, G, GM_SUN};
use sr_space::SpaceError;
use std::f64::consts::TAU;

const LEAPFROG: Integrator = Integrator::Leapfrog;
const WH: Integrator = Integrator::WisdomHolman;
const IAS15: Integrator = Integrator::Ias15;
const ZERO: Vec3 = [0.0; 3];
const EIGHT_PERIOD: f64 = 6.32591398;

fn sig6(x: f64) -> f64 {
    if x == 0.0 {
        return 0.0;
    }
    let d = 5 - x.abs().log10().floor() as i32;
    if d < 0 {
        let s = 10f64.powi(-d);
        return (x / s).round() * s;
    }
    let s = 10f64.powi(d);
    (x * s).round() / s
}

fn orbit_err<T: std::fmt::Debug>(r: Result<T, SpaceError>) -> String {
    match r {
        Err(SpaceError::Orbit(m)) => m,
        other => panic!("expected an orbit error, got {other:?}"),
    }
}

/// A circular pair of masses 1 and 1e-3 at unit separation, G = 1: the period is 2π.
fn pair() -> System {
    let (m, mu) = (1e-3, 1.0 + 1e-3);
    let v = f64::sqrt(mu);
    system(&[1.0, m], &[ZERO, [1.0, 0.0, 0.0]], &[[0.0, -v * m / mu, 0.0], [0.0, v / mu, 0.0]], 1.0, 0.0).unwrap()
}

fn three() -> System {
    at_rest(
        &system(
            &[1.0, 1e-3, 1e-4],
            &[ZERO, [1.0, 0.0, 0.0], [0.0, 2.0, 0.0]],
            &[ZERO, [0.0, 1.0, 0.0], [-f64::sqrt(0.5), 0.0, 0.0]],
            1.0,
            0.0,
        )
        .unwrap(),
    )
}

/// The Chenciner–Montgomery figure eight with Simó's initial conditions.
fn eight() -> System {
    let x1 = [0.97000436, -0.24308753, 0.0];
    let v3 = [-0.93240737, -0.86473146, 0.0];
    let h = [-v3[0] / 2.0, -v3[1] / 2.0, 0.0];
    system(&[1.0; 3], &[x1, [-x1[0], -x1[1], 0.0], ZERO], &[h, h, v3], 1.0, 0.0).unwrap()
}

fn earth() -> (Vec3, Vec3) {
    ([AU_M, 0.0, 0.0], [0.0, (GM_SUN / AU_M).sqrt(), 0.0])
}

#[test]
fn the_sun_pulls_the_earth_at_5_9316_mm_per_s2() {
    let (r, v) = earth();
    let s = system(&[1.98892e30, 5.9722e24], &[ZERO, r], &[ZERO, v], G, 0.0).unwrap();
    assert_eq!(sig6(norm(accelerations(&s, &s.positions)[1])), 0.0059316);
}

#[test]
fn plummer_softening_bounds_a_close_pull() {
    let s = system(&[1.0, 1.0], &[ZERO, [1e-3, 0.0, 0.0]], &[ZERO, ZERO], 1.0, 1.0).unwrap();
    assert_eq!(sig6(norm(accelerations(&s, &s.positions)[0])), 0.000999999);
    // ε enters squared, in the force and in the potential.
    let s = system(&[1.0, 1.0], &[ZERO, [1e-3, 0.0, 0.0]], &[ZERO, ZERO], 1.0, 2.0).unwrap();
    assert!((norm(accelerations(&s, &s.positions)[0]) / 0.00012499995312501466 - 1.0).abs() < 1e-14);
    assert!((potential(&s) / -0.49999993750001176 - 1.0).abs() < 1e-14);
    // Coincident bodies are admitted with softening, and pull nothing.
    let s = system(&[1.0, 1.0], &[ZERO, ZERO], &[ZERO, ZERO], 1.0, 1.0).unwrap();
    assert_eq!(accelerations(&s, &s.positions), vec![ZERO, ZERO]);
}

#[test]
fn the_unit_pair_has_energy_minus_5e_4_and_the_usual_sums() {
    let s = pair();
    assert_eq!(sig6(energy(&s)), -0.0005);
    assert_eq!(energy(&s), kinetic(&s) + potential(&s));
    assert!(norm(momentum(&s)) < 1e-18);
    assert!((centre_of_mass(&s)[0] - 1e-3 / 1.001).abs() < 1e-16);
    let l = angular_momentum(&s);
    assert!((l[2] - 1e-3 * f64::sqrt(1.001) / 1.001).abs() < 1e-15, "{l:?}");
    let r = at_rest(&s);
    assert!(norm(centre_of_mass(&r)) < 1e-16 && norm(momentum(&r)) < 1e-18);
}

#[test]
fn leapfrog_keeps_a_bounded_energy_error_of_second_order() {
    let lf50 = energy_drift(&pair(), LEAPFROG, TAU / 200.0, 50.0 * TAU).unwrap();
    let lf10 = energy_drift(&pair(), LEAPFROG, TAU / 200.0, 10.0 * TAU).unwrap();
    assert_eq!((sig6(lf50), sig6(lf10)), (2.43649e-07, 2.43649e-07));
    let ratio = position_error(&pair(), LEAPFROG, TAU / 100.0, TAU).unwrap()
        / position_error(&pair(), LEAPFROG, TAU / 200.0, TAU).unwrap();
    assert_eq!(sig6(ratio), 3.99551);
    assert_eq!(sig6(energy_drift(&pair(), LEAPFROG, TAU / 20.0, 20.0 * TAU).unwrap()), 0.00212794);
}

#[test]
fn wisdom_holman_is_exact_for_two_bodies_and_close_for_three() {
    assert!(energy_drift(&pair(), WH, TAU / 20.0, 20.0 * TAU).unwrap() < 1e-9);
    assert!(position_error(&pair(), WH, TAU / 20.0, 20.0 * TAU).unwrap() < 1e-9);
    assert_eq!(sig6(energy_drift(&three(), WH, TAU / 20.0, 50.0 * TAU).unwrap()), 1.31264e-06);
    // A single body drifts on a line.
    let one = system(&[1.0], &[[1.0, 2.0, 3.0]], &[[1.0, 0.0, -1.0]], 1.0, 0.0).unwrap();
    assert_eq!(integrate(&one, WH, 0.5, 2.0).unwrap().positions, vec![[3.0, 2.0, 1.0]]);
}

#[test]
fn every_scheme_runs_back_to_its_start() {
    assert!(reversibility(&pair(), LEAPFROG, TAU / 200.0, 10.0 * TAU).unwrap() < 1e-10);
    assert!(reversibility(&three(), WH, TAU / 20.0, 10.0 * TAU).unwrap() < 1e-9);
    assert!(reversibility(&pair(), IAS15, TAU / 20.0, 10.0 * TAU).unwrap() < 1e-10);
}

#[test]
fn the_gauss_radau_nodes_are_computed_and_integrate_to_degree_14() {
    let r = radau();
    assert_eq!((sig6(r.h[1]), sig6(r.h[7])), (0.0562626, 0.977521));
    assert_eq!(r.h[0], 0.0);
    assert!(r.h.windows(2).all(|w| w[0] < w[1]) && r.h[7] < 1.0);
    assert!((r.w.iter().sum::<f64>() - 1.0).abs() < 1e-14);
    for k in 0..=14 {
        let q: f64 = r.w.iter().zip(r.h.iter()).map(|(w, h)| w * h.powi(k)).sum();
        assert!((q - 1.0 / (k as f64 + 1.0)).abs() < 1e-13, "degree {k}: {q}");
    }
    assert_eq!(sig6(r.w.iter().zip(r.h.iter()).map(|(w, h)| w * h.powi(14)).sum()), 0.0666667);
    // Degree 15 is not exact: the rule is Radau's, not a lucky table.
    let q15: f64 = r.w.iter().zip(r.h.iter()).map(|(w, h)| w * h.powi(15)).sum();
    assert!((q15 - 1.0 / 16.0).abs() > 1e-10, "{}", q15 - 1.0 / 16.0);
    // The monomial rows: row j is ∏_{i=1..j} (τ − h_i).
    for j in 0..7 {
        for tau in [0.3, 0.77] {
            let mono: f64 = (0..7).map(|k| r.c[j][k] * f64::powi(tau, k as i32)).sum();
            let prod: f64 = (1..=j).map(|i| tau - r.h[i]).product();
            assert!((mono - prod).abs() < 1e-14, "row {j}");
        }
    }
}

#[test]
fn ias15_is_at_rounding_on_the_pair_and_closes_the_figure_eight() {
    assert!(position_error(&pair(), IAS15, TAU / 20.0, TAU).unwrap() < 1e-8);
    assert!(position_error(&pair(), IAS15, TAU / 40.0, TAU).unwrap() < 1e-12);
    assert!(energy_drift(&pair(), IAS15, TAU / 20.0, 20.0 * TAU).unwrap() < 1e-10);
    let e = eight();
    let out = integrate(&e, IAS15, EIGHT_PERIOD / 400.0, EIGHT_PERIOD).unwrap();
    let worst = out.positions.iter().zip(e.positions.iter()).map(|(a, b)| norm(sub(*a, *b))).fold(0.0, f64::max);
    assert!(worst < 1e-6, "{worst}");
    // The predictor carried from one step to the next is what the step converged to.
    let (one, b) = step_ias15(&pair(), TAU / 20.0, None);
    let (_, b2) = step_ias15(&pair(), TAU / 20.0, Some(&b));
    assert!(b.iter().zip(b2.iter()).all(|(x, y)| x.iter().zip(y.iter()).all(|(p, q)| (p - q).abs() < 1e-14)));
    assert!((one.t - TAU / 20.0).abs() < 1e-16);
}

#[test]
fn momentum_and_angular_momentum_are_conserved() {
    for integ in [LEAPFROG, WH, IAS15] {
        let s = three();
        let (p0, l0) = (momentum(&s), angular_momentum(&s));
        let out = integrate(&s, integ, TAU / 40.0, 5.0 * TAU).unwrap();
        assert!(norm(sub(momentum(&out), p0)) < 1e-15, "{integ:?} momentum");
        assert!(norm(sub(angular_momentum(&out), l0)) / norm(l0) < 1e-12, "{integ:?} angular momentum");
        assert!((out.t - 5.0 * TAU).abs() < 1e-12);
    }
}

#[test]
fn the_adaptive_ias15_lands_on_the_frame_and_bounds_its_error() {
    // An eccentric pair (e 0.9) started at apocentre, half a period on: the pericentre.
    let mu = 1.001;
    let (a, e) = (1.0, 0.9);
    let ra = a * (1.0 + e);
    let va = f64::sqrt(mu * (1.0 - e) / (a * (1.0 + e)));
    let m = 1e-3;
    let s =
        system(&[1.0, m], &[ZERO, [ra, 0.0, 0.0]], &[[0.0, -va * m / mu, 0.0], [0.0, va / mu, 0.0]], 1.0, 0.0).unwrap();
    let s = at_rest(&s);
    let half = std::f64::consts::PI * (a.powi(3) / mu).sqrt();
    let (out, stats) = integrate_adaptive(&s, half, 0.5, 1e-9, MAX_STEPS).unwrap();
    assert_eq!(out.t, half);
    let rel = sub(out.positions[1], out.positions[0]);
    let exact =
        propagate(mu, sub(s.positions[1], s.positions[0]), sub(s.velocities[1], s.velocities[0]), half).unwrap().0;
    assert!(norm(sub(rel, exact)) < 1e-9, "{}", norm(sub(rel, exact)));
    assert!((norm(rel) - a * (1.0 - e)).abs() < 1e-9);
    // The step shrank toward pericentre and grew by at most 1.4 between accepted steps.
    assert!(stats.min < stats.max / 5.0, "{stats:?}");
    // The steps are the reference's (orbits_reference.integrate_adaptive on the same system): the rejection
    // rule, the (ε / error)^(1/7) rescale and the growth bound of 1.4 fix the count; the sizes carry the rounding
    // of the error estimate, a difference of two nearly equal coefficients.
    assert_eq!(stats.count, 234, "{stats:?}");
    assert!((stats.min / 0.0010538303714477948 - 1.0).abs() < 1e-2, "{stats:?}");
    assert!((stats.max / 0.15405551392803749 - 1.0).abs() < 1e-4, "{stats:?}");
    // From a tiny first step the step grows by 1.4 at each accepted one: 1e-6 · (1.4ⁿ − 1) / 0.4 reaches 0.01 at
    // the 25th, which lands on the end.
    let (_, grow) = integrate_adaptive(&s, 0.01, 1e-6, 1e-9, MAX_STEPS).unwrap();
    assert_eq!((grow.count, grow.min), (25, 1e-6), "{grow:?}");
    assert!((grow.max / (1e-6 * 1.4f64.powi(23)) - 1.0).abs() < 1e-12, "{grow:?}");
    // The same call is the same result: no state outside the arguments.
    let (again, _) = integrate_adaptive(&s, half, 0.5, 1e-9, MAX_STEPS).unwrap();
    assert_eq!(again, out);
    // A tighter tolerance takes more steps; too few allowed steps is an error, not a truncated answer.
    let (_, tight) = integrate_adaptive(&s, half, 0.5, 1e-12, MAX_STEPS).unwrap();
    // (At 1e-12 the estimate sits at rounding, so its count is not pinned to the reference's.)
    assert!(tight.count > 3 * stats.count, "{tight:?}");
    assert!(orbit_err(integrate_adaptive(&s, half, 0.5, 1e-9, 10)).contains("more than 10 steps"));
    assert!(orbit_err(integrate_adaptive(&s, half, 0.0, 1e-9, MAX_STEPS)).contains("positive"));
    assert!(orbit_err(integrate_adaptive(&s, half, 0.5, 0.0, MAX_STEPS)).contains("positive"));
    let (zero, _) = integrate_adaptive(&s, 0.0, 0.5, 1e-9, MAX_STEPS).unwrap();
    assert_eq!(zero, s);
}

#[test]
fn the_step_must_divide_the_time() {
    assert_eq!(steps_to(600.0 / 24.0 * TAU / 10.0, TAU / 240.0).unwrap(), 600);
    assert_eq!(steps_to(0.0, 1.0).unwrap(), 0);
    assert!(orbit_err(steps_to(1.5, 1.0)).contains("whole number"));
    assert!(orbit_err(steps_to(1.0, 0.0)).contains("positive"));
    assert!(orbit_err(steps_to(-1.0, 1.0)).contains("non-negative"));
    assert!(orbit_err(steps_to(f64::NAN, 1.0)).contains("non-negative"));
    assert!(orbit_err(integrate(&pair(), LEAPFROG, 1.0 / 2097152.0, 1.0)).contains("more than"));
    assert!(orbit_err(integrate(&pair(), LEAPFROG, 0.3, 1.0)).contains("whole number"));
}

#[test]
fn a_frame_integrated_alone_is_the_frame_played() {
    let s = pair();
    let (step, rate) = (TAU / 240.0, TAU / 10.0);
    for integ in [LEAPFROG, WH, IAS15] {
        let direct = frame_state(&s, integ, step, 240, 24.0, rate).unwrap();
        let mut played = s.clone();
        for k in 1..=240u32 {
            let dt = f64::from(k) / 24.0 * rate - f64::from(k - 1) / 24.0 * rate;
            played = integrate(&played, integ, step, dt).unwrap();
        }
        let worst =
            direct.positions.iter().zip(played.positions.iter()).map(|(a, b)| norm(sub(*a, *b))).fold(0.0, f64::max);
        assert!(worst <= 1e-9, "{integ:?}: {worst}");
    }
    assert!(orbit_err(frame_state(&s, LEAPFROG, step, 1, 0.0, rate)).contains("fps"));
    assert!(orbit_err(frame_state(&s, LEAPFROG, step, 1, 24.0, -1.0)).contains("rate"));
}

#[test]
fn the_plummer_cluster_is_drawn_from_the_random_streams() {
    assert_eq!(sig6(plummer_radius(0.5, 1.0)), 1.30477);
    assert_eq!(sig6(plummer_escape_speed(1.0, 1.0, 1.0, 1.0)), 1.18921);
    assert_eq!(sig6(sr_vector::d24::d24_unit(7, 3, 11)), 0.54057);
    let c = plummer(42, 400, 1.0, 1.0, 1.0).unwrap();
    assert_eq!(sig6(2.0 * kinetic(&c) / potential(&c).abs()), 0.975192);
    let mut radii: Vec<f64> = c.positions.iter().map(|p| norm(*p)).collect();
    radii.sort_by(f64::total_cmp);
    assert_eq!(sig6(radii[200]), 1.33268);
    assert!(norm(centre_of_mass(&c)) < 1e-15 && norm(momentum(&c)) < 1e-15);
    assert!(c.masses.iter().all(|m| *m == 1.0 / 400.0));
    // A body's draw depends on the seed and its index alone: a larger cluster keeps a smaller one's bodies up to
    // the recentring shift.
    let small = plummer(42, 10, 1.0, 1.0, 1.0).unwrap();
    let big = plummer(42, 20, 1.0, 1.0, 1.0).unwrap();
    let shift = sub(big.positions[0], small.positions[0]);
    for i in 0..10 {
        assert!(norm(sub(sub(big.positions[i], small.positions[i]), shift)) < 1e-12);
    }
    let (p3, _) = plummer_sample(42, 3, 1.0, 1.0, 1.0).unwrap();
    assert!((norm(p3) - plummer_radius(sr_vector::d24::d24_unit(42, 0, 3), 1.0)).abs() < 1e-14);
    // The speed's rejection draws come from consecutive indices of channels 3 and 4, bounded.
    let (q, tries) = plummer_q(42, 3, 3).unwrap();
    assert!((0.0..1.0).contains(&q) && (1..=MAX_TRIES).contains(&tries));
    let other = plummer(43, 400, 1.0, 1.0, 1.0).unwrap();
    assert_ne!(other.positions[0], c.positions[0]);
    assert!(orbit_err(plummer(42, 1, 1.0, 1.0, 1.0)).contains("2 to 4096"));
    assert!(orbit_err(plummer(42, MAX_BODIES + 1, 1.0, 1.0, 1.0)).contains("2 to 4096"));
    assert!(orbit_err(plummer(42, 10, 0.0, 1.0, 1.0)).contains("positive"));
    assert!(orbit_err(plummer(42, 10, 1.0, -1.0, 1.0)).contains("positive"));
}

#[test]
fn a_system_refuses_what_it_cannot_move() {
    let one = |m: f64| system(&[m], &[ZERO], &[ZERO], 1.0, 0.0);
    assert!(orbit_err(one(0.0)).contains("positive"));
    assert!(orbit_err(one(f64::INFINITY)).contains("positive"));
    assert!(orbit_err(system(&[], &[], &[], 1.0, 0.0)).contains("bodies"));
    assert!(orbit_err(system(&[1.0], &[ZERO, ZERO], &[ZERO], 1.0, 0.0)).contains("bodies"));
    assert!(orbit_err(system(&[1.0], &[ZERO], &[ZERO], 0.0, 0.0)).contains("G is positive"));
    assert!(orbit_err(system(&[1.0], &[ZERO], &[ZERO], 1.0, -1.0)).contains("softening"));
    assert!(orbit_err(system(&[1.0], &[[f64::NAN, 0.0, 0.0]], &[ZERO], 1.0, 0.0)).contains("finite"));
    assert!(orbit_err(system(&[1.0, 2.0], &[ZERO, ZERO], &[ZERO, ZERO], 1.0, 0.0)).contains("coincide"));
    let n = MAX_BODIES + 1;
    assert!(orbit_err(system(&vec![1.0; n], &vec![ZERO; n], &vec![ZERO; n], 1.0, 1.0)).contains("bodies"));
    let soft = system(&[1.0, 1.0], &[ZERO, [1.0, 0.0, 0.0]], &[ZERO, ZERO], 1.0, 0.1).unwrap();
    assert!(orbit_err(integrate(&soft, WH, 0.1, 1.0)).contains("softening"));
    assert!(orbit_err(step_wisdom_holman(&soft, 0.1)).contains("softening"));
    assert!(orbit_err(position_error(&three(), LEAPFROG, 0.1, 1.0)).contains("two-body"));
    assert_eq!("wisdomHolman".parse::<Integrator>().unwrap(), WH);
    assert_eq!("leapfrog".parse::<Integrator>().unwrap(), LEAPFROG);
    assert_eq!("ias15".parse::<Integrator>().unwrap(), IAS15);
    assert!(orbit_err("rk4".parse::<Integrator>()).contains("leapfrog, wisdomHolman, ias15"));
}

#[test]
fn a_wisdom_holman_planet_keeps_its_kepler_elements() {
    // The Sun and the Earth in SI units: a year at a day's step keeps a and e of the Earth's relative orbit.
    let (r, v) = earth();
    let s = at_rest(&system(&[1.98892e30, 5.9722e24], &[ZERO, r], &[ZERO, v], G, 0.0).unwrap());
    let out = integrate(&s, WH, 86400.0, 365.0 * 86400.0).unwrap();
    let mu = G * (1.98892e30 + 5.9722e24);
    let el0 =
        elements_from_state(mu, sub(s.positions[1], s.positions[0]), sub(s.velocities[1], s.velocities[0])).unwrap();
    let el =
        elements_from_state(mu, sub(out.positions[1], out.positions[0]), sub(out.velocities[1], out.velocities[0]))
            .unwrap();
    assert!(((el.a - el0.a) / el0.a).abs() < 1e-12 && (el.e - el0.e).abs() < 1e-12);
}
