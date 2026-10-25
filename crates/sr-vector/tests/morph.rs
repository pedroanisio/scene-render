//! The path-morph rules of the semantic-tex draft (sr-core srep/srep-0000-semantic-tex.md, Semantics 6 to 10):
//! arc-length resampling, start alignment with the winding fix, linear interpolation and greedy sub-path matching
//! by centroid, checked against the draft's numeric table.

use sr_vector::morph::*;

const SQ: [[f64; 2]; 4] = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
const TRI: [[f64; 2]; 3] = [[0.0, 0.0], [10.0, 0.0], [5.0, 8.0]];
const BOW: [[f64; 2]; 4] = [[0.0, 0.0], [10.0, 10.0], [10.0, 0.0], [0.0, 10.0]];

#[track_caller]
fn close(a: f64, b: f64) {
    assert!((a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1e-300), "{a} != {b}");
}

#[track_caller]
fn close_pts(a: &[[f64; 2]], b: &[[f64; 2]]) {
    assert_eq!(a.len(), b.len(), "{a:?} != {b:?}");
    for (p, q) in a.iter().zip(b) {
        close(p[0], q[0]);
        close(p[1], q[1]);
    }
}

fn shifted(poly: &[[f64; 2]], dx: f64, dy: f64) -> Vec<[f64; 2]> {
    poly.iter().map(|q| [q[0] + dx, q[1] + dy]).collect()
}

fn reversed(poly: &[[f64; 2]]) -> Vec<[f64; 2]> {
    poly.iter().rev().copied().collect()
}

#[test]
fn signed_area_is_the_shoelace_sum() {
    close(signed_area(&SQ), 100.0);
    close(signed_area(&reversed(&SQ)), -100.0);
}

#[test]
fn centroid_of_a_triangle() {
    close_pts(&[centroid(&TRI)], &[[5.0, 2.6666666666666665]]);
}

#[test]
fn centroid_of_a_degenerate_polygon_is_the_vertex_mean() {
    close_pts(&[centroid(&[[0.0, 0.0], [4.0, 0.0], [8.0, 0.0]])], &[[4.0, 0.0]]);
}

#[test]
fn perimeter_of_a_square() {
    close(perimeter(&SQ), 40.0);
}

#[test]
fn resample_a_square() {
    close_pts(
        &resample(&SQ, 8),
        &[[0.0, 0.0], [5.0, 0.0], [10.0, 0.0], [10.0, 5.0], [10.0, 10.0], [5.0, 10.0], [0.0, 10.0], [0.0, 5.0]],
    );
}

#[test]
fn resample_a_triangle() {
    close_pts(
        &resample(&TRI, 6),
        &[
            [0.0, 0.0],
            [4.811327044018868, 0.0],
            [9.622654088037736, 0.0],
            [7.649994700015899, 3.7600084799745614],
            [5.099996466677267, 7.840005653316374],
            [2.549998233338632, 4.079997173341811],
        ],
    );
}

#[test]
fn a_zero_perimeter_sub_path_is_a_point_repeated() {
    // rule 6: a sub-path of zero perimeter is its point, N times
    assert_eq!(resample(&[[3.0, 4.0], [3.0, 4.0]], 5), vec![[3.0, 4.0]; 5]);
}

#[test]
fn self_intersections_count_properly_crossing_edges() {
    assert_eq!(self_intersections(&SQ), 0);
    assert_eq!(self_intersections(&BOW), 1);
}

#[test]
fn align_start_undoes_a_rotation() {
    let r = resample(&SQ, 8);
    assert_eq!(align_start(&r, &rotate(&r, 2)), 6);
}

#[test]
fn align_reverses_an_opposite_winding() {
    let r = resample(&SQ, 8);
    assert_eq!(align(&r, &reversed(&r)), r);
}

#[test]
fn morph_at_0_and_1_are_the_ends() {
    assert_eq!(morph_polygon(&SQ, &TRI, 0.0, 8), resample(&SQ, 8));
    assert_eq!(morph_polygon(&SQ, &TRI, 1.0, 8), align(&resample(&SQ, 8), &resample(&TRI, 8)));
}

#[test]
fn morph_midway() {
    close_pts(
        &morph_polygon(&SQ, &TRI, 0.5, 4),
        &[
            [0.0, 0.0],
            [8.60849528301415, 0.0],
            [8.824997350007951, 6.880004239987279],
            [1.9124986750039743, 8.059997880006359],
        ],
    );
}

#[test]
fn progress_is_clamped() {
    assert_eq!(morph_polygon(&SQ, &TRI, -1.0, 8), morph_polygon(&SQ, &TRI, 0.0, 8));
    assert_eq!(morph_polygon(&SQ, &TRI, 2.0, 8), morph_polygon(&SQ, &TRI, 1.0, 8));
}

#[test]
fn square_to_triangle_never_crosses_itself() {
    let worst = (1..10).map(|p| self_intersections(&morph_polygon(&SQ, &TRI, p as f64 / 10.0, MORPH_SAMPLES))).max();
    assert_eq!(worst, Some(0));
}

#[test]
fn match_subpaths_pairs_nearest_centroids_first() {
    let a = vec![SQ.to_vec(), shifted(&SQ, 100.0, 0.0)];
    let b = vec![shifted(&TRI, 100.0, 1.0), shifted(&TRI, 0.0, 1.0)];
    assert_eq!(match_subpaths(&a, &b), vec![(Some(0), Some(1)), (Some(1), Some(0))]);
}

#[test]
fn an_extra_source_subpath_pairs_with_none() {
    let a = vec![SQ.to_vec(), shifted(&SQ, 100.0, 0.0)];
    assert_eq!(match_subpaths(&a, &[TRI.to_vec()]), vec![(Some(0), Some(0)), (Some(1), None)]);
}

#[test]
fn an_incoming_target_subpath_comes_last() {
    let b = vec![TRI.to_vec(), shifted(&TRI, 100.0, 0.0)];
    assert_eq!(match_subpaths(&[SQ.to_vec()], &b), vec![(Some(0), Some(0)), (None, Some(1))]);
}

#[test]
fn a_left_over_subpath_collapses_to_the_nearest_centroid() {
    let a = vec![SQ.to_vec(), shifted(&SQ, 100.0, 0.0)];
    let m = morph_path(&a, &[TRI.to_vec()], 1.0, 4).unwrap();
    assert_eq!(m.len(), 2);
    close_pts(&m[1], &[[5.0, 2.6666666666666665]; 4]);
}

#[test]
fn an_incoming_subpath_grows_from_the_nearest_centroid() {
    let b = vec![TRI.to_vec(), shifted(&TRI, 100.0, 0.0)];
    let m = morph_path(&[SQ.to_vec()], &b, 0.0, 4).unwrap();
    close_pts(&m[1], &[[5.0, 5.0]; 4]);
    let m = morph_path(&[SQ.to_vec()], &b, 1.0, 4).unwrap();
    close_pts(&m[1], &resample(&shifted(&TRI, 100.0, 0.0), 4));
}

#[test]
fn a_side_without_sub_paths_has_no_morph() {
    // no centroid to collapse to or grow from: the caller keeps the hold of a path key
    assert!(morph_path(&[], &[], 0.5, 8).is_none());
    assert!(morph_path(&[SQ.to_vec()], &[], 0.5, 8).is_none());
    assert!(morph_path(&[], &[TRI.to_vec()], 0.5, 8).is_none());
}
