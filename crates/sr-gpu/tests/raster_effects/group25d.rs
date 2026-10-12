//! A group placed in 2.5D (`threeD="true"`): the schema places "this 2D node" in camera space, so a group turns and
//! foreshortens as one sheet whatever else it carries. Before the fix only a group with an effect was placed; one
//! without drew its children flat and dropped its own `threeD`, `rotationX` and `rotationY` without a warning.
//!
//! The reference is the same group with a neutral effect (`exposure` 0), which goes through the effect chain: drawn
//! flat into an offscreen and composited through the camera about the group's anchor.

use super::common;
use common::*;

const W: u32 = 600;
const H: u32 = 960;

/// 54 white 60 x 60 rectangles, 6 columns by 9 rows at a 100 px pitch, starting at (20, 40): the lab's repro.
fn rects() -> String {
    (0..54)
        .map(|k| {
            let (x, y) = (20 + (k % 6) * 100, 40 + (k / 6) * 100);
            format!(r##"<shape id="r{k}" shape="rect" x="{x}" y="{y}" width="60" height="60" fill="#FFFFFF"/>"##)
        })
        .collect()
}

/// A 600 x 960 frame with `body`; `pre` goes before the composition's nodes (a camera).
fn scene(pre: &str, body: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.1"><project width="{W}" height="{H}" fps="10" duration="1" background="#000000"/>
<composition>{pre}{body}</composition>
<effects><effect id="nop" type="exposure" exposure="0"/></effects></scene>"##
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

/// The repro's group: pivot at the frame's centre, which `x`/`y` keep where it is.
fn group(attrs: &str, angle: &str) -> String {
    format!(
        r#"<group id="g" threeD="true" anchorX="300" anchorY="480" x="300" y="480" {attrs}>{angle}{}</group>"#,
        rects()
    )
}

/// Lit rows (first, last) and the width of the lit run of each.
fn rows(r: &Rendered) -> (u32, u32, u32, u32) {
    let lit = |y: u32| (0..W).filter(|&x| r.at(x, y)[0] > 0.5).collect::<Vec<_>>();
    let ys: Vec<u32> = (0..H).filter(|&y| !lit(y).is_empty()).collect();
    let (top, bottom) = (*ys.first().expect("something is lit"), *ys.last().unwrap());
    let width = |y: u32| {
        let l = lit(y);
        l.last().unwrap() - l.first().unwrap() + 1
    };
    (top, bottom, width(top), width(bottom))
}

fn worst(a: &Rendered, b: &Rendered) -> f32 {
    a.px.iter().zip(&b.px).map(|(p, q)| (0..4).map(|c| (p[c] - q[c]).abs()).fold(0.0f32, f32::max)).fold(0.0, f32::max)
}

fn ok(r: &Rendered) {
    assert!(
        r.stats.errors.is_empty() && r.stats.unsupported.is_empty(),
        "{:?} {:?}",
        r.stats.errors,
        r.stats.unsupported
    );
}

/// `body` renders like `reference`; turned about x (`tilted`), both are foreshortened: the far (top) row narrower than
/// the near (bottom) one.
fn placed_like(pre: &str, body: &str, reference: &str, tilted: bool) {
    let Some(want) = render(&scene(pre, reference)) else { return };
    let got = render(&scene(pre, body)).unwrap();
    ok(&want);
    ok(&got);
    let (w, g) = (rows(&want), rows(&got));
    assert!(!tilted || w.2 + 40 < w.3, "the reference is foreshortened: {w:?}");
    assert_eq!(g, w, "rows (top, bottom, top width, bottom width) of the group against the effect case");
    let d = worst(&got, &want);
    assert!(d <= 1.0 / 255.0, "the group without effects differs from the one with a neutral effect by {d}");
}

#[test]
fn a_threed_group_without_effects_is_placed_like_one_with_a_neutral_effect() {
    let a = r#"rotationX="40""#;
    placed_like("", &group(a, ""), &group(&format!(r#"{a} effects="nop""#), ""), true);
}

#[test]
fn a_threed_group_turned_about_y_is_placed_like_one_with_a_neutral_effect() {
    let a = r#"rotationY="35""#;
    let Some(r) = render(&scene("", &group(a, ""))) else { return };
    // a positive rotationY turns the right edge away: the right column projects shorter than the left one
    let height = |x: u32| (0..H).filter(|&y| r.at(x, y)[0] > 0.5).count();
    let lit: Vec<u32> = (0..W).filter(|&x| height(x) > 0).collect();
    let (l, rt) = (height(lit[0] + 5), height(lit[lit.len() - 1] - 5));
    assert!(rt + 20 < l, "the group turns: left column {l} px lit, right {rt}");
    placed_like("", &group(a, ""), &group(&format!(r#"{a} effects="nop""#), ""), false);
}

#[test]
fn an_isolated_threed_group_is_placed_like_one_with_a_neutral_effect() {
    let a = r#"rotationX="40" isolate="true""#;
    placed_like("", &group(a, ""), &group(&format!(r#"{a} effects="nop""#), ""), true);
}

#[test]
fn a_threed_group_nested_in_a_plain_group_is_placed() {
    let a = r#"rotationX="40""#;
    let wrap = |s: String| format!(r#"<group id="outer">{s}</group>"#);
    placed_like("", &wrap(group(a, "")), &wrap(group(&format!(r#"{a} effects="nop""#), "")), true);
    let iso = |s: String| format!(r#"<group id="outer" isolate="true">{s}</group>"#);
    placed_like("", &iso(group(a, "")), &iso(group(&format!(r#"{a} effects="nop""#), "")), true);
}

#[test]
fn a_threed_group_with_an_expression_angle_under_a_camera_is_placed() {
    let cam = r#"<camera id="cam" fov="40" x="300" y="480" z="-1318.8"/>"#;
    let e = r#"<expression property="rotationX">20 * 2</expression>"#;
    placed_like(cam, &group("", e), &group(r#"effects="nop""#, e), true);
}

#[test]
fn a_turning_threed_group_is_not_served_from_a_stale_offscreen() {
    let turn = r#"<animate property="rotationX"><key time="0" value="0"/><key time="1" value="60"/></animate>"#;
    let d = scene("", &group("", turn));
    let Some(warm) = render_times(&d, &[0.0, 0.5]) else { return };
    let cold = render_times(&d, &[0.5]).unwrap();
    assert_eq!(worst(&warm, &cold), 0.0, "the frame after a turn differs from a cold render");
    assert!(render_times(&d, &[0.0]).unwrap().px != cold.px, "the group turned");
}

#[test]
fn a_flat_group_draws_its_children_as_they_are() {
    // no threeD: the group is only a container, its children draw straight into the frame, bit for bit as without it
    let Some(bare) = render(&scene("", &rects())) else { return };
    let plain = render(&scene(
        "",
        &format!(r#"<group id="g" anchorX="300" anchorY="480" x="300" y="480">{}</group>"#, rects()),
    ))
    .unwrap();
    let off =
        render(&scene("", &group(r#"threeD="false" rotationX="40""#, "").replace(r#"threeD="true" "#, ""))).unwrap();
    ok(&plain);
    assert_eq!(rows(&plain), (40, 899, 560, 560));
    assert!(plain.px == bare.px, "a flat group changes its children's pixels");
    assert!(off.px == bare.px, "threeD=\"false\" with a rotationX changes its children's pixels");
    assert_eq!(plain.stats.targets, bare.stats.targets, "a flat group draws no offscreen");
}

/// Where the implicit camera (fov 60 across the width, at the frame's centre, `W / 2 / tan 30°` in front of z = 0)
/// draws the point `(x, y)` of a node turned by `rx` degrees about a pivot at height `py` (rotationX keeps x): the
/// projection of `project_25` written out.
fn projected(x: f64, y: f64, py: f64, rx: f64) -> (f64, f64) {
    let d = W as f64 / 2.0 / 30f64.to_radians().tan();
    let (vy, vz) = ((y - py) * rx.to_radians().cos(), -(y - py) * rx.to_radians().sin());
    let s = d / (d + vz);
    (W as f64 / 2.0 + (x - W as f64 / 2.0) * s, H as f64 / 2.0 + (py + vy - H as f64 / 2.0) * s)
}

#[test]
fn a_threed_shape_turns_about_its_anchor_where_x_and_y_put_it() {
    // the lab's second case: x/y place the anchor (T(x, y) R S T(-anchor)), so a 520 x 860 rectangle at x=40,
    // y=40 with its anchor at its centre spans -220..300 x -390..470 and turns about (40, 40). Its near (bottom)
    // edge comes 276 px toward the camera at y = 369, above the frame's centre, so it projects up to row 243:
    // rows 0-243 lit is the expected picture, not a bug
    let at = |x: f64, y: f64| {
        format!(
            r##"<shape id="s" shape="rect" x="{x}" y="{y}" width="520" height="860" fill="#FFFFFF" threeD="true" rotationX="40" anchorX="260" anchorY="430"/>"##
        )
    };
    let Some(r) = render(&scene("", &at(40.0, 40.0))) else { return };
    ok(&r);
    let (_, bottom) = projected(0.0, 470.0, 40.0, 40.0);
    assert!((bottom - 243.7).abs() < 0.5, "the arithmetic: {bottom}");
    let (top, last, _, _) = rows(&r);
    assert_eq!(top, 0);
    assert!((last as f64 - bottom).abs() <= 1.0, "lit to row {last}, the projection says {bottom}");
    // the same box placed with its anchor where its centre is (x = 40 + 260, y = 40 + 430) turns about its centre
    let r = render(&scene("", &at(300.0, 470.0))).unwrap();
    let (top, last, top_w, _) = rows(&r);
    let (x0, y0) = projected(40.0, 40.0, 470.0, 40.0);
    let (x1, _) = projected(560.0, 40.0, 470.0, 40.0);
    assert!((top as f64 - y0).abs() <= 1.0, "far edge at row {top}, the projection says {y0}");
    assert!((top_w as f64 - (x1 - x0)).abs() <= 2.0, "far edge {top_w} px wide, the projection says {}", x1 - x0);
    assert_eq!(last, H - 1, "the near edge comes past the bottom of the frame");
}

/// Columns and rows (first, last) where alpha is above ½.
fn lit_box(r: &Rendered) -> (u32, u32, u32, u32) {
    let on = |x: u32, y: u32| r.at(x, y)[3] > 0.5;
    let cols: Vec<u32> = (0..W).filter(|&x| (0..H).any(|y| on(x, y))).collect();
    let rows: Vec<u32> = (0..H).filter(|&y| (0..W).any(|x| on(x, y))).collect();
    (*cols.first().expect("something is lit"), *cols.last().unwrap(), rows[0], *rows.last().unwrap())
}

#[track_caller]
fn same_box(got: &Rendered, want: &Rendered, what: &str) {
    let (g, w) = (lit_box(got), lit_box(want));
    let near = |a: u32, b: u32| a.abs_diff(b) <= 1;
    assert!(
        near(g.0, w.0) && near(g.1, w.1) && near(g.2, w.2) && near(g.3, w.3),
        "{what}: lit columns and rows {g:?}, the reference {w:?}"
    );
}

/// A scene on a transparent frame (the lit box is where alpha is).
fn clear_scene(pre: &str, body: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.1"><project width="{W}" height="{H}" fps="10" duration="1" background="#00000000"/>
<composition>{pre}{body}</composition>
<effects><effect id="nop" type="exposure" exposure="0"/></effects></scene>"##
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

#[test]
fn a_shape_drawn_before_a_threed_group_stays_flat() {
    // the backdrop is drawn (batched) before the group: it must not take the group's placement, and the group must
    // not lose it, whatever the group's blend
    let bd = r##"<shape id="bd" shape="rect" x="0" y="0" width="600" height="960" fill="#3366CC"/>"##;
    for blend in ["normal", "multiply", "screen"] {
        let a = format!(r#"rotationX="40" opacity="0.5" blend="{blend}""#);
        let Some(want) = render(&scene("", &format!("{bd}{}", group(&format!(r#"{a} effects="nop""#), "")))) else {
            return;
        };
        let got = render(&scene("", &format!("{bd}{}", group(&a, "")))).unwrap();
        ok(&got);
        let corner = got.at(1, 1);
        assert!(corner[2] > 0.3 && corner[0] < 0.1, "{blend}: the backdrop covers the frame's corner: {corner:?}");
        let d = worst(&got, &want);
        assert!(d <= 1.0 / 255.0, "{blend}: the group after a backdrop differs from the effect case by {d}");
    }
}

/// A 200 x 200 red square at (200, 380), placed in 2.5D by `attrs`.
fn square(attrs: &str) -> String {
    format!(r##"<shape id="sq" shape="rect" width="200" height="200" fill="#FF0000" threeD="true" {attrs}/>"##)
}

#[test]
fn a_threed_child_of_a_group_that_does_not_move_is_placed_as_at_the_top_level() {
    // the camera sees the child once: a threeD group at rest (no turn, no depth) leaves its threeD child where it
    // would be without the group, with or without an effect on the group
    let cam = r#"<camera id="cam" fov="40" x="400" y="480" z="-1318.8"/>"#;
    let child = square(r#"x="200" y="380" zDepth="-300""#);
    let Some(top) = render(&clear_scene(cam, &child)) else { return };
    ok(&top);
    for extra in ["", r#"effects="nop""#] {
        let g = format!(
            r#"<group id="g" threeD="true" anchorX="300" anchorY="480" x="300" y="480" {extra}>{child}</group>"#
        );
        let got = render(&clear_scene(cam, &g)).unwrap();
        ok(&got);
        same_box(&got, &top, &format!("the threeD child of a group at rest ({extra:?})"));
    }
}

#[test]
fn a_threed_child_of_a_turned_group_is_placed_once_through_the_camera() {
    // a child at depth d in a group turned by 40° about x is where a node turned by 40° about the same pivot lands
    // when it is moved by the turned depth: up by sin 40° · d and back by cos 40° · d
    let d = 200.0f64;
    let (s, c) = (40f64.to_radians().sin(), 40f64.to_radians().cos());
    let child = square(&format!(r#"x="200" y="380" zDepth="{d}""#));
    let reference = square(&format!(
        r#"x="300" y="{}" anchorX="100" anchorY="100" rotationX="40" zDepth="{}""#,
        480.0 + s * d,
        c * d
    ));
    let Some(want) = render(&clear_scene("", &reference)) else { return };
    for extra in ["", r#"effects="nop""#] {
        let g = format!(
            r#"<group id="g" threeD="true" rotationX="40" anchorX="300" anchorY="480" x="300" y="480" {extra}>{child}</group>"#
        );
        let got = render(&clear_scene("", &g)).unwrap();
        ok(&got);
        same_box(&got, &want, &format!("the threeD child of a turned group ({extra:?})"));
    }
}

#[test]
fn a_threed_group_shows_what_lies_beyond_the_frame_when_it_moves_back() {
    // a 1400 x 2000 rectangle covers the frame and more: pushed back by 800 px it shrinks into view, edges and all,
    // as the same rectangle placed on its own does
    let big = r##"<shape id="big" shape="rect" x="-400" y="-520" width="1400" height="2000" fill="#FFFFFF"/>"##;
    let leaf = r##"<shape id="big" shape="rect" x="300" y="480" anchorX="700" anchorY="1000" width="1400" height="2000" fill="#FFFFFF" threeD="true" zDepth="800"/>"##;
    let Some(want) = render(&clear_scene("", leaf)) else { return };
    let (x0, x1, _, _) = lit_box(&want);
    assert!(x0 > 10 && x1 < W - 10, "the reference shrinks into the frame: {:?}", lit_box(&want));
    for extra in ["", r#"effects="nop""#] {
        let g = format!(
            r#"<group id="g" threeD="true" zDepth="800" anchorX="300" anchorY="480" x="300" y="480" {extra}>{big}</group>"#
        );
        let got = render(&clear_scene("", &g)).unwrap();
        ok(&got);
        same_box(&got, &want, &format!("a big group pushed back ({extra:?})"));
    }
}

#[test]
fn threed_cards_keep_offscreens_the_size_of_what_they_draw() {
    // six 100 x 100 cards, each a group turned about y: each offscreen holds its card, not a frame's worth of pixels
    let cards: String = (0..6)
        .map(|k| {
            let (x, y) = (50 + (k % 3) * 180, 200 + (k / 3) * 400);
            format!(
                r##"<group id="c{k}" threeD="true" rotationY="30" anchorX="50" anchorY="50" x="{}" y="{}"><shape id="f{k}" shape="rect" width="100" height="100" fill="#FFFFFF"/></group>"##,
                x + 50,
                y + 50
            )
        })
        .collect();
    let Some(r) = render(&clear_scene("", &cards)) else { return };
    ok(&r);
    let held = r.renderer.cached_offscreens();
    let cards: Vec<_> = held.iter().filter(|(k, _)| k.starts_with("iso:c")).collect();
    assert_eq!(cards.len(), 6, "each card is drawn through an offscreen: {held:?}");
    for (k, s) in cards {
        assert!(s[0] <= 110 && s[1] <= 110, "{k} is {s:?}: a card's offscreen holds the card, not the frame");
    }
}

#[test]
fn the_copies_of_a_threed_repeat_are_not_placed_one_by_one() {
    // a repeat is placed as one picture, like a group: its copies are not 2.5D nodes of their own
    let column: String = (0..9)
        .map(|k| {
            format!(
                r##"<shape id="r{k}" shape="rect" x="20" y="{}" width="60" height="60" fill="#FFFFFF"/>"##,
                40 + k * 100
            )
        })
        .collect();
    let a = r#"threeD="true" rotationX="40" anchorX="300" anchorY="480" x="300" y="480""#;
    let rep = format!(r#"<repeat id="g" count="6" offsetX="100" {a}>{column}</repeat>"#);
    let Some(want) = render(&scene("", &group(r#"rotationX="40""#, ""))) else { return };
    let got = render(&scene("", &rep)).unwrap();
    ok(&got);
    assert_eq!(rows(&got), rows(&want), "the repeat is placed like the group");
    let held = got.renderer.cached_offscreens();
    assert_eq!(got.stats.targets, want.stats.targets, "render targets of the repeat against the group: {held:?}");
}
