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
