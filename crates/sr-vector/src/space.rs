//! The space transforms of the space-transforms draft of sr-core (`srep/srep-0000-space-transforms.md`, Semantics 1
//! to 8): the `matrix`, `map` and `complex` deform modifiers as point maps, the adaptive subdivision of a segment
//! under a map (rule 4), the mapped polyline that breaks at non-finite points (rule 5), the conformality check
//! (rule 6), the stereographic projection of the 3-sphere (rule 7) and the Penrose compactification (rule 8).
//! Every function is a pure function of its arguments.

/// The mapped chord error, in pixels, a subdivided segment may keep (rule 4).
pub const DEFAULT_TOLERANCE: f64 = 0.25;

/// The most times a segment is split along any branch (rule 4): at most 2¹⁰ sub-segments.
pub const MAX_DEPTH: u32 = 10;

/// Rule 1: the SVG matrix (a, b, c, d, e, f) about `center`:
/// x' = cx + a (x − cx) + c (y − cy) + e, y' = cy + b (x − cx) + d (y − cy) + f.
pub fn matrix_map(m: [f64; 6], q: [f64; 2], center: [f64; 2]) -> [f64; 2] {
    let [a, b, c, d, e, f] = m;
    let (x, y) = (q[0] - center[0], q[1] - center[1]);
    [center[0] + a * x + c * y + e, center[1] + b * x + d * y + f]
}

/// The `cols` × `rows` nodes of a regular grid over `[x0, y0, x1, y1]`, row-major from the top-left; none under two
/// columns or rows.
pub fn grid_nodes(cols: usize, rows: usize, b: [f64; 4]) -> Vec<[f64; 2]> {
    if cols < 2 || rows < 2 {
        return Vec::new();
    }
    let [x0, y0, x1, y1] = b;
    (0..rows)
        .flat_map(|j| {
            (0..cols).map(move |i| {
                [x0 + i as f64 * (x1 - x0) / (cols - 1) as f64, y0 + j as f64 * (y1 - y0) / (rows - 1) as f64]
            })
        })
        .collect()
}

/// Rule 2: the pixel `q` read as data (u, v) = ((x − cx)/unit, (cy − y)/unit), +v up; `f` gives (u', v') and the
/// pixel is (cx + unit·u', cy − unit·v').
pub fn expr_map(f: impl Fn(f64, f64) -> [f64; 2], q: [f64; 2], center: [f64; 2], unit: f64) -> [f64; 2] {
    let (u, v) = ((q[0] - center[0]) / unit, (center[1] - q[1]) / unit);
    let [u2, v2] = f(u, v);
    [center[0] + unit * u2, center[1] - unit * v2]
}

/// A complex number.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Complex {
    pub re: f64,
    pub im: f64,
}

impl Complex {
    const ONE: Complex = Complex { re: 1.0, im: 0.0 };
    const INFINITE: Complex = Complex { re: f64::INFINITY, im: f64::INFINITY };

    fn add(self, o: Complex) -> Complex {
        Complex { re: self.re + o.re, im: self.im + o.im }
    }

    fn mul(self, o: Complex) -> Complex {
        Complex { re: self.re * o.re - self.im * o.im, im: self.re * o.im + self.im * o.re }
    }

    /// The quotient; a zero divisor is a pole, infinite.
    fn div(self, o: Complex) -> Complex {
        let d = o.re * o.re + o.im * o.im;
        if d == 0.0 {
            return Complex::INFINITE;
        }
        Complex { re: (self.re * o.re + self.im * o.im) / d, im: (self.im * o.re - self.re * o.im) / d }
    }

    fn is_finite(self) -> bool {
        self.re.is_finite() && self.im.is_finite()
    }
}

/// The named functions of rule 3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComplexFn {
    Square,
    Cube,
    Inverse,
    Exp,
    Log,
    Sqrt,
    Sin,
    Cos,
    Mobius,
    Joukowski,
}

impl ComplexFn {
    /// The function a `function` attribute names.
    pub fn parse(s: &str) -> Option<ComplexFn> {
        Some(match s {
            "square" => ComplexFn::Square,
            "cube" => ComplexFn::Cube,
            "inverse" => ComplexFn::Inverse,
            "exp" => ComplexFn::Exp,
            "log" => ComplexFn::Log,
            "sqrt" => ComplexFn::Sqrt,
            "sin" => ComplexFn::Sin,
            "cos" => ComplexFn::Cos,
            "mobius" => ComplexFn::Mobius,
            "joukowski" => ComplexFn::Joukowski,
            _ => return None,
        })
    }
}

/// Rule 3: w = f(z), on the principal branch where one is needed (`log` with its imaginary part in (−π, π], `sqrt`
/// with a non-negative real part). `mobius` reads (a, b, c, d) of (a z + b)/(c z + d) from eight coefficients, real
/// and imaginary parts in turn. A pole gives a non-finite w, and so does a Möbius map without eight coefficients or
/// with ad − bc = 0, which validation refuses (SPT1, SPT2).
pub fn complex_function(kind: ComplexFn, z: Complex, coefficients: &[f64]) -> Complex {
    let w = match kind {
        ComplexFn::Square => z.mul(z),
        ComplexFn::Cube => z.mul(z).mul(z),
        ComplexFn::Inverse => Complex::ONE.div(z),
        ComplexFn::Exp => {
            let r = z.re.exp();
            Complex { re: r * z.im.cos(), im: r * z.im.sin() }
        }
        ComplexFn::Log => {
            if z.re == 0.0 && z.im == 0.0 {
                return Complex::INFINITE;
            }
            Complex { re: z.re.hypot(z.im).ln(), im: z.im.atan2(z.re) }
        }
        ComplexFn::Sqrt => {
            // the root with a non-negative real part, its imaginary part on the side of z's
            let m = z.re.hypot(z.im);
            let re = ((m + z.re) / 2.0).sqrt();
            let im = ((m - z.re) / 2.0).sqrt().copysign(z.im);
            Complex { re, im }
        }
        ComplexFn::Sin => Complex { re: z.re.sin() * z.im.cosh(), im: z.re.cos() * z.im.sinh() },
        ComplexFn::Cos => Complex { re: z.re.cos() * z.im.cosh(), im: -(z.re.sin() * z.im.sinh()) },
        ComplexFn::Joukowski => z.add(Complex::ONE.div(z)),
        ComplexFn::Mobius => {
            let [a, b, c, d] = match coefficients {
                [a0, a1, b0, b1, c0, c1, d0, d1] => {
                    [[*a0, *a1], [*b0, *b1], [*c0, *c1], [*d0, *d1]].map(|[re, im]| Complex { re, im })
                }
                _ => return Complex::INFINITE,
            };
            let det = a.mul(d).add(b.mul(c).mul(Complex { re: -1.0, im: 0.0 }));
            if det.re == 0.0 && det.im == 0.0 {
                return Complex::INFINITE;
            }
            a.mul(z).add(b).div(c.mul(z).add(d))
        }
    };
    if w.is_finite() {
        w
    } else {
        Complex::INFINITE
    }
}

/// Rule 3: the map of rule 2 with z = u + iv and (u', v') = (Re w, Im w).
pub fn complex_map(kind: ComplexFn, coefficients: &[f64], q: [f64; 2], center: [f64; 2], unit: f64) -> [f64; 2] {
    expr_map(
        |u, v| {
            let w = complex_function(kind, Complex { re: u, im: v }, coefficients);
            [w.re, w.im]
        },
        q,
        center,
        unit,
    )
}

fn finite(q: [f64; 2]) -> bool {
    q[0].is_finite() && q[1].is_finite()
}

fn mid(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// Rule 4: the source points, `p` to `q` inclusive, at which the segment is sampled so that at every kept
/// sub-segment the map of its midpoint is within `tol` of the midpoint of its mapped ends. A segment is split at
/// its midpoint at most [`MAX_DEPTH`] times along any branch; one with a non-finite mapped end or midpoint is split
/// down to the bound, and so is every segment under a tolerance that is not positive.
pub fn subdivide_segment(f: impl Fn([f64; 2]) -> [f64; 2], p: [f64; 2], q: [f64; 2], tol: f64) -> Vec<[f64; 2]> {
    let mut out = vec![p];
    // an explicit stack, the second half under the first, so the points come out in order from p to q
    let mut stack = vec![(p, q, f(p), f(q), 0u32)];
    while let Some((a, b, fa, fb, depth)) = stack.pop() {
        let m = mid(a, b);
        let fm = f(m);
        let kept = finite(fa) && finite(fb) && finite(fm) && dist(fm, mid(fa, fb)) <= tol;
        if depth < MAX_DEPTH && !kept {
            stack.push((m, b, fm, fb, depth + 1));
            stack.push((a, m, fa, fm, depth + 1));
        } else {
            out.push(b);
        }
    }
    out
}

/// The largest distance between the map of a sub-segment's midpoint and the midpoint of its mapped ends.
pub fn chord_error(f: impl Fn([f64; 2]) -> [f64; 2], points: &[[f64; 2]]) -> f64 {
    points.windows(2).map(|w| dist(f(mid(w[0], w[1])), mid(f(w[0]), f(w[1])))).fold(0.0, f64::max)
}

/// Rule 5: the polyline under the map, each segment subdivided by rule 4 and mapped. A non-finite mapped point is
/// dropped and breaks the path (SREP 70), so the result is a list of subpaths. A closed polyline maps its closing
/// segment and, when unbroken, ends on its first mapped point. A lone point maps to itself when its image is finite.
pub fn deform_polyline(
    f: impl Fn([f64; 2]) -> [f64; 2],
    points: &[[f64; 2]],
    tol: f64,
    closed: bool,
) -> Vec<Vec<[f64; 2]>> {
    let Some(&first) = points.first() else { return Vec::new() };
    let mut source = vec![first];
    let closing = if closed && points.len() > 1 { Some(first) } else { None };
    for w in points.windows(2).map(|w| (w[0], w[1])).chain(closing.map(|c| (points[points.len() - 1], c))) {
        source.extend(subdivide_segment(&f, w.0, w.1, tol).into_iter().skip(1));
    }
    let mut parts: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut cur: Vec<[f64; 2]> = Vec::new();
    for s in source {
        let w = f(s);
        if finite(w) {
            cur.push(w);
        } else if !cur.is_empty() {
            parts.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    parts
}

/// Rule 6, the test of a conformal map: the angle in degrees between the images of the directions `d1` and `d2`
/// at `q`, from central differences of step 10⁻⁶ along each.
pub fn tangent_angle(f: impl Fn([f64; 2]) -> [f64; 2], q: [f64; 2], d1: [f64; 2], d2: [f64; 2]) -> f64 {
    let h = 1e-6;
    let image = |d: [f64; 2]| {
        let n = d[0].hypot(d[1]);
        let (dx, dy) = (d[0] / n * h, d[1] / n * h);
        let (a, b) = (f([q[0] - dx, q[1] - dy]), f([q[0] + dx, q[1] + dy]));
        [b[0] - a[0], b[1] - a[1]]
    };
    let (e1, e2) = (image(d1), image(d2));
    let cos = (e1[0] * e2[0] + e1[1] * e2[1]) / (e1[0].hypot(e1[1]) * e2[0].hypot(e2[1]));
    cos.clamp(-1.0, 1.0).acos().to_degrees()
}

/// Rule 7: the point (x, y, z, w) of the 3-sphere projected from the pole w = 1 onto w = 0, (x, y, z)/(1 − w); the
/// pole itself is infinite.
pub fn stereographic(p: [f64; 4]) -> [f64; 3] {
    let d = 1.0 - p[3];
    if d == 0.0 {
        return [f64::INFINITY; 3];
    }
    [p[0] / d, p[1] / d, p[2] / d]
}

/// Rule 7: the point of the 3-sphere that projects to (x, y, z): (2x, 2y, 2z, r² − 1)/(r² + 1).
pub fn inverse_stereographic(q: [f64; 3]) -> [f64; 4] {
    let r2 = q[0] * q[0] + q[1] * q[1] + q[2] * q[2];
    [2.0 * q[0] / (r2 + 1.0), 2.0 * q[1] / (r2 + 1.0), 2.0 * q[2] / (r2 + 1.0), (r2 - 1.0) / (r2 + 1.0)]
}

/// Rule 8: the Penrose compactification of 1+1 Minkowski space, (T, X) of (t, x): with u = t − x and v = t + x,
/// T = (atan v + atan u)/2 and X = (atan v − atan u)/2.
pub fn penrose(t: f64, x: f64) -> [f64; 2] {
    let (au, av) = ((t - x).atan(), (t + x).atan());
    [(av + au) / 2.0, (av - au) / 2.0]
}
