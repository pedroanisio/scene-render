//! Vector and paint work package A3: `meshPoint/@x,@y`, the vector asset's `strokePosition` and `paintOrder`, and
//! the absent mode of the trim `shapeModifier` (simultaneous, as `shape/@trimMode`).

use super::common;
use common::*;

// These regressions must execute the shader; absence of a GPU is a failure, never a green skip.
fn render(d: &sr_model::Document) -> Option<Rendered> {
    common::render_times_on(common::gpu().expect("A3 regressions require a GPU adapter"), d, &[0.0])
}

fn load(xml: &str) -> sr_model::Document {
    sr_model::load_str(xml, &sr_model::LoadOptions { verify_assets: false, base_dir: None })
        .unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

fn max_diff(a: &Rendered, b: &Rendered) -> f32 {
    assert_eq!(a.size, b.size);
    a.px.iter().zip(&b.px).flat_map(|(p, q)| (0..4).map(move |i| (p[i] - q[i]).abs())).fold(0.0, f32::max)
}

// ------------------------------------------------------------------ P1-01: meshPoint x, y

/// A 3 × 3 mesh over a 64 × 32 rect: black everywhere, white at the centre point, whose position `centre` sets.
fn mesh(centre: &str) -> sr_model::Document {
    mesh_animated(centre, "")
}

fn mesh_animated(centre: &str, animation: &str) -> sr_model::Document {
    let mut pts = String::new();
    for r in 0..3 {
        for c in 0..3 {
            let (col, at) = if (r, c) == (1, 1) { ("#FFFFFF", centre) } else { ("#000000", "") };
            let child = if (r, c) == (1, 1) { animation } else { "" };
            pts += &format!(r#"<point row="{r}" col="{c}" color="{col}" {at}>{child}</point>"#);
        }
    }
    load(&format!(
        r##"<scene version="1.2"><project width="64" height="32" fps="10" duration="1" background="#000000"/>
  <paints><meshGradient id="m" rows="3" cols="3" interpolationSpace="srgb">{pts}</meshGradient></paints>
  <composition><shape id="r" shape="rect" width="64" height="32" fill="url(#m)"/></composition></scene>"##
    ))
}

#[test]
fn mesh_points_without_x_y_sit_on_the_regular_grid() {
    let Some(plain) = render(&mesh("")) else { return };
    // the defaults written out are the regular grid: the same picture
    let explicit = render(&mesh(r#"x="0.5" y="0.5""#)).unwrap();
    assert_eq!(plain.px, explicit.px, "explicit regular coordinates must preserve every pixel");
    // the white centre point is at the middle of the box
    assert!(plain.at(32, 16)[0] > 0.9, "{:?}", plain.at(32, 16));
}

#[test]
fn a_moved_mesh_point_carries_its_colour_to_its_new_position() {
    let Some(r) = render(&mesh(r#"x="0.9" y="0.5""#)) else { return };
    // 0.9 × 64 = 57.6: the centre point's white is there now, and the middle of the box is no longer white
    let moved = r.at(57, 16);
    assert!(moved[0] > 0.9 && moved[1] > 0.9 && moved[2] > 0.9, "{moved:?}");
    let middle = r.at(32, 16);
    assert!(middle[0] < 0.8, "{middle:?}");
}

// ------------------------------------------------------------------ P1-09: vector asset stroke position and order

fn shape_doc(attrs: &str) -> sr_model::Document {
    load(&format!(
        r##"<scene version="1.2"><project width="64" height="32" fps="10" duration="1" background="#000000"/>
  <composition><shape id="s" shape="rect" x="10" y="6" width="20" height="20" fill="#FF0000" stroke="#0000FF" strokeWidth="6" {attrs}/></composition></scene>"##
    ))
}

fn asset_doc(attrs: &str) -> sr_model::Document {
    load(&format!(
        r##"<scene version="1.2"><project width="64" height="32" fps="10" duration="1" background="#000000"/>
  <assets><vector id="v" shape="rect" width="20" height="20" fill="#FF0000" stroke="#0000FF" strokeWidth="6" {attrs}/></assets>
  <composition><layer id="l" asset="v" x="10" y="6"/></composition></scene>"##
    ))
}

#[test]
fn vector_asset_strokes_inside_outside_and_in_either_order_like_a_shape() {
    for attrs in [
        r#"strokePosition="inside""#,
        r#"strokePosition="outside""#,
        r#"paintOrder="stroke-fill""#,
        r#"strokePosition="inside" paintOrder="stroke-fill""#,
        "",
    ] {
        let Some(s) = render(&shape_doc(attrs)) else { return };
        let v = render(&asset_doc(attrs)).unwrap();
        assert!(max_diff(&s, &v) < 0.02, "{attrs:?}: diff {}", max_diff(&s, &v));
    }
    let Some(inside) = render(&asset_doc(r#"strokePosition="inside""#)) else { return };
    // the band lies inside the 20 × 20 box (x 10..30): blue at x = 11, nothing at x = 8
    assert_px(&inside, 11, 16, [0.0, 0.0, 1.0, 1.0], 0.02);
    assert_px(&inside, 8, 16, [0.0, 0.0, 0.0, 1.0], 0.02);
    let Some(outside) = render(&asset_doc(r#"strokePosition="outside""#)) else { return };
    assert_px(&outside, 8, 16, [0.0, 0.0, 1.0, 1.0], 0.02);
    assert_px(&outside, 11, 16, [1.0, 0.0, 0.0, 1.0], 0.02);
    // centred, stroke under the fill: the fill covers the inner half of the band
    let Some(under) = render(&asset_doc(r#"paintOrder="stroke-fill""#)) else { return };
    assert_px(&under, 11, 16, [1.0, 0.0, 0.0, 1.0], 0.02);
    assert_px(&under, 8, 16, [0.0, 0.0, 1.0, 1.0], 0.02);
}

// ------------------------------------------------------------------ P3-09: trim modifier mode

fn trim_doc(mode: &str) -> sr_model::Document {
    load(&format!(
        r##"<scene version="1.2"><project width="64" height="32" fps="10" duration="1" background="#000000"/>
  <composition><shape id="p" shape="path" path="M 4 8 L 44 8 M 4 24 L 44 24" width="64" height="32" stroke="#FFFFFF" strokeWidth="2">
    <shapeModifier type="trim" amount="50" {mode}/></shape></composition></scene>"##
    ))
}

#[test]
fn trim_modifier_without_mode_trims_every_subpath_at_once() {
    let Some(absent) = render(&trim_doc("")) else { return };
    // both lines are cut to their first half
    assert_px(&absent, 10, 8, [1.0, 1.0, 1.0, 1.0], 0.02);
    assert_px(&absent, 10, 24, [1.0, 1.0, 1.0, 1.0], 0.02);
    assert_px(&absent, 40, 8, [0.0, 0.0, 0.0, 1.0], 0.02);
    let simultaneous = render(&trim_doc(r#"mode="simultaneous""#)).unwrap();
    assert_eq!(absent.px, simultaneous.px);
    // sequential trims the two lines as one: the second is gone
    let sequential = render(&trim_doc(r#"mode="sequential""#)).unwrap();
    assert_px(&sequential, 10, 8, [1.0, 1.0, 1.0, 1.0], 0.02);
    assert_px(&sequential, 10, 24, [0.0, 0.0, 0.0, 1.0], 0.02);
}

#[test]
fn mesh_warp_caps_unvalidated_dimensions_before_allocating() {
    let d = load(
        r##"<scene version="1.2"><project width="64" height="32" fps="10" duration="1"/>
      <composition><shape id="s" shape="rect" width="64" height="32">
      <deform><modifier type="mesh-warp" rows="4" cols="4"><animate property="rows"><key time="0" value="4"/></animate></modifier></deform></shape></composition></scene>"##,
    );
    let mut g = sr_eval::Evaluator::new(&d, &sr_eval::EvalOptions::default()).unwrap().evaluate(0.0);
    let shape = g.nodes.iter_mut().find(|n| &*n.id == "s").unwrap();
    let modifier = shape.parts.iter_mut().find(|p| p.key.contains("modifier")).unwrap();
    modifier.props.0 = vec![("rows".into(), sr_eval::Value::Num(1000.0)), ("cols".into(), sr_eval::Value::Num(1000.0))];
    let shape = g.nodes.iter().find(|n| &*n.id == "s").unwrap();
    let ds = sr_gpu::vector::deformers(shape, &g, [64.0, 32.0]);
    let sr_vector::deform::Deformer::MeshWarp { rows, cols, offsets, .. } = &ds[0] else { panic!("mesh warp") };
    assert_eq!((*rows, *cols, offsets.len()), (16, 16, 256));
}

#[test]
fn moved_mesh_geometry_and_colour_share_the_catmull_rom_parameter() {
    let mut pts = String::new();
    for r in 0..2 {
        for (c, (x, colour)) in [(0.0, "#000000"), (0.8, "#808080"), (1.0, "#FFFFFF")].iter().enumerate() {
            pts += &format!(r#"<point row="{r}" col="{c}" x="{x}" color="{colour}"/>"#);
        }
    }
    let d = load(&format!(
        r##"<scene version="1.2"><project width="64" height="32" fps="10" duration="1"/>
      <paints><meshGradient id="m" rows="2" cols="3" interpolationSpace="srgb">{pts}</meshGradient></paints>
      <composition><shape id="r" shape="rect" width="64" height="32" fill="url(#m)"/></composition></scene>"##
    ));
    let Some(r) = render(&d) else { return };
    let cr = |u: f64, a: f64, b: f64, c: f64, d: f64| {
        0.5 * ((-u * u * u + 2.0 * u * u - u) * a
            + (3.0 * u * u * u - 5.0 * u * u + 2.0) * b
            + (-3.0 * u * u * u + 4.0 * u * u + u) * c
            + (u * u * u - u * u) * d)
    };
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..50 {
        let u = (lo + hi) / 2.0;
        if cr(u, 0.0, 0.0, 0.8, 1.0) < 32.5 / 64.0 {
            lo = u;
        } else {
            hi = u;
        }
    }
    let encoded = cr((lo + hi) / 2.0, 0.0, 0.0, 128.0 / 255.0, 1.0);
    let expected = srgb_to_linear(encoded as f32);
    assert!((r.at(32, 16)[0] - expected).abs() < 0.004, "{:?}, expected {expected}", r.at(32, 16));
}

#[test]
fn mesh_coordinates_default_independently_and_animate_without_static_values() {
    let y_only = render(&mesh(r#"y="0.75""#)).unwrap();
    let both = render(&mesh(r#"x="0.5" y="0.75""#)).unwrap();
    assert_eq!(y_only.px, both.px);
    assert!(y_only.at(32, 23)[0] > 0.9);
    let animated = mesh_animated(
        "",
        r#"<animate property="x"><key time="0" value="0.5"/><key time="0.5" value="0.9"/></animate>"#,
    );
    let first = render(&animated).unwrap();
    let regular = render(&mesh("")).unwrap();
    assert_eq!(first.px, regular.px);
    let last = common::render_times_on(gpu().unwrap(), &animated, &[0.0, 0.5]).unwrap();
    let moved = render(&mesh(r#"x="0.9""#)).unwrap();
    assert_eq!(last.px, moved.px);
}

#[test]
fn additive_mesh_coordinate_animation_starts_at_the_regular_grid() {
    let d = mesh_animated("", r#"<animate property="x" additive="true"><key time="0" value="0.4"/></animate>"#);
    let actual = render(&d).unwrap();
    let expected = render(&mesh(r#"x="0.9""#)).unwrap();
    assert_eq!(actual.px, expected.px);
}

#[test]
fn a_moved_corner_of_a_two_by_two_mesh_carries_its_colour() {
    let d = load(
        r##"<scene version="1.2"><project width="64" height="32" fps="10" duration="1"/>
      <paints><meshGradient id="m" rows="2" cols="2" interpolationSpace="srgb">
        <point row="0" col="0" x="0.25" y="0.25" color="#FFFFFF"/>
        <point row="0" col="1" color="#000000"/><point row="1" col="0" color="#000000"/>
        <point row="1" col="1" color="#000000"/>
      </meshGradient></paints>
      <composition><shape id="r" shape="rect" width="64" height="32" fill="url(#m)"/></composition></scene>"##,
    );
    let r = render(&d).unwrap();
    // The pixel centre is (16.5, 8.5), half a pixel from the moved corner. Inverting this
    // 2-by-2 bilinear surface gives sRGB white 0.9541688, or 0.8988914 in linear light.
    assert!((r.at(16, 8)[0] - 0.8988914).abs() < 0.004, "{:?}", r.at(16, 8));
}

#[test]
fn mesh_coordinates_outside_the_box_are_not_clamped_to_unit_values() {
    let d = load(
        r##"<scene version="1.2"><project width="64" height="32" fps="10" duration="1"/>
      <paints><meshGradient id="m" rows="2" cols="2" interpolationSpace="srgb">
        <point row="0" col="0" x="-0.25" color="#FFFFFF"/>
        <point row="1" col="0" x="-0.25" color="#FFFFFF"/>
        <point row="0" col="1" color="#000000"/><point row="1" col="1" color="#000000"/>
      </meshGradient></paints>
      <composition><shape id="r" shape="rect" width="64" height="32" fill="url(#m)"/></composition></scene>"##,
    );
    let r = render(&d).unwrap();
    // Two columns interpolate linearly in position, across [-0.25, 1], including the part outside the box.
    let expected = srgb_to_linear(1.0 - (0.5 / 64.0 + 0.25) / 1.25);
    assert!((r.at(0, 16)[0] - expected).abs() < 0.003, "{:?}, expected {expected}", r.at(0, 16));
}
