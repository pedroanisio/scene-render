//! Counter-based randomness and smooth noise.

/// A 64-bit hash of a seed and counters (SplitMix64 finaliser).
pub fn hash(seed: u64, a: u64, b: u64) -> u64 {
    let mut x = seed ^ a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Uniform in [0, 1).
pub fn unit(seed: u64, a: u64, b: u64) -> f64 {
    (hash(seed, a, b) >> 11) as f64 / (1u64 << 53) as f64
}

/// Uniform in [−1, 1).
pub fn signed(seed: u64, a: u64, b: u64) -> f64 {
    unit(seed, a, b) * 2.0 - 1.0
}

/// Smooth 2D value noise in [−1, 1].
pub fn noise2(seed: u64, x: f64, y: f64) -> f64 {
    value_noise2(|i, j| signed(seed, i as u64, j as u64), x, y)
}

/// Smoothstep interpolation of the lattice values `lattice(i, j)` around (x, y).
fn value_noise2(lattice: impl Fn(i64, i64) -> f64, x: f64, y: f64) -> f64 {
    let (xi, yi) = (x.floor(), y.floor());
    let (fx, fy) = (x - xi, y - yi);
    let (u, v) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (x0, y0) = (xi as i64, yi as i64);
    let a = lattice(x0, y0) + (lattice(x0 + 1, y0) - lattice(x0, y0)) * u;
    let b = lattice(x0, y0 + 1) + (lattice(x0 + 1, y0 + 1) - lattice(x0, y0 + 1)) * u;
    a + (b - a) * v
}

/// Divergence-free 2D flow (the curl of value noise), roughly unit magnitude.
pub fn curl2(seed: u64, x: f64, y: f64) -> [f64; 2] {
    curl_of(|x, y| noise2(seed, x, y), x, y)
}

fn curl_of(noise: impl Fn(f64, f64) -> f64, x: f64, y: f64) -> [f64; 2] {
    let e = 0.01;
    let dx = (noise(x + e, y) - noise(x - e, y)) / (2.0 * e);
    let dy = (noise(x, y + e) - noise(x, y - e)) / (2.0 * e);
    [dy * 0.5, -dx * 0.5]
}

/// Where a simulation's draws come from. `Legacy` is the mixer above with a simulation seed, as documents of
/// version 1.0 to 1.7 draw (Appendix A of the random-streams SREP); `Stream` is an element seed E(…, "stream") whose
/// draws are the D24 streams S(e, p, t, i) that version 1.8 reads (its section 5). A draw names a purpose `p`, the
/// number the simulation gives the quantity it draws; a legacy draw reads `p` as its second counter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Draws {
    Legacy(u64),
    Stream(u64),
}

impl Draws {
    /// A draw read once per element, for element `k`: unit(seed, k, p), or S(e, p, 0, k).
    #[inline]
    pub fn unit(self, p: u32, k: u64) -> f64 {
        match self {
            Draws::Legacy(seed) => unit(seed, k, p as u64),
            Draws::Stream(e) => sr_vector::d24::stream(e, p, 0, k),
        }
    }

    /// [`Draws::unit`] in [−1, 1).
    #[inline]
    pub fn signed(self, p: u32, k: u64) -> f64 {
        self.unit(p, k) * 2.0 - 1.0
    }

    /// A draw read once per step: S(e, p, step, k), or what `legacy` draws from the legacy seed, whose
    /// simulations each addressed their step draws their own way.
    #[inline]
    pub fn at_step(self, p: u32, step: u32, k: u64, legacy: impl FnOnce(u64) -> f64) -> f64 {
        match self {
            Draws::Legacy(seed) => legacy(seed),
            Draws::Stream(e) => sr_vector::d24::stream(e, p, step, k),
        }
    }

    /// The legacy seed XORed with `salt`, as a legacy simulation separated two sets of draws; a stream separates
    /// them by purpose and is unchanged.
    #[inline]
    pub fn salted(self, salt: u64) -> Self {
        match self {
            Draws::Legacy(seed) => Draws::Legacy(seed ^ salt),
            stream => stream,
        }
    }

    /// Smooth 2D value noise in [−1, 1]: [`noise2`] of the legacy seed, or the same interpolation over the lattice
    /// draws S_signed(e, p, 0, pack(i, j, 0)).
    pub fn noise2(self, p: u32, x: f64, y: f64) -> f64 {
        match self {
            Draws::Legacy(seed) => noise2(seed, x, y),
            Draws::Stream(e) => {
                value_noise2(|i, j| sr_vector::d24::stream_signed(e, p, 0, sr_vector::d24::pack(i, j, 0)), x, y)
            }
        }
    }

    /// The curl of [`Draws::noise2`], as [`curl2`].
    pub fn curl2(self, p: u32, x: f64, y: f64) -> [f64; 2] {
        curl_of(|x, y| self.noise2(p, x, y), x, y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sr_vector::d24;

    #[test]
    fn legacy_draws_are_the_mixer_of_documents_before_1_8() {
        let d = Draws::Legacy(77);
        for k in [0u64, 1, 5, 1 << 40] {
            for p in [1u32, 2, 11, 12, 37] {
                assert_eq!(d.unit(p, k).to_bits(), unit(77, k, p as u64).to_bits());
                assert_eq!(d.signed(p, k).to_bits(), signed(77, k, p as u64).to_bits());
            }
        }
        assert_eq!(d.salted(0xD20F).unit(1, 3).to_bits(), unit(77 ^ 0xD20F, 3, 1).to_bits());
        assert_eq!(d.at_step(4, 9, 3, |s| unit(s ^ 0x51A1, 3, 9)).to_bits(), unit(77 ^ 0x51A1, 3, 9).to_bits());
        assert_eq!(d.noise2(20, 0.3, -4.7).to_bits(), noise2(77, 0.3, -4.7).to_bits());
        assert_eq!(d.curl2(20, 1.3, 2.9).map(f64::to_bits), curl2(77, 1.3, 2.9).map(f64::to_bits));
    }

    #[test]
    fn stream_draws_are_the_d24_streams_at_purpose_step_and_index() {
        let e = d24::element_seed(0, "p1", None, "stream");
        let d = Draws::Stream(e);
        assert_eq!(d.unit(11, 4).to_bits(), d24::stream(e, 11, 0, 4).to_bits());
        assert_eq!(d.signed(2, 0).to_bits(), d24::stream_signed(e, 2, 0, 0).to_bits());
        // a salt is a legacy seed's: a stream separates its draws by purpose
        assert_eq!(d.salted(0xD20F), d);
        assert_eq!(d.at_step(11, 3, 4, |_| unreachable!()).to_bits(), d24::stream(e, 11, 3, 4).to_bits());
        // value noise over the stream's lattice draws at pack(i, j, 0): exact at lattice points
        assert_eq!(d.noise2(20, 3.0, -2.0).to_bits(), d24::stream_signed(e, 20, 0, d24::pack(3, -2, 0)).to_bits());
        for k in 0..2000 {
            let (x, y) = (k as f64 * 0.37 - 300.0, k as f64 * 0.11 + 7.0);
            let n = d.noise2(20, x, y);
            assert!((-1.0..=1.0).contains(&n), "{n}");
            assert_ne!(n.to_bits(), noise2(e, x, y).to_bits());
        }
    }
}
