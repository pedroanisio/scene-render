//! The planets draft at the renderer (sr-core `srep/srep-0000-planets.md`, rules 1 and 2): `primitive="planet"` is the
//! spheroid of its body's IAU radii or of `radius` and `flattening`, and its material's equirectangular map is read
//! with longitude −180 at the left edge. The camera is a pinhole at the origin looking down +z, x to the right.

mod common;
use common::*;

const SIZE: u32 = 256;

/// A map of four vertical bands, red, green, blue and white from the left edge: longitudes −180 to −90, −90 to 0, 0 to
/// 90 and 90 to 180.
fn bands() -> std::path::PathBuf {
    let colours = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [255, 255, 255, 255]];
    let img = image::RgbaImage::from_fn(256, 128, |x, _| image::Rgba(colours[(x / 64) as usize]));
    write_fixture_png("planet-bands.png", &img)
}

/// A frame of one unlit planet with `attrs` at (0, 0, `z`), seen through a horizontal field of view of `fov` degrees.
fn frame(attrs: &str, z: f64, fov: f64, physics: &str) -> Option<Rendered> {
    let map = bands();
    let xml = format!(
        r##"<scene version="1.8"><project width="{SIZE}" height="{SIZE}" fps="24" duration="1" background="#000000FF"/><materials><material id="m" baseColor="#FFFFFFFF" unlit="true" baseColorMap="{}"/></materials><composition><camera id="eye" x="0" y="0" z="0" fov="{fov}"/><object3D id="p" primitive="planet" material="m" z="{z}" segments="128" {attrs}/></composition>{physics}</scene>"##,
        map.display()
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    let doc = sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"));
    let r = render(&doc)?;
    assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
    Some(r)
}

fn lit(p: [f32; 4]) -> bool {
    p[0] + p[1] + p[2] > 0.5
}

/// The half-extents in pixels of the disc along the frame's middle row and column.
fn extents(r: &Rendered) -> (f64, f64) {
    let c = SIZE / 2;
    let row = (0..SIZE).filter(|&x| lit(r.at(x, c))).count() as f64;
    let col = (0..SIZE).filter(|&y| lit(r.at(c, y))).count() as f64;
    (row / 2.0, col / 2.0)
}

/// The limb's half-extents (horizontal, vertical) in pixels of the spheroid of radii `a` and `c` at distance `d` along
/// the equator: the tangent cone of the sphere the spheroid becomes when y is scaled by a / c.
fn limb(a: f64, c: f64, d: f64, fov: f64) -> (f64, f64) {
    let f = SIZE as f64 / 2.0 / (fov / 2.0).to_radians().tan();
    let s = (d * d - a * a).sqrt();
    (f * a / s, f * c / s)
}

#[test]
fn the_facing_hemisphere_shows_the_maps_western_half() {
    // the camera sees longitude −90 at the disc's centre: the red band (−180 to −90) left of it, green right of it
    let Some(r) = frame(r#"radius="100""#, 400.0, 60.0, "") else { return };
    let c = SIZE / 2;
    let colour = |x: u32| {
        let p = r.at(x, c);
        [p[0] > 0.5, p[1] > 0.5, p[2] > 0.5]
    };
    assert_eq!(colour(c - 25), [true, false, false], "left of the centre");
    assert_eq!(colour(c + 25), [false, true, false], "right of the centre");
}

#[test]
fn the_silhouette_is_the_spheroids_limb() {
    let (a, f, d, fov) = (100.0, 0.5, 600.0, 30.0);
    let Some(r) = frame(&format!(r#"radius="{a}" flattening="{f}""#), d, fov, "") else { return };
    let (w, h) = extents(&r);
    let (ww, wh) = limb(a, a * (1.0 - f), d, fov);
    assert!((w - ww).abs() <= 1.5, "half-width {w} px, the limb's {ww}");
    assert!((h - wh).abs() <= 1.5, "half-height {h} px, the limb's {wh}");
}

#[test]
fn a_body_takes_its_iau_radii_through_pixels_per_meter() {
    // Saturn at 1e-6 px/m: 60.268 by 54.364 scene units, whatever radius and flattening say
    let (d, fov) = (360.0, 30.0);
    let physics = r#"<physics pixelsPerMeter="1e-6"/>"#;
    let Some(r) = frame(r#"body="699" radius="10""#, d, fov, physics) else { return };
    let (w, h) = extents(&r);
    let (ww, wh) = limb(60.268, 54.364, d, fov);
    assert!((w - ww).abs() <= 1.5, "half-width {w} px, Saturn's {ww}");
    assert!((h - wh).abs() <= 1.5, "half-height {h} px, Saturn's {wh}");
}

/// A frame of a planet of radius 100 at (0, 0, 400) seen through 60° with `materials`, the planet's `attrs`, `lights`
/// and `after` (the sections that follow the lights), and no others.
fn lit_frame(materials: &str, attrs: &str, lights: &str, after: &str) -> Option<Rendered> {
    let xml = format!(
        r##"<scene version="1.8"><project width="{SIZE}" height="{SIZE}" fps="24" duration="1" background="#000000FF"/><materials>{materials}</materials><composition><camera id="eye" x="0" y="0" z="0" fov="60"/><object3D id="p" primitive="planet" material="m" radius="100" z="400" segments="256" {attrs}/></composition><lights>{lights}</lights>{after}</scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    let doc = sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"));
    let r = render(&doc)?;
    assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
    Some(r)
}

/// The sphere's surface under pixel (x, y): its outward normal and the unit vector toward the eye, or none off the disc.
fn surface_at(x: u32, y: u32) -> Option<([f64; 3], [f64; 3])> {
    let f = SIZE as f64 / 2.0 / 30f64.to_radians().tan();
    let c = SIZE as f64 / 2.0;
    let d = [(x as f64 + 0.5 - c) / f, (y as f64 + 0.5 - c) / f, 1.0];
    let n = (d[0] * d[0] + d[1] * d[1] + 1.0).sqrt();
    let d = d.map(|v| v / n);
    // |t d − (0, 0, 400)|² = 100²
    let b = d[2] * 400.0;
    let disc = b * b - (400.0f64 * 400.0 - 100.0 * 100.0);
    if disc < 0.0 {
        return None;
    }
    let t = b - disc.sqrt();
    let p = d.map(|v| v * t);
    Some(([p[0] / 100.0, p[1] / 100.0, (p[2] - 400.0) / 100.0], d.map(|v| -v)))
}

fn white_map() -> std::path::PathBuf {
    let img = image::RgbaImage::from_pixel(64, 32, image::Rgba([255, 255, 255, 255]));
    write_fixture_png("planet-white.png", &img)
}

/// A black surface without specular reflection whose emissive map is white: what it shows is its emission alone.
fn night_material() -> String {
    format!(
        r##"<material id="m" baseColor="#000000FF" specular="0" emissive="#FFFFFF" emissiveMap="{}"/>"##,
        white_map().display()
    )
}

#[test]
fn night_lights_fade_across_the_twilight_band() {
    // the sun at +x (a directional light shining toward −x): the disc's right half is day, its left half night
    let sun = [1.0, 0.0, 0.0];
    // the default band is 6 degrees
    for (attrs, twilight) in [("", 6.0), (r#"twilight="30""#, 30.0)] {
        let Some(r) = lit_frame(&night_material(), attrs, r#"<light id="sun" type="directional" yaw="-90"/>"#, "")
        else {
            return;
        };
        let row = SIZE / 2;
        let mut checked = 0;
        for x in 0..SIZE {
            let Some((n, _)) = surface_at(x, row) else { continue };
            // away from the limb, where a pixel covers a wide band of normals
            if n[2] > -0.3 {
                continue;
            }
            let e = (n[0] * sun[0] + n[1] * sun[1] + n[2] * sun[2]).asin().to_degrees();
            let want = (0.5 - e / (2.0 * twilight)).clamp(0.0, 1.0);
            let got = r.at(x, row);
            for k in 0..3 {
                assert!(
                    (got[k] as f64 - want).abs() < 0.03,
                    "twilight {twilight}: x {x} elevation {e:.2}: {got:?} want {want}"
                );
            }
            checked += 1;
        }
        assert!(checked > 80, "{checked} pixels checked");
    }
}

#[test]
fn night_lights_off_or_without_a_sun_leave_the_map_whole() {
    let lights = r#"<light id="sun" type="directional" yaw="-90"/>"#;
    let Some(r) = lit_frame(&night_material(), r#"nightLights="false""#, lights, "") else { return };
    let c = SIZE / 2;
    for x in [c - 30, c + 30] {
        assert!(r.at(x, c)[..3].iter().all(|v| (v - 1.0).abs() < 0.03), "x {x}: {:?}", r.at(x, c));
    }
    // an ambient light is not a sun: every point is lit by its emission alone, unmasked
    let Some(r) = lit_frame(&night_material(), "", r#"<light id="sky" type="ambient" intensity="0"/>"#, "") else {
        return;
    };
    for x in [c - 30, c + 30] {
        assert!(r.at(x, c)[..3].iter().all(|v| (v - 1.0).abs() < 0.03), "x {x}: {:?}", r.at(x, c));
    }
    // a colour without a map is not masked
    let Some(r) =
        lit_frame(r##"<material id="m" baseColor="#000000FF" specular="0" emissive="#FFFFFF"/>"##, "", lights, "")
    else {
        return;
    };
    for x in [c - 30, c + 30] {
        assert!(r.at(x, c)[..3].iter().all(|v| (v - 1.0).abs() < 0.03), "x {x}: {:?}", r.at(x, c));
    }
}

#[test]
fn a_space_lights_the_night_side_away_from_the_sun() {
    // the June solstice of 2026 from the space's origin (the Earth-Moon barycentre): the Sun is toward +z and −y,
    // so the side facing the camera (−z) is night and its emission is whole at the disc's centre
    let space = r#"<space epoch="2026-06-21T08:24:00Z" origin="3"/>"#;
    let Some(r) = lit_frame(&night_material(), "", r#"<light id="sky" type="ambient" intensity="0"/>"#, space) else {
        return;
    };
    let c = SIZE / 2;
    assert!(r.at(c, c)[..3].iter().all(|v| (v - 1.0).abs() < 0.03), "{:?}", r.at(c, c));
    // a directional light comes first: the sun toward +x leaves the left half night and the right half day
    let lights = r#"<light id="sun" type="directional" yaw="-90"/>"#;
    let Some(r) = lit_frame(&night_material(), "", lights, space) else { return };
    assert!(r.at(c - 40, c)[..3].iter().all(|v| (v - 1.0).abs() < 0.03), "{:?}", r.at(c - 40, c));
    assert!(r.at(c + 40, c)[..3].iter().all(|v| v.abs() < 0.03), "{:?}", r.at(c + 40, c));
}

#[test]
fn the_suns_limb_darkens_and_reddens() {
    // the Sun of the draft's rule 3: an emissive white disc whose channels fall as mu^alpha at 610, 550 and 465 nm
    let alpha: [f64; 3] = [610.0, 550.0, 465.0].map(|nm| sr_3d::planet::limb_alpha(nm).unwrap());
    let Some(r) = lit_frame(
        r##"<material id="m" baseColor="#000000FF" specular="0" emissive="#FFFFFF"/>"##,
        r#"limbDarkening="solar""#,
        r#"<light id="sky" type="ambient" intensity="0"/>"#,
        "",
    ) else {
        return;
    };
    let row = SIZE / 2;
    let mut checked = 0;
    for x in 0..SIZE {
        let Some((n, v)) = surface_at(x, row) else { continue };
        let mu = n[0] * v[0] + n[1] * v[1] + n[2] * v[2];
        if mu < 0.2 {
            continue;
        }
        let got = r.at(x, row);
        for k in 0..3 {
            let want = mu.powf(alpha[k]);
            assert!((got[k] as f64 - want).abs() < 0.02, "x {x} mu {mu:.3} channel {k}: {got:?} want {want}");
        }
        checked += 1;
    }
    assert!(checked > 80, "{checked} pixels checked");
    // the limb is redder than the centre, by mu^(alpha red - alpha blue)
    let (edge, mu) = (0..SIZE)
        .find_map(|x| {
            let (n, v) = surface_at(x, row)?;
            let mu = n[0] * v[0] + n[1] * v[1] + n[2] * v[2];
            (mu > 0.25).then_some((x, mu))
        })
        .unwrap();
    let (e, m) = (r.at(edge, row), r.at(SIZE / 2, row));
    let want = mu.powf(alpha[0] - alpha[2]);
    let got = (e[0] / e[2]) as f64 / (m[0] / m[2]) as f64;
    assert!(want > 1.1 && (got / want - 1.0).abs() < 0.03, "{e:?} {m:?}: {got} want {want}");
}

#[test]
fn the_path_tracer_draws_the_night_lights_and_the_limb() {
    // the raster frames' night and limb behaviours, through the tracer: emission is what its first hit shows
    let pt = r#"renderer="pathtrace" pathSamples="1" maxBounces="1" denoise="false""#;
    let frame = |material: &str, attrs: &str, lights: &str| {
        let xml = format!(
            r##"<scene version="1.8"><project width="{SIZE}" height="{SIZE}" fps="24" duration="1" background="#000000FF"/><materials>{material}</materials><composition><camera id="eye" x="0" y="0" z="0" fov="60" {pt}/><object3D id="p" primitive="planet" material="m" radius="100" z="400" segments="256" {attrs}/></composition><lights>{lights}</lights></scene>"##
        );
        let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
        let doc = sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"));
        let r = render(&doc)?;
        assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
        assert!(r.stats.unsupported.iter().all(|u| !u.contains("path trac")), "{:?}", r.stats.unsupported);
        Some(r)
    };
    let c = SIZE / 2;
    let Some(r) = frame(&night_material(), "", r#"<light id="sun" type="directional" yaw="-90"/>"#) else { return };
    assert!(r.at(c - 40, c)[..3].iter().all(|v| (v - 1.0).abs() < 0.05), "{:?}", r.at(c - 40, c));
    assert!(r.at(c + 40, c)[..3].iter().all(|v| v.abs() < 0.05), "{:?}", r.at(c + 40, c));
    // a colour without a map is not masked
    let Some(r) = frame(
        r##"<material id="m" baseColor="#000000FF" specular="0" emissive="#FFFFFF"/>"##,
        "",
        r#"<light id="sun" type="directional" yaw="-90"/>"#,
    ) else {
        return;
    };
    for x in [c - 40, c + 40] {
        assert!(r.at(x, c)[..3].iter().all(|v| (v - 1.0).abs() < 0.05), "x {x}: {:?}", r.at(x, c));
    }
    let Some(r) = frame(
        r##"<material id="m" baseColor="#000000FF" specular="0" emissive="#FFFFFF"/>"##,
        r#"limbDarkening="solar""#,
        r#"<light id="sun" type="directional" yaw="-90"/>"#,
    ) else {
        return;
    };
    let alpha: [f64; 3] = [610.0, 550.0, 465.0].map(|nm| sr_3d::planet::limb_alpha(nm).unwrap());
    for x in [c, c - 30, c - 45] {
        let (n, v) = surface_at(x, c).unwrap();
        let mu = n[0] * v[0] + n[1] * v[1] + n[2] * v[2];
        let got = r.at(x, c);
        for k in 0..3 {
            let want = mu.powf(alpha[k]);
            assert!((got[k] as f64 - want).abs() < 0.03, "x {x} mu {mu:.3} channel {k}: {got:?} want {want}");
        }
    }
}

#[test]
fn the_sun_itself_has_no_night() {
    // body 10 at pixelsPerMeter 1.4374e-7 is a disc of radius about 100 at z 400, as the other frames' planet
    let lights = r#"<light id="sun" type="directional" yaw="-90"/>"#;
    let Some(r) = lit_frame(&night_material(), r#"body="10""#, lights, r#"<physics pixelsPerMeter="1.4374e-7"/>"#)
    else {
        return;
    };
    let c = SIZE / 2;
    for x in [c - 30, c + 30] {
        assert!(r.at(x, c)[..3].iter().all(|v| (v - 1.0).abs() < 0.03), "x {x}: {:?}", r.at(x, c));
    }
}

#[test]
fn a_planet_the_ephemeris_places_takes_the_sun_from_its_own_body() {
    // the Mars barycentre about a space whose origin is the Sun: from the origin there is no sun, from the planet the
    // Sun is toward the scene's origin, and part of the disc the orthographic camera sees is day (black on blue)
    let xml = format!(
        r##"<scene version="1.8"><project width="{SIZE}" height="{SIZE}" fps="24" duration="1" background="#0000FFFF"/><materials>{}</materials><composition><camera id="eye" projection="orthographic" z="-1000"/><object3D id="p" primitive="planet" material="m" radius="30" segments="128" body="4" positionFrom="ephemeris"/></composition><lights><light id="sky" type="ambient" intensity="0"/></lights><physics pixelsPerMeter="3e-10"/><space epoch="2027-03-15T00:00:00Z" origin="10"/></scene>"##,
        night_material()
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    let doc = sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"));
    let Some(r) = render(&doc) else { return };
    assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
    let (mut night, mut day) = (0, 0);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let p = r.at(x, y);
            night += p[..3].iter().all(|v| *v > 0.97) as u32;
            day += p[..3].iter().all(|v| *v < 0.03) as u32;
        }
    }
    // the disc covers about pi 30^2 = 2827 pixels
    assert!(night > 300 && day > 300 && night + day > 2400, "night {night} day {day}");
}

#[test]
fn through_an_orthographic_camera_the_line_of_sight_is_the_view_axis() {
    // the camera 50 units in front of the Sun's surface: toward its position the cosines would be far from the axis's
    let alpha: [f64; 3] = [610.0, 550.0, 465.0].map(|nm| sr_3d::planet::limb_alpha(nm).unwrap());
    let xml = format!(
        r##"<scene version="1.8"><project width="{SIZE}" height="{SIZE}" fps="24" duration="1" background="#000000FF"/><materials><material id="m" baseColor="#000000FF" specular="0" emissive="#FFFFFF"/></materials><composition><camera id="eye" projection="orthographic" orthoHeight="{SIZE}" z="-150"/><object3D id="p" primitive="planet" material="m" radius="100" segments="256" limbDarkening="solar"/></composition><lights><light id="sun" type="directional" yaw="-90"/></lights></scene>"##
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}"));
    let Some(r) = render(&doc) else { return };
    assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
    let c = SIZE / 2;
    let mut checked = 0;
    for x in c..SIZE {
        let u = (x as f64 + 0.5 - c as f64) / 100.0;
        let mu = (1.0 - u * u).max(0.0).sqrt();
        if mu < 0.2 {
            continue;
        }
        let got = r.at(x, c);
        for k in 0..3 {
            let want = mu.powf(alpha[k]);
            assert!((got[k] as f64 - want).abs() < 0.02, "x {x} mu {mu:.3} channel {k}: {got:?} want {want}");
        }
        checked += 1;
    }
    assert!(checked > 80, "{checked} pixels checked");
}
