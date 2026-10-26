//! The star-fields draft at the renderer (sr-core `srep/srep-0000-star-fields.md`, rules 2, 4 and 6): a dome light's
//! `starField` draws each star's flux spread over a disc of `pointSize` pixels at its direction, behind every surface,
//! with the dome's gain. The expected frame is splatted on the CPU from the draft's field through a pinhole camera at
//! the origin looking down +z (RA 90°, Dec 0: Orion), with a horizontal field of view of 60°.

mod common;
use common::*;

use sr_3d::space::stars::scene_direction;
use sr_3d::space::stars::{load_catalogue, CATALOGUE_DIGEST};
use sr_3d::stars::{boundaries, splat, Field, Point, SCENE_FROM_ICRF_AXES};

const SIZE: u32 = 256;
const ZERO: f64 = 1.0;

fn media() -> std::path::PathBuf {
    std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/corpus/media"))
}

fn document(field: &str, light: &str, composition: &str) -> sr_model::Document {
    document_seen(field, light, composition, "")
}

/// A document whose project has the attributes `project` besides its size and timing.
fn document_framed(field: &str, light: &str, composition: &str, project: &str) -> sr_model::Document {
    document_with(field, light, composition, "", project)
}

/// A document whose camera has the attributes `camera` besides its place and field of view.
fn document_seen(field: &str, light: &str, composition: &str, camera: &str) -> sr_model::Document {
    document_with(field, light, composition, camera, "")
}

fn document_with(field: &str, light: &str, composition: &str, camera: &str, project: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.8"><project width="{SIZE}" height="{SIZE}" fps="24" duration="2" background="#000000FF"{project}/><assets><catalogue id="bsc" src="bsc5-compact.csv.gz" digest="sha256:{CATALOGUE_DIGEST}"/></assets><materials><material id="black" baseColor="#000000FF" unlit="true"/></materials><composition><camera id="eye" x="0" y="0" z="0" fov="60"{camera}/>{composition}</composition><lights><light id="sky" type="dome"{light}><starField catalogue="bsc" magnitudeZero="{ZERO}" pointSize="3"{field}/></light></lights></scene>"##
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

/// The field of the stars to magnitude 1 under a dome with `sky="physical"` at midnight of the June solstice on the
/// equator, with the dome's `atmosphere` and `pose` attributes, through the camera turned by `yaw` and `pitch`.
fn under_the_sky(atmosphere: &str, pose: &str, yaw: f64, pitch: f64) -> Option<Rendered> {
    let light = format!(
        r#" sky="physical" atmosphere="{atmosphere}" latitude="0" longitude="0" moon="false" date="2026-06-21T00:00:00Z"{pose}"#
    );
    rendered(&document_seen(r#" limitMagnitude="1""#, &light, "", &format!(r#" yaw="{yaw}" pitch="{pitch}""#)), 0.0)
}

/// The flux of the pixels within `radius` of the frame's centre.
fn central_flux(r: &Rendered, radius: f64) -> [f64; 3] {
    let mut sum = [0.0; 3];
    for y in 0..SIZE {
        for x in 0..SIZE {
            if from_centre(x, y) <= radius {
                for (total, v) in sum.iter_mut().zip(r.at(x, y)) {
                    *total += v as f64;
                }
            }
        }
    }
    sum
}

#[test]
fn the_physical_sky_dims_each_star_by_the_transmittance_toward_it() {
    // rule 6: Vega, 38.8° up at this midnight, is the clear field's star times rule 9's transmittance toward it; the
    // sky itself is not drawn (environmentVisible is false), and `atmosphere="none"` has no layers to dim it. A dome
    // pitched by −30° turns its sky and its field together: the camera follows Vega, which stays 38.8° up its sky
    let (yaw, pitch) = (170.76542f64.to_radians(), 38.78361f64.to_radians());
    let vega = [yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos()];
    let s = sr_3d::atmosphere::atmosphere(&sr_3d::atmosphere::AtmosphereOptions::default()).unwrap();
    let t = s.transmittance([0.0, 0.0, s.planet_radius], sr_3d::physical_sky::to_planet(vega)).unwrap();
    assert!(t[2] < 0.75 * t[0], "the air is decisive: {t:?}");
    for tilt in [0.0f64, -30.0] {
        let (c, n) = (tilt.to_radians().cos(), tilt.to_radians().sin());
        let [x, y, z] = [vega[0], vega[1] * c - vega[2] * n, vega[1] * n + vega[2] * c];
        let (yaw, pitch) = (x.atan2(z).to_degrees(), (-y).asin().to_degrees());
        let pose = format!(r#" pitch="{tilt}""#);
        let (Some(air), Some(clear)) =
            (under_the_sky("earth", &pose, yaw, pitch), under_the_sky("none", &pose, yaw, pitch))
        else {
            return;
        };
        let (got, base) = (central_flux(&air, 4.0), central_flux(&clear, 4.0));
        assert!(base.iter().all(|v| *v > 0.05), "{tilt}: Vega is in the clear frame: {base:?}");
        for c in 0..3 {
            let ratio = got[c] / base[c];
            assert!((ratio - t[c]).abs() <= 0.01 * t[c], "{tilt}, channel {c}: {got:?} over {base:?} against {t:?}");
        }
    }
}

#[test]
fn the_ground_hides_the_stars_below_the_horizon() {
    let (Some(down), Some(free)) = (
        under_the_sky("earth", "", 0.0, -40.0),
        rendered(&document_seen(r#" limitMagnitude="1""#, "", "", r#" pitch="-40""#), 0.0),
    ) else {
        return;
    };
    let flux = |r: &Rendered| central_flux(r, SIZE as f64).iter().sum::<f64>();
    assert!(flux(&free) > 0.05, "stars lie there without a sky: {}", flux(&free));
    assert_eq!(flux(&down), 0.0);
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
}

/// The pixel a scene direction falls on through the test camera, if it is in front of it.
fn project(d: [f64; 3]) -> Option<[f64; 2]> {
    let f = SIZE as f64 / 2.0 / 30f64.to_radians().tan();
    let c = SIZE as f64 / 2.0;
    (d[2] > 0.05).then(|| [c + f * d[0] / d[2], c + f * d[1] / d[2]])
}

/// Rule 5's overlay alone: the frame with `constellations` less the frame without, which must be grey.
fn overlay(constellations: &str, light: &str, composition: &str, project: &str) -> Option<Vec<f32>> {
    let field = format!(r#" constellations="{constellations}""#);
    let with = rendered(&document_framed(&field, light, composition, project), 0.0)?;
    let without = rendered(&document_framed("", light, composition, project), 0.0)?;
    Some(
        with.px
            .iter()
            .zip(&without.px)
            .map(|(a, b)| {
                let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
                assert!((d[0] - d[1]).abs() < 1e-3 && (d[1] - d[2]).abs() < 1e-3, "{d:?} is not grey");
                d[1]
            })
            .collect(),
    )
}

/// The length of the segment `a`–`b` inside the frame.
fn clipped_length(a: [f64; 2], b: [f64; 2]) -> f64 {
    let (mut t0, mut t1) = (0f64, 1f64);
    for k in 0..2 {
        let d = b[k] - a[k];
        for (q, r) in [(-d, a[k]), (d, SIZE as f64 - a[k])] {
            if q == 0.0 {
                if r < 0.0 {
                    return 0.0;
                }
            } else if q < 0.0 {
                t0 = t0.max(r / q);
            } else {
                t1 = t1.min(r / q);
            }
        }
    }
    (t1 - t0).max(0.0) * (b[0] - a[0]).hypot(b[1] - a[1])
}

/// The distance from `p` to the segment `a`–`b`.
fn to_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (ab, ap) = ([b[0] - a[0], b[1] - a[1]], [p[0] - a[0], p[1] - a[1]]);
    let t = ((ap[0] * ab[0] + ap[1] * ab[1]) / (ab[0] * ab[0] + ab[1] * ab[1]).max(1e-12)).clamp(0.0, 1.0);
    (ap[0] - t * ab[0]).hypot(ap[1] - t * ab[1])
}

/// Checks an overlay's brightness: lines and glyph stems reach the colour `0.18 × gain` and nothing exceeds it.
#[track_caller]
fn assert_grey_level(o: &[f32], gain: f32, at_least: f32) {
    let peak = o.iter().fold(0f32, |m, &v| m.max(v));
    assert!(peak <= 0.18 * gain * 1.002 && peak >= at_least * 0.18 * gain, "{peak}");
    assert!(o.iter().all(|&v| v >= -1e-4), "an overlay only adds");
}

#[test]
fn labels_are_drawn_at_their_centroids_behind_surfaces() {
    let cat = load_catalogue(&media().join("bsc5-compact.csv.gz"), Some(CATALOGUE_DIGEST)).unwrap();
    let labels = Field::new(&cat, 6.5, ZERO).unwrap().labels(0.0, true, &SCENE_FROM_ICRF_AXES).unwrap();
    let points: Vec<(&str, [f64; 2])> = labels.iter().filter_map(|(a, d)| Some((*a, project(*d)?))).collect();
    let inside = |p: [f64; 2], m: f64| p.iter().all(|v| (m..SIZE as f64 - m).contains(v));
    assert!(points.iter().any(|(a, p)| *a == "Ori" && inside(*p, 30.0)), "{points:?}");
    let (r, c) = (sphere_radius(), SIZE as f64 / 2.0);
    for (composition, project) in [("", ""), (SPHERE, ""), ("", r#" supersample="2""#)] {
        let Some(o) = overlay("labels", r#" intensity="2""#, composition, project) else { return };
        assert_grey_level(&o, 2.0, 0.9);
        for y in 0..SIZE {
            for x in 0..SIZE {
                let v = o[(y * SIZE + x) as usize];
                let q = [x as f64 + 0.5, y as f64 + 0.5];
                if v > 1e-4 {
                    // a label is three letters 14 pixels high about its point
                    let near = points.iter().any(|(_, p)| (q[0] - p[0]).abs() < 24.0 && (q[1] - p[1]).abs() < 10.0);
                    assert!(near, "({x}, {y}) {v} is near no label");
                }
                if !composition.is_empty() && (q[0] - c).hypot(q[1] - c) < r - 2.0 {
                    assert_eq!(v, 0.0, "({x}, {y}) is behind the sphere");
                }
            }
        }
        for (abbr, p) in points.iter().filter(|(_, p)| inside(*p, 30.0)) {
            if !composition.is_empty() && (p[0] - c).hypot(p[1] - c) < r + 24.0 {
                continue;
            }
            let lit = (0..SIZE * SIZE).filter(|&k| {
                let q = [(k % SIZE) as f64 + 0.5, (k / SIZE) as f64 + 0.5];
                o[k as usize] > 0.05 && (q[0] - p[0]).abs() < 12.0 && (q[1] - p[1]).abs() < 8.0
            });
            assert!(lit.count() > 20, "{abbr} at {p:?} is not drawn");
            // 14 pixels high: the capitals' height with a descender's below it
            let rows: Vec<u32> = (0..SIZE * SIZE)
                .filter(|&k| {
                    let q = [(k % SIZE) as f64 + 0.5, (k / SIZE) as f64 + 0.5];
                    o[k as usize] > 0.05 && (q[0] - p[0]).abs() < 24.0 && (q[1] - p[1]).abs() < 12.0
                })
                .map(|k| k / SIZE)
                .collect();
            let extent = rows.iter().max().unwrap() - rows.iter().min().unwrap() + 1;
            assert!((10..=15).contains(&extent), "{abbr} is {extent} pixels high");
        }
    }
}

#[test]
fn boundaries_are_drawn_along_the_tables_arcs_behind_surfaces() {
    let lines: Vec<Vec<Option<[f64; 2]>>> =
        boundaries().iter().map(|l| l.iter().map(|d| project(scene_direction(*d))).collect()).collect();
    let segments: Vec<([f64; 2], [f64; 2])> = lines
        .iter()
        .flat_map(|l| l.windows(2).filter_map(|w| Some((w[0]?, w[1]?))))
        .filter(|(a, b)| [a, b].iter().any(|p| p.iter().all(|v| (-10.0..SIZE as f64 + 10.0).contains(v))))
        .collect();
    assert!(segments.len() > 20, "{}", segments.len());
    let (r, c) = (sphere_radius(), SIZE as f64 / 2.0);
    for (composition, project) in [("", ""), (SPHERE, ""), ("", r#" supersample="2""#)] {
        let Some(o) = overlay("boundaries", "", composition, project) else { return };
        assert_grey_level(&o, 1.0, 0.5);
        let mut on = 0;
        for y in 0..SIZE {
            for x in 0..SIZE {
                let v = o[(y * SIZE + x) as usize];
                let q = [x as f64 + 0.5, y as f64 + 0.5];
                let d = segments.iter().map(|(a, b)| to_segment(q, *a, *b)).fold(f64::MAX, f64::min);
                if v > 1e-4 {
                    assert!(d < 1.25, "({x}, {y}) {v} is {d} from a boundary");
                }
                let hidden = !composition.is_empty() && (q[0] - c).hypot(q[1] - c) < r + 2.0;
                if hidden && (q[0] - c).hypot(q[1] - c) < r - 2.0 {
                    assert_eq!(v, 0.0, "({x}, {y}) is behind the sphere");
                }
                // a pixel a boundary passes through the middle of is lit
                if d < 0.1 && !hidden {
                    assert!(v > 0.4 * 0.18, "({x}, {y}) {v} is on a boundary");
                    on += 1;
                }
            }
        }
        assert!(on > 50, "{on}");
        // one output pixel wide at any supersampling: the grey over the frame is the boundaries' length in it
        if composition.is_empty() {
            let ink: f64 = o.iter().map(|&v| v as f64 / 0.18).sum();
            let length: f64 = segments.iter().map(|(a, b)| clipped_length(*a, *b)).sum();
            assert!((ink / length - 1.0).abs() < 0.05, "{ink} against {length}");
        }
    }
}

#[test]
fn the_dome_turns_its_labels_and_boundaries() {
    // as the_dome_turns_the_field: a yaw of 90° shows what lay at the dome's −x
    let turn = |[x, y, z]: [f64; 3]| [z, y, -x];
    let cat = load_catalogue(&media().join("bsc5-compact.csv.gz"), Some(CATALOGUE_DIGEST)).unwrap();
    let labels = Field::new(&cat, 6.5, ZERO).unwrap().labels(0.0, true, &SCENE_FROM_ICRF_AXES).unwrap();
    let points: Vec<[f64; 2]> = labels.iter().filter_map(|(_, d)| project(turn(*d))).collect();
    let Some(o) = overlay("labels", r#" yaw="90""#, "", "") else { return };
    let lit: Vec<[f64; 2]> = (0..SIZE * SIZE)
        .filter(|&k| o[k as usize] > 0.05)
        .map(|k| [(k % SIZE) as f64 + 0.5, (k / SIZE) as f64 + 0.5])
        .collect();
    assert!(lit.len() > 100, "{}", lit.len());
    for q in &lit {
        let near = points.iter().any(|p| (q[0] - p[0]).abs() < 24.0 && (q[1] - p[1]).abs() < 10.0);
        assert!(near, "{q:?} is near no turned label");
    }
    let segments: Vec<([f64; 2], [f64; 2])> = boundaries()
        .iter()
        .flat_map(|l| {
            let l: Vec<_> = l.iter().map(|d| project(turn(scene_direction(*d)))).collect();
            l.windows(2).filter_map(|w| Some((w[0]?, w[1]?))).collect::<Vec<_>>()
        })
        .collect();
    let Some(o) = overlay("boundaries", r#" yaw="90""#, "", "") else { return };
    let ink: f64 = o.iter().map(|&v| v as f64 / 0.18).sum();
    let length: f64 = segments.iter().map(|(a, b)| clipped_length(*a, *b)).sum();
    assert!(length > 200.0 && (ink / length - 1.0).abs() < 0.05, "{ink} against {length}");
    for (k, &v) in o.iter().enumerate() {
        if v > 1e-4 {
            let q = [(k as u32 % SIZE) as f64 + 0.5, (k as u32 / SIZE) as f64 + 0.5];
            let d = segments.iter().map(|(a, b)| to_segment(q, *a, *b)).fold(f64::MAX, f64::min);
            assert!(d < 1.25, "{q:?} {v} is {d} from a turned boundary");
        }
    }
}
