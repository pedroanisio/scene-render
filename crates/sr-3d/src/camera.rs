//! The physical camera: pose, projection, depth of field and shake.
//!
//! Scene space is the frame's pixel space extended into depth: origin at the
//! frame's top-left corner on z = 0, +x right, +y down, +z away from the viewer
//! (scene-render conventions 2.1). The implicit camera (no `<camera>`) matches
//! the compositor's 2.5D projection: it sits on the frame's centre axis at
//! `zoom = (w/2) / tan(fov/2)` with a 60° horizontal field of view, looking
//! along +z with y down, so the z = 0 plane maps 1:1 onto the frame (2.2).
//! A `<camera>`'s x, y and z are absolute scene positions (2.3), or positions in
//! its parent's frame when it is parented (5.4).
//!
//! The depth buffer follows rule 1 of the scale draft (`crate::scale`): within SC1 the projection keeps the
//! document's `near` and `far`, its entries are computed in binary64, and a camera's `depth` mode names the buffer
//! value of a distance ([`CameraView::depth_value`]).

use glam::{Mat4, Vec3};

pub use crate::scale::DepthMode;

/// Camera inputs, already resolved from the document.
#[derive(Clone, Debug, PartialEq)]
pub struct CameraParams {
    /// Horizontal field of view, degrees.
    pub fov: f32,
    pub orthographic: bool,
    /// Visible height in scene units for orthographic cameras (default: the frame height).
    pub ortho_height: Option<f32>,
    pub near: f32,
    pub far: f32,
    /// Absolute eye position in scene space; `None` is the implicit camera's eye.
    pub position: Option<Vec3>,
    /// Added to the eye (camera shake).
    pub offset: Vec3,
    /// Degrees.
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    /// World point to look at.
    pub target: Option<Vec3>,
    /// World matrix of the parent frame that `position`, `yaw`, `pitch` and `roll` are in.
    pub frame: Option<Mat4>,
    /// `camera/@depth`: the depth buffer's mapping.
    pub depth: DepthMode,
}

impl Default for CameraParams {
    fn default() -> CameraParams {
        CameraParams {
            fov: 60.0,
            orthographic: false,
            ortho_height: None,
            near: 0.1,
            far: 10000.0,
            position: None,
            offset: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            target: None,
            frame: None,
            depth: DepthMode::Reversed,
        }
    }
}

/// A resolved camera for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraView {
    /// World → camera (camera space: x right, y down, z forward).
    pub view: Mat4,
    /// Camera → clip, reverse-Z (near maps to depth 1, far to 0).
    pub proj: Mat4,
    pub eye: Vec3,
    /// Horizontal focal length in pixels.
    pub focal_px: f32,
    pub near: f32,
    pub far: f32,
    pub orthographic: bool,
    /// The depth buffer's mapping: `proj` is always reversed; a logarithmic camera's renderer writes
    /// [`CameraView::depth_value`] itself.
    pub depth: DepthMode,
}

impl CameraView {
    pub fn view_proj(&self) -> Mat4 {
        self.proj * self.view
    }
    /// Camera-space depth (distance along the view axis) of a world point.
    pub fn depth_of(&self, p: Vec3) -> f32 {
        self.view.transform_point3(p).z
    }

    /// The depth buffer value of camera-space depth `z` (rule 1): reversed n (f − z) / ((f − n) z), or for an
    /// orthographic camera (f − z) / (f − n), 1 at near and 0 at far; logarithmic ln(z / n) / ln(f / n), 0 at near
    /// and 1 at far. Computed in binary64.
    pub fn depth_value(&self, z: f32) -> f32 {
        let (z, n, f) = (z as f64, self.near as f64, self.far as f64);
        let d = match (self.depth, self.orthographic) {
            (DepthMode::Logarithmic, _) => (z / n).ln() / (f / n).ln(),
            (DepthMode::Reversed, true) => (f - z) / (f - n),
            (DepthMode::Reversed, false) => n * (f - z) / ((f - n) * z),
        };
        d as f32
    }
}

/// The near and far planes of a camera: the document's own when SC1 holds (near positive, far beyond it, at most
/// [`crate::scale::MAX_DEPTH_RATIO`] times near), else near at least 1e-4 and far at least near + 1e-3.
pub fn planes(near: f32, far: f32) -> (f32, f32) {
    if crate::scale::depth_ratio_valid(near as f64, far as f64) {
        return (near, far);
    }
    // max ignores a NaN near
    let n = near.max(1e-4);
    (n, if far.is_finite() { far.max(n + 1e-3) } else { n + 1e-3 })
}

/// The reversed perspective projection of a camera `zoom` pixels from a `w`×`h` frame: clip = (fx x, −fy y,
/// n f / (f − n) − n z / (f − n), z), the entries computed in binary64.
pub fn perspective(zoom: f32, w: f32, h: f32, n: f32, f: f32) -> Mat4 {
    let (z, w, h, n, f) = (zoom as f64, w as f64, h as f64, n as f64, f as f64);
    let (a, b) = (-n / (f - n), n / (1.0 - n / f));
    Mat4::from_cols_array(&[
        (2.0 * z / w) as f32,
        0.0,
        0.0,
        0.0,
        0.0,
        (-2.0 * z / h) as f32,
        0.0,
        0.0,
        0.0,
        0.0,
        a as f32,
        1.0,
        0.0,
        0.0,
        b as f32,
        0.0,
    ])
}

/// The reversed orthographic projection of an `ow`×`oh` view: depth (f − z) / (f − n), in binary64.
pub fn orthographic(ow: f32, oh: f32, n: f32, f: f32) -> Mat4 {
    let (n, f) = (n as f64, f as f64);
    Mat4::from_cols_array(&[
        2.0 / ow,
        0.0,
        0.0,
        0.0,
        0.0,
        -2.0 / oh,
        0.0,
        0.0,
        0.0,
        0.0,
        (-1.0 / (f - n)) as f32,
        0.0,
        0.0,
        0.0,
        (f / (f - n)) as f32,
        1.0,
    ])
}

/// Distance of the default camera from the z = 0 plane.
pub fn zoom(width: f32, fov: f32) -> f32 {
    (width * 0.5) / (fov.to_radians() * 0.5).tan()
}

/// The camera looking along +z with y down, rotated by yaw (about y), pitch (about x) and roll (about z).
fn orientation(yaw: f32, pitch: f32, roll: f32) -> Mat4 {
    // Rx(θ) turns +z towards −y (up): positive pitch looks up; positive yaw turns towards +x
    Mat4::from_rotation_y(yaw.to_radians())
        * Mat4::from_rotation_x(pitch.to_radians())
        * Mat4::from_rotation_z(roll.to_radians())
}

/// Resolves a camera for a `w`×`h` frame.
pub fn resolve(p: &CameraParams, w: f32, h: f32) -> CameraView {
    let fov = p.fov.clamp(0.1, 179.0);
    let z = zoom(w, fov);
    let local_eye = p.position.unwrap_or(Vec3::new(w * 0.5, h * 0.5, -z)) + p.offset;
    let (eye, frame_rot) = match p.frame {
        Some(f) => (f.transform_point3(local_eye), Mat4::from_quat(f.to_scale_rotation_translation().1)),
        None => (local_eye, Mat4::IDENTITY),
    };
    let rot = match p.target {
        Some(t) if (t - eye).length_squared() > 1e-8 => {
            // look-at in scene axes (+x right, +y down, +z forward): the same right-handed basis that
            // orientation() gives for a yaw and pitch, so a look-at camera never mirrors the frame
            let f = (t - eye).normalize();
            let down_hint = if f.y.abs() > 0.999 { Vec3::Z } else { Vec3::Y };
            let r = down_hint.cross(f).normalize();
            let d = f.cross(r).normalize();
            Mat4::from_cols(r.extend(0.0), d.extend(0.0), f.extend(0.0), glam::Vec4::W)
                * Mat4::from_rotation_z(p.roll.to_radians())
        }
        _ => frame_rot * orientation(p.yaw, p.pitch, p.roll),
    };
    let cam_to_world = Mat4::from_translation(eye) * rot;
    let view = cam_to_world.inverse();
    let (n, f) = planes(p.near, p.far);
    let proj = if p.orthographic {
        let oh = p.ortho_height.unwrap_or(h).max(1e-3);
        orthographic(oh * w / h, oh, n, f)
    } else {
        perspective(z, w, h, n, f)
    };
    CameraView { view, proj, eye, focal_px: z, near: n, far: f, orthographic: p.orthographic, depth: p.depth }
}

/// Horizontal field of view (degrees) of a lens of `focal_mm` on a sensor `sensor_mm` wide.
pub fn fov_of_lens(focal_mm: f32, sensor_mm: f32) -> f32 {
    2.0 * (sensor_mm / (2.0 * focal_mm)).atan().to_degrees()
}

/// Focal length (mm) for a horizontal field of view on a sensor `sensor_mm` wide.
pub fn lens_of_fov(fov: f32, sensor_mm: f32) -> f32 {
    sensor_mm / (2.0 * (fov.to_radians() * 0.5).tan())
}

/// Whether a `w`×`h` frame is narrower than a `sensor_w`×`sensor_h` sensor, so that the lens is fitted to the sensor's
/// height (the vertical gate) instead of its width. A frame of exactly the sensor's aspect uses the width.
pub fn vertical_gate(sensor_w: f32, sensor_h: f32, w: f32, h: f32) -> bool {
    w * sensor_h < h * sensor_w
}

/// Horizontal field of view (degrees) of a lens of `focal_mm` on a `sensor_w`×`sensor_h` sensor for a `w`×`h` frame:
/// from the sensor's width when the frame is at least as wide as the sensor, else from its height (the frame then
/// shows the full sensor height, so the vertical angle is fixed by `focal_mm` and `sensor_h`).
pub fn fov_of_lens_in_frame(focal_mm: f32, sensor_w: f32, sensor_h: f32, w: f32, h: f32) -> f32 {
    if vertical_gate(sensor_w, sensor_h, w, h) {
        2.0 * ((sensor_h / (2.0 * focal_mm)) * (w / h)).atan().to_degrees()
    } else {
        fov_of_lens(focal_mm, sensor_w)
    }
}

/// Depth-of-field constant: the circle of confusion in pixels is
/// `k · |1/focus − 1/depth|` with depths in scene units (100 per metre).
pub fn coc_scale(focal_mm: f32, f_stop: f32, sensor_mm: f32, width_px: f32) -> f32 {
    // c(mm) = f²/N · |1/zf − 1/z| with z in mm; 1 scene unit = 10 mm
    (focal_mm * focal_mm / f_stop.max(0.1)) * (width_px / sensor_mm.max(1e-3)) / 10.0
}

/// Camera shake from its four fractal noise values N₀…N₃ (channels 0 to 3 at
/// frequency · t): (x, y offset in scene units, roll in degrees, zoom factor). The camera
/// moves amplitude · N₀ right and amplitude · N₁ down, rolls rotation · N₂ and zooms by 1 + zoom · N₃.
pub fn shake(amplitude: f32, rotation: f32, zoom_amount: f32, n: [f64; 4]) -> (f32, f32, f32, f32) {
    (n[0] as f32 * amplitude, n[1] as f32 * amplitude, n[2] as f32 * rotation, 1.0 + n[3] as f32 * zoom_amount)
}

#[cfg(test)]
mod look_at_tests {
    use super::*;

    fn params(position: Vec3, yaw: f32, pitch: f32, target: Option<Vec3>) -> CameraParams {
        CameraParams { position: Some(position), yaw, pitch, target, ..CameraParams::default() }
    }

    /// A look-at camera sees the frame as a camera turned the same way by yaw and pitch does: same basis, no mirror.
    #[test]
    fn look_at_matches_the_equivalent_yaw_and_pitch() {
        let eye = Vec3::new(100.0, -50.0, -400.0);
        for (yaw, pitch) in [(0.0f32, 0.0f32), (30.0, 0.0), (-40.0, 15.0), (10.0, -25.0)] {
            let aimed = params(eye, yaw, pitch, None);
            let fwd = orientation(yaw, pitch, 0.0).transform_vector3(Vec3::Z);
            let looked = params(eye, 0.0, 0.0, Some(eye + fwd * 300.0));
            let (a, b) = (resolve(&aimed, 1920.0, 1080.0).view, resolve(&looked, 1920.0, 1080.0).view);
            for p in [Vec3::new(150.0, -20.0, 0.0), Vec3::new(-80.0, 60.0, 200.0), Vec3::new(40.0, 0.0, -100.0)] {
                let (pa, pb) = (a.transform_point3(p), b.transform_point3(p));
                assert!((pa - pb).length() < 1e-2, "yaw {yaw} pitch {pitch}: {pa} vs {pb}");
            }
            assert!(b.determinant() > 0.0, "look-at view must not mirror");
        }
    }
}

#[cfg(test)]
mod gate_tests {
    use super::*;

    #[test]
    fn a6_a_wide_frame_keeps_the_horizontal_gate() {
        for (w, h) in [(1920.0, 1080.0), (1500.0, 1000.0), (2000.0, 1000.0)] {
            assert!(!vertical_gate(36.0, 24.0, w, h));
            assert_eq!(fov_of_lens_in_frame(50.0, 36.0, 24.0, w, h), fov_of_lens(50.0, 36.0));
        }
    }

    #[test]
    fn a6_a_narrow_frame_takes_its_vertical_field_of_view_from_sensor_height() {
        for (w, h) in [(1080.0f32, 1920.0f32), (1000.0, 1000.0), (1200.0, 900.0)] {
            assert!(vertical_gate(36.0, 24.0, w, h), "{w}x{h}");
            let fov = fov_of_lens_in_frame(50.0, 36.0, 24.0, w, h);
            let vfov = 2.0 * ((h / w) * (fov.to_radians() * 0.5).tan()).atan().to_degrees();
            assert!((vfov - fov_of_lens(50.0, 24.0)).abs() < 1e-3, "{w}x{h}: {vfov}");
        }
    }

    #[test]
    fn a6_the_sensor_height_does_not_matter_for_a_wide_frame() {
        assert_eq!(
            fov_of_lens_in_frame(35.0, 36.0, 24.0, 1920.0, 1080.0),
            fov_of_lens_in_frame(35.0, 36.0, 30.0, 1920.0, 1080.0)
        );
        assert_ne!(
            fov_of_lens_in_frame(35.0, 36.0, 24.0, 1080.0, 1920.0),
            fov_of_lens_in_frame(35.0, 36.0, 30.0, 1080.0, 1920.0)
        );
    }
}
