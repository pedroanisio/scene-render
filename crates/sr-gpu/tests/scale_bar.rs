//! The scale draft drawn, rule 3 (sr-core `srep/srep-0000-scale.md`): a camera's `scaleBar` is the reference's bar at
//! the depth it measures (its `distance`, else the camera's target, else the nearest surface under the frame centre,
//! else `far`; an orthographic camera's visible height), drawn from (`x`, `y`) pixels above the frame's bottom left,
//! and recomputed every frame.

mod common;
use common::*;
use sr_3d::scale::{self, Units};
use sr_gpu::scale_bar::Measured;

const W: u32 = 320;
const H: u32 = 180;

fn doc(size: [u32; 2], fps: u32, duration: f64, composition: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.8"><project width="{}" height="{}" fps="{fps}" duration="{duration}" background="#000000FF"/><materials><material id="grey" baseColor="#404040FF" unlit="true"/><material id="white" baseColor="#FFFFFFFF" unlit="true"/></materials><composition>{composition}</composition></scene>"##,
        size[0], size[1]
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

fn rendered(d: &sr_model::Document, t: f64) -> Option<Rendered> {
    let r = render_times(d, &[t])?;
    assert!(
        r.stats.errors.is_empty() && r.stats.unsupported.is_empty(),
        "{:?} {:?}",
        r.stats.errors,
        r.stats.unsupported
    );
    Some(r)
}

/// The lit pixels of row `y` between columns `x0` and `x1`.
fn lit(r: &Rendered, y: u32, x0: u32, x1: u32) -> Vec<u32> {
    (x0..x1).filter(|&x| r.at(x, y)[0] > 0.5).collect()
}

fn focal(fov: f64, width: u32) -> f64 {
    scale::focal_pixels(fov, width as f64).unwrap()
}

#[test]
fn a_bar_at_its_distance_is_the_reference_s_bar_drawn_from_its_point() {
    let d = doc(
        [W, H],
        10,
        1.0,
        r#"<camera id="eye" x="0" y="0" z="0" far="1e6"><scaleBar distance="5000" x="24" y="24" thickness="4" label="false"/></camera><object3D id="ball" primitive="sphere" radius="10" x="300" z="1000" material="grey"/>"#,
    );
    let Some(r) = rendered(&d, 0.0) else { return };
    let want = scale::scale_bar_at(5000.0, focal(60.0, W), 100.0, 200, Units::Auto).unwrap();
    assert_eq!(r.stats.scale_bars.len(), 1, "{:?}", r.stats.scale_bars);
    let bar = &r.stats.scale_bars[0];
    assert_eq!((bar.camera.as_str(), bar.measured, bar.depth), ("eye", Measured::Distance, 5000.0));
    assert!((bar.pixels - want.pixels).abs() < 1e-9 * want.pixels, "{} != {}", bar.pixels, want.pixels);
    assert_eq!(bar.label, want.label);
    // rows 152 to 155 are the bar (180 − 24 − 4 to 180 − 24); its columns are 24 to 24 + its length
    let end = 24.0 + want.pixels;
    for y in 152..156 {
        let row = lit(&r, y, 0, W);
        assert_eq!(row.first(), Some(&24), "row {y}: {row:?}");
        assert!(
            (*row.last().unwrap() as f64 - (end - 1.0)).abs() <= 1.0,
            "row {y} ends at {:?}, the bar at {end}",
            row.last()
        );
        assert_eq!(row.len(), (row.last().unwrap() - 23) as usize, "row {y} is one run");
    }
    for y in [150, 151, 156, 157] {
        assert!(lit(&r, y, 0, W).is_empty(), "row {y} is outside the bar");
    }
}

#[test]
fn the_label_sits_above_the_bar_in_the_bar_s_colour() {
    let bar = |label: &str| {
        doc(
            [W, H],
            10,
            1.0,
            &format!(
                r##"<camera id="eye" x="0" y="0" z="0" far="1e6"><scaleBar distance="5000" color="#FF0000" label="{label}"/></camera><object3D id="ball" primitive="sphere" radius="10" x="300" z="1000" material="grey"/>"##
            ),
        )
    };
    let Some(with) = rendered(&bar("true"), 0.0) else { return };
    let without = rendered(&bar("false"), 0.0).unwrap();
    // the bar: rows 154 and 155 (180 − 24 − 2 to 180 − 24), red
    let c = with.at(30, 155);
    assert!(c[0] > 0.9 && c[1] < 0.1 && c[2] < 0.1, "the bar is red: {c:?}");
    // the label: the 14 pixels above the bar's 4-pixel gap, from the bar's left
    let label = |r: &Rendered| (136..150).flat_map(|y| lit(r, y, 0, W)).count();
    assert!(label(&with) > 20, "the label is drawn: {} pixels", label(&with));
    assert_eq!(label(&without), 0, "label=\"false\" draws none");
    let left = (136..150).filter_map(|y| lit(&with, y, 0, W).first().copied()).min().unwrap();
    assert!((24..28).contains(&left), "the label starts at the bar's left: {left}");
}

#[test]
fn the_depth_is_the_target_then_the_surface_under_the_centre_then_far() {
    let ball = r#"<object3D id="ball" primitive="sphere" radius="100" z="1000" material="grey"/>"#;
    let aside = r#"<object3D id="ball" primitive="sphere" radius="100" x="3000" z="1000" material="grey"/>"#;
    for (camera, objects, want, depth) in [
        (r#"x="0" y="0" z="-500" target="ball""#, ball, Measured::Target, 1500.0),
        (r#"x="0" y="0" z="0""#, ball, Measured::Surface, 900.0),
        (r#"x="0" y="0" z="-300""#, ball, Measured::Surface, 1200.0),
        (r#"x="0" y="0" z="0""#, aside, Measured::Far, 1e6),
    ] {
        let d = doc([W, H], 10, 1.0, &format!(r#"<camera id="eye" {camera} far="1e6"><scaleBar/></camera>{objects}"#));
        let Some(r) = rendered(&d, 0.0) else { return };
        let bar = &r.stats.scale_bars[0];
        assert_eq!(bar.measured, want, "{camera} {objects}");
        // the sphere's tessellation is within a part in 200 of its radius
        assert!((bar.depth - depth).abs() <= 1e-9 * depth + 0.5, "{want:?}: {} != {depth}", bar.depth);
        let kernel = scale::scale_bar_at(bar.depth, focal(60.0, W), 100.0, 200, Units::Auto).unwrap();
        assert_eq!((bar.pixels, bar.label.as_str()), (kernel.pixels, kernel.label.as_str()));
    }
}

#[test]
fn the_powers_of_ten_camera_s_bar_is_scale_bar_frame_at_every_frame() {
    // the kit's srep-<n>-scale-bar case: the camera keyed geometric from 10 to 1e20 pixels away from 2 s to 40 s
    let d = doc(
        [1920, 1080],
        24,
        41.0,
        r#"<camera id="eye" x="0" y="0" near="1" far="1e20" target="ball"><scaleBar/><animate property="z"><key time="2" value="-10" interpolation="geometric"/><key time="40" value="-1e20"/></animate></camera><object3D id="ball" primitive="sphere" radius="1" material="grey"/>"#,
    );
    for (frame, label) in [(0u64, "10 mm"), (96, "100 mm"), (480, "10 Mm"), (960, "2 pc")] {
        let Some(r) = rendered(&d, frame as f64 / 24.0) else { return };
        let want =
            scale::scale_bar_frame(frame, 24.0, 2.0, 40.0, 10.0, 1e20, focal(60.0, 1920), 100.0, 200, Units::Auto)
                .unwrap();
        let bar = &r.stats.scale_bars[0];
        assert_eq!((bar.measured, bar.label.as_str()), (Measured::Target, label), "frame {frame}");
        assert_eq!(want.label, label, "frame {frame}: the reference");
        assert!(
            (bar.pixels - want.pixels).abs() <= 1e-9 * want.pixels,
            "frame {frame}: {} != {}",
            bar.pixels,
            want.pixels
        );
    }
}

#[test]
fn an_orthographic_camera_measures_its_visible_height() {
    // the kit's srep-<n>-scale-bar-ortho case: orthoHeight 2000 over 1080 rows is 54 pixels a metre
    let d = doc(
        [W, 1080],
        10,
        1.0,
        r#"<camera id="eye" x="0" y="0" z="-100" projection="orthographic" orthoHeight="2000"><scaleBar distance="50"/></camera><object3D id="ball" primitive="sphere" radius="10" material="grey"/>"#,
    );
    let Some(r) = rendered(&d, 0.0) else { return };
    let want = scale::scale_bar(54.0, 200, Units::Auto).unwrap();
    let bar = &r.stats.scale_bars[0];
    assert_eq!((bar.measured, bar.depth), (Measured::OrthoHeight, 2000.0));
    assert!((bar.pixels - want.pixels).abs() < 1e-9 * want.pixels, "{} != {}", bar.pixels, want.pixels);
    assert_eq!(bar.label, want.label);
    // without orthoHeight the camera shows the frame's height: 100 pixels a metre
    let d = doc(
        [W, 1080],
        10,
        1.0,
        r#"<camera id="eye" x="0" y="0" z="-100" projection="orthographic"><scaleBar/></camera><object3D id="ball" primitive="sphere" radius="10" material="grey"/>"#,
    );
    let r = rendered(&d, 0.0).unwrap();
    let want = scale::scale_bar(100.0, 200, Units::Auto).unwrap();
    let bar = &r.stats.scale_bars[0];
    assert_eq!(
        (bar.measured, bar.depth, bar.pixels, bar.label.as_str()),
        (Measured::OrthoHeight, 1080.0, want.pixels, want.label.as_str())
    );
}

#[test]
fn a_viewport_s_camera_draws_its_bar_in_the_viewport_s_frame() {
    let d = doc(
        [W, H],
        10,
        1.0,
        r#"<viewport3D id="vp" x="100" y="50" width="160" height="90" camera="vc"><camera id="vc" x="0" y="0" z="0" far="1e6"><scaleBar distance="5000" x="10" y="10" thickness="4" label="false"/></camera><object3D id="ball" primitive="sphere" radius="10" x="300" z="1000" material="grey"/></viewport3D>"#,
    );
    let Some(r) = rendered(&d, 0.0) else { return };
    assert_eq!(r.stats.scale_bars.len(), 1, "{:?}", r.stats.scale_bars);
    let want = scale::scale_bar_at(5000.0, focal(60.0, 160), 100.0, 200, Units::Auto).unwrap();
    assert!((r.stats.scale_bars[0].pixels - want.pixels).abs() < 1e-9 * want.pixels);
    // the viewport's bottom left is (100, 140): the bar's rows are 126 to 129, from column 110
    let row = lit(&r, 128, 0, W);
    assert_eq!(row.first(), Some(&110), "{row:?}");
    assert!((*row.last().unwrap() as f64 - (110.0 + want.pixels - 1.0)).abs() <= 1.0, "{row:?}");
    assert!(lit(&r, 131, 0, W).is_empty() && lit(&r, 124, 0, W).is_empty());
}

#[test]
fn a_camera_without_a_scale_bar_draws_none() {
    let d = doc(
        [W, H],
        10,
        1.0,
        r#"<camera id="eye" x="0" y="0" z="0"/><object3D id="ball" primitive="sphere" radius="10" x="300" z="1000" material="grey"/>"#,
    );
    let Some(r) = rendered(&d, 0.0) else { return };
    assert!(r.stats.scale_bars.is_empty());
}

#[test]
fn the_document_camera_s_bar_is_drawn_over_every_pass_at_the_nearest_of_their_surfaces() {
    // two 3D passes split by a 2D shape: the farther surface under the centre in the first, the nearer in the second,
    // and a white sphere of the second over the bar's corner
    let d = doc(
        [W, H],
        10,
        1.0,
        r##"<camera id="eye" x="0" y="0" z="0" far="1e6"><scaleBar color="#FF0000" label="false"/></camera><object3D id="far" primitive="sphere" radius="100" z="2000" material="grey"/><shape id="dot" shape="rect" x="300" y="0" width="2" height="2" fill="#0000FF"/><object3D id="near" primitive="sphere" radius="50" z="1000" material="grey"/><object3D id="cover" primitive="sphere" radius="600" x="-1470" y="714" z="3000" material="white"/>"##,
    );
    let Some(r) = rendered(&d, 0.0) else { return };
    assert_eq!(r.stats.scale_bars.len(), 1, "one bar a frame: {:?}", r.stats.scale_bars);
    let bar = &r.stats.scale_bars[0];
    assert_eq!(bar.measured, Measured::Surface);
    assert!((bar.depth - 950.0).abs() <= 0.5, "the nearer surface: {}", bar.depth);
    let under = r.at(20, 155);
    assert!(under[0] > 0.9 && under[1] > 0.9, "the white sphere covers the bar's corner: {under:?}");
    let c = r.at(30, 155);
    assert!(c[0] > 0.9 && c[1] < 0.1 && c[2] < 0.1, "the bar is over it: {c:?}");
}
