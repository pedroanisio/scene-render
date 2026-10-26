//! The physical-sky draft (sr-core srep/srep-0000-physical-sky.md) on the GPU: a dome with `sky="physical"` draws its
//! sky and the discs of the dated sun and moon where SREP 71's gradient sky draws a sun of the same azimuth and
//! elevation, the same with and without a 3D pass, and lights the scene with the sun's power.

mod common;
use common::*;
use sr_3d::atmosphere::{atmosphere, atmosphere_position, AtmosphereOptions, Fog, Scattering};
use sr_3d::physical_sky::to_planet;
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

// ---------------------------------------------------------------- rules 10 and 11: aerial perspective and height fog

/// The draft's Golden observer at noon on a dome of the scene's axes, with `attrs`.
fn golden(attrs: &str) -> String {
    format!("<light id=\"sky\" type=\"dome\" sky=\"physical\" sunIntensity=\"1\" date=\"2003-10-17T12:30:30-07:00\" latitude=\"39.742476\" longitude=\"-105.1786\" moon=\"false\" {attrs}/>")
}

fn golden_sun() -> [f64; 3] {
    let o = Observer { latitude: 39.742476, longitude: -105.1786, ..observer(0.0, 0.0) };
    to_planet(sun_from_document("2003-10-17T12:30:30-07:00", &o).unwrap().1)
}

/// A scene in metres (`physics pixelsPerMeter="1"`) seen by a camera at (0, `y`, 0) looking along +z, with an unlit
/// grey material `u`.
fn metres(y: f64, body: &str, light: &str) -> String {
    scaled(1.0, &format!("y=\"{y}\" far=\"20000\""), body, light)
}

/// [`metres`] at `k` pixels a metre with the camera's attributes `camera`.
fn scaled(k: f64, camera: &str, body: &str, light: &str) -> String {
    kit(&format!("<camera id=\"c\" {camera}/>{body}"), light)
        .replace("</materials>", "<material id=\"u\" baseColor=\"#808080FF\" unlit=\"true\"/></materials>")
        .replace("</lights>", &format!("</lights><physics pixelsPerMeter=\"{k}\"/>"))
}

/// An unlit wall facing the camera, its near face at `z` − 5, centred on (0, `y`).
fn wall(id: &str, y: f64, z: f64, side: f64) -> String {
    format!("<object3D id=\"{id}\" primitive=\"box\" width=\"{side}\" height=\"{side}\" depth=\"10\" y=\"{y}\" z=\"{z}\" material=\"u\"/>")
}

/// The default lens: the focal length of a 60° frame 640 pixels wide.
const FOCAL: f64 = 554.2562584220407;

/// The scene ray through the centre of pixel (x, y) from a camera at (0, `cy`, 0): its origin at the eye, its unit
/// direction, and the length to the plane z = `z`.
fn pixel_ray(x: u32, y: u32, cy: f64, z: f64) -> ([f64; 3], [f64; 3], f64) {
    let d = [(x as f64 + 0.5 - 320.0) / FOCAL, (y as f64 + 0.5 - 180.0) / FOCAL, 1.0];
    let n = (d[0] * d[0] + d[1] * d[1] + 1.0).sqrt();
    ([0.0, cy, 0.0], d.map(|c| c / n), z * n)
}

fn rgb(p: [f32; 4]) -> [f64; 3] {
    [p[0] as f64, p[1] as f64, p[2] as f64]
}

/// A segment end's altitude for the closed form: below 0 it reads as 0 (rule 10, sr-core 78251fd).
fn alt(h: f64) -> f64 {
    h.max(0.0)
}

#[track_caller]
fn near(got: [f64; 3], want: [f64; 3], tol: f64, what: &str) {
    assert!(
        (0..3).all(|c| (got[c] - want[c]).abs() <= tol * want[c].abs().max(1e-3)),
        "{what}: {got:?}, the reference {want:?}"
    );
}

fn earth() -> Scattering {
    Scattering::new(atmosphere(&AtmosphereOptions::default()).unwrap())
}

#[test]
fn fog_without_air_is_the_closed_form() {
    // atmosphere="none": L_h is 0, so a wall keeps T_f of its colour, at ground level and 150 m up
    let body = format!("{}{}", wall("low", 0.0, 1000.0, 200.0), wall("high", -150.0, 1000.0, 60.0));
    let light = |fog: &str| golden(&format!("atmosphere=\"none\" {fog}"));
    let Some(clear) = render_xml(&metres(0.0, &body, &light(""))) else { return };
    let foggy = render_xml(&metres(0.0, &body, &light("fogDensity=\"0.001\" fogHeight=\"100\""))).unwrap();
    let f = Fog::new(0.001, 100.0).unwrap();
    for (x, y) in [(320, 180), (330, 96)] {
        let (o, d, l) = pixel_ray(x, y, 0.0, 995.0);
        let t = f.transmittance(alt(-o[1]), alt(-(o[1] + d[1] * l)), l);
        let c = rgb(clear.at(x, y));
        near(rgb(foggy.at(x, y)), c.map(|v| v * t), 0.01, &format!("pixel ({x}, {y}), T_f {t}"));
    }
    // through an orthographic camera the ray leaves the near plane straight along +z
    let ortho = |fog: &str| {
        metres(0.0, &body, &light(fog)).replace("far=\"20000\"", "far=\"20000\" projection=\"orthographic\"")
    };
    let (oc, of) =
        (render_xml(&ortho("")).unwrap(), render_xml(&ortho("fogDensity=\"0.001\" fogHeight=\"100\"")).unwrap());
    near(
        rgb(of.at(320, 180)),
        rgb(oc.at(320, 180)).map(|v| v * f.transmittance(0.0, 0.0, 995.0)),
        0.01,
        "orthographic",
    );
    // the ground-level wall loses e^(−1) of its light to the kit's digits
    assert!((rgb(foggy.at(320, 180))[0] / rgb(clear.at(320, 180))[0] - (-0.995f64).exp()).abs() < 0.005);
}

#[test]
fn the_closed_form_is_in_metres_from_the_eye_at_its_height() {
    // 100 pixels a metre, the eye 50 m up behind a near plane 200 m deep, a wall 1 km away from 0 to 100 m up: the
    // segment runs from the eye, its length and end altitudes in metres
    let (k, cy) = (100.0, -5000.0);
    let body = "<object3D id=\"w\" primitive=\"box\" width=\"40000\" height=\"10000\" depth=\"100\" y=\"-5000\" z=\"100050\" material=\"u\"/>";
    let camera = format!("y=\"{cy}\" near=\"20000\" far=\"1000000\"");
    let light = |fog: &str| golden(&format!("atmosphere=\"none\" {fog}"));
    let Some(clear) = render_xml(&scaled(k, &camera, body, &light(""))) else { return };
    let foggy = render_xml(&scaled(k, &camera, body, &light("fogDensity=\"0.001\" fogHeight=\"100\""))).unwrap();
    let f = Fog::new(0.001, 100.0).unwrap();
    for (x, y) in [(320, 180), (330, 140), (300, 215)] {
        let (o, d, l) = pixel_ray(x, y, cy, 100000.0);
        let t = f.transmittance(alt(-o[1] / k), alt(-(o[1] + d[1] * l) / k), l / k);
        let c = rgb(clear.at(x, y));
        near(rgb(foggy.at(x, y)), c.map(|v| v * t), 0.01, &format!("pixel ({x}, {y}), T_f {t}"));
    }
}

#[test]
fn a_dense_fog_near_level_keeps_its_digits() {
    // e^(−3) over 1 km with the wall's point 0.9 m above the eye: the closed form near level, to 0.4 %
    let body = wall("low", 0.0, 1000.0, 200.0);
    let light = |fog: &str| golden(&format!("atmosphere=\"none\" {fog}"));
    let Some(clear) = render_xml(&metres(0.0, &body, &light(""))) else { return };
    let foggy = render_xml(&metres(0.0, &body, &light("fogDensity=\"0.003\" fogHeight=\"100\""))).unwrap();
    let f = Fog::new(0.003, 100.0).unwrap();
    for (x, y) in [(320, 179), (320, 180)] {
        let (o, d, l) = pixel_ray(x, y, 0.0, 995.0);
        let t = f.transmittance(alt(-o[1]), alt(-(o[1] + d[1] * l)), l);
        near(rgb(foggy.at(x, y)), rgb(clear.at(x, y)).map(|v| v * t), 0.004, &format!("pixel ({x}, {y}), T_f {t}"));
    }
}

#[test]
fn fog_over_air_without_the_march_is_the_closed_form() {
    // aerialPerspective="false": the wall goes toward the sky's horizon radiance in its azimuth
    // with the dome's intensity 2 the horizon radiance doubles
    let body = format!("{}{}", wall("low", 0.0, 1000.0, 200.0), wall("high", -150.0, 1000.0, 60.0));
    let (s, f, sun) = (earth(), Fog::new(0.001, 100.0).unwrap(), golden_sun());
    let obs = [0.0, 0.0, s.atmosphere.planet_radius];
    for gain in [1.0, 2.0] {
        let light = |fog: &str| golden(&format!("aerialPerspective=\"false\" intensity=\"{gain}\" {fog}"));
        let Some(clear) = render_xml(&metres(0.0, &body, &light(""))) else { return };
        let foggy = render_xml(&metres(0.0, &body, &light("fogDensity=\"0.001\" fogHeight=\"100\""))).unwrap();
        for (x, y) in [(320, 180), (330, 96)] {
            let (o, d, l) = pixel_ray(x, y, 0.0, 995.0);
            let horizon = s.horizon_radiance(obs, to_planet(d), sun).unwrap().map(|c| c * gain);
            let want = f.fogged(rgb(clear.at(x, y)), alt(-o[1]), alt(-(o[1] + d[1] * l)), l, horizon);
            near(rgb(foggy.at(x, y)), want, 0.02, &format!("pixel ({x}, {y}), intensity {gain}"));
        }
    }
}

#[test]
fn the_march_holds_the_reference_on_far_walls() {
    // a camera 2 m up, a wall 5 km away standing on the ground, seen above the camera's level so that every ray
    // ends on the wall above the ground: the colour through the air, then through the air and the fog
    let body = wall("far", -600.0, 5000.0, 1200.0);
    let Some(plain) = render_xml(&metres(-2.0, &body, &golden("aerialPerspective=\"false\""))) else { return };
    let (s, sun) = (earth(), golden_sun());
    let r = s.atmosphere.planet_radius;
    let f = Fog::new(0.0002, 300.0).unwrap();
    for fog in [None, Some(f)] {
        let attrs = fog.map_or(String::new(), |f| format!("fogDensity=\"{}\" fogHeight=\"{}\"", f.density, f.height));
        let seen = render_xml(&metres(-2.0, &body, &golden(&attrs))).unwrap();
        for (x, y) in [(320, 170), (370, 140), (275, 175)] {
            let (o, d, l) = pixel_ray(x, y, -2.0, 4995.0);
            let (camera, dp) = (atmosphere_position(&s.atmosphere, o, 0.0, 1.0).unwrap(), to_planet(d));
            let c = rgb(plain.at(x, y));
            let want = match fog {
                None => s.aerial_perspective(camera, dp, l, c, sun).unwrap(),
                Some(f) => {
                    let horizon = s.horizon_radiance([0.0, 0.0, r], dp, sun).unwrap();
                    s.aerial_perspective_fogged(camera, dp, l, c, sun, &f, horizon).unwrap()
                }
            };
            near(rgb(seen.at(x, y)), want, 0.02, &format!("pixel ({x}, {y}), fog {fog:?}"));
        }
    }
}

#[test]
fn the_fog_leaves_the_sky_alone() {
    let body = wall("low", 0.0, 1000.0, 200.0);
    let light = |fog: &str| golden(&format!("environmentVisible=\"true\" {fog}"));
    let Some(clear) = render_xml(&metres(0.0, &body, &light(""))) else { return };
    let foggy = render_xml(&metres(0.0, &body, &light("fogDensity=\"0.01\" fogHeight=\"100\""))).unwrap();
    let (a, b) = (display(&clear), display(&foggy));
    for bx in [[0, 0, 40, 40], [580, 0, 620, 40], [300, 20, 340, 40]] {
        assert_eq!(mean(&clear, &a, bx), mean(&foggy, &b, bx), "sky {bx:?}");
    }
    assert!(
        mean(&clear, &a, [310, 170, 330, 190])[0] != mean(&foggy, &b, [310, 170, 330, 190])[0],
        "the wall is fogged"
    );
}

#[test]
fn a_moving_camera_is_a_new_volume_each_frame() {
    // the camera keyed 500 m back between frames: frame 1 is the far camera's volume, not frame 0's kept
    let body = wall("far", -2.0, 5000.0, 1200.0);
    let keyed = metres(-2.0, &body, &golden("")).replace(
        "far=\"20000\"/>",
        "far=\"20000\"><animate property=\"z\"><key time=\"0\" value=\"0\"/><key time=\"1\" value=\"-500\"/></animate></camera>",
    );
    let d = sr_model::load_str(&keyed, &sr_model::LoadOptions::without_assets()).unwrap();
    let Some(r) = render_times(&d, &[0.0, 1.0]) else { return };
    assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
    assert_eq!(r.renderer.cached_aerial_volumes(), 1, "one camera's volume is kept");
    let fixed =
        render_xml(&metres(-2.0, &body, &golden("")).replace("far=\"20000\"", "z=\"-500\" far=\"20000\"")).unwrap();
    for (x, y) in [(320, 180), (370, 140)] {
        near(rgb(r.at(x, y)), rgb(fixed.at(x, y)), 0.002, &format!("pixel ({x}, {y})"));
    }
}

#[test]
fn a_camera_under_the_ground_sees_sea_level_air() {
    // 78251fd: a surface segment is walked wherever its ends lie, an altitude below 0 reading as 0. A camera at the
    // implicit camera's eye and lens at the default scale (100 px a metre, no physics element) stands 1.8 m under
    // the sphere, looking at a grey wall 500,000 px (5 km) away at y = 330, with `far` raised past it (the default
    // is 10,000 px): the kit's companion row. An explicit camera's x, y and z default to 0, so the eye is written
    let (k, focal) = (100.0, FOCAL);
    let eye = [320.0, 180.0, -focal];
    let body = "<camera id=\"c\" x=\"320\" y=\"180\" z=\"-554.2562584220407\" far=\"600000\"/><object3D id=\"w\" primitive=\"box\" width=\"200000\" height=\"200000\" depth=\"100\" x=\"320\" y=\"330\" z=\"500050\" material=\"u\"/>";
    let xml = |light: &str| {
        kit(body, light)
            .replace("</materials>", "<material id=\"u\" baseColor=\"#808080FF\" unlit=\"true\"/></materials>")
    };
    let Some(plain) = render_xml(&xml(&golden("aerialPerspective=\"false\""))) else { return };
    let (s, sun) = (earth(), golden_sun());
    let r = s.atmosphere.planet_radius;
    let camera = atmosphere_position(&s.atmosphere, eye, 0.0, k).unwrap();
    assert!(camera[2] < r - 1.7, "the camera is under the sphere: {camera:?}");
    // the air alone (the kit's companion), then the air with a fog, whose 500 m change the colour decisively
    let f = Fog::new(0.002, 100.0).unwrap();
    for fog in [None, Some(f)] {
        let attrs = fog.map_or(String::new(), |f| format!("fogDensity=\"{}\" fogHeight=\"{}\"", f.density, f.height));
        let seen = render_xml(&xml(&golden(&attrs))).unwrap();
        for (x, y) in [(320, 180), (330, 186), (300, 175)] {
            let d = [(x as f64 + 0.5 - 320.0) / focal, (y as f64 + 0.5 - 180.0) / focal, 1.0];
            let n = (d[0] * d[0] + d[1] * d[1] + 1.0).sqrt();
            let (dp, l) = (to_planet(d.map(|v| v / n)), (500000.0 - eye[2]) * n / k);
            let c = rgb(plain.at(x, y));
            let want = match fog {
                None => s.aerial_perspective(camera, dp, l, c, sun).unwrap(),
                Some(f) => {
                    let horizon = s.horizon_radiance([0.0, 0.0, r], dp, sun).unwrap();
                    let fogged = s.aerial_perspective_fogged(camera, dp, l, c, sun, &f, horizon).unwrap();
                    assert!((0..3).any(|i| (fogged[i] - c[i]).abs() > 0.2 * c[i]), "{fogged:?} {c:?}");
                    fogged
                }
            };
            near(rgb(seen.at(x, y)), want, 0.02, &format!("pixel ({x}, {y}), fog {fog:?}"));
        }
    }
    // the fog's closed form reads both ends' altitudes below 0 as 0: a camera 500 m under the sphere (1 px a metre)
    // sees e^(−density × length) to a wall 1 km away at its own height, not the fog of 500 m below sea level
    let body = wall("w", 500.0, 1000.0, 200.0);
    let light = |fog: &str| golden(&format!("atmosphere=\"none\" {fog}"));
    let clear = render_xml(&metres(500.0, &body, &light(""))).unwrap();
    let foggy = render_xml(&metres(500.0, &body, &light("fogDensity=\"0.001\" fogHeight=\"100\""))).unwrap();
    for (x, y) in [(320, 180), (330, 150)] {
        let (_, _, l) = pixel_ray(x, y, 500.0, 995.0);
        let t = (-0.001 * l).exp();
        assert!(rgb(clear.at(x, y))[0] > 0.1, "the wall is seen at ({x}, {y})");
        near(rgb(foggy.at(x, y)), rgb(clear.at(x, y)).map(|v| v * t), 0.01, &format!("pixel ({x}, {y}), T_f {t}"));
    }
}

/// [`metres`] through the path tracer: unlit walls are emitters, so a pixel's colour is its surface's, jittered.
fn traced(y: f64, extra: &str, body: &str, light: &str) -> String {
    scaled(
        1.0,
        &format!("y=\"{y}\" far=\"20000\" {extra} renderer=\"pathtrace\" pathSamples=\"4\" denoise=\"false\""),
        body,
        light,
    )
}

#[test]
fn the_path_tracer_applies_the_march_and_the_closed_form() {
    // the march's wall, through the air and through the air and the fog
    let body = wall("far", -600.0, 5000.0, 1200.0);
    let Some(plain) = render_xml(&traced(-2.0, "", &body, &golden("aerialPerspective=\"false\""))) else { return };
    let (s, sun) = (earth(), golden_sun());
    let r = s.atmosphere.planet_radius;
    let f = Fog::new(0.0002, 300.0).unwrap();
    for fog in [None, Some(f)] {
        let attrs = fog.map_or(String::new(), |f| format!("fogDensity=\"{}\" fogHeight=\"{}\"", f.density, f.height));
        let seen = render_xml(&traced(-2.0, "", &body, &golden(&attrs))).unwrap();
        for (x, y) in [(320, 170), (370, 140), (275, 175)] {
            let (o, d, l) = pixel_ray(x, y, -2.0, 4995.0);
            let (camera, dp) = (atmosphere_position(&s.atmosphere, o, 0.0, 1.0).unwrap(), to_planet(d));
            let c = rgb(plain.at(x, y));
            let want = match fog {
                None => s.aerial_perspective(camera, dp, l, c, sun).unwrap(),
                Some(f) => {
                    let horizon = s.horizon_radiance([0.0, 0.0, r], dp, sun).unwrap();
                    s.aerial_perspective_fogged(camera, dp, l, c, sun, &f, horizon).unwrap()
                }
            };
            // past the tolerance below: the surface's colour alone fails
            assert!((0..3).any(|i| (want[i] - c[i]).abs() > 0.03 * c[i]), "the air shows: {want:?} {c:?}");
            near(rgb(seen.at(x, y)), want, 0.02, &format!("traced pixel ({x}, {y}), fog {fog:?}"));
        }
    }
    // the closed form over the air, at the dome's intensity 2, and without air through both projections
    let body = format!("{}{}", wall("low", 0.0, 1000.0, 200.0), wall("high", -150.0, 1000.0, 60.0));
    let fog = Fog::new(0.001, 100.0).unwrap();
    let foggy = "fogDensity=\"0.001\" fogHeight=\"100\"";
    let over = |a: &str| golden(&format!("aerialPerspective=\"false\" intensity=\"2\" {a}"));
    let (clear, seen) = (
        render_xml(&traced(0.0, "", &body, &over(""))).unwrap(),
        render_xml(&traced(0.0, "", &body, &over(foggy))).unwrap(),
    );
    for (x, y) in [(320, 180), (330, 96)] {
        let (o, d, l) = pixel_ray(x, y, 0.0, 995.0);
        let horizon = s.horizon_radiance([0.0, 0.0, r], to_planet(d), sun).unwrap().map(|c| c * 2.0);
        let want = fog.fogged(rgb(clear.at(x, y)), alt(-o[1]), alt(-(o[1] + d[1] * l)), l, horizon);
        near(rgb(seen.at(x, y)), want, 0.02, &format!("traced pixel ({x}, {y}) over the air"));
    }
    let none = |a: &str| golden(&format!("atmosphere=\"none\" {a}"));
    // a near plane 200 m deep: the perspective segment leaves the eye, the orthographic one the near plane
    for (camera, length) in [("near=\"200\"", 995.0), ("near=\"200\" projection=\"orthographic\"", 795.0)] {
        let clear = render_xml(&traced(0.0, camera, &body, &none(""))).unwrap();
        let seen = render_xml(&traced(0.0, camera, &body, &none(foggy))).unwrap();
        let t = fog.transmittance(0.0, 0.0, length);
        near(rgb(seen.at(320, 180)), rgb(clear.at(320, 180)).map(|v| v * t), 0.01, &format!("traced {camera}"));
    }
    // e^(−3) over 1 km near level, to 0.4 %: rows whose every jittered sample ends 7 to 10 m up, x = h₁/H under 0.01
    let dense = Fog::new(0.003, 1000.0).unwrap();
    let thick = "fogDensity=\"0.003\" fogHeight=\"1000\"";
    let (clear, seen) = (
        render_xml(&traced(0.0, "", &body, &none(""))).unwrap(),
        render_xml(&traced(0.0, "", &body, &none(thick))).unwrap(),
    );
    for (x, y) in [(320, 175), (330, 176)] {
        let (o, d, l) = pixel_ray(x, y, 0.0, 995.0);
        let t = dense.transmittance(alt(-o[1]), alt(-(o[1] + d[1] * l)), l);
        near(rgb(seen.at(x, y)), rgb(clear.at(x, y)).map(|v| v * t), 0.004, &format!("traced dense ({x}, {y})"));
    }
    // a camera 500 m under the sphere: both ends read as altitude 0
    let low = wall("w", 500.0, 1000.0, 200.0);
    let (clear, seen) = (
        render_xml(&traced(500.0, "", &low, &none(""))).unwrap(),
        render_xml(&traced(500.0, "", &low, &none(foggy))).unwrap(),
    );
    let (_, _, l) = pixel_ray(320, 150, 500.0, 995.0);
    near(
        rgb(seen.at(320, 150)),
        rgb(clear.at(320, 150)).map(|v| v * (-0.001 * l).exp()),
        0.01,
        "traced under the sphere",
    );
    // 100 pixels a metre: the segment's length and heights in metres
    let (k, cy) = (100.0, -5000.0);
    let far = "<object3D id=\"w\" primitive=\"box\" width=\"40000\" height=\"10000\" depth=\"100\" y=\"-5000\" z=\"100050\" material=\"u\"/>";
    let camera =
        format!("y=\"{cy}\" near=\"20000\" far=\"1000000\" renderer=\"pathtrace\" pathSamples=\"4\" denoise=\"false\"");
    let (clear, seen) = (
        render_xml(&scaled(k, &camera, far, &none(""))).unwrap(),
        render_xml(&scaled(k, &camera, far, &none(foggy))).unwrap(),
    );
    for (x, y) in [(320, 180), (330, 140)] {
        let (o, d, l) = pixel_ray(x, y, cy, 100000.0);
        let t = fog.transmittance(alt(-o[1] / k), alt(-(o[1] + d[1] * l) / k), l / k);
        near(
            rgb(seen.at(x, y)),
            rgb(clear.at(x, y)).map(|v| v * t),
            0.01,
            &format!("traced ({x}, {y}) at 100 px a metre"),
        );
    }
}

#[test]
fn the_path_tracer_scales_the_march_by_the_dome() {
    // with the dome's intensity 2 the air's light doubles and its transmittance stays: ap(c) + ap(0)
    let body = wall("far", -600.0, 5000.0, 1200.0);
    let light = |a: &str| golden(&format!("intensity=\"2\" {a}"));
    let Some(plain) = render_xml(&traced(-2.0, "", &body, &light("aerialPerspective=\"false\""))) else { return };
    let seen = render_xml(&traced(-2.0, "", &body, &light(""))).unwrap();
    let (s, sun) = (earth(), golden_sun());
    for (x, y) in [(320, 170), (370, 140)] {
        let (o, d, l) = pixel_ray(x, y, -2.0, 4995.0);
        let (camera, dp) = (atmosphere_position(&s.atmosphere, o, 0.0, 1.0).unwrap(), to_planet(d));
        let c = rgb(plain.at(x, y));
        let (lit, air) = (
            s.aerial_perspective(camera, dp, l, c, sun).unwrap(),
            s.aerial_perspective(camera, dp, l, [0.0; 3], sun).unwrap(),
        );
        near(rgb(seen.at(x, y)), [0, 1, 2].map(|i| lit[i] + air[i]), 0.02, &format!("traced pixel ({x}, {y})"));
    }
}

#[test]
fn a_thick_fog_shows_the_horizon_between_its_nodes() {
    // e^(−10) over 1 km under the midnight sun, low in the camera's view: the wall is the horizon's radiance in each
    // pixel's azimuth, which changes by 2 to 6 % a node there; the pixels lie halfway between the table's nodes
    let body = wall("low", 0.0, 1000.0, 600.0);
    let light = midnight("aerialPerspective=\"false\" fogDensity=\"0.01\" fogHeight=\"100\"");
    let (s, sun) = (earth(), to_planet(sun_from_document(MIDNIGHT, &observer(75.0, 0.0)).unwrap().1));
    let r = s.atmosphere.planet_radius;
    for extra in ["", "renderer=\"pathtrace\" pathSamples=\"4\" denoise=\"false\""] {
        let Some(seen) = render_xml(&scaled(1.0, &format!("far=\"20000\" {extra}"), &body, &light)) else { return };
        for x in [217, 256, 373, 402, 442] {
            let (_, d, _) = pixel_ray(x, 180, 0.0, 995.0);
            let want = s.horizon_radiance([0.0, 0.0, r], to_planet(d), sun).unwrap();
            near(rgb(seen.at(x, 180)), want, 0.005, &format!("{extra} pixel ({x}, 180)"));
        }
    }
}

#[test]
fn the_path_tracer_leaves_the_sky_alone() {
    let body = wall("low", 0.0, 1000.0, 200.0);
    let light = |fog: &str| golden(&format!("environmentVisible=\"true\" {fog}"));
    let Some(clear) = render_xml(&traced(0.0, "", &body, &light(""))) else { return };
    let foggy = render_xml(&traced(0.0, "", &body, &light("fogDensity=\"0.01\" fogHeight=\"100\""))).unwrap();
    for (x, y) in [(20, 20), (600, 20), (320, 30)] {
        assert_eq!(rgb(clear.at(x, y)), rgb(foggy.at(x, y)), "sky ({x}, {y})");
    }
    assert!(rgb(clear.at(320, 180))[0] != rgb(foggy.at(320, 180))[0], "the wall is fogged");
}

#[test]
fn a_keyed_date_holds_each_key_until_the_next() {
    // a string-valued key steps: halfway between the keys the sky is the first key's, at the second key the second's
    let noon = "2024-06-21T12:00:00Z";
    let keyed = kit("", &midnight_keyed("date", MIDNIGHT, noon));
    let d = sr_model::load_str(&keyed, &sr_model::LoadOptions::without_assets()).unwrap();
    let Some(half) = render_times(&d, &[0.5]) else { return };
    let end = render_times(&d, &[0.5, 1.0]).unwrap();
    assert!(
        half.stats.errors.is_empty() && end.stats.errors.is_empty(),
        "{:?} {:?}",
        half.stats.errors,
        end.stats.errors
    );
    let first = render_xml(&kit("", &midnight(""))).unwrap();
    let second = render_xml(&kit("", &midnight("")).replace(MIDNIGHT, noon)).unwrap();
    let sky = |r: &Rendered| mean(r, &display(r), [0, 0, 40, 40]);
    let same = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 1.0);
    assert!(!same(sky(&first), sky(&second)), "the two dates differ: {:?} {:?}", sky(&first), sky(&second));
    assert!(same(sky(&half), sky(&first)), "halfway: {:?}, the first key {:?}", sky(&half), sky(&first));
    assert!(same(sky(&end), sky(&second)), "at the second key: {:?}, {:?}", sky(&end), sky(&second));
}

#[test]
fn the_sun_disc_is_brighter_than_one_in_linear_values() {
    // the frame keeps radiance above 1.0 for float outputs: the midnight sun's disc is far brighter than white
    let Some(r) = render_xml(&kit("", &midnight("sunSize=\"5\""))) else { return };
    let peak = r.px.iter().map(|p| p[0].max(p[1]).max(p[2])).fold(0.0f32, f32::max);
    assert!(peak > 10.0, "the disc's peak linear value {peak}");
}
