//! Rule 3 of the scale draft drawn: a camera's `scaleBar`, the largest 1, 2 or 5 × 10^k of a unit that fits
//! `maxWidth` pixels at the measured depth, as a filled bar from (`x`, `y`) pixels from the frame's bottom left with
//! its label above it. The bar and the label are vector paths in frame pixels, drawn over the camera's pass.

use glam::{DMat4, DVec3, Mat4};
use sr_3d::scale::{self, ScaleBar, ScaleError, Units};
use sr_3d::Vertex;
use sr_model::element::Element;
use sr_text::layout::Align;
use sr_text::{chart, FontLib, Style};
use sr_vector::geom::{p, Xf};
use sr_vector::path::Poly;
use sr_vector::scene::{Cmd, FillRule, Paint, Scene};

use crate::vector::Attrs;

/// A label's height, frame pixels.
pub const LABEL_PX: f64 = 14.0;
/// The space between the bar's top and its label's bottom, frame pixels.
pub const LABEL_GAP_PX: f64 = 4.0;
/// Flattening tolerance, frame pixels.
const TOL: f64 = 0.05;

/// A `scaleBar`'s attributes (the colour is read by the renderer, which resolves paints).
#[derive(Debug, Clone, PartialEq)]
pub struct BarSpec {
    pub max_width: u32,
    pub units: Units,
    /// The depth to measure, scene pixels.
    pub distance: Option<f64>,
    pub x: f64,
    pub y: f64,
    pub thickness: f64,
    pub label: bool,
}

/// The camera's `scaleBar` child, if it has one (SC3: at most one).
pub fn bar_of(camera: &dyn Element) -> Option<&dyn Element> {
    sr_model::element::children(camera).into_iter().find(|c| c.element_name() == "scaleBar")
}

/// The attributes of a `scaleBar` with the schema's defaults, refusing what the schema refuses.
pub fn spec(e: &dyn Element) -> Result<BarSpec, ScaleError> {
    let a = Attrs { e, props: None };
    let bad = |m: &str| Err(ScaleError(format!("scaleBar: {m}")));
    let max_width = a.num("maxWidth", 200.0);
    if !(max_width >= 1.0 && max_width.fract() == 0.0 && max_width <= u32::MAX as f64) {
        return bad("maxWidth must be a positive integer");
    }
    let units = Units::parse(a.str("units").as_deref().unwrap_or("auto"))?;
    let distance = a.opt("distance");
    if distance.is_some_and(|d| !(d > 0.0 && d.is_finite())) {
        return bad("distance must be positive");
    }
    let (x, y) = (a.num("x", 24.0), a.num("y", 24.0));
    if !(x.is_finite() && y.is_finite()) {
        return bad("x and y must be finite");
    }
    let thickness = a.num("thickness", 2.0);
    if !(thickness > 0.0 && thickness.is_finite()) {
        return bad("thickness must be positive");
    }
    let label = match a.str("label").as_deref() {
        None | Some("true" | "1") => true,
        Some("false" | "0") => false,
        Some(v) => return bad(&format!("label must be a boolean, not {v:?}")),
    };
    Ok(BarSpec { max_width: max_width as u32, units, distance, x, y, thickness, label })
}

/// Where the measured depth came from (rule 3's order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Measured {
    /// `scaleBar/@distance`.
    Distance,
    /// The distance to the camera's target.
    Target,
    /// The nearest surface under the frame's centre.
    Surface,
    /// Nothing under the centre: the camera's far plane.
    Far,
    /// An orthographic camera: its visible height.
    OrthoHeight,
}

/// A scale bar as drawn: its camera, where its depth came from, the depth (scene pixels; an orthographic camera's
/// visible height), its length in frame pixels and its label.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Drawn {
    pub camera: String,
    pub measured: Measured,
    pub depth: f64,
    pub pixels: f64,
    pub label: String,
}

/// The perspective depth rule 3 measures: `distance`, else the target's distance, else the surface under the centre,
/// else `far`.
pub fn measure(
    spec: &BarSpec,
    target: Option<f64>,
    surface: impl FnOnce() -> Option<f64>,
    far: f64,
) -> (Measured, f64) {
    if let Some(d) = spec.distance {
        return (Measured::Distance, d);
    }
    if let Some(d) = target {
        return (Measured::Target, d);
    }
    match surface() {
        Some(d) => (Measured::Surface, d),
        None => (Measured::Far, far),
    }
}

/// The bar a perspective camera of horizontal focal length `focal_px` draws at `depth` scene pixels.
pub fn perspective(spec: &BarSpec, depth: f64, focal_px: f64, ppm: f64) -> Result<ScaleBar, ScaleError> {
    scale::scale_bar_at(depth, focal_px, ppm, spec.max_width, spec.units)
}

/// The bar an orthographic camera showing `ortho_height` scene pixels in `height` frame pixels draws.
pub fn orthographic(spec: &BarSpec, ortho_height: f64, height: f64, ppm: f64) -> Result<ScaleBar, ScaleError> {
    scale::scale_bar(scale::pixels_per_metre_ortho(ortho_height, height, ppm)?, spec.max_width, spec.units)
}

/// The bar's rectangle in frame pixels (y down): left, top, right, bottom.
pub fn rect(spec: &BarSpec, bar: &ScaleBar, frame: [f64; 2]) -> [f64; 4] {
    let bottom = frame[1] - spec.y;
    [spec.x, bottom - spec.thickness, spec.x + bar.pixels, bottom]
}

/// The bar and, with `label`, its label: left-aligned on the bar, `LABEL_PX` high, `LABEL_GAP_PX` above it, in frame
/// pixels of a frame `frame` pixels in size. A label no font draws is an error, not a bar without one.
pub fn scene(
    lib: &mut FontLib,
    spec: &BarSpec,
    bar: &ScaleBar,
    frame: [f64; 2],
    paint: Paint,
) -> Result<Scene, ScaleError> {
    let [x0, y0, x1, y1] = rect(spec, bar, frame);
    let mut out = Scene::default();
    let pts = vec![p(x0, y0), p(x1, y0), p(x1, y1), p(x0, y1)];
    out.cmds.push(Cmd::Fill {
        polys: vec![Poly { pts, closed: true }],
        rule: FillRule::NonZero,
        paint: paint.clone(),
        opacity: 1.0,
    });
    if spec.label {
        let style = Style { size: LABEL_PX, color: Some(paint), ..Default::default() };
        let glyphs = chart::text(lib, &bar.label, &style, 0.0, 0.0, Align::Start, f64::INFINITY, TOL).scene;
        let [gx0, _, gx1, gy1] = glyphs.bounds().0;
        // an empty drawing has inverted bounds
        if gx0 > gx1 || gx0.is_nan() || gx1.is_nan() {
            return Err(ScaleError(format!("no font draws the scale bar's label {:?}", bar.label)));
        }
        out.extend(glyphs.transformed(&Xf::translate(x0 - gx0, y0 - LABEL_GAP_PX - gy1)));
    }
    Ok(out)
}

/// The distance along `axis` (a unit vector) from `eye` to the nearest triangle of `meshes` it meets between
/// `near` and `far`, both faces counted. Each mesh is its model matrix, vertices and triangle indices, all in the
/// pass's rebased space.
pub fn centre_surface<'a>(
    eye: DVec3,
    axis: DVec3,
    near: f64,
    far: f64,
    meshes: impl IntoIterator<Item = (Mat4, &'a [Vertex], &'a [u32])>,
) -> Option<f64> {
    let mut best: Option<f64> = None;
    for (model, vertices, indices) in meshes {
        let m: DMat4 = model.as_dmat4();
        let at = |i: u32| vertices.get(i as usize).map(|v| m.transform_point3(glam::Vec3::from(v.pos).as_dvec3()));
        for t in indices.as_chunks::<3>().0 {
            let (Some(a), Some(b), Some(c)) = (at(t[0]), at(t[1]), at(t[2])) else { continue };
            if let Some(d) = hit(eye, axis, a, b, c) {
                if near <= d && d <= far && best.is_none_or(|e| d < e) {
                    best = Some(d);
                }
            }
        }
    }
    best
}

/// Möller–Trumbore: the distance along `dir` from `o` to triangle `a b c`, either face.
fn hit(o: DVec3, dir: DVec3, a: DVec3, b: DVec3, c: DVec3) -> Option<f64> {
    let (e1, e2) = (b - a, c - a);
    let pv = dir.cross(e2);
    let det = e1.dot(pv);
    if det.abs() < 1e-300 {
        return None;
    }
    let tv = o - a;
    let u = tv.dot(pv) / det;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let qv = tv.cross(e1);
    let v = dir.dot(qv) / det;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    Some(e2.dot(qv) / det)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec_from(xml: &str) -> Result<BarSpec, ScaleError> {
        let doc = format!(
            r#"<scene version="1.8"><project width="640" height="360" fps="10" duration="1"/><composition><camera id="c">{xml}</camera></composition></scene>"#
        );
        let d = sr_model::load_str(&doc, &sr_model::LoadOptions::without_assets()).unwrap();
        let g = sr_eval::Evaluator::new(&d, &Default::default()).unwrap().evaluate(0.0);
        let c = g.nodes.iter().find(|n| &*n.id == "c").unwrap();
        spec(bar_of(&*c.elem).expect("a scaleBar"))
    }

    fn bar(pixels: f64, label: &str) -> ScaleBar {
        ScaleBar { metres: 1.0, pixels, unit: scale::Unit::M, value: 1.0, label: label.into() }
    }

    #[test]
    fn a_bare_scale_bar_takes_the_schema_s_defaults() {
        let s = spec_from("<scaleBar/>").unwrap();
        assert_eq!(
            s,
            BarSpec {
                max_width: 200,
                units: Units::Auto,
                distance: None,
                x: 24.0,
                y: 24.0,
                thickness: 2.0,
                label: true
            }
        );
    }

    #[test]
    fn a_scale_bar_reads_every_attribute() {
        let s = spec_from(
            r##"<scaleBar maxWidth="320" units="si" distance="1000" x="10" y="5.5" thickness="3" label="false" color="#FF0000"/>"##,
        )
        .unwrap();
        assert_eq!(
            s,
            BarSpec {
                max_width: 320,
                units: Units::Si,
                distance: Some(1000.0),
                x: 10.0,
                y: 5.5,
                thickness: 3.0,
                label: false
            }
        );
    }

    #[test]
    fn a_camera_without_a_scale_bar_has_none() {
        let doc = r#"<scene version="1.8"><project width="64" height="64" fps="10" duration="1"/><composition><camera id="c"/></composition></scene>"#;
        let d = sr_model::load_str(doc, &sr_model::LoadOptions::without_assets()).unwrap();
        let g = sr_eval::Evaluator::new(&d, &Default::default()).unwrap().evaluate(0.0);
        let c = g.nodes.iter().find(|n| &*n.id == "c").unwrap();
        assert!(bar_of(&*c.elem).is_none());
    }

    #[test]
    fn the_depth_is_the_distance_then_the_target_then_the_surface_then_far() {
        let s = |distance| BarSpec { distance, ..spec_from("<scaleBar/>").unwrap() };
        let never = || -> Option<f64> { panic!("the surface is not cast when the depth is known") };
        assert_eq!(measure(&s(Some(7.0)), Some(8.0), never, 100.0), (Measured::Distance, 7.0));
        assert_eq!(measure(&s(None), Some(8.0), never, 100.0), (Measured::Target, 8.0));
        assert_eq!(measure(&s(None), None, || Some(9.0), 100.0), (Measured::Surface, 9.0));
        assert_eq!(measure(&s(None), None, || None, 100.0), (Measured::Far, 100.0));
    }

    #[test]
    fn the_bars_are_the_kernel_s() {
        let s = spec_from(r#"<scaleBar maxWidth="300"/>"#).unwrap();
        let focal = scale::focal_pixels(60.0, 640.0).unwrap();
        assert_eq!(
            perspective(&s, 5000.0, focal, 100.0).unwrap(),
            scale::scale_bar_at(5000.0, focal, 100.0, 300, Units::Auto).unwrap()
        );
        let ortho = scale::scale_bar(scale::pixels_per_metre_ortho(720.0, 360.0, 100.0).unwrap(), 300, Units::Auto);
        assert_eq!(orthographic(&s, 720.0, 360.0, 100.0).unwrap(), ortho.unwrap());
        assert!(perspective(&s, 0.0, focal, 100.0).is_err(), "a depth of 0 has no bar");
    }

    #[test]
    fn the_bar_rises_from_its_point_above_the_frame_s_bottom_left() {
        let s = spec_from(r#"<scaleBar x="10" y="20" thickness="3"/>"#).unwrap();
        assert_eq!(rect(&s, &bar(150.5, "1 m"), [640.0, 360.0]), [10.0, 337.0, 160.5, 340.0]);
    }

    #[test]
    fn the_label_sits_on_the_bar_s_left_above_it_and_is_left_out_on_request() {
        let mut lib = FontLib::new(true);
        let white = Paint::Solid { rgba: [1.0; 4], srgb: false };
        let s = spec_from(r#"<scaleBar x="10" y="20" thickness="3"/>"#).unwrap();
        let drawn = scene(&mut lib, &s, &bar(150.0, "100 mm"), [640.0, 360.0], white.clone()).unwrap();
        assert!(drawn.cmds.len() > 1, "the bar and its glyphs");
        let label = Scene { cmds: drawn.cmds[1..].to_vec() };
        let [x0, y0, x1, y1] = label.bounds().0;
        assert!((x0 - 10.0).abs() < 1e-9, "left-aligned on the bar: {x0}");
        assert!((y1 - (337.0 - LABEL_GAP_PX)).abs() < 1e-9, "its gap above the bar: {y1}");
        assert!(y1 - y0 > 0.5 * LABEL_PX && y1 - y0 <= 1.5 * LABEL_PX, "about a label high: {}", y1 - y0);
        assert!(x1 > x0 + LABEL_PX, "several glyphs wide: {}", x1 - x0);
        let bare = spec_from(r#"<scaleBar x="10" y="20" thickness="3" label="false"/>"#).unwrap();
        let drawn = scene(&mut lib, &bare, &bar(150.0, "100 mm"), [640.0, 360.0], white.clone()).unwrap();
        assert_eq!(drawn.cmds.len(), 1, "the bar alone");
        assert_eq!(drawn.bounds().0, [10.0, 337.0, 160.0, 340.0]);
        let refused = scene(&mut FontLib::new(false), &s, &bar(150.0, "100 mm"), [640.0, 360.0], white);
        assert!(refused.unwrap_err().0.contains("no font draws"), "a label without a font is refused");
    }

    fn quad(z: f32) -> Vec<Vertex> {
        [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]
            .map(|[x, y]| Vertex { pos: [x, y, z], ..bytemuck::Zeroable::zeroed() })
            .to_vec()
    }

    #[test]
    fn the_centre_ray_finds_the_nearest_surface_within_the_planes() {
        let idx = [0u32, 1, 2, 0, 2, 3];
        let (near_quad, far_quad) = (quad(0.0), quad(0.0));
        let eye = DVec3::new(0.0, 0.0, -10.0);
        let place = |z: f32, s: f32| {
            Mat4::from_translation(glam::Vec3::new(0.0, 0.0, z)) * Mat4::from_scale(glam::Vec3::splat(s))
        };
        let meshes = || [(place(30.0, 5.0), &near_quad[..], &idx[..]), (place(90.0, 50.0), &far_quad[..], &idx[..])];
        assert_eq!(centre_surface(eye, DVec3::Z, 1.0, 1000.0, meshes()), Some(40.0));
        assert_eq!(centre_surface(eye, DVec3::Z, 50.0, 1000.0, meshes()), Some(100.0), "nearer than near is not seen");
        assert_eq!(centre_surface(eye, DVec3::Z, 1.0, 39.0, meshes()), None, "beyond far is not seen");
        assert_eq!(centre_surface(eye, DVec3::X, 1.0, 1000.0, meshes()), None, "nothing to the side");
        let off = [(Mat4::from_translation(glam::Vec3::new(3.0, 0.0, 30.0)), &near_quad[..], &idx[..])];
        assert_eq!(centre_surface(eye, DVec3::Z, 1.0, 1000.0, off), None, "a surface beside the axis");
        let back = [(place(-30.0, 5.0), &near_quad[..], &idx[..])];
        assert_eq!(centre_surface(eye, DVec3::Z, 1.0, 1000.0, back), None, "a surface behind the eye");
    }
}
