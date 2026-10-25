//! Force fields. Every field is an acceleration in pixels per second² (y
//! down), applied alike to particles and, through the body mass, to physics
//! bodies. Documents give fields in m/s² with +y up and radii in metres;
//! `sr-eval` converts them to this pixel space.

use crate::rng::Draws;

/// The purposes of the stream draws of a field (random-streams SREP, section 5): lattices read at t = 0.
pub mod purpose {
    /// The value noise of the gusts of a wind field.
    pub const WIND: u32 = 1;
    /// The value noise whose curl a turbulence field is.
    pub const TURBULENCE: u32 = 2;
}

/// Field kinds of the schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    Directional,
    Radial,
    Vortex,
    Turbulence,
    Drag,
    Wind,
    AttractorPath,
}

/// One force field at one time.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub kind: FieldKind,
    /// Centre (radial, vortex).
    pub pos: [f64; 2],
    /// Acceleration vector (directional, wind).
    pub force: [f64; 2],
    pub strength: f64,
    /// Distance exponent of the attenuation.
    pub falloff: f64,
    /// Reach; unbounded when `None`.
    pub radius: Option<f64>,
    /// Noise feature size in pixels (turbulence, wind gusts).
    pub scale: f64,
    /// Polyline (attractor-path).
    pub path: Vec<[f64; 2]>,
    /// The noise draws of wind and turbulence.
    pub draws: Draws,
    pub bodies: bool,
    pub particles: bool,
    /// Centre depth (px, scene z) for 3D bodies (radial).
    pub z: f64,
    /// Acceleration along scene z (directional, wind) for 3D bodies.
    pub force_z: f64,
}

impl Field {
    fn weight(&self, d: f64) -> f64 {
        match self.radius {
            Some(r) if r > 0.0 => {
                if d >= r {
                    0.0
                } else {
                    (1.0 - d / r).powf(self.falloff.max(0.0))
                }
            }
            _ => 1.0 / (d / 100.0).max(1.0).powf(self.falloff.max(0.0)),
        }
    }

    /// Acceleration at `p` for something moving at `v`, at time `t`.
    pub fn accel(&self, p: [f64; 2], v: [f64; 2], t: f64) -> [f64; 2] {
        match self.kind {
            FieldKind::Directional => self.force,
            FieldKind::Wind => {
                let g = 1.0 + 0.5 * self.draws.noise2(purpose::WIND, t * 0.7, p[1] / self.scale.max(1e-6) * 0.1);
                [self.force[0] * g, self.force[1] * g]
            }
            FieldKind::Radial | FieldKind::Vortex => {
                let d = [p[0] - self.pos[0], p[1] - self.pos[1]];
                let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
                if len < 1e-9 {
                    return [0.0, 0.0];
                }
                let k = self.strength * self.weight(len) / len;
                if self.kind == FieldKind::Radial {
                    [d[0] * k, d[1] * k]
                } else {
                    [-d[1] * k, d[0] * k]
                }
            }
            FieldKind::Turbulence => {
                let s = self.scale.max(1e-6) * 100.0;
                let c = self.draws.curl2(purpose::TURBULENCE, p[0] / s + t * 0.3, p[1] / s);
                let w = self.weight(((p[0] - self.pos[0]).powi(2) + (p[1] - self.pos[1]).powi(2)).sqrt());
                let w = if self.radius.is_some() { w } else { 1.0 };
                [c[0] * self.strength * w * 2.0, c[1] * self.strength * w * 2.0]
            }
            FieldKind::Drag => [-v[0] * self.strength, -v[1] * self.strength],
            FieldKind::AttractorPath => {
                let Some(q) = nearest(&self.path, p) else { return [0.0, 0.0] };
                let d = [q[0] - p[0], q[1] - p[1]];
                let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
                if len < 1e-9 {
                    return [0.0, 0.0];
                }
                let k = self.strength * self.weight(len) / len;
                [d[0] * k, d[1] * k]
            }
        }
    }
}

/// Nearest point on a polyline.
pub fn nearest(path: &[[f64; 2]], p: [f64; 2]) -> Option<[f64; 2]> {
    if path.is_empty() {
        return None;
    }
    if path.len() == 1 {
        return Some(path[0]);
    }
    let mut best = (f64::MAX, path[0]);
    for w in path.windows(2) {
        let (a, b) = (w[0], w[1]);
        let ab = [b[0] - a[0], b[1] - a[1]];
        let l2 = ab[0] * ab[0] + ab[1] * ab[1];
        let t = if l2 > 0.0 { (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / l2).clamp(0.0, 1.0) } else { 0.0 };
        let q = [a[0] + ab[0] * t, a[1] + ab[1] * t];
        let d = (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2);
        if d < best.0 {
            best = (d, q);
        }
    }
    Some(best.1)
}

/// The body acceleration of every field on a 3D body at `p` moving at `v` (pixels, scene axes):
/// the fields' 2D action in the xy plane, with directional and wind forces along z, radial
/// fields measured in 3D, and drag on all three axes.
pub fn total3(fields: &[Field], p: [f64; 3], v: [f64; 3], t: f64) -> [f64; 3] {
    total3_for(fields, p, v, t, false)
}

/// Three-dimensional acceleration, selecting particle/fluid fields when
/// `particles` is true, or rigid-body fields otherwise. Same units as `total3`.
pub fn total3_for(fields: &[Field], p: [f64; 3], v: [f64; 3], t: f64, particles: bool) -> [f64; 3] {
    let mut a = [0.0; 3];
    for f in fields.iter().filter(|f| if particles { f.particles } else { f.bodies }) {
        match f.kind {
            FieldKind::Radial => {
                let d = [p[0] - f.pos[0], p[1] - f.pos[1], p[2] - f.z];
                let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                if len > 1e-9 {
                    let k = f.strength * f.weight(len) / len;
                    for i in 0..3 {
                        a[i] += d[i] * k;
                    }
                }
            }
            FieldKind::Drag => {
                for i in 0..3 {
                    a[i] -= v[i] * f.strength;
                }
            }
            _ => {
                let d = f.accel([p[0], p[1]], [v[0], v[1]], t);
                a[0] += d[0];
                a[1] += d[1];
                a[2] += match f.kind {
                    FieldKind::Directional => f.force_z,
                    // the same gusts as the xy part
                    FieldKind::Wind => {
                        f.force_z * (1.0 + 0.5 * f.draws.noise2(purpose::WIND, t * 0.7, p[1] / f.scale.max(1e-6) * 0.1))
                    }
                    _ => 0.0,
                };
            }
        }
    }
    a
}

pub fn total(fields: &[Field], p: [f64; 2], v: [f64; 2], t: f64, particles: bool) -> [f64; 2] {
    let mut a = [0.0, 0.0];
    for f in fields.iter().filter(|f| if particles { f.particles } else { f.bodies }) {
        let d = f.accel(p, v, t);
        a[0] += d[0];
        a[1] += d[1];
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng;

    fn field(kind: FieldKind, draws: Draws) -> Field {
        Field {
            kind,
            pos: [0.0; 2],
            force: [3.0, -2.0],
            strength: 5.0,
            falloff: 0.0,
            radius: None,
            scale: 40.0,
            path: vec![],
            draws,
            bodies: true,
            particles: true,
            z: 0.0,
            force_z: 1.5,
        }
    }

    #[test]
    fn stream_fields_read_purpose_1_for_gusts_and_2_for_turbulence_and_legacy_fields_their_seed() {
        let (p, t) = ([130.0, 270.0], 0.9);
        let (x, y) = (t * 0.7, p[1] / 40.0 * 0.1);
        let (cx, cy) = (p[0] / 4000.0 + t * 0.3, p[1] / 4000.0);
        for (draws, gust, curl) in [
            (
                Draws::Stream(91),
                Draws::Stream(91).noise2(purpose::WIND, x, y),
                Draws::Stream(91).curl2(purpose::TURBULENCE, cx, cy),
            ),
            (Draws::Legacy(91), rng::noise2(91, x, y), rng::curl2(91, cx, cy)),
        ] {
            let g = 1.0 + 0.5 * gust;
            assert_eq!(field(FieldKind::Wind, draws).accel(p, [0.0; 2], t), [3.0 * g, -2.0 * g]);
            assert_eq!(total3_for(&[field(FieldKind::Wind, draws)], [p[0], p[1], 0.0], [0.0; 3], t, true)[2], 1.5 * g);
            assert_eq!(field(FieldKind::Turbulence, draws).accel(p, [0.0; 2], t), [curl[0] * 10.0, curl[1] * 10.0]);
        }
        assert_ne!(Draws::Stream(91).noise2(purpose::WIND, x, y), Draws::Stream(91).noise2(purpose::TURBULENCE, x, y));
    }
}
