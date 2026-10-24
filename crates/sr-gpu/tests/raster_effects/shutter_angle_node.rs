//! `shutterAngle` on a node: its motion blur uses that angle instead of the project's, inherited from the nearest parent.

use super::common;
use common::*;

fn doc(node: &str, group: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.2"><project width="160" height="40" fps="10" duration="2" background="#000000" motionBlur="true" shutterAngle="180" shutterPhase="0" motionBlurSamples="16"/>
        <composition><group id="g" width="160" height="40" {group}>
          <shape id="s" shape="rect" y="16" width="8" height="8" fill="#FFFFFF" {node}><animate property="x"><key time="0" value="10"/><key time="1" value="110"/></animate></shape>
        </group></composition></scene>"##
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: false, base_dir: None })
        .unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

/// Width in pixels of the smear on the row through the square: columns with any ink.
fn smear(d: &sr_model::Document) -> Option<usize> {
    let r = render_sub(d, 0.3)?;
    Some((0..r.size[0]).filter(|&x| r.at(x, 20)[0] > 0.02).count())
}

#[test]
fn the_nodes_shutter_angle_replaces_the_projects() {
    let Some(project) = smear(&doc("", "")) else { return };
    let wide = smear(&doc(r#"shutterAngle="360""#, "")).unwrap();
    let none = smear(&doc(r#"shutterAngle="0""#, "")).unwrap();
    // 100 px per second at 10 fps: a 180 degree shutter smears about 5 px, 360 about 10, 0 nothing
    assert_eq!(none, 8, "a sharp square is its own width");
    assert!(project > 10 && project <= 16, "{project}");
    assert!(wide > project + 3, "360 degrees smears more than 180: {wide} vs {project}");
}

#[test]
fn a_parent_sets_it_for_its_children() {
    let Some(project) = smear(&doc("", "")) else { return };
    let from_group = smear(&doc("", r#"shutterAngle="360""#)).unwrap();
    assert!(from_group > project + 3, "{from_group} vs {project}");
    // the node's own angle beats its parent's
    let own = smear(&doc(r#"shutterAngle="0""#, r#"shutterAngle="360""#)).unwrap();
    assert_eq!(own, 8);
}

/// The default shutter phase (−90°) opens the shutter of the first frame before time 0, where the scene holds its first
/// keys: the first frame blurs as the sample-time formula says instead of drawing the square sharp (sr-core conformance
/// case srep-0055-node-shutter-frame0).
#[test]
fn the_first_frame_blurs_over_a_shutter_that_opens_before_time_zero() {
    let xml = r##"<scene version="1.2"><project width="160" height="40" fps="10" duration="2" background="#000000" motionBlur="true" shutterAngle="180" motionBlurSamples="16"/>
        <composition><shape id="s" shape="rect" y="16" width="8" height="8" fill="#FFFFFF"><animate property="x"><key time="0" value="10"/><key time="1" value="110"/></animate></shape></composition></scene>"##;
    let d = sr_model::load_str(xml, &sr_model::LoadOptions { verify_assets: false, base_dir: None }).unwrap();
    let Some(r) = render_sub(&d, 0.0) else { return };
    let row: Vec<f32> = (0..r.size[0]).map(|x| r.at(x, 20)[0]).collect();
    let inked = row.iter().filter(|&&v| v > 0.02).count();
    let partial = row.iter().filter(|&&v| v > 0.02 && v < 0.98).count();
    // samples from −0.025 s to +0.025 s: the half before 0 holds x = 10, the half after moves up to 2.5 px right, so
    // the 8 px square inks columns 10 to 20, solid where every sample covers it (13 to 17)
    assert!((10..=12).contains(&inked), "the square smears to the right: {inked} columns, {row:?}");
    assert!(partial >= 4, "the smear has soft edges: {partial} partial columns, {row:?}");
    assert!(row[14] > 0.98, "where the square stays all shutter it is solid: {row:?}");
}
