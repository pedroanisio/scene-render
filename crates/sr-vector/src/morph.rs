//! The path morph of the semantic-tex draft of sr-core (`srep/srep-0000-semantic-tex.md`, Semantics 6 to 10): a
//! sub-path resampled by arc length, the target's samples aligned to the source's (winding, then start), the
//! point-wise interpolation, and the greedy pairing of sub-paths by centroid with left-over sub-paths collapsing to, or
//! growing from, a point. Every function is a pure function of its arguments.

/// The samples per sub-path of a morph (rule 6).
pub const MORPH_SAMPLES: usize = 64;

/// The flatness, in pixels, a path key is flattened with before it is resampled (rule 6: the engine's flatness, the
/// one path masks are flattened with).
pub const MORPH_FLATNESS: f64 = 0.25;

fn clamp(p: f64) -> f64 {
    p.clamp(0.0, 1.0)
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn lerp(a: [f64; 2], b: [f64; 2], p: f64) -> [f64; 2] {
    [a[0] + (b[0] - a[0]) * p, a[1] + (b[1] - a[1]) * p]
}

/// Rule 7: the shoelace sum halved; positive for a counter-clockwise polygon in a y-up frame.
pub fn signed_area(poly: &[[f64; 2]]) -> f64 {
    let n = poly.len();
    (0..n).map(|i| poly[i][0] * poly[(i + 1) % n][1] - poly[(i + 1) % n][0] * poly[i][1]).sum::<f64>() / 2.0
}

/// Rule 9: the area centroid; the mean of the vertices when the area is 0.
pub fn centroid(poly: &[[f64; 2]]) -> [f64; 2] {
    let n = poly.len();
    let a = signed_area(poly);
    if a.abs() < 1e-12 {
        let s = poly.iter().fold([0.0, 0.0], |s, q| [s[0] + q[0], s[1] + q[1]]);
        return [s[0] / n as f64, s[1] / n as f64];
    }
    let (mut cx, mut cy) = (0.0, 0.0);
    for i in 0..n {
        let ([x0, y0], [x1, y1]) = (poly[i], poly[(i + 1) % n]);
        let cross = x0 * y1 - x1 * y0;
        cx += (x0 + x1) * cross;
        cy += (y0 + y1) * cross;
    }
    [cx / (6.0 * a), cy / (6.0 * a)]
}

/// The length around the polygon, closed by its chord.
pub fn perimeter(poly: &[[f64; 2]]) -> f64 {
    let n = poly.len();
    (0..n).map(|i| dist(poly[i], poly[(i + 1) % n])).sum()
}

/// Rule 6: `n` points evenly spaced by arc length around the polygon closed by its chord, the first at its first
/// vertex, in its own direction; a polygon of zero perimeter is its first point `n` times, and an empty one gives
/// nothing.
pub fn resample(poly: &[[f64; 2]], n: usize) -> Vec<[f64; 2]> {
    let total = perimeter(poly);
    let m = poly.len();
    if m == 0 {
        return Vec::new();
    }
    if total <= 0.0 || !total.is_finite() {
        return vec![poly[0]; n];
    }
    let step = total / n as f64;
    let (mut edge, mut acc) = (0usize, 0.0);
    let mut out = Vec::with_capacity(n);
    for k in 0..n {
        let target = k as f64 * step;
        loop {
            let (a, b) = (poly[edge % m], poly[(edge + 1) % m]);
            let len = dist(a, b);
            if acc + len >= target - 1e-12 * total || edge >= 2 * m {
                let t = if len == 0.0 { 0.0 } else { ((target - acc) / len).clamp(0.0, 1.0) };
                out.push(lerp(a, b, t));
                break;
            }
            acc += len;
            edge += 1;
        }
    }
    out
}

fn segments_cross(p1: [f64; 2], p2: [f64; 2], p3: [f64; 2], p4: [f64; 2]) -> bool {
    let orient = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    let (d1, d2) = (orient(p3, p4, p1), orient(p3, p4, p2));
    let (d3, d4) = (orient(p1, p2, p3), orient(p1, p2, p4));
    (d1 > 0.0) != (d2 > 0.0) && (d3 > 0.0) != (d4 > 0.0) && d1 != 0.0 && d2 != 0.0 && d3 != 0.0 && d4 != 0.0
}

/// Rule 10: the number of pairs of non-adjacent edges of the closed polygon that properly cross.
pub fn self_intersections(poly: &[[f64; 2]]) -> usize {
    let n = poly.len();
    let mut count = 0;
    for i in 0..n {
        for j in i + 2..n {
            if i == 0 && j == n - 1 {
                continue;
            }
            if segments_cross(poly[i], poly[(i + 1) % n], poly[j], poly[(j + 1) % n]) {
                count += 1;
            }
        }
    }
    count
}

/// `poly` started `k` vertices later: `rotate(poly, k)[i] = poly[(i + k) mod n]`.
pub fn rotate(poly: &[[f64; 2]], k: usize) -> Vec<[f64; 2]> {
    let n = poly.len();
    (0..n).map(|i| poly[(i + k) % n]).collect()
}

/// Rule 7: the rotation k of `b` that minimises Σ|a_i − b_{(i+k) mod n}|², the smallest k on a tie. Both have the same
/// number of samples.
pub fn align_start(a: &[[f64; 2]], b: &[[f64; 2]]) -> usize {
    let n = a.len();
    assert_eq!(b.len(), n, "align: equal sample counts");
    let mut best: Option<f64> = None;
    let mut best_k = 0;
    for k in 0..n {
        let cost: f64 = (0..n)
            .map(|i| {
                let q = b[(i + k) % n];
                (a[i][0] - q[0]).powi(2) + (a[i][1] - q[1]).powi(2)
            })
            .sum();
        if best.is_none_or(|c| cost < c - 1e-12) {
            best = Some(cost);
            best_k = k;
        }
    }
    best_k
}

/// Rule 7: `b` reversed when its winding differs from `a`'s (its first vertex kept first), then rotated by
/// [`align_start`].
pub fn align(a: &[[f64; 2]], b: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut b = b.to_vec();
    if signed_area(a) * signed_area(&b) < 0.0 {
        b[1..].reverse();
    }
    rotate(&b, align_start(a, &b))
}

/// Rule 8: `a` and `b` resampled to `n`, `b`'s samples aligned to `a`'s, and the point-wise interpolation at progress
/// `p` (clamped to [0, 1]); exactly `a`'s samples at 0 and `b`'s aligned samples at 1.
pub fn morph_polygon(a: &[[f64; 2]], b: &[[f64; 2]], p: f64, n: usize) -> Vec<[f64; 2]> {
    let p = clamp(p);
    let ra = resample(a, n);
    let rb = align(&ra, &resample(b, n));
    if p == 0.0 {
        return ra;
    }
    if p == 1.0 {
        return rb;
    }
    ra.iter().zip(&rb).map(|(&q, &r)| lerp(q, r, p)).collect()
}

/// Rule 9: the sub-paths of `a` and `b` paired greedily by the distance between their centroids, the closest pair
/// first, each at most once; `a`'s sub-paths in order with their partner or `None`, then `b`'s unpaired ones as
/// `(None, Some(j))`.
pub fn match_subpaths(a: &[Vec<[f64; 2]>], b: &[Vec<[f64; 2]>]) -> Vec<(Option<usize>, Option<usize>)> {
    let ca: Vec<_> = a.iter().map(|s| centroid(s)).collect();
    let cb: Vec<_> = b.iter().map(|s| centroid(s)).collect();
    let mut cand: Vec<(f64, usize, usize)> =
        (0..a.len()).flat_map(|i| (0..b.len()).map(move |j| (i, j))).map(|(i, j)| (dist(ca[i], cb[j]), i, j)).collect();
    // the reference sorts the tuples: distance, then i, then j
    cand.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)).then(x.2.cmp(&y.2)));
    let (mut pa, mut pb) = (vec![None; a.len()], vec![None; b.len()]);
    for (_, i, j) in cand {
        if pa[i].is_none() && pb[j].is_none() {
            pa[i] = Some(j);
            pb[j] = Some(i);
        }
    }
    let mut out: Vec<_> = pa.iter().enumerate().map(|(i, &j)| (Some(i), j)).collect();
    out.extend(pb.iter().enumerate().filter(|(_, i)| i.is_none()).map(|(j, _)| (None, Some(j))));
    out
}

fn nearest(c: [f64; 2], cs: &[[f64; 2]]) -> [f64; 2] {
    let k = (0..cs.len()).min_by(|&x, &y| dist(c, cs[x]).total_cmp(&dist(c, cs[y])).then(x.cmp(&y))).unwrap_or(0);
    cs[k]
}

/// Rule 9: every pair of [`match_subpaths`] morphed by rule 8; a left-over sub-path of `a` morphs to the point at the
/// centroid of `b`'s nearest sub-path, and a left-over sub-path of `b` grows from the centroid of `a`'s nearest. `None`
/// when either side has no sub-path: there is no centroid to collapse to, and the key holds.
pub fn morph_path(a: &[Vec<[f64; 2]>], b: &[Vec<[f64; 2]>], p: f64, n: usize) -> Option<Vec<Vec<[f64; 2]>>> {
    if a.is_empty() || b.is_empty() {
        return None;
    }
    let p = clamp(p);
    let ca: Vec<_> = a.iter().map(|s| centroid(s)).collect();
    let cb: Vec<_> = b.iter().map(|s| centroid(s)).collect();
    let out = match_subpaths(a, b)
        .into_iter()
        .map(|pair| match pair {
            (Some(i), Some(j)) => morph_polygon(&a[i], &b[j], p, n),
            (Some(i), None) => {
                let c = nearest(ca[i], &cb);
                resample(&a[i], n).into_iter().map(|q| lerp(q, c, p)).collect()
            }
            (None, Some(j)) => {
                let c = nearest(cb[j], &ca);
                resample(&b[j], n).into_iter().map(|q| lerp(c, q, p)).collect()
            }
            (None, None) => unreachable!("match_subpaths pairs every sub-path with at least itself"),
        })
        .collect();
    Some(out)
}

/// Rules 6 to 9 on path data: `a` and `b` parsed and flattened at [`MORPH_FLATNESS`], morphed by [`morph_path`] at
/// progress `p` with [`MORPH_SAMPLES`] samples, and written back as path data of closed polygons. `None` when either
/// does not parse or has no sub-path, so the key holds.
pub fn morph_path_data(a: &str, b: &str, p: f64) -> Option<String> {
    let subs = |d: &str| -> Option<Vec<Vec<[f64; 2]>>> {
        let path = crate::Path::parse(d).ok()?;
        Some(
            path.flatten(MORPH_FLATNESS)
                .into_iter()
                .filter(|q| !q.pts.is_empty())
                .map(|q| q.pts.iter().map(|r| [r.x, r.y]).collect())
                .collect(),
        )
    };
    let m = morph_path(&subs(a)?, &subs(b)?, p, MORPH_SAMPLES)?;
    let mut out = String::new();
    for poly in &m {
        for (i, q) in poly.iter().enumerate() {
            let cmd = if i == 0 { "M" } else { "L" };
            out.push_str(&format!("{cmd} {} {} ", q[0], q[1]));
        }
        out.push_str("Z ");
    }
    Some(out.trim_end().to_string())
}
