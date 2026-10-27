//! The numeric table of the orbits draft (sr-core srep/srep-0000-orbits.md, Conformance), rules 1 and 2, row by
//! row against `sr_space::orbits`, with the invariants the rows stand for (round trips, conservation, closure)
//! and the failure paths of OR1 and OR2.

use sr_space::orbits::{
    closure, elements_from_state, kepler_elliptic, kepler_hyperbolic, norm, orbit, orbit_mu, orbit_state, period,
    propagate, specific_energy, state_from_elements, stumpff_c2, stumpff_c3, sub, Vec3, AU_M, DAY_S, GM, GM_SUN,
};
use sr_space::SpaceError;

/// Six significant digits, the table's rounding.
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

fn earth() -> (Vec3, Vec3) {
    let v = (GM_SUN / AU_M).sqrt();
    ([AU_M, 0.0, 0.0], [0.0, v, 0.0])
}

fn orbit_err(r: Result<impl std::fmt::Debug, SpaceError>) -> String {
    match r {
        Err(SpaceError::Orbit(m)) => m,
        other => panic!("expected an orbit error, got {other:?}"),
    }
}

#[test]
fn the_earths_period_at_one_au_is_365_257_days() {
    assert_eq!(sig6(period(GM_SUN, AU_M).unwrap() / DAY_S), 365.257);
}

#[test]
fn keplers_equation_is_solved_in_both_conics() {
    let e = kepler_elliptic(2.0, 0.5).unwrap();
    assert_eq!(sig6(e), 2.35424);
    assert!((e - 0.5 * e.sin() - 2.0).abs() < 1e-14);
    let h = kepler_hyperbolic(6.0, 2.0).unwrap();
    assert_eq!(sig6(h), 2.10769);
    assert!((2.0 * h.sinh() - h - 6.0).abs() < 1e-13);
    // The elliptic mean anomaly is reduced to (-pi, pi], so a whole turn more gives the same root.
    assert!((kepler_elliptic(2.0 + 2.0 * std::f64::consts::PI, 0.5).unwrap() - e).abs() < 1e-12);
    for e in [0.0, 0.3, 0.9, 0.999] {
        for m in [-3.0, -0.1, 0.0, 1e-9, 1.5, 3.1] {
            let x = kepler_elliptic(m, e).unwrap();
            assert!((x - e * x.sin() - m).abs() < 1e-13, "M {m} e {e}");
        }
    }
}

#[test]
fn keplers_equation_refuses_the_other_conic() {
    assert!(orbit_err(kepler_elliptic(1.0, 1.0)).contains("[0, 1)"));
    assert!(orbit_err(kepler_elliptic(1.0, -0.1)).contains("[0, 1)"));
    assert!(orbit_err(kepler_hyperbolic(1.0, 1.0)).contains("above 1"));
    assert!(orbit_err(kepler_elliptic(f64::NAN, 0.5)).contains("finite"));
}

#[test]
fn the_stumpff_functions_meet_their_series_at_the_parabola() {
    assert_eq!((sig6(stumpff_c2(4.0)), sig6(stumpff_c3(4.0))), (0.354037, 0.136338));
    // Either side of the series' edge |z| = 1e-3 the closed form and the series agree.
    for z in [1e-3f64 - 1e-12, 1e-3 + 1e-12, -1e-3 - 1e-12, -1e-3 + 1e-12] {
        let s = z.abs().sqrt();
        let (c2, c3) = if z > 0.0 {
            ((1.0 - s.cos()) / z, (s - s.sin()) / s.powi(3))
        } else {
            ((s.cosh() - 1.0) / -z, (s.sinh() - s) / s.powi(3))
        };
        assert!((stumpff_c2(z) - c2).abs() < 1e-9, "c2 at {z}");
        assert!((stumpff_c3(z) - c3).abs() < 1e-8, "c3 at {z}");
    }
    // Against the full series Σ (−z)^k / (2k + 2)! and Σ (−z)^k / (2k + 3)!, summed to rounding, over |z| ≤ 1:
    // the short series inside 1e-3 is at rounding, the closed form outside it within its cancellation, ~ε / |z|.
    let series = |z: f64, first: u32| {
        let (mut term, mut sum, mut k) = (1.0, 0.0, 0u32);
        for n in 1..=first {
            term /= n as f64;
        }
        while k < 30 {
            sum += term;
            term *= -z / (((first + 2 * k + 1) * (first + 2 * k + 2)) as f64);
            k += 1;
        }
        sum
    };
    for z in [-1.0f64, -0.3, -0.09, -1e-3, -5e-4, -1e-6, 1e-6, 5e-4, 9.99e-4, 1e-3, 0.09, 0.3, 1.0] {
        let tol = if z.abs() < 1e-3 { 3e-15 } else { 3e-15 + f64::EPSILON / z.abs() };
        assert!((stumpff_c2(z) - series(z, 2)).abs() < tol, "c2 at {z}: {}", stumpff_c2(z) - series(z, 2));
        assert!((stumpff_c3(z) - series(z, 3)).abs() < tol, "c3 at {z}: {}", stumpff_c3(z) - series(z, 3));
    }
    assert_eq!(stumpff_c2(0.0), 0.5);
    assert_eq!(stumpff_c3(0.0), 1.0 / 6.0);
    // The hyperbolic branch.
    assert!((stumpff_c2(-4.0) - (2f64.cosh() - 1.0) / 4.0).abs() < 1e-15);
}

#[test]
fn the_circular_one_au_orbit_has_unit_a_and_zero_e() {
    let (r, v) = earth();
    let el = elements_from_state(GM_SUN, r, v).unwrap();
    assert_eq!(sig6(el.a / AU_M), 1.0);
    assert!(el.e < 1e-12);
    assert_eq!((el.i, el.raan, el.argp), (0.0, 0.0, 0.0));
}

#[test]
fn the_perihelion_of_the_earths_ellipse_is_0_9833_au() {
    let (r, _) = state_from_elements(GM_SUN, AU_M, 0.0167, 0.0, 0.0, 102.9, 0.0).unwrap();
    assert_eq!(sig6(norm(r) / AU_M), 0.9833);
}

#[test]
fn a_quarter_period_after_perihelion_the_true_anomaly_is_122_543() {
    let (r, v) = state_from_elements(GM_SUN, AU_M, 0.3, 0.0, 0.0, 0.0, 0.0).unwrap();
    let (r1, v1) = propagate(GM_SUN, r, v, period(GM_SUN, AU_M).unwrap() / 4.0).unwrap();
    assert_eq!(sig6(elements_from_state(GM_SUN, r1, v1).unwrap().nu), 122.543);
}

#[test]
fn elements_come_back_from_their_state() {
    let cases: [(f64, f64, f64, f64, f64, f64); 6] = [
        (AU_M, 0.3, 7.0, 10.0, 20.0, 30.0),
        (0.3 * AU_M, 0.9, 60.0, 200.0, 300.0, 170.0),
        (2.0 * AU_M, 0.1, 135.0, 45.0, 80.0, 250.0), // retrograde
        (AU_M, 0.5, 179.0, 300.0, 10.0, 5.0),
        (-0.5 * AU_M, 1.5, 30.0, 60.0, 90.0, 20.0), // hyperbola
        (-AU_M, 3.0, 100.0, 120.0, 140.0, 330.0),
    ];
    for (a, e, i, raan, argp, nu) in cases {
        let (r, v) = state_from_elements(GM_SUN, a, e, i, raan, argp, nu).unwrap();
        let el = elements_from_state(GM_SUN, r, v).unwrap();
        assert!(((el.a - a) / a).abs() < 1e-12, "a {a} -> {}", el.a);
        assert!((el.e - e).abs() < 1e-12, "e {e} -> {}", el.e);
        let ang = |x: f64, y: f64| {
            let d = (x - y).rem_euclid(360.0);
            d.min(360.0 - d)
        };
        assert!(ang(el.i, i) < 1e-9 && ang(el.raan, raan) < 1e-9 && ang(el.argp, argp) < 1e-9, "{el:?}");
        assert!(ang(el.nu, nu) < 1e-9, "nu {nu} -> {}", el.nu);
        assert_eq!(el.period.is_some(), e < 1.0);
    }
    // Equatorial: the node is x̂ and raan 0; argp carries the pericentre's longitude.
    let (r, v) = state_from_elements(GM_SUN, AU_M, 0.2, 0.0, 0.0, 70.0, 40.0).unwrap();
    let el = elements_from_state(GM_SUN, r, v).unwrap();
    assert_eq!(el.raan, 0.0);
    assert!((el.argp - 70.0).abs() < 1e-9 && (el.nu - 40.0).abs() < 1e-9);
    // Circular: argp is 0 and nu is the argument of latitude.
    let (r, v) = state_from_elements(GM_SUN, AU_M, 0.0, 20.0, 50.0, 0.0, 75.0).unwrap();
    let el = elements_from_state(GM_SUN, r, v).unwrap();
    assert_eq!(el.argp, 0.0);
    assert!((el.nu - 75.0).abs() < 1e-9 && (el.raan - 50.0).abs() < 1e-9);
}

#[test]
fn an_ellipse_closes_on_itself_after_one_period() {
    let (r, v) = state_from_elements(GM_SUN, AU_M, 0.3, 7.0, 10.0, 20.0, 30.0).unwrap();
    assert!(closure(GM_SUN, r, v).unwrap() < 1e-12);
    let (r, v) = state_from_elements(GM_SUN, 0.3 * AU_M, 0.9, 60.0, 0.0, 0.0, 170.0).unwrap();
    assert!(closure(GM_SUN, r, v).unwrap() < 1e-12);
    let (r, v) = state_from_elements(GM_SUN, -0.5 * AU_M, 1.5, 0.0, 0.0, 0.0, 0.0).unwrap();
    assert!(orbit_err(closure(GM_SUN, r, v)).contains("ellipse"));
}

#[test]
fn a_hyperbola_ten_days_out_and_back_is_within_a_millimetre() {
    let (r, v) = state_from_elements(GM_SUN, -0.5 * AU_M, 1.5, 30.0, 60.0, 90.0, 20.0).unwrap();
    let (r1, v1) = propagate(GM_SUN, r, v, 10.0 * DAY_S).unwrap();
    let (r2, _) = propagate(GM_SUN, r1, v1, -10.0 * DAY_S).unwrap();
    assert!(norm(sub(r2, r)) < 1e-3);
}

#[test]
fn propagation_keeps_the_energy_and_composes() {
    let (r, v) = state_from_elements(GM_SUN, 0.3 * AU_M, 0.9, 60.0, 0.0, 0.0, 170.0).unwrap();
    let e0 = specific_energy(GM_SUN, r, v);
    let p = period(GM_SUN, 0.3 * AU_M).unwrap();
    for k in [-7.3, -0.5, 0.01, 0.25, 0.5, 3.7, 100.0] {
        let (r1, v1) = propagate(GM_SUN, r, v, k * p).unwrap();
        assert!(((specific_energy(GM_SUN, r1, v1) - e0) / e0).abs() < 1e-12, "{k} periods");
    }
    // dt 0 is the state itself, and two steps are one.
    assert_eq!(propagate(GM_SUN, r, v, 0.0).unwrap(), (r, v));
    let (ra, va) = propagate(GM_SUN, r, v, 0.3 * p).unwrap();
    let (rb, _) = propagate(GM_SUN, ra, va, 0.4 * p).unwrap();
    let (rc, _) = propagate(GM_SUN, r, v, 0.7 * p).unwrap();
    assert!(norm(sub(rb, rc)) / norm(rc) < 1e-12);
    // Near the parabola's edge (e 1 + 1e-9 is not admitted, but a state very close to escape speed is).
    let vesc = (2.0 * GM_SUN / AU_M).sqrt();
    let (r1, v1) = propagate(GM_SUN, [AU_M, 0.0, 0.0], [0.0, vesc * (1.0 + 1e-13), 0.0], 30.0 * DAY_S).unwrap();
    let e1 = specific_energy(GM_SUN, r1, v1);
    assert!(e1.abs() < 1e-6 * GM_SUN / AU_M, "near-parabolic energy {e1}");
}

#[test]
fn propagation_refuses_what_has_no_orbit() {
    assert!(orbit_err(propagate(0.0, [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], 1.0)).contains("mu"));
    assert!(orbit_err(propagate(1.0, [0.0; 3], [0.0, 1.0, 0.0], 1.0)).contains("centre"));
    assert!(orbit_err(propagate(1.0, [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], f64::INFINITY)).contains("finite"));
}

#[test]
fn the_orbit_element_reaches_aphelion_half_a_period_later() {
    let o = orbit(GM_SUN, AU_M, 0.0167, 0.0, 0.0, 102.9, 0.0, 0.0).unwrap();
    let p = period(GM_SUN, AU_M).unwrap();
    let (r, _) = orbit_state(&o, p / 2.0).unwrap();
    assert_eq!(sig6(norm(r) / AU_M), 1.0167);
    // Back in time as well, and with an epoch the state is the elements' at that epoch.
    let (r, _) = orbit_state(&o, -p / 2.0).unwrap();
    assert_eq!(sig6(norm(r) / AU_M), 1.0167);
    let o = orbit(GM_SUN, AU_M, 0.0167, 0.0, 0.0, 102.9, 0.0, 1000.0).unwrap();
    assert_eq!(
        orbit_state(&o, 1000.0).unwrap(),
        state_from_elements(GM_SUN, AU_M, 0.0167, 0.0, 0.0, 102.9, 0.0).unwrap()
    );
    assert!(orbit_err(orbit(GM_SUN, AU_M, 0.0, 0.0, 0.0, 0.0, 0.0, f64::NAN)).contains("epoch"));
}

#[test]
fn mu_comes_from_a_body_or_a_mass() {
    let (b, m) = (orbit_mu(None, Some(399)).unwrap(), orbit_mu(Some(1.98892e30), None).unwrap());
    assert_eq!((sig6(b), sig6(m)), (3.986e14, 1.32746e20));
    assert_eq!(GM.len(), 10);
    assert_eq!(orbit_mu(Some(2.0), Some(399)).unwrap(), 2.0 * sr_space::orbits::G);
    assert!(orbit_err(orbit_mu(None, Some(401))).contains("401"));
    assert!(orbit_err(orbit_mu(None, None)).contains("mass or a body"));
    assert!(orbit_err(orbit_mu(Some(0.0), None)).contains("positive"));
}

#[test]
fn or2_conics_are_refused_at_the_parabola_and_with_the_wrong_sign() {
    assert!(orbit_err(state_from_elements(GM_SUN, AU_M, 1.0, 0.0, 0.0, 0.0, 0.0)).contains("parabola"));
    assert!(orbit_err(state_from_elements(GM_SUN, AU_M, 1.0 + 1e-13, 0.0, 0.0, 0.0, 0.0)).contains("parabola"));
    assert!(orbit_err(state_from_elements(GM_SUN, -AU_M, 0.5, 0.0, 0.0, 0.0, 0.0)).contains("positive"));
    assert!(orbit_err(state_from_elements(GM_SUN, AU_M, 1.5, 0.0, 0.0, 0.0, 0.0)).contains("negative"));
    assert!(orbit_err(state_from_elements(GM_SUN, AU_M, -0.1, 0.0, 0.0, 0.0, 0.0)).contains("non-negative"));
    // 1 + 1.5 cos(150°) < 0: beyond the asymptote.
    assert!(orbit_err(state_from_elements(GM_SUN, -AU_M, 1.5, 0.0, 0.0, 0.0, 150.0)).contains("asymptotes"));
    assert!(orbit_err(state_from_elements(-1.0, AU_M, 0.5, 0.0, 0.0, 0.0, 0.0)).contains("mu"));
    assert!(orbit_err(elements_from_state(GM_SUN, [AU_M, 0.0, 0.0], [1.0, 0.0, 0.0])).contains("radial"));
    let vesc = (2.0 * GM_SUN / AU_M).sqrt();
    assert!(orbit_err(elements_from_state(GM_SUN, [AU_M, 0.0, 0.0], [0.0, vesc, 0.0])).contains("parabolic"));
}
