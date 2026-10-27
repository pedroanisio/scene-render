//! An `axes` node at a frame (sr-core srep/srep-0000-plots.md, Semantics 1 to 9): its box and ranges, the
//! polylines of its grid, axis lines and ticks in the box's pixel space, its tick labels, and the path data of its
//! plots. The evaluator lowers an `axes` to path shapes for these parts (see [`crate::program::AxesPart`]); this
//! module computes what each part draws from the frame's values of the `axes` attributes.

use std::fmt::Write as _;

use serde::Serialize;
use sr_model::model as m;

use crate::plots::{self, Axes, PlotError, LABEL_GAP};

/// What an `axes` box shows (Semantics 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AxesKind {
    Cartesian,
    Complex,
    Polar,
    NumberLine,
}

/// How a label's text sits on its point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Align {
    /// Centred below the point: the point is the middle of the text's top edge (x-axis and radial labels).
    Below,
    /// Right-aligned left of the point: the point is the middle of the text's right edge (y-axis labels).
    Left,
    /// Above and left of the point: the point is the text's bottom-right corner ("Re").
    AboveLeft,
    /// Below and right of the point: the point is the text's top-left corner ("Im").
    BelowRight,
}

/// One tick label: the TeX the label typesets and where, in the box's pixel space.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Label {
    pub tex: String,
    pub at: [f64; 2],
    pub align: Align,
}

/// The attributes of an `axes` at one frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    pub kind: AxesKind,
    pub width: f64,
    pub height: f64,
    /// xMin, xMax, yMin, yMax.
    pub ranges: [f64; 4],
    /// xStep and yStep, when written.
    pub steps: [Option<f64>; 2],
    pub ticks: bool,
    pub labels: bool,
    pub grid: bool,
    pub tick_length: f64,
}

/// An `axes` at one frame: its mapping and the geometry of its parts, in the box's pixel space.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AxesFrame {
    pub kind: AxesKind,
    #[serde(skip)]
    pub axes: Axes,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub grid: Vec<Vec<[f64; 2]>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub axis: Vec<Vec<[f64; 2]>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ticks: Vec<Vec<[f64; 2]>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<Label>,
}

impl AxesFrame {
    /// Rule 9: a child's data position mapped by rule 1 into the box's pixel space.
    pub fn place(&self, x: f64, y: f64) -> [f64; 2] {
        plots::to_pixel(&self.axes, x, y)
    }

    /// Rule 9: a child's anchor, an offset in data units, mapped by the linear part of rule 1.
    pub fn offset(&self, dx: f64, dy: f64) -> [f64; 2] {
        let a = &self.axes;
        [dx / (a.x_max - a.x_min) * a.width, -dy / (a.y_max - a.y_min) * a.height]
    }
}

/// Segments per full circle of a polar grid.
const CIRCLE_SEGMENTS: usize = 256;

/// The parts of an `axes` with `spec`'s values (rules 3 to 6 and 9).
pub fn frame(spec: &Spec) -> Result<AxesFrame, PlotError> {
    let [x_min, x_max, y_min, y_max] = spec.ranges;
    // a number-line does not read yMin and yMax: its one axis runs through the middle of the box
    let (y_min, y_max) = if spec.kind == AxesKind::NumberLine { (-1.0, 1.0) } else { (y_min, y_max) };
    let axes = Axes::new(x_min, x_max, y_min, y_max, spec.width, spec.height)?;
    let mut out = AxesFrame { kind: spec.kind, axes, grid: vec![], axis: vec![], ticks: vec![], labels: vec![] };
    let (w, h) = (spec.width, spec.height);
    let half = spec.tick_length / 2.0;
    let step = |k: usize, lo: f64, hi: f64| match spec.steps[k] {
        Some(s) => Ok(s),
        None => plots::nice_step(lo, hi),
    };
    if spec.kind == AxesKind::Polar {
        let r = [x_min, x_max, y_min, y_max].iter().fold(0.0f64, |m, v| m.max(v.abs()));
        let s = step(0, 0.0, r)?;
        let o = plots::to_pixel(&axes, 0.0, 0.0);
        for rho in plots::ticks(0.0, r, s)?.into_iter().filter(|&v| v > 0.0) {
            let circle: Vec<[f64; 2]> = (0..=CIRCLE_SEGMENTS)
                .map(|i| {
                    let th = i as f64 * std::f64::consts::TAU / CIRCLE_SEGMENTS as f64;
                    plots::to_pixel(&axes, rho * th.cos(), rho * th.sin())
                })
                .collect();
            out.grid.extend(plots::clip_polyline(&circle, 0.0, 0.0, w, h));
            if spec.labels {
                let p = plots::to_pixel(&axes, rho, 0.0);
                out.labels.push(Label {
                    tex: plots::tick_label(rho, s, false)?,
                    at: [p[0], p[1] + LABEL_GAP],
                    align: Align::Below,
                });
            }
        }
        for k in 0..12 {
            let th = (k * 30) as f64 * std::f64::consts::PI / 180.0;
            let end = plots::to_pixel(&axes, r * th.cos(), r * th.sin());
            out.grid.extend(plots::clip_polyline(&[o, end], 0.0, 0.0, w, h));
        }
        return Ok(out);
    }
    let line = spec.kind == AxesKind::NumberLine;
    let ax_y = if line { h / 2.0 } else { plots::to_pixel(&axes, 0.0, plots::axis_position(y_min, y_max))[1] };
    let ax_x = plots::to_pixel(&axes, plots::axis_position(x_min, x_max), 0.0)[0];
    let (sx, xs) = {
        let s = step(0, x_min, x_max)?;
        (s, plots::ticks(x_min, x_max, s)?)
    };
    out.axis.push(vec![[0.0, ax_y], [w, ax_y]]);
    let ys = if line {
        None
    } else {
        out.axis.push(vec![[ax_x, h], [ax_x, 0.0]]);
        let s = step(1, y_min, y_max)?;
        Some((s, plots::ticks(y_min, y_max, s)?))
    };
    let xs_px: Vec<f64> = xs.iter().map(|&v| plots::to_pixel(&axes, v, 0.0)[0]).collect();
    let ys_px: Vec<f64> = ys.iter().flat_map(|(_, t)| t.iter().map(|&v| plots::to_pixel(&axes, 0.0, v)[1])).collect();
    if spec.grid && !line {
        out.grid.extend(xs_px.iter().map(|&px| vec![[px, h], [px, 0.0]]));
        out.grid.extend(ys_px.iter().map(|&py| vec![[0.0, py], [w, py]]));
    }
    if spec.ticks {
        out.ticks.extend(xs_px.iter().map(|&px| vec![[px, ax_y - half], [px, ax_y + half]]));
        out.ticks.extend(ys_px.iter().map(|&py| vec![[ax_x - half, py], [ax_x + half, py]]));
    }
    if spec.labels {
        for (&v, &px) in xs.iter().zip(&xs_px) {
            if v != 0.0 || line {
                let at = [px, ax_y + half + LABEL_GAP];
                out.labels.push(Label { tex: plots::tick_label(v, sx, false)?, at, align: Align::Below });
            }
        }
        if let Some((sy, ys)) = &ys {
            let imaginary = spec.kind == AxesKind::Complex;
            for (&v, &py) in ys.iter().zip(&ys_px) {
                if v != 0.0 {
                    let at = [ax_x - half - LABEL_GAP, py];
                    out.labels.push(Label { tex: plots::tick_label(v, *sy, imaginary)?, at, align: Align::Left });
                }
            }
        }
        if spec.kind == AxesKind::Complex {
            out.labels.push(Label {
                tex: "\\mathrm{Re}".into(),
                at: [w - LABEL_GAP, ax_y - LABEL_GAP],
                align: Align::AboveLeft,
            });
            out.labels.push(Label {
                tex: "\\mathrm{Im}".into(),
                at: [ax_x + LABEL_GAP, LABEL_GAP],
                align: Align::BelowRight,
            });
        }
    }
    Ok(out)
}

/// Rules 7 and 2: the subpaths a plot draws from its points in data units. The points are split at their
/// non-finite points, mapped by rule 1 and clipped to the box (when `clip`); a closed curve that nothing cut stays
/// one closed subpath, and one that was cut draws as the open runs inside the box.
pub fn plot_subpaths(a: &Axes, points: &[[f64; 2]], closed: bool, clip: bool) -> Vec<(Vec<[f64; 2]>, bool)> {
    let parts = plots::split_finite(points);
    let closed = closed && parts.len() == 1 && points.len() == parts[0].len();
    let mut out = Vec::new();
    for part in parts {
        let px: Vec<[f64; 2]> = part.iter().map(|p| plots::to_pixel(a, p[0], p[1])).collect();
        let inside = |p: &[f64; 2]| (0.0..=a.width).contains(&p[0]) && (0.0..=a.height).contains(&p[1]);
        if !clip || px.iter().all(inside) {
            out.push((px, closed));
            continue;
        }
        let mut ring = px;
        if closed {
            ring.push(ring[0]);
        }
        out.extend(plots::clip_polyline(&ring, 0.0, 0.0, a.width, a.height).into_iter().map(|s| (s, false)));
    }
    out
}

/// Rule 8's polylines as subpaths: a polyline that closes (its last point repeats its first) is one closed subpath
/// without the repeated point.
pub fn closed_runs(polylines: Vec<Vec<[f64; 2]>>) -> Vec<(Vec<[f64; 2]>, bool)> {
    polylines
        .into_iter()
        .map(|mut p| {
            let closed = p.len() > 2 && p.first() == p.last();
            if closed {
                p.pop();
            }
            (p, closed)
        })
        .collect()
}

/// Path data of subpaths: `M` at each first point, `L` at the others, `Z` after a closed one.
pub fn path_data(subpaths: &[(Vec<[f64; 2]>, bool)]) -> String {
    let mut d = String::new();
    for (points, closed) in subpaths {
        for (i, [x, y]) in points.iter().enumerate() {
            if !d.is_empty() {
                d.push(' ');
            }
            let _ = write!(d, "{} {x} {y}", if i == 0 { 'M' } else { 'L' });
        }
        if *closed && !points.is_empty() {
            d.push_str(" Z");
        }
    }
    d
}

/// Path data of open polylines.
pub fn lines(polylines: &[Vec<[f64; 2]>]) -> String {
    let runs: Vec<(Vec<[f64; 2]>, bool)> = polylines.iter().map(|p| (p.clone(), false)).collect();
    path_data(&runs)
}

/// The `plot` attributes that only the sampling reads; the others (stroke, fill, stroke style, trim, markers) are
/// the plot shape's own.
pub const PLOT_ONLY: &[&str] = &[
    "id",
    "kind",
    "y",
    "x",
    "r",
    "f",
    "x0",
    "x1",
    "t0",
    "t1",
    "theta0",
    "theta1",
    "level",
    "samples",
    "resolution",
    "closed",
];

/// The expressions of a plot of its kind, `(attribute, source)`, and their sampling variables (rules 7 and 8).
pub(crate) fn plot_sources(p: &m::Plot) -> (Vec<(&'static str, String)>, &'static [&'static str]) {
    let src = |a: &'static str, v: &Option<String>| (a, v.clone().unwrap_or_default());
    match p.kind {
        m::PlotKind::Explicit => (vec![src("y", &p.y)], &["x"]),
        m::PlotKind::Parametric => (vec![src("x", &p.x), src("y", &p.y)], &["t"]),
        m::PlotKind::Polar => (vec![src("r", &p.r)], &["theta"]),
        m::PlotKind::Implicit => (vec![src("f", &p.f)], &["x", "y"]),
    }
}

/// A plot with its compiled expressions (indices into `Program::exprs`, in [`plot_sources`] order).
#[derive(Debug, Clone)]
pub struct PlotInst {
    pub kind: m::PlotKind,
    pub exprs: Vec<u32>,
    /// x0 and x1 of an explicit plot, when written; else the frame's xMin and xMax.
    pub x: [Option<f64>; 2],
    pub t: [f64; 2],
    pub theta: [f64; 2],
    pub level: f64,
    pub samples: usize,
    pub resolution: usize,
    pub closed: bool,
}

impl PlotInst {
    pub(crate) fn new(p: &m::Plot, exprs: Vec<u32>) -> PlotInst {
        PlotInst {
            kind: p.kind,
            exprs,
            x: [p.x0, p.x1],
            t: [p.t0, p.t1],
            theta: [p.theta0, p.theta1],
            level: p.level,
            samples: usize::try_from(p.samples).unwrap_or(usize::MAX),
            resolution: usize::try_from(p.resolution).unwrap_or(usize::MAX),
            closed: p.closed,
        }
    }
}

/// The path data of plot `p`, drawn by node `n`, at this frame of its axes (rules 7, 8 and 2).
pub(crate) fn plot_path(
    f: &mut crate::eval::Frame,
    n: u32,
    p: &PlotInst,
    af: &AxesFrame,
    clip: bool,
) -> Result<String, PlotError> {
    let a = &af.axes;
    let mut v = Vec::new();
    let mut run = |f: &mut crate::eval::Frame, k: usize, at: &[[f64; 2]]| {
        f.sampled(n, p.exprs[k], at, &mut v);
        std::mem::take(&mut v)
    };
    let at = |xs: &[f64]| xs.iter().map(|&x| [x, 0.0]).collect::<Vec<_>>();
    let subpaths = match p.kind {
        m::PlotKind::Explicit => {
            let xs = plots::grid(p.x[0].unwrap_or(a.x_min), p.x[1].unwrap_or(a.x_max), p.samples, false)?;
            let ys = run(f, 0, &at(&xs));
            let pts: Vec<[f64; 2]> = xs.iter().zip(&ys).map(|(&x, &y)| [x, y]).collect();
            plot_subpaths(a, &pts, false, clip)
        }
        m::PlotKind::Parametric => {
            let ts = at(&plots::grid(p.t[0], p.t[1], p.samples, p.closed)?);
            let (xs, ys) = (run(f, 0, &ts), run(f, 1, &ts));
            let pts: Vec<[f64; 2]> = xs.iter().zip(&ys).map(|(&x, &y)| [x, y]).collect();
            plot_subpaths(a, &pts, p.closed, clip)
        }
        m::PlotKind::Polar => {
            let ths = plots::grid(p.theta[0], p.theta[1], p.samples, false)?;
            let rs = run(f, 0, &at(&ths));
            let pts: Vec<[f64; 2]> = ths.iter().zip(&rs).map(|(&th, &r)| [r * th.cos(), r * th.sin()]).collect();
            plot_subpaths(a, &pts, p.closed, clip)
        }
        m::PlotKind::Implicit => {
            let (xs, ys) = plots::marching_grid([a.x_min, a.x_max, a.y_min, a.y_max], p.resolution)?;
            let corners: Vec<[f64; 2]> = xs.iter().flat_map(|&x| ys.iter().map(move |&y| [x, y])).collect();
            let vals = run(f, 0, &corners);
            let v: Vec<Vec<f64>> = vals.chunks(ys.len()).map(|c| c.iter().map(|z| z - p.level).collect()).collect();
            closed_runs(plots::marching_squares_on(&xs, &ys, &v))
                .into_iter()
                .flat_map(|(pts, closed)| plot_subpaths(a, &pts, closed, clip))
                .collect()
        }
    };
    Ok(path_data(&subpaths))
}

/// An `object3D primitive="axes"` (rules 10 and 11): its `axes3D` and its `plot3D` children with their compiled
/// expressions.
#[derive(Debug, Clone)]
pub struct Axes3DInst {
    pub axes: m::Axes3D,
    pub plots: Vec<Plot3DInst>,
}

/// A `plot3D` with its x, y and z expressions (indices into `Program::exprs`).
#[derive(Debug, Clone)]
pub struct Plot3DInst {
    pub exprs: [u32; 3],
    pub t: [f64; 2],
    pub samples: usize,
    pub closed: bool,
    pub radius: Option<f64>,
    pub segments: usize,
    /// Linear RGBA, which multiplies the material colour.
    pub color: [f32; 4],
    /// The id of the material the tube is drawn with, instead of the object's.
    pub material: Option<std::sync::Arc<str>>,
}

impl Plot3DInst {
    pub(crate) fn new(p: &m::Plot3D, exprs: [u32; 3]) -> Plot3DInst {
        Plot3DInst {
            exprs,
            t: [p.t0, p.t1],
            samples: usize::try_from(p.samples).unwrap_or(usize::MAX),
            closed: p.closed,
            radius: p.tube_radius.map(|r| r.get()),
            segments: usize::try_from(p.tube_segments).unwrap_or(usize::MAX),
            color: linear(&p.color),
            material: p.material.as_deref().map(Into::into),
        }
    }
}

/// The linear RGBA of a literal colour. A token or paint reference has no colour the evaluator can read: it is
/// white, so the object's material colour shows unchanged.
pub(crate) fn linear(p: &sr_model::values::Paint) -> [f32; 4] {
    use sr_model::values::{Color, Paint};
    let lin = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    match p {
        Paint::Color(Color::Rgba(c)) => [lin(c.r), lin(c.g), lin(c.b), c.a],
        _ => [1.0; 4],
    }
}

/// Segments around an axis line or tick.
const LINE_SEGMENTS: usize = 8;

/// The grid mesh of rule 11 from the points of a curve: the rings of [`plots::tube`] around its finite points, a
/// non-finite point's ring left non-finite so the grid drops it with its two cells.
pub(crate) fn tube_mesh(
    points: &[[f64; 3]],
    radius: f64,
    segments: usize,
    closed: bool,
) -> Result<sr_3d::Primitive, PlotError> {
    let finite: Vec<[f64; 3]> = points.iter().copied().filter(|p| p.iter().all(|c| c.is_finite())).collect();
    let mut rings = plots::tube(&finite, radius, segments)?.into_iter();
    let gap = vec![[f64::NAN; 3]; segments];
    let rings: Vec<Vec<[f64; 3]>> = points
        .iter()
        .map(
            |p| if p.iter().all(|c| c.is_finite()) { rings.next().unwrap_or_else(|| gap.clone()) } else { gap.clone() },
        )
        .collect();
    let nu = rings.len();
    let mut grid = Vec::with_capacity(nu * segments);
    let mut uvs = Vec::with_capacity(nu * segments);
    for j in 0..segments {
        for (i, ring) in rings.iter().enumerate() {
            grid.push(ring[j]);
            uvs.push([i as f32 / (nu - 1).max(1) as f32, j as f32 / segments as f32]);
        }
    }
    Ok(sr_3d::prim::parametric_grid(&grid, &uvs, nu, segments, [closed, true]))
}

/// The meshes of an axes object at this frame, in the object's local space.
#[derive(Debug)]
pub struct Axes3DMeshes {
    /// What the object's material draws: the axis lines, the ticks and the tubes of the plots without a `material`.
    pub own: crate::parametric::ParamMesh,
    /// The tubes of the plots with a `material`, one mesh per material id, in the order the plots first name it.
    pub parts: Vec<(std::sync::Arc<str>, crate::parametric::ParamMesh)>,
}

/// A primitive and the linear colour of its vertices.
type Tinted = (sr_3d::Primitive, [f32; 4]);

/// Merges primitives into one, each vertex coloured with its primitive's colour, and keys the result by its content.
fn merge(parts: Vec<Tinted>) -> crate::parametric::ParamMesh {
    let mut mesh = sr_3d::Primitive::default();
    let mut words: Vec<u64> = Vec::new();
    for (part, c) in parts {
        let base = mesh.vertices.len() as u32;
        mesh.indices.extend(part.indices.iter().map(|k| k + base));
        for mut v in part.vertices {
            v.color = c;
            words.extend(v.pos.iter().chain(&v.color).map(|x| x.to_bits() as u64));
            mesh.vertices.push(v);
        }
        words.push(u64::MAX);
    }
    crate::parametric::ParamMesh { key: crate::rng::hash(&words), mesh }
}

/// The meshes of an axes object at this frame (rules 10 and 11): its three axis lines and their ticks as cylinders
/// of `lineWidth` in the `axes3D`'s `color`, and each plot's tube in its `color`, as vertex colours. A plot whose
/// curve makes no tube (fewer than two distinct finite points) is left out, with a problem.
pub(crate) fn axes3d_mesh(
    f: &mut crate::eval::Frame,
    n: u32,
    inst: &Axes3DInst,
    problems: &mut Vec<String>,
    who: &str,
) -> Result<Axes3DMeshes, PlotError> {
    let a = &inst.axes;
    let size = [a.width.get(), a.height.get(), a.depth.get()];
    let ranges = [[a.x_min, a.x_max], [a.y_min, a.y_max], [a.z_min, a.z_max]];
    let ax = plots::Axes3D::new(ranges[0], ranges[1], ranges[2], size)?;
    let least = size[0].min(size[1]).min(size[2]);
    let radius = a.line_width.map(|w| w.get()).unwrap_or(0.01 * least) / 2.0;
    let tick = a.tick_length.map(|t| t.get()).unwrap_or(0.02 * least);
    let steps = [a.x_step, a.y_step, a.z_step];
    let color = linear(&a.color);
    let at = |d: [f64; 3]| plots::to_scene(&ax, d[0], d[1], d[2]);
    let origin = ranges.map(|r| plots::axis_position(r[0], r[1]));
    let mut parts: Vec<Tinted> = Vec::new();
    let mut by_material: Vec<(std::sync::Arc<str>, Vec<Tinted>)> = Vec::new();
    for k in 0..3 {
        let (mut lo, mut hi) = (origin, origin);
        (lo[k], hi[k]) = (ranges[k][0], ranges[k][1]);
        parts.push((tube_mesh(&[at(lo), at(hi)], radius, LINE_SEGMENTS, false)?, color));
        if !a.ticks {
            continue;
        }
        let step = match steps[k] {
            Some(s) => s.get(),
            None => plots::nice_step(ranges[k][0], ranges[k][1])?,
        };
        // a tick crosses its axis upright, a y tick along x
        let dir = if k == 1 { [tick / 2.0, 0.0, 0.0] } else { [0.0, tick / 2.0, 0.0] };
        for v in plots::ticks(ranges[k][0], ranges[k][1], step)? {
            let mut d = origin;
            d[k] = v;
            let c = at(d);
            let ends = [[c[0] - dir[0], c[1] - dir[1], c[2] - dir[2]], [c[0] + dir[0], c[1] + dir[1], c[2] + dir[2]]];
            parts.push((tube_mesh(&ends, radius, LINE_SEGMENTS, false)?, color));
        }
    }
    for (i, p) in inst.plots.iter().enumerate() {
        let ts: Vec<[f64; 2]> =
            plots::grid(p.t[0], p.t[1], p.samples, p.closed)?.into_iter().map(|t| [t, 0.0]).collect();
        let mut xyz = [Vec::new(), Vec::new(), Vec::new()];
        for (k, v) in xyz.iter_mut().enumerate() {
            f.sampled(n, p.exprs[k], &ts, v);
        }
        let pts: Vec<[f64; 3]> = (0..ts.len()).map(|s| at([xyz[0][s], xyz[1][s], xyz[2][s]])).collect();
        let r = p.radius.unwrap_or(0.015 * least);
        let mesh = match tube_mesh(&pts, r, p.segments, p.closed) {
            Ok(mesh) => mesh,
            Err(e) => {
                problems.push(format!("{who}/plot3D[{i}]: {e}"));
                continue;
            }
        };
        match &p.material {
            None => parts.push((mesh, p.color)),
            Some(id) => match by_material.iter_mut().find(|(m, _)| m == id) {
                Some((_, group)) => group.push((mesh, p.color)),
                None => by_material.push((id.clone(), vec![(mesh, p.color)])),
            },
        }
    }
    Ok(Axes3DMeshes { own: merge(parts), parts: by_material.into_iter().map(|(id, g)| (id, merge(g))).collect() })
}
