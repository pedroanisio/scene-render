//! The numeric table of the fields draft (sr-core srep/srep-0000-fields.md, Conformance), row by row, against
//! `sr_eval::fields`: the sources, the operators, the streamlines, the arrows, the evenly spaced streamlines, line
//! integral convolution, phase portraits, winding numbers and charge lines. Floats are compared to 1e-9 relative,
//! integers, counts and errors exactly, as the draft states. A complex number is `[re, im]`.

use sr_eval::fields::{
    arrow, arrow_grid, charge_field, charge_lines, complex_sampler, curl, dipole_field, dipole_line_constant,
    dipole_line_deviation, divergence, evenly_spaced_streamlines, grid_sampler, inside_box, lic, line_ends_on,
    min_separation, modulus_shade, phase_hue, poynting, rk4_step, source_field, streamline, winding_number, wire_field,
    ArrowStyle, ChargeLines, FieldError, Spacing, Trace,
};

fn close(a: f64, b: f64) -> bool {
    a == b || (a - b).abs() <= 1e-9 * a.abs().max(b.abs())
}

#[track_caller]
fn same(got: [f64; 2], want: [f64; 2]) {
    assert!(close(got[0], want[0]) && close(got[1], want[1]), "{got:?} != {want:?}");
}

#[track_caller]
fn same_points(got: &[[f64; 2]], want: &[[f64; 2]]) {
    assert_eq!(got.len(), want.len(), "{got:?} != {want:?}");
    for (g, w) in got.iter().zip(want) {
        same(*g, *w);
    }
}

fn round(v: f64, d: i32) -> f64 {
    let s = 10f64.powi(d);
    (v * s).round() / s
}

fn mul(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] * b[0] - a[1] * b[1], a[0] * b[1] + a[1] * b[0]]
}

fn div(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    let d = b[0] * b[0] + b[1] * b[1];
    [(a[0] * b[0] + a[1] * b[1]) / d, (a[1] * b[0] - a[0] * b[1]) / d]
}

/// (z − 1)(z + 1) / z.
fn two_zeros_a_pole(z: [f64; 2]) -> [f64; 2] {
    div(mul([z[0] - 1.0, z[1]], [z[0] + 1.0, z[1]]), z)
}

const UNIT: [f64; 4] = [0.0, 0.0, 1.0, 1.0];

fn ramp() -> Vec<Vec<[f64; 2]>> {
    vec![vec![[0.0, 0.0], [1.0, 0.0]], vec![[0.0, 1.0], [1.0, 1.0]]]
}

fn uniform(_: f64, _: f64) -> [f64; 2] {
    [1.0, 0.0]
}

fn rotation(x: f64, y: f64) -> [f64; 2] {
    [-y, x]
}

fn trace(h: f64, max_steps: u64, both: bool) -> Trace {
    Trace { h, max_steps, both, keep_stop: false }
}

fn everywhere(_: [f64; 2]) -> bool {
    true
}

#[test]
fn grid_sources() {
    same(grid_sampler(&ramp(), UNIT).unwrap()(1.0, 0.0), [1.0, 0.0]);
    same(grid_sampler(&ramp(), UNIT).unwrap()(0.5, 0.5), [0.5, 0.5]);
    let ones = vec![vec![[1.0, 1.0]; 2]; 2];
    same(grid_sampler(&ones, UNIT).unwrap()(1.5, 0.5), [0.0, 0.0]);
    assert!(matches!(grid_sampler(&[vec![[0.0; 2]; 2]], UNIT), Err(FieldError::Grid)));
}

#[test]
fn analytic_sources() {
    same(complex_sampler(|z| mul(z, z))(1.0, 1.0), [0.0, 2.0]);
    same(charge_field(&[[0.0, 0.0, 1.0]], [2.0, 0.0]), [0.25, 0.0]);
    same(dipole_field([0.0, 2.0], [0.0, 1.0], [0.0, 0.0]), [0.0, 0.25]);
    same(dipole_field([2.0, 0.0], [0.0, 1.0], [0.0, 0.0]), [0.0, -0.125]);
    let w = wire_field([2.0, 0.0], 1.0, [0.0, 0.0]);
    same([round(w[0], 12) + 0.0, round(w[1], 12)], [0.0, 0.079577471546]);
    same(poynting([1.0, 0.0], 1.0), [0.0, -1.0]);
}

#[test]
fn operators() {
    assert_eq!(round(divergence(|x, y| [x, y], [0.3, 0.7]), 6), 2.0);
    assert_eq!(round(curl(rotation, [0.3, 0.7]), 6), 2.0);
    assert_eq!(round(divergence(|x, y| source_field([x, y], [0.0, 0.0]), [1.0, 0.0]), 6) + 0.0, 0.0);
}

#[test]
fn streamlines() {
    same(rk4_step(|_, _| [3.0, 0.0], [0.0, 0.0], 0.5).unwrap(), [0.5, 0.0]);
    assert_eq!(rk4_step(|_, _| [0.0, 0.0], [0.0, 0.0], 0.5), None);
    let line = streamline(uniform, [0.5, 0.5], &trace(0.25, 100, true), inside_box(UNIT), None).unwrap();
    same_points(&line, &[[0.0, 0.5], [0.25, 0.5], [0.5, 0.5], [0.75, 0.5], [1.0, 0.5]]);
    let circle = streamline(rotation, [1.0, 0.0], &trace(0.01, 100, false), everywhere, None).unwrap();
    let last = circle.last().unwrap();
    assert!((last[0].hypot(last[1]) - 1.0).abs() < 1e-6, "{last:?}");
    assert_eq!(streamline(uniform, [0.0, 0.0], &trace(0.1, 10, true), everywhere, None).unwrap().len(), 21);
    assert!(matches!(streamline(uniform, [0.0, 0.0], &trace(0.0, 10, true), everywhere, None), Err(FieldError::Step)));
}

#[test]
fn arrows() {
    let style = |scale, max_length| ArrowStyle { scale, max_length, ..ArrowStyle::default() };
    let a = arrow([0.0, 0.0], [3.0, 4.0], &style(1.0, 10.0)).unwrap();
    same_points(&a.shaft, &[[0.0, 0.0], [3.0, 4.0]]);
    same_points(&a.head, &[[3.0, 4.0], [1.5, 3.25], [2.7, 2.35]]);
    same(arrow([0.0, 0.0], [30.0, 40.0], &style(1.0, 5.0)).unwrap().shaft[1], [3.0, 4.0]);
    assert_eq!(arrow([0.0, 0.0], [0.0, 0.0], &style(1.0, 5.0)), None);
    assert_eq!(arrow_grid(uniform, UNIT, 3, 2, &style(1.0, 10.0)).unwrap().len(), 6);
    assert!(matches!(arrow_grid(uniform, UNIT, 1, 2, &style(1.0, 10.0)), Err(FieldError::ArrowGrid)));
}

#[test]
fn evenly_spaced() {
    let spacing = |d_sep| Spacing { d_sep, ..Spacing::default() };
    let lines = evenly_spaced_streamlines(uniform, UNIT, &spacing(0.25)).unwrap();
    let mut starts: Vec<f64> = lines.iter().map(|l| round(l[0][1], 9)).collect();
    starts.sort_by(f64::total_cmp);
    assert_eq!(starts, [0.0, 0.25, 0.5, 0.75, 1.0]);
    assert!(min_separation(&lines) >= 0.25 - 1e-9);
    let vortex = evenly_spaced_streamlines(rotation, [-1.0, -1.0, 1.0, 1.0], &spacing(0.2)).unwrap();
    assert!(min_separation(&vortex) >= 0.1 - 0.05 - 1e-9);
    let bounded =
        evenly_spaced_streamlines(rotation, [-1.0, -1.0, 1.0, 1.0], &Spacing { max_lines: 7, ..spacing(0.05) });
    assert_eq!(bounded.unwrap().len(), 7);
    let wide = Spacing { d_test: Some(0.3), ..spacing(0.25) };
    assert!(matches!(evenly_spaced_streamlines(uniform, UNIT, &wide), Err(FieldError::Test)));
}

#[test]
fn line_integral_convolution() {
    let noise = vec![vec![0.2; 4], vec![0.8; 4]];
    let along = lic(uniform, &noise, [0.0, 0.0, 4.0, 2.0], 1.0, 0.5).unwrap();
    let along: Vec<Vec<f64>> = along.iter().map(|r| r.iter().map(|v| round(*v, 12)).collect()).collect();
    assert_eq!(along, noise);
    let across = lic(|_, _| [0.0, 1.0], &noise, [0.0, 0.0, 4.0, 2.0], 1.0, 1.0).unwrap();
    assert_eq!(across, vec![vec![0.5; 4]; 2]);
}

#[test]
fn phase_portraits() {
    assert_eq!(phase_hue([0.0, 1.0]), 0.25);
    assert_eq!(phase_hue([-1.0, 0.0]), 0.5);
    assert_eq!(round(modulus_shade([3.0, 0.0], 2.0), 12), 0.584962500721);
    assert_eq!(winding_number(|z| mul(z, z), [0.0, 0.0], 1.0, 256).unwrap(), 2);
    assert_eq!(winding_number(|z| div([1.0, 0.0], z), [0.0, 0.0], 1.0, 256).unwrap(), -1);
    assert_eq!(winding_number(two_zeros_a_pole, [0.0, 0.0], 2.0, 256).unwrap(), 1);
    assert_eq!(winding_number(two_zeros_a_pole, [0.0, 0.0], 0.5, 256).unwrap(), -1);
    assert!(matches!(winding_number(|z| [z[0] - 1.0, z[1]], [0.0, 0.0], 1.0, 256), Err(FieldError::Winding)));
}

#[test]
fn dipole_and_charge_lines() {
    assert_eq!(round(dipole_line_constant([1.0, 1.0], [0.0, 1.0], [0.0, 0.0]).unwrap(), 12), 2.828427124746);
    assert!(dipole_line_deviation([1.0, 1.0], 0.01, 200, [0.0, 1.0]).unwrap() < 1e-4);
    let one = charge_lines(&[[0.0, 0.0, 1.0]], &ChargeLines { per_charge: 8, ..ChargeLines::default() }).unwrap();
    assert_eq!(one.len(), 8);
    let end = one[2].last().unwrap();
    assert_eq!(round(end[1].atan2(end[0]).to_degrees(), 6), 90.0);
    let pair = [[-1.0, 0.0, 1.0], [1.0, 0.0, -1.0]];
    let lines = charge_lines(&pair, &ChargeLines { per_charge: 8, ..ChargeLines::default() }).unwrap();
    assert_eq!(lines.iter().filter(|l| line_ends_on(l, &pair, 0.05) == Some(1)).count(), 5);
    let wire = |x, y| wire_field([x, y], 1.0, [0.0, 0.0]);
    let circle = streamline(wire, [1.0, 0.0], &trace(0.01, 700, false), everywhere, None).unwrap();
    assert!(circle.iter().all(|p| (p[0].hypot(p[1]) - 1.0).abs() < 1e-6));
}

/// Rule 6's defaults: `test` is half the separation and `step` a quarter of it.
#[test]
fn the_test_distance_and_step_default_to_a_half_and_a_quarter_of_the_separation() {
    let converging = |_: f64, y: f64| [1.0, -y];
    let ranges = [-1.0, -1.0, 1.0, 1.0];
    let implied = evenly_spaced_streamlines(converging, ranges, &Spacing { d_sep: 0.2, ..Spacing::default() });
    let stated = Spacing { d_sep: 0.2, d_test: Some(0.1), h: Some(0.05), ..Spacing::default() };
    assert_eq!(implied.unwrap(), evenly_spaced_streamlines(converging, ranges, &stated).unwrap());
}

/// Rule 7's kernel: samples at arc length ±h, ±2h, … up to `length`, so length 1 and step 1 is one sample each way.
#[test]
fn the_lic_kernel_reaches_its_length_and_no_further() {
    let noise = vec![vec![0.0, 1.0, 0.0, 0.0, 0.0]];
    let out = lic(uniform, &noise, [0.0, 0.0, 5.0, 1.0], 1.0, 1.0).unwrap();
    assert!(close(out[0][2], 1.0 / 3.0), "{out:?}");
}
