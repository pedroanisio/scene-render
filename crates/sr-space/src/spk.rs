//! Rules 3 and 4: JPL SPK files in the DAF format (type 2 Chebyshev segments, either byte order) pinned by a sha256
//! digest, positions through the chain of centres to the solar system barycentre, velocities and light time.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use sha2::{Digest, Sha256};

use super::SpaceError;

/// The most segments, and summary records, a file may hold.
pub const MAX_SEGMENTS: usize = 4096;
/// The light-time iteration's bound.
pub const MAX_LIGHT_TIME_ITERATIONS: usize = 5;
/// The longest chain of centres from a body to the barycentre.
const MAX_CHAIN: usize = 16;
/// The speed of light in km/s.
pub const C_KM_S: f64 = 299792.458;
const RECORD: usize = 1024;

/// One segment's summary and its type 2 trailer.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub target: i32,
    pub center: i32,
    pub frame: i32,
    pub kind: i32,
    pub begin: i64,
    pub finish: i64,
    pub name: String,
    init: f64,
    intlen: f64,
    rsize: usize,
    n: usize,
}

/// An SPK file read into memory.
#[derive(Debug, Clone)]
pub struct Spk {
    pub id: String,
    pub format: String,
    pub nd: i32,
    pub ni: i32,
    pub segments: Vec<Segment>,
    centers: BTreeMap<i32, i32>,
    big: bool,
    digest: [u8; 32],
    data: Vec<u8>,
}

fn err(m: impl Into<String>) -> SpaceError {
    SpaceError::Ephemeris(m.into())
}

fn word<const N: usize>(data: &[u8], off: usize) -> Result<[u8; N], SpaceError> {
    data.get(off..off + N).and_then(|s| s.try_into().ok()).ok_or_else(|| err(format!("byte {off} is outside the file")))
}

impl Spk {
    /// Parse an SPK file's file record, summary records and segment trailers.
    pub fn parse(data: Vec<u8>) -> Result<Spk, SpaceError> {
        if data.len() < 2 * RECORD {
            return Err(err("the file is shorter than a DAF file record and a summary record"));
        }
        let id = String::from_utf8_lossy(&data[0..8]).into_owned();
        if id != "DAF/SPK " {
            return Err(err(format!("not an SPK file: {id:?}")));
        }
        let format = String::from_utf8_lossy(&data[88..96]).into_owned();
        let big = match format.as_str() {
            "LTL-IEEE" => false,
            "BIG-IEEE" => true,
            _ => return Err(err(format!("the DAF byte order is LTL-IEEE or BIG-IEEE, not {format:?}"))),
        };
        let mut spk = Spk {
            id: id.trim_end().to_string(),
            format,
            nd: 0,
            ni: 0,
            segments: Vec::new(),
            centers: BTreeMap::new(),
            big,
            digest: Sha256::digest(&data).into(),
            data,
        };
        spk.nd = spk.int(8)?;
        spk.ni = spk.int(12)?;
        if (spk.nd, spk.ni) != (2, 6) {
            return Err(err(format!("an SPK summary has 2 doubles and 6 integers, not {} and {}", spk.nd, spk.ni)));
        }
        let mut rec = spk.int(76)? as i64;
        let mut visited = BTreeSet::new();
        while rec > 0 {
            if !visited.insert(rec) || visited.len() > MAX_SEGMENTS {
                return Err(err("the summary records loop"));
            }
            let base = (rec as usize - 1) * RECORD;
            if base + 2 * RECORD > spk.data.len() {
                return Err(err(format!("summary record {rec} is beyond the file")));
            }
            let next = spk.double(base)?;
            let nsum = spk.double(base + 16)?;
            if !(0.0..=25.0).contains(&nsum) || nsum.fract() != 0.0 {
                return Err(err("a summary record holds 0 to 25 summaries"));
            }
            for i in 0..nsum as usize {
                let off = base + 24 + i * 40;
                let ints: Vec<i32> = (0..6).map(|k| spk.int(off + 16 + 4 * k)).collect::<Result<_, _>>()?;
                let (target, center, frame, kind) = (ints[0], ints[1], ints[2], ints[3]);
                let (begin, finish) = (ints[4] as i64, ints[5] as i64);
                if kind != 2 {
                    return Err(err(format!("segment type {kind} is not the Chebyshev position type 2")));
                }
                if frame != 1 {
                    return Err(err(format!("segment frame {frame} is not J2000/ICRF (1)")));
                }
                if begin < 1 || finish * 8 > spk.data.len() as i64 || finish - begin < 3 {
                    return Err(err("a segment's addresses are outside the file"));
                }
                if spk.segments.len() >= MAX_SEGMENTS {
                    return Err(err(format!("more than {MAX_SEGMENTS} segments")));
                }
                let name = String::from_utf8_lossy(&spk.data[base + RECORD + i * 40..base + RECORD + (i + 1) * 40])
                    .trim()
                    .to_string();
                let t: Vec<f64> =
                    (0..4).map(|k| spk.double((finish as usize - 4 + k) * 8)).collect::<Result<_, _>>()?;
                let (init, intlen, rsize, n) = (t[0], t[1], t[2], t[3]);
                let ok = intlen > 0.0
                    && intlen.is_finite()
                    && init.is_finite()
                    && rsize >= 5.0
                    && n >= 1.0
                    && rsize.fract() == 0.0
                    && n.fract() == 0.0
                    && (rsize as i64 - 2) % 3 == 0
                    && n * rsize + 4.0 <= (finish - begin + 1) as f64;
                if !ok {
                    return Err(err(format!("the trailer of body {target}'s segment is not a type 2 trailer")));
                }
                if *spk.centers.entry(target).or_insert(center) != center {
                    return Err(err(format!("body {target} has segments about two centres")));
                }
                spk.segments.push(Segment {
                    start: spk.double(off)?,
                    end: spk.double(off + 8)?,
                    target,
                    center,
                    frame,
                    kind,
                    begin,
                    finish,
                    name,
                    init,
                    intlen,
                    rsize: rsize as usize,
                    n: n as usize,
                });
            }
            rec = next as i64;
        }
        if spk.segments.is_empty() {
            return Err(err("the SPK file has no segments"));
        }
        Ok(spk)
    }

    /// Read an SPK file and check its sha256 digest (`sha256:` and 64 lowercase hex digits; None skips the check).
    pub fn open(path: &Path, digest: Option<&str>) -> Result<Spk, SpaceError> {
        let data = std::fs::read(path).map_err(|e| err(format!("cannot read {}: {e}", path.display())))?;
        let spk = Spk::parse(data)?;
        if let Some(d) = digest {
            let want = d.strip_prefix("sha256:").unwrap_or(d);
            if want.len() != 64 || !want.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
                return Err(err(format!("the digest is sha256: and 64 hex digits, not {d:?}")));
            }
            if want != spk.digest_hex() {
                return Err(err(format!("ephemeris digest mismatch: document {want}, file {}", spk.digest_hex())));
            }
        }
        Ok(spk)
    }

    /// The file's sha256 in lowercase hex.
    pub fn digest_hex(&self) -> String {
        self.digest.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Whether the file has segments of a body.
    pub fn has_body(&self, body: i32) -> bool {
        self.centers.contains_key(&body)
    }

    fn int(&self, off: usize) -> Result<i32, SpaceError> {
        let w = word::<4>(&self.data, off)?;
        Ok(if self.big { i32::from_be_bytes(w) } else { i32::from_le_bytes(w) })
    }

    fn double(&self, off: usize) -> Result<f64, SpaceError> {
        let w = word::<8>(&self.data, off)?;
        Ok(if self.big { f64::from_be_bytes(w) } else { f64::from_le_bytes(w) })
    }

    fn doubles(&self, address: i64, n: usize) -> Result<Vec<f64>, SpaceError> {
        if address < 1 {
            return Err(err(format!("address {address} is outside the file")));
        }
        (0..n).map(|k| self.double((address as usize - 1 + k) * 8)).collect()
    }

    /// Position (km) and velocity (km/s) of a body about its own centre from its segment covering et.
    fn segment_state(&self, target: i32, et: f64) -> Result<([f64; 3], [f64; 3]), SpaceError> {
        let Some(seg) = self.segments.iter().find(|s| s.target == target && s.start <= et && et <= s.end) else {
            return Err(if self.has_body(target) {
                err(format!("body {target} has no segment at ephemeris time {et}"))
            } else {
                err(format!("body {target} is not in the ephemeris"))
            });
        };
        let i = ((et - seg.init) / seg.intlen).floor().clamp(0.0, (seg.n - 1) as f64) as usize;
        let rec = self.doubles(seg.begin + (i * seg.rsize) as i64, seg.rsize)?;
        let (mid, radius) = (rec[0], rec[1]);
        if radius.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater)
            || !(mid - radius - 1e-6 <= et && et <= mid + radius + 1e-6)
        {
            return Err(err(format!("record {i} of body {target} does not cover ephemeris time {et}")));
        }
        let nc = (seg.rsize - 2) / 3;
        let t = ((et - mid) / radius).clamp(-1.0, 1.0);
        let (mut pos, mut vel) = ([0.0; 3], [0.0; 3]);
        for k in 0..3 {
            let c = &rec[2 + k * nc..2 + (k + 1) * nc];
            pos[k] = chebyshev(c, t)?;
            vel[k] = chebyshev_derivative(c, t)? / radius;
        }
        Ok((pos, vel))
    }

    /// State of a body relative to the solar system barycentre through the chain of centres.
    fn chain_state(&self, mut body: i32, et: f64) -> Result<([f64; 3], [f64; 3]), SpaceError> {
        let (mut pos, mut vel) = ([0.0; 3], [0.0; 3]);
        let mut hops = 0;
        while body != 0 {
            let Some(&center) = self.centers.get(&body) else {
                return Err(err(format!("body {body} is not in the ephemeris")));
            };
            let (p, v) = self.segment_state(body, et)?;
            for k in 0..3 {
                pos[k] += p[k];
                vel[k] += v[k];
            }
            body = center;
            hops += 1;
            if hops > MAX_CHAIN {
                return Err(err("the chain of centres does not reach the barycentre"));
            }
        }
        Ok((pos, vel))
    }

    /// Position (km) and velocity (km/s) of target relative to center at ephemeris time et, ICRF axes.
    pub fn state(&self, target: i32, center: i32, et: f64) -> Result<([f64; 3], [f64; 3]), SpaceError> {
        if target == center {
            if target != 0 && !self.has_body(target) {
                return Err(err(format!("body {target} is not in the ephemeris")));
            }
            return Ok(([0.0; 3], [0.0; 3]));
        }
        let (pt, vt) = self.chain_state(target, et)?;
        let (pc, vc) = self.chain_state(center, et)?;
        Ok(([pt[0] - pc[0], pt[1] - pc[1], pt[2] - pc[2]], [vt[0] - vc[0], vt[1] - vc[1], vt[2] - vc[2]]))
    }

    pub fn position(&self, target: i32, center: i32, et: f64) -> Result<[f64; 3], SpaceError> {
        Ok(self.state(target, center, et)?.0)
    }

    pub fn velocity(&self, target: i32, center: i32, et: f64) -> Result<[f64; 3], SpaceError> {
        Ok(self.state(target, center, et)?.1)
    }
}

/// The target's position when the light now arriving at the observer left it, and that light time in seconds.
pub fn apparent_position(spk: &Spk, target: i32, observer: i32, et: f64) -> Result<([f64; 3], f64), SpaceError> {
    let obs = spk.position(observer, 0, et)?;
    let (mut lt, mut p) = (0.0, [0.0; 3]);
    for _ in 0..MAX_LIGHT_TIME_ITERATIONS {
        let tgt = spk.position(target, 0, et - lt)?;
        p = [tgt[0] - obs[0], tgt[1] - obs[1], tgt[2] - obs[2]];
        lt = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt() / C_KM_S;
    }
    Ok((p, lt))
}

fn chebyshev_domain(t: f64) -> Result<(), SpaceError> {
    if !(-1.0 - 1e-12..=1.0 + 1e-12).contains(&t) {
        return Err(err(format!("the Chebyshev argument is in [-1, 1], not {t}")));
    }
    Ok(())
}

/// Σ cₖ Tₖ(t) by Clenshaw's recurrence for t in [−1, 1].
pub fn chebyshev(coeffs: &[f64], t: f64) -> Result<f64, SpaceError> {
    chebyshev_domain(t)?;
    let (first, rest) = coeffs.split_first().ok_or_else(|| err("a Chebyshev series has a coefficient"))?;
    let (mut b1, mut b2) = (0.0, 0.0);
    for c in rest.iter().rev() {
        (b1, b2) = (2.0 * t * b1 - b2 + c, b1);
    }
    Ok(t * b1 - b2 + first)
}

/// d/dt of the Chebyshev sum, by T′ₖ₊₁ = 2 Tₖ + 2 t T′ₖ − T′ₖ₋₁.
pub fn chebyshev_derivative(coeffs: &[f64], t: f64) -> Result<f64, SpaceError> {
    chebyshev_domain(t)?;
    let (mut t0, mut t1, mut d0, mut d1) = (1.0, t, 0.0, 1.0);
    let mut out = coeffs.get(1).map_or(0.0, |c| c * d1);
    for c in coeffs.iter().skip(2) {
        let t2 = 2.0 * t * t1 - t0;
        let d2 = 2.0 * t1 + 2.0 * t * d1 - d0;
        out += c * d2;
        (t0, t1, d0, d1) = (t1, t2, d1, d2);
    }
    Ok(out)
}
