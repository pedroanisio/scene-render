//! The numeric table of the space-transforms draft of sr-core (`srep/srep-0000-space-transforms.md`, Conformance),
//! produced by `conformance/tools/space_transforms_reference.py`, against `sr_vector::space`. Floats agree to 1e-9
//! relative; counts, booleans and exact tuples exactly.

use std::f64::consts::PI;

use sr_vector::space::{
    chord_error, complex_function, complex_map, deform_polyline, expr_map, grid_nodes, inverse_stereographic,
    matrix_map, penrose, stereographic, subdivide_segment, tangent_angle, Complex, ComplexFn, DEFAULT_TOLERANCE,
};

fn close(a: f64, b: f64) -> bool {
    a == b || (a - b).abs() <= 1e-9 * a.abs().max(b.abs())
}

fn close2(a: [f64; 2], b: [f64; 2]) -> bool {
    close(a[0], b[0]) && close(a[1], b[1])
}

fn c(re: f64, im: f64) -> Complex {
    Complex { re, im }
}

fn round(x: f64, digits: i32) -> f64 {
    let s = 10f64.powi(digits);
    (x * s).round() / s
}

const O: [f64; 2] = [0.0, 0.0];

fn sq(q: [f64; 2]) -> [f64; 2] {
    complex_map(ComplexFn::Square, &[], q, O, 100.0)
}

#[test]
fn matrix_rows() {
    assert_eq!(matrix_map([1.0, 0.0, 0.0, 1.0, 0.0, 0.0], [3.0, 4.0], O), [3.0, 4.0]);
    assert_eq!(matrix_map([2.0, 0.0, 0.0, 3.0, 10.0, 20.0], [1.0, 1.0], O), [12.0, 23.0]);
    assert_eq!(matrix_map([0.0, 1.0, -1.0, 0.0, 0.0, 0.0], [1.0, 0.0], O), [0.0, 1.0]);
    assert_eq!(matrix_map([2.0, 0.0, 0.0, 2.0, 0.0, 0.0], [110.0, 60.0], [100.0, 50.0]), [120.0, 70.0]);
    let node = grid_nodes(5, 3, [0.0, 0.0, 400.0, 200.0])[7];
    assert_eq!(matrix_map([1.0, 0.5, 0.25, 1.0, 0.0, 0.0], node, O), [225.0, 200.0]);
    assert_eq!(
        grid_nodes(3, 2, [0.0, 0.0, 100.0, 50.0]),
        vec![[0.0, 0.0], [50.0, 0.0], [100.0, 0.0], [0.0, 50.0], [50.0, 50.0], [100.0, 50.0]]
    );
}

#[test]
fn expression_rows() {
    let z2 = expr_map(|u, v| [u * u - v * v, 2.0 * u * v], [150.0, 50.0], [100.0, 100.0], 50.0);
    assert!(close2(z2, [100.0, 0.0]), "{z2:?}");
    assert_eq!(expr_map(|u, v| [u, v + 1.0], [0.0, 0.0], O, 1.0), [0.0, -1.0]);
}

#[test]
fn complex_rows() {
    let f = |k, z| complex_function(k, z, &[]);
    assert_eq!(f(ComplexFn::Square, c(1.0, 2.0)), c(-3.0, 4.0));
    let inv = f(ComplexFn::Inverse, c(0.0, 2.0));
    assert!(close(inv.re, 0.0) && close(inv.im, -0.5), "{inv:?}");
    let e = f(ComplexFn::Exp, c(0.0, PI));
    assert_eq!((round(e.re, 12), round(e.im, 12)), (-1.0, 0.0));
    let l = f(ComplexFn::Log, c(-1.0, 0.0));
    assert!(close(l.re, 0.0) && close(l.im, PI), "{l:?}");
    let s = f(ComplexFn::Sqrt, c(-4.0, 0.0));
    assert!(close(s.re, 0.0) && close(s.im, 2.0), "{s:?}");
    assert_eq!(f(ComplexFn::Joukowski, c(2.0, 0.0)), c(2.5, 0.0));
    let m = complex_function(ComplexFn::Mobius, c(0.0, 0.0), &[1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0]);
    assert!(close(m.re, 0.0) && close(m.im, 1.0), "{m:?}");
    let pole = f(ComplexFn::Inverse, c(0.0, 0.0));
    assert!(!pole.re.is_finite() && !pole.im.is_finite(), "{pole:?}");
    let cm = complex_map(ComplexFn::Square, &[], [150.0, 50.0], [100.0, 100.0], 50.0);
    assert!(close2(cm, [100.0, 0.0]), "{cm:?}");
}

#[test]
fn conformality_rows() {
    let ex = |q| complex_map(ComplexFn::Exp, &[], q, O, 100.0);
    assert_eq!(round(tangent_angle(sq, [30.0, -20.0], [1.0, 0.0], [0.0, 1.0]), 5), 90.0);
    assert_eq!(round(tangent_angle(ex, [30.0, -20.0], [1.0, 0.0], [0.0, 1.0]), 5), 90.0);
    let stretch = |q| expr_map(|u, v| [u, 2.0 * v], q, O, 1.0);
    assert_eq!(round(tangent_angle(stretch, [1.0, 1.0], [1.0, 1.0], [1.0, -1.0]), 5), 126.8699);
}

#[test]
fn subdivision_rows() {
    let shear = |q| matrix_map([2.0, 0.5, 0.0, 1.0, 0.0, 0.0], q, O);
    assert_eq!(subdivide_segment(shear, [0.0, 0.0], [100.0, 100.0], 0.25), vec![[0.0, 0.0], [100.0, 100.0]]);
    let pts = subdivide_segment(sq, [0.0, 0.0], [100.0, 0.0], 0.25);
    assert_eq!(pts.len(), 17);
    assert!(chord_error(sq, &pts) <= 0.25);
    let wiggle = |q: [f64; 2]| [q[0], 1000.0 * (q[0] * 1000.0).sin()];
    assert_eq!(subdivide_segment(wiggle, [0.0, 0.0], [1.0, 0.0], 1e-9).len(), 1025);
    assert_eq!(DEFAULT_TOLERANCE, 0.25);
}

#[test]
fn polyline_rows() {
    let shift = |q| matrix_map([1.0, 0.0, 0.0, 1.0, 5.0, 5.0], q, O);
    assert_eq!(
        deform_polyline(shift, &[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]], 0.25, false),
        vec![vec![[5.0, 5.0], [15.0, 5.0], [15.0, 15.0]]]
    );
    let id = |q| matrix_map([1.0, 0.0, 0.0, 1.0, 0.0, 0.0], q, O);
    assert_eq!(
        deform_polyline(id, &[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]], 0.25, true),
        vec![vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 0.0]]]
    );
    let inv = |q| complex_map(ComplexFn::Inverse, &[], q, O, 10.0);
    assert_eq!(deform_polyline(inv, &[[-20.0, 0.0], [0.0, 0.0], [20.0, 0.0]], 0.25, false).len(), 2);
}

#[test]
fn projection_rows() {
    assert_eq!(stereographic([1.0, 0.0, 0.0, 0.0]), [1.0, 0.0, 0.0]);
    assert_eq!(stereographic([0.0, 0.0, 0.0, -1.0]), [0.0, 0.0, 0.0]);
    assert_eq!(round(stereographic([0.0, 0.0, 1f64.sin(), 1f64.cos()])[2], 12), 1.830487721712);
    let q = inverse_stereographic([0.5, -0.25, 0.75]);
    let want = [0.5333333333333333, -0.26666666666666666, 0.8, -0.06666666666666667];
    assert!(q.iter().zip(want).all(|(a, b)| close(*a, b)), "{q:?}");
    // the table's 0.7853981633974483 is π/4 to the last digit
    assert!(close2(penrose(1.0, 0.0), [std::f64::consts::FRAC_PI_4, 0.0]));
    let ray = |t, x| {
        let p = penrose(t, x);
        round(p[0] - p[1], 12)
    };
    assert_eq!(ray(3.0, 2.0), ray(-1.0, -2.0));
    let inf = penrose(1e12, 0.0);
    // the table's 1.570796327 is π/2 to nine digits
    assert_eq!((round(inf[0], 9), round(inf[1], 9)), (round(std::f64::consts::FRAC_PI_2, 9), 0.0));
}

#[test]
fn malformed_input_is_refused_or_non_finite() {
    // a Möbius map needs eight coefficients and ad − bc ≠ 0: anything else is a non-finite map, never a panic
    let bad = complex_function(ComplexFn::Mobius, c(1.0, 1.0), &[1.0, 0.0, 2.0]);
    assert!(!bad.re.is_finite());
    let singular = complex_function(ComplexFn::Mobius, c(1.0, 1.0), &[1.0, 0.0, 2.0, 0.0, 1.0, 0.0, 2.0, 0.0]);
    assert!(!singular.re.is_finite());
    assert_eq!(ComplexFn::parse("mobius"), Some(ComplexFn::Mobius));
    assert_eq!(ComplexFn::parse("tan"), None);
    // a non-positive or NaN tolerance keeps the segment's ends: the bound is the code's, not the input's
    assert_eq!(subdivide_segment(sq, [0.0, 0.0], [100.0, 0.0], f64::NAN).len(), 1025);
    assert_eq!(subdivide_segment(sq, [0.0, 0.0], [100.0, 0.0], -1.0).len(), 1025);
    // no points map to nothing; a lone point (a zero-length subpath, kept for its caps) maps to its image
    assert!(deform_polyline(sq, &[], 0.25, false).is_empty());
    assert_eq!(deform_polyline(sq, &[[100.0, 100.0]], 0.25, false), vec![vec![sq([100.0, 100.0])]]);
    let pole = |q| complex_map(ComplexFn::Inverse, &[], q, O, 10.0);
    assert!(deform_polyline(pole, &[[0.0, 0.0]], 0.25, false).is_empty());
}

#[test]
fn the_three_deformers_map_as_the_rules_and_map_outlines_adaptively() {
    use sr_vector::deform::{deform_polys, path_tolerance, Deformer, PointMap};
    use sr_vector::geom::p;
    use sr_vector::path::Poly;
    let shear = Deformer::Matrix { m: [1.0, 0.5, 0.25, 1.0, 0.0, 0.0], center: p(0.0, 0.0), tol: 0.25 };
    let q = shear.apply(p(200.0, 100.0));
    assert_eq!((q.x, q.y), (225.0, 200.0));
    let square =
        Deformer::Complex { f: ComplexFn::Square, coefficients: vec![], center: p(100.0, 100.0), unit: 50.0, tol: 0.5 };
    let q = square.apply(p(150.0, 50.0));
    assert!(close2([q.x, q.y], [100.0, 0.0]), "{q:?}");
    let z2 = Deformer::Map {
        f: PointMap::new(|u, v| [u * u - v * v, 2.0 * u * v]),
        center: p(100.0, 100.0),
        unit: 50.0,
        tol: 0.1,
    };
    let q = z2.apply(p(150.0, 50.0));
    assert!(close2([q.x, q.y], [100.0, 0.0]), "{q:?}");
    // the path tolerance is the least of the three types' tolerances; without one of them there is none
    assert_eq!(path_tolerance(&[shear.clone(), square.clone(), z2]), Some(0.1));
    assert_eq!(path_tolerance(&[Deformer::Twist { amount: 10.0, center: p(0.0, 0.0), radius: 5.0 }]), None);
    // a closed square under a translation stays closed, its four corners moved
    let sq = Poly { pts: vec![p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0), p(0.0, 10.0)], closed: true };
    let shift = Deformer::Matrix { m: [1.0, 0.0, 0.0, 1.0, 5.0, 5.0], center: p(0.0, 0.0), tol: 0.25 };
    let out = deform_polys(&[shift], &[sq], 0.25);
    assert_eq!(out.len(), 1);
    assert!(out[0].closed);
    assert_eq!(
        out[0].pts.iter().map(|q| [q.x, q.y]).collect::<Vec<_>>(),
        [[5.0, 5.0], [15.0, 5.0], [15.0, 15.0], [5.0, 15.0]]
    );
    // a line through the pole of 1/z is two open subpaths
    let inv =
        Deformer::Complex { f: ComplexFn::Inverse, coefficients: vec![], center: p(0.0, 0.0), unit: 10.0, tol: 0.25 };
    let line = Poly { pts: vec![p(-20.0, 0.0), p(20.0, 0.0)], closed: false };
    let out = deform_polys(std::slice::from_ref(&inv), &[line], 0.25);
    assert_eq!(out.len(), 2);
    assert!(out.iter().all(|q| !q.closed && q.pts.iter().all(|r| r.x.is_finite() && r.y.is_finite())));
    // a closed outline broken by the pole is open subpaths
    let ring = Poly { pts: vec![p(-10.0, -10.0), p(10.0, 10.0), p(-10.0, 10.0)], closed: true };
    let out = deform_polys(&[inv], &[ring], 0.25);
    assert!(!out.is_empty() && out.iter().all(|q| !q.closed));
}

#[test]
fn a_chain_within_another_space_maps_there_and_back() {
    use sr_vector::deform::{deform_polys, path_tolerance, Deformer};
    use sr_vector::geom::{p, Xf};
    use sr_vector::path::Poly;
    use sr_vector::scene::{Cmd, FillRule, Scene};
    // a group at (100, 0) scaled by 2: a node point (x, y) is (100 + 2x, 2y) in the group's space
    let to = Xf([2.0, 0.0, 0.0, 2.0, 100.0, 0.0]);
    let shift = Deformer::Matrix { m: [1.0, 0.0, 0.0, 1.0, 10.0, 0.0], center: p(0.0, 0.0), tol: 0.5 };
    let within = Deformer::within(to, vec![shift]).expect("invertible");
    // 10 px of the group's space is 5 px of the node's
    let q = within.apply(p(1.0, 1.0));
    assert_eq!((q.x, q.y), (6.0, 1.0));
    assert_eq!(path_tolerance(std::slice::from_ref(&within)), Some(0.5));
    assert_eq!(
        path_tolerance(&[Deformer::within(
            to,
            vec![Deformer::Twist { amount: 1.0, center: p(0.0, 0.0), radius: 1.0 }]
        )
        .unwrap()]),
        None
    );
    // a space that has no inverse has no chain
    assert!(Deformer::within(Xf([0.0; 6]), vec![]).is_none());
    // a scene's fills and masks map as paths, the pole breaking them
    let inv = Deformer::Complex {
        f: sr_vector::space::ComplexFn::Inverse,
        coefficients: vec![],
        center: p(0.0, 0.0),
        unit: 10.0,
        tol: 0.25,
    };
    let line = Poly { pts: vec![p(-20.0, 0.0), p(20.0, 0.0)], closed: false };
    let mut s = Scene::default();
    s.cmds.push(Cmd::Fill {
        polys: vec![line.clone()],
        rule: FillRule::NonZero,
        paint: sr_vector::Paint::Solid { rgba: [1.0; 4], srgb: false },
        opacity: 1.0,
    });
    let want = deform_polys(std::slice::from_ref(&inv), &[line], 0.25);
    s.map_polys(&|polys: &[Poly]| deform_polys(std::slice::from_ref(&inv), polys, 0.25));
    let Cmd::Fill { polys, .. } = &s.cmds[0] else { panic!() };
    assert_eq!(polys, &want);
    assert_eq!(polys.len(), 2);
}
