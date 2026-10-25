//! Bounded participating media and a deterministic reference transport integrator.
//!
//! Extinction follows Beer–Lambert transmittance. Emission is radiance per scene
//! unit at unit density; scattering uses the caller's phase-weighted incident light.
//! The midpoint quadrature integrates each constant-coefficient step analytically,
//! clips to the nearest surface, and combines overlapping media before integration.
//! It is single scattering, not a predictive radiation or fluid dynamics solver.

use std::sync::Arc;

use glam::DVec3;

use crate::{Error, SparseGrid, Transform};

/// A finite, nonempty domain in the volume asset's world coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    lo: [f64; 3],
    hi: [f64; 3],
}

impl Bounds {
    /// The box from `lo` to `hi`. Every coordinate must be finite and every minimum below its maximum.
    pub fn new(lo: [f64; 3], hi: [f64; 3]) -> Result<Self, Error> {
        if (0..3).any(|i| !lo[i].is_finite() || !hi[i].is_finite() || lo[i] >= hi[i]) {
            return Err(Error::Invalid("domain bounds must be finite with minimum < maximum"));
        }
        Ok(Self { lo, hi })
    }

    /// The minimum corner.
    pub fn min(&self) -> [f64; 3] {
        self.lo
    }

    /// The maximum corner.
    pub fn max(&self) -> [f64; 3] {
        self.hi
    }

    /// Whether `p` lies inside, boundary included.
    pub fn contains(&self, p: [f64; 3]) -> bool {
        (0..3).all(|i| p[i] >= self.lo[i] && p[i] <= self.hi[i])
    }

    fn intersect(&self, o: DVec3, d: DVec3, max: f64) -> Option<[f64; 2]> {
        let (mut near, mut far) = (0.0_f64, max);
        for i in 0..3 {
            if d[i] == 0.0 {
                if o[i] < self.lo[i] || o[i] > self.hi[i] {
                    return None;
                }
            } else {
                let a = (self.lo[i] - o[i]) / d[i];
                let b = (self.hi[i] - o[i]) / d[i];
                near = near.max(a.min(b));
                far = far.min(a.max(b));
            }
        }
        (far > near).then_some([near, far])
    }
}

/// Scalar extinction preserves exact RGBA compositing; albedo and emission are linear RGB.
#[derive(Clone, Copy, Debug)]
pub struct Optical {
    /// Multiplier applied to every density sample; nonnegative.
    pub density_scale: f64,
    /// Extinction coefficient per scene unit at unit density; nonnegative.
    pub extinction: f64,
    /// Single-scattering albedo per channel, in [0, 1].
    pub albedo: [f64; 3],
    /// Emitted radiance per scene unit at unit density, per channel; nonnegative.
    pub emission: [f64; 3],
    /// The phase function of the scattering (the media draft, section 4).
    pub phase: Phase,
    /// Maximum path-tracer scattering collisions; zero retains absorption and emission only.
    /// The CPU reference integrator evaluates the first scattering order.
    pub scatter_bounces: u32,
}

impl Default for Optical {
    fn default() -> Self {
        Self {
            density_scale: 1.0,
            extinction: 1.0,
            // white, as the schema's medium/@albedo default (#FFFFFFFF) says; the renderer takes this value too
            albedo: [1.0; 3],
            emission: [0.0; 3],
            phase: Phase::Hg { g: 0.0 },
            scatter_bounces: 1,
        }
    }
}

impl Optical {
    /// Refuses non-finite or out-of-range coefficients and phase parameters.
    pub fn validate(&self) -> Result<(), Error> {
        if self.scatter_bounces > 32
            || !self.density_scale.is_finite()
            || self.density_scale < 0.0
            || !self.extinction.is_finite()
            || self.extinction < 0.0
            || self.albedo.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            || self.emission.iter().any(|v| !v.is_finite() || *v < 0.0)
        {
            return Err(Error::Invalid(
                "medium coefficients must be finite, nonnegative; albedo in [0,1], scatter bounces in [0,32]",
            ));
        }
        self.phase.validate()
    }
}

/// Immutable validated density and material, with an object-to-scene transform.
#[derive(Clone, Debug)]
pub struct Medium {
    density: Arc<SparseGrid>,
    bounds: Option<Bounds>,
    transform: Transform,
    optical: Optical,
    temperature: Option<Temperature>,
    authored_bounds: Option<Bounds>,
    next_density: Option<Arc<SparseGrid>>,
    frame_blend: f64,
    advection: Option<[crate::advection::Advection; 2]>,
}

/// Validated kelvin field, independent of the density grid's transform and resolution.
#[derive(Clone, Debug)]
pub struct Temperature {
    grid: Arc<SparseGrid>,
    scale: f64,
    emission_scale: f64,
    next_grid: Option<Arc<SparseGrid>>,
}

impl Temperature {
    /// The temperature grid of the first frame.
    pub fn grid(&self) -> &Arc<SparseGrid> {
        &self.grid
    }
    /// Multiplier from grid samples to kelvin.
    pub fn scale(&self) -> f64 {
        self.scale
    }
    /// Multiplier on the blackbody colour added to the emission.
    pub fn emission_scale(&self) -> f64 {
        self.emission_scale
    }
    /// The temperature grid of the second frame, when the medium blends two frames.
    pub fn next_grid(&self) -> Option<&Arc<SparseGrid>> {
        self.next_grid.as_ref()
    }
}

impl Medium {
    /// A nonzero grid background requires an explicit finite domain. Otherwise bounds
    /// are inferred from nonzero samples plus the full trilinear interpolation support.
    pub fn new(
        density: Arc<SparseGrid>,
        bounds: Option<Bounds>,
        transform: Transform,
        optical: Optical,
    ) -> Result<Self, Error> {
        optical.validate()?;
        if density.background() < 0.0 {
            return Err(Error::Invalid("negative density background"));
        }
        let mut lo = DVec3::splat(f64::INFINITY);
        let mut hi = DVec3::splat(f64::NEG_INFINITY);
        for (key, values) in density.bricks() {
            for (i, v) in values.iter().enumerate() {
                if *v < 0.0 {
                    return Err(Error::Invalid("negative density sample"));
                }
                if *v == 0.0 {
                    continue;
                }
                let p = DVec3::new(
                    f64::from(key[0]) * 8.0 + (i % 8) as f64,
                    f64::from(key[1]) * 8.0 + (i / 8 % 8) as f64,
                    f64::from(key[2]) * 8.0 + (i / 64) as f64,
                );
                lo = lo.min(p - DVec3::ONE);
                hi = hi.max(p + DVec3::ONE);
            }
        }
        let authored_bounds = bounds;
        let bounds = if bounds.is_some() {
            bounds
        } else if density.background() != 0.0 {
            return Err(Error::Invalid("nonzero density background requires explicit domain bounds"));
        } else if lo.is_finite() {
            let mut world_lo = DVec3::splat(f64::INFINITY);
            let mut world_hi = DVec3::splat(f64::NEG_INFINITY);
            for k in 0..8 {
                let p = std::array::from_fn(|i| if k & (1 << i) == 0 { lo[i] } else { hi[i] });
                let p = DVec3::from(density.transform().index_to_world(p));
                world_lo = world_lo.min(p);
                world_hi = world_hi.max(p);
            }
            Some(Bounds::new(world_lo.to_array(), world_hi.to_array())?)
        } else {
            None
        };
        Ok(Self {
            density,
            bounds,
            transform,
            optical,
            temperature: None,
            authored_bounds,
            next_density: None,
            frame_blend: 0.0,
            advection: None,
        })
    }

    /// Adds thermal emission to the constant optical emission. The emission scale
    /// converts the normalized blackbody RGB to radiance per unit density and length.
    /// Temperature is interpolated in field space before converting to emission.
    pub fn with_temperature(mut self, grid: Arc<SparseGrid>, scale: f64, emission_scale: f64) -> Result<Self, Error> {
        if self.next_density.is_some() {
            return Err(Error::Invalid("set temperature before adding a second volume frame"));
        }
        if !scale.is_finite() || scale <= 0.0 || !emission_scale.is_finite() || emission_scale < 0.0 {
            return Err(Error::Invalid("temperature scale must be positive; emission scale nonnegative and finite"));
        }
        if std::iter::once(grid.background())
            .chain(grid.bricks().flat_map(|(_, v)| v.iter().copied()))
            .any(|v| !(0.0..=crate::thermal::MAX_KELVIN).contains(&(f64::from(v) * scale)))
        {
            return Err(Error::Invalid("scaled temperature samples must be in 0..50000 kelvin"));
        }
        self.temperature = Some(Temperature { grid, scale, emission_scale, next_grid: None });
        Ok(self)
    }

    /// Adds a second cache frame. Both scalar fields are sampled in their own
    /// asset-world space and blended before computing extinction or thermal emission.
    /// Material and object transform belong to this evaluated instant, not to frames.
    pub fn with_next_frame(
        mut self,
        density: Arc<SparseGrid>,
        temperature: Option<Arc<SparseGrid>>,
        blend: f64,
    ) -> Result<Self, Error> {
        if !blend.is_finite() || !(0.0..=1.0).contains(&blend) {
            return Err(Error::Invalid("volume frame blend must be finite and in [0,1]"));
        }
        if self.next_density.is_some() {
            return Err(Error::Invalid("a medium interpolates at most two volume frames"));
        }
        let mut next = Self::new(density.clone(), self.authored_bounds, self.transform, self.optical)?;
        match (&mut self.temperature, temperature) {
            (Some(t), Some(grid)) => {
                next = next.with_temperature(grid.clone(), t.scale, t.emission_scale)?;
                t.next_grid = Some(grid);
            }
            (None, None) => {}
            _ => return Err(Error::Invalid("both volume frames require the selected temperature channel")),
        }
        self.bounds = match (self.bounds, next.bounds) {
            (Some(a), Some(b)) => Some(Bounds::new(
                std::array::from_fn(|i| a.lo[i].min(b.lo[i])),
                std::array::from_fn(|i| a.hi[i].max(b.hi[i])),
            )?),
            (a, b) => a.or(b),
        };
        self.next_density = Some(density);
        self.frame_blend = blend;
        Ok(self)
    }

    /// The density grid of the second frame, when the medium blends two frames.
    pub fn next_density(&self) -> Option<&Arc<SparseGrid>> {
        self.next_density.as_ref()
    }

    /// Applies independent endpoint traces before interpolating density and kelvin.
    /// Add the second frame first. Authored bounds remain a clipping domain;
    /// inferred bounds expand conservatively to include advected field support.
    pub fn with_advection(
        mut self,
        first: crate::advection::Advection,
        second: crate::advection::Advection,
    ) -> Result<Self, Error> {
        if self.next_density.is_none() || self.advection.is_some() {
            return Err(Error::Invalid("advection requires a frame pair and may only be set once"));
        }
        if let Some(mut bounds) = self.bounds {
            if self.authored_bounds.is_none() {
                let travel: [f64; 3] = std::array::from_fn(|i| first.displacement()[i].max(second.displacement()[i]));
                bounds = Bounds::new(
                    std::array::from_fn(|i| bounds.lo[i] - travel[i]),
                    std::array::from_fn(|i| bounds.hi[i] + travel[i]),
                )?;
            }
            first.validate_domain(bounds.lo, bounds.hi)?;
            second.validate_domain(bounds.lo, bounds.hi)?;
            self.bounds = Some(bounds);
        }
        self.advection = Some([first, second]);
        Ok(self)
    }

    /// The velocity traces of the two frames, when the frames are advected before blending.
    pub fn advection(&self) -> Option<&[crate::advection::Advection; 2]> {
        self.advection.as_ref()
    }
    /// Weight of the second frame, in [0, 1]; zero with a single frame.
    pub fn frame_blend(&self) -> f64 {
        self.frame_blend
    }

    /// Density fields referenced by this instance, for upload and resource accounting.
    pub fn density_grids(&self) -> impl Iterator<Item = &Arc<SparseGrid>> {
        std::iter::once(&self.density).chain(self.next_density.as_ref())
    }

    fn sample_field(&self, first: &SparseGrid, next: Option<&Arc<SparseGrid>>, scene: [f64; 3]) -> f64 {
        let local = self.transform.world_to_index(scene);
        let points = self.advection.as_ref().map_or([local; 2], |traces| traces.each_ref().map(|t| t.backtrace(local)));
        let a = f64::from(first.sample_world(points[0]));
        next.map_or(a, |b| a * (1.0 - self.frame_blend) + f64::from(b.sample_world(points[1])) * self.frame_blend)
    }

    /// The temperature field, when the medium has thermal emission.
    pub fn temperature(&self) -> Option<&Temperature> {
        self.temperature.as_ref()
    }

    /// Per-density emission; callers multiply by density and integrate in scene units.
    pub fn sample_emission(&self, scene: [f64; 3]) -> [f64; 3] {
        let mut emission = self.optical.emission;
        if let Some(t) = &self.temperature {
            let kelvin = self.sample_field(&t.grid, t.next_grid.as_ref(), scene) * t.scale;
            let rgb = crate::thermal::blackbody_rgb(kelvin.clamp(0.0, crate::thermal::MAX_KELVIN))
                .expect("validated temperature field");
            for c in 0..3 {
                emission[c] += rgb[c] * t.emission_scale;
            }
        }
        emission
    }

    /// The density grid of the first frame.
    pub fn density(&self) -> &Arc<SparseGrid> {
        &self.density
    }

    /// The domain in asset-world coordinates: authored, or inferred from the density. `None` for an empty grid.
    pub fn bounds(&self) -> Option<Bounds> {
        self.bounds
    }

    /// The object-to-scene transform.
    pub fn transform(&self) -> Transform {
        self.transform
    }

    /// The material coefficients.
    pub fn optical(&self) -> Optical {
        self.optical
    }

    /// Scaled density at a scene position, blended between frames; zero outside the domain.
    pub fn sample_density(&self, scene: [f64; 3]) -> f64 {
        let local = self.transform.world_to_index(scene);
        if !self.bounds.is_some_and(|b| b.contains(local)) {
            return 0.0;
        }
        self.sample_field(&self.density, self.next_density.as_ref(), scene) * self.optical.density_scale
    }

    /// Ray interval in scene-distance units, including nonuniform object scales.
    pub fn interval(&self, ray: Ray) -> Option<[f64; 2]> {
        let o = self.transform.inverse.transform_point3(ray.origin);
        let d = self.transform.inverse.transform_vector3(ray.direction);
        self.bounds?.intersect(o, d, ray.distance)
    }
}

/// A scene-space ray with a unit direction and a finite nearest-surface distance.
#[derive(Clone, Copy, Debug)]
pub struct Ray {
    origin: DVec3,
    direction: DVec3,
    distance: f64,
}

impl Ray {
    /// A ray from `origin` along `direction` (normalized here; must be nonzero) for `distance` scene units.
    pub fn new(origin: [f64; 3], direction: [f64; 3], distance: f64) -> Result<Self, Error> {
        let origin = DVec3::from(origin);
        let direction = DVec3::from(direction);
        if !origin.is_finite() || !direction.is_finite() || !distance.is_finite() || distance < 0.0 {
            return Err(Error::Invalid("ray must be finite with nonnegative distance"));
        }
        let direction = direction.try_normalize().ok_or(Error::Invalid("ray direction must be nonzero"))?;
        Ok(Self { origin, direction, distance })
    }
}

/// Explicit accuracy and work budgets. Insufficient work budgets fail before sampling.
#[derive(Clone, Copy, Debug)]
pub struct March {
    /// Largest quadrature step, in scene units.
    pub step_size: f64,
    /// Largest number of steps along one ray, across all media.
    pub max_steps: u32,
}

impl Default for March {
    fn default() -> Self {
        Self { step_size: 0.5, max_steps: 4096 }
    }
}

/// The result of integrating one ray.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transport {
    /// Radiance emitted and scattered toward the ray origin, linear RGB.
    pub radiance: [f64; 3],
    /// Fraction of light from behind the media that reaches the origin, in [0, 1].
    pub transmittance: f64,
    /// Quadrature steps taken.
    pub steps: u32,
}

/// The phase function of a medium (the media draft, section 4). Directions are propagation directions and μ is
/// the cosine between the arriving and leaving ones, so μ = 1 continues straight on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    /// Henyey–Greenstein with asymmetry g in (−1, 1): medium/@phase="hg" with @anisotropy.
    Hg {
        /// The mean cosine.
        g: f64,
    },
    /// Rayleigh, 3 (1 + μ²) / (16π).
    Rayleigh,
    /// Draine's two-parameter function: g from @anisotropy, alpha ≥ 0 from @phaseAlpha.
    Draine {
        /// The asymmetry, in (−1, 1).
        g: f64,
        /// The side-lobe weight, nonnegative.
        alpha: f64,
    },
    /// The HG and Draine mixture fitted to water droplets of this mean diameter in micrometres, 5 to 50.
    Fog {
        /// The droplet diameter in micrometres.
        diameter: f64,
    },
}

/// Below this |g| Henyey–Greenstein is sampled as isotropic (SREP 79 rule 6).
const HG_ISOTROPIC: f64 = 1e-3;
/// Halvings of [−1, 1] in the Draine inversion: 2 / 2⁶⁴ is far below the 1e-12 the draft asks.
const BISECTION_STEPS: u32 = 64;

impl Phase {
    /// Refuses a parameter outside the domain of rule 16, or a droplet diameter outside the fit's 5 to 50.
    pub fn validate(&self) -> Result<(), Error> {
        let asymmetry = |g: f64| g.is_finite() && g.abs() < 1.0;
        let ok = match *self {
            Phase::Hg { g } => asymmetry(g),
            Phase::Rayleigh => true,
            Phase::Draine { g, alpha } => asymmetry(g) && alpha.is_finite() && alpha >= 0.0,
            Phase::Fog { diameter } => (5.0..=50.0).contains(&diameter),
        };
        if ok {
            Ok(())
        } else {
            Err(Error::Invalid("medium phase needs g in (-1,1), alpha >= 0 and a droplet diameter in [5,50]"))
        }
    }

    /// p(μ), a density over the sphere of leaving directions.
    pub fn eval(&self, mu: f64) -> f64 {
        match *self {
            Phase::Hg { g } => phase_hg(mu, g),
            Phase::Rayleigh => phase_rayleigh(mu),
            Phase::Draine { g, alpha } => phase_draine(mu, g, alpha),
            Phase::Fog { diameter } => phase_fog(mu, diameter),
        }
    }

    /// μ of a leaving direction drawn from this phase function: `lobe` picks the fog's lobe (rule 20) and is
    /// unused by the others, `u` is inverted (rules 18 and 19). Both draws lie in [0, 1).
    pub fn sample(&self, lobe: f64, u: f64) -> f64 {
        match *self {
            Phase::Hg { g } => sample_hg(u, g),
            Phase::Rayleigh => sample_draine(u, 0.0, 1.0),
            Phase::Draine { g, alpha } => sample_draine(u, g, alpha),
            Phase::Fog { diameter } => sample_fog(lobe, u, diameter),
        }
    }

    /// Whether p is the same in every direction: only Henyey–Greenstein with g = 0 (rule 17).
    pub fn is_isotropic(&self) -> bool {
        *self == Phase::Hg { g: 0.0 }
    }
}

/// Rayleigh: 3 (1 + μ²) / (16π).
pub fn phase_rayleigh(mu: f64) -> f64 {
    let mu = mu.clamp(-1.0, 1.0);
    3.0 * (1.0 + mu * mu) / (16.0 * std::f64::consts::PI)
}

/// Draine (2003, equation 5). alpha = 0 is Henyey–Greenstein; g = 0, alpha = 1 is Rayleigh; alpha = 1 is
/// Cornette–Shanks. Callers supply |g| < 1 and alpha ≥ 0.
pub fn phase_draine(mu: f64, g: f64, alpha: f64) -> f64 {
    let mu = mu.clamp(-1.0, 1.0);
    let denominator = 1.0 + g * g - 2.0 * g * mu;
    let norm = (1.0 - g * g) / (4.0 * std::f64::consts::PI * (1.0 + alpha * (1.0 + 2.0 * g * g) / 3.0));
    norm * (1.0 + alpha * mu * mu) / denominator.powf(1.5)
}

/// The first moment of the Draine phase function (Draine 2003, equation A2).
pub fn mean_cosine_draine(g: f64, alpha: f64) -> f64 {
    g * (1.0 + alpha * (3.0 + 2.0 * g * g) / 5.0) / (1.0 + alpha * (1.0 + 2.0 * g * g) / 3.0)
}

/// The fitted parameters of the fog phase function (Jendersie and d'Eon 2023, equations 4 to 7).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FogParameters {
    /// Asymmetry of the Henyey–Greenstein lobe.
    pub g_hg: f64,
    /// Asymmetry of the Draine lobe.
    pub g_d: f64,
    /// Alpha of the Draine lobe.
    pub alpha: f64,
    /// Weight of the Draine lobe.
    pub w_d: f64,
}

/// The fog fit for a droplet diameter d in micrometres; the fit holds for 5 ≤ d ≤ 50, which callers supply.
pub fn fog_parameters(d: f64) -> FogParameters {
    FogParameters {
        g_hg: (-0.0990567 / (d - 1.67154)).exp(),
        g_d: (-2.20679 / (d + 3.91029) - 0.428934).exp(),
        alpha: (3.62489 - 8.29288 / (d + 5.52825)).exp(),
        w_d: (-0.599085 / (d - 0.641583) - 0.665888).exp(),
    }
}

/// (1 − w_D) HG(g_HG) + w_D Draine(g_D, alpha) with the parameters of [`fog_parameters`].
pub fn phase_fog(mu: f64, diameter: f64) -> f64 {
    let p = fog_parameters(diameter);
    (1.0 - p.w_d) * phase_hg(mu, p.g_hg) + p.w_d * phase_draine(mu, p.g_d, p.alpha)
}

/// The cumulative of the Draine phase function in μ, from −1 to `mu` (rule 19), K (I₀ + α I₂) with I_n the integral
/// of νⁿ t^(−3/2) over [−1, μ], t = 1 + g² − 2gν. The draft's antiderivative in t divides by g² and loses every digit
/// for 1e-6 < |g| < 1e-4; this form of the same integrals, with s₁ = 1 + g, s₂ = √t(μ) and P = s₁s₂, never divides
/// by g and holds at g = 0 (60-digit check: 6e-14 relative over |g| ≤ 0.99, α ≤ 30).
pub fn cdf_draine(mu: f64, g: f64, alpha: f64) -> f64 {
    let mu = mu.clamp(-1.0, 1.0);
    let a = 1.0 + g * g;
    let k = (1.0 - g * g) / (2.0 * (1.0 + alpha * (1.0 + 2.0 * g * g) / 3.0));
    let (s1, s2) = (1.0 + g, (a - 2.0 * g * mu).sqrt());
    let p = s1 * s2;
    let i0 = 2.0 * (1.0 + mu) / (p * (s1 + s2));
    let q = (mu - 1.0) * (3.0 * a + p) * (a * (mu - 1.0) + 2.0 * g * mu) / (a + p) + mu * (3.0 * a - p);
    let i2 = 2.0 * (1.0 + mu) * q / (3.0 * p * (s1 + s2) * (a + p));
    k * (i0 + alpha * i2)
}

/// μ of a Henyey–Greenstein direction from a draw in [0, 1), by the inversion of SREP 79 rule 6.
pub fn sample_hg(u: f64, g: f64) -> f64 {
    if g.abs() < HG_ISOTROPIC {
        return 1.0 - 2.0 * u;
    }
    let s = (1.0 - g * g) / (1.0 - g + 2.0 * g * u);
    ((1.0 + g * g - s * s) / (2.0 * g)).clamp(-1.0, 1.0)
}

/// μ of a Draine direction from a draw in [0, 1): the μ with [`cdf_draine`] = u, by bisection of [−1, 1].
pub fn sample_draine(u: f64, g: f64, alpha: f64) -> f64 {
    let (mut low, mut high) = (-1.0, 1.0);
    for _ in 0..BISECTION_STEPS {
        let mid = 0.5 * (low + high);
        if cdf_draine(mid, g, alpha) < u {
            low = mid;
        } else {
            high = mid;
        }
    }
    0.5 * (low + high)
}

/// μ of a fog direction: the Draine lobe when `lobe` < w_D, else the HG lobe, each inverting `u` (rule 20).
pub fn sample_fog(lobe: f64, u: f64, diameter: f64) -> f64 {
    let p = fog_parameters(diameter);
    if lobe < p.w_d {
        sample_draine(u, p.g_d, p.alpha)
    } else {
        sample_hg(u, p.g_hg)
    }
}

/// Normalized Henyey–Greenstein phase. Directions follow propagation, so g>0
/// peaks at cos(theta)=1. Callers supply finite cosine and |g|<1.
pub fn phase_hg(cos_theta: f64, g: f64) -> f64 {
    let denominator = 1.0 + g * g - 2.0 * g * cos_theta.clamp(-1.0, 1.0);
    (1.0 - g * g) / (4.0 * std::f64::consts::PI * denominator.powf(1.5))
}

/// Integrates emission, absorption and single scattering across all overlapping media.
/// `incident` returns phase-weighted, visibility-attenuated incident RGB at a point,
/// given the outgoing propagation direction (toward the viewer) and the medium's phase function.
pub fn integrate(
    media: &[Medium],
    ray: Ray,
    march: March,
    mut incident: impl FnMut([f64; 3], [f64; 3], Phase) -> [f64; 3],
) -> Result<Transport, Error> {
    if !march.step_size.is_finite() || march.step_size <= 0.0 || march.max_steps == 0 {
        return Err(Error::Invalid("march requires positive finite step size and positive step budget"));
    }
    // Each domain boundary splits quadrature intervals, avoiding skipped thin layers
    // and expensive sampling of arbitrarily large empty gaps between media.
    let ranges: Vec<_> = media.iter().filter_map(|m| m.interval(ray)).collect();
    let mut edges: Vec<_> = ranges.iter().flatten().copied().collect();
    edges.sort_by(f64::total_cmp);
    edges.dedup();
    let mut segments = Vec::new();
    let mut required = 0u32;
    for edge in edges.windows(2) {
        let [a, b] = [edge[0], edge[1]];
        let middle = a + (b - a) * 0.5;
        if !ranges.iter().any(|r| middle >= r[0] && middle < r[1]) {
            continue;
        }
        let n = ((b - a) / march.step_size).ceil();
        if n > f64::from(march.max_steps - required) {
            return Err(Error::Limit("medium ray exceeds sample budget; increase maxSteps or stepSize"));
        }
        let n = (n as u32).max(1);
        required = required.checked_add(n).filter(|n| *n <= march.max_steps).ok_or(Error::Limit("medium samples"))?;
        segments.push((a, b, n));
    }
    let mut out = Transport { radiance: [0.0; 3], transmittance: 1.0, steps: required };
    for (a, b, n) in segments {
        let ds = (b - a) / f64::from(n);
        for k in 0..n {
            let point = (ray.origin + ray.direction * (a + (f64::from(k) + 0.5) * ds)).to_array();
            let mut extinction = 0.0;
            let mut source = [0.0; 3];
            for medium in media {
                let density = medium.sample_density(point);
                if density == 0.0 {
                    continue;
                }
                let optical = medium.optical;
                let sigma = density * optical.extinction;
                extinction += sigma;
                let light = if optical.scatter_bounces == 0 || optical.albedo == [0.0; 3] {
                    [0.0; 3]
                } else {
                    incident(point, (-ray.direction).to_array(), optical.phase)
                };
                if light.iter().any(|v| !v.is_finite() || *v < 0.0) {
                    return Err(Error::Invalid("incident radiance must be finite and nonnegative"));
                }
                let emission = medium.sample_emission(point);
                for c in 0..3 {
                    source[c] += density * emission[c] + sigma * optical.albedo[c] * light[c];
                }
            }
            if !extinction.is_finite() || source.iter().any(|v| !v.is_finite()) {
                return Err(Error::Invalid("medium coefficient overflow"));
            }
            let lost = -(-extinction * ds).exp_m1();
            let weight = out.transmittance * if extinction == 0.0 { ds } else { lost / extinction };
            for (dst, src) in out.radiance.iter_mut().zip(source) {
                *dst += weight * src;
            }
            out.transmittance *= (-extinction * ds).exp();
        }
    }
    if out.radiance.iter().any(|v| !v.is_finite()) {
        return Err(Error::Invalid("integrated radiance overflow"));
    }
    Ok(out)
}
