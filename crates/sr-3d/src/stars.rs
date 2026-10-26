//! The star field of a dome light (the star-fields draft, rules 2, 4 and 6): the catalogue's stars to a limiting
//! magnitude with their flux and colour, their scene directions at an epoch, and the disc of `pointSize` pixels each
//! star's flux is spread over. The disc's share of a pixel is its exact area there over the disc's area, so a star's
//! pixels sum to its flux whatever its size and wherever its centre falls (rule 2: the metered brightness is
//! independent of the size).
//!
//! Directions are in the dome's space, the scene's axes (+x right, +y down, +z forward): the time-frames draft's
//! mapping (x, −z, y) of the ICRF ([`SCENE_FROM_ICRF_AXES`]), so the celestial north pole is up, after a space's
//! frame matrix when the scene has one (rule 4).

use crate::space::stars::{moved, star_field, Star};
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
}

impl Field {
    /// The field of rule 6 (SF3: a positive `magnitude_zero`, a finite `limit_magnitude`).
    pub fn new(catalogue: &[Star], limit_magnitude: f64, magnitude_zero: f64) -> Result<Field, SpaceError> {
        let field = star_field(catalogue, limit_magnitude, 0.0, magnitude_zero)?;
        let stars = catalogue.iter().filter(|s| s.vmag <= limit_magnitude);
        Ok(Field {
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
