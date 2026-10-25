//! The space-transforms draft (sr-core srep/srep-0000-space-transforms.md) at the renderer: a matrix shears a
//! shape's path, a map moves it in data units, a pole of 1/z breaks it without drawing across the frame, and an
//! animated group deform moves its children from one frame to the next.

mod common;
use common::*;

fn document(composition: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.8"><project width="320" height="320" fps="24" duration="2" background="#000000FF" seed="1"/>
<composition>{composition}</composition></scene>"##
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}\n{xml}"))
}

fn lit(p: [f32; 4]) -> bool {
    p[0] > 0.5
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

#[test]
fn a_matrix_shears_the_shape() {
    // c = 0.5 about the box centre: the top edge moves 25 px left, the bottom edge 25 px right
    let d = document(
        r##"<shape id="s" shape="rect" x="100" y="100" width="100" height="100" fill="#FFFFFFFF">
            <deform><modifier type="matrix" c="0.5"/></deform></shape>"##,
    );
    let Some(r) = render(&d) else { return };
    clean(&r);
    // near the top (local y 3): x from about 76.5 to 176.5
    assert!(lit(r.at(80, 103)) && !lit(r.at(185, 103)), "{:?} {:?}", r.at(80, 103), r.at(185, 103));
    // near the bottom (local y 97): x from about 123.5 to 223.5
    assert!(lit(r.at(220, 197)) && !lit(r.at(115, 197)), "{:?} {:?}", r.at(220, 197), r.at(115, 197));
}

#[test]
fn a_map_moves_the_shape_in_data_units() {
    // one data unit of 50 px to the right
    let d = document(
        r##"<shape id="s" shape="rect" x="50" y="50" width="40" height="40" fill="#FFFFFFFF">
            <deform><modifier type="map" x="u + 1" y="v" unit="50"/></deform></shape>"##,
    );
    let Some(r) = render(&d) else { return };
    clean(&r);
    assert!(!lit(r.at(70, 70)), "{:?}", r.at(70, 70));
    assert!(lit(r.at(120, 70)), "{:?}", r.at(120, 70));
}

#[test]
fn a_pole_breaks_the_line_and_draws_nothing_across_the_frame() {
    // 1/z about the middle of a horizontal line at 40 px a unit: a point d px from the centre maps to 1600 / d px, so
    // the line's ±160 px map to the two rays beyond ±10 px; nothing is drawn between −10 and 10 px
    let d = document(
        r##"<shape id="s" shape="path" path="M 0 160 L 320 160" width="320" height="320" stroke="#FFFFFFFF" strokeWidth="2">
            <deform><modifier type="complex" function="inverse" centerX="160" centerY="160" unit="40"/></deform></shape>"##,
    );
    let Some(r) = render(&d) else { return };
    clean(&r);
    assert!(r.px.iter().all(|p| p.iter().all(|c| c.is_finite())));
    assert!(lit(r.at(175, 160)) && lit(r.at(145, 160)), "{:?} {:?}", r.at(175, 160), r.at(145, 160));
    assert!(!lit(r.at(160, 160)), "{:?}", r.at(160, 160));
    // off the axis nothing is drawn
    assert!(!lit(r.at(160, 100)) && !lit(r.at(220, 220)));
}

#[test]
fn a_group_deform_redraws_a_child_whose_own_time_repeats() {
    // the triangle plays in a looping 1 s symbol, so at 0.25 s and 1.25 s it is the same node at the same local time;
    // only the enclosing group's mirror differs, and a vector target kept from the first frame would be stale
    let xml = r##"<scene version="1.8"><project width="320" height="320" fps="24" duration="2" background="#000000FF"/>
<symbols><symbol id="tri" width="40" height="40" duration="1">
  <shape id="s" shape="path" path="M 0 0 L 40 0 L 0 40 Z" width="40" height="40" fill="#FFFFFFFF"/></symbol></symbols>
<composition><group id="g"><deform><modifier type="matrix" centerX="120" centerY="120">
  <animate property="a"><key time="0.25" value="1"/><key time="1.25" value="-1"/></animate></modifier></deform>
  <instance id="i" symbol="tri" x="100" y="100" loop="1"/></group></composition></scene>"##;
    let d = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
    let Some(first) = render_times(&d, &[0.25]) else { return };
    clean(&first);
    assert!(lit(first.at(105, 130)) && !lit(first.at(135, 130)), "{:?} {:?}", first.at(105, 130), first.at(135, 130));
    let r = render_times(&d, &[0.25, 1.25]).unwrap();
    clean(&r);
    assert!(!lit(r.at(105, 130)) && lit(r.at(135, 130)), "{:?} {:?}", r.at(105, 130), r.at(135, 130));
}
