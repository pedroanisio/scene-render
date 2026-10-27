//! The scale draft drawn, rule 1 (sr-core `srep/srep-0000-scale.md`): a camera with `near="0.1" far="1e19"` keeps
//! two surfaces one part in 1e4 apart in order at every decade under both depth mappings, and a logarithmic camera
//! draws the image the reversed one draws, with depth of field (which reads the depth buffer back as distance) and
//! with the prepass of ambient occlusion and reflections.

mod common;
use common::*;

const SIZE: u32 = 64;

fn document(depth: &str, composition: &str, camera: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.8"><project width="{SIZE}" height="{SIZE}" fps="24" duration="1" background="#000000FF"/><materials><material id="red" baseColor="#FF0000FF" unlit="true"/><material id="green" baseColor="#00FF00FF" unlit="true"/></materials><composition><camera id="eye" x="0" y="0" z="0" near="0.1" far="1e19" depth="{depth}" {camera}/>{composition}</composition></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

fn rendered(d: &sr_model::Document) -> Option<Rendered> {
    let r = render_times(d, &[0.0])?;
    assert!(
        r.stats.errors.is_empty() && r.stats.unsupported.is_empty(),
        "{:?} {:?}",
        r.stats.errors,
        r.stats.unsupported
    );
    Some(r)
}

/// A red sphere at distance `z` and a green one of the same radius one part in 1e4 behind it.
fn pair(z: f64) -> String {
    let r = 0.3 * z;
    format!(
        r#"<object3D id="near" primitive="sphere" radius="{r:e}" z="{z:e}" material="red"/><object3D id="far" primitive="sphere" radius="{r:e}" z="{:e}" material="green"/>"#,
        z * (1.0 + 1e-4) + r * 1e-4
    )
}

#[test]
fn two_surfaces_one_part_in_1e4_apart_keep_their_order_at_every_decade() {
    for depth in ["reversed", "logarithmic"] {
        for k in [1, 4, 8, 12, 16, 18] {
            let Some(r) = rendered(&document(depth, &pair(10f64.powi(k)), "")) else { return };
            let (mut red, mut green) = (0, 0);
            for c in &r.px {
                red += (c[0] > 0.5 && c[1] < 0.1) as u32;
                green += (c[1] > 0.1) as u32;
            }
            assert!(red > 300, "{depth} 1e{k}: {red} red pixels");
            assert_eq!(green, 0, "{depth} 1e{k}: the farther sphere shows through in {green} pixels");
        }
    }
}

#[test]
fn a_logarithmic_camera_draws_the_reversed_camera_s_image() {
    // a sphere in focus before one out of focus: the blur reads the depth buffer back as distance
    let scene = r#"<object3D id="a" primitive="sphere" radius="3" x="-2" z="30" material="red"/><object3D id="b" primitive="sphere" radius="200" x="400" z="2000" material="green"/>"#;
    // ambient occlusion and reflections draw the prepass, which writes the depth first
    for camera in [
        "",
        r#"depthOfField="true" fStop="1.4" focusTarget="a""#,
        r#"ambientOcclusion="true" screenSpaceReflections="true""#,
    ] {
        let Some(rev) = rendered(&document("reversed", scene, camera)) else { return };
        let log = rendered(&document("logarithmic", scene, camera)).unwrap();
        let worst = rev
            .px
            .iter()
            .zip(&log.px)
            .map(|(a, b)| (0..4).map(|i| (a[i] - b[i]).abs()).fold(0f32, f32::max))
            .fold(0f32, f32::max);
        assert!(worst <= 1.0 / 255.0, "{camera:?}: the images differ by {}", worst * 255.0);
        assert!(rev.px.iter().any(|c| c[1] > 0.2), "{camera:?}: the far sphere is drawn");
    }
}
