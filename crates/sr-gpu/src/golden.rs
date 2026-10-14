//! Golden-frame comparison. A frame matches its golden when PSNR ≥ 50 dB
//! and the largest CIEDE2000 difference is ≤ 1, both measured on
//! display-referred sRGB.

/// Comparison of two images of equal size (straight RGBA, display sRGB in [0, 1]).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Comparison {
    /// Peak signal-to-noise ratio over RGBA, dB (infinite when identical).
    pub psnr: f64,
    /// Largest per-pixel ΔE2000.
    pub max_delta_e: f64,
    /// Mean ΔE2000.
    pub mean_delta_e: f64,
}

impl Comparison {
    /// The acceptance rule for golden frames.
    pub fn passes(&self) -> bool {
        self.psnr >= 50.0 && self.max_delta_e <= 1.0
    }
}

fn lin(v: f64) -> f64 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// Display sRGB → CIE L*a*b* (D65).
pub fn srgb_to_lab(c: [f64; 3]) -> [f64; 3] {
    let [r, g, b] = c.map(lin);
    let x = (0.4124564 * r + 0.3575761 * g + 0.1804375 * b) / 0.95047;
    let y = 0.2126729 * r + 0.7151522 * g + 0.0721750 * b;
    let z = (0.0193339 * r + 0.1191920 * g + 0.9503041 * b) / 1.08883;
    let f = |t: f64| if t > 216.0 / 24389.0 { t.cbrt() } else { (24389.0 / 27.0 * t + 16.0) / 116.0 };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// CIEDE2000 colour difference (Sharma, Wu and Dalal 2005).
pub fn delta_e2000(a: [f64; 3], b: [f64; 3]) -> f64 {
    use std::f64::consts::PI;
    let deg = |r: f64| r * 180.0 / PI;
    let rad = |d: f64| d * PI / 180.0;
    let c1 = (a[1] * a[1] + a[2] * a[2]).sqrt();
    let c2 = (b[1] * b[1] + b[2] * b[2]).sqrt();
    let cm = (c1 + c2) / 2.0;
    let g = 0.5 * (1.0 - (cm.powi(7) / (cm.powi(7) + 25f64.powi(7))).sqrt());
    let (a1, a2) = (a[1] * (1.0 + g), b[1] * (1.0 + g));
    let (c1p, c2p) = ((a1 * a1 + a[2] * a[2]).sqrt(), (a2 * a2 + b[2] * b[2]).sqrt());
    let hue = |x: f64, y: f64| {
        if x == 0.0 && y == 0.0 {
            0.0
        } else {
            let h = deg(y.atan2(x));
            if h < 0.0 {
                h + 360.0
            } else {
                h
            }
        }
    };
    let (h1, h2) = (hue(a1, a[2]), hue(a2, b[2]));
    let dl = b[0] - a[0];
    let dc = c2p - c1p;
    let dh = if c1p * c2p == 0.0 {
        0.0
    } else if (h2 - h1).abs() <= 180.0 {
        h2 - h1
    } else if h2 - h1 > 180.0 {
        h2 - h1 - 360.0
    } else {
        h2 - h1 + 360.0
    };
    let dhh = 2.0 * (c1p * c2p).sqrt() * rad(dh / 2.0).sin();
    let lm = (a[0] + b[0]) / 2.0;
    let cmp = (c1p + c2p) / 2.0;
    let hm = if c1p * c2p == 0.0 {
        h1 + h2
    } else if (h1 - h2).abs() <= 180.0 {
        (h1 + h2) / 2.0
    } else if h1 + h2 < 360.0 {
        (h1 + h2 + 360.0) / 2.0
    } else {
        (h1 + h2 - 360.0) / 2.0
    };
    let t = 1.0 - 0.17 * rad(hm - 30.0).cos() + 0.24 * rad(2.0 * hm).cos() + 0.32 * rad(3.0 * hm + 6.0).cos()
        - 0.20 * rad(4.0 * hm - 63.0).cos();
    let dtheta = 30.0 * (-((hm - 275.0) / 25.0).powi(2)).exp();
    let rc = 2.0 * (cmp.powi(7) / (cmp.powi(7) + 25f64.powi(7))).sqrt();
    let sl = 1.0 + 0.015 * (lm - 50.0).powi(2) / (20.0 + (lm - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * cmp;
    let sh = 1.0 + 0.015 * cmp * t;
    let rt = -rad(2.0 * dtheta).sin() * rc;
    ((dl / sl).powi(2) + (dc / sc).powi(2) + (dhh / sh).powi(2) + rt * (dc / sc) * (dhh / sh)).sqrt()
}

/// Compares two images of straight display-sRGB RGBA values in [0, 1].
pub fn compare(a: &[[f64; 4]], b: &[[f64; 4]]) -> Comparison {
    assert_eq!(a.len(), b.len(), "images differ in size");
    let mut se = 0.0;
    let (mut max_de, mut sum_de) = (0.0f64, 0.0);
    for (p, q) in a.iter().zip(b) {
        for k in 0..4 {
            se += (p[k] - q[k]).powi(2);
        }
        // compare colours composited over mid grey so alpha differences count
        let over = |c: [f64; 4]| [0, 1, 2].map(|k| c[k] * c[3] + 0.5 * (1.0 - c[3]));
        let de = delta_e2000(srgb_to_lab(over(*p)), srgb_to_lab(over(*q)));
        max_de = max_de.max(de);
        sum_de += de;
    }
    let mse = se / (a.len().max(1) * 4) as f64;
    Comparison {
        psnr: if mse == 0.0 { f64::INFINITY } else { 10.0 * (1.0 / mse).log10() },
        max_delta_e: max_de,
        mean_delta_e: sum_de / a.len().max(1) as f64,
    }
}

/// 8-bit RGBA bytes → normalised values.
pub fn from_rgba8(px: &[u8]) -> Vec<[f64; 4]> {
    px.as_chunks::<4>().0.iter().map(|c| c.map(|v| v as f64 / 255.0)).collect()
}

/// An image that cannot be measured.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum GoldenError {
    /// The two images hold different numbers of bytes.
    #[error("the images differ in length: {actual} bytes against {expected}")]
    Length { actual: usize, expected: usize },
    /// The byte count is not a whole number of RGBA8 pixels.
    #[error("{bytes} bytes is not a whole number of RGBA8 pixels")]
    NotRgba8 { bytes: usize },
    /// The pixel count does not match the stated size.
    #[error("a {width}×{height} image holds {} pixels, found {found}", *width as u64 * *height as u64)]
    Dimensions { width: u32, height: u32, found: usize },
    /// There is nothing to measure: no pixels, or a zero size.
    #[error("the image is empty")]
    Empty,
}

/// One colour channel of an RGBA image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Channel {
    R,
    G,
    B,
    A,
}

/// A statistic of the per-pixel absolute difference in one channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Statistic {
    Max,
    Mean,
    P99,
}

/// Absolute difference between two RGBA8 images in one channel, in 8-bit
/// code values.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct ChannelError {
    /// Largest difference of any pixel.
    pub max: u8,
    /// Mean difference over all pixels.
    pub mean: f64,
    /// 99th percentile of the differences, nearest-rank: the smallest value
    /// that at least 99 % of the pixels do not exceed.
    pub p99: u8,
}

/// Bounds on [`ChannelError`], applied to every channel alike.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Tolerance {
    pub max: u8,
    pub mean: f64,
    pub p99: u8,
}

/// A statistic of one channel that exceeds its bound.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Breach {
    pub channel: Channel,
    pub statistic: Statistic,
    pub value: f64,
    pub limit: f64,
}

impl Tolerance {
    /// Every statistic of every channel above its bound, in channel order
    /// (R, G, B, A) and then statistic order (max, mean, p99). An empty list
    /// is a match.
    pub fn breaches(&self, errors: &[ChannelError; 4]) -> Vec<Breach> {
        let mut out = Vec::new();
        for (channel, e) in [Channel::R, Channel::G, Channel::B, Channel::A].into_iter().zip(errors) {
            let measured = [
                (Statistic::Max, e.max as f64, self.max as f64),
                (Statistic::Mean, e.mean, self.mean),
                (Statistic::P99, e.p99 as f64, self.p99 as f64),
            ];
            for (statistic, value, limit) in measured {
                if value > limit {
                    out.push(Breach { channel, statistic, value, limit });
                }
            }
        }
        out
    }
}

/// Bounds for the corpus golden frames, which are 8-bit thumbnails of at most
/// 64 px on the long side ([`thumbnail`]).
///
/// - `p99` ≤ 2: a change of shading, colour management or compositing moves
///   far more than 1 % of a thumbnail's pixels by more than two code values;
///   rounding noise between runs of one adapter does not.
/// - `mean` ≤ 0.5: catches a uniform shift of one code value over the whole
///   frame, which `p99` and `max` both allow.
/// - `max` ≤ 32: a few edge pixels may move with sub-pixel geometry, but a
///   missing or misplaced element moves its pixels by more.
pub const CORPUS_TOLERANCE: Tolerance = Tolerance { max: 32, mean: 0.5, p99: 2 };

/// Per-channel absolute difference between two RGBA8 images of equal size.
pub fn channel_errors(actual: &[u8], expected: &[u8]) -> Result<[ChannelError; 4], GoldenError> {
    if actual.len() != expected.len() {
        return Err(GoldenError::Length { actual: actual.len(), expected: expected.len() });
    }
    if !actual.len().is_multiple_of(4) {
        return Err(GoldenError::NotRgba8 { bytes: actual.len() });
    }
    if actual.is_empty() {
        return Err(GoldenError::Empty);
    }
    let n = actual.len() / 4;
    let mut histograms = [[0usize; 256]; 4];
    for (p, q) in actual.as_chunks::<4>().0.iter().zip(expected.as_chunks::<4>().0) {
        for (c, h) in histograms.iter_mut().enumerate() {
            h[p[c].abs_diff(q[c]) as usize] += 1;
        }
    }
    // nearest rank: ceil(0.99 n), counted from 1
    let rank = (99 * n).div_ceil(100);
    Ok(histograms.map(|h| {
        let (mut max, mut sum, mut seen, mut p99) = (0u8, 0u64, 0usize, None);
        for (d, &count) in h.iter().enumerate() {
            if count == 0 {
                continue;
            }
            max = d as u8;
            sum += d as u64 * count as u64;
            seen += count;
            if p99.is_none() && seen >= rank {
                p99 = Some(d as u8);
            }
        }
        ChannelError { max, mean: sum as f64 / n as f64, p99: p99.unwrap_or(max) }
    }))
}

/// Source cells of an area-average resampling of `n` samples to `m`: for each
/// output sample, the covered source indices with their coverage.
fn cells(n: usize, m: usize) -> Vec<Vec<(usize, f64)>> {
    (0..m)
        .map(|i| {
            let lo = i as f64 * n as f64 / m as f64;
            let hi = (i + 1) as f64 * n as f64 / m as f64;
            (lo.floor() as usize..(hi.ceil() as usize).min(n))
                .map(|j| (j, hi.min(j as f64 + 1.0) - lo.max(j as f64)))
                .filter(|&(_, w)| w > 0.0)
                .collect()
        })
        .collect()
}

/// Area average of a `w`×`h` image to `m`×`k`, each output pixel the
/// coverage-weighted mean of the source pixels under it.
fn area_average(px: impl Fn(usize) -> [f64; 4], [w, h]: [usize; 2], [m, k]: [usize; 2]) -> Vec<[f64; 4]> {
    let (xs, ys) = (cells(w, m), cells(h, k));
    let mut out = Vec::with_capacity(m * k);
    for row in &ys {
        for col in &xs {
            let (mut acc, mut total) = ([0.0f64; 4], 0.0);
            for &(y, wy) in row {
                for &(x, wx) in col {
                    for (a, v) in acc.iter_mut().zip(px(y * w + x)) {
                        *a += wx * wy * v;
                    }
                    total += wx * wy;
                }
            }
            out.push(acc.map(|v| v / total));
        }
    }
    out
}

/// Reduces a premultiplied linear-light frame to at most `side` pixels on its
/// long side by an exact area average, keeping the aspect ratio. Averaging
/// premultiplied linear light is the physically correct reduction: edges keep
/// their coverage and colours do not darken. A frame already within `side` is
/// returned unchanged.
pub fn thumbnail(px: &[[f32; 4]], size: [u32; 2], side: u32) -> Result<(Vec<[f32; 4]>, [u32; 2]), GoldenError> {
    let [w, h] = size;
    if w == 0 || h == 0 || side == 0 {
        return Err(GoldenError::Empty);
    }
    if px.len() as u64 != w as u64 * h as u64 {
        return Err(GoldenError::Dimensions { width: w, height: h, found: px.len() });
    }
    let long = w.max(h);
    if long <= side {
        return Ok((px.to_vec(), size));
    }
    let scale = |v: u32| ((v as f64 * side as f64 / long as f64).round() as u32).max(1);
    let out = [scale(w), scale(h)];
    let avg = area_average(|i| px[i].map(|v| v as f64), [w as usize, h as usize], [out[0] as usize, out[1] as usize]);
    Ok((avg.into_iter().map(|p| p.map(|v| v as f32)).collect(), out))
}

/// Mid grey in 8-bit code values: the ground straight-alpha pixels are seen
/// over, so a transparent pixel differs from an opaque black one.
const GROUND: f64 = 128.0;

/// RGBA8 straight-alpha pixels composited over [`GROUND`], in code values.
fn over_ground(px: &[u8]) -> impl Fn(usize) -> [f64; 4] + '_ {
    move |i| {
        let p = &px[i * 4..i * 4 + 4];
        let a = p[3] as f64 / 255.0;
        let c = |k: usize| p[k] as f64 * a + GROUND * (1.0 - a);
        [c(0), c(1), c(2), 255.0]
    }
}

fn checked_rgba8(px: &[u8], [w, h]: [u32; 2]) -> Result<usize, GoldenError> {
    if !px.len().is_multiple_of(4) {
        return Err(GoldenError::NotRgba8 { bytes: px.len() });
    }
    let n = px.len() / 4;
    if n as u64 != w as u64 * h as u64 {
        return Err(GoldenError::Dimensions { width: w, height: h, found: n });
    }
    if n == 0 {
        return Err(GoldenError::Empty);
    }
    Ok(n)
}

/// Side of the common grid [`distinctness`] reduces both images to.
const GRID: usize = 16;

/// How different two straight-alpha RGBA8 images look: both are composited
/// over mid grey, area-averaged to a common 16×16 grid, and compared as the
/// mean absolute RGB difference in code values (0 for the same picture, 255
/// for black against white). Images of different sizes and aspect ratios are
/// comparable, and the grid makes the measure insensitive to sub-pixel noise.
pub fn distinctness(a: &[u8], a_size: [u32; 2], b: &[u8], b_size: [u32; 2]) -> Result<f64, GoldenError> {
    checked_rgba8(a, a_size)?;
    checked_rgba8(b, b_size)?;
    let grid = |px: &[u8], [w, h]: [u32; 2]| area_average(over_ground(px), [w as usize, h as usize], [GRID, GRID]);
    let (ga, gb) = (grid(a, a_size), grid(b, b_size));
    let sum: f64 = ga.iter().zip(&gb).map(|(p, q)| (0..3).map(|c| (p[c] - q[c]).abs()).sum::<f64>()).sum();
    Ok(sum / (GRID * GRID * 3) as f64)
}

/// The largest range, over R, G and B, of a straight-alpha RGBA8 image
/// composited over mid grey, in code values. Zero means a uniform frame.
pub fn spread(px: &[u8]) -> Result<u8, GoldenError> {
    if !px.len().is_multiple_of(4) {
        return Err(GoldenError::NotRgba8 { bytes: px.len() });
    }
    if px.is_empty() {
        return Err(GoldenError::Empty);
    }
    let seen = over_ground(px);
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for i in 0..px.len() / 4 {
        for ((l, u), v) in lo.iter_mut().zip(&mut hi).zip(seen(i)) {
            *l = l.min(v);
            *u = u.max(v);
        }
    }
    let range = lo.iter().zip(&hi).map(|(l, u)| u - l).fold(0.0, f64::max);
    Ok(range.round() as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ciede2000_matches_the_published_test_data() {
        // Sharma, Wu and Dalal (2005), table 1, pairs 1, 7 and 19
        let cases = [
            ([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485], 2.0425),
            ([50.0, 0.0, 0.0], [50.0, -1.0, 2.0], 2.3669),
            ([50.0, 2.5, 0.0], [50.0, 0.0, -2.5], 4.3065),
        ];
        for (a, b, want) in cases {
            let got = delta_e2000(a, b);
            assert!((got - want).abs() < 1e-4, "{a:?} {b:?}: {got} vs {want}");
        }
    }

    #[test]
    fn psnr_and_acceptance() {
        let a = vec![[0.5, 0.5, 0.5, 1.0]; 100];
        assert!(compare(&a, &a).passes());
        let b = vec![[0.5 + 1.0 / 255.0, 0.5, 0.5, 1.0]; 100];
        let c = compare(&a, &b);
        assert!(c.psnr > 50.0 && c.max_delta_e < 1.0 && c.passes(), "{c:?}");
        let d = vec![[0.6, 0.5, 0.5, 1.0]; 100];
        assert!(!compare(&a, &d).passes());
    }

    /// `n` opaque mid-grey pixels as RGBA8 bytes.
    fn grey(n: usize) -> Vec<u8> {
        [128, 128, 128, 255].repeat(n)
    }

    /// `px` with channel `c` of the first `count` pixels raised by `by`.
    fn raised(px: &[u8], c: usize, count: usize, by: u8) -> Vec<u8> {
        let mut out = px.to_vec();
        for p in out.as_chunks_mut::<4>().0.iter_mut().take(count) {
            p[c] += by;
        }
        out
    }

    const TOL: Tolerance = Tolerance { max: 32, mean: 0.5, p99: 2 };

    #[test]
    fn identical_images_have_no_channel_error() {
        let a = grey(64);
        let e = channel_errors(&a, &a).unwrap();
        assert_eq!(e, [ChannelError { max: 0, mean: 0.0, p99: 0 }; 4]);
        assert!(TOL.breaches(&e).is_empty());
    }

    #[test]
    fn p99_is_the_nearest_rank_percentile_of_each_channel() {
        let a = grey(100);
        // one pixel in a hundred is the 100th rank: the 99th is still exact
        let one = channel_errors(&raised(&a, 0, 1, 50), &a).unwrap();
        assert_eq!(one[0], ChannelError { max: 50, mean: 0.5, p99: 0 });
        let two = channel_errors(&raised(&a, 0, 2, 50), &a).unwrap();
        assert_eq!(two[0], ChannelError { max: 50, mean: 1.0, p99: 50 });
        // the other channels are measured on their own
        assert_eq!(two[1], ChannelError { max: 0, mean: 0.0, p99: 0 });
        assert_eq!(two[3], ChannelError { max: 0, mean: 0.0, p99: 0 });
    }

    #[test]
    fn widespread_moderate_error_fails_on_p99_although_the_maximum_is_allowed() {
        let a = grey(10_000);
        // 2 % of the pixels 10 code values off: under the max bound and under the mean bound
        let e = channel_errors(&raised(&a, 1, 200, 10), &a).unwrap();
        assert_eq!(e[1], ChannelError { max: 10, mean: 0.2, p99: 10 });
        assert_eq!(
            TOL.breaches(&e),
            vec![Breach { channel: Channel::G, statistic: Statistic::P99, value: 10.0, limit: 2.0 }]
        );
    }

    #[test]
    fn a_uniform_shift_fails_on_the_mean_although_max_and_p99_are_allowed() {
        let a = grey(1000);
        let e = channel_errors(&raised(&a, 2, 1000, 1), &a).unwrap();
        assert_eq!(e[2], ChannelError { max: 1, mean: 1.0, p99: 1 });
        assert_eq!(
            TOL.breaches(&e),
            vec![Breach { channel: Channel::B, statistic: Statistic::Mean, value: 1.0, limit: 0.5 }]
        );
    }

    #[test]
    fn one_wild_pixel_fails_on_the_maximum_alone() {
        let a = grey(1000);
        let mut b = a.clone();
        b[3] = 0; // the first pixel turns transparent
        let e = channel_errors(&b, &a).unwrap();
        assert_eq!(e[3], ChannelError { max: 255, mean: 0.255, p99: 0 });
        assert_eq!(
            TOL.breaches(&e),
            vec![Breach { channel: Channel::A, statistic: Statistic::Max, value: 255.0, limit: 32.0 }]
        );
    }

    #[test]
    fn every_breached_statistic_of_every_channel_is_reported() {
        let a = grey(100);
        let b = [255, 0, 128, 255].repeat(100);
        let found = TOL.breaches(&channel_errors(&b, &a).unwrap());
        let mut want = Vec::new();
        for channel in [Channel::R, Channel::G] {
            for statistic in [Statistic::Max, Statistic::Mean, Statistic::P99] {
                let limit = match statistic {
                    Statistic::Max => 32.0,
                    Statistic::Mean => 0.5,
                    Statistic::P99 => 2.0,
                };
                want.push(Breach {
                    channel,
                    statistic,
                    value: if channel == Channel::R { 127.0 } else { 128.0 },
                    limit,
                });
            }
        }
        assert_eq!(found, want);
    }

    #[test]
    fn malformed_images_are_rejected_instead_of_compared() {
        let a = grey(4);
        assert_eq!(channel_errors(&a, &grey(5)), Err(GoldenError::Length { actual: 16, expected: 20 }));
        assert_eq!(channel_errors(&a[..15], &a[..15]), Err(GoldenError::NotRgba8 { bytes: 15 }));
        assert_eq!(channel_errors(&[], &[]), Err(GoldenError::Empty));
    }

    #[test]
    fn thumbnails_average_premultiplied_light_over_each_cell() {
        let px = [[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], [0.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 1.0]];
        // 4×1 to at most 2 px wide: each output pixel is the mean of two
        let near = |got: &[[f32; 4]], want: &[[f32; 4]]| {
            got.len() == want.len()
                && got.iter().flatten().zip(want.iter().flatten()).all(|(a, b)| (a - b).abs() < 1e-6)
        };
        let (t, size) = thumbnail(&px, [4, 1], 2).unwrap();
        assert_eq!(size, [2, 1]);
        assert!(near(&t, &[[0.5, 0.0, 0.5, 1.0], [0.0, 0.5, 0.0, 0.5]]), "{t:?}");
        // 3×1 to 2 px: the middle source pixel is split between the two cells
        let (t, size) = thumbnail(&px[..3], [3, 1], 2).unwrap();
        assert_eq!(size, [2, 1]);
        let third = |v: f32| v / 1.5;
        assert!(near(&t, &[[third(1.0), 0.0, third(0.5), 1.0], [0.0, 0.0, third(0.5), third(0.5)]]), "{t:?}");
    }

    #[test]
    fn thumbnails_keep_the_aspect_ratio_and_leave_small_frames_alone() {
        let wide = vec![[0.25f32; 4]; 480 * 270];
        let (t, size) = thumbnail(&wide, [480, 270], 64).unwrap();
        assert_eq!(size, [64, 36]);
        assert!(t.iter().all(|p| p.iter().all(|v| (v - 0.25).abs() < 1e-6)), "a flat frame stays flat");
        let small: Vec<[f32; 4]> = (0..32 * 32).map(|i| [i as f32, 0.0, 0.0, 1.0]).collect();
        assert_eq!(thumbnail(&small, [32, 32], 64).unwrap(), (small.clone(), [32, 32]));
        assert_eq!(
            thumbnail(&small, [16, 16], 64),
            Err(GoldenError::Dimensions { width: 16, height: 16, found: 1024 })
        );
        assert_eq!(thumbnail(&[], [0, 4], 64), Err(GoldenError::Empty));
        assert_eq!(thumbnail(&small, [32, 32], 0), Err(GoldenError::Empty));
    }

    #[test]
    fn distinctness_measures_the_mean_difference_over_a_common_grid() {
        let black = [0, 0, 0, 255].repeat(64 * 64);
        let white = [255, 255, 255, 255].repeat(64 * 36);
        assert_eq!(distinctness(&black, [64, 64], &black, [64, 64]).unwrap(), 0.0);
        assert_eq!(distinctness(&black, [64, 64], &white, [64, 36]).unwrap(), 255.0);
        // transparency is seen over mid grey, so a transparent frame is not a black one
        let clear = [0, 0, 0, 0].repeat(32 * 32);
        let d = distinctness(&clear, [32, 32], &black, [64, 64]).unwrap();
        assert!((d - 128.0).abs() < 1.0, "{d}");
        // a small mark on the same ground is a small difference
        let mut mark = black.clone();
        for p in mark.as_chunks_mut::<4>().0.iter_mut().take(64 * 4) {
            p[0] = 255;
        }
        let d = distinctness(&mark, [64, 64], &black, [64, 64]).unwrap();
        assert!((d - 255.0 / 16.0 / 3.0).abs() < 1e-9, "{d}");
        assert_eq!(distinctness(&black[..7], [1, 1], &black, [64, 64]), Err(GoldenError::NotRgba8 { bytes: 7 }));
        assert_eq!(
            distinctness(&black, [32, 32], &black, [64, 64]),
            Err(GoldenError::Dimensions { width: 32, height: 32, found: 4096 })
        );
    }

    #[test]
    fn spread_is_zero_only_for_a_uniform_frame() {
        assert_eq!(spread(&grey(16)).unwrap(), 0);
        let mut b = grey(16);
        b[4 * 7 + 2] = 200;
        assert_eq!(spread(&b).unwrap(), 72);
        // an alpha-only change shows over mid grey
        let mut c = [0, 0, 0, 255].repeat(4);
        c[3] = 0;
        assert_eq!(spread(&c).unwrap(), 128);
        assert_eq!(spread(&[]), Err(GoldenError::Empty));
    }
}
