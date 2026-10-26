//! The planets draft's numeric table (sr-core `srep/srep-0000-planets.md`, rules 1 to 7) against `sr_3d::planet`,
//! and the values its rules refuse. The expected values are the table's, which `conformance/tools/planets_reference.py`
//! prints to six significant digits.

use sr_3d::planet::*;

const EARTH: f64 = 6378.1366;
const SATURN: f64 = 60268.0;
const STRIP: [[f64; 4]; 4] = [[0.6, 0.6, 0.5, 0.2], [0.8, 0.75, 0.6, 0.9], [0.0, 0.0, 0.0, 0.0], [0.9, 0.85, 0.7, 0.6]];

fn eye(radii: f64) -> [f64; 3] {
    [0.0, -radii * EARTH, 0.0]
}

fn sun45() -> [f64; 3] {
    [0.5f64.sqrt(), -(0.5f64.sqrt()), 0.0]
}

fn saturn_sun() -> [f64; 3] {
    let b = 26.7f64.to_radians();
    [b.cos(), 0.0, b.sin()]
}

/// Six significant digits, as the table prints them.
#[track_caller]
fn near(got: f64, want: f64) {
    let tol = 5e-6 * want.abs().max(1e-6);
    assert!((got - want).abs() <= tol, "{got} against {want}");
}

#[track_caller]
fn near3(got: [f64; 3], want: [f64; 3]) {
    for c in 0..3 {
        assert!((got[c] - want[c]).abs() <= 1e-9 + 5e-6 * want[c].abs(), "{got:?} against {want:?}");
    }
}

#[test]
fn rule_1_the_bodies_radii_and_scale() {
    assert_eq!(radii(399).unwrap(), (6378.1366, 6356.7519));
    assert_eq!(radii(699).unwrap(), (60268.0, 54364.0));
    assert_eq!(radii(10).unwrap().0, 696000.0);
    for body in [10, 199, 299, 399, 499, 599, 699, 799, 899, 301] {
        assert!(radii(body).is_ok(), "{body}");
    }
    assert!(radii(401).is_err());
    near(flattening(399).unwrap(), 0.00335281);
    near(flattening(699).unwrap(), 0.0979624);
    near(scene_radius(399, 1e-6).unwrap(), 6.37814);
    near(scene_radius(10, 1e-9).unwrap(), 0.696);
    assert!(scene_radius(399, 0.0).is_err());
    assert!(scene_radius(399, f64::NAN).is_err());
}

#[test]
fn rule_1_the_spheroids_points_and_normals() {
    let f = flattening(399).unwrap();
    near3(surface_point(EARTH, f, 0.0, 90.0).unwrap().map(|v| (v * 1e4).round() / 1e4), [0.0, 0.0, 6356.7519]);
    near3(surface_point(EARTH, f, 0.0, 0.0).unwrap(), [EARTH, 0.0, 0.0]);
    // the geodetic normal is the latitude's direction, within 0.2° of the geocentric one on the Earth
    let p = surface_point(EARTH, f, 30.0, 45.0).unwrap();
    near3(surface_normal(EARTH, f, p), body_direction(30.0, 45.0).unwrap());
    let geocentric = angle(p, surface_normal(EARTH, f, p)).to_degrees();
    assert!(geocentric > 0.1 && geocentric < 0.2, "{geocentric}");
    assert!(surface_point(EARTH, 1.0, 0.0, 0.0).is_err());
    assert!(surface_point(0.0, 0.0, 0.0, 0.0).is_err());
    assert!(surface_point(EARTH, f, 0.0, 91.0).is_err());
}

#[test]
fn rule_2_textures_and_the_engines_sphere() {
    assert_eq!(texture_uv(0.0, 0.0).unwrap(), [0.5, 0.5]);
    assert_eq!(texture_uv(-180.0, 90.0).unwrap(), [0.0, 0.0]);
    assert_eq!(texture_uv(90.0, -90.0).unwrap(), [0.75, 1.0]);
    assert_eq!(texture_uv(180.0, 0.0).unwrap(), [0.0, 0.5]);
    assert_eq!(engine_uv(90.0, 0.0).unwrap(), [0.25, 0.5]);
    assert!(texture_uv(f64::INFINITY, 0.0).is_err() && engine_uv(0.0, -90.5).is_err());
    near3(engine_normal(0.25, 0.5).map(|v| (v * 1e9).round() / 1e9 + 0.0), [0.0, 0.0, 1.0]);
    near3(scene_from_body(body_direction(90.0, 0.0).unwrap()).map(|v| (v * 1e9).round() / 1e9 + 0.0), [0.0, 0.0, 1.0]);
    // the engine's normal at a longitude's engine uv is that longitude's direction in the scene's axes
    for (lon, lat) in [(0.0, 0.0), (90.0, 0.0), (-120.0, 35.0), (10.0, -60.0)] {
        let [u, w] = engine_uv(lon, lat).unwrap();
        near3(engine_normal(u, w), scene_from_body(body_direction(lon, lat).unwrap()));
    }
}

#[test]
fn rule_7_the_limb_and_the_terminator() {
    let l = limb(eye(4.0), EARTH).unwrap();
    near(l.angular_radius, 14.4775);
    near(l.radius, 6175.6);
    near3(l.centre, [0.0, -EARTH / 4.0, 0.0]);
    assert!(limb([0.0, -EARTH, 0.0], EARTH).is_err());
    near(terminator_half_angle(EARTH, 149597870.7).unwrap(), 89.9976);
    assert!(terminator_half_angle(EARTH, EARTH).is_err());
    near(phase_angle(eye(5.0), sun45()), 45.0);
    near(illuminated_fraction(60.0).unwrap(), 0.75);
    assert!(illuminated_fraction(180.5).is_err());
    let (a, b) = cusps(eye(5.0), sun45(), EARTH).unwrap().unwrap();
    near(a[2] / EARTH, 0.959166);
    near(b[2] / EARTH, -0.959166);
    // each cusp is on the sphere, on the limb's plane and on the terminator's
    for p in [a, b] {
        near(norm(p), EARTH);
        near(dot(p, eye(5.0)), EARTH * EARTH);
        assert!(dot(p, sun45()).abs() < 1e-6 * EARTH);
    }
    // the Sun on the line of sight, or behind the body: no cusps
    assert_eq!(cusps(eye(5.0), [0.0, -1.0, 0.0], EARTH).unwrap(), None);
    assert_eq!(cusps(eye(5.0), [0.01, 1.0, 0.0], EARTH).unwrap(), None);
    // the terminator's points are on the great circle normal to the Sun
    for p in terminator_points(sun45(), EARTH, 12, None).unwrap() {
        near(norm(p), EARTH);
        assert!(dot(p, sun45()).abs() < 1e-6 * EARTH);
    }
    assert!(lit([1.0, 0.0, 0.0], [1.0, 0.0, 0.0]) && !lit([-1.0, 0.0, 0.0], [1.0, 0.0, 0.0]));
}

#[test]
fn rule_3_day_night_and_the_suns_limb() {
    near(day_weight(unit([1.0, 0.0, 1.0]).unwrap(), [0.0, 0.0, 1.0]), std::f64::consts::FRAC_1_SQRT_2);
    assert_eq!(day_weight([0.0, 0.0, -1.0], [0.0, 0.0, 1.0]), 0.0);
    near(night_weight(body_direction(93.0, 0.0).unwrap(), [1.0, 0.0, 0.0], 6.0).unwrap(), 0.75);
    near(night_weight(body_direction(90.0, 0.0).unwrap(), [1.0, 0.0, 0.0], 6.0).unwrap(), 0.5);
    assert_eq!(night_weight([1.0, 0.0, 0.0], [1.0, 0.0, 0.0], 6.0).unwrap(), 0.0);
    assert_eq!(night_weight([-1.0, 0.0, 0.0], [1.0, 0.0, 0.0], 6.0).unwrap(), 1.0);
    assert!(night_weight([1.0, 0.0, 0.0], [1.0, 0.0, 0.0], 0.0).is_err());
    near(limb_alpha(610.0).unwrap(), 0.455689);
    near(limb_alpha(550.0).unwrap(), 0.507909);
    near(limb_alpha(465.0).unwrap(), 0.604957);
    near(limb_alpha(1000.0).unwrap(), 0.269);
    near(limb_alpha(330.0).unwrap(), -0.507 + 0.441 * 1000.0 / 330.0);
    for nm in [300.0, 380.0, 1101.0] {
        assert!(limb_alpha(nm).is_err(), "{nm}");
    }
    near(limb_darkening(0.25, limb_alpha(550.0).unwrap()).unwrap(), 0.494548);
    assert!(limb_darkening(1.5, 0.5).is_err());
    near(disc_average(limb_alpha(550.0).unwrap()), 0.797477);
    near3(solar_disc(0.1).unwrap(), [0.350196, 0.310521, 0.248338]);
}

#[test]
fn rule_4_the_clouds_shadow_and_drift() {
    let p = surface_point(EARTH, 0.0, 0.0, 60.0).unwrap();
    let q = cloud_shadow_point(p, [0.0, 0.0, 1.0], EARTH, 0.004).unwrap().unwrap();
    near(angle(p, q).to_degrees(), 0.131704);
    near(norm(q), EARTH * 1.004);
    assert_eq!(cloud_shadow_point(p, [0.0, 0.0, -1.0], EARTH, 0.004).unwrap(), None);
    assert!(cloud_shadow_point(p, [0.0, 0.0, 1.0], EARTH, 0.0).is_err());
    near(cloud_longitude(100.0, 1.5), 150.0);
    near(cloud_longitude(300.0, 1.5), 90.0);
    assert_eq!(clouds_uv(150.0, 0.0, 100.0, 1.5).unwrap(), texture_uv(0.0, 0.0).unwrap());
}

#[test]
fn rule_5_the_rings_and_their_shadows() {
    let s = ring_sample(&STRIP, 1.239 + (2.269 - 1.239) / 6.0, 1.239, 2.269).unwrap().unwrap();
    for (got, want) in s.iter().zip([0.7, 0.675, 0.55, 0.55]) {
        near(*got, want);
    }
    assert_eq!(ring_sample(&STRIP, 2.269, 1.239, 2.269).unwrap(), Some(STRIP[3]));
    assert_eq!(ring_sample(&STRIP, 1.2, 1.239, 2.269).unwrap(), None);
    assert!(ring_sample(&STRIP, 1.5, 2.0, 1.0).is_err() && ring_sample(&STRIP[..1], 1.5, 1.0, 2.0).is_err());
    let (along, across) = planet_shadow_on_rings(saturn_sun(), SATURN).unwrap();
    near(along / SATURN, 2.22559);
    near(across / SATURN, 1.0);
    assert_eq!(planet_shadow_on_rings([1.0, 0.0, 0.0], SATURN), None);
    let p = surface_point(SATURN, 0.0, 0.0, -20.0).unwrap();
    near(ring_crossing(p, saturn_sun()).unwrap() / SATURN, 1.61972);
    near(ring_shadow(p, saturn_sun(), &STRIP, 1.239 * SATURN, 2.269 * SATURN).unwrap(), 0.198014);
    // the Sun on the point's own side of the plane: no crossing ahead, full light
    let north = surface_point(SATURN, 0.0, 0.0, 20.0).unwrap();
    assert_eq!(ring_crossing(north, saturn_sun()), None);
    assert_eq!(ring_shadow(north, saturn_sun(), &STRIP, 1.239 * SATURN, 2.269 * SATURN).unwrap(), 1.0);
    // a ring point behind the planet from the Sun is in its shadow, one beside it is not
    let behind = [-1.5 * SATURN, 0.0, 0.0];
    assert!(ring_shadowed_by_planet(behind, saturn_sun(), SATURN));
    assert!(!ring_shadowed_by_planet([0.0, 1.5 * SATURN, 0.0], saturn_sun(), SATURN));
    // on the sunward side the line toward the Sun meets the sphere only behind the point: lit
    assert!(!ring_shadowed_by_planet([1.5 * SATURN, 0.0, 0.0], saturn_sun(), SATURN));
}

#[test]
fn rule_6_the_atmospheres_band_from_orbit() {
    let b = shell_band(6360.0 + 400.0, 6360.0, 100.0).unwrap();
    near(b.planet.unwrap(), 70.1911);
    near(b.shell.unwrap(), 72.8666);
    let inside = shell_band(6360.0 + 50.0, 6360.0, 100.0).unwrap();
    assert!(inside.planet.is_some() && inside.shell.is_none());
    assert!(shell_band(7000.0, 6360.0, 0.0).is_err());
}

#[test]
fn rule_7_spin_drift_and_the_seek_check() {
    near(spin_degrees(399, 86164.0962).unwrap(), 360.0);
    near(spin_degrees(699, 3600.0).unwrap(), 33.7831);
    assert!(spin_degrees(401, 1.0).is_err());
    let s = frame_state(600, 24.0, 60.0, 0.01, 399).unwrap();
    near(s.cloud_longitude, 15.0);
    near(s.seconds, 1500.0);
    assert!(frame_state(0, 0.0, 60.0, 0.01, 399).is_err());
    // the frame computed alone is the state reached by stepping every frame from 0 (SREP 14)
    let (mut lon, mut spin) = (0.0f64, 0.0f64);
    for _ in 0..600 {
        lon = (lon + 60.0 / 24.0 * 0.01) % 360.0;
        spin += spin_degrees(399, 60.0 / 24.0).unwrap();
    }
    assert!((lon - s.cloud_longitude).abs() < 1e-6 && (spin - s.spin).abs() < 1e-6 * s.spin.abs());
}
