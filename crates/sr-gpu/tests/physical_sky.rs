//! The physical-sky draft (sr-core srep/srep-0000-physical-sky.md) on the GPU: a dome with `sky="physical"` draws its
//! sky and the discs of the dated sun and moon where SREP 71's gradient sky draws a sun of the same azimuth and
//! elevation, the same with and without a 3D pass, and lights the scene with the sun's power.

mod common;
use common::*;
use sr_3d::space::moon::moon_from_document;
use sr_3d::space::sun::{sun_from_document, Observer};

fn kit(body: &str, light: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<scene version=\"1.8\">\n<project width=\"640\" height=\"360\" fps=\"24\" duration=\"1\" background=\"#000000FF\" seed=\"1\"/>\n<output id=\"still\" path=\"out/frame_%04d.png\" codec=\"png-sequence\"/>\n<materials><material id=\"m\" baseColor=\"#FFFFFFFF\" roughness=\"1\"/></materials>\n<composition>\n{body}\n</composition>\n<lights>{light}</lights>\n</scene>\n"
    )
}

fn render_xml(xml: &str) -> Option<Rendered> {
    let d =
        sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}\n{xml}"));
    let r = render(&d)?;
    assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
    Some(r)
}

fn display(r: &Rendered) -> Vec<u8> {
    r.renderer.to_srgb8(&r.px)
}

/// The mean colour of the box [x0, x1) × [y0, y1) in 8-bit code values.
fn mean(r: &Rendered, d: &[u8], [x0, y0, x1, y1]: [u32; 4]) -> [f64; 3] {
    let mut sum = [0f64; 3];
    for y in y0..y1 {
        for x in x0..x1 {
            let o = ((y * r.size[0] + x) * 4) as usize;
            for (k, s) in sum.iter_mut().enumerate() {
                *s += d[o + k] as f64;
            }
        }
    }
    sum.map(|s| s / ((x1 - x0) * (y1 - y0)) as f64)
}

/// (cx, cy, w, h) of the pixels of a class, or None under 12 pixels.
fn measure(r: &Rendered, is: impl Fn([u8; 3]) -> bool) -> Option<[f64; 4]> {
    let d = display(r);
    let [w, h] = r.size;
    let (mut n, mut sx, mut sy) = (0usize, 0f64, 0f64);
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..h {
        for x in 0..w {
            let o = ((y * w + x) * 4) as usize;
            if is([d[o], d[o + 1], d[o + 2]]) {
                n += 1;
                sx += x as f64;
                sy += y as f64;
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
        }
    }
    (n >= 12).then(|| [sx / n as f64 + 0.5, sy / n as f64 + 0.5, (x1 - x0 + 1) as f64, (y1 - y0 + 1) as f64])
}

fn white(c: [u8; 3]) -> bool {
    c.iter().all(|v| *v >= 245)
}

fn red(c: [u8; 3]) -> bool {
    c[0] > 127 && c[1] < 77 && c[2] < 77
}

#[track_caller]
fn same_disc(got: Option<[f64; 4]>, want: Option<[f64; 4]>, what: &str) {
    let (got, want) = (got.unwrap_or_else(|| panic!("no {what} drawn")), want.expect("no reference sun drawn"));
    assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 2.0), "{what}: {got:?}, the gradient's sun {want:?}");
}

fn observer(latitude: f64, longitude: f64) -> Observer {
    Observer { latitude, longitude, altitude: 0.0, pressure: 1013.25, temperature: 10.0, delta_t: None }
}

/// SREP 71's sun on a black gradient sky at an azimuth and elevation: a red disc of `size` degrees.
fn gradient_sun(azimuth: f64, elevation: f64, size: f64) -> String {
    format!("<light id=\"sky\" type=\"dome\" sky=\"gradient\" skyZenith=\"#000000\" skyHorizon=\"#000000\" skyGround=\"#000000\" sunAzimuth=\"{azimuth}\" sunElevation=\"{elevation}\" sunSize=\"{size}\" sunColor=\"#FF0000\" sunIntensity=\"1\" environmentVisible=\"true\"/>")
}

/// A small ball in the frame's corner: the frame has a 3D pass, which draws the background.
const SMALL: &str = "<object3D id=\"o\" primitive=\"sphere\" radius=\"20\" x=\"600\" y=\"330\" material=\"m\"/>";

/// The midnight sun at 75° N on the June solstice: low in the north, in the default camera's view.
const MIDNIGHT: &str = "2024-06-21T00:00:00Z";

fn midnight(attrs: &str) -> String {
    format!("<light id=\"sky\" type=\"dome\" sky=\"physical\" sunIntensity=\"1\" date=\"{MIDNIGHT}\" latitude=\"75\" moon=\"false\" environmentVisible=\"true\" {attrs}/>")
}

/// The midnight sun with `property` keyed from `from` at 0 s to `to` at 1 s.
fn midnight_keyed(property: &str, from: &str, to: &str) -> String {
    midnight("").replace(
        "/>",
        &format!("><animate property=\"{property}\"><key time=\"0\" value=\"{from}\"/><key time=\"1\" value=\"{to}\"/></animate></light>"),
    )
}

#[test]
fn the_dated_sun_is_drawn_where_the_gradient_draws_that_sun() {
    let (s, _) = sun_from_document(MIDNIGHT, &observer(75.0, 0.0)).unwrap();
    let Some(physical) = render_xml(&kit("", &midnight("sunSize=\"5\""))) else { return };
    let reference = render_xml(&kit("", &gradient_sun(s.azimuth, 90.0 - s.zenith, 5.0))).unwrap();
    same_disc(measure(&physical, white), measure(&reference, red), "sun");
}

#[test]
fn the_sky_is_blue_above_and_black_below_the_horizon() {
    let Some(r) = render_xml(&kit("", &midnight(""))) else { return };
    let d = display(&r);
    let high = mean(&r, &d, [0, 0, 40, 20]);
    assert!(high[2] > high[0] && high[2] > 10.0, "sky {high:?}");
    let ground = mean(&r, &d, [100, 300, 140, 340]);
    assert!(ground.iter().all(|c| *c <= 1.0), "below the horizon {ground:?}");
}

#[test]
fn the_sky_of_a_3d_pass_is_the_same_sky() {
    // as the dome lies and turned: the background and the 3D pass rotate the sky and its discs alike
    for pose in ["", "yaw=\"12\" pitch=\"4\""] {
        let light = midnight(&format!("sunSize=\"5\" {pose}"));
        let Some(alone) = render_xml(&kit("", &light)) else { return };
        let pass = render_xml(&kit(SMALL, &light)).unwrap();
        let (a, b) = (display(&alone), display(&pass));
        for bx in [[300, 0, 340, 20], [0, 0, 40, 40], [580, 40, 620, 80], [300, 150, 340, 170], [100, 300, 140, 340]] {
            let (ma, mb) = (mean(&alone, &a, bx), mean(&pass, &b, bx));
            assert!(
                ma.iter().zip(mb).all(|(x, y)| (x - y).abs() <= 3.0),
                "{pose} {bx:?}: alone {ma:?}, 3D pass {mb:?}"
            );
        }
        same_disc(measure(&pass, white), measure(&alone, white), &format!("sun of the 3D pass {pose}"));
    }
}

#[test]
fn the_path_tracer_sees_the_same_sky() {
    let body = |renderer: &str| {
        format!("<camera id=\"c\" renderer=\"{renderer}\" pathSamples=\"4\" maxBounces=\"1\" denoise=\"false\"/><object3D id=\"o\" primitive=\"sphere\" radius=\"20\" x=\"600\" y=\"330\" material=\"m\"/>")
    };
    let Some(raster) = render_xml(&kit(&body("raster"), &midnight(""))) else { return };
    let traced = render_xml(&kit(&body("pathtrace"), &midnight(""))).unwrap();
    let (a, b) = (display(&raster), display(&traced));
    let high = mean(&raster, &a, [0, 0, 40, 40]);
    assert!(high[2] > 10.0, "sky {high:?}");
    for bx in [[0, 0, 40, 40], [580, 0, 620, 40], [100, 300, 140, 340]] {
        let (ma, mb) = (mean(&raster, &a, bx), mean(&traced, &b, bx));
        assert!(ma.iter().zip(mb).all(|(x, y)| (x - y).abs() <= 3.0), "{bx:?}: raster {ma:?}, traced {mb:?}");
    }
}

#[test]
fn the_full_moon_is_drawn_where_the_gradient_draws_a_sun_there() {
    // the full moon of the draft's row, at its lower culmination 85° N: low in the north under a sun far below
    let date = "2003-11-09T01:13:00Z";
    let longitude = (0..1440)
        .map(|k| -180.0 + k as f64 * 0.25)
        .min_by(|a, b| {
            let az = |l: f64| {
                let a = moon_from_document(date, &observer(85.0, l)).unwrap().0.azimuth;
                a.min(360.0 - a)
            };
            az(*a).total_cmp(&az(*b))
        })
        .unwrap();
    let (moon, _) = moon_from_document(date, &observer(85.0, longitude)).unwrap();
    assert!((5.0..20.0).contains(&(90.0 - moon.zenith)), "moon at {}", 90.0 - moon.zenith);
    let light = format!("<light id=\"sky\" type=\"dome\" sky=\"physical\" sunIntensity=\"1\" date=\"{date}\" latitude=\"85\" longitude=\"{longitude}\" moonIntensity=\"100000\" environmentVisible=\"true\"/>");
    let Some(physical) = render_xml(&kit("", &light)) else { return };
    let reference =
        render_xml(&kit("", &gradient_sun(moon.azimuth, 90.0 - moon.zenith, moon.angular_diameter))).unwrap();
    same_disc(measure(&physical, white), measure(&reference, red), "moon");
    let pass = render_xml(&kit(SMALL, &light)).unwrap();
    same_disc(measure(&pass, white), measure(&physical, white), "moon of the 3D pass");
    // without the moon nothing is that bright
    let dark = render_xml(&kit("", &light.replace("moonIntensity", "moon=\"false\" moonIntensity"))).unwrap();
    assert!(measure(&dark, white).is_none(), "a disc without the moon");
}

#[test]
fn the_sun_lights_the_scene_with_its_power() {
    // noon at the draft's Golden observer: the sun high behind the camera lights the ball's face
    let body = "<object3D id=\"o\" primitive=\"sphere\" radius=\"80\" x=\"320\" y=\"180\" material=\"m\"/>";
    let light = |size: &str| {
        format!("<light id=\"sky\" type=\"dome\" sky=\"physical\" sunIntensity=\"1\" date=\"2003-10-17T12:30:30-07:00\" latitude=\"39.742476\" longitude=\"-105.1786\" altitude=\"1830.14\" pressure=\"820\" temperature=\"11\" deltaT=\"67\" sunSize=\"{size}\"/>")
    };
    let Some(lit) = render_xml(&kit(body, &light("0.53"))) else { return };
    let sky = render_xml(&kit(body, &light("0"))).unwrap();
    let face = [300, 140, 340, 180];
    let (a, b) = (mean(&lit, &display(&lit), face), mean(&sky, &display(&sky), face));
    assert!(a.iter().sum::<f64>() > b.iter().sum::<f64>() + 30.0, "with the sun {a:?}, the sky alone {b:?}");
    assert!(b.iter().sum::<f64>() > 3.0, "the sky alone lights the ball: {b:?}");
}

#[test]
fn without_an_atmosphere_the_sky_is_black_and_the_tinted_disc_unveiled() {
    let (s, _) = sun_from_document(MIDNIGHT, &observer(75.0, 0.0)).unwrap();
    let Some(r) = render_xml(&kit("", &midnight("sunSize=\"5\" atmosphere=\"none\" sunColor=\"#FF0000\""))) else {
        return;
    };
    let d = display(&r);
    let high = mean(&r, &d, [0, 0, 40, 40]);
    assert!(high.iter().all(|c| *c <= 1.0), "a sky without an atmosphere {high:?}");
    let reference = render_xml(&kit("", &gradient_sun(s.azimuth, 90.0 - s.zenith, 5.0))).unwrap();
    same_disc(measure(&r, red), measure(&reference, red), "red sun");
}

#[test]
fn a_moving_observer_is_a_new_sky_each_frame() {
    // the longitude keyed from 0 to 180 between frames: frame 1 is the sky of longitude 180, not frame 0's kept
    let d = sr_model::load_str(
        &kit("", &midnight_keyed("longitude", "0", "180")),
        &sr_model::LoadOptions::without_assets(),
    )
    .unwrap();
    let Some(r) = render_times(&d, &[0.0, 1.0]) else { return };
    assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
    let fixed = render_xml(&kit("", &midnight("longitude=\"180\""))).unwrap();
    let (a, b) = (display(&r), display(&fixed));
    for bx in [[0, 0, 40, 40], [300, 150, 340, 170], [580, 40, 620, 80]] {
        let (ma, mb) = (mean(&r, &a, bx), mean(&fixed, &b, bx));
        assert!(ma.iter().zip(mb).all(|(x, y)| (x - y).abs() <= 1.0), "{bx:?}: keyed {ma:?}, fixed {mb:?}");
    }
}

#[test]
fn a_sky_the_atmosphere_refuses_is_reported() {
    // a keyed latitude past the pole: the frame names the light and the reason, and draws no sky
    let d = sr_model::load_str(
        &kit("", &midnight_keyed("latitude", "75", "120")),
        &sr_model::LoadOptions::without_assets(),
    )
    .unwrap();
    let Some(r) = render_times(&d, &[1.0]) else { return };
    assert!(r.stats.errors.iter().any(|e| e.contains("sky: ") && e.contains("latitude")), "{:?}", r.stats.errors);
    // and a frame whose 3D pass is lit by it
    let d = sr_model::load_str(
        &kit(SMALL, &midnight_keyed("latitude", "75", "120").replace(" environmentVisible=\"true\"", "")),
        &sr_model::LoadOptions::without_assets(),
    )
    .unwrap();
    let r = render_times(&d, &[1.0]).unwrap();
    assert!(r.stats.errors.iter().any(|e| e.contains("sky: ") && e.contains("latitude")), "{:?}", r.stats.errors);
}

#[test]
fn an_absent_sun_intensity_reads_as_one() {
    let Some(written) = render_xml(&kit("", &midnight(""))) else { return };
    let absent = render_xml(&kit("", &midnight("").replace(" sunIntensity=\"1\"", ""))).unwrap();
    let bx = [0, 0, 40, 40];
    let (a, b) = (mean(&written, &display(&written), bx), mean(&absent, &display(&absent), bx));
    assert!(a[2] > 10.0 && a == b, "sunIntensity=\"1\" {a:?}, absent {b:?}");
}

#[test]
fn sun_intensity_scales_the_sky() {
    let Some(full) = render_xml(&kit("", &midnight(""))) else { return };
    let quarter = render_xml(&kit("", &midnight("").replace("sunIntensity=\"1\"", "sunIntensity=\"0.25\""))).unwrap();
    let bx = [0, 0, 40, 40];
    let (a, b) = (mean(&full, &display(&full), bx), mean(&quarter, &display(&quarter), bx));
    assert!(b[2] + 10.0 < a[2] && b[2] > 0.0, "sunIntensity 1 {a:?}, 0.25 {b:?}");
}

#[test]
fn the_horizon_cuts_a_setting_sun_in_both_passes() {
    // the midnight sun half a degree up: a 5° disc whose lower part is below the horizon
    let latitude = (0..400)
        .map(|k| 60.0 + k as f64 * 0.025)
        .min_by(|a, b| {
            let el = |l: f64| (90.0 - sun_from_document(MIDNIGHT, &observer(l, 0.0)).unwrap().0.zenith - 0.5).abs();
            el(*a).total_cmp(&el(*b))
        })
        .unwrap();
    let light = midnight("sunSize=\"5\"").replace("latitude=\"75\"", &format!("latitude=\"{latitude}\""));
    let body = "<object3D id=\"o\" primitive=\"sphere\" radius=\"20\" x=\"40\" y=\"330\" material=\"m\"/>";
    let Some(alone) = render_xml(&kit("", &light)) else { return };
    let pass = render_xml(&kit(body, &light)).unwrap();
    let hot = |c: [u8; 3]| c[0] >= 250;
    let disc = measure(&alone, hot).expect("no setting sun drawn");
    assert!(disc[3] < disc[2] * 0.75, "the disc is not cut: {disc:?}");
    same_disc(measure(&pass, hot), Some(disc), "setting sun of the 3D pass");
}
