//! ITU-R BS.1770-4 loudness, EBU Tech 3342 loudness range, true peak,
//! normalisation and the true-peak limiter.
//!
//! The loudness-range percentiles follow FilmCraft's meter
//! (crates/audio-dsp/src/loudness.rs, Copyright (c) 2026 ArtCraft Team and
//! the FilmCraft contributors, used under the MIT License; FilmCraft is MIT
//! OR Apache-2.0; see THIRD-PARTY-NOTICES.md), as do the Tech 3342 cases in
//! the tests.

use rayon::prelude::*;

use crate::dsp::{db_to_lin, lin_to_db, Biquad};

/// Samples per rayon task in the per-sample loops.
const CHUNK: usize = 8192;

/// Planar audio: one buffer per channel.
pub type Planar = Vec<Vec<f32>>;

/// The samples of a buffer that may be non-zero: every sample of every
/// channel outside `start..end` is a zero (of either sign). A bound, not
/// necessarily tight; empty when `end <= start`.
///
/// Work on a whole-programme buffer whose result outside the extent is
/// provably a zero, or unchanged, is skipped; work inside it runs with the
/// same operations in the same order, so the result is bit for bit what a
/// pass over the whole buffer produces. (A gain of zero or more keeps a
/// zero's sign, adding a zero to a running sum that starts positive
/// changes nothing, and everything the analysis derives is a square or a
/// magnitude, so the sign of a skipped zero never shows.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Extent {
    /// First sample that may be non-zero.
    pub start: usize,
    /// Past the last sample that may be non-zero.
    pub end: usize,
}

impl Extent {
    /// No samples.
    pub const EMPTY: Extent = Extent { start: 0, end: 0 };

    /// All `len` samples.
    pub fn full(len: usize) -> Extent {
        Extent { start: 0, end: len }
    }

    /// No sample may be non-zero.
    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }

    /// Number of samples.
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Whether `i` is inside.
    pub fn contains(&self, i: usize) -> bool {
        self.start <= i && i < self.end
    }

    /// The smallest extent covering both.
    pub fn union(self, o: Extent) -> Extent {
        if self.is_empty() {
            o
        } else if o.is_empty() {
            self
        } else {
            Extent { start: self.start.min(o.start), end: self.end.max(o.end) }
        }
    }

    /// Clipped to a buffer of `len` samples.
    pub fn clip(self, len: usize) -> Extent {
        Extent { start: self.start.min(len), end: self.end.min(len) }
    }

    /// Widened by `before` and `after` samples within a buffer of `len`
    /// samples (an empty extent stays empty).
    pub fn widen(self, before: usize, after: usize, len: usize) -> Extent {
        if self.is_empty() {
            return self;
        }
        Extent { start: self.start.saturating_sub(before), end: self.end.saturating_add(after).min(len) }
    }

    /// The tight extent of `buf`: from its first to past its last non-zero
    /// sample.
    pub fn of(buf: &Planar) -> Extent {
        Extent::within(buf, Extent::full(buf.first().map(Vec::len).unwrap_or(0)))
    }

    /// The tight extent of `buf` inside `bound`, itself an extent of `buf`.
    pub fn within(buf: &Planar, bound: Extent) -> Extent {
        let mut out = Extent::EMPTY;
        for ch in buf {
            let b = bound.clip(ch.len());
            let seg = &ch[b.start..b.end];
            let first = seg.iter().position(|v| *v != 0.0);
            let last = seg.iter().rposition(|v| *v != 0.0);
            if let (Some(s), Some(e)) = (first, last) {
                out = out.union(Extent { start: b.start + s, end: b.start + e + 1 });
            }
        }
        out
    }
}

/// The K-weighting pre-filter (high shelf) and RLB high-pass for any rate.
pub fn k_weighting(rate: f64) -> [Biquad; 2] {
    let shelf = {
        let (f0, g, q) = (1681.974450955533, 3.999843853973347, 0.7071752369554196);
        let k = (std::f64::consts::PI * f0 / rate).tan();
        let vh = 10f64.powf(g / 20.0);
        let vb = vh.powf(0.4996667741545416);
        let a0 = 1.0 + k / q + k * k;
        Biquad::raw(
            [(vh + vb * k / q + k * k) / a0, 2.0 * (k * k - vh) / a0, (vh - vb * k / q + k * k) / a0],
            [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        )
    };
    let hp = {
        let (f0, q) = (38.13547087602444, 0.5003270373238773);
        let k = (std::f64::consts::PI * f0 / rate).tan();
        let a0 = 1.0 + k / q + k * k;
        Biquad::raw([1.0, -2.0, 1.0], [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0])
    };
    [shelf, hp]
}

/// Mean-square energy of K-weighted audio in blocks of `block` samples every
/// `step`. The weighted energy is exactly zero before `ext` (the filters are
/// at rest) and once they have settled after it, so only that span is
/// filtered and summed; every block is still reported.
fn block_energies(buf: &Planar, ext: Extent, rate: f64, weights: &[f64], block: usize, step: usize) -> Vec<f64> {
    let n = buf.first().map(Vec::len).unwrap_or(0);
    let mut out = Vec::new();
    if n < block {
        return out;
    }
    let ext = ext.clip(n);
    // per weighted channel (independent, so in parallel): the span with non-zero energy and
    // prefix sums over it
    let prefix: Vec<Option<(Extent, Vec<f64>)>> = buf
        .par_iter()
        .enumerate()
        .map(|(c, ch)| {
            if weights.get(c).copied().unwrap_or(1.0) == 0.0 {
                return None;
            }
            let [mut a, mut b] = k_weighting(rate);
            let mut p = Vec::with_capacity(ext.len() + 1);
            p.push(0.0);
            let mut s = 0.0;
            let mut i = ext.start;
            while i < n && (i < ext.end || !(a.settled(Biquad::SETTLED_SQUARE) && b.settled(Biquad::SETTLED_SQUARE))) {
                let y = b.tick(a.tick(ch[i] as f64));
                s += y * y;
                p.push(s);
                i += 1;
            }
            Some((Extent { start: ext.start, end: i }, p))
        })
        .collect();
    let at =
        |span: Extent, p: &[f64], i: usize| if i <= span.start { 0.0 } else { p[(i - span.start).min(p.len() - 1)] };
    let mut start = 0;
    while start + block <= n {
        let mut e = 0.0;
        for (c, p) in prefix.iter().enumerate() {
            if let Some((span, p)) = p {
                let w = weights.get(c).copied().unwrap_or(1.0);
                e += w * (at(*span, p, start + block) - at(*span, p, start)) / block as f64;
            }
        }
        out.push(e);
        start += step;
    }
    out
}

fn lufs(e: f64) -> f64 {
    -0.691 + 10.0 * e.max(1e-20).log10()
}

/// Integrated loudness in LUFS (−∞ as −200 for silence).
pub fn integrated(buf: &Planar, rate: f64, weights: &[f64]) -> f64 {
    integrated_in(buf, Extent::of(buf), rate, weights)
}

/// [`integrated`] of a buffer that is zero outside `ext`.
pub fn integrated_in(buf: &Planar, ext: Extent, rate: f64, weights: &[f64]) -> f64 {
    gated(gating_blocks(buf, ext, rate, weights))
}

/// The energies of the 400 ms gating blocks, 100 ms (75 % overlap) apart:
/// the same windows as [`momentary`] every 100 ms, sample for sample.
fn gating_blocks(buf: &Planar, ext: Extent, rate: f64, weights: &[f64]) -> Vec<f64> {
    windows(buf, ext, rate, weights, 0.4, 0.1)
}

/// Integrated loudness of gating-block energies: −70 LUFS absolute gate,
/// then 10 LU under the loudness of the blocks that pass it.
fn gated(e: Vec<f64>) -> f64 {
    let abs: Vec<f64> = e.into_iter().filter(|&x| lufs(x) > -70.0).collect();
    if abs.is_empty() {
        return -200.0;
    }
    let rel = lufs(abs.iter().sum::<f64>() / abs.len() as f64) - 10.0;
    let gated: Vec<f64> = abs.into_iter().filter(|&x| lufs(x) > rel).collect();
    if gated.is_empty() {
        return -200.0;
    }
    lufs(gated.iter().sum::<f64>() / gated.len() as f64)
}

/// Short-term (3 s) loudness every `hop` seconds.
pub fn short_term(buf: &Planar, rate: f64, weights: &[f64], hop: f64) -> Vec<f64> {
    short_term_in(buf, Extent::of(buf), rate, weights, hop)
}

/// [`short_term`] of a buffer that is zero outside `ext`.
pub fn short_term_in(buf: &Planar, ext: Extent, rate: f64, weights: &[f64], hop: f64) -> Vec<f64> {
    windows(buf, ext, rate, weights, 3.0, hop).into_iter().map(lufs).collect()
}

/// Momentary (400 ms) loudness every `hop` seconds, LUFS; the first window
/// starts at the first sample and only whole windows are measured.
pub fn momentary(buf: &Planar, rate: f64, weights: &[f64], hop: f64) -> Vec<f64> {
    momentary_in(buf, Extent::of(buf), rate, weights, hop)
}

/// [`momentary`] of a buffer that is zero outside `ext`.
pub fn momentary_in(buf: &Planar, ext: Extent, rate: f64, weights: &[f64], hop: f64) -> Vec<f64> {
    windows(buf, ext, rate, weights, 0.4, hop).into_iter().map(lufs).collect()
}

/// The energies of `secs`-long windows every `hop` seconds.
fn windows(buf: &Planar, ext: Extent, rate: f64, weights: &[f64], secs: f64, hop: f64) -> Vec<f64> {
    let block = (secs * rate).round() as usize;
    let step = (hop * rate).round().max(1.0) as usize;
    block_energies(buf, ext, rate, weights, block, step)
}

/// Loudness range in LU (EBU Tech 3342): the spread between the 10th and
/// the 95th percentiles of the short-term loudness, every 100 ms, that
/// passes an absolute gate at −70 LUFS and a relative gate 20 LU under the
/// loudness of the windows that pass the first. 0 when no window passes,
/// which includes a programme shorter than 3 s.
pub fn loudness_range(buf: &Planar, rate: f64, weights: &[f64]) -> f64 {
    loudness_range_in(buf, Extent::of(buf), rate, weights)
}

/// [`loudness_range`] of a buffer that is zero outside `ext`.
pub fn loudness_range_in(buf: &Planar, ext: Extent, rate: f64, weights: &[f64]) -> f64 {
    range(windows(buf, ext, rate, weights, 3.0, 0.1))
}

/// Loudness range of short-term window energies.
fn range(e: Vec<f64>) -> f64 {
    let abs: Vec<f64> = e.into_iter().filter(|&x| lufs(x) >= -70.0).collect();
    if abs.is_empty() {
        return 0.0;
    }
    let rel = lufs(abs.iter().sum::<f64>() / abs.len() as f64) - 20.0;
    // the loudest window is at least the mean, so at least one passes
    let mut l: Vec<f64> = abs.into_iter().map(lufs).filter(|&x| x >= rel).collect();
    l.sort_by(f64::total_cmp);
    // nearest rank, as Tech 3342's reference code rounds
    let at = |p: f64| l[((l.len() - 1) as f64 * p).round() as usize];
    at(0.95) - at(0.10)
}

/// The EBU R 128 measurements of a programme. Loudness levels read −200
/// for silence, and the maxima −200 for a programme shorter than one
/// window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measurement {
    /// Integrated loudness, LUFS.
    pub integrated: f64,
    /// Loudness range, LU (see [`loudness_range`]).
    pub range: f64,
    /// Loudest momentary (400 ms) window, every 100 ms, LUFS.
    pub momentary_max: f64,
    /// Loudest short-term (3 s) window, every 100 ms, LUFS.
    pub short_term_max: f64,
    /// True peak, dBTP.
    pub true_peak: f64,
}

/// Every [`Measurement`] of a buffer at `rate` Hz, with one K-weighted pass
/// for the 400 ms windows and one for the 3 s windows.
pub fn measure(buf: &Planar, rate: f64, weights: &[f64]) -> Measurement {
    measure_in(buf, Extent::of(buf), rate, weights)
}

/// [`measure`] of a buffer that is zero outside `ext`.
pub fn measure_in(buf: &Planar, ext: Extent, rate: f64, weights: &[f64]) -> Measurement {
    // the gating blocks are the momentary windows: 400 ms, every 100 ms
    let blocks = gating_blocks(buf, ext, rate, weights);
    let momentary_max = loudest(&blocks);
    let short = windows(buf, ext, rate, weights, 3.0, 0.1);
    let short_term_max = loudest(&short);
    Measurement {
        integrated: gated(blocks),
        range: range(short),
        momentary_max,
        short_term_max,
        true_peak: true_peak_in(buf, ext, rate),
    }
}

/// The loudest of window energies, LUFS; −200 when there is none or all
/// are silent.
fn loudest(e: &[f64]) -> f64 {
    e.iter().map(|&x| lufs(x)).fold(-200.0, f64::max)
}

/// The oversampling factor true peak uses at `rate` Hz: the smallest power
/// of two from 4 up that brings the signal to 192 kHz or more, the rate
/// ITU-R BS.1770-4 Annex 2 aims its interpolator at, and at most 64. Rates
/// of 48 kHz and up keep 4×: the interpolator's error depends on the factor
/// and on the frequency as a fraction of the rate, not on the rate itself,
/// so 2× at 96 kHz would read peaks near its band edge lower than 4× does.
pub fn true_peak_factor(rate: f64) -> usize {
    let mut f = 4;
    while f < 64 && rate * (f as f64) < 192_000.0 {
        f *= 2;
    }
    f
}

/// Oversampling interpolator (windowed sinc, 12 taps per phase), with
/// [`true_peak_factor`] phases.
struct Oversampler {
    phases: Vec<[f64; 12]>,
}

impl Oversampler {
    fn new(rate: f64) -> Oversampler {
        let factor = true_peak_factor(rate);
        let mut phases = vec![[0.0; 12]; factor];
        for (p, row) in phases.iter_mut().enumerate() {
            for (k, v) in row.iter_mut().enumerate() {
                let x = (k as f64 - 5.5) - (p as f64 / factor as f64 - 0.5);
                let t = x * std::f64::consts::PI;
                let sinc = if x.abs() < 1e-9 { 1.0 } else { t.sin() / t };
                let w = 0.42
                    + 0.5 * (std::f64::consts::PI * x / 6.5).cos()
                    + 0.08 * (2.0 * std::f64::consts::PI * x / 6.5).cos();
                *v = sinc * w.max(0.0);
            }
            let s: f64 = row.iter().sum();
            for v in row.iter_mut() {
                *v /= s;
            }
        }
        Oversampler { phases }
    }

    /// Largest absolute inter-sample value around sample `i` of `ch`.
    fn peak_at(&self, ch: &[f32], i: usize) -> f64 {
        let mut m = (ch[i] as f64).abs();
        for row in &self.phases {
            let mut acc = 0.0;
            for (k, c) in row.iter().enumerate() {
                let j = i as i64 + k as i64 - 5;
                if j >= 0 && (j as usize) < ch.len() {
                    acc += c * ch[j as usize] as f64;
                }
            }
            m = m.max(acc.abs());
        }
        m
    }

    /// The samples whose inter-sample peak can be non-zero when the buffer
    /// is zero outside `ext`: the taps reach 5 samples back and 6 forward.
    fn reach(ext: Extent, len: usize) -> Extent {
        ext.clip(len).widen(6, 6, len)
    }
}

/// True peak in dBTP of a buffer at `rate` Hz, oversampled by
/// [`true_peak_factor`].
pub fn true_peak(buf: &Planar, rate: f64) -> f64 {
    true_peak_in(buf, Extent::of(buf), rate)
}

/// [`true_peak`] of a buffer that is zero outside `ext`.
pub fn true_peak_in(buf: &Planar, ext: Extent, rate: f64) -> f64 {
    let os = Oversampler::new(rate);
    // the maximum is exact whatever the order, and `f64::max` drops a NaN
    // against any partial maximum, as the running maximum did
    let mut m: f64 = 0.0;
    for ch in buf {
        let reach = Oversampler::reach(ext, ch.len());
        let peak = (reach.start..reach.end)
            .into_par_iter()
            .with_min_len(CHUNK)
            .map(|i| os.peak_at(ch, i))
            .reduce(|| 0.0, f64::max);
        m = m.max(peak);
    }
    lin_to_db(m)
}

/// Look-ahead true-peak limiter: gain never lets any channel exceed
/// `ceiling_db` true peak; recovery follows `release` seconds.
pub fn limit(buf: &mut Planar, rate: f64, ceiling_db: f64, release: f64) {
    limit_in(buf, Extent::of(buf), rate, ceiling_db, release)
}

/// [`limit`] of a buffer that is zero outside `ext`: the gain is 1 wherever
/// the oversampler sees only zeros, and applying it to zeros changes nothing.
pub fn limit_in(buf: &mut Planar, ext: Extent, rate: f64, ceiling_db: f64, release: f64) {
    let n = buf.first().map(Vec::len).unwrap_or(0);
    let ext = ext.clip(n);
    if ext.is_empty() {
        return;
    }
    let os = Oversampler::new(rate);
    let ceiling = db_to_lin(ceiling_db) * 0.999;
    let look = ((0.0015 * rate) as usize).max(1);
    // required gain per sample, up to the last one the look-ahead of the extent reads
    let stop = (ext.end + look).min(n);
    let reach = Oversampler::reach(ext, stop);
    // (each sample's requirement is its own minimum over the channels, in channel order)
    let mut need = vec![1.0f64; stop];
    need[reach.start..reach.end].par_chunks_mut(CHUNK).enumerate().for_each(|(k, chunk)| {
        for (j, nd) in chunk.iter_mut().enumerate() {
            let i = reach.start + k * CHUNK + j;
            for ch in buf.iter() {
                let p = os.peak_at(ch, i);
                if p > ceiling {
                    *nd = nd.min(ceiling / p);
                }
            }
        }
    });
    // spread each requirement over the look-ahead window before it (so the gain is already down)
    let mut env = vec![1.0f64; ext.end];
    let mut dq: std::collections::VecDeque<(usize, f64)> = Default::default();
    for i in 0..ext.end + look {
        if i < stop {
            while dq.back().is_some_and(|b| b.1 >= need[i]) {
                dq.pop_back();
            }
            dq.push_back((i, need[i]));
        }
        while dq.front().is_some_and(|f| f.0 + look < i) {
            dq.pop_front();
        }
        if i >= look && i - look < ext.end {
            env[i - look] = dq.front().map(|f| f.1).unwrap_or(1.0);
        }
    }
    // smooth: instant attack (the look-ahead already anticipates), exponential release
    let r = crate::dsp::coeff(release, rate);
    let mut g = 1.0;
    let mut out = vec![1.0; ext.end];
    for i in 0..ext.end {
        let target = env[i];
        g = if target < g { target } else { target + (g - target) * r };
        out[i] = g.min(env[i]);
    }
    for ch in buf.iter_mut() {
        for i in ext.start..ext.end {
            ch[i] = (ch[i] as f64 * out[i]) as f32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f64, amp: f64, secs: f64, rate: f64, phase: f64) -> Vec<f32> {
        (0..(secs * rate) as usize)
            .map(|i| (amp * (2.0 * std::f64::consts::PI * freq * i as f64 / rate + phase).sin()) as f32)
            .collect()
    }

    #[test]
    fn ebu_tech_3341_reference_levels() {
        // stereo 1 kHz sine at −23 dBFS reads −23.0 LUFS; at −33 dBFS reads −33.0 LUFS
        for (level, want) in [(-23.0, -23.0), (-33.0, -33.0)] {
            let s = sine(1000.0, db_to_lin(level), 20.0, 48000.0, 0.0);
            let l = integrated(&vec![s.clone(), s], 48000.0, &[1.0, 1.0]);
            assert!((l - want).abs() < 0.1, "{level} dBFS → {l} LUFS");
        }
        // the relative gate removes a quiet section: 10 s at −36 then 10 s at −23 dBFS... (Tech 3341 case 3)
        let mut s = sine(1000.0, db_to_lin(-36.0), 10.0, 48000.0, 0.0);
        s.extend(sine(1000.0, db_to_lin(-23.0), 60.0, 48000.0, 0.0));
        s.extend(sine(1000.0, db_to_lin(-36.0), 10.0, 48000.0, 0.0));
        let l = integrated(&vec![s.clone(), s], 48000.0, &[1.0, 1.0]);
        assert!((l + 23.0).abs() < 0.1, "{l}");
    }

    #[test]
    fn true_peak_sees_between_samples() {
        // fs/4 sine at 45°: samples reach 0.707·A, the waveform reaches A
        let s = sine(12000.0, 0.5, 1.0, 48000.0, std::f64::consts::FRAC_PI_4);
        let sample_peak = lin_to_db(s.iter().fold(0f32, |m, v| m.max(v.abs())) as f64);
        let tp = true_peak(&vec![s], 48000.0);
        assert!((sample_peak + 9.03).abs() < 0.1);
        assert!((tp + 6.02).abs() < 0.3, "true peak {tp}");
    }

    /// A sine with raised-cosine fades of 50 ms at each end, rounded to f32.
    fn faded(freq: f64, amp: f64, secs: f64, rate: f64, phase: f64) -> Vec<f32> {
        let n = (secs * rate) as usize;
        let ramp = (0.05 * rate) as usize;
        (0..n)
            .map(|i| {
                let edge = i.min(n - 1 - i);
                let g = if edge < ramp {
                    0.5 - 0.5 * (std::f64::consts::PI * edge as f64 / ramp as f64).cos()
                } else {
                    1.0
                };
                (amp * g * (2.0 * std::f64::consts::PI * freq * i as f64 / rate + phase).sin()) as f32
            })
            .collect()
    }

    #[test]
    fn true_peak_at_44k1_reaches_the_waveform_peak() {
        // fs/4 at 0.4375π: every crest falls 1/8 of a sample from the nearest
        // point of a 4× grid, which reads 0.17 dB low; the 192 kHz grid the
        // standard aims for (8× at 44.1 kHz) lands on it
        let rate = 44100.0;
        let s = faded(rate / 4.0, 0.5, 0.25, rate, 0.4375 * std::f64::consts::PI);
        let tp = true_peak(&vec![s], rate);
        assert!((tp - lin_to_db(0.5)).abs() < 0.02, "true peak {tp}, waveform peak {}", lin_to_db(0.5));
    }

    #[test]
    fn true_peak_factor_reaches_192_khz() {
        let table = [
            (8000.0, 32),
            (32000.0, 8),
            (44100.0, 8),
            (48000.0, 4),
            (88200.0, 4),
            (96000.0, 4),
            (192000.0, 4),
            (384000.0, 4),
            (1.0, 64),
            (0.0, 64),
            (f64::INFINITY, 4),
            (f64::NAN, 4),
        ];
        for (rate, want) in table {
            assert_eq!(true_peak_factor(rate), want, "{rate} Hz");
        }
    }

    #[test]
    fn true_peak_keeps_4x_at_high_rates() {
        // 96 kHz, fs/8 with every crest a quarter sample from a sample: on
        // the 4× grid, while 2× would read 0.17 dB low
        let rate = 96000.0;
        let phase = std::f64::consts::FRAC_PI_2 - 2.0 * std::f64::consts::PI * 0.125 * 0.25;
        let tp = true_peak(&vec![faded(rate / 8.0, 0.5, 0.25, rate, phase)], rate);
        assert!((tp - lin_to_db(0.5)).abs() < 0.02, "96 kHz true peak {tp}");
        // 192 kHz, fs/4 at 45°: the samples miss the crest by 3 dB
        let rate = 192000.0;
        let tp = true_peak(&vec![faded(rate / 4.0, 0.5, 0.25, rate, std::f64::consts::FRAC_PI_4)], rate);
        assert!((tp - lin_to_db(0.5)).abs() < 0.02, "192 kHz true peak {tp}");
    }

    #[test]
    fn limiter_caps_true_peak_at_44k1() {
        // the limiter reads the same 8× grid as the meter at 44.1 kHz
        let rate = 44100.0;
        let s = faded(rate / 4.0, 1.0, 0.5, rate, 0.4375 * std::f64::consts::PI);
        let mut b = vec![s.clone(), s];
        limit(&mut b, rate, -1.0, 0.05);
        let tp = true_peak(&b, rate);
        assert!(tp <= -0.95, "{tp}");
        assert!(tp > -1.5, "{tp}");
    }

    #[test]
    fn limiter_caps_true_peak() {
        let s = sine(997.0, 1.0, 2.0, 48000.0, 0.3);
        let mut b = vec![s.clone(), s];
        limit(&mut b, 48000.0, -1.0, 0.05);
        let tp = true_peak(&b, 48000.0);
        assert!(tp <= -0.95, "{tp}");
        assert!(tp > -1.5, "{tp}");
    }

    /// A 1 kHz tone through consecutive `(seconds, dBFS)` segments, with
    /// continuous phase, as a stereo pair: each segment reads its level in
    /// LUFS.
    fn segments(rate: f64, segs: &[(f64, f64)]) -> Planar {
        let mut s = Vec::new();
        let mut phase = 0.0f64;
        let dp = 2.0 * std::f64::consts::PI * 1000.0 / rate;
        for &(secs, level) in segs {
            let a = db_to_lin(level);
            for _ in 0..(secs * rate).round() as usize {
                s.push((a * phase.sin()) as f32);
                phase = (phase + dp) % (2.0 * std::f64::consts::PI);
            }
        }
        vec![s.clone(), s]
    }

    #[test]
    fn ebu_tech_3342_loudness_range() {
        // 20 s per level: the windows that straddle a step sit between the
        // percentiles, and the −50 LUFS steps of the last case fall under
        // the relative gate
        let cases: [(&[f64], f64); 4] = [
            (&[-20.0, -30.0], 10.0),
            (&[-20.0, -15.0], 5.0),
            (&[-40.0, -20.0], 20.0),
            (&[-50.0, -35.0, -20.0, -35.0, -50.0], 15.0),
        ];
        for (levels, want) in cases {
            let segs: Vec<(f64, f64)> = levels.iter().map(|&l| (20.0, l)).collect();
            let buf = segments(48000.0, &segs);
            let lra = loudness_range(&buf, 48000.0, &[1.0, 1.0]);
            assert!((lra - want).abs() < 0.01, "{levels:?}: LRA {lra}, want {want}");
        }
    }

    #[test]
    fn momentary_reads_each_400_ms() {
        // 0.18 s at −20 then 0.22 s at −30 dBFS, repeated: every 400 ms
        // window holds one of each, so momentary loudness is constant
        let rate = 48000.0;
        let segs: Vec<(f64, f64)> = (0..25).flat_map(|_| [(0.18, -20.0), (0.22, -30.0)]).collect();
        let buf = segments(rate, &segs);
        let want = 10.0 * ((0.18 * 0.01 + 0.22 * 0.001) / 0.4f64).log10();
        let m = momentary(&buf, rate, &[1.0, 1.0], 0.1);
        assert_eq!(m.len(), 97, "whole 400 ms windows every 100 ms of 10 s");
        assert!(m.iter().all(|&x| (x - want).abs() < 0.02), "want {want}, got {m:?}");
        let got = measure(&buf, rate, &[1.0, 1.0]);
        assert!((got.momentary_max - want).abs() < 0.02, "{got:?}");
    }

    #[test]
    fn short_term_reads_each_3_s() {
        // 1.34 s at −20 then 1.66 s at −30 dBFS, repeated: short-term
        // loudness is constant, momentary loudness reaches −20
        let rate = 48000.0;
        let segs: Vec<(f64, f64)> = (0..10).flat_map(|_| [(1.34, -20.0), (1.66, -30.0)]).collect();
        let buf = segments(rate, &segs);
        let want = 10.0 * ((1.34 * 0.01 + 1.66 * 0.001) / 3.0f64).log10();
        let st = short_term(&buf, rate, &[1.0, 1.0], 0.1);
        assert!(st.iter().all(|&x| (x - want).abs() < 0.02), "want {want}, got {st:?}");
        let got = measure(&buf, rate, &[1.0, 1.0]);
        assert!((got.short_term_max - want).abs() < 0.02, "{got:?}");
        assert!((got.momentary_max + 20.0).abs() < 0.02, "{got:?}");
        assert!(got.range < 0.05, "{got:?}");
        assert!((got.integrated - want).abs() < 0.1, "{got:?}");
    }

    #[test]
    fn measure_agrees_with_each_measurement() {
        for rate in [44100.0, 48000.0] {
            let buf = segments(rate, &[(5.0, -20.0), (6.0, -32.0), (4.0, -26.0)]);
            let w = [1.0, 1.0];
            let got = measure(&buf, rate, &w);
            assert_eq!(got.integrated.to_bits(), integrated(&buf, rate, &w).to_bits());
            assert_eq!(got.range.to_bits(), loudness_range(&buf, rate, &w).to_bits());
            assert_eq!(got.true_peak.to_bits(), true_peak(&buf, rate).to_bits());
            let mmax = momentary(&buf, rate, &w, 0.1).into_iter().fold(-200.0, f64::max);
            assert_eq!(got.momentary_max.to_bits(), mmax.to_bits(), "{rate}");
            let smax = short_term(&buf, rate, &w, 0.1).into_iter().fold(-200.0, f64::max);
            assert_eq!(got.short_term_max.to_bits(), smax.to_bits(), "{rate}");
            assert!((got.momentary_max + 20.0).abs() < 0.02, "{got:?}");
            assert!(got.range > 5.0, "{got:?}");
        }
    }

    #[test]
    fn the_momentary_maximum_is_the_loudest_momentary_window_at_any_rate() {
        // at 11025 Hz a quarter of the 400 ms block (1102 samples) is not the rounded 100 ms (1103)
        for rate in [8000.0, 11025.0, 22050.0, 32000.0, 44100.0, 48000.0, 88200.0] {
            let buf = segments(rate, &[(2.0, -30.0), (1.5, -18.0), (2.5, -24.0)]);
            let w = [1.0, 1.0];
            let m = momentary(&buf, rate, &w, 0.1);
            let blocks: Vec<f64> = gating_blocks(&buf, Extent::of(&buf), rate, &w).into_iter().map(lufs).collect();
            let bits = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
            assert_eq!(bits(&blocks), bits(&m), "the gating blocks are the momentary windows at {rate}");
            let mmax = m.into_iter().fold(-200.0, f64::max);
            assert_eq!(measure(&buf, rate, &w).momentary_max.to_bits(), mmax.to_bits(), "{rate}");
        }
    }

    #[test]
    fn short_and_silent_programmes() {
        let rate = 48000.0;
        let w = [1.0, 1.0];
        // under 3 s: no short-term window, so no range and no short-term maximum
        let short = segments(rate, &[(2.0, -20.0)]);
        let got = measure(&short, rate, &w);
        assert_eq!(got.range, 0.0);
        assert_eq!(got.short_term_max, -200.0);
        assert!((got.momentary_max + 20.0).abs() < 0.02, "{got:?}");
        assert!(short_term(&short, rate, &w, 0.1).is_empty());
        // under 400 ms: nothing but the peak
        let blip = segments(rate, &[(0.3, -20.0)]);
        let got = measure(&blip, rate, &w);
        assert_eq!((got.integrated, got.range, got.momentary_max, got.short_term_max), (-200.0, 0.0, -200.0, -200.0));
        assert!(momentary(&blip, rate, &w, 0.1).is_empty());
        // silence, and a tone under the −70 LUFS absolute gate
        for buf in [vec![vec![0f32; 480000]; 2], segments(rate, &[(10.0, -75.0)])] {
            let got = measure(&buf, rate, &w);
            assert_eq!((got.integrated, got.range), (-200.0, 0.0), "{got:?}");
        }
        let got = measure(&vec![vec![0f32; 480000]; 2], rate, &w);
        assert_eq!((got.momentary_max, got.short_term_max), (-200.0, -200.0));
        // no samples at all
        let got = measure(&vec![Vec::new(), Vec::new()], rate, &w);
        assert_eq!((got.integrated, got.range, got.momentary_max, got.short_term_max), (-200.0, 0.0, -200.0, -200.0));
    }

    /// A burst of loud tone inside a long silence, with negative zeros
    /// sprinkled in the silence and an odd channel weight.
    fn burst() -> (Planar, Extent) {
        let rate = 48000.0;
        let n = (12.0 * rate) as usize;
        let mut l = vec![0f32; n];
        let mut r = vec![0f32; n];
        let (a, b) = ((4.3 * rate) as usize, (6.7 * rate) as usize);
        for i in a..b {
            let t = i as f64 / rate;
            l[i] = (0.9 * (2.0 * std::f64::consts::PI * 997.0 * t).sin() * (1.0 + 0.3 * (7.0 * t).sin())) as f32;
            r[i] = (0.95 * (2.0 * std::f64::consts::PI * 12000.0 * t + 0.7).sin()) as f32;
        }
        l[a - 100] = -0.0;
        r[b + 7] = -0.0;
        (vec![l, r], Extent { start: a, end: b })
    }

    #[test]
    fn extent_scans_bit_patterns() {
        let (buf, ext) = burst();
        assert_eq!(Extent::of(&buf), ext, "negative zeros are zeros");
        assert_eq!(
            Extent::within(&buf, Extent { start: ext.start + 1000, end: ext.end - 1000 }),
            Extent { start: ext.start + 1000, end: ext.end - 1000 }
        );
        assert_eq!(Extent::of(&vec![vec![0f32; 10], vec![0f32; 10]]), Extent::EMPTY);
        assert_eq!(Extent::EMPTY.union(Extent { start: 3, end: 5 }), Extent { start: 3, end: 5 });
        assert_eq!(Extent { start: 3, end: 5 }.union(Extent { start: 9, end: 9 }), Extent { start: 3, end: 5 });
        assert_eq!(Extent { start: 3, end: 5 }.widen(10, 10, 8), Extent { start: 0, end: 8 });
        assert!(Extent { start: 5, end: 5 }.widen(10, 10, 8).is_empty());
    }

    #[test]
    fn extent_limited_loudness_is_bit_identical() {
        let (buf, ext) = burst();
        let rate = 48000.0;
        for weights in [&[1.0, 1.0][..], &[1.0, 0.0][..], &[1.41, 1.0][..]] {
            let full = Extent::full(buf[0].len());
            let want = integrated_in(&buf, full, rate, weights);
            assert_eq!(integrated_in(&buf, ext, rate, weights).to_bits(), want.to_bits());
            assert_eq!(integrated(&buf, rate, weights).to_bits(), want.to_bits());
            let (a, b) = (short_term_in(&buf, ext, rate, weights, 0.1), short_term_in(&buf, full, rate, weights, 0.1));
            assert_eq!(a.len(), b.len());
            assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()));
            let (a, b) = (momentary_in(&buf, ext, rate, weights, 0.1), momentary_in(&buf, full, rate, weights, 0.1));
            assert_eq!(a.len(), b.len());
            assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()));
            let want = loudness_range_in(&buf, full, rate, weights);
            assert_eq!(loudness_range_in(&buf, ext, rate, weights).to_bits(), want.to_bits());
            let bits = |m: Measurement| {
                [m.integrated, m.range, m.momentary_max, m.short_term_max, m.true_peak].map(f64::to_bits)
            };
            let want = bits(measure_in(&buf, full, rate, weights));
            assert_eq!(bits(measure_in(&buf, ext, rate, weights)), want);
            assert_eq!(bits(measure(&buf, rate, weights)), want);
            let e = block_energies(&buf, ext, rate, weights, 19200, 4800);
            assert!(e
                .iter()
                .zip(block_energies(&buf, full, rate, weights, 19200, 4800))
                .all(|(x, y)| x.to_bits() == y.to_bits()));
        }
        let full = Extent::full(buf[0].len());
        assert_eq!(true_peak_in(&buf, ext, rate).to_bits(), true_peak_in(&buf, full, rate).to_bits());
        assert_eq!(true_peak(&buf, rate).to_bits(), true_peak_in(&buf, full, rate).to_bits());
        assert!(true_peak(&buf, rate) > -1.0);
        let mut a = buf.clone();
        let mut b = buf.clone();
        limit_in(&mut a, ext, rate, -3.0, 0.05);
        limit_in(&mut b, full, rate, -3.0, 0.05);
        let mut c = buf.clone();
        limit(&mut c, rate, -3.0, 0.05);
        assert!(c.iter().flatten().zip(b.iter().flatten()).all(|(x, y)| x.to_bits() == y.to_bits()));
        assert!(a.iter().flatten().zip(b.iter().flatten()).all(|(x, y)| x.to_bits() == y.to_bits()));
        assert!(true_peak(&a, rate) < -2.9);
        // a silent programme: nothing to do
        let silent = vec![vec![0f32; 48000]; 2];
        assert_eq!(integrated_in(&silent, Extent::EMPTY, rate, &[1.0, 1.0]), -200.0);
        assert_eq!(true_peak_in(&silent, Extent::EMPTY, rate), lin_to_db(0.0));
    }
}
