//! The time-frames draft at the evaluator (sr-core `srep/srep-0000-time-frames.md`, rule 7): a document's `space`
//! read once per program, and the `object3D` nodes it places at each frame. `positionFrom="ephemeris"` replaces
//! the node's x, y and z with its body's position about the space's origin in scene pixels; `orientation="bodyFixed"`
//! replaces its rotation, rotationY and rotationX with its body's fixed axes in the space's frame, as the renderer
//! composes them (Rz · Ry · Rx). Scale, parenting and the node's other properties stay its own.

use std::path::Path;

use glam::{DMat3, EulerRot};
use sr_model::model as m;
use sr_space::{Frame, Positions, Space, Spk, TimeScale};

use crate::eval::Props;

/// A document's `space`: its clock and frame, the file its positions come from (none with the approximate elements)
/// and the pixels per metre of its physics.
#[derive(Debug)]
pub struct Sky {
    space: Space,
    file: Option<Spk>,
    pixels_per_meter: f64,
}

impl Sky {
    /// The space of a document whose files resolve against `base`, or why it cannot be read; `None` without one. An
    /// included document's ephemeris assets are known by `ns/id` as well as by their own id.
    pub(crate) fn of(scene: &m::Scene, base: &Path, ns: &str) -> Option<Result<Sky, String>> {
        let s = scene.space.as_ref()?;
        Some(Self::read(scene, s, base, ns).map_err(|e| format!("space: {e}")))
    }

    fn read(scene: &m::Scene, s: &m::Space, base: &Path, ns: &str) -> Result<Sky, String> {
        let scale = match s.time_scale {
            m::TimeScale::Utc => TimeScale::Utc,
            m::TimeScale::Tt => TimeScale::Tt,
            m::TimeScale::Tdb => TimeScale::Tdb,
        };
        let frame = match s.frame {
            m::Frame::Icrf => Frame::Icrf,
            m::Frame::EclipticJ2000 => Frame::EclipticJ2000,
            m::Frame::BodyFixed => Frame::BodyFixed(s.frame_body.ok_or("frame=\"bodyFixed\" needs a frameBody (TF3)")?),
        };
        let space = Space::new(&s.epoch, scale, s.rate.get(), s.origin, frame).map_err(|e| e.to_string())?;
        let file = match s.ephemeris.as_str() {
            "approximate" => None,
            id => {
                let scoped = format!("{ns}/{id}");
                let e = scene
                    .assets
                    .iter()
                    .flat_map(|a| &a.children)
                    .find_map(|c| match c {
                        m::AssetsChild::Ephemeris(e) if e.id == id || (!ns.is_empty() && e.id == scoped) => Some(e),
                        _ => None,
                    })
                    .ok_or_else(|| format!("ephemeris {id:?} names no ephemeris asset (TF5)"))?;
                let path = match sr_model::assets::resolve(&e.src, base) {
                    sr_model::assets::Resolved::Local(path) => path,
                    sr_model::assets::Resolved::Remote(scheme) => {
                        return Err(format!(
                            "ephemeris {id:?}: {} is remote ({scheme}) and is read from a local file",
                            e.src
                        ))
                    }
                };
                Some(Spk::open(&path, Some(&e.digest)).map_err(|err| format!("ephemeris {id:?}: {err}"))?)
            }
        };
        let pixels_per_meter = scene.physics.as_ref().map_or(100.0, |ph| ph.pixels_per_meter.get());
        Ok(Sky { space, file, pixels_per_meter })
    }

    fn positions(&self) -> Positions<'_> {
        self.file.as_ref().map_or(Positions::Approximate, Positions::File)
    }

    /// The star-fields draft's epoch and axes at `t` seconds of scene time (rule 4): the Julian years since J2000 of
    /// the frame's ephemeris time, and the scene axes of the ICRF through the space's frame (rows first).
    pub fn star_frame(&self, t: f64) -> Result<(f64, sr_space::Mat3), String> {
        let et = self.space.time_at(t).map_err(|e| e.to_string())?;
        let frame = self.space.frame_matrix(et).map_err(|e| e.to_string())?;
        Ok((sr_space::stars::years_since_j2000(et), sr_space::mat_mul(&sr_space::SCENE_FROM_ICRF_AXES, &frame)))
    }

    /// Places an object at `t` seconds of scene time: its position, its orientation or both replace its properties.
    pub(crate) fn place(&self, o: &m::Object3D, t: f64, props: &mut Props) -> Result<(), String> {
        let by_position = o.position_from == m::PositionFrom::Ephemeris;
        let by_axes = o.orientation == m::Orientation::BodyFixed;
        if !(by_position || by_axes) {
            return Ok(());
        }
        let body = o.body.ok_or("positionFrom and orientation need a body (TF4)")?;
        let et = self.space.time_at(t).map_err(|e| e.to_string())?;
        if by_position {
            let p = self.positions().position(body, self.space.origin, et).map_err(|e| e.to_string())?;
            let p = self.space.scene_from_icrf(p, [0.0; 3], self.pixels_per_meter, et).map_err(|e| e.to_string())?;
            for (k, v) in [("x", p[0]), ("y", p[1]), ("z", p[2])] {
                props.set(k, v);
            }
        }
        if by_axes {
            let rows = sr_space::body_axes(&self.space, body, et).map_err(|e| e.to_string())?;
            let (z, y, x) = DMat3::from_cols_array_2d(&rows).transpose().to_euler(EulerRot::ZYX);
            for (k, v) in [("rotation", z), ("rotationY", y), ("rotationX", x)] {
                props.set(k, v.to_degrees());
            }
        }
        Ok(())
    }
}
