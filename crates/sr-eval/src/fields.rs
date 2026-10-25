//! The fields and flow of the fields draft (sr-core srep/srep-0000-fields.md; MF5 of SREP 12), as pure functions:
//! the sources (rule 2), divergence and curl (rule 3), streamlines on the unit direction (rule 4), arrow glyphs
//! (rule 5), evenly spaced streamlines after Jobard and Lefer 1997 (rule 6), line integral convolution after
//! Cabral and Leedom 1993 (rule 7), phase portraits after Wegert 2012 (rule 8), winding numbers (rule 9) and
//! charge and dipole lines (rule 10).
//!
//! Each function follows the draft's reference module (`conformance/tools/fields_reference.py`) operation for
//! operation. A field is a function of the data coordinates (x, y), +y up, to (u, v); a complex number is
//! `[re, im]`.

use std::collections::HashMap;
use std::f64::consts::{PI, TAU};

/// An input the draft's rules refuse.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum FieldError {
    /// A grid of fewer than two rows of two nodes, or of rows of different lengths.
    #[error("fields: a grid has at least two rows of at least two nodes, all rows of one length")]
    Grid,
    /// A step that is not positive.
    #[error("fields: the step is positive")]
    Step,
    /// A step or line bound below one.
    #[error("fields: the step and line bounds are positive integers")]
    Bound,
    /// An arrow grid of fewer than two columns or rows.
    #[error("fields: an arrow grid has at least two columns and two rows")]
    ArrowGrid,
    /// A separation that is not positive.
    #[error("fields: the separation is positive")]
    Separation,
    /// A test distance not in (0, separation].
    #[error("fields: the test distance is positive and at most the separation")]
    Test,
    /// A noise raster that is empty or not rectangular, or a kernel length or step that is not positive.
    #[error("fields: line integral convolution needs a rectangular noise raster and a positive length and step")]
    Lic,
    /// A zero or a pole on the winding circle, a radius that is not positive, or fewer than three samples.
    #[error("fields: winding number: a zero or a pole lies on the circle, or the circle has no positive radius")]
    Winding,
    /// A point on the dipole's axis or at its centre, or a zero moment.
    #[error("fields: the dipole line constant is undefined on the axis, at the centre and for a zero moment")]
    Dipole,
}

/// Rule 4: the default step bound of one streamline, each way.
pub const DEFAULT_MAX_STEPS: u64 = 10_000;
/// Rule 6: the default bound on the number of streamlines.
pub const DEFAULT_MAX_LINES: usize = 10_000;
/// The schema's bound on `maxSteps` and `maxLines` (FLD4).
pub const MAX_BOUND: u64 = 1_000_000;
/// Rule 3: the central-difference step, in data units.
pub const DIFF_H: f64 = 1e-4;
/// Rule 9: samples around the circle.
pub const WINDING_SAMPLES: usize = 256;

/// Whether `x` is a positive number (NaN is not).
fn positive(x: f64) -> bool {
    x > 0.0
}

// --- rule 2: sources ------------------------------------------------------------------------------------------------

/// A grid field: `values` is rows of (u, v) nodes, row j at y = y0 + j (y1 − y0) / (rows − 1) and column i at
/// x = x0 + i (x1 − x0) / (cols − 1) over `ranges` = [x0, y0, x1, y1]; bilinear between the nodes, (0, 0) outside
/// the box.
pub fn grid_sampler(values: &[Vec<[f64; 2]>], ranges: [f64; 4]) -> Result<impl Fn(f64, f64) -> [f64; 2], FieldError> {
    let rows: Vec<Vec<[f64; 2]>> = values.to_vec();
    if rows.len() < 2 || rows.iter().any(|r| r.len() < 2 || r.len() != rows[0].len()) {
        return Err(FieldError::Grid);
    }
    let [x0, y0, x1, y1] = ranges;
    let (nx, ny) = (rows[0].len(), rows.len());
    Ok(move |x: f64, y: f64| {
        if !(x0 <= x && x <= x1 && y0 <= y && y <= y1) {
            return [0.0, 0.0];
        }
        let fx = (x - x0) / (x1 - x0) * (nx - 1) as f64;
        let fy = (y - y0) / (y1 - y0) * (ny - 1) as f64;
        let (i, j) = ((fx as usize).min(nx - 2), (fy as usize).min(ny - 2));
        let (s, t) = (fx - i as f64, fy - j as f64);
        let (a, b) = (rows[j][i], rows[j][i + 1]);
        let (c, d) = (rows[j + 1][i], rows[j + 1][i + 1]);
        [0, 1].map(|k| (a[k] * (1.0 - s) + b[k] * s) * (1.0 - t) + (c[k] * (1.0 - s) + d[k] * s) * t)
    })
}

/// A complex function as a vector field: (x, y) to (Re f(z), Im f(z)) with z = x + i y.
pub fn complex_sampler(f: impl Fn([f64; 2]) -> [f64; 2]) -> impl Fn(f64, f64) -> [f64; 2] {
    move |x, y| f([x, y])
}

/// The Coulomb field of point charges [x, y, q] in the plane through them: the sum of q r / |r|³ with r = p − c
/// (the constant is 1); (0, 0) at a charge.
pub fn charge_field(charges: &[[f64; 3]], p: [f64; 2]) -> [f64; 2] {
    let (mut ex, mut ey) = (0.0, 0.0);
    for &[cx, cy, q] in charges {
        let (rx, ry) = (p[0] - cx, p[1] - cy);
        let r = rx.hypot(ry);
        if r == 0.0 {
            return [0.0, 0.0];
        }
        ex += q * rx / r.powi(3);
        ey += q * ry / r.powi(3);
    }
    [ex, ey]
}

/// The field of a point dipole of moment `m` at `center` in the plane of its axis: (3 (m · r̂) r̂ − m) / r³; (0, 0)
/// at the centre.
pub fn dipole_field(p: [f64; 2], m: [f64; 2], center: [f64; 2]) -> [f64; 2] {
    let (rx, ry) = (p[0] - center[0], p[1] - center[1]);
    let r = rx.hypot(ry);
    if r == 0.0 {
        return [0.0, 0.0];
    }
    let (ux, uy) = (rx / r, ry / r);
    let dot = m[0] * ux + m[1] * uy;
    [(3.0 * dot * ux - m[0]) / r.powi(3), (3.0 * dot * uy - m[1]) / r.powi(3)]
}

/// The field of a straight current out of the plane at `center`: current / (2π r), counterclockwise; (0, 0) at
/// the wire.
pub fn wire_field(p: [f64; 2], current: f64, center: [f64; 2]) -> [f64; 2] {
    let (rx, ry) = (p[0] - center[0], p[1] - center[1]);
    let r2 = rx * rx + ry * ry;
    if r2 == 0.0 {
        return [0.0, 0.0];
    }
    let k = current / (2.0 * PI * r2);
    [-ry * k, rx * k]
}

/// A 2D point source: r / |r|².
pub fn source_field(p: [f64; 2], center: [f64; 2]) -> [f64; 2] {
    let (rx, ry) = (p[0] - center[0], p[1] - center[1]);
    let r2 = rx * rx + ry * ry;
    if r2 == 0.0 {
        [0.0, 0.0]
    } else {
        [rx / r2, ry / r2]
    }
}

/// E × B for an in-plane E and an out-of-plane B: (E_y B, −E_x B).
pub fn poynting(e: [f64; 2], b: f64) -> [f64; 2] {
    [e[1] * b, -e[0] * b]
}

/// A straight wire in the plane, `[x1, y1, x2, y2, q, i]`, carrying the charge q per unit length and the current i
/// from the first point to the second: the in-plane E of the finite line charge (Griffiths example 2.1) and the
/// out-of-plane B of the segment (Biot–Savart, Griffiths example 5.5), constant 1, as [E_x, E_y, B_z]; zero on the
/// wire's line. With t̂ the direction of the wire, n̂ its left normal, s the position along it and d the signed
/// distance from its line, φ = ((L − s)/√((L − s)² + d²) + s/√(s² + d²)) / d, E = q (φ n̂ + (1/√(s² + d²) −
/// 1/√((L − s)² + d²)) t̂) and B_z = i φ / 4π, so a long wire gives i / (2π d).
pub fn line_field(p: [f64; 2], wire: [f64; 6]) -> [f64; 3] {
    let [x1, y1, x2, y2, q, i] = wire;
    let (tx, ty) = (x2 - x1, y2 - y1);
    let length = tx.hypot(ty);
    let (tx, ty) = (tx / length, ty / length);
    let (nx, ny) = (-ty, tx);
    let (px, py) = (p[0] - x1, p[1] - y1);
    let (s, d) = (px * tx + py * ty, px * nx + py * ny);
    if d == 0.0 {
        return [0.0, 0.0, 0.0];
    }
    let (r1, r2) = ((s * s + d * d).sqrt(), ((length - s) * (length - s) + d * d).sqrt());
    let perp = ((length - s) / r2 + s / r1) / d;
    let par = 1.0 / r1 - 1.0 / r2;
    [q * (perp * nx + par * tx), q * (perp * ny + par * ty), i * perp / (4.0 * PI)]
}

/// The Poynting vector at `p`: E the sum of the charges `[x, y, q]`, the dipoles `[x, y, mx, my]` and the wires' line
/// charges, B_z the sum of the wires' currents ([`line_field`]).
pub fn poynting_at(p: [f64; 2], charges: &[[f64; 3]], dipoles: &[[f64; 4]], wires: &[[f64; 6]]) -> [f64; 2] {
    let mut e = charge_field(charges, p);
    for &[x, y, mx, my] in dipoles {
        let d = dipole_field(p, [mx, my], [x, y]);
        e = [e[0] + d[0], e[1] + d[1]];
    }
    let mut b = 0.0;
    for &w in wires {
        let [wx, wy, wb] = line_field(p, w);
        e = [e[0] + wx, e[1] + wy];
        b += wb;
    }
    poynting(e, b)
}

// --- rule 3: operators ----------------------------------------------------------------------------------------------

/// ∂u/∂x + ∂v/∂y by central differences of step [`DIFF_H`].
pub fn divergence(f: impl Fn(f64, f64) -> [f64; 2], p: [f64; 2]) -> f64 {
    let ([x, y], h) = (p, DIFF_H);
    (f(x + h, y)[0] - f(x - h, y)[0]) / (2.0 * h) + (f(x, y + h)[1] - f(x, y - h)[1]) / (2.0 * h)
}

/// ∂v/∂x − ∂u/∂y by central differences of step [`DIFF_H`], positive counterclockwise.
pub fn curl(f: impl Fn(f64, f64) -> [f64; 2], p: [f64; 2]) -> f64 {
    let ([x, y], h) = (p, DIFF_H);
    (f(x + h, y)[1] - f(x - h, y)[1]) / (2.0 * h) - (f(x, y + h)[0] - f(x, y - h)[0]) / (2.0 * h)
}

// --- rule 4: streamlines --------------------------------------------------------------------------------------------

fn unit(f: &impl Fn(f64, f64) -> [f64; 2], p: [f64; 2]) -> Option<[f64; 2]> {
    let [u, v] = f(p[0], p[1]);
    let n = u.hypot(v);
    if n == 0.0 || !n.is_finite() {
        return None;
    }
    Some([u / n, v / n])
}

/// One Runge–Kutta 4 step of length `h` along the unit direction of `f`, so `h` is arc length; None where `f` is
/// zero or not finite at any stage.
pub fn rk4_step(f: impl Fn(f64, f64) -> [f64; 2], p: [f64; 2], h: f64) -> Option<[f64; 2]> {
    step(&f, p, h)
}

fn step(f: &impl Fn(f64, f64) -> [f64; 2], p: [f64; 2], h: f64) -> Option<[f64; 2]> {
    let k1 = unit(f, p)?;
    let k2 = unit(f, [p[0] + 0.5 * h * k1[0], p[1] + 0.5 * h * k1[1]])?;
    let k3 = unit(f, [p[0] + 0.5 * h * k2[0], p[1] + 0.5 * h * k2[1]])?;
    let k4 = unit(f, [p[0] + h * k3[0], p[1] + h * k3[1]])?;
    Some([
        p[0] + h * (k1[0] + 2.0 * k2[0] + 2.0 * k3[0] + k4[0]) / 6.0,
        p[1] + h * (k1[1] + 2.0 * k2[1] + 2.0 * k3[1] + k4[1]) / 6.0,
    ])
}

/// The predicate of a box [x0, y0, x1, y1], edges included.
pub fn inside_box(ranges: [f64; 4]) -> impl Fn([f64; 2]) -> bool + Copy {
    let [x0, y0, x1, y1] = ranges;
    move |p| x0 <= p[0] && p[0] <= x1 && y0 <= p[1] && p[1] <= y1
}

/// How [`streamline`] marches.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trace {
    /// The arc length of a step.
    pub h: f64,
    /// The most steps each way.
    pub max_steps: u64,
    /// Backward as well as forward.
    pub both: bool,
    /// Keep the point that met the stop condition as the line's last.
    pub keep_stop: bool,
}

fn march(
    f: &impl Fn(f64, f64) -> [f64; 2],
    p0: [f64; 2],
    h: f64,
    t: &Trace,
    inside: &impl Fn([f64; 2]) -> bool,
    stop: Option<&dyn Fn([f64; 2]) -> bool>,
) -> Vec<[f64; 2]> {
    let mut pts = Vec::new();
    let mut p = p0;
    for _ in 0..t.max_steps {
        let Some(q) = step(f, p, h).filter(|q| inside(*q)) else { break };
        if stop.is_some_and(|s| s(q)) {
            if t.keep_stop {
                pts.push(q);
            }
            break;
        }
        pts.push(q);
        p = q;
    }
    pts
}

/// The polyline through `p0`: steps of `t.h` forward (and backward when `t.both`) while the point is inside, the
/// field is not zero and fewer than `t.max_steps` steps were taken each way. A point for which `stop` holds ends
/// the line: it is left out, or kept as the last point when `t.keep_stop`.
pub fn streamline(
    f: impl Fn(f64, f64) -> [f64; 2],
    p0: [f64; 2],
    t: &Trace,
    inside: impl Fn([f64; 2]) -> bool,
    stop: Option<&dyn Fn([f64; 2]) -> bool>,
) -> Result<Vec<[f64; 2]>, FieldError> {
    if !positive(t.h) {
        return Err(FieldError::Step);
    }
    if t.max_steps < 1 {
        return Err(FieldError::Bound);
    }
    let fwd = march(&f, p0, t.h, t, &inside, stop);
    let back = if t.both { march(&f, p0, -t.h, t, &inside, stop) } else { Vec::new() };
    Ok(back.into_iter().rev().chain(std::iter::once(p0)).chain(fwd).collect())
}

// --- rule 5: arrows -------------------------------------------------------------------------------------------------

/// The glyph attributes of rule 5.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArrowStyle {
    /// Shaft length per unit of |F|.
    pub scale: f64,
    /// The longest shaft.
    pub max_length: f64,
    /// Head length as a fraction of the shaft.
    pub head_length: f64,
    /// Head half-width as a fraction of the head length.
    pub head_width: f64,
}

impl Default for ArrowStyle {
    fn default() -> ArrowStyle {
        ArrowStyle { scale: 1.0, max_length: 40.0, head_length: 0.3, head_width: 0.5 }
    }
}

/// An arrow glyph: its shaft (start, tip) and its head (tip, left, right).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Arrow {
    /// From the node to the tip.
    pub shaft: [[f64; 2]; 2],
    /// The tip and the two corners of the head's base.
    pub head: [[f64; 2]; 3],
}

/// The glyph at `p` of vector `v`: a shaft of length min(|v| scale, maxLength) along `v` and a head of
/// `head_length` of the shaft and half-width `head_width` of that; None for a zero or non-finite vector.
pub fn arrow(p: [f64; 2], v: [f64; 2], s: &ArrowStyle) -> Option<Arrow> {
    let n = v[0].hypot(v[1]);
    if n == 0.0 || !n.is_finite() {
        return None;
    }
    let length = (n * s.scale).min(s.max_length);
    let (dx, dy) = (v[0] / n, v[1] / n);
    let tip = [p[0] + dx * length, p[1] + dy * length];
    let hl = s.head_length * length;
    let hw = s.head_width * hl;
    let base = [tip[0] - dx * hl, tip[1] - dy * hl];
    let left = [base[0] - dy * hw, base[1] + dx * hw];
    let right = [base[0] + dy * hw, base[1] - dx * hw];
    Some(Arrow { shaft: [p, tip], head: [tip, left, right] })
}

/// The glyphs at the `cols` × `rows` nodes spanning `ranges` (edges included), row by row from y0.
pub fn arrow_grid(
    f: impl Fn(f64, f64) -> [f64; 2],
    ranges: [f64; 4],
    cols: usize,
    rows: usize,
    s: &ArrowStyle,
) -> Result<Vec<Option<Arrow>>, FieldError> {
    if cols < 2 || rows < 2 {
        return Err(FieldError::ArrowGrid);
    }
    let [x0, y0, x1, y1] = ranges;
    let mut out = Vec::with_capacity(cols * rows);
    for j in 0..rows {
        let y = y0 + (y1 - y0) * j as f64 / (rows - 1) as f64;
        for i in 0..cols {
            let x = x0 + (x1 - x0) * i as f64 / (cols - 1) as f64;
            out.push(arrow([x, y], f(x, y), s));
        }
    }
    Ok(out)
}

// --- rule 6: evenly spaced streamlines (Jobard and Lefer 1997) -------------------------------------------------------

/// The attributes of rule 6.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spacing {
    /// d_sep, the separation of seeds from accepted lines.
    pub d_sep: f64,
    /// d_test, the distance at which a line stops near another; d_sep / 2 when None.
    pub d_test: Option<f64>,
    /// The step; d_sep / 4 when None.
    pub h: Option<f64>,
    /// The step bound of one line, each way.
    pub max_steps: u64,
    /// The bound on the number of lines.
    pub max_lines: usize,
    /// The first seed; the box centre when None.
    pub seed: Option<[f64; 2]>,
}

impl Default for Spacing {
    fn default() -> Spacing {
        Spacing {
            d_sep: 1.0,
            d_test: None,
            h: None,
            max_steps: DEFAULT_MAX_STEPS,
            max_lines: DEFAULT_MAX_LINES,
            seed: None,
        }
    }
}

/// A point of an accepted streamline, the line it belongs to and its arc index.
type Point = ([f64; 2], usize, i64);

/// Points of accepted streamlines in cells of size d.
struct Buckets {
    d: f64,
    cells: HashMap<(i64, i64), Vec<Point>>,
}

impl Buckets {
    fn key(&self, p: [f64; 2]) -> (i64, i64) {
        ((p[0] / self.d).floor() as i64, (p[1] / self.d).floor() as i64)
    }

    fn add(&mut self, p: [f64; 2], line: usize, index: i64) {
        let k = self.key(p);
        self.cells.entry(k).or_default().push((p, line, index));
    }

    /// The smallest distance from `p` to a stored point, where a point of `line` counts only when its arc index
    /// is at least `lag` from `index` (infinite when none is within the neighbouring cells).
    fn nearest(&self, p: [f64; 2], line: Option<usize>, index: i64, lag: i64) -> f64 {
        let (kx, ky) = self.key(p);
        let mut best = f64::INFINITY;
        for i in kx - 1..=kx + 1 {
            for j in ky - 1..=ky + 1 {
                for (q, owner, k) in self.cells.get(&(i, j)).into_iter().flatten() {
                    if Some(*owner) == line && (k - index).abs() < lag {
                        continue;
                    }
                    best = best.min((p[0] - q[0]).hypot(p[1] - q[1]));
                }
            }
        }
        best
    }
}

/// Jobard and Lefer 1997 (rule 6). The first streamline starts at the seed. A queue holds the accepted lines in
/// order; for each, the candidate seeds lie at d_sep on both sides of each of its points, and a candidate becomes a
/// line when it is inside the box, the field is not zero there, and it is at least d_sep from every accepted point.
/// When the queue is empty, the nodes of the grid of spacing d_sep from (x0, y0), row by row, are tried under the
/// same test. A line is traced both ways and a step ends it when its point is within d_test of another line, or of
/// its own at least ⌈d_sep / h⌉ + 1 steps away along the arc (a closed orbit).
pub fn evenly_spaced_streamlines(
    f: impl Fn(f64, f64) -> [f64; 2],
    ranges: [f64; 4],
    s: &Spacing,
) -> Result<Vec<Vec<[f64; 2]>>, FieldError> {
    let d_sep = s.d_sep;
    if !positive(d_sep) {
        return Err(FieldError::Separation);
    }
    let d_test = s.d_test.unwrap_or(0.5 * d_sep);
    if !(0.0 < d_test && d_test <= d_sep) {
        return Err(FieldError::Test);
    }
    let h = s.h.unwrap_or(d_sep / 4.0);
    if !positive(h) {
        return Err(FieldError::Step);
    }
    if s.max_lines < 1 || s.max_steps < 1 {
        return Err(FieldError::Bound);
    }
    let [x0, y0, x1, y1] = ranges;
    let inside = inside_box(ranges);
    let seed = s.seed.unwrap_or([(x0 + x1) / 2.0, (y0 + y1) / 2.0]);
    let mut buckets = Buckets { d: d_sep, cells: HashMap::new() };
    let mut lines: Vec<Vec<[f64; 2]>> = Vec::new();
    let lag = (d_sep / h).ceil() as i64 + 1;

    let trace = |p0: [f64; 2], buckets: &mut Buckets, token: usize| -> Vec<[f64; 2]> {
        buckets.add(p0, token, 0);
        let mut halves: [Vec<[f64; 2]>; 2] = Default::default();
        for (half, sgn) in [1.0, -1.0].into_iter().enumerate() {
            let mut p = p0;
            for n in 1..=s.max_steps {
                let idx = (sgn * n as f64) as i64;
                let Some(q) = step(&f, p, sgn * h) else { break };
                if !inside(q) || buckets.nearest(q, Some(token), idx, lag) < d_test {
                    break;
                }
                buckets.add(q, token, idx);
                halves[half].push(q);
                p = q;
            }
        }
        let [fwd, back] = halves;
        back.into_iter().rev().chain(std::iter::once(p0)).chain(fwd).collect()
    };

    if inside(seed) && unit(&f, seed).is_some() {
        let line = trace(seed, &mut buckets, 0);
        lines.push(line);
    }
    let nx = ((x1 - x0) / d_sep + 1e-9).floor() as i64;
    let ny = ((y1 - y0) / d_sep + 1e-9).floor() as i64;
    let mut scan = (0..=ny).flat_map(|j| (0..=nx).map(move |i| [x0 + i as f64 * d_sep, y0 + j as f64 * d_sep]));
    let mut k = 0;
    while lines.len() < s.max_lines {
        if k >= lines.len() {
            // the queue is empty: the next grid node far enough from every line seeds one, so a region the seed's
            // line does not reach, or a seed on a zero, is still covered
            let Some(c) = scan.next() else { break };
            if buckets.nearest(c, None, 0, 0) < d_sep - 1e-12 || unit(&f, c).is_none() {
                continue;
            }
            let line = trace(c, &mut buckets, lines.len());
            lines.push(line);
            continue;
        }
        let line = lines[k].clone();
        k += 1;
        for (i, &p) in line.iter().enumerate() {
            if lines.len() >= s.max_lines {
                break;
            }
            let q = line.get(i + 1).or(if i > 0 { line.get(i - 1) } else { None });
            let d = match q {
                None => match unit(&f, p) {
                    Some(d) => d,
                    None => continue,
                },
                Some(q) => {
                    let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
                    let n = dx.hypot(dy);
                    if n == 0.0 {
                        continue;
                    }
                    [dx / n, dy / n]
                }
            };
            for sgn in [1.0, -1.0] {
                let c = [p[0] - d[1] * d_sep * sgn, p[1] + d[0] * d_sep * sgn];
                if !inside(c) || buckets.nearest(c, None, 0, 0) < d_sep - 1e-12 || unit(&f, c).is_none() {
                    continue;
                }
                let line = trace(c, &mut buckets, lines.len());
                lines.push(line);
                if lines.len() >= s.max_lines {
                    break;
                }
            }
        }
    }
    Ok(lines)
}

/// The smallest distance between a point of one line and a point of another; infinite below two lines.
pub fn min_separation(lines: &[Vec<[f64; 2]>]) -> f64 {
    let mut best = f64::INFINITY;
    for (a, la) in lines.iter().enumerate() {
        for lb in &lines[a + 1..] {
            for p in la {
                for q in lb {
                    best = best.min((p[0] - q[0]).hypot(p[1] - q[1]));
                }
            }
        }
    }
    best
}

// --- rule 7: line integral convolution (Cabral and Leedom 1993) ------------------------------------------------------

/// The raster of the box kernel of half-length `length` along the streamline through each pixel centre of `noise`
/// (rows from y0, columns from x0): the mean of the noise at the nearest pixel of each sample at arc length 0,
/// ±h, ±2h, … up to `length` each way, samples outside the raster skipped.
pub fn lic(
    f: impl Fn(f64, f64) -> [f64; 2],
    noise: &[Vec<f64>],
    ranges: [f64; 4],
    length: f64,
    h: f64,
) -> Result<Vec<Vec<f64>>, FieldError> {
    if noise.is_empty() || noise[0].is_empty() || noise.iter().any(|r| r.len() != noise[0].len()) {
        return Err(FieldError::Lic);
    }
    if !(positive(length) && positive(h)) {
        return Err(FieldError::Lic);
    }
    let [x0, y0, x1, y1] = ranges;
    let (ny, nx) = (noise.len(), noise[0].len());
    let (pw, ph) = ((x1 - x0) / nx as f64, (y1 - y0) / ny as f64);
    let at = |p: [f64; 2]| {
        let i = ((p[0] - x0) / pw).floor();
        let j = ((p[1] - y0) / ph).floor();
        (0.0 <= i && i < nx as f64 && 0.0 <= j && j < ny as f64).then(|| noise[j as usize][i as usize])
    };
    let n = (length / h + 1e-9).floor() as u64;
    let mut out = Vec::with_capacity(ny);
    for (j, row) in noise.iter().enumerate() {
        let mut line = Vec::with_capacity(nx);
        for (i, &v0) in row.iter().enumerate() {
            let c = [x0 + (i as f64 + 0.5) * pw, y0 + (j as f64 + 0.5) * ph];
            let (mut acc, mut cnt) = (v0, 1.0);
            for sgn in [1.0, -1.0] {
                let mut p = c;
                for _ in 0..n {
                    let Some(q) = step(&f, p, sgn * h) else { break };
                    if let Some(v) = at(q) {
                        acc += v;
                        cnt += 1.0;
                    }
                    p = q;
                }
            }
            line.push(acc / cnt);
        }
        out.push(line);
    }
    Ok(out)
}

// --- rules 8 and 9: phase portraits (Wegert 2012) ---------------------------------------------------------------------

fn nonzero_finite(w: [f64; 2]) -> bool {
    !(w[0] == 0.0 && w[1] == 0.0) && w[0].is_finite() && w[1].is_finite()
}

/// The hue of `w` in [0, 1): arg w / 2π modulo 1; 0 for 0 and for a non-finite `w`.
pub fn phase_hue(w: [f64; 2]) -> f64 {
    if !nonzero_finite(w) {
        return 0.0;
    }
    (w[1].atan2(w[0]) / TAU).rem_euclid(1.0)
}

/// The fractional part of log_base |w| in [0, 1): 0 at powers of the base, and 0 for 0 and a non-finite `w`.
pub fn modulus_shade(w: [f64; 2], base: f64) -> f64 {
    let r = w[0].hypot(w[1]);
    if r == 0.0 || !r.is_finite() {
        return 0.0;
    }
    (r.ln() / base.ln()).rem_euclid(1.0)
}

/// The turns of arg f(z) along the circle of `radius` about `center`, counterclockwise, from the wrapped
/// differences of arg at `samples` equally spaced points: zeros minus poles inside, with multiplicity. A zero or
/// non-finite value on a sample is an error.
pub fn winding_number(
    f: impl Fn([f64; 2]) -> [f64; 2],
    center: [f64; 2],
    radius: f64,
    samples: usize,
) -> Result<i64, FieldError> {
    if !positive(radius) || samples < 3 {
        return Err(FieldError::Winding);
    }
    let mut args = Vec::with_capacity(samples);
    for k in 0..samples {
        let a = TAU * k as f64 / samples as f64;
        let w = f([center[0] + radius * a.cos(), center[1] + radius * a.sin()]);
        if !nonzero_finite(w) {
            return Err(FieldError::Winding);
        }
        args.push(w[1].atan2(w[0]));
    }
    let mut total = 0.0;
    for (k, a) in args.iter().enumerate() {
        let b = args[(k + 1) % samples];
        total += (b - a + PI).rem_euclid(TAU) - PI;
    }
    Ok((total / TAU).round() as i64)
}

// --- rule 10: dipole and charge lines ---------------------------------------------------------------------------------

/// r / sin²θ, θ the angle between p − center and the dipole axis: constant along a field line of a point dipole.
pub fn dipole_line_constant(p: [f64; 2], m: [f64; 2], center: [f64; 2]) -> Result<f64, FieldError> {
    let (rx, ry) = (p[0] - center[0], p[1] - center[1]);
    let (r, mn) = (rx.hypot(ry), m[0].hypot(m[1]));
    if r == 0.0 || mn == 0.0 {
        return Err(FieldError::Dipole);
    }
    let cos_t = (rx * m[0] + ry * m[1]) / (r * mn);
    let sin2 = 1.0 - cos_t * cos_t;
    if sin2 <= 1e-12 {
        return Err(FieldError::Dipole);
    }
    Ok(r / sin2)
}

/// The largest relative change of the dipole line constant along `steps` steps of length `h` from `p0`, forward.
pub fn dipole_line_deviation(p0: [f64; 2], h: f64, steps: u64, m: [f64; 2]) -> Result<f64, FieldError> {
    let f = |x, y| dipole_field([x, y], m, [0.0, 0.0]);
    let t = Trace { h, max_steps: steps, both: false, keep_stop: false };
    let line = streamline(f, p0, &t, |_| true, None)?;
    let c0 = dipole_line_constant(p0, m, [0.0, 0.0])?;
    let mut worst: f64 = 0.0;
    for p in line {
        worst = worst.max((dipole_line_constant(p, m, [0.0, 0.0])? / c0 - 1.0).abs());
    }
    Ok(worst)
}

/// The attributes of rule 10's charge lines.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChargeLines {
    /// Lines from each positive charge.
    pub per_charge: u32,
    /// The starting radius, and the radius at which a line ends on a negative charge.
    pub r0: f64,
    /// The step.
    pub h: f64,
    /// The step bound of one line.
    pub max_steps: u64,
    /// The box, [x0, y0, x1, y1].
    pub ranges: [f64; 4],
}

impl Default for ChargeLines {
    fn default() -> ChargeLines {
        ChargeLines { per_charge: 8, r0: 0.05, h: 0.01, max_steps: 5000, ranges: [-5.0, -5.0, 5.0, 5.0] }
    }
}

/// Field lines of point charges [x, y, q]: from each positive charge, `per_charge` lines start at `r0` at angles
/// 2πk / `per_charge` and follow the field forward until they come within `r0` of a negative charge (that point is
/// the line's last), leave the box, or reach `max_steps`.
pub fn charge_lines(charges: &[[f64; 3]], c: &ChargeLines) -> Result<Vec<Vec<[f64; 2]>>, FieldError> {
    if c.per_charge < 1 {
        return Err(FieldError::Bound);
    }
    let f = |x, y| charge_field(charges, [x, y]);
    let negatives: Vec<[f64; 2]> = charges.iter().filter(|q| q[2] < 0.0).map(|q| [q[0], q[1]]).collect();
    let stop = |p: [f64; 2]| negatives.iter().any(|n| (p[0] - n[0]).hypot(p[1] - n[1]) < c.r0);
    let t = Trace { h: c.h, max_steps: c.max_steps, both: false, keep_stop: true };
    let mut out = Vec::new();
    for &[cx, cy, q] in charges {
        if q <= 0.0 {
            continue;
        }
        for k in 0..c.per_charge {
            let a = TAU * k as f64 / c.per_charge as f64;
            let p0 = [cx + c.r0 * a.cos(), cy + c.r0 * a.sin()];
            out.push(streamline(f, p0, &t, inside_box(c.ranges), Some(&stop))?);
        }
    }
    Ok(out)
}

/// The index of the charge within `r0` of the line's last point.
pub fn line_ends_on(line: &[[f64; 2]], charges: &[[f64; 3]], r0: f64) -> Option<usize> {
    let [x, y] = *line.last()?;
    charges.iter().position(|c| (x - c[0]).hypot(y - c[1]) <= r0 + 1e-9)
}
