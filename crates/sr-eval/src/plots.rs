//! The coordinate systems and plots of the plots draft (sr-core srep/srep-0000-plots.md; MF1 of SREP 12), as
//! pure functions: the data-to-pixel mapping of an `axes` box (rule 1), clipping to the box (rule 2), axis lines
//! (rule 3), nice steps and ticks (rule 4), tick labels (rule 5), sampled curves (rule 7), implicit curves by
//! marching squares (rule 8), and the 3D mapping and tubes of `axes3D` and `plot3D` (rule 10).
//!
//! Each function follows the draft's reference module (`conformance/tools/plots_reference.py`) operation for
//! operation, so the point sets and label texts are the reference's in 64-bit floats.

use std::collections::HashMap;

/// An input the draft's rules refuse.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PlotError {
    /// A min not below its max, or a non-finite bound.
    #[error("plots: {0}: each min must be below its max")]
    Range(&'static str),
    /// A box size that is not positive.
    #[error("plots: {0}: the size must be positive")]
    Size(&'static str),
    /// A tick step that is not positive.
    #[error("plots: {0}: the step must be positive")]
    Step(&'static str),
    /// Fewer than two samples.
    #[error("plots: {0} samples, at least 2 are needed")]
    Samples(usize),
    /// A marching-squares grid of no cells.
    #[error("plots: resolution {0}, at least 1 is needed")]
    Resolution(usize),
    /// A tube without two distinct points, a radius that is not positive, or fewer than three segments.
    #[error("plots: tube: {0}")]
    Tube(&'static str),
}

/// The default tick step cuts a range into at most this many intervals (rule 4).
pub const MAX_INTERVALS: f64 = 10.0;
/// Pixels between a tick mark and its label (rule 5).
pub const LABEL_GAP: f64 = 4.0;

fn ordered(lo: f64, hi: f64) -> bool {
    lo < hi && lo.is_finite() && hi.is_finite()
}

/// A 2D axes box: the data ranges and the box size in pixels; the box's top-left is the node's origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Axes {
    pub x_min: f64,
    pub x_max: f64,
    pub y_min: f64,
    pub y_max: f64,
    pub width: f64,
    pub height: f64,
}

impl Axes {
    pub fn new(x_min: f64, x_max: f64, y_min: f64, y_max: f64, width: f64, height: f64) -> Result<Axes, PlotError> {
        if !ordered(x_min, x_max) || !ordered(y_min, y_max) {
            return Err(PlotError::Range("axes"));
        }
        if !(width > 0.0 && height > 0.0) {
            return Err(PlotError::Size("axes"));
        }
        Ok(Axes { x_min, x_max, y_min, y_max, width, height })
    }
}

/// Rule 1: data (x, y) to the box's pixel; +y data is up, so the pixel y grows downward from `y_max`.
pub fn to_pixel(a: &Axes, x: f64, y: f64) -> [f64; 2] {
    [(x - a.x_min) / (a.x_max - a.x_min) * a.width, (a.y_max - y) / (a.y_max - a.y_min) * a.height]
}

/// The inverse of [`to_pixel`].
pub fn from_pixel(a: &Axes, px: f64, py: f64) -> [f64; 2] {
    [a.x_min + px / a.width * (a.x_max - a.x_min), a.y_max - py / a.height * (a.y_max - a.y_min)]
}

/// Rule 4: the multiples k × step inside [lo, hi], each the product k × step (never accumulated), with a
/// tolerance of 1e-9 · step at the ends so 3 × 0.1 is inside [0, 0.3].
pub fn ticks(lo: f64, hi: f64, step: f64) -> Result<Vec<f64>, PlotError> {
    if !ordered(lo, hi) {
        return Err(PlotError::Range("ticks"));
    }
    if !(step > 0.0 && step.is_finite()) {
        return Err(PlotError::Step("ticks"));
    }
    let eps = 1e-9 * step;
    let k0 = ((lo - eps) / step).ceil() as i64;
    let k1 = ((hi + eps) / step).floor() as i64;
    Ok((k0..=k1).map(|k| k as f64 * step).collect())
}

/// Rule 4: the smallest step m × 10ᵏ, m ∈ {1, 2, 5}, with (hi − lo)/step ≤ 10.
pub fn nice_step(lo: f64, hi: f64) -> Result<f64, PlotError> {
    if !ordered(lo, hi) {
        return Err(PlotError::Range("nice step"));
    }
    let span = hi - lo;
    let mut k = span.log10().floor() - 2.0;
    loop {
        for m in [1.0, 2.0, 5.0] {
            let step = m * 10f64.powf(k);
            if span / step <= MAX_INTERVALS + 1e-9 {
                return Ok(step);
            }
        }
        k += 1.0;
    }
}

/// Rule 5: the fewest decimals (0 to 12) that write the step exactly: 1 → 0, 0.5 → 1, 0.25 → 2, 0.001 → 3.
pub fn label_decimals(step: f64) -> Result<u32, PlotError> {
    if !(step > 0.0 && step.is_finite()) {
        return Err(PlotError::Step("label"));
    }
    Ok((0..13u32)
        .find(|&d| {
            let s = 10f64.powi(d as i32);
            ((step * s).round() / s - step).abs() <= 1e-9 * step
        })
        .unwrap_or(12))
}

/// Rule 5: the label text of the tick at `value` with `step`: the value with the step's decimals, trailing zeros
/// and a trailing point removed, −0 written 0; on the imaginary axis the suffix i, 1 and −1 written i and −i.
pub fn tick_label(value: f64, step: f64, imaginary: bool) -> Result<String, PlotError> {
    let mut s = format!("{value:.*}", label_decimals(step)? as usize);
    if s.contains('.') {
        s.truncate(s.trim_end_matches('0').trim_end_matches('.').len());
    }
    if s == "-0" || s.is_empty() {
        s = "0".into();
    }
    Ok(match (imaginary, s.as_str()) {
        (false, _) | (_, "0") => s,
        (true, "1") => "i".into(),
        (true, "-1") => "-i".into(),
        (true, _) => s + "i",
    })
}

/// Rule 3: where the perpendicular axis line sits: at 0 when [lo, hi] holds it, else at lo.
pub fn axis_position(lo: f64, hi: f64) -> f64 {
    if lo <= 0.0 && 0.0 <= hi {
        0.0
    } else {
        lo
    }
}

/// The `n` sampling values over [a, b] of rule 7: a + i·Δ, Δ = (b − a)/(n − 1), or /n when `closed`.
pub fn grid(a: f64, b: f64, n: usize, closed: bool) -> Result<Vec<f64>, PlotError> {
    if n < 2 {
        return Err(PlotError::Samples(n));
    }
    let delta = (b - a) / if closed { n } else { n - 1 } as f64;
    Ok((0..n).map(|i| a + i as f64 * delta).collect())
}

/// Rule 7, explicit: (x_i, f(x_i)) at `n` evenly spaced x from x0 to x1.
pub fn sample_explicit(f: impl Fn(f64) -> f64, x0: f64, x1: f64, n: usize) -> Result<Vec<[f64; 2]>, PlotError> {
    Ok(grid(x0, x1, n, false)?.into_iter().map(|x| [x, f(x)]).collect())
}

/// Rule 7, parametric: (x(t_i), y(t_i)) with t_i as SREP 70 rule 2.
pub fn sample_parametric(
    fx: impl Fn(f64) -> f64,
    fy: impl Fn(f64) -> f64,
    t0: f64,
    t1: f64,
    n: usize,
    closed: bool,
) -> Result<Vec<[f64; 2]>, PlotError> {
    Ok(grid(t0, t1, n, closed)?.into_iter().map(|t| [fx(t), fy(t)]).collect())
}

/// Rule 7, polar: (r cos θ, r sin θ), r = r(θ), at `n` evenly spaced θ (radians) from θ0 to θ1.
pub fn sample_polar(fr: impl Fn(f64) -> f64, theta0: f64, theta1: f64, n: usize) -> Result<Vec<[f64; 2]>, PlotError> {
    Ok(grid(theta0, theta1, n, false)?
        .into_iter()
        .map(|th| {
            let r = fr(th);
            [r * th.cos(), r * th.sin()]
        })
        .collect())
}

/// SREP 70 rule 2: points with a non-finite coordinate are dropped, and the finite points between two drops form
/// one subpath each.
pub fn split_finite(points: &[[f64; 2]]) -> Vec<Vec<[f64; 2]>> {
    let mut parts = Vec::new();
    let mut cur = Vec::new();
    for p in points {
        if p.iter().all(|c| c.is_finite()) {
            cur.push(*p);
        } else if !cur.is_empty() {
            parts.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    parts
}

/// Rule 12: the largest vertical distance in pixels between the chord of two consecutive samples of y = f(x)
/// over the axes' x range and the curve at the chord's middle.
pub fn chord_error_px(a: &Axes, f: impl Fn(f64) -> f64, n: usize) -> Result<f64, PlotError> {
    let pts = sample_explicit(&f, a.x_min, a.x_max, n)?;
    Ok(pts.windows(2).fold(0.0, |worst: f64, w| {
        let xm = 0.5 * (w[0][0] + w[1][0]);
        let chord = to_pixel(a, xm, 0.5 * (w[0][1] + w[1][1]))[1];
        let curve = to_pixel(a, xm, f(xm))[1];
        worst.max((chord - curve).abs())
    }))
}

/// The part of segment p–q inside [x0, x1] × [y0, y1], cut against the four half-planes in turn; a cut sets the
/// cut coordinate to the boundary value exactly.
fn clip_segment(p: [f64; 2], q: [f64; 2], x0: f64, y0: f64, x1: f64, y1: f64) -> Option<([f64; 2], [f64; 2])> {
    let mut seg = (p, q);
    for (axis, bound, below) in [(0, x0, false), (0, x1, true), (1, y0, false), (1, y1, true)] {
        let (a, b) = seg;
        let inside = |c: f64| if below { c <= bound } else { c >= bound };
        let (ina, inb) = (inside(a[axis]), inside(b[axis]));
        seg = match (ina, inb) {
            (true, true) => (a, b),
            (false, false) => return None,
            _ => {
                let t = (bound - a[axis]) / (b[axis] - a[axis]);
                let other = 1 - axis;
                let mut cross = [0.0; 2];
                cross[axis] = bound;
                cross[other] = a[other] + t * (b[other] - a[other]);
                if ina {
                    (a, cross)
                } else {
                    (cross, b)
                }
            }
        };
    }
    Some(seg)
}

/// Rule 2: the polyline clipped to [x0, x1] × [y0, y1], one subpath per run of segments whose ends coincide.
pub fn clip_polyline(points: &[[f64; 2]], x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Vec<[f64; 2]>> {
    let mut parts = Vec::new();
    let mut cur: Vec<[f64; 2]> = Vec::new();
    for w in points.windows(2) {
        let Some((a, b)) = clip_segment(w[0], w[1], x0, y0, x1, y1) else { continue };
        if cur.last() == Some(&a) {
            cur.push(b);
        } else {
            if !cur.is_empty() {
                parts.push(std::mem::take(&mut cur));
            }
            cur = vec![a, b];
        }
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    parts
}

/// A point as a map key: equal coordinates (−0 and 0 alike) give equal keys.
fn key(p: [f64; 2]) -> [u64; 2] {
    [(p[0] + 0.0).to_bits(), (p[1] + 0.0).to_bits()]
}

/// Rule 8: the polylines of f(x, y) = `level` over `ranges` = [x0, x1, y0, y1] on a `resolution` × `resolution`
/// grid of cells. A corner is below when f − level < 0; a crossing is interpolated from the edge's left or lower
/// corner and shared by the two cells of the edge; a saddle is resolved by the mean of the four corners; a cell
/// with a non-finite corner draws nothing. A closed polyline repeats its first point at its end.
pub fn marching_squares(
    f: impl Fn(f64, f64) -> f64,
    ranges: [f64; 4],
    resolution: usize,
    level: f64,
) -> Result<Vec<Vec<[f64; 2]>>, PlotError> {
    let (xs, ys) = marching_grid(ranges, resolution)?;
    let v: Vec<Vec<f64>> = xs.iter().map(|&x| ys.iter().map(|&y| f(x, y) - level).collect()).collect();
    Ok(marching_squares_on(&xs, &ys, &v))
}

/// The corner coordinates of rule 8's grid: x_i = x0 + i·(x1 − x0)/R and y_j likewise, i, j = 0 … R.
pub fn marching_grid(ranges: [f64; 4], resolution: usize) -> Result<(Vec<f64>, Vec<f64>), PlotError> {
    let [x0, x1, y0, y1] = ranges;
    if resolution < 1 {
        return Err(PlotError::Resolution(resolution));
    }
    if !ordered(x0, x1) || !ordered(y0, y1) {
        return Err(PlotError::Range("implicit plot"));
    }
    let n = resolution;
    let xs = (0..=n).map(|i| x0 + i as f64 * (x1 - x0) / n as f64).collect();
    let ys = (0..=n).map(|j| y0 + j as f64 * (y1 - y0) / n as f64).collect();
    Ok((xs, ys))
}

/// [`marching_squares`] over corner values already evaluated: `v[i][j]` = f(xs[i], ys[j]) − level on the grid of
/// [`marching_grid`].
pub fn marching_squares_on(xs: &[f64], ys: &[f64], v: &[Vec<f64>]) -> Vec<Vec<[f64; 2]>> {
    let n = xs.len() - 1;
    let mut crossings: HashMap<(usize, usize, bool), [f64; 2]> = HashMap::new();
    let mut crossing = |i: usize, j: usize, horizontal: bool| -> [f64; 2] {
        *crossings.entry((i, j, horizontal)).or_insert_with(|| {
            if horizontal {
                let (va, vb) = (v[i][j], v[i + 1][j]);
                let t = va / (va - vb);
                [xs[i] + t * (xs[i + 1] - xs[i]), ys[j]]
            } else {
                let (va, vb) = (v[i][j], v[i][j + 1]);
                let t = va / (va - vb);
                [xs[i], ys[j] + t * (ys[j + 1] - ys[j])]
            }
        })
    };
    let mut segments: Vec<([f64; 2], [f64; 2])> = Vec::new();
    for i in 0..n {
        for j in 0..n {
            // a bottom-left, then counter-clockwise
            let c = [v[i][j], v[i + 1][j], v[i + 1][j + 1], v[i][j + 1]];
            if !c.iter().all(|x| x.is_finite()) {
                continue;
            }
            let below = c.map(|x| x < 0.0);
            let k = below.iter().filter(|b| **b).count();
            // the edges ab, bc, cd and da by their index
            let mut edge = |e: usize| match e {
                0 => crossing(i, j, true),
                1 => crossing(i + 1, j, false),
                2 => crossing(i, j + 1, true),
                _ => crossing(i, j, false),
            };
            match k {
                0 | 4 => {}
                1 | 3 => {
                    let odd = below.iter().position(|b| *b == (k == 1)).unwrap_or(0);
                    let (e0, e1) = [(3, 0), (0, 1), (1, 2), (2, 3)][odd];
                    segments.push((edge(e0), edge(e1)));
                }
                _ => match below {
                    [true, true, false, false] | [false, false, true, true] => segments.push((edge(3), edge(1))),
                    [false, true, true, false] | [true, false, false, true] => segments.push((edge(0), edge(2))),
                    _ => {
                        let centre_below = (c[0] + c[1] + c[2] + c[3]) / 4.0 < 0.0;
                        if below[0] == centre_below {
                            segments.push((edge(0), edge(1)));
                            segments.push((edge(2), edge(3)));
                        } else {
                            segments.push((edge(3), edge(0)));
                            segments.push((edge(1), edge(2)));
                        }
                    }
                },
            }
        }
    }
    join(&segments)
}

/// Segments with a common endpoint joined into polylines, in the reference's order: each unused segment starts a
/// chain that grows from its end, then, unless it closed, from its start.
fn join(segments: &[([f64; 2], [f64; 2])]) -> Vec<Vec<[f64; 2]>> {
    let mut by_point: HashMap<[u64; 2], Vec<usize>> = HashMap::new();
    for (idx, (p, q)) in segments.iter().enumerate() {
        by_point.entry(key(*p)).or_default().push(idx);
        by_point.entry(key(*q)).or_default().push(idx);
    }
    let mut used = vec![false; segments.len()];
    let mut take = |point: [f64; 2], used: &mut Vec<bool>| -> Option<[f64; 2]> {
        let idx = *by_point.get_mut(&key(point))?.iter().find(|&&i| !used[i])?;
        used[idx] = true;
        let (p, q) = segments[idx];
        Some(if key(p) == key(point) { q } else { p })
    };
    let mut lines = Vec::new();
    for (idx, (p, q)) in segments.iter().enumerate() {
        if used[idx] {
            continue;
        }
        used[idx] = true;
        let mut chain = std::collections::VecDeque::from([*p, *q]);
        while let Some(next) = take(chain[chain.len() - 1], &mut used) {
            chain.push_back(next);
            if key(next) == key(chain[0]) {
                break;
            }
        }
        if key(chain[chain.len() - 1]) != key(chain[0]) {
            while let Some(prev) = take(chain[0], &mut used) {
                chain.push_front(prev);
            }
        }
        lines.push(chain.into_iter().collect());
    }
    lines
}

/// A 3D axes: the data ranges and the box size in scene units, centred on the object's origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Axes3D {
    pub x: [f64; 2],
    pub y: [f64; 2],
    pub z: [f64; 2],
    /// width, height and depth.
    pub size: [f64; 3],
}

impl Axes3D {
    pub fn new(x: [f64; 2], y: [f64; 2], z: [f64; 2], size: [f64; 3]) -> Result<Axes3D, PlotError> {
        if ![x, y, z].iter().all(|r| ordered(r[0], r[1])) {
            return Err(PlotError::Range("axes3D"));
        }
        if !size.iter().all(|s| *s > 0.0) {
            return Err(PlotError::Size("axes3D"));
        }
        Ok(Axes3D { x, y, z, size })
    }
}

/// Rule 10: data to the object's local scene units, the data centre at the origin and +y data up (−y scene).
pub fn to_scene(a: &Axes3D, x: f64, y: f64, z: f64) -> [f64; 3] {
    let m = |r: [f64; 2], v: f64, s: f64| (v - (r[0] + r[1]) / 2.0) / (r[1] - r[0]) * s;
    [m(a.x, x, a.size[0]), -m(a.y, y, a.size[1]), m(a.z, z, a.size[2])]
}

/// The parallel-transport frame at a point of a tube: tangent, normal and binormal, unit vectors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub t: [f64; 3],
    pub n: [f64; 3],
    pub b: [f64; 3],
}

fn unit(v: [f64; 3]) -> Option<[f64; 3]> {
    let n = v[0].hypot(v[1]).hypot(v[2]);
    (n != 0.0).then(|| v.map(|c| c / n))
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn sub(b: [f64; 3], a: [f64; 3]) -> [f64; 3] {
    [b[0] - a[0], b[1] - a[1], b[2] - a[2]]
}

/// Rule 10: the parallel-transport frame at each point of a polyline. T_i is the unit central difference
/// (one-sided at the ends; the previous tangent where it vanishes, the next distinct point's before the first);
/// N₀ is the unit part, normal to T₀, of the coordinate axis least aligned with T₀; N_i is N_{i−1} made normal to
/// T_i and renormalised; B = T × N.
pub fn tube_frames(points: &[[f64; 3]]) -> Result<Vec<Frame>, PlotError> {
    if points.len() < 2 || points.iter().all(|p| *p == points[0]) {
        return Err(PlotError::Tube("at least two distinct points"));
    }
    let last = points.len() - 1;
    let mut tangents: Vec<[f64; 3]> = Vec::with_capacity(points.len());
    let mut prev: Option<[f64; 3]> = None;
    for i in 0..points.len() {
        let mut t = unit(sub(points[(i + 1).min(last)], points[i.saturating_sub(1)])).or(prev);
        if t.is_none() {
            t = points[i + 1..].iter().find_map(|q| unit(sub(*q, points[i])));
        }
        let t = t.ok_or(PlotError::Tube("at least two distinct points"))?;
        tangents.push(t);
        prev = Some(t);
    }
    let t0 = tangents[0];
    let mut axis = 0;
    for k in 1..3 {
        if t0[k].abs() < t0[axis].abs() {
            axis = k;
        }
    }
    let mut e = [0.0; 3];
    e[axis] = 1.0;
    let d = dot(e, t0);
    let mut n = unit([e[0] - d * t0[0], e[1] - d * t0[1], e[2] - d * t0[2]])
        .ok_or(PlotError::Tube("no normal to the first tangent"))?;
    Ok(tangents
        .into_iter()
        .map(|t| {
            let d = dot(n, t);
            n = unit([n[0] - d * t[0], n[1] - d * t[1], n[2] - d * t[2]]).unwrap_or(n);
            Frame { t, n, b: cross(t, n) }
        })
        .collect())
}

/// Rule 10: the vertex rings of a tube of `radius` around the polyline: ring i has `segments` vertices
/// p_i + radius·(cos φ_j N_i + sin φ_j B_i), φ_j = 2πj/segments.
pub fn tube(points: &[[f64; 3]], radius: f64, segments: usize) -> Result<Vec<Vec<[f64; 3]>>, PlotError> {
    if radius.is_nan() || radius <= 0.0 {
        return Err(PlotError::Tube("the radius must be positive"));
    }
    if segments < 3 {
        return Err(PlotError::Tube("at least 3 segments"));
    }
    let frames = tube_frames(points)?;
    Ok(points
        .iter()
        .zip(frames)
        .map(|(p, f)| {
            (0..segments)
                .map(|j| {
                    let phi = 2.0 * std::f64::consts::PI * j as f64 / segments as f64;
                    let (s, c) = phi.sin_cos();
                    [0, 1, 2].map(|k| p[k] + radius * (c * f.n[k] + s * f.b[k]))
                })
                .collect()
        })
        .collect())
}
