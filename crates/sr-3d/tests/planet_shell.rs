//! Rule 6 of the sr-core planets draft (srep/srep-0000-planets.md): a planet's atmosphere shell as
//! `sr_3d::planet_shell` tabulates it for one observer, held to `sr_3d::atmosphere`'s integrals.

use sr_3d::atmosphere::{atmosphere, unit, AtmosphereOptions, Scattering};
use sr_3d::planet::shell_band;
use sr_3d::planet_shell::{PlanetShell, SHELL_HEADER};

type V3 = [f64; 3];

const R: f64 = 6_378_137.0;
const HEIGHT: f64 = 100_000.0;

fn earth(irradiance: V3) -> Scattering {
    let o = AtmosphereOptions {
        planet_radius: R,
        top_radius: R + HEIGHT,
        sun_irradiance: irradiance,
        ..Default::default()
    };
    Scattering::new(atmosphere(&o).unwrap())
}

/// An orthonormal basis with `up` third: the frame the reference's observer on +z sees.
fn basis(up: V3) -> [V3; 3] {
    let a = if up[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
    let x = unit(cross(a, up)).unwrap();
    [x, cross(up, x), up]
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn into(b: &[V3; 3], v: V3) -> V3 {
    b.map(|r| r[0] * v[0] + r[1] * v[1] + r[2] * v[2])
}

fn from(b: &[V3; 3], v: V3) -> V3 {
    std::array::from_fn(|i| b[0][i] * v[0] + b[1][i] * v[1] + b[2][i] * v[2])
}

/// The unit direction `nadir` degrees from the observer's nadir, turned `azimuth` degrees about it (local frame).
fn from_nadir(nadir: f64, azimuth: f64) -> V3 {
    let (n, a) = (nadir.to_radians(), azimuth.to_radians());
    [n.sin() * a.cos(), n.sin() * a.sin(), -n.cos()]
}

/// 400 km above a point off every axis, the sun 48° from that point's zenith.
fn orbit() -> (V3, [V3; 3], V3) {
    let up = unit([0.3, -0.8, 0.52]).unwrap();
    let b = basis(up);
    let observer = up.map(|c| c * (R + 400_000.0));
    let sun = from(&b, [48f64.to_radians().sin() * 0.6, 48f64.to_radians().sin() * 0.8, 48f64.to_radians().cos()]);
    (observer, b, sun)
}

fn within(got: V3, want: V3, tol: f64, what: &str) {
    for c in 0..3 {
        assert!((got[c] - want[c]).abs() <= tol * want[c].abs().max(1e-12), "{what}: {got:?}, the integral {want:?}");
    }
}

#[test]
fn the_shell_is_the_physical_skys_radiance_from_the_observer_in_any_frame() {
    let (observer, b, sun) = orbit();
    let s = earth([1.0; 3]);
    let shell = PlanetShell::new(R, HEIGHT, observer, Some(sun), [1.0; 3]).unwrap();
    let obs_local = [0.0, 0.0, R + 400_000.0];
    let band = shell_band(R + 400_000.0, R, HEIGHT).unwrap();
    let (ground, top) = (band.planet.unwrap(), band.shell.unwrap());
    let mut lit = 0;
    for nadir in [ground + 0.05, ground + 0.4, (ground + top) / 2.0, top - 0.3] {
        for azimuth in [0.0, 37.0, 90.0, 180.0, 251.0] {
            let local = from_nadir(nadir, azimuth);
            let want = s.sky_radiance(obs_local, local, into(&b, sun)).unwrap();
            let got = shell.radiance(from(&b, local));
            within(got, want, 0.05, &format!("{nadir:.3}° from the nadir at azimuth {azimuth}"));
            lit += usize::from(want[2] > 0.0);
        }
    }
    assert!(lit >= 18, "the band is lit: {lit} of 20");
}

#[test]
fn the_shell_is_dark_toward_the_planet_and_above_its_top() {
    let (observer, b, sun) = orbit();
    let shell = PlanetShell::new(R, HEIGHT, observer, Some(sun), [1.0; 3]).unwrap();
    let band = shell_band(R + 400_000.0, R, HEIGHT).unwrap();
    let (ground, top) = (band.planet.unwrap(), band.shell.unwrap());
    for nadir in [0.0, 30.0, ground - 0.05] {
        assert_eq!(shell.radiance(from(&b, from_nadir(nadir, 20.0))), [0.0; 3], "toward the planet at {nadir}°");
        assert_eq!(shell.transmittance(from(&b, from_nadir(nadir, 20.0))), [0.0; 3], "through the planet at {nadir}°");
    }
    for nadir in [top + 0.2, 90.0, 180.0] {
        assert_eq!(shell.radiance(from(&b, from_nadir(nadir, 20.0))), [0.0; 3], "past the top at {nadir}°");
        assert_eq!(shell.transmittance(from(&b, from_nadir(nadir, 20.0))), [1.0; 3], "past the top at {nadir}°");
    }
}

#[test]
fn the_shell_transmits_as_the_atmosphere_does() {
    let (observer, b, sun) = orbit();
    let s = earth([1.0; 3]);
    let shell = PlanetShell::new(R, HEIGHT, observer, Some(sun), [1.0; 3]).unwrap();
    let obs_local = [0.0, 0.0, R + 400_000.0];
    let band = shell_band(R + 400_000.0, R, HEIGHT).unwrap();
    let (ground, top) = (band.planet.unwrap(), band.shell.unwrap());
    for nadir in [ground + 0.3, (ground + top) / 2.0, top - 0.5, top - 0.1] {
        let local = from_nadir(nadir, 73.0);
        let want = s.atmosphere.transmittance(obs_local, local).unwrap();
        within(shell.transmittance(from(&b, local)), want, 0.03, &format!("{nadir:.3}° from the nadir"));
    }
}

#[test]
fn the_shell_scales_with_the_irradiance_and_scatters_nothing_without_a_sun() {
    let (observer, b, sun) = orbit();
    let one = PlanetShell::new(R, HEIGHT, observer, Some(sun), [1.0; 3]).unwrap();
    let tinted = PlanetShell::new(R, HEIGHT, observer, Some(sun), [2.0, 1.0, 0.5]).unwrap();
    let dark = PlanetShell::new(R, HEIGHT, observer, None, [1.0; 3]).unwrap();
    let band = shell_band(R + 400_000.0, R, HEIGHT).unwrap();
    let d = from(&b, from_nadir((band.planet.unwrap() + band.shell.unwrap()) / 2.0, 10.0));
    let (l1, l2) = (one.radiance(d), tinted.radiance(d));
    within(l2, [2.0 * l1[0], l1[1], 0.5 * l1[2]], 1e-9, "the irradiance's tint");
    assert_eq!(dark.radiance(d), [0.0; 3], "no sun, no light scattered");
    assert_eq!(dark.transmittance(d), one.transmittance(d), "the air absorbs alike without a sun");
}

#[test]
fn the_volume_carries_the_shell_to_the_surfaces_it_covers() {
    let (observer, b, sun) = orbit();
    let s = earth([1.0; 3]);
    let shell = PlanetShell::new(R, HEIGHT, observer, Some(sun), [1.0; 3]).unwrap();
    // a pinhole straight down, 20° across: every column meets the ground
    let ray = |u: f64, v: f64| {
        let t = 10f64.to_radians().tan();
        (observer, from(&b, unit([(2.0 * u - 1.0) * t, (2.0 * v - 1.0) * t, -1.0]).unwrap()))
    };
    let volume = shell.volume(&ray, 600_000.0, [9, 9, 128]).unwrap();
    let colour = [0.2, 0.3, 0.1];
    for (u, v) in [(0.5, 0.5), (0.0, 1.0), (0.25, 0.75)] {
        let d = ray(u, v).1;
        let local = into(&b, d);
        let distance = ground_distance(R + 400_000.0, local);
        let want = s.aerial_perspective([0.0, 0.0, R + 400_000.0], local, distance, colour, into(&b, sun)).unwrap();
        within(volume.apply(u, v, distance, colour), want, 0.02, &format!("the ground at ({u}, {v})"));
    }
}

fn ground_distance(r_obs: f64, d: V3) -> f64 {
    let b = r_obs * d[2];
    let c = r_obs * r_obs - R * R;
    -b - (b * b - c).sqrt()
}

#[test]
fn the_shell_table_is_what_a_shader_reads() {
    let (observer, b, sun) = orbit();
    let shell = PlanetShell::new(R, HEIGHT, observer, Some(sun), [1.0; 3]).unwrap();
    let t = shell.table();
    let (rows, cols) = (t[3][0] as usize, t[3][1] as usize);
    assert_eq!(t.len(), SHELL_HEADER + rows * cols + rows);
    for (i, row) in t[..3].iter().enumerate() {
        for c in 0..3 {
            assert!((row[c] as f64 - b[i][c]).abs() < 1e-6, "row {i} of the rotation into the observer's frame");
        }
    }
    let band = shell_band(R + 400_000.0, R, HEIGHT).unwrap();
    assert!((t[0][3] as f64 + (90.0 - band.planet.unwrap()).to_radians()).abs() < 1e-6, "the horizon's elevation");
}

#[test]
fn the_shell_refuses_what_it_cannot_draw() {
    let (observer, _, sun) = orbit();
    assert!(PlanetShell::new(0.0, HEIGHT, observer, Some(sun), [1.0; 3]).is_err());
    assert!(PlanetShell::new(R, 0.0, observer, Some(sun), [1.0; 3]).is_err());
    assert!(PlanetShell::new(R, HEIGHT, [f64::NAN, 0.0, R], Some(sun), [1.0; 3]).is_err());
    assert!(PlanetShell::new(R, HEIGHT, [0.0; 3], Some(sun), [1.0; 3]).is_err());
    assert!(PlanetShell::new(R, HEIGHT, observer, Some([0.0, 0.0, 2.0]), [1.0; 3]).is_err());
    assert!(PlanetShell::new(R, HEIGHT, observer, Some(sun), [-1.0, 1.0, 1.0]).is_err());
    assert!(PlanetShell::new(R, HEIGHT, observer, None, [-1.0, 1.0, 1.0]).is_err(), "an irradiance it would not use");
}

#[test]
fn from_inside_the_air_the_shell_reaches_the_zenith() {
    let up = unit([0.3, -0.8, 0.52]).unwrap();
    let b = basis(up);
    let observer = up.map(|c| c * (R + 2_000.0));
    let sun = from(&b, [0.5, 0.0, 0.75f64.sqrt()]);
    let s = earth([1.0; 3]);
    let shell = PlanetShell::new(R, HEIGHT, observer, Some(sun), [1.0; 3]).unwrap();
    let obs_local = [0.0, 0.0, R + 2_000.0];
    for nadir in [95.0, 135.0, 179.0] {
        let local = from_nadir(nadir, 120.0);
        let want = s.sky_radiance(obs_local, local, into(&b, sun)).unwrap();
        within(shell.radiance(from(&b, local)), want, 0.05, &format!("{nadir}° from the nadir"));
        let t = s.atmosphere.transmittance(obs_local, local).unwrap();
        within(shell.transmittance(from(&b, local)), t, 0.03, &format!("the transmittance {nadir}° from the nadir"));
    }
}

/// An orthographic view 7000 km out along `-view` from the centre, off every axis, the sun 60° from the view axis.
fn parallel_view() -> (V3, V3, V3, [V3; 3]) {
    let view = unit([-0.3, 0.8, -0.52]).unwrap();
    let b = basis(view.map(|c| -c));
    let observer = from(&b, [1_000.0, -2_000.0, 7_000_000.0]);
    let sun = from(&b, [60f64.to_radians().sin() * 0.8, 60f64.to_radians().sin() * 0.6, 60f64.to_radians().cos()]);
    (view, observer, sun, b)
}

/// The point of the ray of impact `rho` metres at `azimuth` degrees about the view axis, past the shell's top on the
/// observer's side (local frame of `parallel_view`'s basis, up toward the observer).
fn impact(rho: f64, azimuth: f64) -> V3 {
    let a = azimuth.to_radians();
    [rho * a.cos(), rho * a.sin(), R + HEIGHT + 1_000.0]
}

#[test]
fn a_parallel_view_reads_each_ray_by_its_impact_on_the_shell() {
    let (view, observer, sun, b) = parallel_view();
    let s = earth([1.0; 3]);
    let shell = PlanetShell::parallel(R, HEIGHT, observer, view, Some(sun), [1.0; 3]).unwrap();
    let mut lit = 0;
    for rho in [R + 300.0, R + 5_000.0, R + 20_000.0, R + 60_000.0] {
        for azimuth in [0.0, 37.0, 90.0, 180.0, 251.0] {
            let local = impact(rho, azimuth);
            let want = s.sky_radiance(local, [0.0, 0.0, -1.0], into(&b, sun)).unwrap();
            // any point of the ray, here the observer's plane, reads the same
            let origin = from(&b, [local[0], local[1], 6_900_000.0]);
            within(shell.radiance_on(origin), want, 0.05, &format!("{rho} m at azimuth {azimuth}"));
            let t = s.atmosphere.transmittance(local, [0.0, 0.0, -1.0]).unwrap();
            within(shell.transmittance_on(origin), t, 0.03, &format!("the transmittance at {rho} m"));
            lit += usize::from(want[2] > 0.0);
        }
    }
    // the rays over the limb's night side carry no light
    assert!(lit >= 12, "the band is lit: {lit} of 20");
}

#[test]
fn a_parallel_view_is_dark_inside_the_limb_and_clear_past_the_top() {
    let (view, observer, sun, b) = parallel_view();
    let shell = PlanetShell::parallel(R, HEIGHT, observer, view, Some(sun), [1.0; 3]).unwrap();
    for rho in [0.0, 0.5 * R, R - 10.0] {
        let o = from(&b, impact(rho, 20.0));
        assert_eq!(shell.radiance_on(o), [0.0; 3], "inside the limb at {rho} m");
        assert_eq!(shell.transmittance_on(o), [0.0; 3], "through the planet at {rho} m");
    }
    for rho in [R + HEIGHT + 10.0, 2.0 * R] {
        let o = from(&b, impact(rho, 20.0));
        assert_eq!(shell.radiance_on(o), [0.0; 3], "past the top at {rho} m");
        assert_eq!(shell.transmittance_on(o), [1.0; 3], "past the top at {rho} m");
    }
}

#[test]
fn a_parallel_view_refuses_what_it_cannot_draw() {
    let (view, observer, sun, _) = parallel_view();
    assert!(PlanetShell::parallel(R, HEIGHT, observer, view, Some(sun), [1.0; 3]).is_ok());
    // the rays start in a plane across the view: it lies past the shell's top, on the observer's side
    let inside = observer.map(|c| c * (R + HEIGHT - 1.0) / 7_000_000.3);
    assert!(PlanetShell::parallel(R, HEIGHT, inside, view, Some(sun), [1.0; 3]).is_err());
    let beyond = view.map(|c| c * 7_000_000.0);
    assert!(PlanetShell::parallel(R, HEIGHT, beyond, view, Some(sun), [1.0; 3]).is_err());
    assert!(PlanetShell::parallel(R, HEIGHT, observer, view.map(|c| c * 2.0), Some(sun), [1.0; 3]).is_err());
    assert!(PlanetShell::parallel(R, HEIGHT, observer, view, Some(sun), [-1.0, 1.0, 1.0]).is_err());
    assert!(PlanetShell::parallel(R, 0.0, observer, view, Some(sun), [1.0; 3]).is_err());
}

#[test]
fn a_parallel_shells_table_carries_the_observers_offset_across_the_view() {
    let (view, observer, sun, b) = parallel_view();
    let shell = PlanetShell::parallel(R, HEIGHT, observer, view, Some(sun), [1.0; 3]).unwrap();
    let t = shell.table();
    let (rows, cols) = (t[3][0] as usize, t[3][1] as usize);
    assert_eq!(t.len(), SHELL_HEADER + rows * cols + rows);
    assert_eq!(t[3][2], 1.0, "a parallel view");
    assert_eq!(PlanetShell::new(R, HEIGHT, observer, Some(sun), [1.0; 3]).unwrap().table()[3][2], 0.0);
    assert!((t[0][3] as f64 - R).abs() < 1.0 && (t[1][3] as f64 - R - HEIGHT).abs() < 1.0, "the limb and the top");
    let local = into(&b, observer);
    for c in 0..3 {
        assert!((t[4][c] as f64 - local[c]).abs() < 1.0, "the observer in the view's frame: {:?}", t[4]);
        assert!((t[2][c] as f64 + view[c]).abs() < 1e-6, "up is toward the observer");
    }
}
