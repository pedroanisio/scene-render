//! The atmosphere of a dome light with `sky="physical"`: rules 8, 9 and 11 of the sr-core draft "Add the physical
//! sun, moon and sky to dome lights" (`srep/srep-0000-physical-sky.md`), in the integrals of its reference module
//! (`conformance/tools/physical_sky_reference.py`). Hillaire's (2020) Earth on Bruneton and Neyret's model:
//! Rayleigh, Mie and ozone layers, the transmittance by Gauss–Legendre quadrature, single scattering with Hillaire's
//! isotropic multiple-scattering estimate, aerial perspective and a height fog in closed form.
//!
//! The planet is centred on the origin in metres, its up the radial direction; a ground observer stands on the +z
//! axis. The renderer's tables are held to these integrals.

use std::f64::consts::PI;

/// The moon's geometric albedo (rule 7).
pub const MOON_ALBEDO: f64 = 0.12;
/// The most quadrature segments a ray takes.
pub const MAX_SAMPLES: usize = 65536;
/// The heights of the multiple-scattering table.
pub const MS_HEIGHTS: usize = 8;
/// The sun cosines of the multiple-scattering table.
pub const MS_COSINES: usize = 8;
/// The directions each multiple-scattering cell integrates over.
pub const MS_DIRECTIONS: usize = 32;
/// The segments along each of those directions.
pub const MS_SAMPLES: usize = 6;

type V3 = [f64; 3];

/// A value of the physical-sky draft its rules refuse.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("sky: {0}")]
pub struct SkyError(pub String);

fn err<T>(message: &str) -> Result<T, SkyError> {
    Err(SkyError(message.into()))
}

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn mul(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}

/// The unit vector along `a`.
pub fn unit(a: V3) -> Result<V3, SkyError> {
    let n = norm(a);
    if n == 0.0 {
        return err("a zero direction has no unit");
    }
    Ok(mul(a, 1.0 / n))
}

fn march(p: V3, d: V3, length: f64) -> V3 {
    add(p, mul(d, length))
}

/// The document's knobs of rule 8; everything else is Hillaire's Earth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AtmosphereOptions {
    /// Metres (`planetRadius`).
    pub planet_radius: f64,
    /// Metres from the centre: `planetRadius` + `atmosphereHeight`.
    pub top_radius: f64,
    /// A multiplier on the Rayleigh layer: 0 under `atmosphere="none"`.
    pub rayleigh: f64,
    /// A multiplier on the Mie layer (`aerosol`).
    pub aerosol: f64,
    /// A multiplier on the ozone layer (`ozone`).
    pub ozone: f64,
    /// `groundAlbedo`, 0 to 1.
    pub ground_albedo: f64,
    /// The sun's irradiance at the top of the atmosphere (`sunIrradiance` times `sunIntensity`).
    pub sun_irradiance: V3,
}

impl Default for AtmosphereOptions {
    fn default() -> Self {
        AtmosphereOptions {
            planet_radius: 6360000.0,
            top_radius: 6460000.0,
            rayleigh: 1.0,
            aerosol: 1.0,
            ozone: 1.0,
            ground_albedo: 0.3,
            sun_irradiance: [1.0; 3],
        }
    }
}

/// Rule 8's atmosphere, coefficients per metre.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Atmosphere {
    pub planet_radius: f64,
    pub top_radius: f64,
    pub rayleigh_scattering: V3,
    pub rayleigh_scale: f64,
    pub mie_scattering: f64,
    pub mie_absorption: f64,
    pub mie_scale: f64,
    pub mie_anisotropy: f64,
    pub ozone_absorption: V3,
    pub ozone_centre: f64,
    pub ozone_width: f64,
    pub ground_albedo: f64,
    pub sun_irradiance: V3,
}

/// Rule 8: Hillaire's Earth with the document's knobs.
pub fn atmosphere(o: &AtmosphereOptions) -> Result<Atmosphere, SkyError> {
    if !(o.planet_radius > 0.0 && o.planet_radius.is_finite())
        || !(o.top_radius > o.planet_radius && o.top_radius.is_finite())
    {
        return err("the planet radius is positive and the atmosphere's top radius larger");
    }
    if !(o.rayleigh >= 0.0 && o.aerosol >= 0.0 && o.ozone >= 0.0)
        || !(o.rayleigh.is_finite() && o.aerosol.is_finite() && o.ozone.is_finite())
    {
        return err("aerosol, ozone and the scale heights are non-negative");
    }
    if !(0.0..=1.0).contains(&o.ground_albedo) {
        return err("the ground albedo is 0 to 1");
    }
    if !o.sun_irradiance.iter().all(|c| *c >= 0.0 && c.is_finite()) {
        return err("the sun's irradiance is finite and non-negative");
    }
    Ok(Atmosphere {
        planet_radius: o.planet_radius,
        top_radius: o.top_radius,
        rayleigh_scattering: [5.802e-6, 13.558e-6, 33.1e-6].map(|c| c * o.rayleigh),
        rayleigh_scale: 8000.0,
        mie_scattering: 3.996e-6 * o.aerosol,
        mie_absorption: 4.4e-6 * o.aerosol,
        mie_scale: 1200.0,
        mie_anisotropy: 0.8,
        ozone_absorption: [0.650e-6, 1.881e-6, 0.085e-6].map(|c| c * o.ozone),
        ozone_centre: 25000.0,
        ozone_width: 15000.0,
        ground_albedo: o.ground_albedo,
        sun_irradiance: o.sun_irradiance,
    })
}

/// The 4-point Gauss–Legendre nodes and weights on [−1, 1].
const GL: [(f64, f64); 4] = [
    (-0.8611363115940526, 0.3478548451374538),
    (-0.3399810435848563, 0.6521451548625461),
    (0.3399810435848563, 0.6521451548625461),
    (0.8611363115940526, 0.3478548451374538),
];

/// Segment ends along a path: uniform, or quadratically clustered toward the start.
fn segments(length: f64, samples: usize, cluster: bool) -> Vec<f64> {
    (0..=samples)
        .map(|i| {
            let f = i as f64 / samples as f64;
            if cluster {
                length * f.powi(2)
            } else {
                length * i as f64 / samples as f64
            }
        })
        .collect()
}

/// The nearest positive distance along d from p to the sphere about the origin of `radius`.
fn sphere_hit(p: V3, d: V3, radius: f64) -> Option<f64> {
    let b = dot(p, d);
    let c = dot(p, p) - radius * radius;
    let disc = b * b - c;
    if disc < 0.0 {
        return None;
    }
    let s = disc.sqrt();
    let (t0, t1) = (-b - s, -b + s);
    if t1 < 0.0 {
        return None;
    }
    Some(if t0 >= 0.0 { t0 } else { t1 })
}

/// The sun's irradiance a physical sky is lit by: `sunIrradiance` times `sunIntensity`, which reads as 1 when the
/// document leaves it out (SREP 71's default 0 is the gradient sky's sunless decoration).
pub fn effective_sun_irradiance(sun_irradiance: V3, sun_intensity: Option<f64>) -> Result<V3, SkyError> {
    if !sun_irradiance.iter().all(|c| (0.0..=1.0).contains(c)) {
        return err("sunIrradiance is three channels in [0, 1]");
    }
    let k = sun_intensity.unwrap_or(1.0);
    if !(k >= 0.0 && k.is_finite()) {
        return err("sunIntensity is a finite non-negative number");
    }
    Ok(sun_irradiance.map(|c| c * k))
}

/// The solid angle of a disc of the given angular diameter in degrees, 2π(1 − cos(θ/2)).
pub fn disc_solid_angle(angular_diameter: f64) -> Result<f64, SkyError> {
    if !(0.0..=180.0).contains(&angular_diameter) {
        return err("a disc's angular diameter is 0 to 180 degrees");
    }
    Ok(2.0 * PI * (1.0 - (angular_diameter.to_radians() / 2.0).cos()))
}

/// The Lambert sphere's phase law at a phase angle in degrees: 1 at full phase, 1/π at quadrature, 0 at new.
pub fn lambert_phase(phase_angle: f64) -> Result<f64, SkyError> {
    if !(0.0..=180.0).contains(&phase_angle) {
        return err("a phase angle is 0 to 180 degrees");
    }
    let a = phase_angle.to_radians();
    Ok((a.sin() + (PI - a) * a.cos()) / PI)
}

/// Rayleigh's phase function of the cosine between the view and the sun.
pub fn rayleigh_phase(mu: f64) -> f64 {
    3.0 / (16.0 * PI) * (1.0 + mu * mu)
}

/// Henyey–Greenstein's phase function of anisotropy `g`.
pub fn hg_phase(mu: f64, g: f64) -> f64 {
    (1.0 - g * g) / (4.0 * PI * (1.0 + g * g - 2.0 * g * mu).powf(1.5))
}

/// 2π times the Simpson integral of `f` over μ in [−1, 1] with `n` (even) intervals.
pub fn integrate_phase(f: impl Fn(f64) -> f64, n: usize) -> f64 {
    let h = 2.0 / n as f64;
    let mut s = f(-1.0) + f(1.0);
    for i in 1..n {
        s += if i % 2 == 1 { 4.0 } else { 2.0 } * f(-1.0 + i as f64 * h);
    }
    2.0 * PI * s * h / 3.0
}

fn fibonacci_sphere(n: usize) -> Vec<V3> {
    let golden = PI * (3.0 - 5f64.sqrt());
    (0..n)
        .map(|i| {
            let z = 1.0 - 2.0 * (i as f64 + 0.5) / n as f64;
            let r = (1.0 - z * z).max(0.0).sqrt();
            let t = golden * i as f64;
            [r * t.cos(), r * t.sin(), z]
        })
        .collect()
}

fn check_samples(samples: usize) -> Result<(), SkyError> {
    if !(1..=MAX_SAMPLES).contains(&samples) {
        return Err(SkyError(format!("samples is 1 to {MAX_SAMPLES}")));
    }
    Ok(())
}

impl Atmosphere {
    fn top(&self) -> f64 {
        self.top_radius - self.planet_radius
    }

    fn height(&self, p: V3) -> f64 {
        norm(p) - self.planet_radius
    }

    /// The Rayleigh layer's density at height `h`, zero outside the atmosphere.
    pub fn rayleigh_density(&self, h: f64) -> f64 {
        if (0.0..=self.top()).contains(&h) {
            (-h / self.rayleigh_scale).exp()
        } else {
            0.0
        }
    }

    /// The Mie layer's density at height `h`, zero outside the atmosphere.
    pub fn mie_density(&self, h: f64) -> f64 {
        if (0.0..=self.top()).contains(&h) {
            (-h / self.mie_scale).exp()
        } else {
            0.0
        }
    }

    /// The ozone tent's density at height `h`.
    pub fn ozone_density(&self, h: f64) -> f64 {
        (1.0 - (h - self.ozone_centre).abs() / self.ozone_width).max(0.0)
    }

    fn extinction(&self, h: f64) -> V3 {
        let (r, m, o) = (self.rayleigh_density(h), self.mie_density(h), self.ozone_density(h));
        std::array::from_fn(|c| {
            self.rayleigh_scattering[c] * r
                + (self.mie_scattering + self.mie_absorption) * m
                + self.ozone_absorption[c] * o
        })
    }

    /// The (Rayleigh, Mie) scattering coefficients at height `h`.
    fn scattering(&self, h: f64) -> (V3, f64) {
        let (r, m) = (self.rayleigh_density(h), self.mie_density(h));
        (self.rayleigh_scattering.map(|c| c * r), self.mie_scattering * m)
    }

    /// The distance along the ray to the atmosphere, the ray's length inside it and whether it ends on the ground.
    /// A ray that starts on the ground and heads inward has length 0 and ends on the ground.
    fn path_end(&self, p: V3, d: V3) -> Result<(f64, f64, bool), SkyError> {
        let r = norm(p);
        if r < self.planet_radius - 1e-6 {
            return err("the observer is below the ground");
        }
        let start = if r > self.top_radius {
            match sphere_hit(p, d, self.top_radius) {
                Some(t) => t,
                None => return Ok((0.0, 0.0, false)),
            }
        } else {
            0.0
        };
        let q = march(p, d, start);
        if dot(q, d) < 0.0 {
            if let Some(tg) = sphere_hit(q, d, self.planet_radius) {
                return Ok((start, tg, true));
            }
        }
        // the far exit from the top sphere
        let b = dot(q, d);
        let c = dot(q, q) - self.top_radius * self.top_radius;
        Ok((start, -b + (b * b - c).max(0.0).sqrt(), false))
    }

    /// Whether the ray from p along d meets the planet before leaving the atmosphere.
    pub fn hits_ground(&self, p: V3, d: V3) -> Result<bool, SkyError> {
        Ok(self.path_end(p, d)?.2)
    }

    /// The optical depth of the segment [s0, s1] along d from p by 4-point Gauss–Legendre.
    fn depth(&self, p: V3, d: V3, s0: f64, s1: f64) -> V3 {
        let (half, mid) = ((s1 - s0) / 2.0, (s1 + s0) / 2.0);
        let mut out = [0.0; 3];
        for (x, w) in GL {
            let e = self.extinction(self.height(march(p, d, mid + half * x)));
            for c in 0..3 {
                out[c] += w * e[c];
            }
        }
        out.map(|o| o * half)
    }

    /// Rule 9: the transmittance from p along the unit d to the top of the atmosphere, zero when the ray meets the
    /// ground first (64 segments clustered toward p).
    pub fn transmittance(&self, p: V3, d: V3) -> Result<V3, SkyError> {
        self.transmittance_with(p, d, 64, None, true)
    }

    /// [`Atmosphere::transmittance`] over `samples` segments, uniform or clustered, and over at most `length`.
    pub fn transmittance_with(
        &self,
        p: V3,
        d: V3,
        samples: usize,
        length: Option<f64>,
        cluster: bool,
    ) -> Result<V3, SkyError> {
        check_samples(samples)?;
        let (start, mut total, ground) = self.path_end(p, d)?;
        if ground && length.is_none_or(|l| l >= start + total) {
            return Ok([0.0; 3]);
        }
        if let Some(l) = length {
            total = total.min(l - start).max(0.0);
        }
        if total <= 0.0 {
            return Ok([1.0; 3]);
        }
        let q = march(p, d, start);
        let ends = segments(total, samples, cluster);
        let mut depth = [0.0; 3];
        for i in 0..samples {
            let seg = self.depth(q, d, ends[i], ends[i + 1]);
            for c in 0..3 {
                depth[c] += seg[c];
            }
        }
        Ok(depth.map(|x| (-x).exp()))
    }

    /// The sun disc seen along d from `obs`: the irradiance through the transmittance, zero through the ground.
    pub fn sun_radiance(&self, obs: V3, d: V3) -> Result<V3, SkyError> {
        let t = self.transmittance(obs, d)?;
        Ok(std::array::from_fn(|c| self.sun_irradiance[c] * t[c]))
    }

    /// Rule 6: the sun disc's radiance, `sunColor` times [`Atmosphere::sun_radiance`] over the disc's solid angle, so
    /// that the disc's power over `sun_size` degrees is the transmitted irradiance whatever the size; no disc (zeros)
    /// at size 0.
    pub fn sun_disc_radiance(&self, obs: V3, d: V3, sun_size: f64, sun_color: V3) -> Result<V3, SkyError> {
        if !sun_color.iter().all(|c| (0.0..=1.0).contains(c)) {
            return err("sunColor is three channels in [0, 1]");
        }
        let omega = disc_solid_angle(sun_size)?;
        if omega == 0.0 {
            return Ok([0.0; 3]);
        }
        let e = self.sun_radiance(obs, d)?;
        Ok(std::array::from_fn(|c| sun_color[c] * e[c] / omega))
    }

    /// Rule 7: the moon disc's radiance, uniform over the disc: `moon_intensity` times [`MOON_ALBEDO`] times the
    /// Lambert phase law at the moon's phase angle times the sun's irradiance over π, through the transmittance along
    /// the moon's direction (zero below the horizon). Only the moon's zenith angle matters to an observer on the z axis.
    pub fn moon_radiance(&self, obs: V3, moon: &sr_space::moon::Moon, moon_intensity: f64) -> Result<V3, SkyError> {
        if !(moon_intensity >= 0.0 && moon_intensity.is_finite()) {
            return err("moonIntensity is a finite non-negative number");
        }
        let z = moon.zenith.to_radians();
        let t = self.transmittance(obs, [z.sin(), 0.0, z.cos()])?;
        let k = moon_intensity * MOON_ALBEDO * lambert_phase(moon.phase_angle)? / PI;
        Ok(std::array::from_fn(|c| k * self.sun_irradiance[c] * t[c]))
    }

    /// Hillaire's isotropic multiple-scattering estimate at height `h` and sun cosine `mu_s`: the second-order
    /// radiance over one minus the fraction f scattered again, and f, both integrated over a Fibonacci sphere of
    /// directions with the uniform phase 1 / 4π, a Lambertian ground included.
    pub fn ms_cell(&self, h: f64, mu_s: f64) -> (V3, V3) {
        let p = [0.0, 0.0, self.planet_radius + h];
        let sun = [(1.0 - mu_s * mu_s).max(0.0).sqrt(), 0.0, mu_s];
        let (mut l2, mut f) = ([0.0; 3], [0.0; 3]);
        let dw = 4.0 * PI / MS_DIRECTIONS as f64;
        // p is at or above the ground and every sample count is in range, so the rays cannot fail
        let transmit = |q: V3| self.transmittance_with(q, sun, 8, None, true).unwrap_or([0.0; 3]);
        for d in fibonacci_sphere(MS_DIRECTIONS) {
            let Ok((start, total, ground)) = self.path_end(p, d) else { continue };
            if total <= 0.0 {
                continue;
            }
            let q = march(p, d, start);
            let ends = segments(total, MS_SAMPLES, true);
            let mut trans = [1.0; 3];
            for i in 0..MS_SAMPLES {
                let (s0, s1) = (ends[i], ends[i + 1]);
                let seg = self.depth(q, d, s0, s1);
                let mid = march(q, d, (s0 + s1) / 2.0);
                let t_mid: V3 = std::array::from_fn(|c| trans[c] * (-seg[c] / 2.0).exp());
                let (sr, sm) = self.scattering(self.height(mid));
                let t_sun = transmit(mid);
                for c in 0..3 {
                    let sig = sr[c] + sm;
                    f[c] += t_mid[c] * sig / (4.0 * PI) * (s1 - s0) * dw;
                    l2[c] += t_mid[c] * sig / (4.0 * PI) * self.sun_irradiance[c] * t_sun[c] * (s1 - s0) * dw;
                }
                for c in 0..3 {
                    trans[c] *= (-seg[c]).exp();
                }
            }
            if ground {
                let g = march(q, d, total);
                let n = mul(g, 1.0 / norm(g));
                let cos_sun = dot(n, sun).max(0.0);
                let t_sun = if cos_sun > 0.0 { transmit(g) } else { [0.0; 3] };
                for c in 0..3 {
                    l2[c] += trans[c] * self.ground_albedo / PI * self.sun_irradiance[c] * t_sun[c] * cos_sun
                        / (4.0 * PI)
                        * dw;
                }
            }
        }
        (std::array::from_fn(|c| l2[c] / (1.0 - f[c])), f)
    }

    /// The fraction of light scattered once more at (h, mu_s).
    pub fn multiple_scattering_factor(&self, h: f64, mu_s: f64) -> V3 {
        self.ms_cell(h, mu_s).1
    }

    /// From an observer on the +z axis above the atmosphere: the unit direction in the x–z plane grazing the
    /// planet's limb, turned `degrees` toward the nadir (positive) or away (negative).
    pub fn limb_direction(&self, obs: V3, degrees: f64) -> Result<V3, SkyError> {
        let r = norm(obs);
        if r <= self.top_radius {
            return err("the limb is seen from above the atmosphere");
        }
        let gamma = (self.planet_radius / r).asin() - degrees.to_radians();
        Ok([gamma.sin(), 0.0, -gamma.cos()])
    }
}

/// An atmosphere with its multiple-scattering table over height and sun cosine (rule 9), filled once: the table
/// depends only on the atmosphere, so a frame rendered alone equals the frame reached by playing.
#[derive(Debug, Clone)]
pub struct Scattering {
    pub atmosphere: Atmosphere,
    /// `MS_HEIGHTS` × `MS_COSINES` cells, heights first.
    table: Vec<V3>,
}

impl Scattering {
    /// Fills the table of `a`.
    pub fn new(a: Atmosphere) -> Scattering {
        let top = a.top();
        let table = (0..MS_HEIGHTS * MS_COSINES)
            .map(|k| {
                let (i, j) = (k / MS_COSINES, k % MS_COSINES);
                a.ms_cell(top * i as f64 / (MS_HEIGHTS - 1) as f64, -1.0 + 2.0 * j as f64 / (MS_COSINES - 1) as f64).0
            })
            .collect();
        Scattering { atmosphere: a, table }
    }

    /// The table bilinear in height and sun cosine, clamped to its edges.
    pub fn multiple(&self, h: f64, mu_s: f64) -> V3 {
        let hf = (h / self.atmosphere.top()).clamp(0.0, 1.0) * (MS_HEIGHTS - 1) as f64;
        let mf = (mu_s.clamp(-1.0, 1.0) + 1.0) / 2.0 * (MS_COSINES - 1) as f64;
        let (i0, j0) = (hf.floor() as usize, mf.floor() as usize);
        let (i1, j1) = ((i0 + 1).min(MS_HEIGHTS - 1), (j0 + 1).min(MS_COSINES - 1));
        let (ti, tj) = (hf - i0 as f64, mf - j0 as f64);
        let mut out = [0.0; 3];
        for (i, wi) in [(i0, 1.0 - ti), (i1, ti)] {
            for (j, wj) in [(j0, 1.0 - tj), (j1, tj)] {
                if wi * wj == 0.0 {
                    continue;
                }
                let psi = self.table[i * MS_COSINES + j];
                for c in 0..3 {
                    out[c] += wi * wj * psi[c];
                }
            }
        }
        out
    }

    /// The in-scattered radiance along d from p, to `length` or the atmosphere's end, and the transmittance there.
    fn inscatter(
        &self,
        p: V3,
        d: V3,
        sun: V3,
        samples: usize,
        multiple: bool,
        length: Option<f64>,
    ) -> Result<(V3, V3), SkyError> {
        check_samples(samples)?;
        let a = &self.atmosphere;
        let (start, mut total, _) = a.path_end(p, d)?;
        if let Some(l) = length {
            total = total.min(l - start).max(0.0);
        }
        if total <= 0.0 {
            return Ok(([0.0; 3], [1.0; 3]));
        }
        let q = march(p, d, start);
        let ends = segments(total, samples, true);
        let mu = dot(d, sun);
        let (pr, pm) = (rayleigh_phase(mu), hg_phase(mu, a.mie_anisotropy));
        let (mut trans, mut out) = ([1.0; 3], [0.0; 3]);
        for i in 0..samples {
            let (s0, s1) = (ends[i], ends[i + 1]);
            let (half, mid) = ((s1 - s0) / 2.0, (s1 + s0) / 2.0);
            let mut inner = [0.0; 3];
            for (x, w) in GL {
                // the transmittance from the segment's start to the node, then the scattering at the node
                let s = mid + half * x;
                let node_depth = if s > s0 { a.depth(q, d, s0, s) } else { [0.0; 3] };
                let y = march(q, d, s);
                let hy = a.height(y);
                let (sr, sm) = a.scattering(hy);
                let t_sun = a.transmittance_with(y, sun, 8, None, true)?;
                let mu_s = dot(mul(y, 1.0 / norm(y)), sun);
                let psi = if multiple { self.multiple(hy, mu_s) } else { [0.0; 3] };
                for c in 0..3 {
                    let t_node = trans[c] * (-node_depth[c]).exp();
                    let single = (sr[c] * pr + sm * pm) * a.sun_irradiance[c] * t_sun[c];
                    inner[c] += w * t_node * (single + (sr[c] + sm) * psi[c]);
                }
            }
            let seg = a.depth(q, d, s0, s1);
            for c in 0..3 {
                out[c] += inner[c] * half;
                trans[c] *= (-seg[c]).exp();
            }
        }
        Ok((out, trans))
    }

    /// Rule 9: the sky's radiance in the unit direction d from `obs` toward the unit `sun`, single and multiple
    /// scattering over 32 segments; zero for a direction that meets the ground or misses the atmosphere.
    pub fn sky_radiance(&self, obs: V3, d: V3, sun: V3) -> Result<V3, SkyError> {
        self.sky_radiance_with(obs, d, sun, true, 32)
    }

    /// [`Scattering::sky_radiance`] with or without the multiple-scattering term, over `samples` segments.
    pub fn sky_radiance_with(&self, obs: V3, d: V3, sun: V3, multiple: bool, samples: usize) -> Result<V3, SkyError> {
        let (_, total, ground) = self.atmosphere.path_end(obs, d)?;
        if ground || total <= 0.0 {
            return Ok([0.0; 3]);
        }
        Ok(self.inscatter(obs, d, sun, samples, multiple, None)?.0)
    }

    /// The sky's irradiance on the ground under an observer on the +z axis, a midpoint grid of 6 × 12 cells over
    /// the hemisphere (16 segments a ray).
    pub fn sky_irradiance(&self, obs: V3, sun: V3) -> Result<V3, SkyError> {
        let (zenith_steps, azimuth_steps) = (6, 12);
        let mut out = [0.0; 3];
        for i in 0..zenith_steps {
            let theta = (i as f64 + 0.5) / zenith_steps as f64 * (PI / 2.0);
            for j in 0..azimuth_steps {
                let phi = (j as f64 + 0.5) / azimuth_steps as f64 * 2.0 * PI;
                let d = [theta.sin() * phi.cos(), theta.sin() * phi.sin(), theta.cos()];
                let l = self.sky_radiance_with(obs, d, sun, true, 16)?;
                let w =
                    theta.cos() * theta.sin() * (PI / 2.0 / zenith_steps as f64) * (2.0 * PI / azimuth_steps as f64);
                for c in 0..3 {
                    out[c] += l[c] * w;
                }
            }
        }
        Ok(out)
    }

    /// Rule 10: a surface of `colour` at `distance` along d from `obs`, seen through the atmosphere: the colour times
    /// the transmittance plus the in-scattering up to it (16 segments); at distance 0 the colour itself.
    pub fn aerial_perspective(&self, obs: V3, d: V3, distance: f64, colour: V3, sun: V3) -> Result<V3, SkyError> {
        if distance <= 0.0 {
            return Ok(colour);
        }
        let (inscatter, trans) = self.inscatter(obs, d, sun, 16, true, Some(distance))?;
        Ok(std::array::from_fn(|c| colour[c] * trans[c] + inscatter[c]))
    }
}

/// Rule 11: a fog of extinction `density` · e^(−h / `height`) per metre.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fog {
    pub density: f64,
    pub height: f64,
}

impl Fog {
    /// `fogDensity` (non-negative) and `fogHeight` (positive), both finite.
    pub fn new(density: f64, height: f64) -> Result<Fog, SkyError> {
        if !(density >= 0.0 && density.is_finite() && height > 0.0 && height.is_finite()) {
            return err("fogDensity is non-negative and fogHeight positive");
        }
        Ok(Fog { density, height })
    }

    /// The transmittance of a straight segment of `length` metres from altitude `h0` to `h1`, in closed form.
    pub fn transmittance(&self, h0: f64, h1: f64, length: f64) -> f64 {
        if length <= 0.0 {
            return 1.0;
        }
        let hs = self.height;
        let depth = if h0 == h1 {
            self.density * length * (-h0 / hs).exp()
        } else {
            self.density * length * hs / (h1 - h0) * ((-h0 / hs).exp() - (-h1 / hs).exp())
        };
        (-depth).exp()
    }
}
