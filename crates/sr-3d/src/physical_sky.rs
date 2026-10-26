//! The sky of a dome light with `sky="physical"` as the renderer draws it: rule 9's radiance of the sr-core
//! physical-sky draft (`srep/srep-0000-physical-sky.md`) tabulated over the directions an observer sees, Hillaire's
//! (2020) sky-view table. [`crate::atmosphere`] holds the integrals; the table is held to them.
//!
//! The table's columns are the azimuth from the sun's vertical, 0 to 180° (the sky is symmetric about it), and its
//! rows the elevation from the geometric horizon to the zenith, spaced by the square of their index so that the
//! horizon's band gets the most rows. Every node is the integral itself; between nodes the table is bilinear. The
//! zenith is a node. Below the horizon every direction meets the planet, and rule 9's radiance there is zero.

use std::f64::consts::{FRAC_PI_2, PI};

use glam::Vec3;
use rayon::prelude::*;

use crate::atmosphere::{atmosphere, AtmosphereOptions, Scattering, SkyError};
use crate::env::{dir_of, Equirect};
use crate::sky::add_disc;
use crate::space::moon::moon_from_document;
use crate::space::sun::{sun_from_document, Observer};
use crate::space::SpaceError;

/// The table's azimuths, 0 to 180° from the sun.
pub const SKY_VIEW_AZIMUTHS: usize = 96;
/// The table's elevations, the horizon to the zenith.
pub const SKY_VIEW_ELEVATIONS: usize = 128;

type V3 = [f64; 3];

/// A direction of SREP 71's scene axes (up −y, azimuth 0 along +z, azimuth 90° along +x) in the planet frame of
/// [`crate::atmosphere`] (up +z): a rotation, (x, y, z) to (x, z, −y).
pub fn to_planet(d: V3) -> V3 {
    [d[0], d[2], -d[1]]
}

/// Rule 9's sky over every direction of one observer and one sun.
#[derive(Debug, Clone, PartialEq)]
pub struct SkyView {
    /// The elevation of the geometric horizon, radians: 0 on the ground, the limb's depression above it.
    horizon: f64,
    /// The sun's azimuth about the observer's up, radians.
    sun_azimuth: f64,
    /// `SKY_VIEW_ELEVATIONS` rows of `SKY_VIEW_AZIMUTHS`, the horizon's row first.
    table: Vec<V3>,
}

impl SkyView {
    /// The table of `s` for an observer `altitude` metres above the ground and the unit `sun` of the planet frame.
    pub fn new(s: &Scattering, altitude: f64, sun: V3) -> Result<SkyView, SkyError> {
        if !(altitude >= 0.0 && altitude.is_finite()) {
            return Err(SkyError("the observer's altitude is finite and not below the ground".into()));
        }
        let n = (sun[0] * sun[0] + sun[1] * sun[1] + sun[2] * sun[2]).sqrt();
        if (n - 1.0).abs() >= 1e-6 || n.is_nan() {
            return Err(SkyError("the sun's direction is a unit".into()));
        }
        let r = s.atmosphere.planet_radius;
        let obs = [0.0, 0.0, r + altitude];
        let horizon = -(r / (r + altitude)).acos();
        let sun_azimuth = sun[1].atan2(sun[0]);
        let v = SkyView { horizon, sun_azimuth, table: Vec::new() };
        let table = (0..SKY_VIEW_ELEVATIONS * SKY_VIEW_AZIMUTHS)
            .into_par_iter()
            .map(|k| {
                let (i, j) = (k / SKY_VIEW_AZIMUTHS, k % SKY_VIEW_AZIMUTHS);
                let d = v.node(i, j);
                s.sky_radiance(obs, d, sun)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(SkyView { table, ..v })
    }

    /// The elevation of the geometric horizon, radians.
    pub fn horizon(&self) -> f64 {
        self.horizon
    }

    /// The table's number of nodes.
    pub fn len(&self) -> usize {
        self.table.len()
    }

    /// Whether the table has no nodes (never, once built).
    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    /// The elevation of row `i`: the horizon, nudged off the tangent of the ground, plus the square of its index.
    fn elevation(&self, i: usize) -> f64 {
        let t = i as f64 / (SKY_VIEW_ELEVATIONS - 1) as f64;
        let e = self.horizon + (FRAC_PI_2 - self.horizon) * t * t;
        if i == 0 && self.horizon < 0.0 {
            e + 1e-6
        } else {
            e
        }
    }

    /// The unit direction of node (i, j).
    fn node(&self, i: usize, j: usize) -> V3 {
        let e = self.elevation(i);
        let a = self.sun_azimuth + PI * j as f64 / (SKY_VIEW_AZIMUTHS - 1) as f64;
        if i == SKY_VIEW_ELEVATIONS - 1 {
            return [0.0, 0.0, 1.0];
        }
        [e.cos() * a.cos(), e.cos() * a.sin(), e.sin()]
    }

    /// The sky's radiance in the unit direction d of the planet frame: bilinear in the table, zero below the horizon.
    pub fn radiance(&self, d: V3) -> V3 {
        let e = d[2].clamp(-1.0, 1.0).asin();
        if e < self.horizon {
            return [0.0; 3];
        }
        let t = ((e - self.horizon) / (FRAC_PI_2 - self.horizon)).clamp(0.0, 1.0).sqrt();
        let fi = t * (SKY_VIEW_ELEVATIONS - 1) as f64;
        let a = if d[0] == 0.0 && d[1] == 0.0 { 0.0 } else { d[1].atan2(d[0]) - self.sun_azimuth };
        // the angle from the sun's vertical, 0 to π
        let a = a.rem_euclid(2.0 * PI);
        let a = if a > PI { 2.0 * PI - a } else { a };
        let fj = a / PI * (SKY_VIEW_AZIMUTHS - 1) as f64;
        let (i0, j0) = (fi.floor() as usize, fj.floor() as usize);
        let (i1, j1) = ((i0 + 1).min(SKY_VIEW_ELEVATIONS - 1), (j0 + 1).min(SKY_VIEW_AZIMUTHS - 1));
        let (ti, tj) = (fi - i0 as f64, fj - j0 as f64);
        let mut out = [0.0; 3];
        for (i, wi) in [(i0, 1.0 - ti), (i1, ti)] {
            for (j, wj) in [(j0, 1.0 - tj), (j1, tj)] {
                let w = wi * wj;
                if w == 0.0 {
                    continue;
                }
                let l = self.table[i * SKY_VIEW_AZIMUTHS + j];
                for c in 0..3 {
                    out[c] += w * l[c];
                }
            }
        }
        out
    }
}

/// A dome's physical sky in SREP 71's dome axes (up −y): the sky-view table, the sun's disc (rule 6) and the moon's
/// (rule 7), in linear working values.
#[derive(Debug, Clone, PartialEq)]
pub struct PhysicalSky {
    view: SkyView,
    /// Unit direction toward the sun's centre.
    sun_dir: [f32; 3],
    /// Cosine of half the disc's angular diameter.
    sun_cos: f32,
    /// The disc's radiance: the transmitted irradiance over the disc's solid angle, tinted; zero for no disc.
    sun: [f32; 3],
    /// Unit direction toward the moon's centre.
    moon_dir: [f32; 3],
    /// Cosine of half the moon's angular diameter; above 1 for no moon.
    moon_cos: f32,
    /// The moon disc's radiance; zero for no moon.
    moon: [f32; 3],
}

/// A dome light's physical sky as the document writes it.
#[derive(Debug, Clone, PartialEq)]
pub struct PhysicalSkyOptions {
    /// `date` at this frame.
    pub date: String,
    /// `latitude`, `longitude`, `altitude`, `pressure`, `temperature` and `deltaT`.
    pub observer: Observer,
    /// The atmosphere's knobs and the sun's effective irradiance ([`atmosphere_options`]).
    pub atmosphere: AtmosphereOptions,
    /// `sunSize`, degrees.
    pub sun_size: f64,
    /// `sunColor`, tinting the disc alone.
    pub sun_color: [f64; 3],
    /// `moon`.
    pub moon: bool,
    /// `moonIntensity`.
    pub moon_intensity: f64,
}

/// Rule 8's knobs as the document writes them: `atmosphere="earth"` or `"none"` (no layers), `aerosol`, `ozone`,
/// `planetRadius`, `atmosphereHeight`, `groundAlbedo`, and the sun's effective irradiance
/// ([`crate::atmosphere::effective_sun_irradiance`]).
pub fn atmosphere_options(
    earth: bool,
    aerosol: f64,
    ozone: f64,
    planet_radius: f64,
    atmosphere_height: f64,
    ground_albedo: f64,
    sun_irradiance: V3,
) -> AtmosphereOptions {
    let k = if earth { 1.0 } else { 0.0 };
    AtmosphereOptions {
        planet_radius,
        top_radius: planet_radius + atmosphere_height,
        rayleigh: k,
        aerosol: aerosol * k,
        ozone: ozone * k,
        ground_albedo,
        sun_irradiance,
    }
}

impl PhysicalSky {
    /// The sky of `s` for an observer `altitude` metres up and the unit `sun` of the scene axes, with a disc of
    /// `sun_size` degrees at rule 6's `sun_disc_radiance` tinted by `sun_color`, and no moon. A sun that meets the
    /// ground is transmitted as zero and draws no disc.
    pub fn new(
        s: &Scattering,
        altitude: f64,
        sun: [f64; 3],
        sun_size: f64,
        sun_color: V3,
    ) -> Result<PhysicalSky, SkyError> {
        let up = to_planet(sun);
        let view = SkyView::new(s, altitude, up)?;
        let obs = [0.0, 0.0, s.atmosphere.planet_radius + altitude];
        let disc = s.atmosphere.sun_disc_radiance(obs, up, sun_size, sun_color)?;
        Ok(PhysicalSky {
            view,
            sun_dir: sun.map(|c| c as f32),
            sun_cos: (sun_size.to_radians() / 2.0).cos() as f32,
            sun: disc.map(|c| c as f32),
            moon_dir: [0.0; 3],
            moon_cos: 2.0,
            moon: [0.0; 3],
        })
    }

    /// Rules 1 to 9 for a frame: the sun and the moon of `o.date` for its observer, the sky of its atmosphere. Nothing
    /// is carried between frames (rule 12): the multiple-scattering table is filled anew.
    pub fn from_document(o: &PhysicalSkyOptions) -> Result<PhysicalSky, SkyError> {
        let space = |e: SpaceError| SkyError(e.to_string());
        let (_, sun) = sun_from_document(&o.date, &o.observer).map_err(space)?;
        let s = Scattering::new(atmosphere(&o.atmosphere)?);
        let mut p = PhysicalSky::new(&s, o.observer.altitude, sun, o.sun_size, o.sun_color)?;
        if o.moon {
            let (moon, dir) = moon_from_document(&o.date, &o.observer).map_err(space)?;
            let obs = [0.0, 0.0, s.atmosphere.planet_radius + o.observer.altitude];
            let l = s.atmosphere.moon_radiance(obs, &moon, o.moon_intensity)?;
            p.moon_dir = dir.map(|c| c as f32);
            p.moon_cos = (moon.angular_diameter.to_radians() as f32 / 2.0).cos();
            p.moon = l.map(|c| c as f32);
        }
        Ok(p)
    }

    /// Whether a sun disc is drawn.
    pub fn has_sun(&self) -> bool {
        self.sun.iter().any(|c| *c > 0.0)
    }

    /// Whether a moon disc is drawn.
    fn has_moon(&self) -> bool {
        self.moon.iter().any(|c| *c > 0.0)
    }

    /// The disc's radiance.
    pub fn sun(&self) -> Vec3 {
        Vec3::from(self.sun)
    }

    /// The disc's solid angle in steradians.
    pub fn sun_solid_angle(&self) -> f32 {
        2.0 * std::f32::consts::PI * (1.0 - self.sun_cos)
    }

    /// The sine of the horizon's elevation: a direction d of the dome is above it when −d.y ≥ this.
    fn horizon_sin(&self) -> f32 {
        self.view.horizon().sin() as f32
    }

    /// The sky alone at a unit direction of the dome.
    pub fn sky(&self, d: Vec3) -> Vec3 {
        let l = self.view.radiance(to_planet(d.as_dvec3().to_array()));
        Vec3::new(l[0] as f32, l[1] as f32, l[2] as f32)
    }

    /// The radiance at a unit direction of the dome: the sky, plus the discs where they are above the horizon.
    pub fn radiance(&self, d: Vec3) -> Vec3 {
        let mut c = self.sky(d);
        if -d.y < self.horizon_sin() {
            return c;
        }
        if self.has_sun() && d.dot(Vec3::from(self.sun_dir)) >= self.sun_cos {
            c += self.sun();
        }
        if self.has_moon() && d.dot(Vec3::from(self.moon_dir)) >= self.moon_cos {
            c += Vec3::from(self.moon);
        }
        c
    }

    /// The sky without its discs as an equirectangular image `width` × `width / 2`: the visible background, which
    /// draws the discs itself.
    pub fn visible(&self, width: u32) -> Equirect {
        let (w, h) = (width.max(4), (width / 2).max(2));
        let rgb = (0..w * h)
            .into_par_iter()
            .map(|k| self.sky(dir_of(((k % w) as f32 + 0.5) / w as f32, ((k / w) as f32 + 0.5) / h as f32)).to_array())
            .collect();
        Equirect { width: w, height: h, rgb }
    }

    /// The sky and its discs as an equirectangular image for image-based lighting: each disc's power spread by
    /// [`add_disc`].
    pub fn equirect(&self, width: u32) -> Equirect {
        let mut eq = self.visible(width);
        if self.has_sun() {
            add_disc(&mut eq, Vec3::from(self.sun_dir), self.sun_cos, self.sun());
        }
        if self.has_moon() {
            add_disc(&mut eq, Vec3::from(self.moon_dir), self.moon_cos, Vec3::from(self.moon));
        }
        eq
    }

    /// The discs as five vec4 for a shader (`sky_discs` of sr-gpu's `sampling.wgsl`): the moon's direction (xyz,
    /// cosine of half its size; above 1 for no moon), the mode (w = 2, a physical sky), the moon's radiance, the sun's
    /// direction (xyz, cosine of half its size) and the sun's radiance (rgb, the sine of the horizon's elevation).
    pub fn uniforms(&self) -> [[f32; 4]; 5] {
        let v = |c: [f32; 3], w: f32| [c[0], c[1], c[2], w];
        [
            v(self.moon_dir, self.moon_cos),
            v([0.0; 3], 2.0),
            v(self.moon, 0.0),
            v(self.sun_dir, self.sun_cos),
            v(self.sun, self.horizon_sin()),
        ]
    }
}
