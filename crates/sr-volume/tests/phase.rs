//! The phase functions of the media draft (schema/UPSTREAM), against the draft's numeric-test table: closed forms to
//! 1e-9 relative, normalisation and mean cosine by quadrature, and the sampling inversions against their cumulatives.

use sr_volume::medium::{
    cdf_draine, fog_parameters, mean_cosine_draine, phase_draine, phase_fog, phase_hg, phase_rayleigh, sample_draine,
    sample_fog, sample_hg, Optical, Phase,
};

fn close(got: f64, want: f64) {
    assert!((got - want).abs() <= 1e-9 * want.abs().max(1e-300), "{got} != {want}");
}

#[test]
fn the_numeric_table() {
    close(phase_hg(1.0, 0.5), 0.477464829275686);
    close(phase_hg(-0.3, -0.7), 0.03666778241049058);
    close(phase_hg(0.2, 0.0), 0.07957747154594767);
    close(phase_rayleigh(0.0), 0.05968310365946075);
    close(phase_rayleigh(1.0), 0.1193662073189215);
    close(phase_draine(0.25, 0.5, 2.0), 0.03357174580844667);
    close(phase_draine(0.0, 0.76, 1.0), 0.009871756516322017);
    close(phase_draine(0.5, 0.0, 1.0), 0.07460387957432595);
    close(mean_cosine_draine(0.7, 1.5), 0.7717587939698493);
    close(cdf_draine(0.0, 0.7, 1.5), 0.05850844468502295);
    close(cdf_draine(0.5, 0.0, 1.0), 0.703125);
    close(sample_hg(0.5, 0.5), 0.6875);
    close(sample_hg(0.25, 0.0), 0.5);
    close(sample_draine(0.5, 0.7, 1.5), 0.916240041954955);
    close(sample_draine(0.01, -0.6, 3.0), -0.9986281066556024);
    for (d, want) in [
        (5.0, [0.9706779637913735, 0.5083416163246844, 17.06809849400308, 0.44782948956417074]),
        (10.0, [0.988176691700256, 0.5556712547839497, 21.995520856274638, 0.4819554318404214]),
        (50.0, [0.997952443482353, 0.6250844703668166, 32.315427791972155, 0.5076183244841661]),
    ] {
        let p = fog_parameters(d);
        for (got, want) in [p.g_hg, p.g_d, p.alpha, p.w_d].into_iter().zip(want) {
            close(got, want);
        }
    }
    close(phase_fog(1.0, 10.0), 586.8597367386202);
    close(phase_fog(0.0, 10.0), 0.0017256316450645836);
    close(phase_fog(-1.0, 20.0), 0.010913951791898586);
    close(sample_fog(0.1, 0.5, 10.0), 0.8855784590725071);
    close(sample_fog(0.9, 0.5, 10.0), 0.9997911404650676);
}

/// 2π ∫ p dμ over [−1, 1], and the same with μ, by the midpoint rule in x with μ = 1 − 2x⁴, which crowds the nodes
/// at μ = 1, where a fog's forward peak is a few 1e-6 wide.
fn moments(p: impl Fn(f64) -> f64) -> (f64, f64) {
    let n = 400_000;
    let dx = 1.0 / n as f64;
    let (mut zeroth, mut first) = (0.0, 0.0);
    for i in 0..n {
        let x = (i as f64 + 0.5) * dx;
        let mu = 1.0 - 2.0 * x.powi(4);
        let w = p(mu) * 8.0 * x.powi(3) * dx;
        zeroth += w;
        first += w * mu;
    }
    let tau = std::f64::consts::TAU;
    (tau * zeroth, tau * first)
}

#[test]
fn every_phase_is_normalised_and_has_its_mean_cosine() {
    for phase in [
        Phase::Hg { g: 0.6 },
        Phase::Hg { g: -0.3 },
        Phase::Rayleigh,
        Phase::Draine { g: 0.5, alpha: 2.0 },
        Phase::Draine { g: -0.4, alpha: 0.0 },
        Phase::Fog { diameter: 5.0 },
        Phase::Fog { diameter: 50.0 },
    ] {
        let (zeroth, first) = moments(|mu| phase.eval(mu));
        assert!((zeroth - 1.0).abs() < 1e-4, "{phase:?}: {zeroth}");
        let want = match phase {
            Phase::Hg { g } => g,
            Phase::Rayleigh => 0.0,
            Phase::Draine { g, alpha } => mean_cosine_draine(g, alpha),
            Phase::Fog { diameter } => {
                let p = fog_parameters(diameter);
                (1.0 - p.w_d) * p.g_hg + p.w_d * mean_cosine_draine(p.g_d, p.alpha)
            }
        };
        assert!((first - want).abs() < 1e-3, "{phase:?}: {first} != {want}");
    }
}

#[test]
fn rayleigh_and_hg_are_draine_cases() {
    for mu in [-1.0, -0.4, 0.0, 0.3, 1.0] {
        close(phase_draine(mu, 0.0, 1.0), phase_rayleigh(mu));
        close(phase_draine(mu, 0.45, 0.0), phase_hg(mu, 0.45));
    }
    // Rayleigh scatters 3/4 as much sideways as HG's isotropic case
    close(phase_rayleigh(0.0) / phase_hg(0.0, 0.0), 0.75);
}

#[test]
fn the_draine_cumulative_spans_zero_to_one_and_its_inverse_returns_the_draw() {
    for (g, alpha) in [(0.7, 1.5), (-0.6, 3.0), (0.0, 1.0), (0.3, 0.0), (5e-7, 2.0)] {
        assert!(cdf_draine(-1.0, g, alpha).abs() < 1e-12, "{g} {alpha}");
        assert!((cdf_draine(1.0, g, alpha) - 1.0).abs() < 1e-9, "{g} {alpha}");
        for u in [0.0, 0.01, 0.25, 0.5, 0.9, 0.999] {
            let mu = sample_draine(u, g, alpha);
            assert!((-1.0..=1.0).contains(&mu));
            assert!((cdf_draine(mu, g, alpha) - u).abs() < 1e-9, "{g} {alpha} {u}: {mu}");
        }
    }
    // near g = 0 the cumulative tends to its g = 0 value, where the draft's antiderivative in t loses every digit
    for g in [1.1e-6, 2e-6, 1e-5, 1e-4, -1e-5] {
        assert!((cdf_draine(1.0, g, 1.0) - 1.0).abs() < 1e-12, "{g}");
        assert!((cdf_draine(0.3, g, 1.0) - cdf_draine(0.3, 0.0, 1.0)).abs() < 2.0 * g.abs(), "{g}");
        assert!(sample_draine(0.5, g, 1.0).abs() < 1e-3, "{g}");
    }
}

#[test]
fn a_phase_samples_its_own_density() {
    // a histogram of 64 bins of μ from evenly spread draws matches the integral of p over each bin
    for phase in
        [Phase::Hg { g: 0.5 }, Phase::Rayleigh, Phase::Draine { g: -0.3, alpha: 2.0 }, Phase::Fog { diameter: 10.0 }]
    {
        let bins = 64;
        let mut count = vec![0.0; bins];
        let (n, lobes) = (4096, 64);
        for i in 0..n {
            for j in 0..lobes {
                let mu = phase.sample((j as f64 + 0.5) / lobes as f64, (i as f64 + 0.5) / n as f64);
                count[(((mu + 1.0) / 2.0 * bins as f64) as usize).min(bins - 1)] += 1.0 / (n * lobes) as f64;
            }
        }
        for (b, got) in count.iter().enumerate() {
            let (lo, hi) = (-1.0 + 2.0 * b as f64 / bins as f64, -1.0 + 2.0 * (b + 1) as f64 / bins as f64);
            let steps = 2000;
            let want: f64 = (0..steps)
                .map(|k| phase.eval(lo + (hi - lo) * (k as f64 + 0.5) / steps as f64) * (hi - lo) / steps as f64)
                .sum::<f64>()
                * std::f64::consts::TAU;
            assert!((got - want).abs() < 2e-3 + 0.01 * want, "{phase:?} bin {b}: {got} != {want}");
        }
    }
}

#[test]
fn optical_validates_the_phase_parameters() {
    let ok = |phase| Optical { phase, ..Default::default() }.validate().is_ok();
    assert!(ok(Phase::Rayleigh));
    assert!(ok(Phase::Draine { g: 0.9, alpha: 0.0 }));
    assert!(ok(Phase::Fog { diameter: 5.0 }) && ok(Phase::Fog { diameter: 50.0 }));
    for bad in [
        Phase::Hg { g: 1.0 },
        Phase::Hg { g: f64::NAN },
        Phase::Draine { g: 0.5, alpha: -0.1 },
        Phase::Draine { g: -1.0, alpha: 1.0 },
        Phase::Draine { g: 0.5, alpha: f64::INFINITY },
        Phase::Fog { diameter: 4.99 },
        Phase::Fog { diameter: 50.01 },
        Phase::Fog { diameter: f64::NAN },
    ] {
        assert!(!ok(bad), "{bad:?}");
    }
    assert_eq!(Optical::default().phase, Phase::Hg { g: 0.0 });
}
