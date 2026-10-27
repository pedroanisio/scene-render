//! The sky-view table of a dome light with `sky="physical"` against the integrals of `sr_3d::atmosphere` (rule 9 of
//! the sr-core physical-sky draft), and the sun disc it is drawn with (rule 6).

use sr_3d::atmosphere::{atmosphere, unit, AtmosphereOptions, Scattering};
use sr_3d::physical_sky::{to_planet, SkyView, SKY_VIEW_AZIMUTHS, SKY_VIEW_ELEVATIONS};

const R: f64 = 6360000.0;

fn earth() -> Scattering {
    Scattering::new(atmosphere(&AtmosphereOptions::default()).unwrap())
}

/// The unit direction of elevation `e` and azimuth `a` (degrees) about an observer on the +z axis.
fn dir(e: f64, a: f64) -> [f64; 3] {
    let (e, a) = (e.to_radians(), a.to_radians());
    [e.cos() * a.cos(), e.cos() * a.sin(), e.sin()]
}

fn rel(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|c| (a[c] - b[c]).abs() / b[c].abs().max(1e-12)).fold(0.0, f64::max)
}

#[test]
fn the_zenith_is_the_reference_integral() {
    // the kit's sky-day case reads the zenith pixel to 2 %: the table holds the zenith as a node
    let s = earth();
    for sun_elevation in [60.0, 30.0, 5.0] {
        let sun = dir(sun_elevation, 40.0);
        let v = SkyView::new(&s, 0.0, sun).unwrap();
        let want = s.sky_radiance([0.0, 0.0, R], [0.0, 0.0, 1.0], sun).unwrap();
        assert!(rel(v.radiance([0.0, 0.0, 1.0]), want) < 1e-9, "{sun_elevation}");
    }
}

#[test]
fn the_sky_between_the_nodes_is_within_two_percent() {
    // a sun at 30°: every direction 1° and more above the horizon and 10° and more from the sun, against the
    // integral itself
    let s = earth();
    let sun = dir(30.0, 0.0);
    let v = SkyView::new(&s, 0.0, sun).unwrap();
    let mut worst: f64 = 0.0;
    for e in [1.0, 2.5, 7.0, 15.0, 33.0, 52.0, 77.0, 89.0] {
        for a in [13.0, 47.0, 95.0, 141.0, 179.0, -60.0] {
            let d = dir(e, a);
            let cos = d.iter().zip(sun).map(|(x, y)| x * y).sum::<f64>();
            if cos > 10f64.to_radians().cos() {
                continue;
            }
            let want = s.sky_radiance([0.0, 0.0, R], d, sun).unwrap();
            worst = worst.max(rel(v.radiance(d), want));
        }
    }
    assert!(worst < 0.02, "{worst}");
}

#[test]
fn the_sky_is_symmetric_about_the_sun_and_black_below_the_horizon() {
    let s = earth();
    let sun = dir(20.0, 70.0);
    let v = SkyView::new(&s, 0.0, sun).unwrap();
    for (e, a) in [(10.0, 30.0), (45.0, 120.0), (3.0, 179.0)] {
        let l = v.radiance(dir(e, 70.0 + a));
        let r = v.radiance(dir(e, 70.0 - a));
        assert!(rel(l, r) < 1e-12, "{e} {a}");
        assert!(l.iter().all(|c| *c > 0.0));
    }
    for e in [-0.5, -10.0, -90.0] {
        assert_eq!(v.radiance(dir(e, 10.0)), [0.0; 3], "{e}");
    }
}

#[test]
fn from_orbit_the_shell_lies_between_the_limb_and_the_top() {
    // 400 km up: the band from the planet's limb to the direction grazing the top scatters, above it is black
    let s = earth();
    let h = 400000.0;
    let sun = dir(0.0, 0.0);
    let v = SkyView::new(&s, h, sun).unwrap();
    let limb = -(R / (R + h)).acos().to_degrees();
    let top = -(s.atmosphere.top_radius / (R + h)).acos().to_degrees();
    assert!((v.horizon().to_degrees() - limb).abs() < 1e-9);
    let mid = v.radiance(dir((limb + top) / 2.0, 0.0));
    assert!(mid.iter().all(|c| *c > 0.0), "{mid:?}");
    assert_eq!(v.radiance(dir(top + 0.5, 0.0)), [0.0; 3]);
    assert_eq!(v.radiance(dir(30.0, 90.0)), [0.0; 3]);
    assert_eq!(v.radiance(dir(limb - 0.5, 0.0)), [0.0; 3]);
}

#[test]
fn the_horizon_row_is_sky_where_the_tangent_rounds_into_the_ground() {
    // from 8848 m the ray along the geometric horizon meets the planet in floating point: the table's first row is
    // taken just above it, so the band at the horizon is the sky's and not black
    let s = earth();
    let h = 8848.0;
    let obs = [0.0, 0.0, R + h];
    let sun = dir(20.0, 0.0);
    let v = SkyView::new(&s, h, sun).unwrap();
    let e = v.horizon();
    assert!(s.atmosphere.hits_ground(obs, [e.cos(), 0.0, e.sin()]).unwrap());
    let e = e + 1e-9;
    let want = s.sky_radiance(obs, [(e + 1e-6).cos(), 0.0, (e + 1e-6).sin()], sun).unwrap();
    assert!(
        rel(v.radiance([e.cos(), 0.0, e.sin()]), want) < 0.02,
        "{:?} {want:?}",
        v.radiance([e.cos(), 0.0, e.sin()])
    );
}

#[test]
fn the_table_has_its_stated_size_and_a_frame_alone_equals_a_frame_rebuilt() {
    // rule 12: no state carried between frames
    let s = earth();
    let sun = unit([0.3, -0.2, 0.6]).unwrap();
    let a = SkyView::new(&s, 120.0, sun).unwrap();
    let _ = SkyView::new(&s, 0.0, dir(-10.0, 0.0)).unwrap();
    let b = SkyView::new(&s, 120.0, sun).unwrap();
    assert_eq!(a, b);
    assert_eq!(a.len(), SKY_VIEW_AZIMUTHS * SKY_VIEW_ELEVATIONS);
}

#[test]
fn a_sun_that_is_not_a_unit_is_refused() {
    let s = earth();
    assert!(SkyView::new(&s, 0.0, [0.0, 0.0, 2.0]).is_err());
    // an observer below the sphere is lifted onto it (78251fd: `altitude` below 0 is the ellipsoid's)
    assert_eq!(SkyView::new(&s, -430.0, dir(10.0, 0.0)).unwrap(), SkyView::new(&s, 0.0, dir(10.0, 0.0)).unwrap());
    assert!(SkyView::new(&s, f64::NAN, dir(10.0, 0.0)).is_err());
}

#[test]
fn scene_axes_turn_up_to_the_planets_up() {
    // SREP 71's axes: up is −y, azimuth 0 is +z, azimuth 90° is +x
    assert_eq!(to_planet([0.0, -1.0, 0.0]), [0.0, 0.0, 1.0]);
    let (x, z) = (to_planet([1.0, 0.0, 0.0]), to_planet([0.0, 0.0, 1.0]));
    // a right-handed turn: the planet frame's x × y is its up
    let cross = [x[1] * z[2] - x[2] * z[1], x[2] * z[0] - x[0] * z[2], x[0] * z[1] - x[1] * z[0]];
    assert_eq!(cross.map(|c: f64| c.abs()), [0.0, 0.0, 1.0]);
}

mod dome {
    use super::*;
    use glam::Vec3;
    use sr_3d::env::dir_of;
    use sr_3d::physical_sky::PhysicalSky;

    /// The scene-axes direction of elevation `e` and azimuth `a` (degrees): up −y, azimuth 0 along +z.
    fn scene(e: f64, a: f64) -> [f64; 3] {
        let (e, a) = (e.to_radians(), a.to_radians());
        [e.cos() * a.sin(), -e.sin(), e.cos() * a.cos()]
    }

    fn sky(e: f64, size: f64) -> (Scattering, PhysicalSky) {
        let s = earth();
        let p = PhysicalSky::new(&s, 0.0, scene(e, 30.0), size, [1.0; 3]).unwrap();
        (s, p)
    }

    #[test]
    fn the_disc_holds_the_transmitted_irradiance() {
        // rule 6: the disc's power is rule 9's sun_radiance, the irradiance through the transmittance
        for e in [60.0, 10.0, 1.0] {
            let (s, p) = sky(e, 0.53);
            let want = s.atmosphere.sun_radiance([0.0, 0.0, R], to_planet(scene(e, 30.0))).unwrap();
            let omega = 2.0 * std::f64::consts::PI * (1.0 - (0.53f64.to_radians() / 2.0).cos());
            let d = Vec3::from(scene(e, 30.0).map(|c| c as f32));
            let disc = p.radiance(d) - p.sky(d);
            for c in 0..3 {
                assert!((disc[c] as f64 * omega - want[c]).abs() < 1e-4 * want[c], "{e} {c}: {disc}");
            }
            // just outside the disc only the sky remains
            let off = Vec3::from(scene(e + 0.3, 30.0).map(|c| c as f32));
            assert_eq!(p.radiance(off), p.sky(off));
        }
    }

    #[test]
    fn the_sky_is_the_table_in_scene_axes() {
        let (s, p) = sky(30.0, 0.53);
        for (e, a) in [(90.0, 0.0), (20.0, 120.0), (5.0, -40.0)] {
            let want = s.sky_radiance([0.0, 0.0, R], to_planet(scene(e, a)), to_planet(scene(30.0, 30.0))).unwrap();
            let got = p.sky(Vec3::from(scene(e, a).map(|c| c as f32)));
            for c in 0..3 {
                assert!((got[c] as f64 - want[c]).abs() < 0.02 * want[c], "{e} {a}: {got} {want:?}");
            }
        }
        assert_eq!(p.sky(Vec3::Y), Vec3::ZERO);
    }

    #[test]
    fn a_sun_below_the_horizon_or_of_size_zero_draws_no_disc() {
        let (_, below) = sky(-2.0, 0.53);
        assert!(!below.has_sun());
        let (_, none) = sky(40.0, 0.0);
        assert!(!none.has_sun());
        assert_eq!(none.uniforms()[4][..3], [0.0; 3]);
        let (_, up) = sky(40.0, 0.53);
        assert!(up.has_sun());
    }

    #[test]
    fn the_transmittance_toward_a_direction_is_rule_9s_from_the_observer() {
        // the star-fields draft, rule 6: a star is dimmed by the transmittance toward it, zero below the horizon
        let s = earth();
        for h in [0.0, 3000.0] {
            let p = PhysicalSky::new(&s, h, scene(30.0, 30.0), 0.53, [1.0; 3]).unwrap();
            for (e, a) in [(90.0, 0.0), (38.8, 170.0), (4.0, -60.0)] {
                let want = s.atmosphere.transmittance([0.0, 0.0, R + h], to_planet(scene(e, a))).unwrap();
                assert!(rel(p.transmittance(scene(e, a)).unwrap(), want) < 1e-12, "{h} {e} {a}");
            }
            assert_eq!(p.transmittance(scene(-5.0, 10.0)).unwrap(), [0.0; 3]);
        }
        // lower in the sky, longer through the air: dimmer, and bluer light lost first
        let p = PhysicalSky::new(&s, 0.0, scene(30.0, 30.0), 0.53, [1.0; 3]).unwrap();
        let (high, low) = (p.transmittance(scene(60.0, 0.0)).unwrap(), p.transmittance(scene(10.0, 0.0)).unwrap());
        assert!((0..3).all(|c| low[c] < high[c]) && low[2] < low[0], "{high:?} {low:?}");
        // an observer below the sphere is lifted onto it
        let under = PhysicalSky::new(&s, -430.0, scene(30.0, 30.0), 0.53, [1.0; 3]).unwrap();
        assert_eq!(under.transmittance(scene(20.0, 0.0)).unwrap(), p.transmittance(scene(20.0, 0.0)).unwrap());
        assert!(p.transmittance([0.0, 0.0, 2.0]).is_err());
    }

    #[test]
    fn a_disc_is_drawn_above_the_horizon_only() {
        // a sun whose centre is 0.1° up: the half of its disc below the horizon is the ground's, black
        let (_, p) = sky(0.1, 1.0);
        assert!(p.has_sun());
        let lower = Vec3::from(scene(-0.2, 30.0).map(|c| c as f32));
        assert_eq!(p.radiance(lower), Vec3::ZERO);
        let upper = Vec3::from(scene(0.3, 30.0).map(|c| c as f32));
        assert!(p.radiance(upper).x > p.sky(upper).x);
    }

    #[test]
    fn the_lighting_bake_holds_the_sun_and_the_visible_image_does_not() {
        let (_, p) = sky(25.0, 0.53);
        let omega = p.sun_solid_angle();
        for width in [128, 512] {
            let lit = p.equirect(width);
            let seen = p.visible(width);
            let (w, h) = (lit.width, lit.height);
            let mut extra = Vec3::ZERO;
            for y in 0..h {
                let v = (y as f32 + 0.5) / h as f32;
                let sa = (2.0 * std::f32::consts::PI / w as f32)
                    * (std::f32::consts::PI / h as f32)
                    * (v * std::f32::consts::PI).sin();
                for x in 0..w {
                    let i = (y * w + x) as usize;
                    let d = dir_of((x as f32 + 0.5) / w as f32, v);
                    assert!((Vec3::from(seen.rgb[i]) - p.sky(d)).abs().max_element() < 1e-6);
                    extra += (Vec3::from(lit.rgb[i]) - Vec3::from(seen.rgb[i])) * sa;
                }
            }
            let want = p.sun() * omega;
            assert!((extra - want).abs().max_element() < 1e-3 * want.max_element(), "{width}: {extra} {want}");
        }
    }

    #[test]
    fn the_uniforms_name_the_physical_mode_the_disc_and_the_horizon() {
        let (_, p) = sky(25.0, 0.53);
        let u = p.uniforms();
        assert_eq!(u[1][3], 2.0);
        let d = scene(25.0, 30.0);
        for c in 0..3 {
            assert!((u[3][c] as f64 - d[c]).abs() < 1e-6);
            assert_eq!(u[4][c], p.sun()[c]);
        }
        assert_eq!(u[3][3], (0.53f32.to_radians() / 2.0).cos());
        assert_eq!(u[4][3], 0.0, "the ground observer's horizon is level");
        // the moon's slot draws nothing
        assert_eq!(u[2][..3], [0.0; 3]);
        assert!(u[0][3] > 1.0);
    }

    #[test]
    fn the_tint_colours_the_disc_alone_and_a_bad_size_is_refused() {
        let s = earth();
        let white = PhysicalSky::new(&s, 0.0, scene(40.0, 0.0), 0.53, [1.0; 3]).unwrap();
        let red = PhysicalSky::new(&s, 0.0, scene(40.0, 0.0), 0.53, [1.0, 0.5, 0.0]).unwrap();
        assert_eq!(red.sun(), white.sun() * Vec3::new(1.0, 0.5, 0.0));
        let d = Vec3::from(scene(70.0, 90.0).map(|c| c as f32));
        assert_eq!(red.sky(d), white.sky(d));
        for size in [-0.1, f64::NAN, f64::INFINITY] {
            assert!(PhysicalSky::new(&s, 0.0, scene(40.0, 0.0), size, [1.0; 3]).is_err(), "{size}");
        }
    }

    #[test]
    fn from_a_height_the_horizon_dips_and_the_disc_follows_it() {
        // 30 km up the geometric horizon is 5.56° down: a sun 3° down is above it and drawn
        let s = earth();
        let h = 30000.0;
        let p = PhysicalSky::new(&s, h, scene(-3.0, 0.0), 0.53, [1.0; 3]).unwrap();
        let dip = (R / (R + h)).acos();
        assert!((p.uniforms()[4][3] as f64 + dip.sin()).abs() < 1e-6, "{}", p.uniforms()[4][3]);
        assert!(p.has_sun());
        let d = Vec3::from(scene(-3.0, 0.0).map(|c| c as f32));
        assert!(p.radiance(d).x > p.sky(d).x);
        let under = Vec3::from(scene(-6.0, 0.0).map(|c| c as f32));
        assert_eq!(p.radiance(under), Vec3::ZERO);
    }
}

mod document {
    use super::*;
    use glam::Vec3;
    use sr_3d::atmosphere::{disc_solid_angle, effective_sun_irradiance};
    use sr_3d::physical_sky::{atmosphere_options, PhysicalSky, PhysicalSkyOptions};
    use sr_3d::space::moon::moon_from_document;
    use sr_3d::space::sun::{sun_from_document, Observer};

    fn equator() -> Observer {
        Observer { latitude: 0.0, longitude: 0.0, altitude: 0.0, pressure: 1013.25, temperature: 10.0, delta_t: None }
    }

    fn options(date: &str) -> PhysicalSkyOptions {
        PhysicalSkyOptions {
            date: date.into(),
            observer: equator(),
            atmosphere: atmosphere_options(true, 1.0, 1.0, 6360000.0, 100000.0, 0.3, [1.0; 3]),
            sun_size: 0.53,
            sun_color: [1.0; 3],
            moon: true,
            moon_intensity: 1.0,
        }
    }

    const FULL: &str = "2003-11-09T01:13:00Z";

    #[test]
    fn the_sun_comes_from_the_date() {
        // the report's example as a dome light: the disc is centred on rule 6's direction (the kit's sky-spa case)
        let golden = Observer {
            latitude: 39.742476,
            longitude: -105.1786,
            altitude: 1830.14,
            pressure: 820.0,
            temperature: 11.0,
            delta_t: Some(67.0),
        };
        let o = PhysicalSkyOptions { observer: golden, ..options("2003-10-17T12:30:30-07:00") };
        let p = PhysicalSky::from_document(&o).unwrap();
        let (_, d) = sun_from_document("2003-10-17T12:30:30-07:00", &golden).unwrap();
        let u = p.uniforms();
        for c in 0..3 {
            assert!((u[3][c] as f64 - d[c]).abs() < 1e-6, "{u:?} {d:?}");
        }
        assert!(p.has_sun());
        let centre = Vec3::from(d.map(|c| c as f32));
        assert!(p.radiance(centre).x > 100.0 * p.sky(centre).x);
    }

    #[test]
    fn the_document_gives_its_sky_air_and_sun_for_the_aerial_perspective() {
        // the parts are the sky of `from_document`, its scattering and the sun of rule 6 in the scene's axes
        let o = PhysicalSkyOptions {
            atmosphere: atmosphere_options(true, 2.0, 1.0, 6360000.0, 100000.0, 0.3, [1.0; 3]),
            ..options(FULL)
        };
        let (p, s, sun) = PhysicalSky::from_document_parts(&o).unwrap();
        assert_eq!(p, PhysicalSky::from_document(&o).unwrap());
        assert_eq!(sun, sun_from_document(FULL, &o.observer).unwrap().1);
        let air = Scattering::new(atmosphere(&o.atmosphere).unwrap());
        let (obs, d) = ([0.0, 0.0, air.atmosphere.planet_radius + 10.0], unit([1.0, 0.2, 0.1]).unwrap());
        assert_eq!(s.sky_radiance(obs, d, to_planet(sun)).unwrap(), air.sky_radiance(obs, d, to_planet(sun)).unwrap());
        let bad = PhysicalSkyOptions { date: "not a date".into(), ..options(FULL) };
        assert!(PhysicalSky::from_document_parts(&bad).is_err());
    }

    #[test]
    fn the_full_moon_is_drawn_at_its_place_size_and_radiance() {
        let a = sr_3d::atmosphere::atmosphere(&AtmosphereOptions::default()).unwrap();
        let p = PhysicalSky::from_document(&options(FULL)).unwrap();
        let (moon, d) = moon_from_document(FULL, &equator()).unwrap();
        let want = a.moon_radiance([0.0, 0.0, R], &moon, 1.0).unwrap();
        let u = p.uniforms();
        for c in 0..3 {
            assert!((u[0][c] as f64 - d[c]).abs() < 1e-6);
            assert!((u[2][c] as f64 - want[c]).abs() < 1e-6 * want[c], "{u:?} {want:?}");
        }
        assert_eq!(u[0][3], (moon.angular_diameter.to_radians() as f32 / 2.0).cos());
        assert!((moon.angular_diameter - 0.5).abs() < 0.02, "{}", moon.angular_diameter);
        let centre = Vec3::from(d.map(|c| c as f32));
        let disc = p.radiance(centre) - p.sky(centre);
        assert!((disc - Vec3::from(want.map(|c| c as f32))).abs().max_element() < 1e-6);
        // the lighting bake holds the moon's power as well as the sun's
        let lit = p.equirect(256);
        let seen = p.visible(256);
        let mut extra = Vec3::ZERO;
        let (w, h) = (lit.width, lit.height);
        for y in 0..h {
            let v = (y as f32 + 0.5) / h as f32;
            let sa = (2.0 * std::f32::consts::PI / w as f32)
                * (std::f32::consts::PI / h as f32)
                * (v * std::f32::consts::PI).sin();
            for x in 0..w {
                let i = (y * w + x) as usize;
                extra += (Vec3::from(lit.rgb[i]) - Vec3::from(seen.rgb[i])) * sa;
            }
        }
        let sun = p.sun() * p.sun_solid_angle();
        let moon_power = Vec3::from(want.map(|c| c as f32)) * disc_solid_angle(moon.angular_diameter).unwrap() as f32;
        assert!((extra - sun - moon_power).abs().max_element() < 1e-3 * (sun + moon_power).max_element());
    }

    #[test]
    fn no_moon_new_moon_and_a_dark_sun() {
        let u = PhysicalSky::from_document(&PhysicalSkyOptions { moon: false, ..options(FULL) }).unwrap().uniforms();
        assert_eq!(u[2][..3], [0.0; 3]);
        assert!(u[0][3] > 1.0);
        let new = PhysicalSky::from_document(&options("2003-10-25T12:50:00Z")).unwrap().uniforms();
        // the draft's row: zero to its six digits
        assert!(new[2][..3].iter().all(|c| *c < 5e-7), "{new:?}");
        // sunIntensity="0": a black sky and a dark moon
        let dark = atmosphere_options(
            true,
            1.0,
            1.0,
            6360000.0,
            100000.0,
            0.3,
            effective_sun_irradiance([1.0; 3], Some(0.0)).unwrap(),
        );
        let p = PhysicalSky::from_document(&PhysicalSkyOptions { atmosphere: dark, ..options(FULL) }).unwrap();
        assert!(!p.has_sun());
        assert_eq!(p.uniforms()[2][..3], [0.0; 3]);
        assert_eq!(p.sky(Vec3::NEG_Y), Vec3::ZERO);
        // moonIntensity scales the moon alone
        let twice = PhysicalSky::from_document(&PhysicalSkyOptions { moon_intensity: 2.0, ..options(FULL) }).unwrap();
        let once = PhysicalSky::from_document(&options(FULL)).unwrap();
        assert_eq!(twice.uniforms()[2][0], 2.0 * once.uniforms()[2][0]);
        assert_eq!(twice.sun(), once.sun());
    }

    #[test]
    fn atmosphere_none_is_a_black_sky_and_an_untouched_sun() {
        let none = atmosphere_options(false, 1.0, 1.0, 6360000.0, 100000.0, 0.3, [1.0; 3]);
        assert_eq!((none.rayleigh, none.aerosol, none.ozone), (0.0, 0.0, 0.0));
        let earth = atmosphere_options(true, 2.0, 0.5, 3389500.0, 11100.0, 0.1, [1.0, 0.5, 0.25]);
        assert_eq!((earth.rayleigh, earth.aerosol, earth.ozone, earth.top_radius), (1.0, 2.0, 0.5, 3400600.0));
        assert_eq!(
            (earth.planet_radius, earth.ground_albedo, earth.sun_irradiance),
            (3389500.0, 0.1, [1.0, 0.5, 0.25])
        );
        let p = PhysicalSky::from_document(&PhysicalSkyOptions { atmosphere: none, ..options("2003-10-17T12:00:00Z") })
            .unwrap();
        assert_eq!(p.sky(Vec3::NEG_Y), Vec3::ZERO);
        let omega = disc_solid_angle(0.53).unwrap() as f32;
        assert!((p.sun() * omega - Vec3::ONE).abs().max_element() < 1e-5, "{}", p.sun());
    }

    #[test]
    fn the_rules_refusals_are_errors() {
        assert!(PhysicalSky::from_document(&options("2003-10-17T12:30")).is_err());
        assert!(PhysicalSky::from_document(&options("1850-01-01T00:00:00Z")).is_err());
        let far = PhysicalSkyOptions { observer: Observer { latitude: 91.0, ..equator() }, ..options(FULL) };
        assert!(PhysicalSky::from_document(&far).is_err());
        assert!(PhysicalSky::from_document(&PhysicalSkyOptions { moon_intensity: -1.0, ..options(FULL) }).is_err());
        assert!(
            PhysicalSky::from_document(&PhysicalSkyOptions { sun_color: [2.0, 0.0, 0.0], ..options(FULL) }).is_err()
        );
        assert!(PhysicalSky::from_document(&PhysicalSkyOptions { sun_size: 181.0, ..options(FULL) }).is_err());
    }
}

/// A sky's discs light from their own directions: the bake's irradiance holds each as a point of its power, which
/// the planets draft's cloud and ring shadows under a physical sky's sun take back out, while its mips hold each
/// spread over the texels it covers. The rest of the sky is projected from the bake's small copy, whose texel would
/// carry a disc up to half its width away.
#[test]
fn a_disc_lights_the_bake_as_a_point_of_its_power() {
    use glam::Vec3;
    use sr_3d::env::{dir_of, prefilter, prefilter_discs, sh_eval, sh_point, Disc, Equirect};
    let (radiance, cos) = (Vec3::new(3.0e4, 2.5e4, 2.0e4), 0.265f32.to_radians().cos());
    let power = radiance * (2.0 * std::f32::consts::PI * (1.0 - cos));
    let sky = Equirect {
        width: 512,
        height: 256,
        rgb: (0..512 * 256).map(|k| [0.1 + (k % 512) as f32 / 5120.0, 0.2, 0.3 + (k / 512) as f32 / 2560.0]).collect(),
    };
    let alone = prefilter(&sky, 512, 7);
    let texel_sa = |y: u32| {
        (2.0 * std::f32::consts::PI / 512.0)
            * (std::f32::consts::PI / 256.0)
            * ((y as f32 + 0.5) / 256.0 * std::f32::consts::PI).sin()
    };
    for dir in [Vec3::new(0.3, -0.8, 0.52), Vec3::new(-0.9, -0.1, 0.42), Vec3::new(0.0, -1.0, 0.0)] {
        let dir = dir.normalize();
        let disc = Disc { dir, cos, radiance };
        assert!((disc.power() - power).abs().max_element() < 1e-3 * power.x, "{}", disc.power());
        // the engine's bake: 512 wide, prefiltered at 512 over 7 levels
        let pf = prefilter_discs(&sky, 512, 7, &[disc]);
        let scale = power.x / std::f32::consts::PI;
        for k in 0..400 {
            let n = dir_of((k % 20) as f32 / 20.0 + 0.025, (k / 20) as f32 / 20.0 + 0.025);
            let (got, want) = (sh_eval(&pf.sh, n), sh_eval(&alone.sh, n) + sh_point(power, dir, n));
            assert!((got - want).abs().max_element() < 1e-4 * scale, "{dir} at {n}: {got}, want {want}");
        }
        // the sharpest mip holds the disc's power about its direction
        let (w, h, px) = &pf.mips[0];
        let (mut held, mut off) = (Vec3::ZERO, 0.0f32);
        for (i, (p, q)) in px.iter().zip(&alone.mips[0].2).enumerate() {
            let (x, y) = (i as u32 % w, i as u32 / w);
            let add = (Vec3::new(p[0], p[1], p[2]) - Vec3::new(q[0], q[1], q[2])) * texel_sa(y);
            if add.x > 1e-3 * power.x {
                held += add;
                off = off.max(dir_of((x as f32 + 0.5) / *w as f32, (y as f32 + 0.5) / *h as f32).angle_between(dir));
            }
        }
        assert!((held.x / power.x - 1.0).abs() < 0.02 && off < 0.03, "{dir}: {held} of {power}, {off} rad off");
        // the projection, not Lambert's cosine: a sixteenth of P / π stays opposite the disc
        let back = sh_point(power, dir, -dir).x / scale;
        assert!((back - 0.0625).abs() < 1e-4, "{back}");
    }
}
