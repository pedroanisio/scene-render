//! Rules 10 and 11 of the sr-core physical-sky draft (`srep/srep-0000-physical-sky.md`) as the renderer applies them
//! to surfaces: a per-camera volume of [`crate::atmosphere`]'s march, held to `aerial_perspective`, and a table of
//! the fog's horizon radiance over the azimuth, held to `horizon_radiance`.
//!
//! The volume's columns are the camera's directions at the screen points (i / (W − 1), j / (H − 1)), corners
//! included, and its slices the distances `max_distance` × (k / D)², k = 0 to D, so the near field gets the most
//! slices; slice 0 is the camera, where a surface keeps its colour. Every node is the march itself, run once per column
//! through every slice; between nodes the volume is trilinear in (u, v, √(s / max_distance)), as a GPU samples it.

use std::f64::consts::PI;

use rayon::prelude::*;

use crate::atmosphere::{atmosphere_position, march, Fog, Scattering, SkyError};
use crate::physical_sky::to_planet;

type V3 = [f64; 3];

/// The largest side of the volume.
pub const MAX_SIDE: usize = 4096;

/// The horizon table's azimuths over the full turn.
pub const HORIZON_AZIMUTHS: usize = 360;

/// The in-scattered radiance and the transmittance from one camera to every distance in every direction it sees.
#[derive(Debug, Clone, PartialEq)]
pub struct AerialVolume {
    /// Columns across, columns up and slices beyond the camera's.
    size: [usize; 3],
    /// Metres to the last slice.
    max_distance: f64,
    /// Per node, x fastest, then y, then the slice: the radiance (rgb, alpha 1).
    inscatter: Vec<[f32; 4]>,
    /// Per node: the transmittance (rgb, alpha 1).
    transmittance: Vec<[f32; 4]>,
}

impl AerialVolume {
    /// The volume of `s` along the rays `ray(u, v)` = (origin, unit direction) of a camera (planet frame, metres) for
    /// the unit `sun` and a `fog` whose source is the horizon radiance at the dome's `observer` (on the z axis), over
    /// `max_distance` metres in `size` = [columns across (≥ 2), columns up (≥ 2), slices (≥ 1)].
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        s: &Scattering,
        ray: &(dyn Fn(f64, f64) -> (V3, V3) + Sync),
        observer: V3,
        sun: V3,
        fog: Option<&Fog>,
        max_distance: f64,
        size: [usize; 3],
    ) -> Result<AerialVolume, SkyError> {
        let [w, h, slices] = size;
        if w < 2 || h < 2 || slices < 1 || size.iter().any(|n| *n > MAX_SIDE) {
            return Err(SkyError(format!("the volume is 2 to {MAX_SIDE} columns each way and 1 to {MAX_SIDE} slices")));
        }
        if !(max_distance > 0.0 && max_distance.is_finite()) {
            return Err(SkyError("the volume's depth is a positive number of metres".into()));
        }
        if observer[0] != 0.0 || observer[1] != 0.0 {
            return Err(SkyError("the horizon is evaluated at an observer on the z axis".into()));
        }
        let distances: Vec<f64> = (0..=slices).map(|k| max_distance * (k as f64 / slices as f64).powi(2)).collect();
        let columns: Vec<Vec<(V3, V3)>> = (0..w * h)
            .into_par_iter()
            .map(|n| {
                let (camera, d) = ray((n % w) as f64 / (w - 1) as f64, (n / w) as f64 / (h - 1) as f64);
                column(s, camera, observer, d, sun, fog, &distances)
            })
            .collect::<Result<_, _>>()?;
        let cells = w * h * (slices + 1);
        let (mut inscatter, mut transmittance) = (vec![[0.0; 4]; cells], vec![[0.0; 4]; cells]);
        for (n, col) in columns.iter().enumerate() {
            for (k, (l, t)) in col.iter().enumerate() {
                let i = k * w * h + n;
                inscatter[i] = [l[0] as f32, l[1] as f32, l[2] as f32, 1.0];
                transmittance[i] = [t[0] as f32, t[1] as f32, t[2] as f32, 1.0];
            }
        }
        Ok(AerialVolume { size: [w, h, slices + 1], max_distance, inscatter, transmittance })
    }

    /// Texels across, up and deep (the camera's slice included).
    pub fn size(&self) -> [usize; 3] {
        self.size
    }

    /// Metres to the last slice.
    pub fn max_distance(&self) -> f64 {
        self.max_distance
    }

    /// The radiance texels, x fastest.
    pub fn inscatter(&self) -> &[[f32; 4]] {
        &self.inscatter
    }

    /// The transmittance texels, x fastest.
    pub fn transmittance(&self) -> &[[f32; 4]] {
        &self.transmittance
    }

    /// A surface of `colour` at the screen point (u, v) and `distance` metres, seen through the volume: the colour
    /// times the transmittance plus the radiance, trilinear between the nodes and clamped to the last slice.
    pub fn apply(&self, u: f64, v: f64, distance: f64, colour: V3) -> V3 {
        let [w, h, n] = self.size;
        let z = (distance.max(0.0) / self.max_distance).min(1.0).sqrt() * (n - 1) as f64;
        let axes = [u.clamp(0.0, 1.0) * (w - 1) as f64, v.clamp(0.0, 1.0) * (h - 1) as f64, z];
        let (mut l, mut t) = ([0.0; 3], [0.0; 3]);
        for corner in 0..8 {
            let mut index = 0;
            let mut weight = 1.0;
            for (axis, (x, stride)) in axes.iter().zip([1, w, w * h]).enumerate() {
                let side = [w, h, n][axis];
                let lo = (x.floor() as usize).min(side - 1);
                let f = x - lo as f64;
                let hi = corner >> axis & 1 == 1;
                index += (if hi { (lo + 1).min(side - 1) } else { lo }) * stride;
                weight *= if hi { f } else { 1.0 - f };
            }
            if weight == 0.0 {
                continue;
            }
            for c in 0..3 {
                l[c] += weight * self.inscatter[index][c] as f64;
                t[c] += weight * self.transmittance[index][c] as f64;
            }
        }
        std::array::from_fn(|c| colour[c] * t[c] + l[c])
    }
}

/// One column: the march's running radiance and transmittance at every distance from the camera.
fn column(
    s: &Scattering,
    camera: V3,
    observer: V3,
    d: V3,
    sun: V3,
    fog: Option<&Fog>,
    distances: &[f64],
) -> Result<Vec<(V3, V3)>, SkyError> {
    if !camera.iter().chain(&d).all(|c| c.is_finite()) {
        return Err(SkyError("a camera ray is finite".into()));
    }
    let (start, total, _) = s.atmosphere.path_end(camera, d)?;
    let fog = match fog {
        Some(f) => Some((f, s.horizon_radiance(observer, d, sun)?)),
        None => None,
    };
    // the distances from the atmosphere's near end, held within the ray's length in it
    let ends: Vec<f64> = distances.iter().map(|x| (x - start).clamp(0.0, total.max(0.0))).collect();
    let mut out = vec![([0.0; 3], [1.0; 3]); distances.len()];
    s.integrate(march(camera, d, start), d, sun, &ends, true, fog, |i, l, t| out[i + 1] = (l, t))?;
    Ok(out)
}

/// Rule 11's L_h over the azimuth at the dome's observer: `HORIZON_AZIMUTHS` nodes of `horizon_radiance`, linear
/// between them.
#[derive(Debug, Clone, PartialEq)]
pub struct HorizonTable {
    table: Vec<V3>,
}

impl HorizonTable {
    /// The table of `s` for the `observer` on the z axis and the unit `sun` (planet frame).
    pub fn new(s: &Scattering, observer: V3, sun: V3) -> Result<HorizonTable, SkyError> {
        if observer[0] != 0.0 || observer[1] != 0.0 {
            return Err(SkyError("the horizon is evaluated at an observer on the z axis".into()));
        }
        let table = (0..HORIZON_AZIMUTHS)
            .into_par_iter()
            .map(|k| {
                let a = 2.0 * PI * k as f64 / HORIZON_AZIMUTHS as f64;
                s.horizon_radiance(observer, [a.cos(), a.sin(), 0.0], sun)
            })
            .collect::<Result<_, _>>()?;
        Ok(HorizonTable { table })
    }

    /// The nodes, the azimuth 2πk / `HORIZON_AZIMUTHS` from +x toward +y.
    pub fn nodes(&self) -> &[V3] {
        &self.table
    }

    /// L_h in d's azimuth (planet frame); a vertical d takes azimuth +x.
    pub fn radiance(&self, d: V3) -> V3 {
        let n = d[0].hypot(d[1]);
        let a = if n < 1e-12 { 0.0 } else { d[1].atan2(d[0]).rem_euclid(2.0 * PI) };
        let x = a / (2.0 * PI) * HORIZON_AZIMUTHS as f64;
        let k = (x.floor() as usize).min(HORIZON_AZIMUTHS - 1);
        let f = x - k as f64;
        let (p, q) = (self.table[k], self.table[(k + 1) % HORIZON_AZIMUTHS]);
        std::array::from_fn(|c| p[c] * (1.0 - f) + q[c] * f)
    }
}

/// A dome's physical sky as rules 10 and 11 apply it to surfaces: its atmosphere and sun, the march when
/// `aerialPerspective="true"` over air, the fog, and the scene's scale and origin (`physics/@pixelsPerMeter`, the
/// observer at `altitude` at the origin).
#[derive(Debug, Clone)]
pub struct AerialSky {
    scattering: Scattering,
    altitude: f64,
    /// The sun in the planet frame.
    sun: V3,
    march: bool,
    fog: Option<Fog>,
    pixels_per_meter: f64,
    horizon: HorizonTable,
}

impl AerialSky {
    /// The sky of `s` for an observer `altitude` metres up and the unit `sun` of the dome's axes, marching when
    /// `march` and fogged by `fog`; none when it has neither a march nor a fog of some density.
    pub fn new(
        s: Scattering,
        altitude: f64,
        sun: V3,
        march: bool,
        fog: Option<Fog>,
        pixels_per_meter: f64,
    ) -> Result<Option<AerialSky>, SkyError> {
        if !(pixels_per_meter > 0.0 && pixels_per_meter.is_finite()) {
            return Err(SkyError("pixelsPerMeter is a positive number".into()));
        }
        if !(altitude >= 0.0 && altitude.is_finite()) {
            return Err(SkyError("the observer's altitude is finite and not below the ground".into()));
        }
        let fog = fog.filter(|f| f.density > 0.0);
        if !march && fog.is_none() {
            return Ok(None);
        }
        let sun = to_planet(sun);
        let horizon = HorizonTable::new(&s, [0.0, 0.0, s.atmosphere.planet_radius + altitude], sun)?;
        Ok(Some(AerialSky { scattering: s, altitude, sun, march, fog, pixels_per_meter, horizon }))
    }

    /// The volume along the rays `ray(u, v)` = (origin in scene pixels, direction) of a camera in the dome's axes, to
    /// `max_distance` scene pixels: none for the fog's closed form.
    pub fn volume(
        &self,
        ray: &(dyn Fn(f64, f64) -> (V3, V3) + Sync),
        max_distance: f64,
        size: [usize; 3],
    ) -> Result<Option<AerialVolume>, SkyError> {
        if !self.march {
            return Ok(None);
        }
        let (a, k) = (&self.scattering.atmosphere, self.pixels_per_meter);
        // scene points are refused here, unmapped, by the first column that meets one
        let mapped = |u: f64, v: f64| {
            let (o, d) = ray(u, v);
            let p = atmosphere_position(a, o, self.altitude, k).unwrap_or([f64::NAN; 3]);
            (p, to_planet(d))
        };
        let observer = [0.0, 0.0, a.planet_radius + self.altitude];
        AerialVolume::new(&self.scattering, &mapped, observer, self.sun, self.fog.as_ref(), max_distance / k, size)
            .map(Some)
    }

    /// The fog's horizon table at the observer.
    pub fn horizon(&self) -> &HorizonTable {
        &self.horizon
    }

    /// The renderer's uniforms for a volume `max_distance` metres deep: [mode (1 the march, 2 the closed form), pixels
    /// per metre, altitude, depth], [fog density, fog height, horizon nodes, 0], with no fog of density 0 and height
    /// 100.
    pub fn uniforms(&self, max_distance: f64) -> [[f32; 4]; 2] {
        let mode = if self.march { 1.0 } else { 2.0 };
        let (density, height) = self.fog.map_or((0.0, 100.0), |f| (f.density, f.height));
        [
            [mode, self.pixels_per_meter as f32, self.altitude as f32, max_distance as f32],
            [density as f32, height as f32, HORIZON_AZIMUTHS as f32, 0.0],
        ]
    }
}
