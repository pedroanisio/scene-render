//! Rules 10 and 11 of the physical-sky draft (sr-core srep/srep-0000-physical-sky.md) as the renderer applies them:
//! `sr_3d::aerial`'s per-camera volume and horizon table held to `sr_3d::atmosphere`'s march and `horizon_radiance`.

use sr_3d::aerial::{AerialSky, AerialVolume, HorizonTable, HORIZON_AZIMUTHS};
use sr_3d::atmosphere::{atmosphere, unit, AtmosphereOptions, Fog, Scattering};
use sr_3d::physical_sky::to_planet;

type Ray = dyn Fn(f64, f64) -> (V3, V3) + Sync;

type V3 = [f64; 3];

fn earth() -> Scattering {
    Scattering::new(atmosphere(&AtmosphereOptions::default()).unwrap())
}

/// A pinhole looking along the scene's +z, 60° across and 16:9, x right and v up: the scene direction of the screen
/// point (u, v) in [0, 1]².
fn scene_dir(u: f64, v: f64) -> V3 {
    let (th, tv) = ((30f64).to_radians().tan(), (30f64).to_radians().tan() * 9.0 / 16.0);
    unit([(2.0 * u - 1.0) * th, -(2.0 * v - 1.0) * tv, 1.0]).unwrap()
}

/// The pinhole's planet-frame direction.
fn pinhole(u: f64, v: f64) -> V3 {
    to_planet(scene_dir(u, v))
}

/// The pinhole's rays from a planet-frame camera.
fn rays(camera: V3) -> impl Fn(f64, f64) -> (V3, V3) + Sync {
    move |u, v| (camera, pinhole(u, v))
}

fn within(got: V3, want: V3, tol: f64, what: &str) {
    for c in 0..3 {
        assert!((got[c] - want[c]).abs() <= tol * want[c].abs(), "{what}: {got:?}, the march {want:?}");
    }
}

#[test]
fn the_volume_follows_the_march_on_its_columns_and_between_them() {
    let s = earth();
    let r = s.atmosphere.planet_radius;
    let (obs, camera) = ([0.0, 0.0, r], [0.0, 0.0, r + 2.0]);
    let sun = unit([0.6, 0.3, 0.2]).unwrap();
    let v = AerialVolume::new(&s, &rays(camera), obs, sun, None, 50000.0, [33, 19, 64]).unwrap();
    let colour = [0.4, 0.3, 0.2];
    for (u, w) in [(0.0, 0.0), (0.5, 0.5), (1.0, 1.0), (0.3125, 0.5)] {
        // (0.3125, 0.5) is a node: 10 of 32 across, 9 of 18 up
        for d in [3.0, 400.0, 1000.0, 12345.0, 50000.0] {
            let want = s.aerial_perspective(camera, pinhole(u, w), d, colour, sun).unwrap();
            within(v.apply(u, w, d, colour), want, 0.005, &format!("node ({u}, {w}) at {d} m"));
        }
    }
    for (u, w) in [(0.21, 0.53), (0.77, 0.41), (0.5, 0.97)] {
        for d in [1000.0, 20000.0] {
            let want = s.aerial_perspective(camera, pinhole(u, w), d, colour, sun).unwrap();
            within(v.apply(u, w, d, colour), want, 0.02, &format!("({u}, {w}) at {d} m"));
        }
    }
    assert_eq!(v.apply(0.4, 0.6, 0.0, colour), colour, "at the camera the colour itself");
    assert_eq!(v.apply(0.4, 0.6, 9e9, colour), v.apply(0.4, 0.6, 50000.0, colour), "beyond the volume its far slice");
}

#[test]
fn the_fog_marches_with_the_air_in_the_volume() {
    let s = earth();
    let r = s.atmosphere.planet_radius;
    let (obs, camera) = ([0.0, 0.0, r], [0.0, 0.0, r + 2.0]);
    let sun = unit([1.0, 0.0, 0.5]).unwrap();
    let f = Fog::new(0.001, 100.0).unwrap();
    let v = AerialVolume::new(&s, &rays(camera), obs, sun, Some(&f), 5000.0, [33, 19, 64]).unwrap();
    let colour = [0.5; 3];
    for (u, w) in [(0.5, 0.5), (0.25, 0.5), (0.4, 0.62)] {
        let d = pinhole(u, w);
        let horizon = s.horizon_radiance(obs, d, sun).unwrap();
        for dist in [100.0, 1000.0, 3000.0] {
            let want = s.aerial_perspective_fogged(camera, d, dist, colour, sun, &f, horizon).unwrap();
            within(v.apply(u, w, dist, colour), want, 0.02, &format!("({u}, {w}) at {dist} m"));
        }
    }
    // no fog is the air's volume to the bit
    let air = AerialVolume::new(&s, &rays(camera), obs, sun, None, 5000.0, [9, 5, 16]).unwrap();
    let none = Fog::new(0.0, 100.0).unwrap();
    let still = AerialVolume::new(&s, &rays(camera), obs, sun, Some(&none), 5000.0, [9, 5, 16]).unwrap();
    assert_eq!(air, still);
}

#[test]
fn the_volume_refuses_what_it_cannot_hold() {
    let s = earth();
    let r = s.atmosphere.planet_radius;
    let (obs, camera, sun) = ([0.0, 0.0, r], [0.0, 0.0, r + 2.0], [0.0, 0.0, 1.0]);
    let make = |camera: V3, max: f64, size: [usize; 3]| AerialVolume::new(&s, &rays(camera), obs, sun, None, max, size);
    assert!(make(camera, 1000.0, [2, 2, 1]).is_ok());
    for size in [[1, 2, 1], [2, 1, 1], [2, 2, 0], [4097, 2, 1]] {
        assert!(make(camera, 1000.0, size).is_err(), "{size:?}");
    }
    for max in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(make(camera, max, [2, 2, 1]).is_err(), "{max}");
    }
    // a camera under the sphere marches its segments through sea-level air (78251fd)
    let under = make([0.0, 0.0, r - 10.0], 1000.0, [2, 2, 1]).unwrap();
    let want = s.aerial_perspective([0.0, 0.0, r - 10.0], pinhole(0.0, 0.0), 1000.0, [0.5; 3], sun).unwrap();
    within(under.apply(0.0, 0.0, 1000.0, [0.5; 3]), want, 1e-5, "a camera under the ground");
    for bad in [([f64::NAN, 0.0, r + 2.0], [0.0, 0.0, 1.0]), ([0.0, 0.0, r + 2.0], [0.0, f64::INFINITY, 1.0])] {
        let ray = move |_: f64, _: f64| bad;
        assert!(AerialVolume::new(&s, &ray, obs, sun, None, 1000.0, [2, 2, 1]).is_err(), "{bad:?}");
    }
    assert!(
        AerialVolume::new(&s, &rays(camera), [1.0, 0.0, r], sun, None, 1000.0, [2, 2, 1]).is_err(),
        "off the z axis"
    );
}

#[test]
fn the_horizon_table_follows_the_horizon_radiance() {
    let s = earth();
    let r = s.atmosphere.planet_radius;
    // a sun 2° up, where the horizon toward it peaks
    for (obs, sun) in
        [([0.0, 0.0, r], unit([1.0, 0.2, 0.035]).unwrap()), ([0.0, 0.0, r + 500.0], unit([0.3, -0.4, 0.8]).unwrap())]
    {
        let t = HorizonTable::new(&s, obs, sun).unwrap();
        // the last node's interval, 359° to 360°, wraps to the first
        for k in 0..38 {
            let a = if k == 37 { 359.5f64.to_radians() } else { (k as f64 * 10.0 + 3.7).to_radians() };
            let d = [a.cos(), a.sin(), -0.3];
            within(t.radiance(d), s.horizon_radiance(obs, d, sun).unwrap(), 0.01, &format!("azimuth {k}"));
        }
        assert_eq!(t.radiance([0.0, 0.0, 1.0]), t.radiance([1.0, 0.0, 0.0]), "a vertical ray takes +x");
    }
    assert!(HorizonTable::new(&s, [0.0, 1.0, r], [0.0, 0.0, 1.0]).is_err());
}

#[test]
fn a_camera_above_the_air_marches_from_the_top() {
    // from 400 km the surface segment is walked from the camera, 300 km of vacuum before the air (78251fd): the
    // nodes hold the converged march (1024 slices) to 5e-4, and the reference's 16 segments, clustered toward the camera and
    // so coarse in the air, to the kit's 2 %
    let s = earth();
    let r = s.atmosphere.planet_radius;
    let (obs, camera) = ([0.0, 0.0, r], [0.0, 0.0, r + 400000.0]);
    let down = move |u: f64, v: f64| (camera, unit([(2.0 * u - 1.0) * 0.1, (2.0 * v - 1.0) * 0.1, -1.0]).unwrap());
    let sun = unit([0.3, 0.2, 0.9]).unwrap();
    let v = AerialVolume::new(&s, &down, obs, sun, None, 400000.0, [3, 3, 64]).unwrap();
    let fine = AerialVolume::new(&s, &down, obs, sun, None, 400000.0, [3, 3, 1024]).unwrap();
    let colour = [0.2, 0.3, 0.4];
    for k in [50, 60, 63, 64] {
        let d = 400000.0 * (k as f64 / 64.0).powi(2);
        within(v.apply(0.5, 0.5, d, colour), fine.apply(0.5, 0.5, d, colour), 5e-4, &format!("slice {k}"));
        let want = s.aerial_perspective(camera, down(0.5, 0.5).1, d, colour, sun).unwrap();
        within(v.apply(0.5, 0.5, d, colour), want, 0.02, &format!("slice {k} against the reference"));
        // the air begins 300 km down: slice 50 (244 km) is vacuum, slices 60 on (351 km) are in the air
        assert_eq!(v.apply(0.5, 0.5, d, colour) == colour, k < 60, "slice {k}");
    }
}

#[test]
fn the_fog_takes_its_horizon_at_the_observer_not_the_camera() {
    // a camera 300 m up looking 10° down: the fog's source is L_h at the dome's observer on the ground
    let s = earth();
    let r = s.atmosphere.planet_radius;
    let (obs, camera) = ([0.0, 0.0, r], [0.0, 0.0, r + 300.0]);
    let (c, sn) = (10f64.to_radians().cos(), 10f64.to_radians().sin());
    let tilt = move |u: f64, v: f64| {
        let d = pinhole(u, v);
        (camera, unit([d[0], d[1] * c + d[2] * sn, d[2] * c - d[1] * sn]).unwrap())
    };
    let sun = unit([0.2, 1.0, 0.1]).unwrap();
    let f = Fog::new(0.004, 250.0).unwrap();
    let v = AerialVolume::new(&s, &tilt, obs, sun, Some(&f), 3000.0, [33, 19, 64]).unwrap();
    for (u, w) in [(0.5, 0.5), (0.25, 0.25)] {
        let d = tilt(u, w).1;
        let horizon = s.horizon_radiance(obs, d, sun).unwrap();
        for dist in [1000.0, 2500.0] {
            let want = s.aerial_perspective_fogged(camera, d, dist, [0.5; 3], sun, &f, horizon).unwrap();
            within(v.apply(u, w, dist, [0.5; 3]), want, 0.005, &format!("({u}, {w}) at {dist} m"));
        }
    }
}

#[test]
fn the_aerial_sky_maps_the_scene_into_the_planet() {
    let s = earth();
    let r = s.atmosphere.planet_radius;
    let sun = unit([0.2, -0.8, 0.5]).unwrap();
    let f = Fog::new(0.001, 100.0).unwrap();
    let a = AerialSky::new(s.clone(), 10.0, sun, true, Some(f), 100.0).unwrap().expect("a march");
    // a camera 2 m up and 3 m along x in scene pixels, its rays in the dome's axes
    let scene = |u: f64, v: f64| ([300.0, -200.0, 0.0], scene_dir(u, v));
    let planet = move |u: f64, v: f64| ([3.0, 0.0, r + 12.0], pinhole(u, v));
    let got = a.volume(&scene, 500000.0, [5, 4, 8]).unwrap().expect("a volume");
    let obs = [0.0, 0.0, r + 10.0];
    let want = AerialVolume::new(&s, &planet as &Ray, obs, to_planet(sun), Some(&f), 5000.0, [5, 4, 8]).unwrap();
    assert_eq!(got, want);
    assert_eq!(a.horizon(), &HorizonTable::new(&s, obs, to_planet(sun)).unwrap());
    let n = HORIZON_AZIMUTHS as f32;
    assert_eq!(a.uniforms(5000.0), [[1.0, 100.0, 10.0, 5000.0], [0.001, 100.0, n, 0.0]]);
    // the closed form: no volume, the fog's uniforms
    let closed = AerialSky::new(s.clone(), 0.0, sun, false, Some(f), 1.0).unwrap().expect("a fog");
    assert!(closed.volume(&scene, 1000.0, [2, 2, 1]).unwrap().is_none());
    assert_eq!(closed.uniforms(1000.0), [[2.0, 1.0, 0.0, 1000.0], [0.001, 100.0, n, 0.0]]);
    // nothing to apply: no march and no fog, or a fog of no density
    assert!(AerialSky::new(s.clone(), 0.0, sun, false, None, 1.0).unwrap().is_none());
    assert!(AerialSky::new(s.clone(), 0.0, sun, false, Some(Fog::new(0.0, 100.0).unwrap()), 1.0).unwrap().is_none());
    // the march alone carries no fog
    let air = AerialSky::new(s.clone(), 0.0, sun, true, None, 1.0).unwrap().expect("a march");
    assert_eq!(air.uniforms(10.0)[1], [0.0, 100.0, n, 0.0]);
    for k in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(AerialSky::new(s.clone(), 0.0, sun, true, None, k).is_err(), "pixelsPerMeter {k}");
    }
    let low = AerialSky::new(s.clone(), -430.0, sun, true, None, 1.0).unwrap().expect("a march");
    assert_eq!(low.horizon(), air.horizon(), "an observer under the sphere is lifted onto it");
    for altitude in [f64::NAN, f64::INFINITY] {
        assert!(
            AerialSky::new(s.clone(), altitude, sun, false, Some(Fog::new(0.001, 100.0).unwrap()), 1.0).is_err(),
            "altitude {altitude}"
        );
    }
    let under = |_: f64, _: f64| ([0.0, 5000.0, 0.0], [0.0, 0.0, 1.0]);
    a.volume(&under, 1000.0, [2, 2, 1]).unwrap().expect("a camera 40 m under the ground marches");
}
