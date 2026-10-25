//! The tagged formula parts of the semantic-tex draft (sr-core srep/srep-0000-semantic-tex.md, Semantics 1 to 4):
//! `\part{name}{body}` found with its depth and stripped for the typesetter, parts matched between two formulas by
//! name or by TeX, and the motion of a matched part and the fade of an unmatched one, checked against the draft's
//! numeric table.

use sr_text::parts::*;

const EQ: &str = r"\part{lhs}{E} = \part{rhs}{m c^2}";
const NEST: &str = r"\part{f}{\frac{\part{n}{a}}{\part{d}{b}}}";
const A3: &str = r"\part{a}{x} + \part{b}{y} = \part{c}{z}";
const B3: &str = r"\part{c}{z} - \part{b}{y} = \part{d}{w}";

fn ps(tex: &str) -> Vec<Part> {
    parts(tex).unwrap_or_else(|e| panic!("{tex}: {e}"))
}

#[track_caller]
fn close(a: f64, b: f64) {
    assert!((a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1e-300), "{a} != {b}");
}

#[test]
fn part_names_and_tex() {
    let p = ps(EQ);
    assert_eq!(p.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["lhs", "rhs"]);
    assert_eq!(p.iter().map(|p| p.tex.as_str()).collect::<Vec<_>>(), ["E", "m c^2"]);
    assert_eq!(p.iter().map(|p| p.index).collect::<Vec<_>>(), [0, 1]);
}

#[test]
fn nested_parts_have_depths_and_the_outer_keeps_its_tags() {
    let p = ps(NEST);
    assert_eq!(p.iter().map(|p| (p.name.as_str(), p.depth)).collect::<Vec<_>>(), [("f", 0), ("n", 1), ("d", 1)]);
    assert_eq!(p[0].tex, r"\frac{\part{n}{a}}{\part{d}{b}}");
}

#[test]
fn a_formula_without_tags_has_no_parts() {
    assert!(ps(r"\sqrt{x^2 + y^2}").is_empty());
}

#[test]
fn strip_parts_leaves_the_groups() {
    assert_eq!(strip_parts(EQ).unwrap(), "{E} = {m c^2}");
    assert_eq!(strip_parts(NEST).unwrap(), r"{\frac{{a}}{{b}}}");
    assert_eq!(strip_parts(r"\sqrt{x}").unwrap(), r"\sqrt{x}");
}

#[test]
fn an_escaped_brace_is_not_a_group() {
    let p = ps(r"\part{s}{\{ x \}}");
    assert_eq!(p[0].tex, r"\{ x \}");
}

#[test]
fn malformed_tags_are_errors() {
    for (bad, why) in [
        (r"\part{a}x", "body"),
        (r"\part{a b}{x}", "NMTOKEN"),
        (r"\part{}{x}", "NMTOKEN"),
        (r"\part{a}{x} + \part{a}{y}", "duplicate"),
        (r"\part{a}{x", "unclosed"),
        (r"x}", "stray"),
        (r"\part{a}", "body"),
    ] {
        let e = parts(bad).expect_err(bad);
        assert!(e.contains(why), "{bad}: {e}");
        assert!(strip_parts(bad).is_err(), "{bad}");
    }
}

#[test]
fn match_by_name() {
    let m = match_parts(&ps(A3), &ps(B3), MatchBy::Name);
    let got: Vec<_> = m.matched.iter().map(|(a, b)| (a.name.as_str(), b.name.as_str(), a.tex.as_str())).collect();
    assert_eq!(got, [("b", "b", "y"), ("c", "c", "z")]);
    assert_eq!(m.out.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["a"]);
    assert_eq!(m.incoming.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["d"]);
}

#[test]
fn match_by_tex_pairs_in_order_each_at_most_once() {
    let a = ps(r"\part{p}{x^2} + \part{q}{2x} + \part{r}{x^2}");
    let b = ps(r"\part{u}{x^2} + \part{v}{x^2}");
    let m = match_parts(&a, &b, MatchBy::Tex);
    let got: Vec<_> = m.matched.iter().map(|(a, b)| (a.name.as_str(), b.name.as_str(), a.tex.as_str())).collect();
    assert_eq!(got, [("p", "u", "x^2"), ("r", "v", "x^2")]);
    assert_eq!(m.out.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["q"]);
    assert!(m.incoming.is_empty());
}

#[test]
fn only_outermost_parts_take_part_in_a_morph() {
    // rule 2: a nested part moves with its container
    let m = match_parts(&ps(NEST), &ps(r"\part{n}{a} + \part{f}{b}"), MatchBy::Name);
    assert_eq!(m.matched.iter().map(|(a, _)| a.name.as_str()).collect::<Vec<_>>(), ["f"]);
    assert!(m.out.is_empty());
    assert_eq!(m.incoming.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["n"]);
}

#[test]
fn pair_kinds() {
    assert_eq!(pair_kind("x ^ 2", "x^2"), PairKind::Move);
    assert_eq!(pair_kind("x^2", "y^2"), PairKind::Morph);
}

#[test]
fn box_midway() {
    let b = box_at(PartBox::new(10.0, 20.0, 40.0, 10.0), PartBox::new(100.0, 50.0, 80.0, 20.0), 0.5);
    for (g, w) in [b.x, b.y, b.w, b.h].into_iter().zip([55.0, 35.0, 60.0, 15.0]) {
        close(g, w);
    }
}

#[test]
fn part_transform_midway() {
    let [dx, dy, s] =
        part_transform(PartBox::new(0.0, 0.0, 20.0, 10.0), PartBox::new(100.0, 100.0, 40.0, 20.0), 0.5).unwrap();
    close(dx, 55.0);
    close(dy, 52.5);
    close(s, 1.5);
}

#[test]
fn part_transform_needs_boxes_with_area() {
    assert!(part_transform(PartBox::new(0.0, 0.0, 0.0, 10.0), PartBox::new(0.0, 0.0, 4.0, 4.0), 0.5).is_err());
    assert!(part_transform(PartBox::new(0.0, 0.0, 4.0, 4.0), PartBox::new(0.0, 0.0, 4.0, 0.0), 0.5).is_err());
}

#[test]
fn fades() {
    close(opacity_out(0.25), 0.75);
    close(opacity_in(0.25), 0.25);
    // p is clamped
    close(opacity_out(1.5), 0.0);
    close(opacity_in(-1.0), 0.0);
}

// ------------------------------------------------------------------------------ the typesetter (rule 1, rule 3)

use sr_text::formula;
use sr_vector::scene::Paint;
use std::sync::{Mutex, MutexGuard, OnceLock};

fn lib() -> MutexGuard<'static, sr_text::FontLib> {
    static L: OnceLock<Mutex<sr_text::FontLib>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(sr_text::FontLib::new(true))).lock().unwrap_or_else(|e| e.into_inner())
}

fn white() -> Paint {
    Paint::Solid { rgba: [1.0; 4], srgb: false }
}

fn pixels(d: &sr_text::Drawing, size: [u32; 2]) -> Vec<[f32; 4]> {
    let e = sr_vector::tile::encode(&d.scene, size);
    sr_vector::tile::render_cpu(&e, &|_, _, _| [1.0; 4])
}

#[test]
fn a_tagged_formula_draws_exactly_as_the_untagged_one() {
    let mut lib = lib();
    for tex in [
        EQ,
        NEST,
        r"\part{s}{x}^2 + \sqrt{\part{r}{y}}",
        r"\begin{pmatrix}\part{a}{1} & 2\\3 & \part{b}{4}\end{pmatrix}",
    ] {
        let tagged =
            formula::draw(&mut lib, tex, 48.0, [320.0, 160.0], white(), 0.1).unwrap_or_else(|e| panic!("{tex}: {e}"));
        let plain = formula::draw(&mut lib, &strip_parts(tex).unwrap(), 48.0, [320.0, 160.0], white(), 0.1).unwrap();
        assert!(pixels(&tagged, [320, 160]) == pixels(&plain, [320, 160]), "{tex}");
        let [w, h] = formula::measure(&mut lib, tex, 48.0).unwrap();
        assert_eq!([w, h], formula::measure(&mut lib, &strip_parts(tex).unwrap(), 48.0).unwrap(), "{tex}");
    }
}

#[test]
fn every_glyph_belongs_to_its_innermost_part() {
    let mut lib = lib();
    let t = formula::typeset_parts(&mut lib, EQ, 48.0, [320.0, 160.0], 0.1).unwrap();
    assert_eq!(t.parts.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["lhs", "rhs"]);
    // E; =; m, c, 2
    let owners: Vec<Option<usize>> = t.pieces.iter().filter(|p| !p.path.segs.is_empty()).map(|p| p.part).collect();
    assert_eq!(owners, [Some(0), None, Some(1), Some(1), Some(1)]);
    let (l, r) = (t.boxes[0].unwrap(), t.boxes[1].unwrap());
    assert!(l.x + l.w < r.x, "{l:?} {r:?}");
    // inside the image
    for b in [l, r] {
        assert!(b.x >= 0.0 && b.y >= 0.0 && b.x + b.w <= 320.0 && b.y + b.h <= 160.0, "{b:?}");
    }
}

#[test]
fn a_nested_part_lies_in_its_container() {
    let mut lib = lib();
    let t = formula::typeset_parts(&mut lib, NEST, 48.0, [320.0, 160.0], 0.1).unwrap();
    assert_eq!(t.parent, [None, Some(0), Some(0)]);
    let f = t.boxes[0].unwrap();
    for i in [1, 2] {
        let b = t.boxes[i].unwrap();
        assert!(b.x >= f.x && b.y >= f.y && b.x + b.w <= f.x + f.w && b.y + b.h <= f.y + f.h, "{f:?} {b:?}");
    }
    // the numerator above the denominator
    assert!(t.boxes[1].unwrap().y + t.boxes[1].unwrap().h < t.boxes[2].unwrap().y);
    // the fraction rule is f's own; a and b are n's and d's
    assert!(t.pieces.iter().any(|p| p.part == Some(0)));
    assert!(t.in_part(t.pieces.iter().position(|p| p.part == Some(1)).unwrap(), 0));
    assert!(!t.in_part(t.pieces.iter().position(|p| p.part == Some(1)).unwrap(), 2));
}

#[test]
fn part_boxes_follow_the_fit() {
    // a box too small for the natural size scales the layout down, and the part boxes with it
    let mut lib = lib();
    let big = formula::typeset_parts(&mut lib, EQ, 48.0, [1000.0, 400.0], 0.1).unwrap();
    let small = formula::typeset_parts(&mut lib, EQ, 48.0, [100.0, 40.0], 0.1).unwrap();
    let [w, h] = formula::measure(&mut lib, EQ, 48.0).unwrap();
    let fit = (100.0 / w).min(40.0 / h);
    assert!(fit < 1.0);
    let (b, s) = (big.boxes[1].unwrap(), small.boxes[1].unwrap());
    assert!((s.w - b.w * fit).abs() < 0.05 && (s.h - b.h * fit).abs() < 0.05, "{b:?} {s:?} {fit}");
}

#[test]
fn a_part_that_produces_nothing_has_no_box() {
    let mut lib = lib();
    let t = formula::typeset_parts(&mut lib, r"x + \part{e}{}", 48.0, [320.0, 160.0], 0.1).unwrap();
    assert_eq!(t.boxes, [None]);
}

#[test]
fn malformed_tags_are_formula_errors() {
    let mut lib = lib();
    for bad in [r"\part{a}x", r"\part{a}{x} + \part{a}{y}", r"\part{a b}{x}", r"\part{a}"] {
        assert!(formula::draw(&mut lib, bad, 48.0, [100.0, 100.0], white(), 0.1).is_err(), "{bad}");
        assert!(formula::typeset_parts(&mut lib, bad, 48.0, [100.0, 100.0], 0.1).is_err(), "{bad}");
    }
    // \partial is a symbol, not a tag
    assert!(formula::draw(&mut lib, r"\partial x", 48.0, [100.0, 100.0], white(), 0.1).is_ok());
}

/// The ink box of a piece, `[x0, y0, x1, y1]`.
fn ink(t: &formula::Parted, k: usize) -> [f64; 4] {
    let mut r = sr_vector::geom::Rect::EMPTY;
    for poly in t.pieces[k].path.flatten(0.05) {
        for q in poly.pts {
            r.add(q);
        }
    }
    r.0
}

#[test]
fn an_accent_is_centred_over_its_base_rule_11() {
    // the kit's case: \hat{x} and \bar{x} at size 96, the accent cluster over the base cluster within 0.5 px
    let mut lib = lib();
    for tex in [r"\hat{x}", r"\bar{x}", r"\hat{a}", r"\tilde{n}", r"\dot{y}"] {
        let t = formula::typeset_parts(&mut lib, tex, 96.0, [400.0, 400.0], 0.05).unwrap();
        assert_eq!(t.pieces.len(), 2, "{tex}");
        let (base, accent) = (ink(&t, 0), ink(&t, 1));
        let centre = |r: [f64; 4]| (r[0] + r[2]) / 2.0;
        assert!((centre(base) - centre(accent)).abs() <= 0.5, "{tex}: base {base:?} accent {accent:?}");
        assert!(accent[3] <= base[1], "{tex}: the accent is above the base");
    }
}

#[test]
fn matrix_cells_are_centred_in_their_columns_rule_11() {
    // rule 11 as the draft states it: each cell centred in its column, TeX's \halign with centred cells, so the cells'
    // advance boxes share their column's centre; a cell's ink may sit off its advance centre (a "100" by 1.3 px at
    // 48 px), so the advance centre is read as the cell's ink centre less its ink offset when typeset alone
    let mut lib = lib();
    let cells = ["1", "10", "100", "1"];
    let tex = format!(
        r"\begin{{pmatrix}} \part{{a}}{{{}}} & \part{{b}}{{{}}} \\ \part{{c}}{{{}}} & \part{{d}}{{{}}} \end{{pmatrix}}",
        cells[0], cells[1], cells[2], cells[3]
    );
    let size_box = [400.0, 200.0];
    let t = formula::typeset_parts(&mut lib, &tex, 48.0, size_box, 0.05).unwrap();
    let ink_centre = |b: PartBox| b.x + b.w / 2.0;
    let mut advance_centre = |i: usize| {
        let alone = formula::typeset_parts(&mut lib, &format!(r"\part{{p}}{{{}}}", cells[i]), 48.0, size_box, 0.05);
        // typeset alone, the advance box is centred in the image
        let offset = ink_centre(alone.unwrap().boxes[0].unwrap()) - size_box[0] / 2.0;
        ink_centre(t.boxes[i].unwrap()) - offset
    };
    let (a, b, c, d) = (advance_centre(0), advance_centre(1), advance_centre(2), advance_centre(3));
    assert!((a - c).abs() <= 0.5, "{a} {c}");
    assert!((b - d).abs() <= 0.5, "{b} {d}");
}
