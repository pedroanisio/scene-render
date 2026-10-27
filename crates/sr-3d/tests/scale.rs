//! The scale draft's numeric table (sr-core `srep/srep-0000-scale.md`, rules 1 to 5) against `sr_3d::scale`, row by
//! row, the seek check of rule 2, and the values its rules refuse. The expected values are the table's, which
//! `conformance/tools/scale_reference.py` prints to six significant digits.

use sr_3d::scale::*;

const DEPTH: (f64, f64) = (0.1, 1e19);
const R: DepthMode = DepthMode::Reversed;
const L: DepthMode = DepthMode::Logarithmic;
const FLOAT: Buffer = Buffer::Float;
const FIXED: Buffer = Buffer::Fixed;
/// The powers of ten camera of the table: from 10 to 1e20 pixels between 2 s and 40 s.
const POWERS: (f64, f64, f64, f64) = (2.0, 40.0, 10.0, 1e20);

/// Six significant digits, as the table prints them.
#[track_caller]
fn near(got: f64, want: f64) {
    let tol = 5e-6 * want.abs();
    assert!((got - want).abs() <= tol, "{got} against {want}");
}

fn focal() -> f64 {
    focal_pixels(60.0, 1920.0).unwrap()
}

fn bar_at(depth: f64, units: Units) -> ScaleBar {
    scale_bar_at(depth, focal(), 100.0, 200, units).unwrap()
}

#[test]
fn binary32_spacing() {
    assert_eq!(f32_ulp(1.0), 1.1920928955078125e-07);
    assert_eq!(f32_ulp(1e20), 8796093022208.0);
    near(f32_ulp(1e-20), 8.07794e-28);
    // at a power of two the spacing is that of the binade above; at zero, the smallest subnormal
    assert_eq!((f32_ulp(2.0), f32_ulp(-2.0), f32_ulp(0.0)), (2.0f64.powi(-22), 2.0f64.powi(-22), 2.0f64.powi(-149)));
    assert_eq!(f32_ulp(1e-45), 2.0f64.powi(-149));
    // on either side of every binade's edge
    for e in -120..120 {
        let p = 2f64.powi(e);
        assert_eq!(f32_ulp(p), 2f64.powi((e - 23).max(-149)), "2^{e}");
        assert_eq!(f32_ulp(p.next_down()), 2f64.powi((e - 24).max(-149)), "below 2^{e}");
    }
}

#[test]
fn rule_1_depth_mappings() {
    let (n, f) = DEPTH;
    assert_eq!(depth(0.1, n, f, R).unwrap(), 1.0);
    near(depth(0.2, n, f, R).unwrap(), 0.5);
    near(depth(1e10, n, f, R).unwrap(), 1e-11);
    assert_eq!(depth(f, n, f, R).unwrap(), 0.0);
    // where far is not much beyond near
    near(depth(1.5, 1.0, 2.0, R).unwrap(), 1.0 / 3.0);
    near(depth(1e9, n, f, L).unwrap(), 0.5);
    near(depth(1e15, n, f, L).unwrap(), 0.8);
    assert_eq!((depth(n, n, f, L).unwrap(), depth(f, n, f, L).unwrap()), (0.0, 1.0));
    near(distance(1e-11, n, f, R).unwrap(), 9999999990.0);
    near(distance(0.8, n, f, L).unwrap(), 1e15);
    near(depth_slope(1e10, n, f, R).unwrap(), 1e-21);
    near(depth_slope(1e10, n, f, L).unwrap(), 2.17147e-12);
    // distance inverts depth under both mappings
    for mode in [R, L] {
        for z in [0.3, 7.0, 1e5, 3e12, 1e18] {
            let back = distance(depth(z, n, f, mode).unwrap(), n, f, mode).unwrap();
            assert!((back - z).abs() <= 1e-9 * z, "{mode:?} {z} {back}");
        }
    }
}

#[test]
fn rule_1_depth_resolution() {
    let (n, f) = DEPTH;
    near(resolvable_gap(1e10, n, f, R, FLOAT).unwrap(), 867.362);
    near(relative_resolution(1e10, n, f, R, FLOAT).unwrap(), 8.67362e-08);
    near(relative_resolution(1e10, n, f, R, FIXED).unwrap(), 5960.46);
    near(relative_resolution(1e10, n, f, L, FIXED).unwrap(), 2.7449e-06);
    near(relative_resolution(1e10, n, f, L, FLOAT).unwrap(), 2.7449e-06);
    near(worst_resolution(n, f, R, FLOAT).unwrap(), 1.19209e-07);
    near(worst_resolution(n, f, L, FLOAT).unwrap(), 2.7449e-06);
    near(worst_resolution(n, f, R, FIXED).unwrap(), 5.96046e+12);
    assert!(resolves(n, f, R, FLOAT).unwrap());
    assert!(!resolves(n, f, R, FIXED).unwrap());
    assert!(resolves(n, f, L, FIXED).unwrap());
    // a fixed buffer under the reversed mapping resolves while 2^-24 far / near is at most 1e-5 (to far = 168 near)
    assert!(resolves(1.0, 100.0, R, FIXED).unwrap());
    assert!(!resolves(1.0, 1000.0, R, FIXED).unwrap());
    assert_eq!(depth_ulp(0.25, FIXED), 2.0f64.powi(-24));
    assert_eq!(depth_ulp(0.25, FLOAT), 2.0f64.powi(-25));
}

#[test]
fn rule_1_and_5_the_depth_ratio_and_the_modes() {
    assert!(depth_ratio_valid(0.1, 1e19));
    assert!(!depth_ratio_valid(0.1, 2e19));
    assert!(!depth_ratio_valid(0.0, 1.0) && !depth_ratio_valid(-1.0, 1.0) && !depth_ratio_valid(2.0, 1.0));
    assert!(!depth_ratio_valid(f64::NAN, 1.0) && !depth_ratio_valid(1.0, f64::NAN));
    assert!(validate_camera(0.1, 1e19).is_ok());
    assert!(err(validate_camera(0.1, 2e19)).contains("SC1"));
    assert_eq!(DepthMode::parse("reversed").unwrap(), R);
    assert_eq!(DepthMode::parse("logarithmic").unwrap(), L);
    assert!(err(DepthMode::parse("linear")).contains("depth mode"));
}

#[test]
fn rule_1_refuses_points_and_planes_outside_the_domain() {
    let (n, f) = DEPTH;
    for (z, n, f) in [(0.05, n, f), (2e19, n, f), (f64::NAN, n, f), (1.0, 0.0, f), (1.0, -1.0, f), (1.0, 2.0, 1.0)] {
        for mode in [R, L] {
            assert!(depth(z, n, f, mode).is_err(), "{z} {n} {f} {mode:?}");
            assert!(depth_slope(z, n, f, mode).is_err(), "{z} {n} {f} {mode:?}");
            assert!(resolvable_gap(z, n, f, mode, FLOAT).is_err(), "{z} {n} {f} {mode:?}");
        }
    }
    for d in [-0.1, 1.1, f64::NAN] {
        assert!(distance(d, n, f, R).is_err() && distance(d, n, f, L).is_err(), "{d}");
    }
    assert!(worst_resolution(1.0, 1.0, R, FLOAT).is_err());
}

#[test]
fn rule_2_geometric_keys() {
    let g = Interpolation::Geometric;
    near(geometric(10.0, 1e20, 0.5).unwrap(), 3.16228e+10);
    near(geometric(10.0, 1e20, 4.0 / 19.0).unwrap(), 100000.0);
    assert_eq!((geometric(10.0, 1e20, 0.0).unwrap(), geometric(10.0, 1e20, 1.0).unwrap()), (10.0, 1e20));
    // two negative values: the magnitude moves in the log domain
    near(geometric(-1.0, -100.0, 0.5).unwrap(), -10.0);
    near(interpolate(10.0, 1e20, 0.5, Interpolation::Linear).unwrap(), 5e+19);
    near(interpolate(10.0, 1e20, 0.5, g).unwrap(), 3.16228e+10);
    // a linear pair may cross zero
    assert_eq!(interpolate(-1.0, 1.0, 0.5, Interpolation::Linear).unwrap(), 0.0);
    near(decades(10.0, 1e20).unwrap(), 19.0);
    assert_eq!((decade(999.9999).unwrap(), decade(1000.0).unwrap(), decade(0.01).unwrap()), (2, 3, -2));
    // on either side of every decade's edge
    for k in -300..300 {
        let p: f64 = format!("1e{k}").parse().unwrap();
        if 10f64.powf(k as f64) != p {
            // the literal falls below pow(10, k) (1e23 and 1e210), so the decade below, as in the reference
            assert!(10f64.powf(k as f64) > p && decade(p).unwrap() == k - 1, "1e{k}");
            continue;
        }
        assert_eq!(decade(p).unwrap(), k, "1e{k}");
        assert_eq!(decade(p.next_down()).unwrap(), k - 1, "below 1e{k}");
        assert_eq!(decade(p.next_up()).unwrap(), k, "above 1e{k}");
    }
    assert_eq!(Interpolation::parse("geometric").unwrap(), g);
    assert!(err(Interpolation::parse("cubic")).contains("interpolation"));
}

#[test]
fn rule_2_and_5_sc2_refuses_zero_and_mixed_signs() {
    let g = Interpolation::Geometric;
    for (v0, v1) in [(0.0, 1.0), (1.0, 0.0), (-1.0, 10.0), (1.0, -10.0)] {
        assert!(err(validate_keys(v0, v1, g)).contains("SC2"), "{v0} {v1}");
        assert!(geometric(v0, v1, 0.5).is_err() && interpolate(v0, v1, 0.5, g).is_err(), "{v0} {v1}");
    }
    assert!(validate_keys(-1.0, 10.0, Interpolation::Linear).is_ok());
    for u in [-0.01, 1.01, f64::NAN] {
        assert!(geometric(1.0, 10.0, u).is_err(), "{u}");
    }
    assert!(decade(0.0).is_err() && decade(-1.0).is_err() && decades(0.0, 1.0).is_err() && decades(1.0, -1.0).is_err());
}

#[test]
fn rule_2_the_powers_of_ten_camera() {
    let (t0, t1, d0, d1) = POWERS;
    near(powers_of_ten(10.0, t0, t1, d0, d1).unwrap(), 100000.0);
    // held outside the key interval
    assert_eq!((powers_of_ten(0.0, t0, t1, d0, d1).unwrap(), powers_of_ten(50.0, t0, t1, d0, d1).unwrap()), (d0, d1));
    near(time_at(1e15, t0, t1, d0, d1).unwrap(), 30.0);
    // within 1e-12 beyond d1 the time is t1
    assert_eq!(time_at(1e20 * (1.0 + 1e-13), t0, t1, d0, d1).unwrap(), 40.0);
    near(frame_distance(96, 24.0, t0, t1, d0, d1).unwrap(), 100.0);
    // one decade every 2 s
    for k in 1..=20 {
        let d = 10f64.powi(k);
        near(time_at(d, t0, t1, d0, d1).unwrap(), t0 + 2.0 * (k - 1) as f64);
    }
    assert!(powers_of_ten(1.0, 2.0, 2.0, d0, d1).is_err() && powers_of_ten(f64::NAN, t0, t1, d0, d1).is_err());
    assert!(time_at(1.0, t0, t1, d0, d1).is_err() && time_at(1e21, t0, t1, d0, d1).is_err());
    assert!(time_at(1e5, 2.0, 1.0, d0, d1).is_err() && time_at(1e5, t0, t1, -1.0, d1).is_err());
    assert!(frame_distance(1, 0.0, t0, t1, d0, d1).is_err());
}

#[test]
fn rule_2_the_seek_check() {
    // 960 frames played as a running geometric product agree with each frame evaluated alone
    let (t0, t1, d0, d1) = POWERS;
    let fps = 24.0;
    let hold = (t0 * fps).round() as u64;
    let ratio = (d1 / d0).powf(1.0 / ((t1 - t0) * fps));
    let mut played = d0;
    for frame in 0..=960u64 {
        if frame > hold {
            played = (played * ratio).min(d1);
        }
        let direct = frame_distance(frame, fps, t0, t1, d0, d1).unwrap();
        assert!((direct - played).abs() <= 1e-9 * direct, "frame {frame}: {direct} played {played}");
        assert_eq!(bar_at(direct, Units::Auto).label, bar_at(played, Units::Auto).label, "frame {frame}");
    }
}

#[test]
fn rule_3_pixels_per_metre() {
    near(focal(), 1662.77);
    near(pixels_per_metre(1000.0, focal(), 100.0).unwrap(), 166.277);
    near(pixels_per_metre_ortho(2000.0, 1080.0, 100.0).unwrap(), 54.0);
    for fov in [0.0, 180.0, -1.0, f64::NAN] {
        assert!(focal_pixels(fov, 1920.0).is_err(), "{fov}");
    }
    assert!(focal_pixels(60.0, 0.0).is_err());
    for (d, fp, ppm) in [(0.0, 1.0, 1.0), (1.0, 0.0, 1.0), (1.0, 1.0, 0.0), (f64::NAN, 1.0, 1.0)] {
        assert!(pixels_per_metre(d, fp, ppm).is_err(), "{d} {fp} {ppm}");
        assert!(pixels_per_metre_ortho(d, fp, ppm).is_err(), "{d} {fp} {ppm}");
    }
}

#[test]
fn rule_3_the_scale_bar() {
    assert_eq!(nice_length(2.1383).unwrap(), 2.0);
    assert_eq!(nice_length(0.9).unwrap(), 0.5);
    assert_eq!((nice_length(5.0).unwrap(), nice_length(9.99).unwrap(), nice_length(10.0).unwrap()), (5.0, 5.0, 10.0));
    assert!(nice_length(0.0).is_err() && nice_length(f64::NAN).is_err());
    let b = bar_at(1000.0, Units::Auto);
    assert_eq!(b.label, "1 m");
    near(b.pixels, 166.277);
    assert_eq!((b.unit, b.value, b.metres), (Unit::M, 1.0, 1.0));
    assert_eq!(bar_at(1e10, Units::Auto).label, "10 Mm");
    assert_eq!(bar_at(1e15, Units::Auto).label, "5 au");
    assert_eq!(bar_at(1e18, Units::Auto).label, "5000 au");
    assert_eq!(bar_at(1e20, Units::Auto).label, "2 pc");
    assert_eq!(bar_at(1e20, Units::Si).label, "100000000 Gm");
    let (t0, t1, d0, d1) = POWERS;
    let frame = |f| scale_bar_frame(f, 24.0, t0, t1, d0, d1, focal(), 100.0, 200, Units::Auto).unwrap().label;
    // the scale-bar kit case's frames: distances of 10, 100, 1e10 and 1e20 px
    assert_eq!([frame(0), frame(96), frame(480), frame(960)], ["10 mm", "100 mm", "10 Mm", "2 pc"].map(String::from));
    // the bar is between 40 % and 100 % of maxWidth at every depth
    let mut depth = 0.5;
    while depth < 1e21 {
        let b = bar_at(depth, Units::Auto);
        assert!(b.pixels <= 200.0 * (1.0 + 1e-12) && b.pixels >= 80.0 * (1.0 - 1e-12), "{depth}: {b:?}");
        depth *= 1.37;
    }
}

#[test]
fn rule_3_the_unit_ladder_and_labels() {
    assert_eq!(bar_unit(1e11, Units::Auto), Unit::M);
    assert_eq!(bar_unit(AU_M, Units::Auto), Unit::Au);
    assert_eq!(bar_unit(LY_M, Units::Auto), Unit::Ly);
    assert_eq!(bar_unit(1e30, Units::Auto), Unit::Pc);
    assert_eq!(bar_unit(1e30, Units::Si), Unit::M);
    near(PC_M, 3.08568e16);
    assert_eq!([Unit::M, Unit::Au, Unit::Ly, Unit::Pc].map(Unit::metres), [1.0, AU_M, LY_M, PC_M]);
    assert_eq!(scale_label(2e-3, Unit::M).unwrap(), "2 mm");
    assert_eq!(scale_label(5e-6, Unit::M).unwrap(), "5 µm");
    assert_eq!(scale_label(1e-12, Unit::M).unwrap(), "1 pm");
    // below the picometre the value is a fraction of the smallest prefix
    assert_eq!(scale_label(5e-13, Unit::M).unwrap(), "0.5 pm");
    assert_eq!(scale_label(2e3, Unit::Pc).unwrap(), "2 kpc");
    assert_eq!(scale_label(5e9, Unit::Pc).unwrap(), "5 Gpc");
    assert_eq!(scale_label(0.5, Unit::Pc).unwrap(), "0.5 pc");
    assert_eq!(scale_label(20000.0, Unit::Au).unwrap(), "20000 au");
    assert_eq!(scale_label(1e15, Unit::Ly).unwrap(), "1e+15 ly");
    assert_eq!(scale_label(2.5e-7, Unit::Ly).unwrap(), "2.5e-07 ly");
    assert_eq!(scale_label(2.5e-5, Unit::Ly).unwrap(), "2.5e-05 ly");
    assert_eq!(scale_label(2.5e-4, Unit::Ly).unwrap(), "0.00025 ly");
    assert_eq!(scale_label(123456.5, Unit::Au).unwrap(), "123456 au");
    assert!(scale_label(0.0, Unit::M).is_err());
    assert_eq!(Units::parse("si").unwrap(), Units::Si);
    assert!(err(Units::parse("imperial")).contains("units"));
    assert_eq!([Unit::M, Unit::Au, Unit::Ly, Unit::Pc].map(Unit::name), ["m", "au", "ly", "pc"]);
}

#[test]
fn rule_3_refuses_a_bar_below_a_picometre() {
    assert!(err(scale_bar(1e15, 200, Units::Auto)).contains("picometre"));
    assert!(scale_bar(1e14, 200, Units::Auto).is_ok());
    assert!(scale_bar(0.0, 200, Units::Auto).is_err() && scale_bar(1.0, 0, Units::Auto).is_err());
}

#[test]
fn rule_4_the_floating_origin() {
    let p17 = 2f64.powi(17);
    assert_eq!(world_error(1e20 + p17, 1e20).unwrap(), 131072.0);
    assert_eq!(rebased_error(1e20 + p17, 1e20).unwrap(), 0.0);
    near(world_error(1e8 + 0.1, 1e8).unwrap(), 0.1);
    assert_eq!(rebased_error(1e8 + 0.1, 1e8).unwrap(), 0.0);
    assert!(rebase_needed(1e20 + p17, 1e20).unwrap());
    assert!(!rebase_needed(1e6 + 0.25, 1e6).unwrap());
    assert!(!rebase_needed(1e20, 1e20).unwrap());
    // a world error, but under 1e-5 of the gap
    near(world_error(1000.1, 0.0).unwrap(), 2.44141e-05);
    assert!(!rebase_needed(1000.1, 0.0).unwrap());
    near(rebased_error(1.0 / 3.0, 0.0).unwrap(), 9.93411e-09);
    assert_eq!(world_extent(1.0).unwrap(), 16777216.0);
    near(world_extent(1e13).unwrap(), 1.47574e+20);
    assert!(world_extent(0.0).is_err());
    // two spheres 2^17 apart at 1e20 with the eye between them: rebased they stand at ±2^16, in world binary32
    // they collapse onto the eye
    let eye = [1e20, 0.0, 0.0];
    let ps = [[1e20 - p17 / 2.0, 0.0, 0.0], [1e20 + p17 / 2.0, 0.0, 0.0]];
    assert_eq!(floating_origin(&ps, eye).unwrap(), vec![[-65536.0f32, 0.0, 0.0], [65536.0, 0.0, 0.0]]);
    assert_eq!(world_positions(&ps, eye).unwrap(), vec![[0.0f32, 0.0, 0.0], [0.0, 0.0, 0.0]]);
    // binary32 overflows beyond 3.4e38
    assert!(err(world_error(1e39, 0.0)).contains("binary32"));
    assert!(floating_origin(&[[1e39, 0.0, 0.0]], [0.0; 3]).is_err());
}

#[test]
fn rule_5_sc3_and_sc4_the_scale_bar() {
    assert!(validate_scale_bar(200, None, 0.1, 1e19, 1920).is_ok());
    assert!(validate_scale_bar(1920, Some(1e19), 0.1, 1e19, 1920).is_ok());
    assert!(err(validate_scale_bar(0, None, 0.1, 1e19, 1920)).contains("SC3"));
    assert!(err(validate_scale_bar(1921, None, 0.1, 1e19, 1920)).contains("SC3"));
    for d in [0.05, 2e19, f64::NAN] {
        assert!(err(validate_scale_bar(200, Some(d), 0.1, 1e19, 1920)).contains("SC4"), "{d}");
    }
}

fn err<T: std::fmt::Debug>(r: Result<T, ScaleError>) -> String {
    r.unwrap_err().0
}
