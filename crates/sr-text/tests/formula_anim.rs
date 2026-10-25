//! The drawing side of the semantic-tex draft (sr-core srep/srep-0000-semantic-tex.md): a `formulaAnimator`'s
//! offsets on the glyphs of its part about the part's box (Semantics 5, the 0.5 px bar of rule 11), and a
//! `formulaMorph` drawn as moving pairs, morphing pairs and fades (Semantics 2 to 4, 6 to 9).

use sr_text::animate::Props;
use sr_text::formula;
use sr_text::formula_anim::*;
use sr_text::parts::{box_at, MatchBy, PartBox};
use sr_vector::geom::{Rect, Xf};
use sr_vector::scene::{Cmd, Paint};
use std::sync::{Mutex, MutexGuard, OnceLock};

const EQ: &str = r"\part{lhs}{E} = \part{rhs}{m c^2}";
const EQ2: &str = r"\part{lhs}{E} = \part{rhs}{m^2}";
const NEST: &str = r"\part{f}{\frac{\part{n}{a}}{\part{d}{b}}}";
const BOX: [f64; 2] = [640.0, 320.0];

fn lib() -> MutexGuard<'static, sr_text::FontLib> {
    static L: OnceLock<Mutex<sr_text::FontLib>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(sr_text::FontLib::new(true))).lock().unwrap_or_else(|e| e.into_inner())
}

fn white() -> Paint {
    Paint::Solid { rgba: [1.0; 4], srgb: false }
}

fn pixels(d: &sr_text::Drawing) -> Vec<[f32; 4]> {
    let e = sr_vector::tile::encode(&d.scene, [BOX[0] as u32, BOX[1] as u32]);
    sr_vector::tile::render_cpu(&e, &|_, _, _| [1.0; 4])
}

fn anim(part: &str, props: Props) -> (String, Props) {
    (part.to_string(), props)
}

fn corners(b: PartBox) -> [[f64; 2]; 4] {
    [[b.x, b.y], [b.x + b.w, b.y], [b.x, b.y + b.h], [b.x + b.w, b.y + b.h]]
}

#[track_caller]
fn maps(xf: &Xf, from: [f64; 2], to: [f64; 2], tol: f64) {
    let q = xf.apply(sr_vector::geom::p(from[0], from[1]));
    assert!((q.x - to[0]).abs() <= tol && (q.y - to[1]).abs() <= tol, "{from:?} -> ({}, {}), want {to:?}", q.x, q.y);
}

fn pieces_of(t: &formula::Parted, part: usize) -> Vec<usize> {
    (0..t.pieces.len()).filter(|&k| t.in_part(k, part) && !t.pieces[k].path.segs.is_empty()).collect()
}

// ------------------------------------------------------------------------------------------ the animator

#[test]
fn an_animator_moves_its_part_by_x_and_y_rule_11() {
    let mut lib = lib();
    let t = formula::typeset_parts(&mut lib, EQ, 48.0, BOX, 0.1).unwrap();
    let fx = part_fx(&t, &[anim("rhs", Props { x: Some(20.0), y: Some(-40.0), ..Default::default() })]).unwrap();
    let rhs = t.boxes[1].unwrap();
    for k in pieces_of(&t, 1) {
        for c in corners(rhs) {
            maps(&fx[k].xf, c, [c[0] + 20.0, c[1] - 40.0], 0.5);
        }
    }
    // every other glyph stays where the typesetter put it
    for k in (0..t.pieces.len()).filter(|&k| !t.in_part(k, 1)) {
        assert_eq!(fx[k], PieceFx::default());
    }
    // the kit's case: the lhs glyphs are pixel-identical to the plain formula's
    let moved = draw_animated(
        &mut lib,
        EQ,
        48.0,
        BOX,
        white(),
        &[anim("rhs", Props { y: Some(-40.0), ..Default::default() })],
        0.1,
    )
    .unwrap();
    let plain = formula::draw(&mut lib, EQ, 48.0, BOX, white(), 0.1).unwrap();
    let (a, b) = (pixels(&moved), pixels(&plain));
    let x_end = rhs.x.floor() as usize - 1;
    for y in 0..BOX[1] as usize {
        for x in 0..x_end {
            let i = y * BOX[0] as usize + x;
            assert_eq!(a[i], b[i], "({x}, {y})");
        }
    }
    assert_ne!(a, b);
}

#[test]
fn scale_and_rotation_turn_about_the_box_centre() {
    let mut lib = lib();
    let t = formula::typeset_parts(&mut lib, EQ, 48.0, BOX, 0.1).unwrap();
    let rhs = t.boxes[1].unwrap();
    let [cx, cy] = rhs.center();
    let k = pieces_of(&t, 1)[0];
    let fx = part_fx(&t, &[anim("rhs", Props { scale: Some(2.0), ..Default::default() })]).unwrap();
    maps(&fx[k].xf, [cx, cy], [cx, cy], 1e-9);
    maps(&fx[k].xf, [rhs.x, rhs.y], [cx - rhs.w, cy - rhs.h], 1e-9);
    let fx =
        part_fx(&t, &[anim("rhs", Props { scale_x: Some(3.0), scale_y: Some(0.5), ..Default::default() })]).unwrap();
    maps(&fx[k].xf, [rhs.x, rhs.y], [cx - 1.5 * rhs.w, cy - 0.25 * rhs.h], 1e-9);
    let fx = part_fx(&t, &[anim("rhs", Props { rotation: Some(90.0), ..Default::default() })]).unwrap();
    maps(&fx[k].xf, [cx, cy], [cx, cy], 1e-9);
    // y down: a quarter turn clockwise on screen takes the point right of the centre below it
    maps(&fx[k].xf, [cx + 10.0, cy], [cx, cy + 10.0], 1e-9);
}

#[test]
fn the_anchor_is_measured_from_the_box_top_left() {
    let mut lib = lib();
    let t = formula::typeset_parts(&mut lib, EQ, 48.0, BOX, 0.1).unwrap();
    let rhs = t.boxes[1].unwrap();
    let k = pieces_of(&t, 1)[0];
    let fx = part_fx(
        &t,
        &[anim("rhs", Props { scale: Some(2.0), anchor_x: Some(0.0), anchor_y: Some(0.0), ..Default::default() })],
    )
    .unwrap();
    maps(&fx[k].xf, [rhs.x, rhs.y], [rhs.x, rhs.y], 1e-9);
    maps(&fx[k].xf, [rhs.x + rhs.w, rhs.y + rhs.h], [rhs.x + 2.0 * rhs.w, rhs.y + 2.0 * rhs.h], 1e-9);
    // one axis given: the other stays at the centre
    let fx =
        part_fx(&t, &[anim("rhs", Props { scale: Some(2.0), anchor_x: Some(0.0), ..Default::default() })]).unwrap();
    maps(&fx[k].xf, [rhs.x, rhs.center()[1]], [rhs.x, rhs.center()[1]], 1e-9);
}

#[test]
fn animators_on_one_part_combine_in_document_order() {
    let mut lib = lib();
    let t = formula::typeset_parts(&mut lib, EQ, 48.0, BOX, 0.1).unwrap();
    let rhs = t.boxes[1].unwrap();
    let [cx, cy] = rhs.center();
    let k = pieces_of(&t, 1)[0];
    let fx = part_fx(
        &t,
        &[
            anim("rhs", Props { x: Some(10.0), scale: Some(2.0), opacity: Some(0.5), ..Default::default() }),
            anim("rhs", Props { x: Some(5.0), scale: Some(1.5), opacity: Some(0.5), ..Default::default() }),
        ],
    )
    .unwrap();
    // offsets add, factors multiply, as a textAnimator's at amount 1
    maps(&fx[k].xf, [cx, cy], [cx + 15.0, cy], 1e-9);
    maps(&fx[k].xf, [cx + 1.0, cy], [cx + 15.0 + 3.0, cy], 1e-9);
    assert!((fx[k].opacity - 0.25).abs() < 1e-12);
}

#[test]
fn a_nested_part_moves_with_its_container() {
    let mut lib = lib();
    let t = formula::typeset_parts(&mut lib, NEST, 48.0, BOX, 0.1).unwrap();
    let fx = part_fx(
        &t,
        &[
            anim("f", Props { x: Some(10.0), ..Default::default() }),
            anim("n", Props { y: Some(5.0), ..Default::default() }),
        ],
    )
    .unwrap();
    let probe = [100.0, 100.0];
    for k in pieces_of(&t, 1) {
        maps(&fx[k].xf, probe, [110.0, 105.0], 1e-9);
    }
    for k in pieces_of(&t, 2) {
        maps(&fx[k].xf, probe, [110.0, 100.0], 1e-9);
    }
}

#[test]
fn paint_opacity_stroke_and_blur_reach_the_part() {
    let mut lib = lib();
    let red = Paint::Solid { rgba: [1.0, 0.0, 0.0, 1.0], srgb: false };
    let t = formula::typeset_parts(&mut lib, EQ, 48.0, BOX, 0.1).unwrap();
    let fx = part_fx(
        &t,
        &[anim(
            "rhs",
            Props {
                fill: Some(red.clone()),
                stroke: Some(red.clone()),
                stroke_width: Some(2.0),
                opacity: Some(0.5),
                ..Default::default()
            },
        )],
    )
    .unwrap();
    let k = pieces_of(&t, 1)[0];
    assert_eq!(fx[k].fill.as_ref(), Some(&red));
    assert_eq!(fx[k].stroke.as_ref(), Some(&red));
    assert_eq!(fx[k].stroke_width, 2.0);
    let d = draw_pieces(&t, &fx, white(), 0.1);
    let fills: Vec<(&Paint, f64)> = d
        .scene
        .cmds
        .iter()
        .filter_map(|c| match c {
            Cmd::Fill { paint, opacity, .. } => Some((paint, *opacity)),
            _ => None,
        })
        .collect();
    // the rhs glyphs, alike, fill and stroke once at half opacity; the rest white and opaque
    assert_eq!(fills.iter().filter(|(p, o)| **p == red && *o == 0.5).count(), 2);
    assert!(fills.iter().filter(|(p, _)| **p == white()).all(|(_, o)| *o == 1.0));
    // a blurred part draws apart, for the compositor to blur
    let blurred = draw_animated(
        &mut lib,
        EQ,
        48.0,
        BOX,
        white(),
        &[anim("rhs", Props { blur: Some(3.0), ..Default::default() })],
        0.1,
    )
    .unwrap();
    assert_eq!(blurred.blurred.len(), 1);
    assert!((blurred.blurred[0].0 - 3.0).abs() < 1.0);
    let plain = formula::draw(&mut lib, EQ, 48.0, BOX, white(), 0.1).unwrap();
    assert_ne!(pixels(&blurred), pixels(&plain));
}

#[test]
fn a_part_the_formula_lacks_is_an_error() {
    let mut lib = lib();
    let e = draw_animated(&mut lib, EQ, 48.0, BOX, white(), &[anim("nope", Props::default())], 0.1).unwrap_err();
    assert!(e.contains("nope"), "{e}");
}

#[test]
fn an_empty_part_or_no_animator_draws_the_plain_formula() {
    let mut lib = lib();
    let tex = r"x + \part{e}{}";
    let d = draw_animated(
        &mut lib,
        tex,
        48.0,
        BOX,
        white(),
        &[anim("e", Props { x: Some(30.0), ..Default::default() })],
        0.1,
    )
    .unwrap();
    assert_eq!(pixels(&d), pixels(&formula::draw(&mut lib, tex, 48.0, BOX, white(), 0.1).unwrap()));
    let d = draw_animated(&mut lib, EQ, 48.0, BOX, white(), &[], 0.1).unwrap();
    assert_eq!(pixels(&d), pixels(&formula::draw(&mut lib, EQ, 48.0, BOX, white(), 0.1).unwrap()));
}

// ---------------------------------------------------------------------------------------------- the morph

fn bounds(p: &sr_vector::Path) -> PartBox {
    let mut r = Rect::EMPTY;
    for q in p.flatten(0.05) {
        for v in q.pts {
            r.add(v);
        }
    }
    let [x0, y0, x1, y1] = r.0;
    PartBox::new(x0, y0, x1 - x0, y1 - y0)
}

fn morph(from: &str, to: &str, by: MatchBy, p: f64) -> sr_text::Drawing {
    draw_morph(&mut lib(), from, to, by, p, 48.0, BOX, white(), 0.1).unwrap()
}

#[test]
fn the_morph_is_the_from_formula_at_0_and_the_to_formula_at_1() {
    // the box holds both at their natural size: each is placed as the formula asset places it
    let mut lib = lib();
    let (a, b) = (
        formula::draw(&mut lib, EQ, 48.0, BOX, white(), 0.1).unwrap(),
        formula::draw(&mut lib, EQ2, 48.0, BOX, white(), 0.1).unwrap(),
    );
    drop(lib);
    for by in [MatchBy::Name, MatchBy::Tex] {
        assert_eq!(pixels(&morph(EQ, EQ2, by, 0.0)), pixels(&a));
        assert_eq!(pixels(&morph(EQ, EQ2, by, 1.0)), pixels(&b));
        // progress is clamped
        assert_eq!(pixels(&morph(EQ, EQ2, by, -1.0)), pixels(&a));
        assert_eq!(pixels(&morph(EQ, EQ2, by, 2.0)), pixels(&b));
    }
}

#[test]
fn both_formulas_share_the_fit_of_their_union_box() {
    let mut lib = lib();
    let small = [120.0, 60.0];
    let (a, b) = morph_parts(&mut lib, EQ, r"\part{lhs}{E} = \part{rhs}{m c^2 + p^2 c^2}", 48.0, small, 0.1).unwrap();
    // E is typeset the same in both, so it has the same height at either end and moves without scaling
    let (la, lb) = (a.boxes[0].unwrap(), b.boxes[0].unwrap());
    assert!((la.h - lb.h).abs() < 1e-9 && (la.w - lb.w).abs() < 1e-9, "{la:?} {lb:?}");
    // the wider formula fits the box, scaled down
    let wide = b.boxes.iter().flatten().fold(Rect::EMPTY, |r, q| {
        let mut r = r;
        r.add(sr_vector::geom::p(q.x, q.y));
        r.add(sr_vector::geom::p(q.x + q.w, q.y + q.h));
        r
    });
    assert!(wide.0[0] >= -0.5 && wide.0[2] <= small[0] + 0.5, "{wide:?}");
    let alone = formula::typeset_parts(&mut lib, EQ, 48.0, small, 0.1).unwrap();
    assert!(la.h < alone.boxes[0].unwrap().h - 1.0, "the shared fit is the wider formula's");
}

#[test]
fn a_moving_pair_follows_the_box_path() {
    let mut lib = lib();
    let swapped = r"\part{rhs}{m c^2} = \part{lhs}{E}";
    let (a, b) = morph_parts(&mut lib, EQ, swapped, 48.0, BOX, 0.1).unwrap();
    let pieces = morph_pieces(&a, &b, MatchBy::Name, 0.5);
    for (i, j) in [(0, 1), (1, 0)] {
        let want = box_at(a.boxes[i].unwrap(), b.boxes[j].unwrap(), 0.5);
        let mut r = Rect::EMPTY;
        for pc in pieces.iter().filter(|pc| pc.from == Some(i)) {
            assert_eq!(pc.opacity, 1.0);
            let q = bounds(&pc.path);
            r.add(sr_vector::geom::p(q.x, q.y));
            r.add(sr_vector::geom::p(q.x + q.w, q.y + q.h));
        }
        let [x0, y0, x1, y1] = r.0;
        for (got, w) in [(x0, want.x), (y0, want.y), (x1, want.x + want.w), (y1, want.y + want.h)] {
            assert!((got - w).abs() < 0.5, "part {i}: {:?} want {want:?}", r.0);
        }
    }
}

#[test]
fn a_morphing_pair_morphs_its_centred_outlines_along_the_centre_path() {
    let mut lib = lib();
    let (a, b) = morph_parts(&mut lib, EQ, EQ2, 48.0, BOX, 0.1).unwrap();
    let p = 0.4;
    let pieces = morph_pieces(&a, &b, MatchBy::Name, p);
    let morphed: Vec<_> = pieces.iter().filter(|pc| pc.from == Some(1) && pc.to == Some(1)).collect();
    assert_eq!(morphed.len(), 1);
    let (ba, bb) = (a.boxes[1].unwrap(), b.boxes[1].unwrap());
    let c = box_at(ba, bb, p).center();
    let polys = |t: &formula::Parted, part: usize, at: [f64; 2]| -> Vec<Vec<[f64; 2]>> {
        pieces_of(t, part)
            .into_iter()
            .flat_map(|k| t.pieces[k].path.flatten(MORPH_TOL))
            .filter(|q| !q.pts.is_empty())
            .map(|q| q.pts.iter().map(|v| [v.x - at[0], v.y - at[1]]).collect())
            .collect()
    };
    let want = sr_vector::morph::morph_path(
        &polys(&a, 1, ba.center()),
        &polys(&b, 1, bb.center()),
        p,
        sr_vector::morph::MORPH_SAMPLES,
    )
    .unwrap();
    let got: Vec<Vec<[f64; 2]>> =
        morphed[0].path.flatten(MORPH_TOL).iter().map(|q| q.pts.iter().map(|v| [v.x, v.y]).collect()).collect();
    assert_eq!(got.len(), want.len());
    for (g, w) in got.iter().zip(&want) {
        assert_eq!(g.len(), w.len());
        for (q, r) in g.iter().zip(w) {
            assert!((q[0] - (r[0] + c[0])).abs() < 1e-6 && (q[1] - (r[1] + c[1])).abs() < 1e-6, "{q:?} {r:?}");
        }
    }
    // E moves (here in place) and stays opaque
    assert!(pieces.iter().filter(|pc| pc.from == Some(0)).all(|pc| pc.opacity == 1.0));
}

#[test]
fn unpaired_parts_and_untagged_glyphs_fade() {
    let mut lib = lib();
    let renamed = r"\part{x}{E} = \part{y}{m c^2}";
    let (a, b) = morph_parts(&mut lib, EQ, renamed, 48.0, BOX, 0.1).unwrap();
    let p = 0.25;
    let pieces = morph_pieces(&a, &b, MatchBy::Name, p);
    // by name nothing pairs: every glyph of the source fades out, every glyph of the target fades in, "=" too
    let drawn = |t: &formula::Parted| t.pieces.iter().filter(|pc| !pc.path.segs.is_empty()).count();
    assert_eq!(pieces.iter().filter(|pc| pc.to.is_none() && pc.opacity == 0.75).count(), drawn(&a));
    assert_eq!(pieces.iter().filter(|pc| pc.from.is_none() && pc.opacity == 0.25).count(), drawn(&b));
    // by TeX both parts pair and move in place: midway is the formula itself, where only "=" cross-fades
    let by_tex = morph_pieces(&a, &b, MatchBy::Tex, p);
    assert_eq!(by_tex.iter().filter(|pc| pc.opacity == 1.0).count(), drawn(&a) - 1);
    drop(lib);
    assert_ne!(pixels(&morph(EQ, renamed, MatchBy::Name, 0.5)), pixels(&morph(EQ, renamed, MatchBy::Tex, 0.5)));
}

#[test]
fn malformed_formulas_are_errors() {
    let mut lib = lib();
    assert!(draw_morph(&mut lib, r"\part{a}x", EQ, MatchBy::Name, 0.5, 48.0, BOX, white(), 0.1).is_err());
    assert!(draw_morph(&mut lib, EQ, r"\frac{", MatchBy::Name, 0.5, 48.0, BOX, white(), 0.1).is_err());
}
