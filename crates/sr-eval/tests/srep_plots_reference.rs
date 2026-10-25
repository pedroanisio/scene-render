//! The numeric table of the plots draft (sr-core srep/srep-0000-plots.md, Conformance), row by row, against
//! `sr_eval::plots`: the mapping, nice steps, ticks, labels, sampling, clipping, marching squares, the 3D mapping
//! and the tube frames. Floats are compared to 1e-9 relative, strings and counts exactly, as the draft states.

use std::f64::consts::PI;

use sr_eval::plots::{
    axis_position, chord_error_px, clip_polyline, from_pixel, label_decimals, marching_squares, nice_step,
    sample_explicit, sample_parametric, sample_polar, split_finite, tick_label, ticks, to_pixel, to_scene, tube,
    tube_frames, Axes, Axes3D, PlotError,
};

fn close(a: f64, b: f64) -> bool {
    a == b || (a - b).abs() <= 1e-9 * a.abs().max(b.abs())
}

#[track_caller]
fn same(got: &[f64], want: &[f64]) {
    assert!(got.len() == want.len() && got.iter().zip(want).all(|(a, b)| close(*a, *b)), "{got:?} != {want:?}");
}

#[track_caller]
fn same_points(got: &[[f64; 2]], want: &[[f64; 2]]) {
    same(&got.concat(), &want.concat());
}

#[track_caller]
fn same_lines(got: &[Vec<[f64; 2]>], want: &[&[[f64; 2]]]) {
    assert_eq!(got.len(), want.len(), "{got:?}");
    for (g, w) in got.iter().zip(want) {
        same_points(g, w);
    }
}

fn round(v: f64, d: i32) -> f64 {
    let s = 10f64.powi(d);
    (v * s).round() / s
}

fn ax() -> Axes {
    Axes::new(-1.0, 1.0, -1.0, 1.0, 400.0, 300.0).unwrap()
}

fn circle() -> Vec<Vec<[f64; 2]>> {
    marching_squares(|x, y| x * x + y * y - 1.0, [-1.2, 1.2, -1.2, 1.2], 64, 0.0).unwrap()
}

#[test]
fn nice_steps() {
    for (lo, hi, want) in
        [(0.0, 10.0, 1.0), (0.0, 1.0, 0.1), (-3.0, 3.0, 1.0), (0.0, 25.0, 5.0), (0.0, 7.0, 1.0), (0.5, 0.9, 0.05)]
    {
        let got = nice_step(lo, hi).unwrap();
        assert!(close(got, want), "nice_step({lo}, {hi}) = {got}, want {want}");
    }
}

#[test]
fn tick_values() {
    same(&ticks(-1.0, 1.0, 0.5).unwrap(), &[-1.0, -0.5, 0.0, 0.5, 1.0]);
    same(&ticks(0.1, 0.9, 0.25).unwrap(), &[0.25, 0.5, 0.75]);
    let edge = ticks(0.0, 0.3, 0.1).unwrap();
    same(&edge, &[0.0, 0.1, 0.2, 0.30000000000000004]);
    assert_eq!(edge[3], 3.0 * 0.1, "a tick is k × step, never accumulated");
}

#[test]
fn tick_labels() {
    let d: Vec<u32> = [1.0, 0.5, 0.25, 0.001].iter().map(|s| label_decimals(*s).unwrap()).collect();
    assert_eq!(d, [0, 1, 2, 3]);
    assert_eq!(tick_label(1.0, 0.5, false).unwrap(), "1");
    assert_eq!(tick_label(-0.5, 0.5, false).unwrap(), "-0.5");
    assert_eq!(tick_label(0.75, 0.25, false).unwrap(), "0.75");
    assert_eq!(tick_label(2.0, 1.0, true).unwrap(), "2i");
    assert_eq!(tick_label(-1.0, 1.0, true).unwrap(), "-i");
    assert_eq!(tick_label(0.0, 1.0, true).unwrap(), "0");
    assert_eq!(tick_label(-0.0, 0.5, false).unwrap(), "0");
    // the third tick of 0.1 is 0.30000000000000004 and is labelled by the step's decimals
    assert_eq!(tick_label(3.0 * 0.1, 0.1, false).unwrap(), "0.3");
}

#[test]
fn axis_positions() {
    assert_eq!(axis_position(-1.0, 1.0), 0.0);
    assert_eq!(axis_position(2.0, 5.0), 2.0);
}

#[test]
fn the_mapping_and_its_inverse() {
    same(&to_pixel(&ax(), 0.5, 0.5), &[300.0, 75.0]);
    same(&from_pixel(&ax(), 300.0, 75.0), &[0.5, 0.5]);
}

#[test]
fn sampled_curves() {
    same_points(
        &sample_explicit(|x| x * x, 0.0, 1.0, 5).unwrap(),
        &[[0.0, 0.0], [0.25, 0.0625], [0.5, 0.25], [0.75, 0.5625], [1.0, 1.0]],
    );
    let c: Vec<[f64; 2]> = sample_parametric(f64::cos, f64::sin, 0.0, 2.0 * PI, 4, true)
        .unwrap()
        .iter()
        .map(|p| [round(p[0], 12), round(p[1], 12)])
        .collect();
    same_points(&c, &[[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [-0.0, -1.0]]);
    let p: Vec<[f64; 2]> =
        sample_polar(|_| 1.0, 0.0, PI, 3).unwrap().iter().map(|p| [round(p[0], 12), round(p[1], 12)]).collect();
    same_points(&p, &[[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0]]);
    let pole = sample_explicit(|x| if x != 0.0 { 1.0 / x } else { f64::INFINITY }, -1.0, 1.0, 5).unwrap();
    same_lines(&split_finite(&pole), &[&[[-1.0, -1.0], [-0.5, -2.0]], &[[0.5, 2.0], [1.0, 1.0]]]);
}

#[test]
fn clipping() {
    same_lines(&clip_polyline(&[[-1.0, 0.5], [3.0, 0.5]], 0.0, 0.0, 2.0, 1.0), &[&[[0.0, 0.5], [2.0, 0.5]]]);
    same_lines(
        &clip_polyline(&[[0.5, -1.0], [0.5, 0.5], [0.5, 2.0], [1.5, 2.0], [1.5, 0.5]], 0.0, 0.0, 2.0, 1.0),
        &[&[[0.5, 0.0], [0.5, 0.5], [0.5, 1.0]], &[[1.5, 1.0], [1.5, 0.5]]],
    );
}

#[test]
fn marching_squares_rows() {
    same_lines(
        &marching_squares(|x, y| x + y - 1.5, [0.0, 2.0, 0.0, 2.0], 2, 0.0).unwrap(),
        &[&[[1.5, 0.0], [1.0, 0.5], [0.5, 1.0], [0.0, 1.5]]],
    );
    let c = circle();
    assert_eq!((c.len(), c[0].len(), c[0][0] == c[0][c[0].len() - 1]), (1, 213, true));
    let err = c[0].iter().map(|p| (p[0].hypot(p[1]) - 1.0).abs()).fold(0.0, f64::max) * 300.0 / 2.4;
    assert!(close(round(err, 6), 0.02197), "{err}");
    assert_eq!(marching_squares(|x, y| x * y - 0.1, [-1.0, 1.0, -1.0, 1.0], 1, 0.0).unwrap().len(), 2);
}

#[test]
fn chord_error_of_a_sine() {
    let a = Axes::new(-PI, PI, -1.5, 1.5, 500.0, 300.0).unwrap();
    assert!(close(round(chord_error_px(&a, f64::sin, 256).unwrap(), 6), 0.007589));
}

#[test]
fn the_3d_mapping_and_tubes() {
    let a = Axes3D::new([-1.0, 1.0], [-1.0, 1.0], [0.0, 2.0], [200.0, 200.0, 400.0]).unwrap();
    same(&to_scene(&a, 1.0, 1.0, 2.0), &[100.0, -100.0, 200.0]);
    let f = tube_frames(&[[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, 2.0]]).unwrap();
    same(&[f[1].t, f[1].n, f[1].b].concat(), &[0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
    let r = tube(&[[0.0, 0.0, 0.0], [0.0, 0.0, 1.0]], 0.5, 4).unwrap();
    same(&r[0][1].map(|c| round(c, 12)), &[0.0, 0.5, 0.0]);
}

#[test]
fn invalid_inputs_are_typed_errors() {
    assert!(matches!(Axes::new(1.0, 1.0, -1.0, 1.0, 10.0, 10.0), Err(PlotError::Range(_))));
    assert!(matches!(Axes::new(-1.0, 1.0, -1.0, 1.0, 0.0, 10.0), Err(PlotError::Size(_))));
    assert!(matches!(Axes3D::new([0.0, 1.0], [2.0, 1.0], [0.0, 1.0], [1.0; 3]), Err(PlotError::Range(_))));
    assert!(matches!(nice_step(1.0, 1.0), Err(PlotError::Range(_))));
    assert!(matches!(nice_step(0.0, f64::NAN), Err(PlotError::Range(_))));
    assert!(matches!(ticks(0.0, 1.0, 0.0), Err(PlotError::Step(_))));
    assert!(matches!(label_decimals(-1.0), Err(PlotError::Step(_))));
    assert!(matches!(sample_explicit(|x| x, 0.0, 1.0, 1), Err(PlotError::Samples(1))));
    assert!(matches!(marching_squares(|x, _| x, [0.0, 1.0, 0.0, 1.0], 0, 0.0), Err(PlotError::Resolution(0))));
    assert!(matches!(tube_frames(&[[1.0, 2.0, 3.0], [1.0, 2.0, 3.0]]), Err(PlotError::Tube(_))));
    assert!(matches!(tube(&[[0.0; 3], [0.0, 0.0, 1.0]], 0.5, 2), Err(PlotError::Tube(_))));
    assert!(matches!(tube(&[[0.0; 3], [0.0, 0.0, 1.0]], 0.0, 8), Err(PlotError::Tube(_))));
}

#[test]
fn a_cell_with_a_non_finite_corner_draws_nothing() {
    // f is NaN on the column x = 0.5 of a 2 × 2 grid over [0, 1]²: every cell touches it, so nothing is drawn
    let f = |x: f64, y: f64| if x == 0.5 { f64::NAN } else { x + y - 1.0 };
    assert!(marching_squares(f, [0.0, 1.0, 0.0, 1.0], 2, 0.0).unwrap().is_empty());
}
