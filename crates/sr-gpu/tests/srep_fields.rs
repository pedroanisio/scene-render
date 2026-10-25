//! The fields draft (sr-core srep/srep-0000-fields.md) at the renderer: a portrait's and a lic's raster fill the
//! field's box, rows from its top, at the presentation's opacity, and a presentation of paths draws its stroke.

mod common;
use common::*;

fn scene(composition: &str) -> sr_model::Document {
    let xml = format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<scene version="1.8">
<project width="640" height="360" fps="24" duration="2" background="#000000FF" seed="5"/>
<output id="still" path="out/frame_%04d.png" codec="png-sequence"/>
<composition>{composition}</composition>
</scene>"##
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}"))
}

#[track_caller]
fn clean(r: &Rendered) {
    assert!(
        r.stats.errors.is_empty() && r.stats.unsupported.is_empty(),
        "{:?} {:?}",
        r.stats.errors,
        r.stats.unsupported
    );
}

/// The box: 400 x 200 pixels at (100, 50) over data [-2, 2] x [-1, 1].
const BOX: &str = r#"id="f" x="100" y="50" width="400" height="200" xMin="-2" xMax="2" yMin="-1" yMax="1""#;

#[test]
fn a_portrait_of_the_identity_fills_the_box_with_arg_as_hue() {
    let d = scene(&format!(r#"<field {BOX} kind="complex" w="x; y"><portrait modulus="false"/></field>"#));
    let Some(r) = render(&d) else { return };
    clean(&r);
    let (g, s) = (srgb_to_linear(0.5), 0.06);
    // arg 0 on the right is red, π on the left cyan, π/2 at the top hue 1/4, -π/2 at the bottom hue 3/4
    assert_px(&r, 490, 150, [1.0, 0.0, 0.0, 1.0], s);
    assert_px(&r, 110, 150, [0.0, 1.0, 1.0, 1.0], s);
    assert_px(&r, 300, 55, [g, 1.0, 0.0, 1.0], s);
    assert_px(&r, 300, 245, [g, 0.0, 1.0, 1.0], s);
    // nothing outside the box
    for (x, y) in [(95, 150), (505, 150), (300, 45), (300, 255)] {
        assert_px(&r, x, y, [0.0, 0.0, 0.0, 1.0], 0.01);
    }
}

#[test]
fn a_portrait_draws_at_its_opacity() {
    let d =
        scene(&format!(r#"<field {BOX} kind="complex" w="x; y"><portrait modulus="false" opacity="0.5"/></field>"#));
    let Some(r) = render(&d) else { return };
    clean(&r);
    assert_px(&r, 490, 150, [0.5, 0.0, 0.0, 1.0], 0.06);
}

#[test]
fn a_lic_is_grey_noise_over_the_box_only() {
    let d = scene(&format!(r#"<field {BOX} u="1" v="0.2"><lic length="20" resolution="100"/></field>"#));
    let Some(r) = render(&d) else { return };
    clean(&r);
    let inside: Vec<f32> = (60..240)
        .step_by(7)
        .flat_map(|y| (110..490).step_by(7).map(move |x| (x, y)))
        .map(|(x, y)| {
            let p = r.at(x, y);
            assert!((p[0] - p[1]).abs() < 1e-3 && (p[1] - p[2]).abs() < 1e-3, "({x},{y}) is not grey: {p:?}");
            p[0]
        })
        .collect();
    let mean = inside.iter().sum::<f32>() / inside.len() as f32;
    let var = inside.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / inside.len() as f32;
    assert!(mean > 0.05 && var > 1e-4, "a flat lic: mean {mean}, variance {var}");
    for (x, y) in [(95, 150), (505, 150), (300, 45), (300, 255)] {
        assert_px(&r, x, y, [0.0, 0.0, 0.0, 1.0], 0.01);
    }
}

#[test]
fn arrows_draw_their_stroke() {
    let d = scene(&format!(
        r##"<field {BOX} u="1" v="0" stroke="#FF0000FF" strokeWidth="4"><arrows cols="3" rows="3"/></field>"##
    ));
    let Some(r) = render(&d) else { return };
    clean(&r);
    // the centre arrow's shaft runs right from the box's centre (300, 150) for its 40 px
    assert_px(&r, 320, 150, [1.0, 0.0, 0.0, 1.0], 0.05);
    assert_px(&r, 320, 165, [0.0, 0.0, 0.0, 1.0], 0.01);
}

#[test]
fn an_animated_raster_redraws_inside_a_kept_group() {
    // the field's real part falls from 1.9 to -2.1 at the box's right edge: arg 0, then π
    let d = scene(&format!(
        r#"<group id="iso" isolate="true" opacity="0.99"><field {BOX} kind="complex" w="x - 4*time; y"><portrait modulus="false"/></field></group>"#
    ));
    let Some(r) = render_times(&d, &[0.0, 1.0]) else { return };
    clean(&r);
    let p = r.at(490, 150);
    assert!(p[0] < 0.05 && p[1] > 0.9 && p[2] > 0.9, "a stale raster at (490, 150): {p:?}");
}

#[test]
fn a_raster_that_changes_over_the_shutter_is_not_one_picture_moved() {
    // a uniform field whose hue turns half a circle over the 1/48 s shutter, on a field that moves: a picture drawn
    // once and moved would show one full-saturation hue, the samples together blend several
    let xml = format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<scene version="1.8">
<project width="640" height="360" fps="24" duration="2" background="#000000FF" seed="5" motionBlur="true" shutterAngle="180" motionBlurSamples="12" adaptiveMotionBlur="false"/>
<output id="still" path="out/frame_%04d.png" codec="png-sequence"/>
<composition><field {BOX} kind="complex" w="Math.cos(150.79644737231007*time); Math.sin(150.79644737231007*time)"><animate property="x"><key time="0" value="100"/><key time="2" value="120"/></animate><portrait modulus="false"/></field></composition>
</scene>"##
    );
    let d = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}"));
    let Some(shots) = render_sub_frames(&d, &[0.5]) else { return };
    let p = shots[0].at(300, 150);
    let lo = p[0].min(p[1]).min(p[2]);
    assert!(lo > 0.03, "one hue moved, not the blend of the shutter's hues: {p:?}");
}
