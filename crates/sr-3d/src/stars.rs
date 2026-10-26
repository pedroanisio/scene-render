//! The star field of a dome light (the star-fields draft, rules 2, 4 and 6): the catalogue's stars to a limiting
//! magnitude with their flux and colour, their scene directions at an epoch, and the disc of `pointSize` pixels each
//! star's flux is spread over. The disc's share of a pixel is its exact area there over the disc's area, so a star's
//! pixels sum to its flux whatever its size and wherever its centre falls (rule 2: the metered brightness is
//! independent of the size).
//!
//! Directions are in the dome's space, the scene's axes (+x right, +y down, +z forward): the time-frames draft's
//! mapping (x, −z, y) of the ICRF ([`SCENE_FROM_ICRF_AXES`]), so the celestial north pole is up, after a space's
//! frame matrix when the scene has one (rule 4).

use std::collections::BTreeMap;
use std::sync::OnceLock;

use crate::space::stars::{constellation, direction, moved, precess, star_field, Star, BOUNDARIES};
use crate::space::{mat_apply, Mat3, SpaceError};

pub use crate::space::SCENE_FROM_ICRF_AXES;

/// One star of a field: its direction (ICRF in a [`Field`], scene axes from [`Field::at`]), and its flux and linear
/// colour (Y = 1) of rules 2 and 3.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FieldStar {
    pub hr: u32,
    pub direction: [f64; 3],
    pub flux: f64,
    pub colour: [f64; 3],
    pub constellation: &'static str,
}

/// The stars of a catalogue to a limiting magnitude: what does not change with the epoch, computed once.
#[derive(Debug, Clone)]
pub struct Field {
    stars: Vec<(Star, FieldStar)>,
    /// Every catalogue star by its constellation, whatever the limit (rule 5's labels).
    members: BTreeMap<&'static str, Vec<Star>>,
}

impl Field {
    /// The field of rule 6 (SF3: a positive `magnitude_zero`, a finite `limit_magnitude`).
    pub fn new(catalogue: &[Star], limit_magnitude: f64, magnitude_zero: f64) -> Result<Field, SpaceError> {
        let field = star_field(catalogue, limit_magnitude, 0.0, magnitude_zero)?;
        let stars = catalogue.iter().filter(|s| s.vmag <= limit_magnitude);
        let mut members: BTreeMap<&'static str, Vec<Star>> = BTreeMap::new();
        for s in catalogue {
            members.entry(constellation(s.ra, s.dec, 2000.0)?).or_default().push(s.clone());
        }
        Ok(Field {
            members,
            stars: stars
                .zip(field)
                .map(|(s, f)| {
                    let star = FieldStar {
                        hr: f.hr,
                        direction: f.direction,
                        flux: f.flux,
                        colour: f.colour,
                        constellation: f.constellation,
                    };
                    (s.clone(), star)
                })
                .collect(),
        })
    }

    /// How many stars the field draws.
    pub fn len(&self) -> usize {
        self.stars.len()
    }

    /// Whether the field draws no star.
    pub fn is_empty(&self) -> bool {
        self.stars.is_empty()
    }

    /// The field `years` Julian years after J2000 (rule 4) in the scene axes `axes` (rows first) of the ICRF: each
    /// star moved by its proper motion when `proper_motion`, else at its J2000 place.
    pub fn at(&self, years: f64, proper_motion: bool, axes: &Mat3) -> Result<Vec<FieldStar>, SpaceError> {
        self.stars
            .iter()
            .map(|(s, f)| {
                let d = if proper_motion { moved(s, years)? } else { f.direction };
                Ok(FieldStar { direction: mat_apply(axes, d), ..*f })
            })
            .collect()
    }
}

impl Field {
    /// Rule 5's labels: each constellation's abbreviation at the centroid of the catalogue stars it holds (the
    /// normalised mean of their directions `years` after J2000, moved when `proper_motion`), in the axes `axes`, in
    /// alphabetical order. Every catalogue star counts, so a label stays where it is whatever the limiting magnitude.
    pub fn labels(
        &self,
        years: f64,
        proper_motion: bool,
        axes: &Mat3,
    ) -> Result<Vec<(&'static str, [f64; 3])>, SpaceError> {
        self.members
            .iter()
            .map(|(abbr, stars)| {
                let mut sum = [0.0; 3];
                for s in stars {
                    let d = moved(s, if proper_motion { years } else { 0.0 })?;
                    sum = [sum[0] + d[0], sum[1] + d[1], sum[2] + d[2]];
                }
                let n = (sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]).sqrt();
                Ok((*abbr, mat_apply(axes, sum.map(|v| v / n))))
            })
            .collect()
    }
}

/// The longest step, in right ascension or declination, between two points of a boundary line (rule 5).
pub const BOUNDARY_STEP_DEG: f64 = 0.5;

/// Rule 5's boundaries as ICRF directions: lines no two of whose consecutive points are more than
/// [`BOUNDARY_STEP_DEG`] apart. Roman's table is constant on each cell of the grid its right ascensions and
/// declinations make at equinox 1875, so a boundary is each edge of that grid whose two cells name different
/// constellations: an arc of constant declination or of constant right ascension, precessed to J2000, where the
/// field is drawn. Computed once.
pub fn boundaries() -> &'static [Vec<[f64; 3]>] {
    static LINES: OnceLock<Vec<Vec<[f64; 3]>>> = OnceLock::new();
    LINES.get_or_init(|| {
        let sorted = |mut v: Vec<f64>| {
            v.sort_by(f64::total_cmp);
            v.dedup();
            v
        };
        let ras = sorted(BOUNDARIES.iter().flat_map(|r| [r.0, r.1]).chain([0.0, 24.0]).collect());
        let decs = sorted(BOUNDARIES.iter().map(|r| r.2).chain([-90.0, 90.0]).collect());
        let (nr, nd) = (ras.len() - 1, decs.len() - 1);
        // the constellation of cell (i, j), between ras[i]..ras[i + 1] hours and decs[j]..decs[j + 1] degrees
        let cell: Vec<&str> = (0..nd)
            .flat_map(|j| (0..nr).map(move |i| (i, j)))
            .map(|(i, j)| {
                let (ra, dec) = ((ras[i] + ras[i + 1]) / 2.0, (decs[j] + decs[j + 1]) / 2.0);
                constellation(ra * 15.0, dec, 1875.0).unwrap_or("")
            })
            .collect();
        let at = |i: usize, j: usize| cell[j * nr + i];
        // a line from (ra0, dec0) to (ra1, dec1) at 1875, hours and degrees, sampled evenly
        let line = |(ra0, dec0): (f64, f64), (ra1, dec1): (f64, f64)| -> Vec<[f64; 3]> {
            let span = ((ra1 - ra0) * 15.0).abs().max((dec1 - dec0).abs());
            let n = (span / BOUNDARY_STEP_DEG).ceil().max(1.0) as usize;
            (0..=n)
                .filter_map(|k| {
                    let t = k as f64 / n as f64;
                    let (ra, dec) = ((ra0 + (ra1 - ra0) * t) * 15.0, dec0 + (dec1 - dec0) * t);
                    let (ra, dec) = precess(ra, dec, 1875.0, 2000.0).ok()?;
                    direction(ra, dec).ok()
                })
                .collect()
        };
        let mut lines = Vec::new();
        for j in 0..nd {
            for i in 0..nr {
                // the meridian edge on the cell's west side (the first cell's west neighbour is the last)
                let west = if i == 0 { nr - 1 } else { i - 1 };
                if at(i, j) != at(west, j) {
                    lines.push(line((ras[i], decs[j]), (ras[i], decs[j + 1])));
                }
                // the parallel edge on its south side
                if j > 0 && at(i, j) != at(i, j - 1) {
                    lines.push(line((ras[i], decs[j]), (ras[i + 1], decs[j])));
                }
            }
        }
        lines
    })
}

/// Twice the area of the disc of radius `r` about the origin left of `t` per unit height: the primitive of
/// √(r² − t²), for t in [−r, r].
fn half_chord_integral(t: f64, r: f64) -> f64 {
    (t * (r * r - t * t).max(0.0).sqrt() + r * r * (t / r).clamp(-1.0, 1.0).asin()) / 2.0
}

/// The area of the disc of radius `r` about the origin where X ≤ x and Y ≤ y.
fn corner_area(x: f64, y: f64, r: f64) -> f64 {
    let s = |t: f64| half_chord_integral(t, r);
    let x = x.clamp(-r, r);
    if y >= r {
        return 2.0 * (s(x) - s(-r));
    }
    if y <= -r {
        return 0.0;
    }
    // columns with |t| ≤ c cross the line Y = y; outside it they are wholly below it (y ≥ 0) or above it (y < 0)
    let c = (r * r - y * y).sqrt();
    let crossing = if x <= -c {
        0.0
    } else {
        let m = x.min(c);
        y * (m + c) + s(m) - s(-c)
    };
    if y < 0.0 {
        return crossing;
    }
    let left = 2.0 * (s(x.min(-c)) - s(-r));
    let right = if x > c { 2.0 * (s(x) - s(c)) } else { 0.0 };
    left + crossing + right
}

/// The share of a star's flux that pixel (`px`, `py`), the square [px, px + 1] × [py, py + 1], holds when the flux
/// is spread evenly over the disc of radius `r` pixels about (`cx`, `cy`): the disc's area in the pixel over its whole
/// area. The shares of all pixels sum to 1.
pub fn disc_cover(cx: f64, cy: f64, r: f64, px: i64, py: i64) -> f64 {
    let (x0, y0) = (px as f64 - cx, py as f64 - cy);
    let (x1, y1) = (x0 + 1.0, y0 + 1.0);
    let a = corner_area(x1, y1, r) - corner_area(x0, y1, r) - corner_area(x1, y0, r) + corner_area(x0, y0, r);
    (a / (std::f64::consts::PI * r * r)).max(0.0)
}

/// A star on the frame: its centre in pixels (pixel (i, j) is the square [i, i + 1] × [j, j + 1]) and its flux per
/// channel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub pixel: [f64; 2],
    pub rgb: [f32; 3],
}

/// Adds each point's flux to `buf` (`size[0]` × `size[1]` pixels, rows first), spread over a disc `point_size` pixels
/// across; the share that falls outside the frame is lost, and a point whose centre is not finite adds nothing.
pub fn splat(buf: &mut [[f32; 3]], size: [u32; 2], points: &[Point], point_size: f64) {
    let r = point_size / 2.0;
    let [w, h] = size.map(i64::from);
    for p in points {
        // a centre that is not finite gives an empty range: float-to-integer casts saturate, and NaN casts to 0
        let [cx, cy] = p.pixel;
        let (x0, x1) = (((cx - r).floor() as i64).max(0), ((cx + r).ceil() as i64).min(w));
        let (y0, y1) = (((cy - r).floor() as i64).max(0), ((cy + r).ceil() as i64).min(h));
        for py in y0..y1 {
            for px in x0..x1 {
                let k = disc_cover(cx, cy, r, px, py) as f32;
                let c = &mut buf[(py * w + px) as usize];
                for (c, v) in c.iter_mut().zip(p.rgb) {
                    *c += v * k;
                }
            }
        }
    }
}
