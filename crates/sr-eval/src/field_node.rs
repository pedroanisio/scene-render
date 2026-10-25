//! The fields draft (sr-core srep/srep-0000-fields.md) at the evaluator: a `field` node's source and presentations
//! as the program holds them, and the parts the frame computes from them by the rules of [`crate::fields`].
//!
//! The program lowers a `field` to one synthetic child per presentation: a path shape for `arrows`, `streamlines`,
//! `lines` and `contours`, whose `path` the frame sets, and a raster node for `lic` and `portrait`, whose
//! [`FieldRaster`] the frame computes. Paths and rasters are in the field box's pixels, its top-left at the node's
//! origin; the rules run in data units, +y up, and the pixel attributes convert through the box's pixels per unit
//! (the mean of the two axes').

use std::sync::Arc;

use sr_vector::space::{complex_function, Complex, ComplexFn};

use crate::fields;
use crate::plots;

/// What a field is (rule 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Vector,
    Scalar,
    Complex,
}

/// Where a field's values come from (rule 2).
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// `u` and `v` (vector) or `u` alone (scalar): expressions of x and y, indices into `Program::exprs`.
    Exprs { u: u32, v: Option<u32> },
    /// `w` as its real and imaginary expressions.
    W([u32; 2]),
    /// A named complex function with its coefficients (mobius).
    Function { f: ComplexFn, coefficients: Vec<f64> },
    /// A data source's rows on a regular lattice: rows of (u, v) from yMin, over its lattice box [x0, y0, x1, y1].
    Grid { lattice: Lattice, ranges: [f64; 4] },
    /// A `forceField` of physics, by its element key.
    ForceField(Arc<str>),
    /// The sum of the charge [x, y, q], current [x, y, i] and dipole [x, y, mx, my] children.
    Sources { charges: Vec<[f64; 3]>, currents: Vec<[f64; 3]>, dipoles: Vec<[f64; 4]> },
}

/// A presentation child, with its attributes; lengths are in pixels as written.
#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    Arrows {
        cols: usize,
        rows: usize,
        style: fields::ArrowStyle,
    },
    Streamlines {
        separation: f64,
        test: Option<f64>,
        step: Option<f64>,
        max_steps: u64,
        max_lines: usize,
        seed: [Option<f64>; 2],
    },
    Lines {
        per_charge: u32,
        radius: f64,
        step: Option<f64>,
        max_steps: u64,
    },
    Contours {
        levels: Vec<f64>,
        resolution: usize,
    },
    Lic {
        length: f64,
        step: f64,
        resolution: Option<usize>,
        seed: u64,
        opacity: f64,
    },
    Portrait {
        modulus: bool,
        base: f64,
        opacity: f64,
    },
}

/// A `field` node: its kind, source and presentations; `axes` is the enclosing `axes` node, whose box and ranges
/// the field takes.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldInst {
    pub kind: Kind,
    pub source: Source,
    pub parts: Vec<Part>,
    pub axes: Option<u32>,
    /// The `field` node, whose attributes and timeline the frame reads.
    pub node: u32,
}

/// A computed raster of a `lic` or `portrait`: `cols` x `rows` display-encoded RGBA pixels, rows from the box's top,
/// drawn over the box at `opacity`.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct FieldRaster {
    pub cols: usize,
    pub rows: usize,
    pub rgba: Vec<[f32; 4]>,
    pub opacity: f64,
}

/// The default separation of `streamlines`, in pixels, from which `lines` also takes its default step.
pub const DEFAULT_SEPARATION: f64 = 20.0;
/// The largest raster side, as the schema bounds a lic's resolution.
pub const MAX_RASTER: usize = 4096;

/// The field box at a frame: its size in pixels and its data ranges [x0, y0, x1, y1].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FieldBox {
    pub width: f64,
    pub height: f64,
    pub ranges: [f64; 4],
}

impl FieldBox {
    /// The box, when its ranges are ordered and its size positive.
    pub fn new(width: f64, height: f64, ranges: [f64; 4]) -> Result<FieldBox, String> {
        let [x0, y0, x1, y1] = ranges;
        if !(x0 < x1 && y0 < y1 && [x0, y0, x1, y1].iter().all(|v| v.is_finite())) {
            return Err(format!("the field's ranges {ranges:?} are not ordered"));
        }
        if !(width > 0.0 && height > 0.0 && width.is_finite() && height.is_finite()) {
            return Err(format!("the field's box {width} x {height} is empty"));
        }
        Ok(FieldBox { width, height, ranges })
    }

    /// Pixels per data unit: the mean of the two axes'.
    pub fn ppu(&self) -> f64 {
        let [x0, y0, x1, y1] = self.ranges;
        0.5 * (self.width / (x1 - x0) + self.height / (y1 - y0))
    }

    /// Data (x, y) to the box's pixel, +y up.
    pub fn px(&self, p: [f64; 2]) -> [f64; 2] {
        let [x0, y0, x1, y1] = self.ranges;
        [(p[0] - x0) / (x1 - x0) * self.width, (y1 - p[1]) / (y1 - y0) * self.height]
    }
}

/// A grid source's field values, row by row from the smallest y, each row from the smallest x.
pub type Lattice = Vec<Vec<[f64; 2]>>;

/// A grid source's lattice from a data source's rows (records of numbers x, y, u, v, or x, y, s for a scalar
/// field, s held as (s, 0)): the rows of nodes from the smallest y, each from the smallest x, and the lattice box
/// [x0, y0, x1, y1]. Every node of the lattice of the distinct x and y values appears exactly once, with at least
/// two of each.
pub fn lattice(rows: &crate::expr::vm::V, scalar: bool) -> Result<(Lattice, [f64; 4]), String> {
    use crate::expr::vm::V;
    let V::Arr(rows) = rows else { return Err("the data source is not a list of rows".into()) };
    let names: &[&str] = if scalar { &["x", "y", "s"] } else { &["x", "y", "u", "v"] };
    let mut nodes = Vec::with_capacity(rows.len());
    for (i, r) in rows.iter().enumerate() {
        let V::Obj(o) = r else { return Err(format!("row {} is not a record", i + 1)) };
        let get = |k: &str| match o.get(k) {
            Some(V::Num(v)) if v.is_finite() => Ok(*v),
            _ => Err(format!("row {} has no number {k}", i + 1)),
        };
        let vals = names.iter().map(|k| get(k)).collect::<Result<Vec<f64>, String>>()?;
        nodes.push(vals);
    }
    let distinct = |k: usize| {
        let mut v: Vec<f64> = nodes.iter().map(|n| n[k]).collect();
        v.sort_by(f64::total_cmp);
        v.dedup();
        v
    };
    let (xs, ys) = (distinct(0), distinct(1));
    if xs.len() < 2 || ys.len() < 2 || xs.len() * ys.len() != nodes.len() {
        return Err(format!("{} rows are not a regular lattice of at least 2 x 2 nodes", nodes.len()));
    }
    let mut grid = vec![vec![None; xs.len()]; ys.len()];
    for n in &nodes {
        let i = xs.binary_search_by(|x| x.total_cmp(&n[0])).map_err(|_| "a node is off the lattice")?;
        let j = ys.binary_search_by(|y| y.total_cmp(&n[1])).map_err(|_| "a node is off the lattice")?;
        if grid[j][i].replace(if scalar { [n[2], 0.0] } else { [n[2], n[3]] }).is_some() {
            return Err(format!("the node ({}, {}) appears twice", n[0], n[1]));
        }
    }
    let spacing = |v: &[f64]| {
        let d = (v[v.len() - 1] - v[0]) / (v.len() - 1) as f64;
        v.iter().enumerate().all(|(k, x)| (x - (v[0] + k as f64 * d)).abs() <= 1e-9 * d.abs().max(1.0))
    };
    if !(spacing(&xs) && spacing(&ys)) {
        return Err("the lattice is not evenly spaced".into());
    }
    let lattice = grid.into_iter().map(|row| row.into_iter().flatten().collect()).collect();
    Ok((lattice, [xs[0], ys[0], xs[xs.len() - 1], ys[ys.len() - 1]]))
}

/// The value of a complex function at x + iy, as (Re w, Im w).
pub fn complex_at(f: ComplexFn, coefficients: &[f64], x: f64, y: f64) -> [f64; 2] {
    let w = complex_function(f, Complex { re: x, im: y }, coefficients);
    [w.re, w.im]
}

/// The gradient of a scalar field by central differences of step `fields::DIFF_H` (rule 3's step), which `lic`
/// draws for a scalar field (rule 1).
pub fn gradient(s: impl Fn(f64, f64) -> f64) -> impl Fn(f64, f64) -> [f64; 2] {
    move |x, y| {
        let h = fields::DIFF_H;
        [(s(x + h, y) - s(x - h, y)) / (2.0 * h), (s(x, y + h) - s(x, y - h)) / (2.0 * h)]
    }
}

fn path(b: &FieldBox, polylines: &[Vec<[f64; 2]>], closed: impl Fn(usize) -> bool) -> String {
    let subpaths: Vec<(Vec<[f64; 2]>, bool)> = polylines
        .iter()
        .enumerate()
        .filter(|(_, l)| !l.is_empty())
        .map(|(i, l)| (l.iter().map(|p| b.px(*p)).collect(), closed(i)))
        .collect();
    crate::axes::path_data(&subpaths)
}

/// Rule 5: the glyphs of the grid as path data, each a shaft and an open head (left, tip, right).
pub fn arrows_path(
    f: impl Fn(f64, f64) -> [f64; 2],
    b: &FieldBox,
    cols: usize,
    rows: usize,
    style: &fields::ArrowStyle,
) -> Result<String, fields::FieldError> {
    let style = fields::ArrowStyle { max_length: style.max_length / b.ppu(), ..*style };
    let glyphs = fields::arrow_grid(f, b.ranges, cols, rows, &style)?;
    let lines: Vec<Vec<[f64; 2]>> =
        glyphs.iter().flatten().flat_map(|a| [a.shaft.to_vec(), vec![a.head[1], a.head[0], a.head[2]]]).collect();
    Ok(path(b, &lines, |_| false))
}

/// Rule 6: evenly spaced streamlines as path data; `separation`, `test` and `step` in pixels, the seed in data units
/// (default the box's centre).
#[allow(clippy::too_many_arguments)]
pub fn streamlines_path(
    f: impl Fn(f64, f64) -> [f64; 2],
    b: &FieldBox,
    separation: f64,
    test: Option<f64>,
    step: Option<f64>,
    max_steps: u64,
    max_lines: usize,
    seed: [Option<f64>; 2],
) -> Result<String, fields::FieldError> {
    let ppu = b.ppu();
    let [x0, y0, x1, y1] = b.ranges;
    let s = fields::Spacing {
        d_sep: separation / ppu,
        d_test: test.map(|t| t / ppu),
        h: step.map(|h| h / ppu),
        max_steps,
        max_lines,
        seed: Some([seed[0].unwrap_or(0.5 * (x0 + x1)), seed[1].unwrap_or(0.5 * (y0 + y1))]),
    };
    Ok(path(b, &fields::evenly_spaced_streamlines(f, b.ranges, &s)?, |_| false))
}

/// Rule 10: the field lines of the charges as path data; `step` in pixels (default a quarter of the default
/// separation), `radius` in data units.
pub fn lines_path(
    charges: &[[f64; 3]],
    b: &FieldBox,
    per_charge: u32,
    radius: f64,
    step: Option<f64>,
    max_steps: u64,
) -> Result<String, fields::FieldError> {
    let h = step.unwrap_or(DEFAULT_SEPARATION / 4.0) / b.ppu();
    let c = fields::ChargeLines { per_charge, r0: radius, h, max_steps, ranges: b.ranges };
    Ok(path(b, &fields::charge_lines(charges, &c)?, |_| false))
}

/// The level curves of a scalar field at each level by marching squares on a `resolution` grid (the plots
/// draft's rule 8), as path data; `sample` evaluates the field at a batch of points.
pub fn contours_path(
    sample: impl FnOnce(&[[f64; 2]]) -> Vec<f64>,
    b: &FieldBox,
    levels: &[f64],
    resolution: usize,
) -> Result<String, plots::PlotError> {
    let [x0, y0, x1, y1] = b.ranges;
    let (xs, ys) = plots::marching_grid([x0, x1, y0, y1], resolution)?;
    let corners: Vec<[f64; 2]> = xs.iter().flat_map(|&x| ys.iter().map(move |&y| [x, y])).collect();
    let vals = sample(&corners);
    let mut subpaths = Vec::new();
    for level in levels {
        let v: Vec<Vec<f64>> = vals.chunks(ys.len()).map(|c| c.iter().map(|z| z - level).collect()).collect();
        for (pts, closed) in crate::axes::closed_runs(plots::marching_squares_on(&xs, &ys, &v)) {
            subpaths.push((pts.iter().map(|p| b.px(*p)).collect::<Vec<_>>(), closed));
        }
    }
    Ok(crate::axes::path_data(&subpaths))
}

/// The raster size of a lic: `resolution` columns (default the box's width in pixels), rows in the box's
/// proportion, each side at least 1 and at most [`MAX_RASTER`].
pub fn lic_size(b: &FieldBox, resolution: Option<usize>) -> [usize; 2] {
    let side = |v: f64| (v.round() as usize).clamp(1, MAX_RASTER);
    let cols = resolution.unwrap_or_else(|| side(b.width)).clamp(1, MAX_RASTER);
    [cols, side(cols as f64 * b.height / b.width)]
}

/// Rule 7: the lic raster over the D24 noise U(seed, row, column) (rows from yMin), as grey pixels from the box's
/// top; `length` and `step` in pixels.
pub fn lic_raster(
    f: impl Fn(f64, f64) -> [f64; 2],
    b: &FieldBox,
    length: f64,
    step: f64,
    resolution: Option<usize>,
    seed: u64,
    opacity: f64,
) -> Result<FieldRaster, fields::FieldError> {
    let [cols, rows] = lic_size(b, resolution);
    let noise: Vec<Vec<f64>> =
        (0..rows).map(|j| (0..cols).map(|i| crate::rng::d24_unit(seed, j as u64, i as u64)).collect()).collect();
    let ppu = b.ppu();
    let out = fields::lic(f, &noise, b.ranges, length / ppu, step / ppu)?;
    let rgba = out.iter().rev().flat_map(|row| row.iter().map(|&v| [v as f32, v as f32, v as f32, 1.0])).collect();
    Ok(FieldRaster { cols, rows, rgba, opacity })
}

/// HSL with full saturation to display-encoded RGB; `h` in [0, 1).
pub fn hsl(h: f64, l: f64) -> [f64; 3] {
    let c = 1.0 - (2.0 * l - 1.0).abs();
    let hp = h * 6.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [r + m, g + m, b + m]
}

/// Rule 8: the phase portrait at each pixel centre of the box (its pixel size, at most [`MAX_RASTER`] a side):
/// hue arg w / 2π, full saturation, lightness 0.5 less 0.25 × frac(log_base |w|) when `modulus` is set.
pub fn portrait_raster(
    w: impl Fn(f64, f64) -> [f64; 2],
    b: &FieldBox,
    modulus: bool,
    base: f64,
    opacity: f64,
) -> FieldRaster {
    let side = |v: f64| (v.round() as usize).clamp(1, MAX_RASTER);
    let (cols, rows) = (side(b.width), side(b.height));
    let [x0, y0, x1, y1] = b.ranges;
    let mut rgba = Vec::with_capacity(cols * rows);
    for j in 0..rows {
        let y = y1 - (j as f64 + 0.5) / rows as f64 * (y1 - y0);
        for i in 0..cols {
            let x = x0 + (i as f64 + 0.5) / cols as f64 * (x1 - x0);
            let v = w(x, y);
            let l = if modulus { 0.5 - 0.25 * fields::modulus_shade(v, base) } else { 0.5 };
            let [r, g, bl] = hsl(fields::phase_hue(v), l);
            rgba.push([r as f32, g as f32, bl as f32, 1.0]);
        }
    }
    FieldRaster { cols, rows, rgba, opacity }
}
