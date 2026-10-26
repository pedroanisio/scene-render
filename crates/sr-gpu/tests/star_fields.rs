//! The star-fields draft at the renderer (sr-core `srep/srep-0000-star-fields.md`, rules 2, 4 and 6): a dome light's
//! `starField` draws each star's flux spread over a disc of `pointSize` pixels at its direction, behind every surface,
//! with the dome's gain. The expected frame is splatted on the CPU from the draft's field through a pinhole camera at
//! the origin looking down +z (RA 90°, Dec 0: Orion), with a horizontal field of view of 60°.

mod common;
use common::*;

use sr_3d::space::stars::{load_catalogue, CATALOGUE_DIGEST};
use sr_3d::stars::{splat, Field, Point, SCENE_FROM_ICRF_AXES};

const SIZE: u32 = 256;
const ZERO: f64 = 1.0;

fn media() -> std::path::PathBuf {
    std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/corpus/media"))
}

fn document(field: &str, light: &str, composition: &str) -> sr_model::Document {
    document_seen(field, light, composition, "")
}

/// A document whose camera has the attributes `camera` besides its place and field of view.
fn document_seen(field: &str, light: &str, composition: &str, camera: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.8"><project width="{SIZE}" height="{SIZE}" fps="24" duration="2" background="#000000FF"/><assets><catalogue id="bsc" src="bsc5-compact.csv.gz" digest="sha256:{CATALOGUE_DIGEST}"/></assets><materials><material id="black" baseColor="#000000FF" unlit="true"/></materials><composition><camera id="eye" x="0" y="0" z="0" fov="60"{camera}/>{composition}</composition><lights><light id="sky" type="dome"{light}><starField catalogue="bsc" magnitudeZero="{ZERO}" pointSize="3"{field}/></light></lights></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(media()) };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

/// The frame the draft predicts: the field `years` after J2000 at `gain` times its flux.
fn expected(years: f64, gain: f64) -> Vec<[f32; 3]> {
    expected_turned(years, gain, |d| d)
}

/// The frame of a dome that `turn`s the field's directions into the world.
fn expected_turned(years: f64, gain: f64, turn: impl Fn([f64; 3]) -> [f64; 3]) -> Vec<[f32; 3]> {
    let cat = load_catalogue(&media().join("bsc5-compact.csv.gz"), Some(CATALOGUE_DIGEST)).unwrap();
    let mut field = Field::new(&cat, 6.5, ZERO).unwrap().at(years, true, &SCENE_FROM_ICRF_AXES).unwrap();
    for s in &mut field {
        s.direction = turn(s.direction);
    }
    let f = SIZE as f64 / 2.0 / 30f64.to_radians().tan();
    let points: Vec<Point> = field
        .iter()
        .filter(|s| s.direction[2] > 0.0)
        .map(|s| {
            let [x, y, z] = s.direction;
            let c = SIZE as f64 / 2.0;
            Point { pixel: [c + f * x / z, c + f * y / z], rgb: s.colour.map(|v| (v * s.flux * gain) as f32) }
        })
        .collect();
    let mut buf = vec![[0f32; 3]; (SIZE * SIZE) as usize];
    splat(&mut buf, [SIZE, SIZE], &points, 3.0);
    buf
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

/// Compares the frame with the prediction over the pixels `keep` selects: every channel within half a percent of
/// the brightest pixel, and the flux of the selection within a tenth of a percent.
#[track_caller]
fn assert_matches(r: &Rendered, want: &[[f32; 3]], keep: impl Fn(u32, u32) -> bool) {
    let peak = want.iter().flatten().fold(0f32, |m, &v| m.max(v));
    assert!(peak > 0.05, "the predicted frame is dark: {peak}");
    let (mut got_sum, mut want_sum) = (0f64, 0f64);
    for y in 0..SIZE {
        for x in 0..SIZE {
            if !keep(x, y) {
                continue;
            }
            let (g, w) = (r.at(x, y), want[(y * SIZE + x) as usize]);
            for c in 0..3 {
                assert!((g[c] - w[c]).abs() <= 5e-3 * peak, "pixel ({x}, {y}) channel {c}: {} against {}", g[c], w[c]);
                got_sum += g[c] as f64;
                want_sum += w[c] as f64;
            }
        }
    }
    assert!((got_sum - want_sum).abs() <= 1e-3 * want_sum, "flux {got_sum} against {want_sum}");
}

#[test]
fn the_field_is_drawn_at_its_places_with_its_flux() {
    let Some(r) = rendered(&document("", "", ""), 0.0) else { return };
    assert_matches(&r, &expected(0.0, 1.0), |_, _| true);
}

#[test]
fn the_domes_intensity_and_exposure_scale_the_field_and_its_epoch_moves_it() {
    let d = document(r#" epoch="12000""#, r#" intensity="1.5" exposure="1""#, "");
    let Some(r) = rendered(&d, 0.0) else { return };
    assert_matches(&r, &expected(10000.0, 3.0), |_, _| true);
}

#[test]
fn the_dome_turns_the_field() {
    // a yaw of 90° about +y takes the dome's +z to the world's +x: the camera sees what lay at the dome's −x
    let d = document("", r#" yaw="90""#, "");
    let Some(r) = rendered(&d, 0.0) else { return };
    assert_matches(&r, &expected_turned(0.0, 1.0, |[x, y, z]| [z, y, -x]), |_, _| true);
}

/// The distance of a pixel's centre from the frame's.
fn from_centre(x: u32, y: u32) -> f64 {
    let c = SIZE as f64 / 2.0;
    (x as f64 + 0.5 - c).hypot(y as f64 + 0.5 - c)
}

/// The radius in pixels of the sphere 50 units across at 500 in front of the camera.
fn sphere_radius() -> f64 {
    50.0 / (500f64.powi(2) - 50f64.powi(2)).sqrt() * (SIZE as f64 / 2.0) / 30f64.to_radians().tan()
}

const SPHERE: &str = r#"<object3D id="ball" primitive="sphere" radius="50" z="500" material="black"/>"#;

#[test]
fn the_cameras_exposure_scales_the_field_with_and_without_a_3d_pass() {
    let want = expected(0.0, 2.0);
    for composition in ["", SPHERE] {
        let Some(r) = rendered(&document_seen("", "", composition, r#" exposure="1""#), 0.0) else { return };
        assert_matches(&r, &want, |x, y| from_centre(x, y) > sphere_radius() + 3.0);
    }
}

#[test]
fn surfaces_hide_the_stars_behind_them() {
    // an unlit black sphere 50 units across at 500 in front: a disc of about 22 px radius at the centre
    let Some(r) = rendered(&document("", "", SPHERE), 0.0) else { return };
    let want = expected(0.0, 1.0);
    let (d, radius) = (from_centre, sphere_radius());
    assert_matches(&r, &want, |x, y| d(x, y) > radius + 3.0);
    for y in 0..SIZE {
        for x in 0..SIZE {
            if d(x, y) < radius - 2.0 {
                assert_eq!(r.at(x, y)[..3], [0.0; 3], "({x}, {y}) is behind the sphere");
            }
        }
    }
    // the sphere hides stars the empty frame shows
    let hidden: f32 =
        (0..SIZE * SIZE).filter(|&k| d(k % SIZE, k / SIZE) < radius - 2.0).map(|k| want[k as usize][0]).sum();
    assert!(hidden > 0.01, "{hidden}");
}

#[test]
fn a_frame_rendered_alone_equals_the_frame_reached_by_playing() {
    let d = document("", "", "");
    let (Some(alone), Some(played)) = (render_times(&d, &[1.0]), render_times(&d, &[0.0, 0.5, 1.0])) else { return };
    assert_eq!(alone.px, played.px);
}

/// What a frame reports it does not draw.
fn unsupported(d: &sr_model::Document) -> Option<Vec<String>> {
    let r = render_times(d, &[0.0])?;
    assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
    Some(r.stats.unsupported)
}

#[track_caller]
fn reports(d: &sr_model::Document, note: &str) {
    let Some(got) = unsupported(d) else { return };
    assert!(got.iter().any(|m| m.contains(note)), "want {note:?} in {got:?}");
}

#[test]
fn what_is_not_drawn_is_reported() {
    for composition in ["", SPHERE] {
        let d = document_seen("", "", composition, r#" projection="orthographic""#);
        reports(&d, "a star field is not drawn through an orthographic camera");
    }
    let d = document_seen("", "", SPHERE, r#" renderer="pathtrace" pathSamples="1" maxBounces="1""#);
    reports(&d, "a star field is drawn by the raster renderer only");
    reports(&document(r#" constellations="labels""#, "", ""), "starField constellations are not drawn");
}
