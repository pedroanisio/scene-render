// Adapted from FilmCraft (crates/scopes/src/lib.rs, crates/scopes/src/summary.rs, crates/color/src/lib.rs),
// Copyright (c) 2026 ArtCraft Team and the FilmCraft contributors, used under the MIT License; see
// THIRD-PARTY-NOTICES.md.

//! Image scopes as automated QA: the luma waveform, RGB parade, histogram and Cb/Cr vectorscope of
//! video frames and stills, with the level checks a delivery review makes on them.
//!
//! The input is a [`Signal`]: a frame's R'G'B' values (display-encoded, 0..1 nominal; values
//! outside are the super-whites, sub-blacks and out-of-gamut colours of float and Y'CbCr sources)
//! and, for limited-range Y'CbCr sources, the code values themselves. Y'CbCr comes from the
//! stream's matrix ([`Matrix`]: BT.601, BT.709 or BT.2020 non-constant luminance). Every sample is
//! measured; nothing is decimated.
//!
//! | Measure | Counts a sample when |
//! |---|---|
//! | Black / white clipping | R', G', B' or Y' is at or beyond 0 / 1, within half a code value |
//! | Illegal (limited range) | a Y' code is outside 16–235, or a Cb/Cr code outside 16–240 (scaled by 2^(bits − 8): 64–940 and 64–960 at 10 bits) |
//! | Illegal (float) | R', G' or B' is below 0, above 1, or not a number |
//! | Out of gamut | R', G' or B' is outside −5 %..105 % (the EBU R 103 tolerance), or not a number |
//! | Above the chroma limit | √(Cb² + Cr²) exceeds the limit: by default the largest 75 % colour-bar chroma of the matrix |
//!
//! [`Scopes`] accumulates frames: the levels of the whole set, the worst frame of each measure,
//! and the scope grids, summarised numerically by [`Scopes::summary`]. [`analyse`] runs it over a
//! file: every `step`-th frame of a video, or the single frame of a still (an animated image
//! contributes its first frame; alpha is ignored).

use std::path::{Path, PathBuf};

use image::DynamicImage;
use serde::Serialize;

use crate::decode::{PixelLayout, VideoDecoder, VideoFrame};
use crate::probe::VideoInfo;
use crate::{still, MediaError};

#[cfg(test)]
mod tests;

/// Schema identifier of a [`Report`].
pub const SCHEMA: &str = "scene-render-scopes/1";
/// Waveform and parade columns: frames of any width fold onto this many.
pub const WAVEFORM_COLUMNS: usize = 480;
/// Half-width of the vectorscope plane in Cb/Cr units (the 100 % targets of every matrix fit).
pub const VECTOR_EXTENT: f32 = 0.6;
/// Histogram bins per channel.
pub const BINS: usize = 256;
/// Lowest R'G'B' value inside the gamut (EBU R 103: −5 %).
pub const GAMUT_LOW: f32 = -0.05;
/// Highest R'G'B' value inside the gamut (EBU R 103: 105 %).
pub const GAMUT_HIGH: f32 = 1.05;

/// Column ranges a waveform trace is summarised over.
const SUMMARY_BUCKETS: usize = 8;
/// Bins of the summarised histogram.
const HISTOGRAM_SUMMARY_BINS: usize = 32;
/// Densest vectorscope cells reported.
const PEAKS: usize = 8;
/// Smallest share of the samples a reported vectorscope cell holds.
const PEAK_SHARE: f32 = 0.01;
/// Decoder look-ahead for sequential analysis.
const LOOKAHEAD: usize = 4;

/// Errors from scope analysis.
#[derive(Debug, thiserror::Error)]
pub enum ScopeError {
    /// Probing or decoding the video failed.
    #[error(transparent)]
    Media(#[from] MediaError),
    /// The file cannot be read.
    #[error("cannot read {}: {source}", .path.display())]
    Open {
        /// The file.
        path: PathBuf,
        /// Cause.
        source: std::io::Error,
    },
    /// The still image cannot be decoded.
    #[error("{0}")]
    Still(String),
    /// A frame's samples do not match its declared size or layout.
    #[error("frame does not match its layout: {0}")]
    Frame(String),
    /// Invalid settings.
    #[error("{0}")]
    Invalid(String),
}

// ------------------------------------------------------------------------------------ colour

/// Y'CbCr matrix coefficients.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize)]
pub enum Matrix {
    /// ITU-R BT.601 (SMPTE 170M, BT.470 BG).
    #[serde(rename = "BT.601")]
    Bt601,
    /// ITU-R BT.709.
    #[default]
    #[serde(rename = "BT.709")]
    Bt709,
    /// ITU-R BT.2020 non-constant luminance.
    #[serde(rename = "BT.2020")]
    Bt2020,
}

impl Matrix {
    /// (Kr, Kb) luma coefficients.
    pub fn kr_kb(self) -> (f32, f32) {
        match self {
            Matrix::Bt601 => (0.299, 0.114),
            Matrix::Bt709 => (0.2126, 0.0722),
            Matrix::Bt2020 => (0.2627, 0.0593),
        }
    }

    /// The matrix an FFmpeg `color_space` tag names, and whether the tag named one; untagged
    /// streams follow the usual convention (BT.709 from 720 lines up, BT.601 below).
    pub fn from_tag(tag: &str, height: u32) -> (Matrix, bool) {
        match tag {
            "bt2020nc" | "bt2020c" | "bt2020_ncl" => (Matrix::Bt2020, true),
            "smpte170m" | "bt470bg" | "bt601" => (Matrix::Bt601, true),
            "bt709" => (Matrix::Bt709, true),
            _ if height >= 720 => (Matrix::Bt709, false),
            _ => (Matrix::Bt601, false),
        }
    }

    /// Display name (`BT.709`, …).
    pub fn name(self) -> &'static str {
        match self {
            Matrix::Bt601 => "BT.601",
            Matrix::Bt709 => "BT.709",
            Matrix::Bt2020 => "BT.2020",
        }
    }
}

/// Non-linear R'G'B' to normalised Y'CbCr (Y' 0..1, Cb and Cr −0.5..0.5).
#[inline]
pub fn rgb_to_ycbcr(r: f32, g: f32, b: f32, m: Matrix) -> [f32; 3] {
    let (kr, kb) = m.kr_kb();
    let kg = 1.0 - kr - kb;
    let y = kr * r + kg * g + kb * b;
    [y, (b - y) / (2.0 * (1.0 - kb)), (r - y) / (2.0 * (1.0 - kr))]
}

/// Normalised Y'CbCr to non-linear R'G'B'.
#[inline]
pub fn ycbcr_to_rgb(y: f32, cb: f32, cr: f32, m: Matrix) -> [f32; 3] {
    let (kr, kb) = m.kr_kb();
    let kg = 1.0 - kr - kb;
    let r = y + 2.0 * (1.0 - kr) * cr;
    let b = y + 2.0 * (1.0 - kb) * cb;
    let g = (y - kr * r - kb * b) / kg;
    [r, g, b]
}

/// Angle (degrees, counter-clockwise from +Cb, 0..360) of a chroma point.
pub fn angle_deg(cb: f32, cr: f32) -> f32 {
    cr.atan2(cb).to_degrees().rem_euclid(360.0)
}

/// The angle of pure red on a matrix's vectorscope (BT.709 ≈ 103°).
pub fn red_angle(m: Matrix) -> f32 {
    let [_, cb, cr] = rgb_to_ycbcr(1.0, 0.0, 0.0, m);
    angle_deg(cb, cr)
}

/// A colour-bar target of the vectorscope.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Target {
    /// `R`, `Mg`, `B`, `Cy`, `G` or `Yl`.
    pub name: &'static str,
    /// The bar's R'G'B'.
    pub rgb: [f32; 3],
    /// Its Cb.
    pub cb: f32,
    /// Its Cr.
    pub cr: f32,
}

/// The six colour-bar targets (R, Mg, B, Cy, G, Yl) at `amplitude` (0.75 or 1.0).
pub fn targets(m: Matrix, amplitude: f32) -> [Target; 6] {
    let a = amplitude;
    let t = |name, rgb: [f32; 3]| {
        let [_, cb, cr] = rgb_to_ycbcr(rgb[0], rgb[1], rgb[2], m);
        Target { name, rgb, cb, cr }
    };
    [
        t("R", [a, 0.0, 0.0]),
        t("Mg", [a, 0.0, a]),
        t("B", [0.0, 0.0, a]),
        t("Cy", [0.0, a, a]),
        t("G", [0.0, a, 0.0]),
        t("Yl", [a, a, 0.0]),
    ]
}

/// The default chroma limit: the largest chroma of the matrix's 75 % colour bars (magenta and
/// green; 0.4468 in BT.709), the most saturated colour broadcast test signals carry.
pub fn default_chroma_limit(m: Matrix) -> f32 {
    targets(m, 0.75).iter().map(|t| t.cb.hypot(t.cr)).fold(0.0, f32::max)
}

fn check_limit(limit: Option<f32>) -> Result<(), ScopeError> {
    match limit {
        None => Ok(()),
        Some(l) if l.is_finite() && l > 0.0 => Ok(()),
        Some(l) => Err(ScopeError::Invalid(format!("the chroma limit must be a positive number, not {l}"))),
    }
}

// ------------------------------------------------------------------------------------ settings

/// How scopes are computed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// Y'CbCr matrix.
    pub matrix: Matrix,
    /// Clamp Signal: values are clamped to 0..1 before they are plotted.
    pub clamp: bool,
    /// Waveform and parade levels.
    pub rows: usize,
    /// Vectorscope grid size (square; odd, so the centre is a cell).
    pub vector_size: usize,
    /// Waveform and parade columns.
    pub columns: usize,
}

impl Default for Params {
    fn default() -> Self {
        Params { matrix: Matrix::Bt709, clamp: true, rows: 256, vector_size: 255, columns: WAVEFORM_COLUMNS }
    }
}

impl Params {
    /// The plotted value range of waveform rows.
    pub fn range(&self) -> (f32, f32) {
        if self.clamp {
            (0.0, 1.0)
        } else {
            (-0.1, 1.1)
        }
    }
}

// ------------------------------------------------------------------------------------ signal

/// The code values of a limited-range Y'CbCr source.
#[derive(Clone, Debug, PartialEq)]
pub struct Codes {
    /// Bits per component.
    pub depth: u32,
    /// Y', Cb, Cr codes per sample, row-major.
    pub ycc: Vec<[u16; 3]>,
}

/// A frame's R'G'B' samples, row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct Signal {
    /// Width.
    pub w: usize,
    /// Height.
    pub h: usize,
    /// R'G'B' per sample.
    pub rgb: Vec<[f32; 3]>,
    /// Code values of a limited-range Y'CbCr source.
    pub codes: Option<Codes>,
    /// Half a quantisation step of the source: values this close to 0 or 1 are clipped.
    pub tolerance: f32,
    /// Floating-point samples, whose range is open.
    pub float: bool,
}

fn expect_len(w: usize, h: usize, per: usize, len: usize) -> Result<(), ScopeError> {
    let want = w.checked_mul(h).and_then(|n| n.checked_mul(per));
    if want != Some(len) {
        return Err(ScopeError::Frame(format!("{w}×{h} needs {} values, got {len}", want.unwrap_or(usize::MAX))));
    }
    Ok(())
}

impl Signal {
    /// From straight RGBA, 8 bits.
    pub fn from_rgba8(w: usize, h: usize, px: &[u8]) -> Result<Signal, ScopeError> {
        expect_len(w, h, 4, px.len())?;
        let lut: [f32; 256] = std::array::from_fn(|i| i as f32 / 255.0);
        let rgb = px
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&[r, g, b, _]| [lut[r as usize], lut[g as usize], lut[b as usize]])
            .collect();
        Ok(Signal { w, h, rgb, codes: None, tolerance: 0.5 / 255.0, float: false })
    }

    /// From straight RGBA, 16 bits.
    pub fn from_rgba16(w: usize, h: usize, px: &[u16]) -> Result<Signal, ScopeError> {
        expect_len(w, h, 4, px.len())?;
        let f = |v: u16| v as f32 / 65535.0;
        let rgb = px.as_chunks::<4>().0.iter().map(|&[r, g, b, _]| [f(r), f(g), f(b)]).collect();
        Ok(Signal { w, h, rgb, codes: None, tolerance: 0.5 / 65535.0, float: false })
    }

    /// From straight RGBA, 32-bit float.
    pub fn from_rgba_f32(w: usize, h: usize, px: &[f32]) -> Result<Signal, ScopeError> {
        expect_len(w, h, 4, px.len())?;
        let rgb = px.as_chunks::<4>().0.iter().map(|&[r, g, b, _]| [r, g, b]).collect();
        Ok(Signal { w, h, rgb, codes: None, tolerance: 0.0, float: true })
    }

    /// A synthetic floating-point signal.
    pub fn from_fn(w: usize, h: usize, f: impl Fn(usize, usize) -> [f32; 3]) -> Signal {
        let rgb = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| f(x, y)).collect();
        Signal { w, h, rgb, codes: None, tolerance: 0.0, float: true }
    }

    /// From Y'CbCr code values at `depth` bits (8 to 16), full or limited range.
    pub fn from_ycbcr(
        w: usize,
        h: usize,
        depth: u32,
        full_range: bool,
        matrix: Matrix,
        f: impl Fn(usize, usize) -> [u16; 3],
    ) -> Result<Signal, ScopeError> {
        if !(8..=16).contains(&depth) {
            return Err(ScopeError::Invalid(format!("Y'CbCr depth must be 8 to 16 bits, not {depth}")));
        }
        let ycc: Vec<[u16; 3]> = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| f(x, y)).collect();
        Ok(Self::from_codes(w, h, depth, full_range, matrix, ycc))
    }

    fn from_codes(w: usize, h: usize, depth: u32, full: bool, matrix: Matrix, ycc: Vec<[u16; 3]>) -> Signal {
        let max = ((1u32 << depth) - 1) as f32;
        let s = (1u32 << (depth - 8)) as f32;
        let (y0, ys, c0, cs) = if full {
            (0.0, max, (1u32 << (depth - 1)) as f32, max)
        } else {
            (16.0 * s, 219.0 * s, 128.0 * s, 224.0 * s)
        };
        let rgb = ycc
            .iter()
            .map(|&[y, cb, cr]| {
                ycbcr_to_rgb((y as f32 - y0) / ys, (cb as f32 - c0) / cs, (cr as f32 - c0) / cs, matrix)
            })
            .collect();
        let codes = (!full).then_some(Codes { depth, ycc });
        Signal { w, h, rgb, codes, tolerance: 0.5 / ys, float: false }
    }

    /// From a decoded video frame of a stream with `info`, converted with `matrix`.
    pub fn from_frame(f: &VideoFrame, info: &VideoInfo, matrix: Matrix) -> Result<Signal, ScopeError> {
        let expect = f.layout.planes(f.width, f.height);
        if f.planes.len() != expect.len() {
            return Err(ScopeError::Frame(format!(
                "{:?} has {} planes, got {}",
                f.layout,
                expect.len(),
                f.planes.len()
            )));
        }
        for (i, (p, &(w, h, b))) in f.planes.iter().zip(&expect).enumerate() {
            let len = (w as usize).checked_mul(h as usize).and_then(|n| n.checked_mul(b as usize));
            if p.width != w || p.height != h || Some(p.data.len()) != len {
                return Err(ScopeError::Frame(format!(
                    "plane {i} of a {}×{} {:?} frame is {}×{} with {} bytes",
                    f.width,
                    f.height,
                    f.layout,
                    p.width,
                    p.height,
                    p.data.len()
                )));
            }
        }
        let (w, h) = (f.width as usize, f.height as usize);
        let le = |d: &[u8], i: usize| u16::from_le_bytes([d[2 * i], d[2 * i + 1]]);
        match f.layout {
            PixelLayout::Rgba8 => Signal::from_rgba8(w, h, &f.planes[0].data),
            PixelLayout::Rgba16 => {
                let d = &f.planes[0].data;
                Signal::from_rgba16(w, h, &(0..d.len() / 2).map(|i| le(d, i)).collect::<Vec<_>>())
            }
            PixelLayout::Yuv8(sx, sy) | PixelLayout::Yuv16(sx, sy) => {
                let wide = matches!(f.layout, PixelLayout::Yuv16(..));
                let depth = if wide { info.depth.clamp(9, 16) } else { 8 };
                let shift = 16 - depth;
                let cw = expect[1].0 as usize;
                let at = |plane: usize, i: usize| {
                    let d = &f.planes[plane].data;
                    if wide {
                        le(d, i) >> shift
                    } else {
                        d[i] as u16
                    }
                };
                let ycc = (0..h)
                    .flat_map(|y| (0..w).map(move |x| (x, y)))
                    .map(|(x, y)| {
                        let c = (y >> sy) * cw + (x >> sx);
                        [at(0, y * w + x), at(1, c), at(2, c)]
                    })
                    .collect();
                Ok(Signal::from_codes(w, h, depth, info.full_range, matrix, ycc))
            }
        }
    }

    /// From a decoded still image (alpha is ignored).
    pub fn from_image(img: &DynamicImage) -> Result<Signal, ScopeError> {
        let (w, h) = (img.width() as usize, img.height() as usize);
        match img {
            DynamicImage::ImageRgb32F(_) | DynamicImage::ImageRgba32F(_) => {
                Signal::from_rgba_f32(w, h, img.to_rgba32f().as_raw())
            }
            DynamicImage::ImageLuma16(_)
            | DynamicImage::ImageLumaA16(_)
            | DynamicImage::ImageRgb16(_)
            | DynamicImage::ImageRgba16(_) => Signal::from_rgba16(w, h, img.to_rgba16().as_raw()),
            _ => Signal::from_rgba8(w, h, img.to_rgba8().as_raw()),
        }
    }
}

// ------------------------------------------------------------------------------------ grids

/// A count grid: `data[row * cols + col]`, row 0 = the lowest level.
#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    /// Trace name (`Y`, `R`, `G`, `B`).
    pub name: &'static str,
    /// Columns.
    pub cols: usize,
    /// Rows.
    pub rows: usize,
    /// Counts.
    pub data: Vec<u64>,
}

impl Grid {
    fn new(name: &'static str, cols: usize, rows: usize) -> Grid {
        Grid { name, cols, rows, data: vec![0; cols * rows] }
    }
    /// Count of a cell.
    pub fn at(&self, col: usize, row: usize) -> u64 {
        self.data[row * self.cols + col]
    }
    /// All counts.
    pub fn total(&self) -> u64 {
        self.data.iter().sum()
    }
    /// The largest count.
    pub fn max(&self) -> u64 {
        self.data.iter().copied().max().unwrap_or(0)
    }
    /// The rows hit in a column, lowest first.
    pub fn rows_in(&self, col: usize) -> Vec<usize> {
        (0..self.rows).filter(|&r| self.at(col, r) > 0).collect()
    }
}

/// Waveform or parade: one grid per trace.
#[derive(Clone, Debug, PartialEq)]
pub struct Waveform {
    /// Level of the lowest row.
    pub lo: f32,
    /// Level of the highest row.
    pub hi: f32,
    /// One grid per trace.
    pub traces: Vec<Grid>,
}

/// Row of value `v` in `[lo, hi]` over `rows` levels (None outside, or NaN).
#[inline]
pub fn row_of(v: f32, lo: f32, hi: f32, rows: usize) -> Option<usize> {
    let t = (v - lo) / (hi - lo);
    if !(0.0..=1.0).contains(&t) {
        return None;
    }
    Some(((t * (rows - 1) as f32).round() as usize).min(rows - 1))
}

impl Waveform {
    fn new(names: &[&'static str], p: &Params) -> Waveform {
        let (lo, hi) = p.range();
        let (cols, rows) = (p.columns.max(1), p.rows.max(2));
        Waveform { lo, hi, traces: names.iter().map(|&n| Grid::new(n, cols, rows)).collect() }
    }

    /// A trace by name.
    pub fn trace(&self, name: &str) -> Option<&Grid> {
        self.traces.iter().find(|t| t.name == name)
    }

    /// The value at the centre of a row.
    pub fn level(&self, row: usize, rows: usize) -> f32 {
        self.lo + (self.hi - self.lo) * row as f32 / (rows.max(2) - 1) as f32
    }

    /// Plots every sample: trace `t` gets `values(rgb)[t]`.
    fn add(&mut self, s: &Signal, p: &Params, values: impl Fn([f32; 3]) -> [f32; 3]) {
        let (lo, hi) = (self.lo, self.hi);
        for y in 0..s.h {
            for x in 0..s.w {
                let mut c = s.rgb[y * s.w + x];
                if p.clamp {
                    c = c.map(|v| v.clamp(0.0, 1.0));
                }
                let v = values(c);
                for (g, &v) in self.traces.iter_mut().zip(&v) {
                    let v = if p.clamp { v.clamp(0.0, 1.0) } else { v };
                    if let Some(r) = row_of(v, lo, hi, g.rows) {
                        let col = x * g.cols / s.w;
                        g.data[r * g.cols + col] += 1;
                    }
                }
            }
        }
    }
}

fn add_luma(w: &mut Waveform, s: &Signal, p: &Params) {
    let m = p.matrix;
    w.add(s, p, |[r, g, b]| [rgb_to_ycbcr(r, g, b, m)[0], 0.0, 0.0]);
}

/// The luma waveform of a signal (trace `Y`).
pub fn luma_waveform(s: &Signal, p: &Params) -> Waveform {
    let mut w = Waveform::new(&["Y"], p);
    add_luma(&mut w, s, p);
    w
}

/// The RGB parade of a signal (traces `R`, `G`, `B`, drawn side by side).
pub fn rgb_parade(s: &Signal, p: &Params) -> Waveform {
    let mut w = Waveform::new(&["R", "G", "B"], p);
    w.add(s, p, |c| c);
    w
}

// ------------------------------------------------------------------------------------ histogram

/// 256-bin histograms of R', G', B' and Y'.
#[derive(Clone, Debug, PartialEq)]
pub struct Histogram {
    /// R' bins.
    pub r: Vec<u64>,
    /// G' bins.
    pub g: Vec<u64>,
    /// B' bins.
    pub b: Vec<u64>,
    /// Y' bins.
    pub y: Vec<u64>,
    /// Samples.
    pub samples: u64,
    /// Samples below 0 per channel (R, G, B, Y); with Clamp Signal also counted in the first bin.
    pub below: [u64; 4],
    /// Samples above 1 per channel (R, G, B, Y); with Clamp Signal also counted in the last bin.
    pub above: [u64; 4],
}

/// Bin of a value (None when outside 0..1).
#[inline]
pub fn bin_of(v: f32) -> Option<usize> {
    if (0.0..=1.0 + 1e-6).contains(&v) {
        Some(((v * 255.0).round() as usize).min(BINS - 1))
    } else {
        None
    }
}

impl Histogram {
    fn new() -> Histogram {
        Histogram {
            r: vec![0; BINS],
            g: vec![0; BINS],
            b: vec![0; BINS],
            y: vec![0; BINS],
            samples: 0,
            below: [0; 4],
            above: [0; 4],
        }
    }

    fn add(&mut self, s: &Signal, p: &Params) {
        self.samples += s.rgb.len() as u64;
        for &[r, g, b] in &s.rgb {
            let y = rgb_to_ycbcr(r, g, b, p.matrix)[0];
            for (i, v) in [r, g, b, y].into_iter().enumerate() {
                let bins = match i {
                    0 => &mut self.r,
                    1 => &mut self.g,
                    2 => &mut self.b,
                    _ => &mut self.y,
                };
                match bin_of(v) {
                    Some(k) => bins[k] += 1,
                    None if v.is_nan() => {}
                    None => {
                        let low = v < 0.0;
                        if low {
                            self.below[i] += 1;
                        } else {
                            self.above[i] += 1;
                        }
                        if p.clamp {
                            bins[if low { 0 } else { BINS - 1 }] += 1;
                        }
                    }
                }
            }
        }
    }
}

/// The histograms of a signal.
pub fn histogram(s: &Signal, p: &Params) -> Histogram {
    let mut h = Histogram::new();
    h.add(s, p);
    h
}

// ------------------------------------------------------------------------------------ vectorscope

/// A vectorscope: `data[y * size + x]`, x = Cb (right = +), y = Cr (row 0 = top = +Cr).
#[derive(Clone, Debug, PartialEq)]
pub struct Vectorscope {
    /// Grid size.
    pub size: usize,
    /// Half-width of the plane in Cb/Cr units.
    pub extent: f32,
    /// Counts.
    pub data: Vec<u64>,
    /// Plotted samples.
    pub samples: u64,
    sum: [f64; 2],
}

/// Cell of a (Cb, Cr) point on a `size`² plane spanning ±`extent`.
#[inline]
pub fn cell_of(cb: f32, cr: f32, size: usize, extent: f32) -> Option<(usize, usize)> {
    let n = (size - 1) as f32;
    let fx = (cb / extent * 0.5 + 0.5) * n;
    let fy = (0.5 - cr / extent * 0.5) * n;
    if !(0.0..=n).contains(&fx) || !(0.0..=n).contains(&fy) {
        return None;
    }
    Some((fx.round() as usize, fy.round() as usize))
}

impl Vectorscope {
    fn new(size: usize) -> Self {
        Vectorscope { size, extent: VECTOR_EXTENT, data: vec![0; size * size], samples: 0, sum: [0.0; 2] }
    }
    /// Count of a cell.
    pub fn at(&self, x: usize, y: usize) -> u64 {
        self.data[y * self.size + x]
    }
    /// The cell of a (Cb, Cr) point.
    pub fn cell(&self, cb: f32, cr: f32) -> Option<(usize, usize)> {
        cell_of(cb, cr, self.size, self.extent)
    }
    /// (Cb, Cr) at the centre of a cell.
    pub fn point(&self, x: usize, y: usize) -> [f32; 2] {
        let n = (self.size - 1) as f32;
        [(x as f32 / n - 0.5) * 2.0 * self.extent, (0.5 - y as f32 / n) * 2.0 * self.extent]
    }
    /// Mean (Cb, Cr) of the plotted samples.
    pub fn mean(&self) -> [f32; 2] {
        if self.samples == 0 {
            return [0.0; 2];
        }
        let n = self.samples as f64;
        [(self.sum[0] / n) as f32, (self.sum[1] / n) as f32]
    }

    fn add(&mut self, s: &Signal, p: &Params) {
        for &c in &s.rgb {
            let [r, g, b] = if p.clamp { c.map(|x| x.clamp(0.0, 1.0)) } else { c };
            let [_, cb, cr] = rgb_to_ycbcr(r, g, b, p.matrix);
            if let Some((x, y)) = self.cell(cb, cr) {
                self.data[y * self.size + x] += 1;
                self.samples += 1;
                self.sum[0] += cb as f64;
                self.sum[1] += cr as f64;
            }
        }
    }
}

/// The Cb/Cr vectorscope of a signal.
pub fn vectorscope(s: &Signal, p: &Params) -> Vectorscope {
    let mut v = Vectorscope::new(p.vector_size.max(2));
    v.add(s, p);
    v
}

// ------------------------------------------------------------------------------------ summaries

/// Minimum, maximum and mean of a set of levels (percent of full scale).
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Level {
    /// Minimum.
    pub min: f64,
    /// Maximum.
    pub max: f64,
    /// Mean.
    pub mean: f64,
}

#[derive(Clone, Copy, Debug, Default)]
struct Acc {
    min: f32,
    max: f32,
    sum: f64,
    n: u64,
}

impl Acc {
    fn add(&mut self, v: f32, count: u64) {
        if self.n == 0 {
            self.min = v;
            self.max = v;
        } else {
            self.min = self.min.min(v);
            self.max = self.max.max(v);
        }
        self.sum += v as f64 * count as f64;
        self.n += count;
    }
    fn level(&self) -> Option<Level> {
        (self.n > 0).then(|| Level {
            min: round2(self.min),
            max: round2(self.max),
            mean: round2(self.sum / self.n as f64),
        })
    }
}

/// Rounded to 0.01 (as f64, so JSON shows `78.43`, not the f32's `78.43000030517578`).
fn round2(v: impl Into<f64>) -> f64 {
    (v.into() * 100.0).round() / 100.0
}

fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

/// Per-channel levels of the samples themselves (R', G', B', Y'), in percent.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ChannelStats {
    /// R'.
    pub r: Level,
    /// G'.
    pub g: Level,
    /// B'.
    pub b: Level,
    /// Y'.
    pub y: Level,
    /// Mean chroma (Cb, Cr) in −50..50.
    pub chroma_mean: [f64; 2],
    /// Angle of the mean chroma, counter-clockwise from +Cb.
    pub hue_deg: f64,
    /// Magnitude of the mean chroma in percent of 0.5.
    pub saturation: f64,
}

/// A trace split into `buckets` column ranges, each with the levels its cells hold (None when a
/// range holds no samples).
pub fn trace_columns(w: &Waveform, g: &Grid, buckets: usize) -> Vec<Option<Level>> {
    let buckets = buckets.clamp(1, g.cols.max(1));
    (0..buckets)
        .map(|b| {
            let c0 = b * g.cols / buckets;
            let c1 = ((b + 1) * g.cols / buckets).max(c0 + 1).min(g.cols);
            let mut a = Acc::default();
            for col in c0..c1 {
                for row in 0..g.rows {
                    let n = g.at(col, row);
                    if n > 0 {
                        a.add(w.level(row, g.rows) * 100.0, n);
                    }
                }
            }
            a.level()
        })
        .collect()
}

/// A dense spot of a vectorscope.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Peak {
    /// Cell column.
    pub x: usize,
    /// Cell row.
    pub y: usize,
    /// Cb at the cell centre (percent: −50..50 is the nominal range).
    pub cb: f64,
    /// Cr at the cell centre (percent).
    pub cr: f64,
    /// Angle, counter-clockwise from +Cb.
    pub angle_deg: f64,
    /// Distance from the centre in percent of 0.5.
    pub magnitude: f64,
    /// Share of the plotted samples in this cell (0..1).
    pub share: f64,
}

/// The `n` densest cells holding at least `min_share` of the samples, densest first.
pub fn peaks(v: &Vectorscope, n: usize, min_share: f32) -> Vec<Peak> {
    let total = v.samples.max(1) as f64;
    let mut cells: Vec<(u64, usize)> =
        v.data.iter().enumerate().filter(|(_, c)| **c > 0).map(|(i, c)| (*c, i)).collect();
    cells.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    cells
        .into_iter()
        .filter(|(c, _)| *c as f64 / total >= min_share as f64)
        .take(n)
        .map(|(c, i)| {
            let (x, y) = (i % v.size, i / v.size);
            let [cb, cr] = v.point(x, y);
            Peak {
                x,
                y,
                cb: round2(cb * 100.0),
                cr: round2(cr * 100.0),
                angle_deg: round2(angle_deg(cb, cr)),
                magnitude: round2(cb.hypot(cr) / 0.5 * 100.0),
                share: round4(c as f64 / total),
            }
        })
        .collect()
}

/// R', G', B' and Y' values of one measure.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Channels {
    /// R'.
    pub r: f64,
    /// G'.
    pub g: f64,
    /// B'.
    pub b: f64,
    /// Y'.
    pub y: f64,
}

impl Channels {
    fn of(counts: [u64; 4], total: u64) -> Channels {
        let [r, g, b, y] = counts.map(|c| frac(c, total));
        Channels { r, g, b, y }
    }
    fn max(&self) -> f64 {
        self.r.max(self.g).max(self.b).max(self.y)
    }
}

/// The histograms re-binned to [`HISTOGRAM_SUMMARY_BINS`] bins of sample fractions.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HistogramSummary {
    /// Bins per channel.
    pub bins: usize,
    /// Samples.
    pub samples: u64,
    /// R' fractions per bin.
    pub r: Vec<f64>,
    /// G' fractions per bin.
    pub g: Vec<f64>,
    /// B' fractions per bin.
    pub b: Vec<f64>,
    /// Y' fractions per bin.
    pub y: Vec<f64>,
    /// Fractions below 0.
    pub below: Channels,
    /// Fractions above 1.
    pub above: Channels,
}

/// The parade's traces summarised over column ranges.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ParadeSummary {
    /// R'.
    pub r: Vec<Option<Level>>,
    /// G'.
    pub g: Vec<Option<Level>>,
    /// B'.
    pub b: Vec<Option<Level>>,
}

/// The waveform and parade summarised over column ranges, left to right.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WaveformSummary {
    /// Columns of the grids.
    pub columns: usize,
    /// Rows of the grids.
    pub rows: usize,
    /// Column ranges.
    pub buckets: usize,
    /// Y' levels per column range.
    pub luma: Vec<Option<Level>>,
    /// R', G', B' levels per column range.
    pub parade: ParadeSummary,
}

/// A 75 % colour-bar target in percent.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TargetSummary {
    /// `R`, `Mg`, `B`, `Cy`, `G` or `Yl`.
    pub name: &'static str,
    /// Cb (percent).
    pub cb: f64,
    /// Cr (percent).
    pub cr: f64,
    /// Distance from the centre in percent of 0.5.
    pub magnitude: f64,
}

/// The vectorscope's densest cells and the targets to compare them with.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct VectorscopeSummary {
    /// Grid size.
    pub size: usize,
    /// Half-width of the plane in Cb/Cr units.
    pub extent: f32,
    /// Plotted samples.
    pub samples: u64,
    /// Mean (Cb, Cr) in percent.
    pub mean: [f64; 2],
    /// Densest cells.
    pub peaks: Vec<Peak>,
    /// 75 % colour-bar targets.
    pub targets: Vec<TargetSummary>,
}

/// Numeric summary of every scope.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Summary {
    /// Per-channel levels (None when no sample was finite).
    pub channels: Option<ChannelStats>,
    /// Histograms.
    pub histogram: HistogramSummary,
    /// Waveform and parade.
    pub waveform: WaveformSummary,
    /// Vectorscope.
    pub vectorscope: VectorscopeSummary,
}

// ------------------------------------------------------------------------------------ levels

fn frac(n: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        n as f64 / total as f64
    }
}

/// Fractions of R', G', B'.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Rgb {
    /// R'.
    pub r: f64,
    /// G'.
    pub g: f64,
    /// B'.
    pub b: f64,
}

/// Fractions of Y', Cb, Cr codes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Ycc {
    /// Y'.
    pub y: f64,
    /// Cb.
    pub cb: f64,
    /// Cr.
    pub cr: f64,
}

/// Fractions of samples clipped at black (0 and below) and white (1 and above).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Clipping {
    /// At or below 0.
    pub black: Channels,
    /// At or above 1.
    pub white: Channels,
}

/// Float samples outside 0..1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct FloatRange {
    /// Fraction of samples below 0, per channel.
    pub below: Rgb,
    /// Fraction of samples above 1, per channel.
    pub above: Rgb,
    /// Fraction of samples with a channel that is not a number or infinite.
    pub non_finite: f64,
}

/// Limited-range codes outside the legal range.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Legal {
    /// Bits per component.
    pub depth: u32,
    /// Legal Y' codes (inclusive).
    pub y: [u32; 2],
    /// Legal Cb/Cr codes (inclusive).
    pub c: [u32; 2],
    /// Fraction of samples below the legal range, per component.
    pub below: Ycc,
    /// Fraction of samples above the legal range, per component.
    pub above: Ycc,
}

/// Chroma and gamut excursions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Saturation {
    /// The chroma limit √(Cb² + Cr²) applied.
    pub chroma_limit: f64,
    /// Fraction of samples whose chroma exceeds the limit.
    pub above_limit: f64,
    /// Fraction of samples with R', G' or B' outside −5 %..105 %, or not a number.
    pub out_of_gamut: f64,
    /// The largest finite chroma.
    pub max_chroma: Option<f64>,
}

/// Level checks of a frame or a set of frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Levels {
    /// Samples measured.
    pub samples: u64,
    /// Clipping at black and white.
    pub clipping: Clipping,
    /// Float samples outside 0..1 (float sources only).
    pub float_range: Option<FloatRange>,
    /// Codes outside the legal range (limited-range Y'CbCr sources only).
    pub legal: Option<Legal>,
    /// Fraction of samples with any illegal value: a code outside the legal range, or a float value
    /// outside 0..1. None when the source has no legal range (8- and 16-bit RGB, full-range Y'CbCr).
    pub illegal: Option<f64>,
    /// Chroma and gamut.
    pub saturation: Saturation,
}

fn legal_range(depth: u32) -> ([u32; 2], [u32; 2]) {
    let s = depth.saturating_sub(8);
    ([16 << s, 235 << s], [16 << s, 240 << s])
}

#[derive(Clone, Copy, Debug, Default)]
struct Counts {
    samples: u64,
    black: [u64; 4],
    white: [u64; 4],
    float: bool,
    float_below: [u64; 3],
    float_above: [u64; 3],
    non_finite: u64,
    float_any: u64,
    legal_depth: Option<u32>,
    legal_samples: u64,
    legal_below: [u64; 3],
    legal_above: [u64; 3],
    legal_any: u64,
    above_chroma: u64,
    out_of_gamut: u64,
    max_chroma: Option<f32>,
}

fn add3(a: &mut [u64; 3], b: [u64; 3]) {
    a.iter_mut().zip(b).for_each(|(a, b)| *a += b);
}

fn add4(a: &mut [u64; 4], b: [u64; 4]) {
    a.iter_mut().zip(b).for_each(|(a, b)| *a += b);
}

impl Counts {
    fn merge(&mut self, o: &Counts) {
        self.samples += o.samples;
        add4(&mut self.black, o.black);
        add4(&mut self.white, o.white);
        self.float |= o.float;
        add3(&mut self.float_below, o.float_below);
        add3(&mut self.float_above, o.float_above);
        self.non_finite += o.non_finite;
        self.float_any += o.float_any;
        self.legal_depth = self.legal_depth.max(o.legal_depth);
        self.legal_samples += o.legal_samples;
        add3(&mut self.legal_below, o.legal_below);
        add3(&mut self.legal_above, o.legal_above);
        self.legal_any += o.legal_any;
        self.above_chroma += o.above_chroma;
        self.out_of_gamut += o.out_of_gamut;
        self.max_chroma = match (self.max_chroma, o.max_chroma) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
    }

    fn levels(&self, limit: f32) -> Levels {
        let n = self.samples;
        let rgb = |c: [u64; 3]| Rgb { r: frac(c[0], n), g: frac(c[1], n), b: frac(c[2], n) };
        let float_range = self.float.then(|| FloatRange {
            below: rgb(self.float_below),
            above: rgb(self.float_above),
            non_finite: frac(self.non_finite, n),
        });
        let legal = self.legal_depth.map(|depth| {
            let (y, c) = legal_range(depth);
            let ln = self.legal_samples;
            let ycc = |v: [u64; 3]| Ycc { y: frac(v[0], ln), cb: frac(v[1], ln), cr: frac(v[2], ln) };
            Legal { depth, y, c, below: ycc(self.legal_below), above: ycc(self.legal_above) }
        });
        let illegal = if legal.is_some() {
            Some(frac(self.legal_any, self.legal_samples))
        } else if self.float {
            Some(frac(self.float_any, n))
        } else {
            None
        };
        Levels {
            samples: n,
            clipping: Clipping { black: Channels::of(self.black, n), white: Channels::of(self.white, n) },
            float_range,
            legal,
            illegal,
            saturation: Saturation {
                chroma_limit: limit as f64,
                above_limit: frac(self.above_chroma, n),
                out_of_gamut: frac(self.out_of_gamut, n),
                max_chroma: self.max_chroma.map(f64::from),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Stats {
    channels: [Acc; 4],
    chroma: [f64; 2],
    chroma_n: u64,
}

impl Stats {
    fn summary(&self) -> Option<ChannelStats> {
        let [r, g, b, y] = self.channels.map(|a| a.level());
        let n = self.chroma_n.max(1) as f64;
        let (cb, cr) = ((self.chroma[0] / n) as f32, (self.chroma[1] / n) as f32);
        Some(ChannelStats {
            r: r?,
            g: g?,
            b: b?,
            y: y?,
            chroma_mean: [round2(cb * 100.0), round2(cr * 100.0)],
            hue_deg: round2(angle_deg(cb, cr)),
            saturation: round2(cb.hypot(cr) / 0.5 * 100.0),
        })
    }
}

/// Measures every sample of `s`.
fn measure(s: &Signal, m: Matrix, limit: f32, stats: &mut Stats) -> Counts {
    let tol = s.tolerance.max(1e-6);
    let mut k = Counts { samples: s.rgb.len() as u64, float: s.float, ..Counts::default() };
    let legal = s.codes.as_ref().map(|c| {
        let (y, ch) = legal_range(c.depth);
        k.legal_depth = Some(c.depth);
        k.legal_samples = c.ycc.len() as u64;
        (c, [y, ch, ch])
    });
    for (i, &[r, g, b]) in s.rgb.iter().enumerate() {
        let [y, cb, cr] = rgb_to_ycbcr(r, g, b, m);
        for (j, v) in [r, g, b, y].into_iter().enumerate() {
            if v <= tol {
                k.black[j] += 1;
            }
            if v >= 1.0 - tol {
                k.white[j] += 1;
            }
            if v.is_finite() {
                stats.channels[j].add(v * 100.0, 1);
            }
        }
        if s.float {
            let mut any = false;
            for (j, v) in [r, g, b].into_iter().enumerate() {
                if v < 0.0 {
                    k.float_below[j] += 1;
                    any = true;
                }
                if v > 1.0 {
                    k.float_above[j] += 1;
                    any = true;
                }
            }
            if ![r, g, b].iter().all(|v| v.is_finite()) {
                k.non_finite += 1;
                any = true;
            }
            k.float_any += any as u64;
        }
        if let Some((codes, ranges)) = legal {
            if let Some(c) = codes.ycc.get(i) {
                let mut any = false;
                for (j, (&v, [lo, hi])) in c.iter().zip(ranges).enumerate() {
                    let v = v as u32;
                    if v < lo {
                        k.legal_below[j] += 1;
                        any = true;
                    }
                    if v > hi {
                        k.legal_above[j] += 1;
                        any = true;
                    }
                }
                k.legal_any += any as u64;
            }
        }
        if ![r, g, b].iter().all(|v| (GAMUT_LOW..=GAMUT_HIGH).contains(v)) {
            k.out_of_gamut += 1;
        }
        let chroma = (cb * cb + cr * cr).sqrt();
        if chroma.is_finite() {
            if chroma > limit {
                k.above_chroma += 1;
            }
            k.max_chroma = Some(k.max_chroma.map_or(chroma, |m| m.max(chroma)));
            stats.chroma[0] += cb as f64;
            stats.chroma[1] += cr as f64;
            stats.chroma_n += 1;
        }
    }
    k
}

/// The frame where a measure peaked.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Worst {
    /// Frame index (0 for a still).
    pub frame: i64,
    /// The measure's fraction in that frame.
    pub value: f64,
}

/// The worst frame of each measure (the first frame when several tie).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct WorstFrames {
    /// Largest black-clipped fraction of any channel.
    pub black_clipped: Option<Worst>,
    /// Largest white-clipped fraction of any channel.
    pub white_clipped: Option<Worst>,
    /// Largest illegal fraction (sources with a legal range only).
    pub illegal: Option<Worst>,
    /// Largest out-of-gamut fraction.
    pub out_of_gamut: Option<Worst>,
    /// Largest fraction above the chroma limit.
    pub above_chroma_limit: Option<Worst>,
}

fn worse(slot: &mut Option<Worst>, frame: i64, value: Option<f64>) {
    if let Some(value) = value {
        if slot.is_none_or(|w| value > w.value) {
            *slot = Some(Worst { frame, value });
        }
    }
}

// ------------------------------------------------------------------------------------ accumulation

/// Scopes and levels accumulated over frames.
#[derive(Clone, Debug)]
pub struct Scopes {
    params: Params,
    limit: f32,
    frames: u64,
    counts: Counts,
    stats: Stats,
    worst: WorstFrames,
    luma: Waveform,
    parade: Waveform,
    histogram: Histogram,
    vectorscope: Vectorscope,
}

impl Scopes {
    /// Empty scopes; `chroma_limit` replaces [`default_chroma_limit`] of the matrix.
    pub fn new(params: Params, chroma_limit: Option<f32>) -> Result<Scopes, ScopeError> {
        check_limit(chroma_limit)?;
        Ok(Scopes {
            params,
            limit: chroma_limit.unwrap_or_else(|| default_chroma_limit(params.matrix)),
            frames: 0,
            counts: Counts::default(),
            stats: Stats::default(),
            worst: WorstFrames::default(),
            luma: Waveform::new(&["Y"], &params),
            parade: Waveform::new(&["R", "G", "B"], &params),
            histogram: Histogram::new(),
            vectorscope: Vectorscope::new(params.vector_size.max(2)),
        })
    }

    /// Adds frame `frame`, returning its own levels.
    pub fn add(&mut self, frame: i64, s: &Signal) -> Levels {
        let p = self.params;
        let counts = measure(s, p.matrix, self.limit, &mut self.stats);
        self.counts.merge(&counts);
        self.frames += 1;
        add_luma(&mut self.luma, s, &p);
        self.parade.add(s, &p, |c| c);
        self.histogram.add(s, &p);
        self.vectorscope.add(s, &p);
        let l = counts.levels(self.limit);
        let w = &mut self.worst;
        worse(&mut w.black_clipped, frame, Some(l.clipping.black.max()));
        worse(&mut w.white_clipped, frame, Some(l.clipping.white.max()));
        worse(&mut w.illegal, frame, l.illegal);
        worse(&mut w.out_of_gamut, frame, Some(l.saturation.out_of_gamut));
        worse(&mut w.above_chroma_limit, frame, Some(l.saturation.above_limit));
        l
    }

    /// Levels of every frame added.
    pub fn levels(&self) -> Levels {
        self.counts.levels(self.limit)
    }

    /// The worst frame of each measure.
    pub fn worst(&self) -> WorstFrames {
        self.worst
    }

    /// Frames added.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// The chroma limit applied.
    pub fn chroma_limit(&self) -> f32 {
        self.limit
    }

    /// The luma waveform.
    pub fn luma(&self) -> &Waveform {
        &self.luma
    }

    /// The RGB parade.
    pub fn parade(&self) -> &Waveform {
        &self.parade
    }

    /// The histograms.
    pub fn histogram(&self) -> &Histogram {
        &self.histogram
    }

    /// The vectorscope.
    pub fn vectorscope(&self) -> &Vectorscope {
        &self.vectorscope
    }

    /// Numeric summary of every scope.
    pub fn summary(&self) -> Summary {
        let h = &self.histogram;
        let rebin = |bins: &[u64]| -> Vec<f64> {
            let per = BINS / HISTOGRAM_SUMMARY_BINS;
            bins.chunks(per).map(|c| round4(frac(c.iter().sum(), h.samples))).collect()
        };
        let cols =
            |w: &Waveform, name: &str| w.trace(name).map(|g| trace_columns(w, g, SUMMARY_BUCKETS)).unwrap_or_default();
        let v = &self.vectorscope;
        let [mcb, mcr] = v.mean();
        Summary {
            channels: self.stats.summary(),
            histogram: HistogramSummary {
                bins: HISTOGRAM_SUMMARY_BINS,
                samples: h.samples,
                r: rebin(&h.r),
                g: rebin(&h.g),
                b: rebin(&h.b),
                y: rebin(&h.y),
                below: Channels::of(h.below, h.samples),
                above: Channels::of(h.above, h.samples),
            },
            waveform: WaveformSummary {
                columns: self.params.columns.max(1),
                rows: self.params.rows.max(2),
                buckets: SUMMARY_BUCKETS,
                luma: cols(&self.luma, "Y"),
                parade: ParadeSummary {
                    r: cols(&self.parade, "R"),
                    g: cols(&self.parade, "G"),
                    b: cols(&self.parade, "B"),
                },
            },
            vectorscope: VectorscopeSummary {
                size: v.size,
                extent: v.extent,
                samples: v.samples,
                mean: [round2(mcb * 100.0), round2(mcr * 100.0)],
                peaks: peaks(v, PEAKS, PEAK_SHARE),
                targets: targets(self.params.matrix, 0.75)
                    .iter()
                    .map(|t| TargetSummary {
                        name: t.name,
                        cb: round2(t.cb * 100.0),
                        cr: round2(t.cr * 100.0),
                        magnitude: round2(t.cb.hypot(t.cr) / 0.5 * 100.0),
                    })
                    .collect(),
            },
        }
    }
}

// ------------------------------------------------------------------------------------ files

/// What was analysed.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Source {
    /// `video` or `still`.
    pub kind: &'static str,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
    /// Codec (video) or decoder (still).
    pub decoder: String,
    /// FFmpeg pixel format (video) or sample format (still).
    pub pixel_format: String,
    /// Y'CbCr matrix the scopes use.
    pub matrix: Matrix,
    /// Whether the stream tagged its matrix (otherwise it follows from the height, or is BT.709 for stills).
    pub matrix_tagged: bool,
    /// `limited` or `full` for Y'CbCr video; None for RGB.
    pub range: Option<&'static str>,
    /// Bits per component.
    pub depth: u32,
    /// Floating-point samples.
    pub float: bool,
    /// The still stores linear scene light (HDR, OpenEXR), not display-encoded values.
    pub linear: bool,
}

/// Frames analysed.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Frames {
    /// Frames measured.
    pub analysed: u64,
    /// Every `step`-th frame was measured.
    pub step: u64,
}

/// The scopes report of a file.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Report {
    /// [`SCHEMA`].
    pub schema: &'static str,
    /// The file.
    pub file: String,
    /// What was analysed.
    pub source: Source,
    /// Frames analysed.
    pub frames: Frames,
    /// Levels of every analysed frame together.
    pub levels: Levels,
    /// The worst frame of each measure.
    pub worst: WorstFrames,
    /// Scope summaries.
    pub scopes: Summary,
    /// Decoding problems (damaged or truncated media).
    pub warnings: Vec<String>,
}

fn pct(v: f64) -> String {
    format!("{:.2}%", v * 100.0)
}

impl Report {
    /// The report as text.
    pub fn human(&self) -> String {
        use std::fmt::Write;
        let s = &self.source;
        let l = &self.levels;
        let mut o = String::new();
        let range = s.range.map(|r| format!(", {r} range")).unwrap_or_default();
        let tagged = if s.matrix_tagged { "" } else { " (untagged)" };
        let _ = writeln!(
            o,
            "{}: {} {}×{}, {} {}-bit{range}",
            self.file, s.kind, s.width, s.height, s.pixel_format, s.depth
        );
        let _ = writeln!(
            o,
            "  matrix {}{tagged}; {} frame(s) analysed, step {}",
            s.matrix.name(),
            self.frames.analysed,
            self.frames.step
        );
        let ch = |c: &Channels| format!("R {} G {} B {} Y {}", pct(c.r), pct(c.g), pct(c.b), pct(c.y));
        let _ = writeln!(o, "  black clipped  {}", ch(&l.clipping.black));
        let _ = writeln!(o, "  white clipped  {}", ch(&l.clipping.white));
        if let Some(g) = &l.legal {
            let _ = writeln!(
                o,
                "  illegal        {} (Y {}–{}: {} below, {} above; C {}–{}: Cb {} / {}, Cr {} / {})",
                pct(l.illegal.unwrap_or(0.0)),
                g.y[0],
                g.y[1],
                pct(g.below.y),
                pct(g.above.y),
                g.c[0],
                g.c[1],
                pct(g.below.cb),
                pct(g.above.cb),
                pct(g.below.cr),
                pct(g.above.cr)
            );
        } else if let Some(f) = &l.float_range {
            let _ = writeln!(
                o,
                "  illegal        {} (below 0: R {} G {} B {}; above 1: R {} G {} B {}; not finite {})",
                pct(l.illegal.unwrap_or(0.0)),
                pct(f.below.r),
                pct(f.below.g),
                pct(f.below.b),
                pct(f.above.r),
                pct(f.above.g),
                pct(f.above.b),
                pct(f.non_finite)
            );
        } else {
            let _ = writeln!(o, "  illegal        n/a (no legal range for this source)");
        }
        let sat = &l.saturation;
        let _ = writeln!(o, "  out of gamut   {}", pct(sat.out_of_gamut));
        let _ = writeln!(
            o,
            "  chroma         {} above {:.4}; max {}",
            pct(sat.above_limit),
            sat.chroma_limit,
            sat.max_chroma.map_or("n/a".into(), |m| format!("{m:.4}"))
        );
        let w = &self.worst;
        for (name, v) in [
            ("black clipped", w.black_clipped),
            ("white clipped", w.white_clipped),
            ("illegal", w.illegal),
            ("out of gamut", w.out_of_gamut),
            ("above chroma", w.above_chroma_limit),
        ] {
            if let Some(v) = v.filter(|v| v.value > 0.0) {
                let _ = writeln!(o, "  worst {name}: frame {} ({})", v.frame, pct(v.value));
            }
        }
        if let Some(c) = &self.scopes.channels {
            let lv = |l: &Level| format!("{:.2}/{:.2}/{:.2}", l.min, l.mean, l.max);
            let _ = writeln!(o, "  levels min/mean/max %: R {} G {} B {} Y {}", lv(&c.r), lv(&c.g), lv(&c.b), lv(&c.y));
            let _ = writeln!(o, "  mean chroma: hue {:.2}°, saturation {:.2}%", c.hue_deg, c.saturation);
        }
        for warning in &self.warnings {
            let _ = writeln!(o, "  warning: {warning}");
        }
        o
    }
}

/// Analyses a video (every `step`-th frame) or a still image file.
pub fn analyse(path: &Path, step: u64, chroma_limit: Option<f32>) -> Result<Report, ScopeError> {
    if step == 0 {
        return Err(ScopeError::Invalid("step must be at least 1".into()));
    }
    let step_frames = i64::try_from(step).map_err(|_| ScopeError::Invalid(format!("step {step} is too large")))?;
    check_limit(chroma_limit)?;
    std::fs::metadata(path).map_err(|source| ScopeError::Open { path: path.to_path_buf(), source })?;
    let file = path.display().to_string();
    if still::is_still(path) {
        let st = still::open(path).map_err(ScopeError::Still)?;
        let img = &st.image;
        let colour = img.color();
        let mut scopes = Scopes::new(Params::default(), chroma_limit)?;
        scopes.add(0, &Signal::from_image(img)?);
        let source = Source {
            kind: "still",
            width: img.width(),
            height: img.height(),
            decoder: st.decoder.clone(),
            pixel_format: format!("{colour:?}").to_lowercase(),
            matrix: Matrix::Bt709,
            matrix_tagged: false,
            range: None,
            depth: colour.bytes_per_pixel() as u32 * 8 / (colour.channel_count() as u32).max(1),
            float: matches!(img, DynamicImage::ImageRgb32F(_) | DynamicImage::ImageRgba32F(_)),
            linear: st.linear,
        };
        return Ok(report(file, source, step, &scopes, Vec::new()));
    }
    let mut dec = VideoDecoder::open(path, None, LOOKAHEAD)?;
    let info = dec.info.clone();
    let (matrix, tagged) = Matrix::from_tag(&info.matrix, info.height);
    let mut scopes = Scopes::new(Params { matrix, ..Params::default() }, chroma_limit)?;
    let mut i = 0i64;
    loop {
        let f = dec.frame(i)?;
        // past the end the decoder holds the last frame
        if f.index < i {
            break;
        }
        scopes.add(f.index, &Signal::from_frame(&f, &info, matrix)?);
        i = match i.checked_add(step_frames) {
            Some(n) => n,
            None => break,
        };
    }
    let yuv = matches!(dec.layout, PixelLayout::Yuv8(..) | PixelLayout::Yuv16(..));
    let source = Source {
        kind: "video",
        width: info.width,
        height: info.height,
        decoder: info.codec.clone(),
        pixel_format: info.pix_fmt.clone(),
        matrix,
        matrix_tagged: tagged,
        range: yuv.then_some(if info.full_range { "full" } else { "limited" }),
        depth: info.depth,
        float: false,
        linear: false,
    };
    Ok(report(file, source, step, &scopes, dec.warnings.clone()))
}

fn report(file: String, source: Source, step: u64, scopes: &Scopes, warnings: Vec<String>) -> Report {
    Report {
        schema: SCHEMA,
        file,
        source,
        frames: Frames { analysed: scopes.frames(), step },
        levels: scopes.levels(),
        worst: scopes.worst(),
        scopes: scopes.summary(),
        warnings,
    }
}
