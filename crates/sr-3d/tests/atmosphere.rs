//! Rules 8, 9 and 11 of the physical-sky draft (sr-core srep/srep-0000-physical-sky.md): its numeric rows against
//! `sr_3d::atmosphere`, the reference module's values on rays the rows do not reach (oblique, from altitude and
//! from orbit, over a length, with the aerosol and ozone knobs), and the failure paths of the rules.

use sr_3d::atmosphere::{
    atmosphere, hg_phase, integrate_phase, rayleigh_phase, unit, Atmosphere, AtmosphereOptions, Fog, Scattering,
    SkyError, MAX_SAMPLES,
};

type V3 = [f64; 3];

fn earth() -> Atmosphere {
    atmosphere(&AtmosphereOptions::default()).unwrap()
}

fn ground(a: &Atmosphere) -> V3 {
    [0.0, 0.0, a.planet_radius + 1.0]
}

fn round(x: f64, digits: i32) -> f64 {
    let s = 10f64.powi(digits);
    (x * s).round() / s
}

/// Equal to the reference module's value to rounding: the sums run in its order.
fn close(got: V3, want: V3, what: &str) {
    for c in 0..3 {
        assert!((got[c] - want[c]).abs() <= 1e-9 * want[c].abs().max(1e-6), "{what}: {got:?}, the reference {want:?}");
    }
}

#[test]
fn the_ozone_tent_and_the_exponential_layers() {
    let a = earth();
    assert_eq!((a.ozone_density(25000.0), a.ozone_density(17500.0), a.ozone_density(40000.0)), (1.0, 0.5, 0.0));
    assert_eq!(a.rayleigh_density(0.0), 1.0);
    assert_eq!(a.mie_density(1200.0), (-1.0f64).exp());
    // zero above the top and below the ground
    assert_eq!((a.rayleigh_density(100000.1), a.mie_density(-0.1)), (0.0, 0.0));
}

#[test]
fn the_vertical_transmittance_row_and_its_closed_form() {
    let a = earth();
    let t = a.transmittance([0.0, 0.0, a.planet_radius], [0.0, 0.0, 1.0]).unwrap();
    assert_eq!(t.map(|x| round(x, 6)), [0.935904, 0.863504, 0.7587]);
    // without ozone the depth is the two layers' integral in closed form
    let a = atmosphere(&AtmosphereOptions { ozone: 0.0, ..Default::default() }).unwrap();
    let t = a.transmittance([0.0, 0.0, a.planet_radius], [0.0, 0.0, 1.0]).unwrap();
    for (c, t) in t.iter().enumerate() {
        let depth = a.rayleigh_scattering[c] * a.rayleigh_scale * (1.0 - (-100000.0 / a.rayleigh_scale).exp())
            + (a.mie_scattering + a.mie_absorption) * a.mie_scale * (1.0 - (-100000.0 / a.mie_scale).exp());
        assert!((t - (-depth).exp()).abs() <= 1e-6);
    }
}

#[test]
fn a_ray_into_the_ground_is_dark() {
    let a = earth();
    assert_eq!(a.transmittance([0.0, 0.0, 6360010.0], unit([1.0, 0.0, -0.1]).unwrap()).unwrap(), [0.0; 3]);
    let s = Scattering::new(a);
    assert_eq!(
        s.sky_radiance(ground(&a), unit([1.0, 0.0, -0.1]).unwrap(), unit([1.0, 0.0, 1.0]).unwrap()).unwrap(),
        [0.0; 3]
    );
}

#[test]
fn transmittance_on_rays_the_rows_do_not_reach() {
    let a = earth();
    let g = ground(&a);
    let orbit = [0.0, 0.0, 6760000.0];
    close(
        a.transmittance(g, unit([1.0, 0.0, 0.2]).unwrap()).unwrap(),
        [0.7214958488082687, 0.48683047608906593, 0.25455330013511235],
        "oblique",
    );
    close(
        a.transmittance([0.0, 0.0, a.planet_radius + 30000.0], unit([1.0, 0.0, -0.05]).unwrap()).unwrap(),
        [0.5562523412561597, 0.20394889556275642, 0.31603983680829056],
        "from 30 km",
    );
    close(
        a.transmittance(orbit, a.limb_direction(orbit, -0.5).unwrap()).unwrap(),
        [0.488465897227088, 0.14685647411414657, 0.19288456356947722],
        "from orbit",
    );
    close(
        a.transmittance_with(g, [1.0, 0.0, 0.0], 64, Some(5000.0), true).unwrap(),
        [0.9315306910518585, 0.8961046112852513, 0.8127043358738745],
        "over 5 km",
    );
    close(
        a.transmittance_with(g, [0.0, 0.0, 1.0], 8, None, false).unwrap(),
        [0.9359644130865359, 0.8635078327423338, 0.7587925782157321],
        "uniform segments",
    );
    let thick = atmosphere(&AtmosphereOptions { aerosol: 2.0, ozone: 0.0, ..Default::default() }).unwrap();
    close(
        thick.transmittance(g, unit([0.3, 0.2, 0.5]).unwrap()).unwrap(),
        [0.9212711615211929, 0.8534748641436335, 0.7039605152411729],
        "aerosol 2, no ozone",
    );
    let none =
        atmosphere(&AtmosphereOptions { aerosol: 0.0, ozone: 0.0, rayleigh: 0.0, ..Default::default() }).unwrap();
    assert_eq!(none.transmittance(g, unit([1.0, 0.0, 0.01]).unwrap()).unwrap(), [1.0; 3]);
}

#[test]
fn the_phase_functions() {
    assert_eq!(round(rayleigh_phase(0.0) * 16.0 * std::f64::consts::PI, 12), 3.0);
    assert_eq!(round(integrate_phase(|mu| hg_phase(mu, 0.8), 2000), 6), 1.0);
    assert!((integrate_phase(|mu| hg_phase(mu, 0.8), 2000) - 1.0000001045090012).abs() < 1e-12);
    assert!((hg_phase(0.3, 0.8) - 0.022930084381033656).abs() < 1e-15);
    assert!((rayleigh_phase(0.3) - 0.06505458298881223).abs() < 1e-15);
}

#[test]
fn the_zenith_radiance_row() {
    let a = earth();
    let s = Scattering::new(a);
    let l = s.sky_radiance(ground(&a), [0.0, 0.0, 1.0], unit([1.0, 0.0, 1.0]).unwrap()).unwrap();
    assert_eq!(l.map(|x| round(x, 6)), [0.008623, 0.025027, 0.085859]);
    close(l, [0.00862335731061662, 0.025027288504962133, 0.0858589393622989], "zenith");
}

#[test]
fn single_scattering_converges_at_32_samples() {
    let a = earth();
    let s = Scattering::new(a);
    let (d, sun) = (unit([0.3, 0.4, 0.6]).unwrap(), unit([1.0, 0.0, 0.5]).unwrap());
    let coarse = s.sky_radiance_with(ground(&a), d, sun, false, 32).unwrap();
    let fine = s.sky_radiance_with(ground(&a), d, sun, false, 512).unwrap();
    close(coarse, [0.005109537472427442, 0.009831028835627996, 0.018952385681255218], "32 samples");
    close(fine, [0.005109537547691062, 0.009831029605246969, 0.0189523857642624], "512 samples");
    assert!((0..3).all(|c| (coarse[c] - fine[c]).abs() <= 1e-3 * fine[c]));
    assert!(fine[2] > fine[0] && fine[0] > 0.0, "the sky is blue: {fine:?}");
}

#[test]
fn sky_radiance_on_rays_the_rows_do_not_reach() {
    let a = earth();
    let s = Scattering::new(a);
    let g = ground(&a);
    let (high, low) = (unit([1.0, 0.0, 1.0]).unwrap(), unit([1.0, 0.0, 0.02]).unwrap());
    close(
        s.sky_radiance(g, unit([1.0, 0.0, 0.01]).unwrap(), low).unwrap(),
        [0.21691022351441852, 0.08336183306368741, 0.06588762764408236],
        "horizon toward a low sun",
    );
    close(
        s.sky_radiance(g, unit([-1.0, 0.0, 0.05]).unwrap(), low).unwrap(),
        [0.03682778082794982, 0.04646918080055505, 0.06814401617963611],
        "away from a low sun",
    );
    close(
        s.sky_radiance([0.0, 0.0, a.planet_radius + 30000.0], unit([0.2, 0.1, 0.3]).unwrap(), high).unwrap(),
        [0.00027074698317727943, 0.0007560765138372243, 0.0028268367963544625],
        "from 30 km",
    );
    let orbit = [0.0, 0.0, 6760000.0];
    close(
        s.sky_radiance(orbit, a.limb_direction(orbit, -0.5).unwrap(), high).unwrap(),
        [0.03587711120739213, 0.06630831298472485, 0.3066255024150057],
        "the shell from orbit",
    );
    assert_eq!(s.sky_radiance(orbit, [1.0, 0.0, 0.0], high).unwrap(), [0.0; 3], "space away from the planet");
    let lit =
        atmosphere(&AtmosphereOptions { sun_irradiance: [2.0, 1.0, 0.5], ground_albedo: 0.8, ..Default::default() })
            .unwrap();
    close(
        Scattering::new(lit).sky_radiance(g, unit([0.1, 0.2, 0.9]).unwrap(), high).unwrap(),
        [0.020356825094549582, 0.028094794797467886, 0.04601973938934305],
        "a coloured sun over a bright ground",
    );
}

#[test]
fn the_multiple_scattering_estimate() {
    let a = earth();
    let f = a.multiple_scattering_factor(1.0, 0.5);
    assert_eq!(f.map(|x| round(x, 4)), [0.073, 0.1338, 0.2306]);
    close(f, [0.07302418896486053, 0.13375168579655117, 0.23059254770249782], "factor");
    let (psi, f) = a.ms_cell(25000.0, -0.1);
    close(psi, [0.001341729195271027, 0.0007206671819406949, 0.002228085282686361], "cell radiance");
    close(f, [0.06834573381444109, 0.12482875638641432, 0.24593434417038673], "cell factor");
}

#[test]
fn the_sky_lights_the_ground_less_than_the_sun() {
    let a = earth();
    let f = a.multiple_scattering_factor(1.0, 0.5);
    let e = Scattering::new(a).sky_irradiance(ground(&a), unit([1.0, 0.0, 0.5]).unwrap()).unwrap();
    assert!(f.iter().all(|x| (0.0..1.0).contains(x)) && f.iter().all(|x| *x > 0.0));
    assert!(e.iter().all(|x| *x > 0.0 && *x < 1.0), "{e:?}");
    close(e, [0.04617385492457451, 0.12261108184404666, 0.3683930346414634], "sky irradiance");
}

#[test]
fn aerial_perspective_runs_from_the_colour_to_the_sky() {
    let a = earth();
    let s = Scattering::new(a);
    let (g, d, sun, colour) = (ground(&a), [1.0, 0.0, 0.0], unit([1.0, 0.0, 0.5]).unwrap(), [0.2, 0.4, 0.1]);
    assert_eq!(s.aerial_perspective(g, d, 0.0, colour, sun).unwrap(), colour);
    let near = s.aerial_perspective(g, d, 1000.0, colour, sun).unwrap();
    let far = s.aerial_perspective(g, d, 50000.0, colour, sun).unwrap();
    close(near, [0.1994465645964712, 0.3954001070957427, 0.1061469047635986], "1 km");
    close(far, [0.18073340870358046, 0.26136785098535303, 0.2326846116393179], "50 km");
    let sky = s.sky_radiance(g, d, sun).unwrap();
    assert!((0..3).all(|c| (far[c] - sky[c]).abs() < (near[c] - sky[c]).abs()));
}

#[test]
fn the_sun_disc_through_the_air() {
    let a = earth();
    let low = unit([1.0, 0.0, 0.02]).unwrap();
    close(
        a.sun_radiance(ground(&a), low).unwrap(),
        [0.20235428418122572, 0.03764878430256392, 0.0012461972485592451],
        "a low sun",
    );
    let high = a.sun_radiance(ground(&a), unit([1.0, 0.0, 1.0]).unwrap()).unwrap();
    // a low sun is redder
    assert!(high[0] / high[2] < 0.20235428418122572 / 0.0012461972485592451);
}

#[test]
fn the_limb_from_orbit() {
    let a = earth();
    let orbit = [0.0, 0.0, 6760000.0];
    assert_eq!(a.limb_direction(orbit, 0.0).unwrap().map(|c| round(c, 6)), [0.940828, 0.0, -0.338883]);
    close(a.limb_direction(orbit, -2.0).unwrap(), [0.9520821330886123, 0.0, -0.30584246247608904], "2 degrees above");
}

#[test]
fn height_fog_in_closed_form() {
    let f = Fog::new(0.001, 100.0).unwrap();
    assert_eq!(round(f.transmittance(0.0, 50.0, 1000.0), 9), 0.455236288);
    assert_eq!(f.transmittance(0.0, 0.0, 1000.0), (-1.0f64).exp());
    assert!(
        (f.transmittance(0.0, 50.0, 1000.0) - (-0.001 * 1000.0 * 2.0 * (1.0 - (-0.5f64).exp())).exp()).abs() <= 1e-12
    );
    assert!((f.transmittance(500.0, 500.0, 1000.0) - 0.9932847020678415).abs() < 1e-15);
    // the kit's wall at 500 m: a depth of 0.001 · 1000 · e^(−5), e^(−0.0067) to the kit's digits
    assert_eq!(round(-f.transmittance(500.0, 500.0, 1000.0).ln(), 4), 0.0067);
    assert!((Fog::new(0.002, 250.0).unwrap().transmittance(400.0, 10.0, 700.0) - 0.5060814016754804).abs() < 1e-15);
    assert_eq!(f.transmittance(0.0, 50.0, 0.0), 1.0);
}

#[test]
fn values_the_rules_refuse() {
    let bad = |o: AtmosphereOptions| matches!(atmosphere(&o), Err(SkyError(_)));
    let d = AtmosphereOptions::default;
    assert!(bad(AtmosphereOptions { planet_radius: 0.0, ..d() }));
    assert!(bad(AtmosphereOptions { top_radius: 6360000.0, ..d() }));
    assert!(bad(AtmosphereOptions { aerosol: -0.1, ..d() }));
    assert!(bad(AtmosphereOptions { ozone: f64::NAN, ..d() }));
    assert!(bad(AtmosphereOptions { ground_albedo: 1.1, ..d() }));
    assert!(bad(AtmosphereOptions { sun_irradiance: [1.0, -1.0, 1.0], ..d() }));
    assert!(bad(AtmosphereOptions { sun_irradiance: [1.0, f64::INFINITY, 1.0], ..d() }));
    let a = earth();
    let below = [0.0, 0.0, a.planet_radius - 1.0];
    assert!(a.transmittance(below, [0.0, 0.0, 1.0]).is_err());
    assert!(Scattering::new(a).sky_radiance(below, [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]).is_err());
    assert!(a.transmittance_with(ground(&a), [0.0, 0.0, 1.0], 0, None, true).is_err());
    assert!(a.transmittance_with(ground(&a), [0.0, 0.0, 1.0], MAX_SAMPLES + 1, None, true).is_err());
    assert!(a.limb_direction(ground(&a), 0.0).is_err(), "the limb is seen from above the atmosphere");
    assert!(unit([0.0; 3]).is_err());
    for (density, height) in [(-1.0, 100.0), (0.001, 0.0), (f64::NAN, 100.0), (0.001, f64::INFINITY)] {
        assert!(Fog::new(density, height).is_err(), "{density} {height}");
    }
}
