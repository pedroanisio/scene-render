//! A `spacetime` diagram in an `axes` (sr-core srep/srep-0000-relativity.md, Semantics 1 and 5): what its parts
//! draw at a frame, in the box's pixel space. The evaluator lowers the diagram to path shapes for its grid, the
//! light cones of its events, its worldlines, its light rays and its event points (see
//! [`crate::program::AxesPart::Spacetime`]); this module computes each part's path data and the labels of its events
//! and worldlines from the frame's values of the diagram's attributes, every event through the kernel's
//! `diagram_pixel` (boost, compactification, then the axes mapping).
//!
//! Children's times are data units of t; the diagram is drawn in the units where light moves one unit of x per
//! unit of its y axis, so a time t is drawn at c t (the y axis's tick labels are divided by c to read t again).

use sr_3d::relativity::{self as rel, AxesBox, Compact};

use crate::axes::{path_data, Align, AxesFrame, Label};
use crate::plots::{self, LABEL_GAP};

/// A part of a diagram, drawn by one path shape, in paint order: the grid under the cones, the cones under the
/// lines, the event points on top.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Grid,
    /// The future (`true`) or past cone of an event.
    Cone(u32, bool),
    Worldline(u32),
    Ray(u32),
    Event(u32),
}

/// An `event`.
#[derive(Debug, Clone, PartialEq)]
pub struct EventInst {
    pub x: f64,
    pub t: f64,
    pub light_cone: bool,
    /// How far the cones reach, in units of t; else the box's yMax − yMin (drawn units).
    pub extent: Option<f64>,
    pub marker_size: f64,
    pub label: Option<String>,
}

/// How a worldline moves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Motion {
    /// At speed β.
    Inertial(f64),
    /// The Rindler hyperbola of proper acceleration a (units of c per unit of t) from rest.
    Accelerated(f64),
    /// x(τ) and t(τ), compiled expressions of `tau` (indices into `Program::exprs`).
    Parametric([u32; 2]),
}

/// A `worldline`.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldlineInst {
    pub motion: Motion,
    pub x0: f64,
    pub t0: f64,
    /// The proper-time range, in units of t; each else ∓ the box's yMax − yMin.
    pub tau: [Option<f64>; 2],
    pub samples: usize,
    pub label: Option<String>,
}

/// A `lightRay`.
#[derive(Debug, Clone, PartialEq)]
pub struct RayInst {
    pub x: f64,
    pub t: f64,
    /// +1 or −1: the ray's direction in x.
    pub direction: f64,
    /// In drawn units; else the box's yMax − yMin.
    pub length: Option<f64>,
}

/// A `spacetime` with its children, as built.
#[derive(Debug, Clone, PartialEq)]
pub struct SpacetimeInst {
    pub id: String,
    pub c: f64,
    pub boost: f64,
    pub grid: bool,
    pub compact: Compact,
    pub events: Vec<EventInst>,
    pub worldlines: Vec<WorldlineInst>,
    pub rays: Vec<RayInst>,
}

/// A diagram of the program: as built, and the element target of its `spacetime`, when it animates.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagram {
    /// The `axes` node.
    pub axes: u32,
    pub inst: SpacetimeInst,
    pub own: Option<u32>,
}

/// The diagram's attribute values at a frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub beta: f64,
    pub c: f64,
    pub grid: bool,
    pub compact: Compact,
}

/// Subpaths of path data: each its pixels and whether it is closed.
type Subpaths = Vec<(Vec<[f64; 2]>, bool)>;

/// Vertices of an event's point.
const POINT_SEGMENTS: usize = 32;
/// Vertices of a line that a compactification bends (a straight line of the plane is sampled at its ends).
const COMPACT_SAMPLES: usize = 512;
/// Lines on each side of 0 that the grid draws under a compactification, which brings infinity into the box.
const COMPACT_GRID_LINES: i64 = 8;

/// Reads `x` as a kernel error's text.
fn rel_err(e: rel::RelativityError) -> String {
    e.0
}

/// The box of an axes frame, in its own pixel space.
fn box_of(af: &AxesFrame) -> AxesBox {
    let a = &af.axes;
    AxesBox {
        x: 0.0,
        y: 0.0,
        width: a.width,
        height: a.height,
        x_min: a.x_min,
        x_max: a.x_max,
        y_min: a.y_min,
        y_max: a.y_max,
    }
}

/// The box's extent: yMax − yMin, in drawn units.
fn extent(af: &AxesFrame) -> f64 {
    af.axes.y_max - af.axes.y_min
}

/// The pixel of a drawn-unit event (x, c t) of the box frame: boost, compactify, map.
fn pixel(v: &View, b: &AxesBox, x: f64, ct: f64) -> Result<[f64; 2], String> {
    let (px, py) = rel::diagram_pixel(x, ct, b, v.beta, v.compact).map_err(rel_err)?;
    Ok([px, py])
}

/// The pixel of a point already in the drawn frame (the grid's): compactified and mapped with no boost.
fn drawn_pixel(v: &View, b: &AxesBox, x: f64, t: f64) -> Result<[f64; 2], String> {
    let (px, py) = rel::diagram_pixel(x, t, b, 0.0, v.compact).map_err(rel_err)?;
    Ok([px, py])
}

/// `n` parameters from `lo` to `hi`, both included.
fn linspace(lo: f64, hi: f64, n: usize) -> Vec<f64> {
    let n = n.max(2);
    (0..n).map(|i| lo + (hi - lo) * i as f64 / (n - 1) as f64).collect()
}

/// The parameters of a line of the plane: its two ends when nothing bends it, else the whole line through
/// tan of an open angle, so that both ends reach infinity's place in the compactified box.
fn line_params(v: &View, lo: f64, hi: f64) -> Vec<f64> {
    if v.compact == Compact::None {
        return vec![lo, hi];
    }
    let h = std::f64::consts::FRAC_PI_2;
    linspace(-h, h, COMPACT_SAMPLES + 2)[1..=COMPACT_SAMPLES].iter().map(|s| s.tan()).collect()
}

/// A polyline of pixels, split at non-finite points and clipped to the box.
fn clipped(af: &AxesFrame, px: &[[f64; 2]]) -> Subpaths {
    let (w, h) = (af.axes.width, af.axes.height);
    plots::split_finite(px)
        .into_iter()
        .flat_map(|run| plots::clip_polyline(&run, 0.0, 0.0, w, h))
        .map(|p| (p, false))
        .collect()
}

/// A polygon clipped to the box (Sutherland–Hodgman against its four edges).
fn clip_polygon(af: &AxesFrame, poly: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let (w, h) = (af.axes.width, af.axes.height);
    let mut out = poly.to_vec();
    for (axis, bound, below) in [(0, 0.0, false), (0, w, true), (1, 0.0, false), (1, h, true)] {
        let inside = |p: &[f64; 2]| if below { p[axis] <= bound } else { p[axis] >= bound };
        let input = std::mem::take(&mut out);
        for (i, cur) in input.iter().enumerate() {
            let prev = input[(i + input.len() - 1) % input.len()];
            let cross = |a: [f64; 2], b: [f64; 2]| {
                let t = (bound - a[axis]) / (b[axis] - a[axis]);
                let mut c = [0.0; 2];
                c[axis] = bound;
                c[1 - axis] = a[1 - axis] + t * (b[1 - axis] - a[1 - axis]);
                c
            };
            match (inside(&prev), inside(cur)) {
                (true, true) => out.push(*cur),
                (true, false) => out.push(cross(prev, *cur)),
                (false, true) => {
                    out.push(cross(prev, *cur));
                    out.push(*cur);
                }
                (false, false) => {}
            }
        }
        if out.is_empty() {
            break;
        }
    }
    out
}

/// The rest-frame range of x and of c t over the box's corners, which a diagram at boost β draws: the box frame's
/// lines and the straight lines that cross the box lie within it.
fn rest_ranges(v: &View, af: &AxesFrame) -> Result<[[f64; 2]; 2], String> {
    let a = &af.axes;
    let mut r = [[f64::INFINITY, f64::NEG_INFINITY]; 2];
    for (x, t) in [(a.x_min, a.y_min), (a.x_min, a.y_max), (a.x_max, a.y_min), (a.x_max, a.y_max)] {
        let (xr, tr) = rel::inverse_boost(x, t, v.beta).map_err(rel_err)?;
        r[0] = [r[0][0].min(xr), r[0][1].max(xr)];
        r[1] = [r[1][0].min(tr), r[1][1].max(tr)];
    }
    Ok(r)
}

/// The grid: the box frame's lines x = k and t = k at the axes' tick steps, where the boost carries them
/// (`diagram_grid`, the box frame seen from the drawn frame, which it leaves at −β).
fn grid(v: &View, af: &AxesFrame) -> Result<Subpaths, String> {
    let b = box_of(af);
    let a = &af.axes;
    let [sx, st] = af.steps;
    let ks = |lo: f64, hi: f64, step: f64| -> Result<Vec<f64>, String> {
        if v.compact == Compact::None {
            plots::ticks(lo, hi, step).map_err(|e| e.to_string())
        } else {
            Ok((-COMPACT_GRID_LINES..=COMPACT_GRID_LINES).map(|k| k as f64 * step).collect())
        }
    };
    let [xr, tr] = rest_ranges(v, af)?;
    let mut out = Vec::new();
    for k in ks(xr[0], xr[1], sx)? {
        let px = line_params(v, a.y_min, a.y_max)
            .into_iter()
            .map(|t| {
                let (x, t) = rel::grid_x_line(-v.beta, k, t).map_err(rel_err)?;
                drawn_pixel(v, &b, x, t)
            })
            .collect::<Result<Vec<_>, String>>()?;
        out.extend(clipped(af, &px));
    }
    for k in ks(tr[0], tr[1], st)? {
        let px = line_params(v, a.x_min, a.x_max)
            .into_iter()
            .map(|x| {
                let (x, t) = rel::grid_t_line(-v.beta, k, x).map_err(rel_err)?;
                drawn_pixel(v, &b, x, t)
            })
            .collect::<Result<Vec<_>, String>>()?;
        out.extend(clipped(af, &px));
    }
    Ok(out)
}

/// The future or past cone of an event: the triangle of the event and its cone's two ends, filled, its edges
/// sampled when a compactification bends them.
fn cone(v: &View, af: &AxesFrame, e: &EventInst, future: bool) -> Result<Subpaths, String> {
    let b = box_of(af);
    let reach = e.extent.map_or(extent(af), |x| x * v.c);
    let apex = (e.x, e.t * v.c);
    let c = rel::light_cone(apex, reach).map_err(rel_err)?;
    let ends = if future { c.future } else { c.past };
    let corners = [apex, ends[0], ends[1]];
    let n = if v.compact == Compact::None { 1 } else { COMPACT_SAMPLES };
    let mut poly = Vec::new();
    for i in 0..3 {
        let (p, q) = (corners[i], corners[(i + 1) % 3]);
        for s in 0..n {
            let u = s as f64 / n as f64;
            poly.push(pixel(v, &b, p.0 + (q.0 - p.0) * u, p.1 + (q.1 - p.1) * u)?);
        }
    }
    if !poly.iter().flatten().all(|c| c.is_finite()) {
        return Err("a light cone has a point the compactification cannot place".into());
    }
    let poly = clip_polygon(af, &poly);
    Ok(if poly.len() >= 3 { vec![(poly, true)] } else { vec![] })
}

/// An event's point: a circle `marker_size` across about its pixel, when the pixel is inside the box.
fn point(v: &View, af: &AxesFrame, e: &EventInst) -> Result<Subpaths, String> {
    let p = pixel(v, &box_of(af), e.x, e.t * v.c)?;
    let inside = (0.0..=af.axes.width).contains(&p[0]) && (0.0..=af.axes.height).contains(&p[1]);
    if !inside {
        return Ok(vec![]);
    }
    let r = e.marker_size / 2.0;
    let circle = (0..POINT_SEGMENTS)
        .map(|i| {
            let th = i as f64 * std::f64::consts::TAU / POINT_SEGMENTS as f64;
            [p[0] + r * th.cos(), p[1] + r * th.sin()]
        })
        .collect();
    Ok(vec![(circle, true)])
}

/// The drawn-unit events (x, c t) of a worldline, before the boost.
fn worldline_events(
    v: &View,
    af: &AxesFrame,
    w: &WorldlineInst,
    eval: &mut dyn FnMut(u32, &[f64]) -> Vec<f64>,
) -> Result<Vec<(f64, f64)>, String> {
    let e = extent(af) / v.c;
    let (tau0, tau1) = (w.tau[0].unwrap_or(-e), w.tau[1].unwrap_or(e));
    let (x0, ct0) = (w.x0, w.t0 * v.c);
    match w.motion {
        Motion::Inertial(beta) => {
            let [_, tr] = rest_ranges(v, af)?;
            line_params(v, tr[0], tr[1])
                .into_iter()
                .map(|ct| rel::worldline_inertial(x0, ct0, beta, ct).map_err(rel_err))
                .collect()
        }
        Motion::Accelerated(a) => linspace(tau0, tau1, w.samples)
            .into_iter()
            .map(|tau| {
                let (x, t) = rel::worldline_accelerated(a, tau).map_err(rel_err)?;
                Ok((x0 + v.c * x, ct0 + v.c * t))
            })
            .collect(),
        Motion::Parametric([fx, ft]) => {
            // each τ with its neighbours τ ± h, so that the kernel checks the tangent on the expressions' values
            const H: f64 = 1e-6;
            let taus = linspace(tau0, tau1, w.samples);
            let at: Vec<f64> = taus.iter().flat_map(|&t| [t - H, t, t + H]).collect();
            let (xs, ts) = (eval(fx, &at), eval(ft, &at));
            taus.iter()
                .enumerate()
                .map(|(i, &tau)| {
                    let pick = |vals: &[f64], s: f64| match s.partial_cmp(&tau) {
                        Some(std::cmp::Ordering::Less) => vals[3 * i],
                        Some(std::cmp::Ordering::Greater) => vals[3 * i + 2],
                        _ => vals[3 * i + 1],
                    };
                    rel::worldline_parametric(|s| pick(&xs, s), |s| v.c * pick(&ts, s), tau).map_err(rel_err)
                })
                .collect()
        }
    }
}

/// A worldline's pixels, clipped to the box.
fn worldline(
    v: &View,
    af: &AxesFrame,
    w: &WorldlineInst,
    eval: &mut dyn FnMut(u32, &[f64]) -> Vec<f64>,
) -> Result<Subpaths, String> {
    let b = box_of(af);
    let px = worldline_events(v, af, w, eval)?
        .into_iter()
        .map(|(x, ct)| pixel(v, &b, x, ct))
        .collect::<Result<Vec<_>, String>>()?;
    Ok(clipped(af, &px))
}

/// A light ray: from its event along (direction, 1) for its length.
fn ray(v: &View, af: &AxesFrame, r: &RayInst) -> Result<Subpaths, String> {
    let b = box_of(af);
    let len = r.length.unwrap_or(extent(af));
    let n = if v.compact == Compact::None { 2 } else { COMPACT_SAMPLES };
    let px = linspace(0.0, len, n)
        .into_iter()
        .map(|s| pixel(v, &b, r.x + r.direction * s, r.t * v.c + s))
        .collect::<Result<Vec<_>, String>>()?;
    Ok(clipped(af, &px))
}

/// The path data of `part` of diagram `d` at the frame's values `v`; `eval` samples a compiled expression of `tau`.
pub fn part_path(
    d: &SpacetimeInst,
    part: Part,
    v: &View,
    af: &AxesFrame,
    eval: &mut dyn FnMut(u32, &[f64]) -> Vec<f64>,
) -> Result<String, String> {
    let subpaths = match part {
        Part::Grid if v.grid => grid(v, af)?,
        Part::Grid => vec![],
        Part::Cone(i, future) => cone(v, af, &d.events[i as usize], future)?,
        Part::Event(i) => point(v, af, &d.events[i as usize])?,
        Part::Worldline(i) => {
            worldline(v, af, &d.worldlines[i as usize], eval).map_err(|e| format!("worldline{i}: {e}"))?
        }
        Part::Ray(i) => ray(v, af, &d.rays[i as usize])?,
    };
    Ok(path_data(&subpaths))
}

/// A label's text as TeX: `\text` takes its argument as written, so its braces must pair.
fn tex(s: &str) -> Result<String, String> {
    let mut depth = 0i64;
    for ch in s.chars() {
        depth += match ch {
            '{' => 1,
            '}' => -1,
            _ => 0,
        };
        if depth < 0 {
            break;
        }
    }
    if depth != 0 {
        return Err(format!("the label {s:?} has unpaired braces"));
    }
    Ok(format!("\\text{{{s}}}"))
}

/// The labels of the diagram's events (beside their points) and worldlines (beside the last point drawn), and the
/// y axis's tick labels divided by c.
pub fn labels(
    d: &SpacetimeInst,
    v: &View,
    af: &mut AxesFrame,
    eval: &mut dyn FnMut(u32, &[f64]) -> Vec<f64>,
) -> Result<(), String> {
    if v.c != 1.0 {
        let [_, st] = af.steps;
        for l in af.labels.iter_mut().filter(|l| l.align == Align::Left) {
            let value: f64 = plots::from_pixel(&af.axes, 0.0, l.at[1])[1];
            let tick = (value / st).round() * st;
            l.tex = plots::tick_label(tick / v.c, st / v.c, false).map_err(|e| e.to_string())?;
        }
    }
    let b = box_of(af);
    let mut out = Vec::new();
    for e in &d.events {
        let Some(text) = &e.label else { continue };
        let p = pixel(v, &b, e.x, e.t * v.c)?;
        let r = e.marker_size / 2.0 + LABEL_GAP;
        out.push(Label { tex: tex(text)?, at: [p[0] + r, p[1] + r], align: Align::BelowRight });
    }
    for (i, w) in d.worldlines.iter().enumerate() {
        let Some(text) = &w.label else { continue };
        let drawn = worldline(v, af, w, eval).map_err(|e| format!("worldline{i}: {e}"))?;
        if let Some(end) = drawn.last().and_then(|(p, _)| p.last()) {
            out.push(Label { tex: tex(text)?, at: [end[0] + LABEL_GAP, end[1] + LABEL_GAP], align: Align::BelowRight });
        }
    }
    af.labels.extend(out);
    Ok(())
}
