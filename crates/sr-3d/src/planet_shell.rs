//! Rule 6 of the sr-core planets draft (`srep/srep-0000-planets.md`): a planet's atmosphere shell, the physical-sky
//! draft's atmosphere about the body's centre, as the renderer draws it for one observer.
//!
//! Rays that miss the planet take a table of rule 9's radiance over the elevation from the horizon and the azimuth from
//! the sun's vertical, as [`crate::physical_sky::SkyView`]'s, and a table of the transmittance to space over the same
//! rows; surfaces take [`crate::aerial::AerialVolume`]'s march. Both are in the observer's frame, its up the direction
//! from the centre. The rows run from the horizon to the elevation that grazes the shell's top (to the zenith from
//! inside the air), spaced by the square of their index, so that from orbit every row is in the band the light is in.
//! A ray that passes above the shell's top meets no air: its radiance is zero and its transmittance one exactly.
//!
//! An orthographic camera's rays share a direction and no point: [`PlanetShell::parallel`] tabulates them by where
//! they cross the plane across that direction, their impact radius about the centre from the limb to the top
//! (spaced alike) and their azimuth about the view's axis from the sun's side; every point of a ray reads the same.

use std::f64::consts::{FRAC_PI_2, PI};

use rayon::prelude::*;

use crate::aerial::AerialVolume;
use crate::atmosphere::{atmosphere, unit, AtmosphereOptions, Scattering, SkyError};

/// The table's elevations, the horizon to the top's grazing elevation.
pub const SHELL_ELEVATIONS: usize = 256;
/// The table's azimuths, 0 to 180° from the sun's vertical (the sky is symmetric about it).
pub const SHELL_AZIMUTHS: usize = 96;

type V3 = [f64; 3];

/// The vec4 rows of [`PlanetShell::table`] ahead of its nodes.
pub const SHELL_HEADER: usize = 5;

/// One observer's view of a planet's atmosphere shell.
#[derive(Debug, Clone)]
pub struct PlanetShell {
    scattering: Scattering,
    /// Rows: the observer frame's x, y and up in the caller's frame (up toward the observer of a parallel view).
    to_local: [V3; 3],
    /// The observer in its frame, metres from the centre.
    observer: V3,
    /// Rays that share a direction, the observer's -up, tabulated by their impact radius; else rays from the observer
    /// tabulated by their elevation.
    parallel: bool,
    /// The sun in the observer's frame (its up when there is none).
    sun: V3,
    /// The rows' first coordinate: the elevation of the planet's limb (the geometric horizon), radians; the planet's
    /// radius in metres for a parallel view.
    low: f64,
    /// The rows' last: the elevation that grazes the shell's top, radians, π/2 from inside the shell; the top's
    /// radius for a parallel view.
    high: f64,
    /// The sun's azimuth about the observer's up, radians: the table's column 0.
    sun_azimuth: f64,
    /// `SHELL_ELEVATIONS` rows of `SHELL_AZIMUTHS` radiances, the horizon's row first.
    radiance: Vec<V3>,
    /// The transmittance to space along every row.
    transmittance: Vec<V3>,
}

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

impl PlanetShell {
    /// The shell of a planet of `radius` metres under air `height` metres deep, seen from `observer` (metres from the
    /// centre, any frame) and lit by the unit `sun` of that frame with `irradiance`; without a sun it scatters nothing.
    pub fn new(
        radius: f64,
        height: f64,
        observer: V3,
        sun: Option<V3>,
        irradiance: V3,
    ) -> Result<PlanetShell, SkyError> {
        if !observer.iter().all(|c| c.is_finite()) {
            return Err(SkyError("the observer is a finite point".into()));
        }
        let up = unit(observer).map_err(|_| SkyError("the observer is off the planet's centre".into()))?;
        PlanetShell::build(radius, height, observer, up, false, sun, irradiance)
    }

    /// The shell seen along the unit `view` by rays that start in the plane across it through `observer`, as an
    /// orthographic camera's do; that plane lies past the shell's top on the observer's side.
    pub fn parallel(
        radius: f64,
        height: f64,
        observer: V3,
        view: V3,
        sun: Option<V3>,
        irradiance: V3,
    ) -> Result<PlanetShell, SkyError> {
        if !observer.iter().chain(&view).all(|c| c.is_finite()) || (dot(view, view).sqrt() - 1.0).abs() >= 1e-6 {
            return Err(SkyError("the view is a unit direction from a finite point".into()));
        }
        if -dot(observer, view) <= radius + height {
            return Err(SkyError("the rays of a parallel view start past the shell's top, toward it".into()));
        }
        PlanetShell::build(radius, height, observer, view.map(|c| -c), true, sun, irradiance)
    }

    fn build(
        radius: f64,
        height: f64,
        observer: V3,
        up: V3,
        parallel: bool,
        sun: Option<V3>,
        irradiance: V3,
    ) -> Result<PlanetShell, SkyError> {
        let distance = dot(observer, observer).sqrt();
        let o = AtmosphereOptions {
            planet_radius: radius,
            top_radius: radius + height,
            sun_irradiance: if sun.is_some() { irradiance } else { [0.0; 3] },
            ..Default::default()
        };
        if !irradiance.iter().all(|c| *c >= 0.0 && c.is_finite()) {
            return Err(SkyError("the sun's irradiance is finite and non-negative".into()));
        }
        let scattering = Scattering::new(atmosphere(&o)?);
        let a = if up[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
        let x = unit(cross(a, up))?;
        let to_local = [x, cross(up, x), up];
        let sun = match sun {
            Some(s) => {
                if (dot(s, s).sqrt() - 1.0).abs() >= 1e-6 {
                    return Err(SkyError("the sun's direction is a unit".into()));
                }
                to_local.map(|r| dot(r, s))
            }
            None => [0.0, 0.0, 1.0],
        };
        let top = radius + height;
        let (low, high) = if parallel {
            (radius, top)
        } else {
            let d = distance.max(radius);
            (-(radius / d).acos(), if distance > top { -(top / distance).acos() } else { FRAC_PI_2 })
        };
        let sun_azimuth = sun[1].atan2(sun[0]);
        let shell = PlanetShell {
            scattering,
            observer: to_local.map(|r| dot(r, observer)),
            to_local,
            parallel,
            sun,
            low,
            high,
            sun_azimuth,
            radiance: Vec::new(),
            transmittance: Vec::new(),
        };
        let (s, view) = (&shell.scattering, &shell);
        let radiance = (0..SHELL_ELEVATIONS * SHELL_AZIMUTHS)
            .into_par_iter()
            .map(|k| {
                let (o, d) = view.node(k / SHELL_AZIMUTHS, k % SHELL_AZIMUTHS);
                s.sky_radiance(o, d, sun)
            })
            .collect::<Result<_, _>>()?;
        let transmittance = (0..SHELL_ELEVATIONS)
            .into_par_iter()
            .map(|i| {
                // the air is the same at every azimuth: one fixed, so that the sun has no part in it
                let (o, d) = view.node_at(i, 0.0);
                s.atmosphere.transmittance(o, d)
            })
            .collect::<Result<_, _>>()?;
        Ok(PlanetShell { radiance, transmittance, ..shell })
    }

    /// The coordinate of row `i`: the low end, nudged off the tangent of the ground, plus the square of its index.
    fn row_coordinate(&self, i: usize) -> f64 {
        let t = i as f64 / (SHELL_ELEVATIONS - 1) as f64;
        let e = self.low + (self.high - self.low) * t * t;
        if i == 0 {
            e + if self.parallel { 1e-3 } else { 1e-9 }
        } else {
            e
        }
    }

    /// The ray of node (i, j), observer's frame: its origin, past the top for a parallel view, and unit direction.
    fn node(&self, i: usize, j: usize) -> (V3, V3) {
        self.node_at(i, self.sun_azimuth + PI * j as f64 / (SHELL_AZIMUTHS - 1) as f64)
    }

    /// The ray of row i at the azimuth `a`, radians.
    fn node_at(&self, i: usize, a: f64) -> (V3, V3) {
        let e = self.row_coordinate(i);
        if self.parallel {
            ([e * a.cos(), e * a.sin(), self.high + 1_000.0], [0.0, 0.0, -1.0])
        } else {
            ([0.0, 0.0, self.observer_distance()], [e.cos() * a.cos(), e.cos() * a.sin(), e.sin()])
        }
    }

    /// The perspective observer's distance from the centre, at least the planet's radius.
    fn observer_distance(&self) -> f64 {
        dot(self.observer, self.observer).sqrt().max(self.scattering.atmosphere.planet_radius)
    }

    /// The fractional row of coordinate e, none outside the rows.
    fn row(&self, e: f64) -> Option<f64> {
        if e < self.low || e > self.high {
            return None;
        }
        let t = ((e - self.low) / (self.high - self.low)).clamp(0.0, 1.0).sqrt();
        Some(t * (SHELL_ELEVATIONS - 1) as f64)
    }

    fn local(&self, d: V3) -> V3 {
        self.to_local.map(|r| dot(r, d))
    }

    /// The row coordinate of the ray from `origin` (caller's frame, metres from the centre) along the unit d, and its
    /// direction (perspective) or point (parallel) in the observer's frame, whose azimuth the columns read.
    fn coordinate(&self, origin: V3, d: V3) -> (f64, V3) {
        if self.parallel {
            let l = self.local(origin);
            ((l[0] * l[0] + l[1] * l[1]).sqrt(), l)
        } else {
            let l = self.local(d);
            (l[2].clamp(-1.0, 1.0).asin(), l)
        }
    }

    /// Rule 9's radiance along the unit d (caller's frame) of a ray from the observer that meets nothing else: zero
    /// toward the planet and past the shell's top; bilinear between the nodes.
    pub fn radiance(&self, d: V3) -> V3 {
        self.radiance_of(self.coordinate([0.0; 3], d))
    }

    /// The transmittance to space along the unit d (caller's frame) from the observer: zero toward the planet, one
    /// past the shell's top; linear between the rows.
    pub fn transmittance(&self, d: V3) -> V3 {
        self.transmittance_of(self.coordinate([0.0; 3], d).0)
    }

    /// A parallel view's radiance along the ray through `origin` (caller's frame, metres from the centre).
    pub fn radiance_on(&self, origin: V3) -> V3 {
        self.radiance_of(self.coordinate(origin, [0.0; 3]))
    }

    /// A parallel view's transmittance along the ray through `origin`.
    pub fn transmittance_on(&self, origin: V3) -> V3 {
        self.transmittance_of(self.coordinate(origin, [0.0; 3]).0)
    }

    fn radiance_of(&self, (e, l): (f64, V3)) -> V3 {
        let Some(fi) = self.row(e) else {
            return [0.0; 3];
        };
        let a = if l[0] == 0.0 && l[1] == 0.0 { 0.0 } else { l[1].atan2(l[0]) - self.sun_azimuth };
        // the angle from the sun's side, 0 to π
        let a = a.rem_euclid(2.0 * PI);
        let a = if a > PI { 2.0 * PI - a } else { a };
        let fj = a / PI * (SHELL_AZIMUTHS - 1) as f64;
        let (i0, j0) = ((fi.floor() as usize).min(SHELL_ELEVATIONS - 2), (fj.floor() as usize).min(SHELL_AZIMUTHS - 2));
        let (ti, tj) = (fi - i0 as f64, fj - j0 as f64);
        let mut out = [0.0; 3];
        for (i, wi) in [(i0, 1.0 - ti), (i0 + 1, ti)] {
            for (j, wj) in [(j0, 1.0 - tj), (j0 + 1, tj)] {
                let l = self.radiance[i * SHELL_AZIMUTHS + j];
                for c in 0..3 {
                    out[c] += wi * wj * l[c];
                }
            }
        }
        out
    }

    fn transmittance_of(&self, e: f64) -> V3 {
        if e > self.high {
            return [1.0; 3];
        }
        let Some(f) = self.row(e) else {
            return [0.0; 3];
        };
        let i = (f.floor() as usize).min(SHELL_ELEVATIONS - 2);
        let w = f - i as f64;
        let (a, b) = (self.transmittance[i], self.transmittance[i + 1]);
        std::array::from_fn(|c| a[c] * (1.0 - w) + b[c] * w)
    }

    /// The march to the surfaces along the rays `ray(u, v)` = (origin in metres from the centre, unit direction) of
    /// the caller's frame, over `max_distance` metres in `size` ([`AerialVolume::new`]).
    pub fn volume(
        &self,
        ray: &(dyn Fn(f64, f64) -> (V3, V3) + Sync),
        max_distance: f64,
        size: [usize; 3],
    ) -> Result<AerialVolume, SkyError> {
        let mapped = |u: f64, v: f64| {
            let (o, d) = ray(u, v);
            (self.local(o), self.local(d))
        };
        AerialVolume::new(
            &self.scattering,
            &mapped,
            [0.0, 0.0, self.observer_distance()],
            self.sun,
            None,
            max_distance,
            size,
        )
    }

    /// What a shader reads, as vec4 rows: the rotation into the observer's frame (rows x, y, up; w the rows' first and
    /// last coordinates and the sun's azimuth), then [rows, columns, 1 for a parallel view else 0, 0], the observer in
    /// its frame (metres), then the radiance nodes row by row and the transmittance of every row.
    pub fn table(&self) -> Vec<[f32; 4]> {
        let v = |a: V3, w: f64| [a[0] as f32, a[1] as f32, a[2] as f32, w as f32];
        let mut out = vec![
            v(self.to_local[0], self.low),
            v(self.to_local[1], self.high),
            v(self.to_local[2], self.sun_azimuth),
            [SHELL_ELEVATIONS as f32, SHELL_AZIMUTHS as f32, f32::from(u8::from(self.parallel)), 0.0],
            v(self.observer, 0.0),
        ];
        out.extend(self.radiance.iter().map(|l| v(*l, 0.0)));
        out.extend(self.transmittance.iter().map(|t| v(*t, 0.0)));
        out
    }
}
