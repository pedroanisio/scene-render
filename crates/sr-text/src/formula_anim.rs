//! The drawing side of the semantic-tex draft of sr-core (`srep/srep-0000-semantic-tex.md`): a `formulaAnimator`'s
//! offsets on the glyphs of one part (Semantics 5), and a `formulaMorph` drawn as moving pairs, morphing pairs and
//! fades (Semantics 2 to 4, 6 to 9). Every function is a pure function of its arguments (Semantics 12).

use std::collections::HashSet;

use sr_vector::geom::{p, Xf, P};
use sr_vector::morph::{morph_path, MORPH_FLATNESS, MORPH_SAMPLES};
use sr_vector::scene::{Cmd, FillRule, Paint, Scene};
use sr_vector::stroke::{self, Cap, Join, Style as StrokeStyle};
use sr_vector::{Path, Poly};

use crate::animate::Props;
use crate::font::FontLib;
use crate::formula::{self, Parted};
use crate::glyph::Drawing;
use crate::parts::{box_at, match_parts, opacity_in, opacity_out, pair_kind, MatchBy, PairKind, PartBox};

/// The flatness, in formula-image pixels, glyph outlines are flattened with before they are morphed (rule 6: the
/// engine's, the one the path morph of a shape uses).
pub const MORPH_TOL: f64 = MORPH_FLATNESS;

/// What the animators do to one glyph or rule of a formula: its transform in the formula image, and its opacity,
/// paints and blur. The default leaves it as the typesetter put it.
#[derive(Debug, Clone, PartialEq)]
pub struct PieceFx {
    pub xf: Xf,
    pub opacity: f64,
    /// Replaces the formula's colour.
    pub fill: Option<Paint>,
    pub stroke: Option<Paint>,
    pub stroke_width: f64,
    /// Radius in formula-image pixels.
    pub blur: f64,
}

impl Default for PieceFx {
    fn default() -> PieceFx {
        PieceFx { xf: Xf::IDENTITY, opacity: 1.0, fill: None, stroke: None, stroke_width: 0.0, blur: 0.0 }
    }
}

/// The animators of one part, combined as a `textAnimator`'s at a selection amount of 1 with `combine="add"`:
/// offsets add, factors multiply, a later paint or anchor replaces an earlier one.
struct Acc {
    anchor: [f64; 2],
    dx: f64,
    dy: f64,
    rot: f64,
    sc: f64,
    sx: f64,
    sy: f64,
    op: f64,
    sw: f64,
    blur: f64,
    fill: Option<Paint>,
    stroke: Option<Paint>,
}

impl Acc {
    fn new(b: PartBox) -> Acc {
        Acc {
            anchor: b.center(),
            dx: 0.0,
            dy: 0.0,
            rot: 0.0,
            sc: 1.0,
            sx: 1.0,
            sy: 1.0,
            op: 1.0,
            sw: 0.0,
            blur: 0.0,
            fill: None,
            stroke: None,
        }
    }

    fn add(&mut self, b: PartBox, pr: &Props) {
        // the anchor is measured from the box's top-left; an axis not given stays at the centre
        if let Some(ax) = pr.anchor_x {
            self.anchor[0] = b.x + ax;
        }
        if let Some(ay) = pr.anchor_y {
            self.anchor[1] = b.y + ay;
        }
        self.dx += pr.x.unwrap_or(0.0);
        self.dy += pr.y.unwrap_or(0.0);
        self.rot += pr.rotation.unwrap_or(0.0);
        self.sw += pr.stroke_width.unwrap_or(0.0);
        self.blur += pr.blur.unwrap_or(0.0).max(0.0);
        self.sc *= pr.scale.unwrap_or(1.0);
        self.sx *= pr.scale_x.unwrap_or(1.0);
        self.sy *= pr.scale_y.unwrap_or(1.0);
        self.op *= pr.opacity.unwrap_or(1.0);
        if pr.fill.is_some() {
            self.fill.clone_from(&pr.fill);
        }
        if pr.stroke.is_some() {
            self.stroke.clone_from(&pr.stroke);
        }
    }

    /// Scaled, turned and moved about the anchor, in a textAnimator's order.
    fn xf(&self) -> Xf {
        let [ax, ay] = self.anchor;
        Xf::translate(ax + self.dx, ay + self.dy)
            .mul(&Xf::rotate(self.rot))
            .mul(&Xf::scale(self.sc * self.sx, self.sc * self.sy))
            .mul(&Xf::translate(-ax, -ay))
    }
}

/// Rule 5: the effect of `anims` (each a part name and the animator's values at this frame, in document order) on
/// every piece of `t`. A piece takes the animators of its own part, then those of each part containing it, outwards:
/// a nested part moves with its container. A part that produces nothing is not animated; a name that is no part of
/// the formula is an error.
pub fn part_fx(t: &Parted, anims: &[(String, Props)]) -> Result<Vec<PieceFx>, String> {
    let mut acc: Vec<Option<Acc>> = (0..t.parts.len()).map(|_| None).collect();
    for (name, pr) in anims {
        let i = t
            .parts
            .iter()
            .position(|q| q.name == *name)
            .ok_or_else(|| format!("formulaAnimator: the formula has no part {name:?}"))?;
        let Some(b) = t.boxes[i] else { continue };
        acc[i].get_or_insert_with(|| Acc::new(b)).add(b, pr);
    }
    Ok((0..t.pieces.len())
        .map(|k| {
            let mut fx = PieceFx::default();
            let mut at = t.pieces[k].part;
            while let Some(i) = at {
                if let Some(a) = &acc[i] {
                    fx.xf = a.xf().mul(&fx.xf);
                    fx.opacity *= a.op;
                    fx.stroke_width += a.sw;
                    fx.blur += a.blur;
                    // the innermost part's paint wins
                    if fx.fill.is_none() {
                        fx.fill.clone_from(&a.fill);
                    }
                    if fx.stroke.is_none() {
                        fx.stroke.clone_from(&a.stroke);
                    }
                }
                at = t.parent[i];
            }
            fx.opacity = fx.opacity.clamp(0.0, 1.0);
            fx
        })
        .collect())
}

/// Blur radii share a scene per step of √2, as glyph blurs do.
fn blur_bucket(r: f64) -> i32 {
    (r.log2() * 2.0).round() as i32
}

fn fill_run(s: &mut Scene, path: &Path, e: &PieceFx, color: &Paint, tol: f64) {
    s.fill(path, FillRule::NonZero, e.fill.clone().unwrap_or_else(|| color.clone()), e.opacity, tol);
    // a stroke paint strokes the outline centred, at least 1 px wide, as a textAnimator's on an unstroked glyph
    if let Some(sp) = &e.stroke {
        let style = StrokeStyle { width: e.stroke_width.max(1.0), cap: Cap::Butt, join: Join::Round, miter_limit: 4.0 };
        let polys = stroke::stroke(&path.flatten(tol), &style, tol);
        if !polys.is_empty() && e.opacity > 0.0 {
            s.cmds.push(Cmd::Fill { polys, rule: FillRule::NonZero, paint: sp.clone(), opacity: e.opacity });
        }
    }
}

/// The pieces of `t` drawn with their effects in `color`; consecutive pieces with the same effect fill together, so
/// that a formula without effects draws exactly as [`formula::draw`] draws it. Blurred pieces go to
/// [`Drawing::blurred`] for the compositor.
pub fn draw_pieces(t: &Parted, fx: &[PieceFx], color: Paint, tol: f64) -> Drawing {
    let mut d = Drawing::default();
    let mut blurred: Vec<(i32, Scene)> = Vec::new();
    let mut k = 0;
    while k < t.pieces.len() {
        let e = &fx[k];
        let mut j = k + 1;
        while j < t.pieces.len() && fx[j] == *e {
            j += 1;
        }
        let mut path = Path::default();
        for pc in &t.pieces[k..j] {
            path.extend(&pc.path.transform(&e.xf));
        }
        if e.blur >= 0.5 {
            let b = blur_bucket(e.blur);
            let at = match blurred.iter().position(|(q, _)| *q == b) {
                Some(at) => at,
                None => {
                    blurred.push((b, Scene::default()));
                    blurred.len() - 1
                }
            };
            fill_run(&mut blurred[at].1, &path, e, &color, tol);
        } else {
            fill_run(&mut d.scene, &path, e, &color, tol);
        }
        k = j;
    }
    d.blurred = blurred.into_iter().map(|(b, s)| (libm::exp2(b as f64 / 2.0), s)).collect();
    d
}

/// A formula with `formulaAnimator`s: `tex` typeset as [`formula::draw`] typesets it and drawn with [`part_fx`].
pub fn draw_animated(
    lib: &mut FontLib,
    tex: &str,
    size: f64,
    size_box: [f64; 2],
    color: Paint,
    anims: &[(String, Props)],
    tol: f64,
) -> Result<Drawing, String> {
    if anims.is_empty() {
        return formula::draw(lib, tex, size, size_box, color, tol);
    }
    let t = formula::typeset_parts(lib, tex, size, size_box, tol)?;
    let fx = part_fx(&t, anims)?;
    Ok(draw_pieces(&t, &fx, color, tol))
}

/// The two formulas of a morph typeset at `size`, each centred in the `size_box` image as a formula is, and scaled
/// down by the one fit of their union box, so that equal TeX has equal glyphs at either end.
pub fn morph_parts(
    lib: &mut FontLib,
    from: &str,
    to: &str,
    size: f64,
    size_box: [f64; 2],
    tol: f64,
) -> Result<(Parted, Parted), String> {
    let ([wa, ha], [wb, hb]) = (formula::measure(lib, from, size)?, formula::measure(lib, to, size)?);
    let fit = (size_box[0] / wa.max(wb).max(1e-9)).min(size_box[1] / ha.max(hb).max(1e-9)).min(1.0);
    Ok((
        formula::typeset_parts_fit(lib, from, size, size_box, fit, tol)?,
        formula::typeset_parts_fit(lib, to, size, size_box, fit, tol)?,
    ))
}

/// A run of outline of a morph at some progress: what is drawn, at what opacity, and the outermost part of the source
/// and of the target it comes from (`None` for a glyph of no part, or the side it does not come from).
#[derive(Debug, Clone)]
pub struct MorphPiece {
    pub path: Path,
    pub opacity: f64,
    pub from: Option<usize>,
    pub to: Option<usize>,
}

fn outer(t: &Parted, k: usize) -> Option<usize> {
    let mut at = t.pieces[k].part?;
    while let Some(q) = t.parent[at] {
        at = q;
    }
    Some(at)
}

/// The flattened outlines of part `part` (nested parts included), less `c`.
fn part_polys(t: &Parted, part: usize, c: [f64; 2]) -> Vec<Vec<[f64; 2]>> {
    (0..t.pieces.len())
        .filter(|&k| t.in_part(k, part))
        .flat_map(|k| t.pieces[k].path.flatten(MORPH_TOL))
        .filter(|q| !q.pts.is_empty())
        .map(|q| q.pts.iter().map(|v| [v.x - c[0], v.y - c[1]]).collect())
        .collect()
}

fn part_path(t: &Parted, part: usize) -> Path {
    let mut path = Path::default();
    for k in (0..t.pieces.len()).filter(|&k| t.in_part(k, part)) {
        path.extend(&t.pieces[k].path);
    }
    path
}

/// Rules 2 to 4 and 6 to 9 at progress `p` (clamped to [0, 1]) between the typeset formulas `a` and `b` of
/// [`morph_parts`]. The outermost parts of `a` and `b` pair by `by`; a pair whose TeX is equal moves its glyphs
/// along the box path, a pair whose TeX differs morphs its outlines, centred on the box path, as one path; the
/// parts of `a` left over and its glyphs of no part fade out, those of `b` fade in. The source's pieces come in
/// layout order, then the target's.
pub fn morph_pieces(a: &Parted, b: &Parted, by: MatchBy, p: f64) -> Vec<MorphPiece> {
    let p = p.clamp(0.0, 1.0);
    // a pair whose parts both produce something; with one side empty, the other fades as if unpaired
    let mut pair: Vec<Option<(usize, PartBox, PartBox)>> = vec![None; a.parts.len()];
    let mut paired_b = HashSet::new();
    for (x, y) in match_parts(&a.parts, &b.parts, by).matched {
        if let (Some(ba), Some(bb)) = (a.boxes[x.index], b.boxes[y.index]) {
            pair[x.index] = Some((y.index, ba, bb));
            paired_b.insert(y.index);
        }
    }
    let mut out = Vec::new();
    let mut morphed = HashSet::new();
    for k in 0..a.pieces.len() {
        if a.pieces[k].path.segs.is_empty() {
            continue;
        }
        let o = outer(a, k);
        let Some((i, (j, ba, bb))) = o.and_then(|i| pair[i].map(|q| (i, q))) else {
            out.push(MorphPiece { path: a.pieces[k].path.clone(), opacity: opacity_out(p), from: o, to: None });
            continue;
        };
        let (ca, c) = (ba.center(), box_at(ba, bb, p).center());
        match pair_kind(&a.parts[i].tex, &b.parts[j].tex) {
            PairKind::Move => {
                // rule 3's motion: the centre along the box path, scaled by the heights about the source centre
                let s = if ba.h > 0.0 { 1.0 + (bb.h / ba.h - 1.0) * p } else { 1.0 };
                let xf = Xf::translate(c[0], c[1]).mul(&Xf::scale(s, s)).mul(&Xf::translate(-ca[0], -ca[1]));
                out.push(MorphPiece {
                    path: a.pieces[k].path.transform(&xf),
                    opacity: 1.0,
                    from: Some(i),
                    to: Some(j),
                });
            }
            PairKind::Morph => {
                if !morphed.insert(i) {
                    continue;
                }
                let path = if p == 0.0 {
                    part_path(a, i)
                } else if p == 1.0 {
                    part_path(b, j)
                } else {
                    match morph_path(&part_polys(a, i, ca), &part_polys(b, j, bb.center()), p, MORPH_SAMPLES) {
                        Some(m) => sr_vector::path::polys_to_path(
                            &m.iter()
                                .map(|q| Poly { pts: q.iter().map(|v| p_at(*v, c)).collect(), closed: true })
                                .collect::<Vec<_>>(),
                        ),
                        // a side without outline (rule 9's hold): the source holds until the end
                        None => part_path(a, i),
                    }
                };
                out.push(MorphPiece { path, opacity: 1.0, from: Some(i), to: Some(j) });
            }
        }
    }
    for k in 0..b.pieces.len() {
        if b.pieces[k].path.segs.is_empty() {
            continue;
        }
        let o = outer(b, k);
        if o.is_some_and(|j| paired_b.contains(&j)) {
            continue;
        }
        out.push(MorphPiece { path: b.pieces[k].path.clone(), opacity: opacity_in(p), from: None, to: o });
    }
    out
}

fn p_at(v: [f64; 2], c: [f64; 2]) -> P {
    p(v[0] + c[0], v[1] + c[1])
}

/// A `formulaMorph` at progress `p` in `color`: exactly the `from` formula at 0 and the `to` formula at 1 (as
/// [`formula::draw`] draws them, at the shared fit of [`morph_parts`]), and [`morph_pieces`] between, consecutive
/// pieces of one opacity filled together.
#[allow(clippy::too_many_arguments)]
pub fn draw_morph(
    lib: &mut FontLib,
    from: &str,
    to: &str,
    by: MatchBy,
    p: f64,
    size: f64,
    size_box: [f64; 2],
    color: Paint,
    tol: f64,
) -> Result<Drawing, String> {
    let (a, b) = morph_parts(lib, from, to, size, size_box, tol)?;
    let p = p.clamp(0.0, 1.0);
    let mut d = Drawing::default();
    if p == 0.0 || p == 1.0 {
        let mut path = Path::default();
        for pc in &(if p == 0.0 { a } else { b }).pieces {
            path.extend(&pc.path);
        }
        d.scene.fill(&path, FillRule::NonZero, color, 1.0, tol);
        return Ok(d);
    }
    let pieces = morph_pieces(&a, &b, by, p);
    let mut k = 0;
    while k < pieces.len() {
        let mut path = Path::default();
        let mut j = k;
        while j < pieces.len() && pieces[j].opacity == pieces[k].opacity {
            path.extend(&pieces[j].path);
            j += 1;
        }
        d.scene.fill(&path, FillRule::NonZero, color.clone(), pieces[k].opacity, tol);
        k = j;
    }
    Ok(d)
}
