//! The sr-core scale draft (`srep/srep-0000-scale.md`, SP5 of SREP 12): depth mappings and the gap a depth buffer
//! resolves, geometric key interpolation and the powers of ten camera, the scale bar's unit ladder and label, and
//! the binary32 error a floating origin avoids, each held to the draft's reference module
//! (`conformance/tools/scale_reference.py`).
//!
//! Distances are scene pixels unless a function says metres.

// The domain checks are negated comparisons so that NaN is refused with the out-of-range values, as the reference's
// `not x > 0` refuses it.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use std::f64::consts::PI;

/// The largest `far / near` SC1 allows.
pub const MAX_DEPTH_RATIO: f64 = 1e20;
/// The relative gap a camera must resolve at every decade (rule 1) and the world error that calls for a rebase
/// (rule 4).
pub const RESOLUTION: f64 = 1e-5;
/// The astronomical unit (IAU 2012), metres.
pub const AU_M: f64 = 149_597_870_700.0;
/// The light year (a Julian year of light), metres.
pub const LY_M: f64 = 9_460_730_472_580_800.0;
/// The parsec, 648 000 / π au, metres.
pub const PC_M: f64 = 648_000.0 / PI * AU_M;
/// The shortest bar, metres.
pub const MIN_BAR_M: f64 = 1e-12;

const F32_MANTISSA: i32 = 23;
const FIXED_BITS: i32 = 24;
const STEPS: [f64; 3] = [1.0, 2.0, 5.0];

/// A value outside the draft's domain.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{0}")]
pub struct ScaleError(pub String);

fn err<T>(message: &str) -> Result<T, ScaleError> {
    Err(ScaleError(message.into()))
}

/// `camera/@depth`: rule 1's mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DepthMode {
    /// d = n (f − z) / ((f − n) z), 1 at near and 0 at far.
    #[default]
    Reversed,
    /// d = ln(z / n) / ln(f / n), 0 at near and 1 at far.
    Logarithmic,
}

impl DepthMode {
    pub fn parse(s: &str) -> Result<DepthMode, ScaleError> {
        match s {
            "reversed" => Ok(DepthMode::Reversed),
            "logarithmic" => Ok(DepthMode::Logarithmic),
            _ => Err(ScaleError(format!("unknown depth mode {s:?}"))),
        }
    }
}

/// A depth buffer's storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Buffer {
    /// binary32.
    Float,
    /// 24-bit fixed point.
    Fixed,
}

/// A key pair's interpolation (rule 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interpolation {
    Linear,
    Geometric,
}

impl Interpolation {
    pub fn parse(s: &str) -> Result<Interpolation, ScaleError> {
        match s {
            "linear" => Ok(Interpolation::Linear),
            "geometric" => Ok(Interpolation::Geometric),
            _ => Err(ScaleError(format!("unknown interpolation {s:?}"))),
        }
    }
}

/// `scaleBar/@units`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Units {
    /// The ladder m, au, ly, pc.
    #[default]
    Auto,
    /// Metres only.
    Si,
}

impl Units {
    pub fn parse(s: &str) -> Result<Units, ScaleError> {
        match s {
            "auto" => Ok(Units::Auto),
            "si" => Ok(Units::Si),
            _ => Err(ScaleError(format!("unknown units {s:?}"))),
        }
    }
}

/// A unit of the bar's ladder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    M,
    Au,
    Ly,
    Pc,
}

impl Unit {
    const LADDER: [Unit; 4] = [Unit::M, Unit::Au, Unit::Ly, Unit::Pc];

    pub fn name(self) -> &'static str {
        match self {
            Unit::M => "m",
            Unit::Au => "au",
            Unit::Ly => "ly",
            Unit::Pc => "pc",
        }
    }

    pub fn metres(self) -> f64 {
        match self {
            Unit::M => 1.0,
            Unit::Au => AU_M,
            Unit::Ly => LY_M,
            Unit::Pc => PC_M,
        }
    }

    /// The SI prefix exponents the label may use, for metres and parsecs.
    fn prefixes(self) -> Option<(i32, i32)> {
        match self {
            Unit::M => Some((-12, 9)),
            Unit::Pc => Some((0, 9)),
            Unit::Au | Unit::Ly => None,
        }
    }
}

// ------------------------------------------------------------------------------------ binary32

fn f32_of(x: f64) -> Result<f64, ScaleError> {
    let r = x as f32;
    if x.is_finite() && !r.is_finite() {
        return Err(ScaleError(format!("{x} overflows binary32")));
    }
    Ok(r as f64)
}

/// The spacing of binary32 values at |x|: 2^(e − 23) for 2^e ≤ |x| < 2^(e + 1), the smallest subnormal at 0.
pub fn f32_ulp(x: f64) -> f64 {
    let a = x.abs();
    if a == 0.0 {
        return 2f64.powi(-149);
    }
    let mut e = a.log2().floor() as i32;
    if 2f64.powi(e + 1) <= a {
        e += 1;
    } else if 2f64.powi(e) > a {
        e -= 1;
    }
    2f64.powi((e - F32_MANTISSA).max(-149))
}

// ------------------------------------------------------------------------------------ rule 1: depth

fn planes(near: f64, far: f64) -> Result<(), ScaleError> {
    if !(near > 0.0) {
        return err("near must be positive");
    }
    if !(far > near) {
        return err("far must exceed near");
    }
    Ok(())
}

fn between(z: f64, near: f64, far: f64) -> Result<(), ScaleError> {
    planes(near, far)?;
    if !(near <= z && z <= far) {
        return err("z must lie between near and far");
    }
    Ok(())
}

/// The depth buffer value of a point at distance `z`.
pub fn depth(z: f64, near: f64, far: f64, mode: DepthMode) -> Result<f64, ScaleError> {
    between(z, near, far)?;
    Ok(match mode {
        DepthMode::Reversed => near * (far - z) / ((far - near) * z),
        DepthMode::Logarithmic => (z / near).ln() / (far / near).ln(),
    })
}

/// The distance whose depth is `d`, the inverse of [`depth`].
pub fn distance(d: f64, near: f64, far: f64, mode: DepthMode) -> Result<f64, ScaleError> {
    planes(near, far)?;
    if !(0.0..=1.0).contains(&d) {
        return err("depth must lie in [0, 1]");
    }
    Ok(match mode {
        DepthMode::Reversed => near * far / (d * (far - near) + near),
        DepthMode::Logarithmic => near * (far / near).powf(d),
    })
}

/// |dd/dz| at distance `z`.
pub fn depth_slope(z: f64, near: f64, far: f64, mode: DepthMode) -> Result<f64, ScaleError> {
    between(z, near, far)?;
    Ok(match mode {
        DepthMode::Reversed => near * far / ((far - near) * z * z),
        DepthMode::Logarithmic => 1.0 / (z * (far / near).ln()),
    })
}

/// The smallest change a depth buffer stores at value `d`.
pub fn depth_ulp(d: f64, kind: Buffer) -> f64 {
    match kind {
        Buffer::Float => f32_ulp(d),
        Buffer::Fixed => 2f64.powi(-FIXED_BITS),
    }
}

/// The smallest distance gap at `z` that maps to distinct depth values.
pub fn resolvable_gap(z: f64, near: f64, far: f64, mode: DepthMode, kind: Buffer) -> Result<f64, ScaleError> {
    Ok(depth_ulp(depth(z, near, far, mode)?, kind) / depth_slope(z, near, far, mode)?)
}

/// [`resolvable_gap`] over `z`.
pub fn relative_resolution(z: f64, near: f64, far: f64, mode: DepthMode, kind: Buffer) -> Result<f64, ScaleError> {
    Ok(resolvable_gap(z, near, far, mode, kind)? / z)
}

/// The largest relative resolution over the decades near × 10^k up to far, and just short of far.
pub fn worst_resolution(near: f64, far: f64, mode: DepthMode, kind: Buffer) -> Result<f64, ScaleError> {
    planes(near, far)?;
    let mut worst = 0f64;
    let mut k = 0;
    loop {
        let z = near * 10f64.powf(k as f64);
        if z >= far {
            break;
        }
        worst = worst.max(relative_resolution(z, near, far, mode, kind)?);
        k += 1;
    }
    Ok(worst.max(relative_resolution(far * (1.0 - 1e-9), near, far, mode, kind)?))
}

/// True when the buffer resolves [`RESOLUTION`] at every sampled distance.
pub fn resolves(near: f64, far: f64, mode: DepthMode, kind: Buffer) -> Result<bool, ScaleError> {
    Ok(worst_resolution(near, far, mode, kind)? <= RESOLUTION)
}

/// SC1: 0 < near < far and far / near ≤ 1e20.
pub fn depth_ratio_valid(near: f64, far: f64) -> bool {
    near > 0.0 && far > near && far / near <= MAX_DEPTH_RATIO
}

/// SC1 (rule 5).
pub fn validate_camera(near: f64, far: f64) -> Result<(), ScaleError> {
    if !depth_ratio_valid(near, far) {
        return err("SC1: far / near must not exceed 1e20");
    }
    Ok(())
}

// ------------------------------------------------------------------------------------ rule 2: geometric keys

/// SC2: a geometric key pair is non-zero and of one sign.
pub fn validate_keys(v0: f64, v1: f64, mode: Interpolation) -> Result<(), ScaleError> {
    if mode == Interpolation::Geometric && (v0 == 0.0 || v1 == 0.0 || (v0 < 0.0) != (v1 < 0.0)) {
        return err("SC2: geometric keys must be non-zero and of one sign");
    }
    Ok(())
}

/// v0 (v1 / v0)^u for u in [0, 1].
pub fn geometric(v0: f64, v1: f64, u: f64) -> Result<f64, ScaleError> {
    validate_keys(v0, v1, Interpolation::Geometric)?;
    if !(0.0..=1.0).contains(&u) {
        return err("u must lie in [0, 1]");
    }
    Ok(if u == 0.0 {
        v0
    } else if u == 1.0 {
        v1
    } else {
        v0 * (v1 / v0).powf(u)
    })
}

/// A key pair interpolated linearly or geometrically.
pub fn interpolate(v0: f64, v1: f64, u: f64, mode: Interpolation) -> Result<f64, ScaleError> {
    match mode {
        Interpolation::Linear => Ok(v0 + (v1 - v0) * u),
        Interpolation::Geometric => geometric(v0, v1, u),
    }
}

/// ⌊log10 v⌋.
pub fn decade(v: f64) -> Result<i32, ScaleError> {
    if !(v > 0.0) {
        return err("decade needs a positive value");
    }
    let mut k = v.log10().floor() as i32;
    if 10f64.powf((k + 1) as f64) <= v {
        k += 1;
    } else if 10f64.powf(k as f64) > v {
        k -= 1;
    }
    Ok(k)
}

/// log10(v1 / v0).
pub fn decades(v0: f64, v1: f64) -> Result<f64, ScaleError> {
    if !(v0 > 0.0 && v1 > 0.0) {
        return err("decades need positive values");
    }
    Ok((v1 / v0).log10())
}

/// The powers of ten camera's distance at `t`: geometric from d0 at t0 to d1 at t1, held outside (a NaN `t` is
/// refused by [`geometric`]).
pub fn powers_of_ten(t: f64, t0: f64, t1: f64, d0: f64, d1: f64) -> Result<f64, ScaleError> {
    if !(t1 > t0) {
        return err("t1 must exceed t0");
    }
    geometric(d0, d1, ((t - t0) / (t1 - t0)).clamp(0.0, 1.0))
}

/// The time at which the powers of ten camera is at distance `d`.
pub fn time_at(d: f64, t0: f64, t1: f64, d0: f64, d1: f64) -> Result<f64, ScaleError> {
    if !(t1 > t0) {
        return err("t1 must exceed t0");
    }
    validate_keys(d0, d1, Interpolation::Geometric)?;
    let u = (d / d0).ln() / (d1 / d0).ln();
    if !(0.0..=1.0 + 1e-12).contains(&u) {
        return err("d lies outside [d0, d1]");
    }
    Ok(t0 + (t1 - t0) * u.min(1.0))
}

/// [`powers_of_ten`] at frame / fps: a pure function of the frame (the seek check).
pub fn frame_distance(frame: u64, fps: f64, t0: f64, t1: f64, d0: f64, d1: f64) -> Result<f64, ScaleError> {
    if !(fps > 0.0) {
        return err("fps must be positive");
    }
    powers_of_ten(frame as f64 / fps, t0, t1, d0, d1)
}

// ------------------------------------------------------------------------------------ rule 3: the scale bar

/// The focal length in pixels of a horizontal field of view: (width / 2) / tan(fov / 2).
pub fn focal_pixels(fov_deg: f64, width_px: f64) -> Result<f64, ScaleError> {
    if !(fov_deg > 0.0 && fov_deg < 180.0) {
        return err("fov must lie in (0, 180) degrees");
    }
    if !(width_px > 0.0) {
        return err("width must be positive");
    }
    Ok(width_px / 2.0 / (fov_deg.to_radians() / 2.0).tan())
}

fn ppm_positive(ppm: f64) -> Result<(), ScaleError> {
    if !(ppm > 0.0) {
        return err("pixelsPerMeter must be positive");
    }
    Ok(())
}

/// Screen pixels per metre at a depth of `depth_px` scene pixels: pixelsPerMeter × focal / depth.
pub fn pixels_per_metre(depth_px: f64, focal_px: f64, ppm: f64) -> Result<f64, ScaleError> {
    if !(depth_px > 0.0) {
        return err("depth must be positive");
    }
    if !(focal_px > 0.0) {
        return err("focal length must be positive");
    }
    ppm_positive(ppm)?;
    Ok(ppm * focal_px / depth_px)
}

/// Screen pixels per metre of an orthographic camera: pixelsPerMeter × viewport height / orthoHeight.
pub fn pixels_per_metre_ortho(ortho_height_px: f64, viewport_height_px: f64, ppm: f64) -> Result<f64, ScaleError> {
    if !(ortho_height_px > 0.0) {
        return err("orthoHeight must be positive");
    }
    if !(viewport_height_px > 0.0) {
        return err("viewport height must be positive");
    }
    ppm_positive(ppm)?;
    Ok(ppm * viewport_height_px / ortho_height_px)
}

/// The largest 1, 2 or 5 × 10^k not above `limit`.
pub fn nice_length(limit: f64) -> Result<f64, ScaleError> {
    if !(limit > 0.0) {
        return err("the limit must be positive");
    }
    let base = 10f64.powf(decade(limit)? as f64);
    Ok(STEPS.iter().map(|s| s * base).filter(|c| *c <= limit * (1.0 + 1e-12)).fold(base, f64::max))
}

/// The largest unit of the ladder not above `max_metres`; metres under [`Units::Si`].
pub fn bar_unit(max_metres: f64, units: Units) -> Unit {
    match units {
        Units::Si => Unit::M,
        Units::Auto => Unit::LADDER.into_iter().rev().find(|u| u.metres() <= max_metres).unwrap_or(Unit::M),
    }
}

/// Python's `%.6g`.
fn g6(v: f64) -> String {
    let sci = format!("{v:.5e}");
    let (mantissa, exp) = sci.split_once('e').expect("a formatted exponent");
    let x: i32 = exp.parse().expect("a formatted exponent");
    let trim = |s: String| if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
    if (-4..6).contains(&x) {
        trim(format!("{v:.*}", (5 - x) as usize))
    } else {
        format!("{}e{}{:02}", trim(mantissa.to_string()), if x < 0 { '-' } else { '+' }, x.abs())
    }
}

fn value_text(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        g6(v)
    }
}

/// The bar's label: SI prefixes from p to G on metres and parsecs, plain values on au and ly.
pub fn scale_label(value: f64, unit: Unit) -> Result<String, ScaleError> {
    let Some((lo, hi)) = unit.prefixes() else {
        return Ok(format!("{} {}", value_text(value), unit.name()));
    };
    let e = (3 * decade(value)?.div_euclid(3)).clamp(lo, hi);
    let prefix = match e {
        -12 => "p",
        -9 => "n",
        -6 => "µ",
        -3 => "m",
        0 => "",
        3 => "k",
        6 => "M",
        _ => "G",
    };
    Ok(format!("{} {prefix}{}", value_text(value / 10f64.powf(e as f64)), unit.name()))
}

/// A scale bar: its length in metres and screen pixels, its unit, its value in that unit and its label.
#[derive(Debug, Clone, PartialEq)]
pub struct ScaleBar {
    pub metres: f64,
    pub pixels: f64,
    pub unit: Unit,
    pub value: f64,
    pub label: String,
}

/// The largest 1, 2 or 5 × 10^k bar of the chosen unit that fits `max_width` pixels.
pub fn scale_bar(px_per_m: f64, max_width: u32, units: Units) -> Result<ScaleBar, ScaleError> {
    if !(px_per_m > 0.0) {
        return err("pixels per metre must be positive");
    }
    if max_width == 0 {
        return err("maxWidth must be positive");
    }
    let max_metres = max_width as f64 / px_per_m;
    if max_metres < MIN_BAR_M {
        return err("no scale bar of one picometre or more fits");
    }
    let unit = bar_unit(max_metres, units);
    let value = nice_length(max_metres / unit.metres())?;
    let metres = value * unit.metres();
    Ok(ScaleBar { metres, pixels: metres * px_per_m, unit, value, label: scale_label(value, unit)? })
}

/// [`scale_bar`] at a perspective depth.
pub fn scale_bar_at(
    depth_px: f64,
    focal_px: f64,
    ppm: f64,
    max_width: u32,
    units: Units,
) -> Result<ScaleBar, ScaleError> {
    scale_bar(pixels_per_metre(depth_px, focal_px, ppm)?, max_width, units)
}

/// The scale bar of a powers of ten camera at a frame.
#[allow(clippy::too_many_arguments)]
pub fn scale_bar_frame(
    frame: u64,
    fps: f64,
    t0: f64,
    t1: f64,
    d0: f64,
    d1: f64,
    focal_px: f64,
    ppm: f64,
    max_width: u32,
    units: Units,
) -> Result<ScaleBar, ScaleError> {
    scale_bar_at(frame_distance(frame, fps, t0, t1, d0, d1)?, focal_px, ppm, max_width, units)
}

// ------------------------------------------------------------------------------------ rule 4: the floating origin

/// The error of p − eye computed in binary32 world coordinates.
pub fn world_error(p: f64, eye: f64) -> Result<f64, ScaleError> {
    Ok((f32_of(f32_of(p)? - f32_of(eye)?)? - (p - eye)).abs())
}

/// The error of p − eye computed in binary64 and then rounded to binary32.
pub fn rebased_error(p: f64, eye: f64) -> Result<f64, ScaleError> {
    let exact = p - eye;
    Ok((f32_of(exact)? - exact).abs())
}

fn to_f32(x: f64) -> Result<f32, ScaleError> {
    Ok(f32_of(x)? as f32)
}

/// Every position rebased about the eye in binary64 and rounded to binary32.
pub fn floating_origin(positions: &[[f64; 3]], eye: [f64; 3]) -> Result<Vec<[f32; 3]>, ScaleError> {
    positions.iter().map(|p| Ok([to_f32(p[0] - eye[0])?, to_f32(p[1] - eye[1])?, to_f32(p[2] - eye[2])?])).collect()
}

/// The same subtraction done in binary32 world coordinates, what the floating origin avoids.
pub fn world_positions(positions: &[[f64; 3]], eye: [f64; 3]) -> Result<Vec<[f32; 3]>, ScaleError> {
    let e = [f32_of(eye[0])?, f32_of(eye[1])?, f32_of(eye[2])?];
    positions
        .iter()
        .map(|p| Ok([to_f32(f32_of(p[0])? - e[0])?, to_f32(f32_of(p[1])? - e[1])?, to_f32(f32_of(p[2])? - e[2])?]))
        .collect()
}

/// True when the binary32 world error exceeds [`RESOLUTION`] of the gap.
pub fn rebase_needed(p: f64, eye: f64) -> Result<bool, ScaleError> {
    let gap = (p - eye).abs();
    if gap == 0.0 {
        return Ok(false);
    }
    Ok(world_error(p, eye)? > RESOLUTION * gap)
}

/// The magnitude below which binary32 world coordinates still resolve a gap: 2^(24 + ⌊log2 gap⌋).
pub fn world_extent(gap: f64) -> Result<f64, ScaleError> {
    if !(gap > 0.0) {
        return err("gap must be positive");
    }
    Ok(2f64.powi(F32_MANTISSA + 1 + gap.log2().floor() as i32))
}

// ------------------------------------------------------------------------------------ rule 5: validation

/// SC3: `maxWidth` is positive and not above the scene width; SC4: a given `distance` lies between near and far.
/// (`units` outside `auto` and `si` is refused by [`Units::parse`].)
pub fn validate_scale_bar(
    max_width: u32,
    distance_px: Option<f64>,
    near: f64,
    far: f64,
    scene_width: u32,
) -> Result<(), ScaleError> {
    if max_width == 0 || max_width > scene_width {
        return err("SC3: maxWidth must be a positive integer not above the scene width");
    }
    if let Some(d) = distance_px {
        if !(near <= d && d <= far) {
            return err("SC4: distance must lie between near and far");
        }
    }
    Ok(())
}
