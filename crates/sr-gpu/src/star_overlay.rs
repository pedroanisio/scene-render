//! Rule 5 of the star-fields draft drawn: a dome's constellation labels and boundaries as a coverage mask over the
//! rendered frame. Labels are the abbreviations in the default sans-serif face, centred on their points; boundaries
//! are lines through the projected boundary points. Both are rasterised on the CPU by the fine stage the GPU runs
//! ([`sr_vector::tile::render_cpu_rows`]), tile rows in parallel, and drawn in one grey behind every surface.

use glam::{Mat4, Vec3, Vec4};
use rayon::prelude::*;
use sr_text::layout::Align;
use sr_text::{chart, FontLib, Style};
use sr_vector::geom::{p, Xf, P};
use sr_vector::path::Path;
use sr_vector::scene::{Cmd, FillRule, Paint, Scene};
use sr_vector::stroke::{stroke, Cap, Join, Style as Stroke};

/// The overlay's grey as a fraction of the dome's gain (`label_value` of the draft).
pub const GREY: f32 = 0.18;
/// A label's height, output pixels (`LABEL_HEIGHT_PX`).
pub const LABEL_PX: f64 = 14.0;
/// A boundary's width, output pixels (`BOUNDARY_WIDTH_PX`).
pub const LINE_PX: f64 = 1.0;

/// Flattening tolerance, rendered pixels.
const TOL: f64 = 0.05;

/// What a field's overlay draws, in world directions.
pub struct Marks<'a> {
    pub labels: &'a [(&'static str, Vec3)],
    pub lines: &'a [Vec<Vec3>],
}

/// The pixel of world direction `d` through `vp`, in a frame of `size` pixels, when it is in front of the camera.
fn project(vp: &Mat4, d: Vec3, size: [u32; 2]) -> Option<P> {
    let c = *vp * Vec4::new(d.x, d.y, d.z, 0.0);
    // the direction is a unit vector: w is the cosine of its angle from the axis, so a margin keeps the pixel finite
    (c.w > 0.02 * d.length())
        .then(|| p((c.x / c.w + 1.0) as f64 * 0.5 * size[0] as f64, (1.0 - c.y / c.w) as f64 * 0.5 * size[1] as f64))
}

/// The coverage of `marks` seen through `vp` in a frame of `size` pixels, `scale` rendered pixels to an output pixel,
/// one byte a pixel, rows first.
pub fn coverage(lib: &mut FontLib, marks: &Marks, vp: &Mat4, size: [u32; 2], scale: f64) -> Vec<u8> {
    let [w, h] = size.map(|v| v as f64);
    let mut scene = Scene::default();
    let white = || Paint::Solid { rgba: [1.0; 4], srgb: false };
    // a piece is drawn when it may touch the frame
    let margin = 2.0 * scale;
    let outside = |a: P, b: P| {
        (a.x < -margin && b.x < -margin)
            || (a.y < -margin && b.y < -margin)
            || (a.x > w + margin && b.x > w + margin)
            || (a.y > h + margin && b.y > h + margin)
    };
    let mut path = Path::default();
    for line in marks.lines {
        let mut last: Option<P> = None;
        for &d in line {
            let q = project(vp, d, size);
            match (last, q) {
                (Some(a), Some(b)) if !outside(a, b) => {
                    path.move_to(a);
                    path.line_to(b);
                }
                _ => {}
            }
            last = q;
        }
    }
    let style = Stroke { width: LINE_PX * scale, cap: Cap::Butt, join: Join::Bevel, miter_limit: 4.0 };
    let polys = stroke(&path.flatten(TOL), &style, TOL);
    if !polys.is_empty() {
        scene.cmds.push(Cmd::Fill { polys, rule: FillRule::NonZero, paint: white(), opacity: 1.0 });
    }
    let text = Style { size: LABEL_PX * scale, color: Some(white()), ..Default::default() };
    for &(abbr, d) in marks.labels {
        let Some(q) = project(vp, d, size) else { continue };
        // a label reaches at most a few heights from its point
        let (mx, my) = (4.0 * LABEL_PX * scale, LABEL_PX * scale);
        if q.x < -mx || q.x > w + mx || q.y < -my || q.y > h + my {
            continue;
        }
        let glyphs = chart::text(lib, abbr, &text, 0.0, 0.0, Align::Start, f64::INFINITY, TOL).scene;
        let [x0, y0, x1, y1] = glyphs.bounds().0;
        if !(x0 <= x1 && y0 <= y1) {
            continue;
        }
        // centred on its point
        scene.extend(glyphs.transformed(&Xf::translate(q.x - (x0 + x1) / 2.0, q.y - (y0 + y1) / 2.0)));
    }
    if scene.cmds.is_empty() {
        return vec![0; (size[0] * size[1]) as usize];
    }
    let e = sr_vector::tile::encode(&scene, size);
    let paint = |_: u32, _: f32, _: f32| [1.0f32; 4];
    let band = (size[0] * sr_vector::tile::TILE) as usize;
    let mut out = vec![0u8; (size[0] * size[1]) as usize];
    out.par_chunks_mut(band).enumerate().for_each(|(ty, dst)| {
        let rows = sr_vector::tile::render_cpu_rows(&e, ty as u32..ty as u32 + 1, &paint);
        for (o, v) in dst.iter_mut().zip(rows) {
            *o = (v[3].clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    });
    out
}
