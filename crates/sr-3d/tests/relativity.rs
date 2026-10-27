//! The relativity draft's numeric table (sr-core `srep/srep-0000-relativity.md`, rules 1 to 5) against
//! `sr_3d::relativity`, row by row, the seek check of rule 5, and the values its rules refuse. The expected values
//! are the table's, which `conformance/tools/relativity_reference.py` prints to six significant digits.

// The expected values are the table's six-digit numbers, some of which are near ln 2, π/4 and π/2.
#![allow(clippy::approx_constant)]

use sr_3d::relativity::*;

/// Six significant digits, as the table prints them (and 5e-7 absolute about zero).
#[track_caller]
fn near(got: f64, want: f64) {
    let tol = (5e-6 * want.abs()).max(5e-7);
    assert!((got - want).abs() <= tol, "{got} against {want}");
}

#[track_caller]
fn near2(got: (f64, f64), want: (f64, f64)) {
    near(got.0, want.0);
    near(got.1, want.1);
}

/// The box of the table's last row: 400 × 300 pixels at (100, 50) over x ∈ [−2, 2], t ∈ [−1.5, 1.5].
const BOX: AxesBox =
    AxesBox { x: 100.0, y: 50.0, width: 400.0, height: 300.0, x_min: -2.0, x_max: 2.0, y_min: -1.5, y_max: 1.5 };

#[test]
fn rule_1_boosts_and_intervals() {
    near(gamma(0.6).unwrap(), 1.25);
    near(rapidity(0.6).unwrap(), 0.693147);
    near2(boost(1.0, 0.0, 0.6).unwrap(), (1.25, -0.75));
    near2(boost(0.0, 1.0, 0.6).unwrap(), (-0.75, 1.25));
    assert_eq!(interval(3.0, 5.0), 16.0);
    for beta in [0.6, 0.999, -0.999, 0.3] {
        let (x, t) = boost(3.0, 5.0, beta).unwrap();
        assert!((interval(x, t) - 16.0).abs() <= 1e-9, "{beta}: {}", interval(x, t));
        // and the inverse boost takes the event back
        let back = inverse_boost(x, t, beta).unwrap();
        assert!((back.0 - 3.0).abs() <= 1e-9 && (back.1 - 5.0).abs() <= 1e-9, "{beta}: {back:?}");
    }
    near(compose_beta(0.5, 0.5).unwrap(), 0.8);
    near(beta_from_rapidity(rapidity(0.5).unwrap() + rapidity(0.5).unwrap()), 0.8);
    near(time_dilation(0.6).unwrap(), 1.25);
    near(length_contraction(0.6).unwrap(), 0.8);
    near(doppler(0.6).unwrap(), 2.0);
}

#[test]
fn rule_1_separations_cones_and_worldlines() {
    assert_eq!(classify((0.0, 0.0), (1.0, 2.0)), Separation::Timelike);
    assert_eq!(classify((0.0, 0.0), (2.0, 1.0)), Separation::Spacelike);
    let (a, b) = (boost(0.0, 0.0, 0.9).unwrap(), boost(3.0, 3.0, 0.9).unwrap());
    assert_eq!(classify(a, b), Separation::Lightlike);
    // light rays whose boosted interval rounds to a few 1e-13, and a separation 1e-6 off the light cone
    for beta in [0.7, 0.9, 0.99] {
        for (a, b) in [((0.37, 1.1), (-2.63, 4.1)), ((1.3, 0.7), (6.3, 5.7))] {
            let (a, b) = (boost(a.0, a.1, beta).unwrap(), boost(b.0, b.1, beta).unwrap());
            assert_eq!(classify(a, b), Separation::Lightlike, "{beta}: {}", interval_between(a, b));
        }
    }
    assert_eq!(classify((0.0, 0.0), (3.0, 3.0 + 1e-6)), Separation::Timelike);
    near(proper_time((0.0, 0.0), (3.0, 5.0)).unwrap(), 4.0);
    near(proper_length((0.0, 0.0), (5.0, 3.0)).unwrap(), 4.0);
    // the cone of (1, 2) reaching 3, boosted at 0.9: its edges are still at 45 degrees
    let cone = light_cone((1.0, 2.0), 3.0).unwrap();
    let future = cone.future.map(|(x, t)| boost(x, t, 0.9).unwrap());
    near2(cone_slopes(boost(1.0, 2.0, 0.9).unwrap(), future), (1.0, 1.0));
    assert_eq!(cone.past, [(-2.0, -1.0), (4.0, -1.0)]);
    assert!(inside_future_cone((0.0, 0.0), (0.5, 1.0)) && !inside_future_cone((0.0, 0.0), (1.0, 0.5)));
    assert!(!inside_future_cone((0.0, 0.0), (0.0, -1.0)));
    near2(worldline_inertial(1.0, 0.0, 0.5, 2.0).unwrap(), (2.0, 2.0));
    near2(worldline_accelerated(2.0, 1.0).unwrap(), (1.3811, 1.81343));
    near(accelerated_beta(2.0, 1.0).unwrap(), 0.964028);
    near2(worldline_parametric(|tau| 0.5 * tau, |tau| tau, 2.0).unwrap(), (1.0, 2.0));
    near2(grid_x_line(0.6, 2.0, 1.0).unwrap(), (2.2, 1.0));
    near2(grid_x_line(-0.6, 2.0, 1.0).unwrap(), (1.0, 1.0));
    near2(grid_t_line(0.6, 2.0, 1.0).unwrap(), (1.0, 2.2));
    let grid = boosted_grid(0.6, 2, 3.0).unwrap();
    assert_eq!((grid.x.len(), grid.t.len()), (5, 5));
    // every line x' = k of the grid is x' = k in the boosted frame, and every line t' = k is t' = k
    for (k, line) in (-2..=2).zip(&grid.x) {
        for &(x, t) in line {
            near(boost(x, t, 0.6).unwrap().0, f64::from(k));
        }
    }
    for (k, line) in (-2..=2).zip(&grid.t) {
        for &(x, t) in line {
            near(boost(x, t, 0.6).unwrap().1, f64::from(k));
        }
    }
}

#[test]
fn rule_1_refuses_speeds_of_light_and_spacelike_worldlines() {
    for beta in [1.0, -1.0, 1.5, f64::NAN] {
        assert!(gamma(beta).is_err() && boost(0.0, 0.0, beta).is_err() && rapidity(beta).is_err(), "{beta}");
        assert!(worldline_inertial(0.0, 0.0, beta, 1.0).is_err() && grid_x_line(beta, 1.0, 0.0).is_err());
    }
    assert!(compose_beta(0.5, 1.0).is_err());
    assert!(proper_time((0.0, 0.0), (2.0, 1.0)).is_err() && proper_length((0.0, 0.0), (1.0, 2.0)).is_err());
    assert!(light_cone((0.0, 0.0), 0.0).is_err() && light_cone((0.0, 0.0), f64::NAN).is_err());
    assert!(worldline_accelerated(0.0, 1.0).is_err() && accelerated_beta(-1.0, 1.0).is_err());
    // a tangent faster than light, and one that runs backwards in time
    assert!(worldline_parametric(|tau| 2.0 * tau, |tau| tau, 0.0).is_err());
    assert!(worldline_parametric(|_| 0.0, |tau| -tau, 0.0).is_err());
}

#[test]
fn rule_2_the_penrose_diamond() {
    near2(penrose(1.0, 0.0), (0.785398, 0.0));
    near2(penrose(1e6, 0.0), (1.5708, 0.0));
    near(penrose_slope((2.0, -1.0), 1.0).unwrap(), 1.0);
    near(penrose_slope((2.0, -1.0), -1.0).unwrap(), -1.0);
    assert!(penrose_slope((0.0, 0.0), 0.5).is_err());
    let back = penrose_inverse(penrose(0.7, -2.5).0, penrose(0.7, -2.5).1).unwrap();
    near2(back, (0.7, -2.5));
    assert!(penrose_inverse(1.0, 1.0).is_err());
    let h = std::f64::consts::FRAC_PI_2;
    assert_eq!(penrose_infinity(Infinity::Spacelike), (h, 0.0));
    assert_eq!(penrose_infinity(Infinity::Future), (0.0, h));
    assert_eq!(penrose_infinity(Infinity::Past), (0.0, -h));
    // every point lies inside the diamond
    for (x, t) in [(1e9, 3.0), (-4.0, 1e7), (2.0, -2.5), (0.0, 0.0)] {
        let (px, pt) = penrose(x, t);
        assert!(px.abs() + pt.abs() <= h + 1e-12, "({x}, {t})");
    }
}

#[test]
fn rule_2_kruskal_szekeres() {
    near2(kruskal(6.0, 0.0, 2.0).unwrap(), (0.0, 6.33807));
    near2(kruskal(6.0, 3.0, 2.0).unwrap(), (5.2119, 8.20579));
    let (t, x) = kruskal(6.0, 3.0, 2.0).unwrap();
    near(x * x - t * t, 40.1711);
    near2(kruskal(1.0, 0.0, 2.0).unwrap(), (0.907943, 0.0));
    near(kruskal_singularity(3.0), 3.16228);
    near(penrose(3.0, kruskal_singularity(3.0)).1, 0.785398);
    near(kruskal_radius(t, x, 2.0).unwrap(), 6.0);
    // inside the horizon too, and on it
    let (t, x) = kruskal(0.5, 1.0, 2.0).unwrap();
    near(kruskal_radius(t, x, 2.0).unwrap(), 0.5);
    assert_eq!(kruskal_radius(1.0, 1.0, 2.0).unwrap(), 2.0);
    near2(penrose_schwarzschild(6.0, 3.0, 2.0).unwrap(), penrose(8.20579, 5.2119));
    assert!(kruskal_radius(2.0, 0.0, 2.0).is_err(), "beyond the singularity");
    assert!(kruskal_radius(1.5f64.sqrt(), 0.0, 2.0).is_err(), "just beyond the singularity");
    let (t, x) = kruskal(1e-3, 0.0, 2.0).unwrap();
    near(kruskal_radius(t, x, 2.0).unwrap(), 1e-3);
    assert!(kruskal(0.0, 0.0, 2.0).is_err() && kruskal(1.0, 0.0, 0.0).is_err());
    assert_eq!(compactify(0.3, 0.4, Compact::None).unwrap(), (0.3, 0.4));
    assert_eq!(compactify(0.3, 0.4, Compact::Minkowski).unwrap(), penrose(0.3, 0.4));
    assert_eq!(
        compactify(6.0, 3.0, Compact::Schwarzschild(2.0)).unwrap(),
        penrose_schwarzschild(6.0, 3.0, 2.0).unwrap()
    );
    assert!(compactify(6.0, 3.0, Compact::Schwarzschild(0.0)).is_err());
}

#[test]
fn rule_3_flamm_s_paraboloid_and_the_potential_sheet() {
    near(flamm(4.0, 2.0).unwrap(), 4.0);
    near(flamm_slope(4.0, 2.0).unwrap(), 1.0);
    assert_eq!(flamm_slope(2.0, 2.0).unwrap(), f64::INFINITY);
    near(proper_radial_distance(2.0, 4.0, 2.0).unwrap(), 4.59117);
    near(flamm_arc_length(2.0, 4.0, 2.0).unwrap(), 4.59117);
    near(proper_radial_distance(3.0, 10.0, 2.0).unwrap(), 8.78253);
    // the sheet's reason to exist: radial arc length on it is proper radial distance
    for (a, b) in [(2.0, 10.0), (3.0, 3.5), (7.0, 70.0)] {
        let (s, d) = (flamm_arc_length(a, b, 2.0).unwrap(), proper_radial_distance(a, b, 2.0).unwrap());
        assert!((s - d).abs() <= 1e-9 * d, "{a}..{b}: {s} against {d}");
    }
    near(flamm_curvature(4.0, 2.0).unwrap(), -0.015625);
    near(potential(2.0, 4.0).unwrap(), -2.0);
    near(potential_slope(2.0, 4.0).unwrap(), 1.0);
    let p = embedding_point(Embedding::Flamm(2.0), 4.0, 0.0).unwrap();
    near2((p[0], p[1]), (4.0, -4.0));
    near(p[2], 0.0);
    // the potential sheet's height is -k / r, so it sags (scene y is down) by k / r
    let q = embedding_point(Embedding::Potential(4.0), 2.0, std::f64::consts::FRAC_PI_2).unwrap();
    near2((q[1], q[2]), (2.0, 2.0));
    assert!(flamm(1.0, 2.0).is_err() && flamm(4.0, 0.0).is_err() && potential(0.0, 1.0).is_err());
    assert!(proper_radial_distance(4.0, 3.0, 2.0).is_err() && flamm_arc_length(4.0, 3.0, 2.0).is_err());
}

#[test]
fn rule_3_the_embedding_grid() {
    let g = embedding_grid(Embedding::Flamm(2.0), 2.0, 10.0, 5, 8, 33).unwrap();
    assert_eq!((g.rings.len(), g.spokes.len()), (5, 8));
    assert!(g.rings.iter().all(|r| r.len() == 33) && g.spokes.iter().all(|s| s.len() == 33));
    // rings at rMin .. rMax, each at its height; a spoke from rMin to rMax at its angle
    for (i, ring) in g.rings.iter().enumerate() {
        let r = 2.0 + 2.0 * i as f64;
        for p in ring {
            near(p[0].hypot(p[2]), r);
            near(p[1], -flamm(r, 2.0).unwrap());
        }
    }
    let spoke = &g.spokes[2];
    near2((spoke[0][0], spoke[0][2]), (0.0, 2.0));
    near2((spoke[32][0], spoke[32][2]), (0.0, 10.0));
    // the spoke's polyline is the radial distance (from below, as a chord is)
    let length: f64 = spoke.windows(2).map(|w| (0..3).map(|a| (w[1][a] - w[0][a]).powi(2)).sum::<f64>().sqrt()).sum();
    let d = proper_radial_distance(2.0, 10.0, 2.0).unwrap();
    assert!(length < d && length > d * 0.99, "{length} against {d}");
    // a sheet from beyond the horizon keeps its own rs, and the potential sheet its own k
    let far = embedding_grid(Embedding::Flamm(2.0), 4.0, 10.0, 1, 3, 4).unwrap();
    near(far.rings[0][0][1], -4.0);
    let pot = embedding_grid(Embedding::Potential(4.0), 0.5, 10.0, 2, 3, 2).unwrap();
    near(pot.rings[0][0][1], 8.0);
    assert!(embedding_grid(Embedding::Flamm(2.0), 1.0, 10.0, 2, 3, 2).is_err(), "inside the horizon");
    assert!(embedding_grid(Embedding::Flamm(2.0), 10.0, 4.0, 2, 3, 2).is_err());
    assert!(embedding_grid(Embedding::Potential(4.0), 0.5, 10.0, 0, 3, 2).is_err());
    assert!(embedding_grid(Embedding::Potential(4.0), 0.5, 10.0, 1, 2, 2).is_err());
    let one = embedding_grid(Embedding::Potential(4.0), 0.5, 10.0, 1, 3, 1).unwrap_err();
    assert!(one.0.contains("samples >= 2"), "{one}");
}

#[test]
fn rule_4_strain_and_the_chirp() {
    near2(strain_displacement(1.0, 0.0, 0.2, 0.0), (0.1, 0.0));
    near2(ring_axes(0.2, 0.0), (1.1, 0.9));
    let d = std::f64::consts::FRAC_1_SQRT_2;
    let p = strain_point(d, d, 0.0, 0.2, 1.0, 0.0, 0.0);
    near(p.0.hypot(p.1), 1.1);
    // the plus ellipse along the axes at the crest, a circle a quarter period on, the other ellipse half a period on
    let ring = |t: f64| strained_ring(0.2, 0.0, 4, 1.0, 1.0, t, 0.0);
    let r0 = ring(0.0);
    near2(r0[0], (1.1, 0.0));
    near2(r0[1], (0.0, 0.9));
    for p in ring(0.25) {
        near(p.0.hypot(p.1), 1.0);
    }
    near2(ring(0.5)[0], (0.9, 0.0));
    near(chirp_mass(36.0, 29.0).unwrap(), 28.0956);
    near(gw_frequency(0.2, 28.0 * M_SUN).unwrap(), 34.4138);
    near(time_to_merger(150.0, 28.0 * M_SUN).unwrap(), 0.00394525);
    near(time_to_merger(gw_frequency(0.2, 28.0 * M_SUN).unwrap(), 28.0 * M_SUN).unwrap(), 0.2);
    near(gw_amplitude(410.0 * MPC_M, 28.0 * M_SUN, 150.0).unwrap(), 2.1131e-21);
    near(chirp_cycles(0.2, 0.05, 28.0 * M_SUN).unwrap(), 6.38227);
    assert!(chirp_mass(0.0, 1.0).is_err() && gw_frequency(0.0, M_SUN).is_err() && gw_frequency(1.0, 0.0).is_err());
    assert!(time_to_merger(0.0, M_SUN).is_err() && gw_amplitude(0.0, M_SUN, 1.0).is_err());
    assert!(chirp_cycles(0.05, 0.2, M_SUN).is_err() && chirp_cycles(0.2, 0.0, M_SUN).is_err());
}

#[test]
fn rule_5_rapidity_keys_and_the_seek_check() {
    near(frame_beta(120, 24.0, 0.0, 0.8, 0.0, 10.0).unwrap(), 0.5);
    assert_eq!(frame_beta(0, 24.0, 0.0, 0.8, 0.0, 10.0).unwrap(), 0.0);
    near(frame_beta(240, 24.0, 0.0, 0.8, 0.0, 10.0).unwrap(), 0.8);
    near(frame_beta(480, 24.0, 0.0, 0.8, 0.0, 10.0).unwrap(), 0.8);
    assert!(frame_beta(1, 0.0, 0.0, 0.8, 0.0, 10.0).is_err() && frame_beta(1, 24.0, 0.0, 0.8, 1.0, 1.0).is_err());
    assert!(frame_beta(1, 24.0, 0.0, 1.0, 0.0, 10.0).is_err());
    assert!(seek_test());
}

#[test]
fn rule_1_and_2_the_diagram_pixel() {
    near2(diagram_pixel(1.0, 0.0, &BOX, 0.6, Compact::None).unwrap(), (425.0, 275.0));
    near2(diagram_pixel(0.0, 0.0, &BOX, 0.0, Compact::None).unwrap(), (300.0, 200.0));
    // the diamond's spacelike infinity sits pi/2 to the right of the centre
    let (px, _) = diagram_pixel(1e12, 0.0, &BOX, 0.0, Compact::Minkowski).unwrap();
    near(px, 300.0 + std::f64::consts::FRAC_PI_2 * 100.0);
    let flat = AxesBox { x_max: -2.0, ..BOX };
    assert!(diagram_pixel(0.0, 0.0, &flat, 0.0, Compact::None).is_err());
}

#[test]
fn rule_6_validation() {
    assert!(validate_spacetime(1.0, 0.5, Compact::None).is_ok());
    assert!(validate_spacetime(1.0, 0.5, Compact::Schwarzschild(2.0)).is_ok());
    for (c, beta, compact) in
        [(0.0, 0.5, Compact::None), (1.0, 1.0, Compact::None), (1.0, 0.0, Compact::Schwarzschild(0.0))]
    {
        assert!(validate_spacetime(c, beta, compact).unwrap_err().0.starts_with("RD1"), "{c} {beta} {compact:?}");
    }
    assert!(validate_worldline(WorldlineKind::Inertial, Some(0.5), None).is_ok());
    assert!(validate_worldline(WorldlineKind::Parametric, None, None).is_ok());
    for (kind, beta, a) in [
        (WorldlineKind::Inertial, None, None),
        (WorldlineKind::Inertial, Some(1.0), None),
        (WorldlineKind::Accelerated, None, None),
        (WorldlineKind::Accelerated, None, Some(0.0)),
    ] {
        assert!(validate_worldline(kind, beta, a).unwrap_err().0.starts_with("RD2"), "{kind:?}");
    }
    assert!(validate_embedding(2.0, 10.0, 12, 24).is_ok());
    for (lo, hi, rings, spokes) in [(0.0, 1.0, 1, 3), (2.0, 2.0, 1, 3), (1.0, 2.0, 0, 3), (1.0, 2.0, 1, 2)] {
        assert!(validate_embedding(lo, hi, rings, spokes).unwrap_err().0.starts_with("RD3"));
    }
    assert!(validate_strain(0.2, 0.0, Some(1.0), None, None).is_ok());
    assert!(validate_strain(0.2, 0.0, None, Some(28.0), Some(1.0)).is_ok());
    for (hp, hc, f, mc, merger) in [
        (1.5, 0.0, Some(1.0), None, None),
        (0.0, -2.0, Some(1.0), None, None),
        (0.2, 0.0, None, None, None),
        (0.2, 0.0, Some(1.0), Some(28.0), Some(1.0)),
        (0.2, 0.0, None, Some(28.0), None),
        (0.2, 0.0, Some(0.0), None, None),
        (0.2, 0.0, None, Some(-1.0), Some(1.0)),
    ] {
        assert!(validate_strain(hp, hc, f, mc, merger).unwrap_err().0.starts_with("RD4"), "{hp} {hc} {f:?} {mc:?}");
    }
}

#[test]
fn the_diagram_grid_is_the_box_frame_where_the_boost_carries_it() {
    // the box frame's line x = k, boosted, lies on the drawn frame's line of diagram_grid, and t = k likewise
    for beta in [-0.9, -0.3, 0.0, 0.6, 0.95] {
        let g = diagram_grid(beta, 2, 3.0).unwrap();
        assert_eq!(g, boosted_grid(-beta, 2, 3.0).unwrap());
        for k in [-2.0, 1.0, 2.0] {
            for t in [-1.0, 0.25, 2.0] {
                let (x, tb) = boost(k, t, beta).unwrap();
                near2(grid_x_line(-beta, k, tb).unwrap(), (x, tb));
                let (xb, tt) = boost(t, k, beta).unwrap();
                near2(grid_t_line(-beta, k, xb).unwrap(), (xb, tt));
            }
        }
    }
}
