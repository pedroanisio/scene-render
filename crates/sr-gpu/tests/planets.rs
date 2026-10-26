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
    surface_through(x as f64 + 0.5, y as f64 + 0.5)
}

/// [`surface_at`] through the point (px, py) of the frame, in pixels.
fn surface_through(px: f64, py: f64) -> Option<([f64; 3], [f64; 3])> {
    let f = SIZE as f64 / 2.0 / 30f64.to_radians().tan();
    let c = SIZE as f64 / 2.0;
    let d = [(px - c) / f, (py - c) / f, 1.0];
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

/// A cloud map, 360 × 180 (a texel a degree), of colour `rgb` and alpha 1 over the longitudes `lo` to `hi` and 0 elsewhere.
fn cloud_band(name: &str, rgb: [u8; 3], (lo, hi): (f64, f64), (south, north): (f64, f64)) -> std::path::PathBuf {
    let img = image::RgbaImage::from_fn(360, 180, |x, y| {
        let (lon, lat) = (x as f64 + 0.5 - 180.0, 90.0 - y as f64 - 0.5);
        let a = if (lo..hi).contains(&lon) && (south..north).contains(&lat) { 255 } else { 0 };
        image::Rgba([rgb[0], rgb[1], rgb[2], a])
    });
    write_fixture_png(name, &img)
}

#[test]
fn the_raster_pass_names_the_clouds_shadows_it_does_not_draw() {
    let map = cloud_band("planet-cloud-band.png", [0, 0, 0], (-50.0, -30.0), (-90.0, 90.0));
    let attrs = format!(r#"cloudMap="{}" cloudHeight="0.2""#, map.display());
    let light = r#"<light id="sun" type="directional" yaw="-90" intensity="3"/>"#;
    // the cloud layer takes the occlusion map's binding, which a ground with one keeps: lit as without clouds where
    // the eye sees it bare (an opaque white map read as the layer would leave it black)
    let ground = format!(
        r##"<material id="m" baseColor="#FFFFFFFF" specular="0" roughness="1" occlusionMap="{}"/>"##,
        white_map().display()
    );
    let (Some(bare), Some(r)) = (planet_at(&ground, "", light, 0.0), planet_at(&ground, &attrs, light, 0.0)) else {
        return;
    };
    let note = "p: the clouds' shadow on a ground with an occlusionMap is not drawn by the raster pass";
    assert!(r.stats.unsupported.iter().any(|u| u == note), "{:?}", r.stats.unsupported);
    assert!(!bare.stats.unsupported.iter().any(|u| u == note), "{:?}", bare.stats.unsupported);
    let c = SIZE / 2;
    let seen = crossing(c + 30, c, 120.0).unwrap();
    assert!(!(-52.0..-28.0).contains(&longitude(seen)), "x {} sees the band at {:.1}", c + 30, longitude(seen));
    let (got, want) = (r.at(c + 30, c)[0], bare.at(c + 30, c)[0]);
    assert!(want > 0.2 && (got - want).abs() < 0.02, "{got} with the clouds, {want} without");
    // under a physical sky's sun the shadow, which darkens the light of directional lights, is not drawn
    let sky = r#"<light id="sky" type="dome" sky="physical" sunIntensity="1" date="2024-06-21T12:00:00Z" latitude="0" moon="false"/>"#;
    let material = r##"<material id="m" baseColor="#FFFFFFFF" specular="0" roughness="1"/>"##;
    let Some(r) = planet_at(material, &attrs, sky, 0.0) else { return };
    let note = "p: the clouds' shadow under a physical sky's sun is not drawn";
    assert!(r.stats.unsupported.iter().any(|u| u == note), "{:?}", r.stats.unsupported);
    let Some(r) = planet_at(material, &format!(r#"{attrs} cloudShadow="false""#), sky, 0.0) else { return };
    assert!(!r.stats.unsupported.iter().any(|u| u == note), "{:?}", r.stats.unsupported);
}

#[test]
fn the_path_tracer_casts_the_clouds_shadow_unless_it_is_off() {
    // the raster frame's patch, traced on a sphere and an oblate body: ground seen bare is the frame without the
    // shadow times 1 − alpha at its shadow point
    let ((lo, hi), (south, north)) = ((-50.0, -30.0), (10.0, 60.0));
    let map = cloud_band("planet-cloud-patch.png", [0, 0, 0], (lo, hi), (south, north));
    let frame = |shadow: bool, f: f64, turn: f64| {
        let xml = format!(
            r##"<scene version="1.8"><project width="{SIZE}" height="{SIZE}" fps="24" duration="1" background="#000000FF"/><materials><material id="m" baseColor="#FFFFFFFF" specular="0" roughness="1"/></materials><composition><camera id="eye" x="0" y="0" z="0" fov="60" renderer="pathtrace" pathSamples="1" maxBounces="1" denoise="false"/><object3D id="p" primitive="planet" material="m" radius="100" z="400" segments="256" flattening="{f}" rotationY="{turn}" cloudMap="{}" cloudHeight="0.2" cloudShadow="{shadow}"/></composition><lights><light id="sun" type="directional" yaw="-90" intensity="3"/></lights></scene>"##,
            map.display()
        );
        let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
        let doc = sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"));
        let r = render(&doc)?;
        assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
        Some(r)
    };
    // a body turned about its pole (rotationY θ, westward) carries its layer θ west: the sun meets it in its own axes
    for (f, turn) in [(0.0, 0.0), (0.3, 0.0), (0.3, 15.0)] {
        let (Some(on), Some(off)) = (frame(true, f, turn), frame(false, f, turn)) else { return };
        // alpha at a shell point, its texel at the point's geodetic latitude; none near the patch's edges, which the
        // tracer's sample jittered within its pixel may straddle
        let alpha = |q: [f64; 3]| {
            let (lon, lat) = (longitude(q) + turn, geodetic_latitude(q, f));
            let near = |v: f64, e: f64| (v - e).abs() < 3.0;
            (!(near(lon, lo) || near(lon, hi) || near(lat, south) || near(lat, north)))
                .then_some(if (lo..hi).contains(&lon) && (south..north).contains(&lat) { 1.0 } else { 0.0 })
        };
        let c = SIZE / 2;
        let (mut checked, mut shaded) = (0, 0);
        for row in (c - 50..c + 50).step_by(5) {
            for x in 0..SIZE {
                let (Some(p), Some(qv)) = (crossing_spheroid(x, row, 100.0, f), crossing_spheroid(x, row, 120.0, f))
                else {
                    continue;
                };
                // the ground seen bare, lit
                let lit = off.at(x, row)[0];
                if alpha(qv) != Some(0.0) || lit < 0.05 {
                    continue;
                }
                let through = match shadow_point(p, [1.0, 0.0, 0.0], 100.0, 0.2, f).map(alpha) {
                    Some(Some(a)) => 1.0 - a as f32,
                    Some(None) => continue,
                    None => 1.0,
                };
                let dark = on.at(x, row)[0];
                assert!(
                    (dark - lit * through).abs() < 0.03,
                    "f {f} turn {turn} ({x}, {row}): {dark} under the clouds, {lit} without"
                );
                checked += 1;
                shaded += (through < 0.5) as u32;
            }
        }
        assert!(checked > 200 && shaded > 20, "f {f} turn {turn}: {checked} pixels checked, {shaded} in the shadow");
    }
}

/// The planet of radius 100 (unless `attrs` gives one) at (0, 0, 400) seen through 60°, of `material` with `attrs`,
/// under `lights`, at `t` seconds.
fn planet_at(material: &str, attrs: &str, lights: &str, t: f64) -> Option<Rendered> {
    planet_seen("", material, attrs, lights, t)
}

/// The path tracer's camera of one sample a pixel, its first hit's direct light only.
const TRACED: &str = r#"renderer="pathtrace" pathSamples="1" maxBounces="1" denoise="false""#;

/// `planet_at` through a camera of attributes `camera`.
fn planet_seen(camera: &str, material: &str, attrs: &str, lights: &str, t: f64) -> Option<Rendered> {
    let radius = if attrs.contains("radius=") { "" } else { r#"radius="100""# };
    let xml = format!(
        r##"<scene version="1.8"><project width="{SIZE}" height="{SIZE}" fps="24" duration="4" background="#000000FF"/><materials>{material}</materials><composition><camera id="eye" x="0" y="0" z="0" fov="60" {camera}/><object3D id="p" primitive="planet" material="m" {radius} z="400" segments="256" {attrs}/></composition><lights>{lights}</lights></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    let doc = sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"));
    let r = render_times(&doc, &[t])?;
    assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
    Some(r)
}

/// The first point at distance `radius` from the planet's centre along pixel (x, y)'s ray, in the planet's frame.
fn crossing(x: u32, y: u32, radius: f64) -> Option<[f64; 3]> {
    crossing_spheroid(x, y, radius, 0.0)
}

/// The first point of the spheroid of equatorial `radius` and `flattening` (its pole the scene's y) along pixel
/// (x, y)'s ray, in the planet's frame.
fn crossing_spheroid(x: u32, y: u32, radius: f64, flattening: f64) -> Option<[f64; 3]> {
    let f = SIZE as f64 / 2.0 / 30f64.to_radians().tan();
    let c = SIZE as f64 / 2.0;
    let d = [(x as f64 + 0.5 - c) / f, (y as f64 + 0.5 - c) / f, 1.0];
    // stretched along the pole the spheroid is the sphere of its equatorial radius; the eye is 400 before the centre
    let k = 1.0 / (1.0 - flattening);
    let ds = [d[0], d[1] * k, d[2]];
    let a = ds[0] * ds[0] + ds[1] * ds[1] + ds[2] * ds[2];
    let b = -400.0 * ds[2];
    let disc = b * b - a * (400.0f64 * 400.0 - radius * radius);
    (disc >= 0.0).then(|| {
        let t = (-b - disc.sqrt()) / a;
        [d[0] * t, d[1] * t, d[2] * t - 400.0]
    })
}

/// `cloud_shadow_point` of a scene-axes point of the planet's frame toward a scene-axes `sun`: the kernel's pole is
/// z, the scene's −y.
fn shadow_point(p: [f64; 3], sun: [f64; 3], radius: f64, height: f64, flattening: f64) -> Option<[f64; 3]> {
    let (body, sun) = ([p[0], p[2], -p[1]], [sun[0], sun[2], -sun[1]]);
    let q = sr_3d::planet::cloud_shadow_point(body, sun, radius, height, flattening).unwrap()?;
    Some([q[0], -q[2], q[1]])
}

/// The geodetic latitude (degrees) of a scene-axes point of a spheroid of `flattening` (its north pole −y).
fn geodetic_latitude(q: [f64; 3], flattening: f64) -> f64 {
    (-q[1]).atan2((1.0 - flattening).powi(2) * (q[0] * q[0] + q[2] * q[2]).sqrt()).to_degrees()
}

/// The longitude of a scene-axes point of the planet's frame (the body's x is the scene's x, its y the scene's z).
fn longitude(p: [f64; 3]) -> f64 {
    p[2].atan2(p[0]).to_degrees()
}

#[test]
fn the_cloud_shell_stands_above_the_ground_and_drifts_east() {
    // white clouds over the longitudes −180 to −90, the left of the disc; the ground black and the light from the eye
    let map = cloud_band("planet-cloud-west.png", [255, 255, 255], (-180.0, -90.0), (-90.0, 90.0));
    let material = r##"<material id="m" baseColor="#000000FF" specular="0" roughness="1"/>"##;
    let attrs = format!(r#"cloudMap="{}" cloudHeight="0.2" cloudRotation="90""#, map.display());
    let light = r#"<light id="sun" type="directional" intensity="3"/>"#;
    let c = SIZE / 2;
    // the ground's limb is 57 pixels from the centre, the shell's (radius 120) 70
    let Some(r) = planet_at(material, &attrs, light, 0.0) else { return };
    assert!(r.at(c - 30, c)[0] > 0.2, "{:?}", r.at(c - 30, c));
    assert!(r.at(c - 64, c)[0] > 0.05, "the shell beyond the ground's limb: {:?}", r.at(c - 64, c));
    assert!(r.at(c + 30, c)[0] < 0.01, "{:?}", r.at(c + 30, c));
    // after a second at 90 degrees a second, eastward: the clouds over −90 to 0, the right of the disc
    let Some(r) = planet_at(material, &attrs, light, 1.0) else { return };
    assert!(r.at(c - 30, c)[0] < 0.01, "{:?}", r.at(c - 30, c));
    assert!(r.at(c + 30, c)[0] > 0.2, "{:?}", r.at(c + 30, c));
}

/// A cloud-shadow frame to check: black opaque clouds over `lon` and `lat` (degrees), the sun toward longitude 0 at
/// latitude `sun_lat` north (pitch −`sun_lat` from the yaw −90 light), the shadow on or off, the layer's drift rate
/// and time, the body's flattening.
struct ShadowCase {
    lon: (f64, f64),
    lat: (f64, f64),
    sun_lat: f64,
    shadow: bool,
    rate: f64,
    t: f64,
    f: f64,
}

/// Every lit pixel of `rows` whose texels are clear of the patch's edges shows the bare ground times 1 − alpha at its
/// shadow point and at its view through the shell (h = 0.2); returns how many it checked and how many sat in the
/// shadow seen bare.
fn check_shadow(c: &ShadowCase, rows: &[u32]) -> Option<(u32, u32)> {
    let (h, (lo, hi), (south, north)) = (0.2, c.lon, c.lat);
    let name = format!("planet-cloud-{lo}-{hi}-{south}-{north}.png");
    let map = cloud_band(&name, [0, 0, 0], (lo, hi), (south, north));
    let material = r##"<material id="m" baseColor="#FFFFFFFF" specular="0" roughness="1"/>"##;
    let light = format!(r#"<light id="sun" type="directional" yaw="-90" pitch="{}" intensity="3"/>"#, -c.sun_lat);
    let body = format!(r#"flattening="{}""#, c.f);
    let clouds = format!(
        r#"cloudMap="{}" cloudHeight="{h}" cloudRotation="{}" cloudShadow="{}" {body}"#,
        map.display(),
        c.rate,
        c.shadow
    );
    let bare = planet_at(material, &body, &light, 0.0)?;
    let r = planet_at(material, &clouds, &light, c.t)?;
    let drift = c.rate * c.t;
    let phi = c.sun_lat.to_radians();
    let sun = [phi.cos(), -phi.sin(), 0.0];
    // alpha of the drifted layer at a shell point, its texel at the point's geodetic latitude, none near the patch's
    // edges (the texels' filtering)
    let alpha = |q: [f64; 3]| {
        let (lon, lat) = (longitude(q) - drift, geodetic_latitude(q, c.f));
        let near = |v: f64, e: f64| (v - e).abs() < 1.5;
        (!(near(lon, lo) || near(lon, hi) || near(lat, south) || near(lat, north)))
            .then_some(if (lo..hi).contains(&lon) && (south..north).contains(&lat) { 1.0 } else { 0.0 })
    };
    let (mut checked, mut shaded) = (0, 0);
    for &row in rows {
        for x in 0..SIZE {
            let (Some(p), Some(qv)) =
                (crossing_spheroid(x, row, 100.0, c.f), crossing_spheroid(x, row, 100.0 * (1.0 + h), c.f))
            else {
                continue;
            };
            let lit = bare.at(x, row)[0] as f64;
            if lit < 0.05 || p[2] > -30.0 {
                continue;
            }
            let Some(seen) = alpha(qv) else { continue };
            let through = match shadow_point(p, sun, 100.0, h, c.f) {
                Some(q) if c.shadow => match alpha(q) {
                    Some(a) => 1.0 - a,
                    None => continue,
                },
                _ => 1.0,
            };
            let want = lit * through * (1.0 - seen);
            let got = r.at(x, row)[0] as f64;
            assert!(
                (got - want).abs() < 0.03,
                "shadow {} drift {drift} flattening {} sun {}: ({x}, {row}) lon {:.1}: {got} want {want}",
                c.shadow,
                c.f,
                c.sun_lat,
                longitude(p)
            );
            checked += 1;
            shaded += (through < 0.5 && seen == 0.0) as u32;
        }
    }
    Some((checked, shaded))
}

#[test]
fn the_clouds_shadow_falls_where_the_sunward_ray_leaves_the_shell() {
    // the patch over the longitudes −50 to −30 and the latitudes 10 to 60 N, the ground white; the north is up the
    // frame (the scene's −y)
    let c = SIZE / 2;
    let rows = [c - 50, c - 40, c - 30, c - 20, c - 10, c, c + 20];
    let patch =
        |shadow, rate, t, f| ShadowCase { lon: (-50.0, -30.0), lat: (10.0, 60.0), sun_lat: 0.0, shadow, rate, t, f };
    // a westward drift of 20 degrees (the layer's 340) keeps its shadow on the side the eye sees past the patch; on
    // an oblate body the shell is its spheroid scaled by 1 + h
    for case in [
        patch(true, 0.0, 0.0, 0.0),
        patch(true, -10.0, 2.0, 0.0),
        patch(false, 0.0, 0.0, 0.0),
        patch(true, 0.0, 0.0, 0.3),
    ] {
        let Some((checked, shaded)) = check_shadow(&case, &rows) else { return };
        assert!(checked > 150, "flattening {}: {checked} pixels checked", case.f);
        assert!(
            !case.shadow || shaded > 20,
            "drift {}, flattening {}: {shaded} in the shadow",
            case.rate * case.t,
            case.f
        );
    }
    // a sun 30 degrees north over an oblate body: beside the terminator the geodetic normal still faces the Sun where
    // the radius does not, and there the shadow of a patch sunward of it falls (the eye sees the shell clear above)
    let terminator =
        ShadowCase { lon: (-80.0, -20.0), lat: (-70.0, 70.0), sun_lat: 30.0, shadow: true, rate: 0.0, t: 0.0, f: 0.3 };
    let rows: Vec<u32> = (c - 60..c + 60).step_by(6).collect();
    let Some((checked, shaded)) = check_shadow(&terminator, &rows) else { return };
    assert!(checked > 100 && shaded > 20, "{checked} pixels checked, {shaded} in the shadow");
    // the layer turns and scales with the body: a body of radius 50 at scale 2 is the body of radius 100, and one
    // turned 20 degrees about its pole under a layer drifted 20 degrees east shows the layer where it began
    let map = cloud_band("planet-cloud-patch.png", [0, 0, 0], (-50.0, -30.0), (10.0, 60.0));
    let material = r##"<material id="m" baseColor="#FFFFFFFF" specular="0" roughness="1"/>"##;
    let light = r#"<light id="sun" type="directional" yaw="-90" intensity="3"/>"#;
    let clouds = |rate: f64| format!(r#"cloudMap="{}" cloudHeight="0.2" cloudRotation="{rate}""#, map.display());
    let rows = [c - 50, c - 40, c - 30, c - 20, c - 10, c, c + 20];
    let Some(still) = planet_at(material, &clouds(0.0), light, 0.0) else { return };
    let cases = [
        (r#"radius="50" scaleX="2" scaleY="2" scaleZ="2""#, 0.0, 0.0, "scaled"),
        (r#"rotationY="20""#, 20.0, 1.0, "turned"),
    ];
    for (body, rate, t, case) in cases {
        let Some(r) = planet_at(material, &format!("{} {body}", clouds(rate)), light, t) else { return };
        let mut far = 0;
        for row in rows {
            for x in 0..SIZE {
                far += ((r.at(x, row)[0] - still.at(x, row)[0]).abs() > 0.05) as u32;
            }
        }
        assert!(far < 10, "{case}: {far} pixels differ from the still body's");
    }
}

#[test]
fn the_clouds_shadow_only_the_suns_light() {
    // rule 4 darkens rule 3's sun, the first directional light: a second directional light from another side lights
    // the ground as it would without the shadow, so the frame is the sun's under the shadow plus its own without
    let map = cloud_band("planet-cloud-patch.png", [0, 0, 0], (-50.0, -30.0), (10.0, 60.0));
    let material = r##"<material id="m" baseColor="#FFFFFFFF" specular="0" roughness="1"/>"##;
    let clouds = |shadow: bool| format!(r#"cloudMap="{}" cloudHeight="0.2" cloudShadow="{shadow}""#, map.display());
    let sun = r#"<light id="sun" type="directional" yaw="-90" intensity="1.5"/>"#;
    let side = r#"<light id="side" type="directional" yaw="39" pitch="-17" intensity="1.5"/>"#;
    for camera in ["", r#"renderer="pathtrace" pathSamples="1" maxBounces="1" denoise="false""#] {
        let at = |attrs: &str, lights: &str| {
            let xml = format!(
                r##"<scene version="1.8"><project width="{SIZE}" height="{SIZE}" fps="24" duration="1" background="#000000FF"/><materials>{material}</materials><composition><camera id="eye" x="0" y="0" z="0" fov="60" {camera}/><object3D id="p" primitive="planet" material="m" radius="100" z="400" segments="256" {attrs}/></composition><lights>{lights}</lights></scene>"##
            );
            let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
            let doc = sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"));
            let r = render(&doc)?;
            assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
            Some(r)
        };
        let (Some(both), Some(sun_alone), Some(sun_bare), Some(side_alone)) = (
            at(&clouds(true), &format!("{sun}{side}")),
            at(&clouds(true), sun),
            at(&clouds(false), sun),
            at(&clouds(false), side),
        ) else {
            return;
        };
        let (mut checked, mut apart) = (0, 0);
        for y in (8..SIZE - 8).step_by(4) {
            for x in (8..SIZE - 8).step_by(2) {
                let want = sun_alone.at(x, y)[0] + side_alone.at(x, y)[0];
                if want > 0.9 {
                    continue;
                }
                let got = both.at(x, y)[0];
                assert!((got - want).abs() < 0.02, "{camera:?} ({x}, {y}): {got}, want {want}");
                checked += 1;
                // where the sun's light is shadowed
                apart += (sun_bare.at(x, y)[0] + side_alone.at(x, y)[0] - got > 0.05) as u32;
            }
        }
        assert!(checked > 1000 && apart > 20, "{camera:?}: {checked} pixels checked, {apart} in the sun's shadow");
    }
}

#[test]
fn an_oblate_bodys_cloud_shell_is_its_spheroid_scaled() {
    // white clouds everywhere over a black ground of flattening 0.3, the light from the eye: over the pole the shell
    // ends at c (1 + h), well inside a sphere of a (1 + h)
    let map = cloud_band("planet-cloud-all.png", [255, 255, 255], (-180.0, 180.0), (-90.0, 90.0));
    let material = r##"<material id="m" baseColor="#000000FF" specular="0" roughness="1"/>"##;
    let attrs = format!(r#"cloudMap="{}" cloudHeight="0.2" flattening="0.3""#, map.display());
    let light = r#"<light id="sun" type="directional" intensity="3"/>"#;
    let Some(r) = planet_at(material, &attrs, light, 0.0) else { return };
    let c = SIZE / 2;
    let (mut inside, mut outside) = (0, 0);
    for y in 0..c {
        let on_spheroid = crossing_spheroid(c, y, 120.0, 0.3).is_some_and(|q| q[2] < -60.0);
        let on_sphere = crossing(c, y, 120.0).is_some();
        let v = r.at(c, y)[0];
        if on_spheroid {
            assert!(v > 0.2, "row {y} on the shell: {v}");
            inside += 1;
        } else if on_sphere && crossing_spheroid(c, y + 2, 120.0, 0.3).is_none() {
            assert!(v < 0.01, "row {y} beyond the shell: {v}");
            outside += 1;
        }
    }
    assert!(inside > 10 && outside > 10, "{inside} rows on the shell, {outside} beyond it");
}

/// The draft's four-texel ring strip (sr-core's `STRIP`) in eight bits, inner edge first.
const RING_STRIP: [[u8; 4]; 4] = [[153, 153, 128, 51], [204, 191, 153, 230], [0, 0, 0, 0], [230, 217, 178, 153]];

/// The strip's colours, opaque: the tracer draws a translucent surface by chance, sample by sample.
const OPAQUE_STRIP: [[u8; 4]; 4] = [[153, 153, 128, 255], [204, 191, 153, 255], [0, 0, 0, 255], [230, 217, 178, 255]];

fn ring_strip(texels: &[[u8; 4]; 4]) -> std::path::PathBuf {
    let img = image::RgbaImage::from_fn(4, 1, |x, _| image::Rgba(texels[x as usize]));
    let name = if texels[0][3] == 255 { "planet-ring-opaque.png" } else { "planet-ring-strip.png" };
    write_fixture_png(name, &img)
}

/// `ring_sample` of an eight-bit strip at `rho` over [inner, outer], its colour decoded to linear texel by texel.
fn strip_at(texels: &[[u8; 4]; 4], rho: f64, inner: f64, outer: f64) -> Option<[f64; 4]> {
    let strip = texels.map(|t| {
        let c = |v: u8| srgb_to_linear(v as f32 / 255.0) as f64;
        [c(t[0]), c(t[1]), c(t[2]), t[3] as f64 / 255.0]
    });
    sr_3d::planet::ring_sample(&strip, rho, inner, outer).unwrap()
}

/// The ring frame's tilt: the body turned by rotationX, its pole (the scene's −y) toward the eye's side.
const RING_TILT: f64 = 30.0;

/// World to the body's own scene axes (R_x(tilt) transposed) about the centre 400 down the view axis.
fn to_body(v: [f64; 3]) -> [f64; 3] {
    let (s, c) = RING_TILT.to_radians().sin_cos();
    [v[0], v[1] * c + v[2] * s, -v[1] * s + v[2] * c]
}

fn from_body(v: [f64; 3]) -> [f64; 3] {
    let (s, c) = RING_TILT.to_radians().sin_cos();
    [v[0], v[1] * c - v[2] * s, v[1] * s + v[2] * c]
}

/// What a ray from the eye through (px, py) in pixels meets in the body's scene axes: the ring plane's point and
/// whether the spheroid of radius 100 and `flattening` stands before it.
fn ring_ray(px: f64, py: f64, flattening: f64) -> Option<([f64; 3], bool)> {
    let f = SIZE as f64 / 2.0 / 30f64.to_radians().tan();
    let c = SIZE as f64 / 2.0;
    let d = to_body([(px - c) / f, (py - c) / f, 1.0]);
    let o = to_body([0.0, 0.0, -400.0]);
    let t = -o[1] / d[1];
    if t <= 0.0 {
        return None;
    }
    let q = [o[0] + d[0] * t, 0.0, o[2] + d[2] * t];
    let k = 1.0 / (1.0 - flattening);
    let (os, ds) = ([o[0], o[1] * k, o[2]], [d[0], d[1] * k, d[2]]);
    let a = ds.iter().map(|v| v * v).sum::<f64>();
    let b = os.iter().zip(ds).map(|(o, d)| o * d).sum::<f64>();
    let cc = os.iter().map(|v| v * v).sum::<f64>() - 100.0 * 100.0;
    let disc = b * b - a * cc;
    Some((q, disc >= 0.0 && (-b - disc.sqrt()) / a < t))
}

/// The sun of the ring frames, `elevation` degrees north of the ring plane toward the body's −x: its light's yaw and
/// pitch and the direction toward it in the body's scene axes.
fn ring_sun(elevation: f64) -> (f64, f64, [f64; 3]) {
    let b = elevation.to_radians();
    let s = [-b.cos(), -b.sin(), 0.0];
    let (yaw, pitch) = light_toward(s);
    (yaw, pitch, s)
}

/// The yaw and pitch of a directional light toward the unit `s` of the body's scene axes.
fn light_toward(s: [f64; 3]) -> (f64, f64) {
    let w = from_body(s);
    let d = [-w[0], -w[1], -w[2]];
    (d[0].atan2(d[2]).to_degrees(), (-d[1]).asin().to_degrees())
}

/// `ring_shadowed_by_planet` of a body scene-axes point: the kernel's pole is z, the scene's −y.
fn ring_in_shadow(q: [f64; 3], sun: [f64; 3], flattening: f64) -> bool {
    let (qb, sb) = ([q[0], q[2], -q[1]], [sun[0], sun[2], -sun[1]]);
    sr_3d::planet::ring_shadowed_by_planet(qb, sb, 100.0, flattening).unwrap()
}

/// Each pixel of `r` whose four corners agree on what they meet: the ring's lit pixels are one factor times the
/// strip's linear colour times its alpha (alpha-blended over the black ground and background), those in the planet's
/// shadow or behind the planet black. Returns the factor, the lit, shadowed and hidden counts, and the lit pixels
/// whose ring point the sphere of the equatorial radius would have shadowed.
fn check_ring(
    r: &Rendered,
    texels: &[[u8; 4]; 4],
    flattening: f64,
    (inner, outer): (f64, f64),
    sun: [f64; 3],
) -> (f64, u32, u32, u32, u32) {
    #[derive(PartialEq)]
    enum Meets {
        Lit,
        Shadowed,
        Hidden,
        Other,
    }
    let meets = |px: f64, py: f64| match ring_ray(px, py, flattening) {
        Some((q, hidden)) => {
            let rho = q[0].hypot(q[2]);
            if !(inner..=outer).contains(&rho) {
                Meets::Other
            } else if hidden {
                Meets::Hidden
            } else if ring_in_shadow(q, sun, flattening) {
                Meets::Shadowed
            } else {
                Meets::Lit
            }
        }
        None => Meets::Other,
    };
    let (mut lit, mut ratios) = (Vec::new(), Vec::new());
    let (mut shadowed, mut hidden, mut beyond_sphere) = (0, 0, 0);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (fx, fy) = (x as f64, y as f64);
            // the tracer jitters within the pixel; a pixel counts only when a margin about it meets one thing
            let m = meets(fx + 0.5, fy + 0.5);
            let corners = [(-0.5, -0.5), (1.5, -0.5), (-0.5, 1.5), (1.5, 1.5)];
            if corners.iter().any(|(dx, dy)| meets(fx + dx, fy + dy) != m) {
                continue;
            }
            let got = r.at(x, y);
            match m {
                Meets::Lit => {
                    // the strip's colour times its alpha at the pixel's centre, and its range over the pixel, where
                    // the tracer's one sample may fall
                    let want_at = |px: f64, py: f64| {
                        let (q, _) = ring_ray(px, py, flattening).unwrap();
                        let s = strip_at(texels, q[0].hypot(q[2]), inner, outer).unwrap();
                        [s[0] * s[3], s[1] * s[3], s[2] * s[3]]
                    };
                    let want = want_at(fx + 0.5, fy + 0.5);
                    let mut range = [(f64::MAX, f64::MIN); 3];
                    for (dx, dy) in [(0.5, 0.5), (0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                        for (r, w) in range.iter_mut().zip(want_at(fx + dx, fy + dy)) {
                            *r = (r.0.min(w), r.1.max(w));
                        }
                    }
                    for (g, w) in got.iter().zip(want) {
                        if w > 0.05 {
                            ratios.push(*g as f64 / w);
                        }
                    }
                    let (q, _) = ring_ray(fx + 0.5, fy + 0.5, flattening).unwrap();
                    beyond_sphere += ring_in_shadow(q, sun, 0.0) as u32;
                    lit.push((x, y, got, range));
                }
                Meets::Shadowed => {
                    assert!(got[0].max(got[1]).max(got[2]) < 0.01, "({x}, {y}) in the planet's shadow: {got:?}");
                    shadowed += 1;
                }
                Meets::Hidden => {
                    assert!(got[0].max(got[1]).max(got[2]) < 0.01, "({x}, {y}) behind the planet: {got:?}");
                    hidden += 1;
                }
                Meets::Other => {}
            }
        }
    }
    ratios.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let k = ratios.get(ratios.len() / 2).copied().unwrap_or(0.0);
    assert!(k > 0.1, "the ring's light factor {k}");
    for (x, y, got, range) in &lit {
        for (g, (lo, hi)) in got.iter().zip(range) {
            let (lo, hi, g) = (k * lo, k * hi, *g as f64);
            assert!(g > lo - 0.01 - 0.03 * lo && g < hi + 0.01 + 0.03 * hi, "({x}, {y}): {got:?} want {lo} to {hi}");
        }
    }
    (k, lit.len() as u32, shadowed, hidden, beyond_sphere)
}

#[test]
fn the_rings_show_the_strip_across_their_radius_and_the_spheroids_shadow() {
    let material = r##"<material id="m" baseColor="#000000FF" specular="0" roughness="1"/>"##;
    // the raster pass blends the translucent strip, the tracer draws the opaque one
    for (camera, texels) in [("", &RING_STRIP), (TRACED, &OPAQUE_STRIP)] {
        let strip = ring_strip(texels);
        // the eye 30 degrees north of the plane sees the lit face under a northern sun and the unlit one under a
        // southern sun, both lit alike
        let mut factors = Vec::new();
        for (f, elevation) in [(0.0, 26.7), (0.3, 26.7), (0.3, -26.7)] {
            let (yaw, pitch, sun) = ring_sun(elevation);
            let light = format!(r#"<light id="sun" type="directional" yaw="{yaw}" pitch="{pitch}" intensity="3"/>"#);
            let attrs = format!(
                r#"rotationX="{RING_TILT}" flattening="{f}" ringMap="{}" ringInner="1.3" ringOuter="2.2""#,
                strip.display()
            );
            let Some(r) = planet_seen(camera, material, &attrs, &light, 0.0) else { return };
            let (k, lit, shadowed, hidden, beyond) = check_ring(&r, texels, f, (130.0, 220.0), sun);
            let case = format!("{camera:?} flattening {f} sun {elevation}");
            eprintln!("{case}: k {k}, {lit} lit, {shadowed} shadowed, {hidden} hidden, {beyond} beyond");
            assert!(
                lit > 1500 && shadowed > 100 && hidden > 100,
                "{case}: {lit} lit, {shadowed} shadowed, {hidden} hidden"
            );
            // the spheroid's shadow ends short of the sphere's: there the ring is lit
            assert!(
                f == 0.0 || elevation < 0.0 || beyond > 100,
                "{case}: {beyond} lit where a sphere's shadow would fall"
            );
            factors.push(k);
            // a diffuse plane with no specular reflection lit by |n · s|: intensity × sin β / π
            let lambert = 3.0 * elevation.abs().to_radians().sin() / std::f64::consts::PI;
            assert!((k / lambert - 1.0).abs() < 0.005, "{case}: factor {k} against {lambert}");
        }
        assert!((factors[2] / factors[1] - 1.0).abs() < 0.005, "{camera:?}: the faces' factors {factors:?}");
    }
}

/// The ground point a ray from the eye through (px, py) meets on the tilted spheroid of radius 100 and `flattening`,
/// in the body's scene axes, and whether the annulus over [inner, outer] stands before it.
fn ground_ray(px: f64, py: f64, flattening: f64, inner: f64, outer: f64) -> Option<([f64; 3], bool)> {
    let f = SIZE as f64 / 2.0 / 30f64.to_radians().tan();
    let c = SIZE as f64 / 2.0;
    let d = to_body([(px - c) / f, (py - c) / f, 1.0]);
    let o = to_body([0.0, 0.0, -400.0]);
    let k = 1.0 / (1.0 - flattening);
    let (os, ds) = ([o[0], o[1] * k, o[2]], [d[0], d[1] * k, d[2]]);
    let a = ds.iter().map(|v| v * v).sum::<f64>();
    let b = os.iter().zip(ds).map(|(o, d)| o * d).sum::<f64>();
    let disc = b * b - a * (os.iter().map(|v| v * v).sum::<f64>() - 100.0 * 100.0);
    if disc < 0.0 {
        return None;
    }
    let t = (-b - disc.sqrt()) / a;
    let p = [o[0] + d[0] * t, o[1] + d[1] * t, o[2] + d[2] * t];
    let tp = -o[1] / d[1];
    let rho = (o[0] + d[0] * tp).hypot(o[2] + d[2] * tp);
    Some((p, tp > 0.0 && tp < t && (inner..=outer).contains(&rho)))
}

/// `ring_shadow` of a body scene-axes ground point under the eight-bit strip: the kernel's pole is z.
fn ring_light(p: [f64; 3], sun: [f64; 3], inner: f64, outer: f64) -> f64 {
    let strip = RING_STRIP.map(|t| t.map(|v| v as f64 / 255.0));
    let (pb, sb) = ([p[0], p[2], -p[1]], [sun[0], sun[2], -sun[1]]);
    sr_3d::planet::ring_shadow(pb, sb, &strip, inner, outer).unwrap()
}

#[test]
fn the_rings_shadow_on_the_ground_dims_the_suns_light_alone() {
    // a sun 26.7 degrees south of the plane casts the ring's shadow on the northern hemisphere the eye sees; one as
    // far north casts none there, though the line away from it crosses the ring; a second light from the eye's side
    // is not shadowed by the ring
    let strip = ring_strip(&RING_STRIP);
    let material = r##"<material id="m" baseColor="#FFFFFFFF" specular="0" roughness="1"/>"##;
    let side = {
        let s: [f64; 3] = [0.4, -0.5, -0.77];
        let n = (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]).sqrt();
        light_toward([s[0] / n, s[1] / n, s[2] / n])
    };
    let side_light =
        format!(r#"<light id="side" type="directional" yaw="{}" pitch="{}" intensity="0.5"/>"#, side.0, side.1);
    let cases = [
        ("", 0.0, -26.7),
        ("", 0.3, -26.7),
        ("", 0.3, 26.7),
        (TRACED, 0.0, -26.7),
        (TRACED, 0.3, -26.7),
        (TRACED, 0.3, 26.7),
    ];
    for (camera, f, elevation) in cases {
        let (yaw, pitch, sun) = ring_sun(elevation);
        let sun_light = format!(r#"<light id="sun" type="directional" yaw="{yaw}" pitch="{pitch}" intensity="2"/>"#);
        let away = [-sun[0], -sun[1], -sun[2]];
        let body = format!(r#"rotationX="{RING_TILT}" flattening="{f}""#);
        let rings = format!(r#"{body} ringMap="{}" ringInner="1.3" ringOuter="2.2""#, strip.display());
        let Some(sun_alone) = planet_seen(camera, material, &body, &sun_light, 0.0) else { return };
        let Some(side_alone) = planet_seen(camera, material, &body, &side_light, 0.0) else { return };
        let Some(r) = planet_seen(camera, material, &rings, &format!("{sun_light}{side_light}"), 0.0) else { return };
        let (mut checked, mut shaded, mut behind) = (0, 0, 0);
        for y in 0..SIZE {
            for x in 0..SIZE {
                let (fx, fy) = (x as f64, y as f64);
                let Some((p, covered)) = ground_ray(fx + 0.5, fy + 0.5, f, 130.0, 220.0) else { continue };
                let light = ring_light(p, sun, 130.0, 220.0);
                // the tracer jitters within the pixel: the corners meet the ground clear of the ring and of the
                // strip's edges
                let corners = [(-0.5, -0.5), (1.5, -0.5), (-0.5, 1.5), (1.5, 1.5)];
                let steady = corners.iter().all(|(dx, dy)| {
                    ground_ray(fx + dx, fy + dy, f, 130.0, 220.0)
                        .is_some_and(|(q, c)| !c && (ring_light(q, sun, 130.0, 220.0) - light).abs() < 0.1)
                });
                if covered || !steady {
                    continue;
                }
                // the light through the ring over the pixel, where the tracer's one sample may fall
                let (mut lo, mut hi) = (light, light);
                for (dx, dy) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                    let (q, _) = ground_ray(fx + dx, fy + dy, f, 130.0, 220.0).unwrap();
                    let l = ring_light(q, sun, 130.0, 220.0);
                    (lo, hi) = (lo.min(l), hi.max(l));
                }
                let (a, b) = (sun_alone.at(x, y), side_alone.at(x, y));
                let got = r.at(x, y);
                for c in 0..3 {
                    let (want_lo, want_hi) = (a[c] as f64 * lo + b[c] as f64, a[c] as f64 * hi + b[c] as f64);
                    let g = got[c] as f64;
                    assert!(
                        g > want_lo - 0.01 - 0.03 * want_lo && g < want_hi + 0.01 + 0.03 * want_hi,
                        "{camera:?} flattening {f}: ({x}, {y}) channel {c}: {g} want {want_lo} to {want_hi}"
                    );
                }
                checked += 1;
                shaded += (light < 0.9 && a[0] > 0.05) as u32;
                // lit ground whose line away from the Sun crosses the ring's opaque part
                behind += (ring_light(p, away, 130.0, 220.0) < 0.9 && a[0] > 0.05) as u32;
            }
        }
        let case = format!("{camera:?} flattening {f} sun {elevation}");
        eprintln!("{case}: {checked} checked, {shaded} in the ring's shadow, {behind} with the ring behind");
        assert!(checked > 5000, "{case}: {checked} checked");
        assert!(
            if elevation < 0.0 { shaded > 300 } else { shaded == 0 && behind > 300 },
            "{case}: {shaded} shaded, {behind} behind"
        );
    }
}

#[test]
fn the_rings_scale_with_the_body_and_default_to_saturns() {
    // a body of radius 50 at scale 2 is the body of radius 100, its ring and both shadows included; without radii the
    // ring spans Saturn's 1.239 to 2.269 equatorial radii
    let strip = ring_strip(&RING_STRIP);
    let (white, black) = (
        r##"<material id="m" baseColor="#FFFFFFFF" specular="0" roughness="1"/>"##,
        r##"<material id="m" baseColor="#000000FF" specular="0" roughness="1"/>"##,
    );
    for (material, elevation) in [(black, 26.7), (white, -26.7)] {
        let (yaw, pitch, _) = ring_sun(elevation);
        let light = format!(r#"<light id="sun" type="directional" yaw="{yaw}" pitch="{pitch}" intensity="3"/>"#);
        let ring = format!(r#"rotationX="{RING_TILT}" flattening="0.3" ringMap="{}""#, strip.display());
        let cases = [
            (
                format!(r#"{ring} ringInner="1.3" ringOuter="2.2""#),
                format!(r#"{ring} ringInner="1.3" ringOuter="2.2" radius="50" scaleX="2" scaleY="2" scaleZ="2""#),
            ),
            (format!(r#"{ring} ringInner="1.239" ringOuter="2.269""#), ring.clone()),
        ];
        for (a, b) in cases {
            let (Some(ra), Some(rb)) = (planet_at(material, &a, &light, 0.0), planet_at(material, &b, &light, 0.0))
            else {
                return;
            };
            let far = ra.px.iter().zip(&rb.px).filter(|(p, q)| (0..3).any(|c| (p[c] - q[c]).abs() > 0.02)).count();
            let ring_pixels = ra.px.iter().zip(&rb.px).filter(|(p, q)| p[0].max(q[0]) > 0.02).count();
            assert!(far < 20 && ring_pixels > 2000, "{b}: {far} of {ring_pixels} pixels differ from {a}");
        }
    }
}

#[test]
fn the_raster_pass_names_the_rings_shadows_it_does_not_draw() {
    // under a physical sky's sun, which lights the scene through the sky's bake, neither ring shadow is drawn
    let attrs = format!(r#"rotationX="{RING_TILT}" ringMap="{}""#, ring_strip(&RING_STRIP).display());
    let material = r##"<material id="m" baseColor="#FFFFFFFF" specular="0" roughness="1"/>"##;
    let note = "p: the rings' shadows under a physical sky's sun are not drawn";
    let sky = r#"<light id="sky" type="dome" sky="physical" sunIntensity="1" date="2024-06-21T12:00:00Z" latitude="0" moon="false"/>"#;
    let Some(r) = planet_at(material, &attrs, sky, 0.0) else { return };
    assert!(r.stats.unsupported.iter().any(|u| u == note), "{:?}", r.stats.unsupported);
    let light = r#"<light id="sun" type="directional" yaw="-90" intensity="3"/>"#;
    let Some(r) = planet_at(material, &attrs, light, 0.0) else { return };
    assert!(!r.stats.unsupported.iter().any(|u| u == note), "{:?}", r.stats.unsupported);
}

/// The draft's Lambert material (`specular="0"`): with no specular reflection the diffuse keeps all the light, not the
/// share 1 − F(v·h) a reflection would leave it. The sun is 135° from the line of sight, where F of a dielectric at
/// 1.5 runs from 0.08 to 0.2 across the crescent. Each pixel's radiance over n·s is regressed on the 1 − F it would
/// carry: a slope of 1 relative to their means is the Fresnel loss, 0 a Lambert surface. The raster pass is exact
/// enough to hold to one factor; the tracer's 64 jittered points a pixel leave it about 1 % of noise.
#[test]
fn a_material_without_specular_reflection_shades_as_lambert() {
    let s = [0.5f64.sqrt(), 0.0, 0.5f64.sqrt()];
    let (yaw, pitch) = ((-s[0]).atan2(-s[2]).to_degrees(), s[1].asin().to_degrees());
    let material = r##"<material id="m" baseColor="#FFFFFFFF" metallic="0" roughness="1" specular="0"/>"##;
    let lights = format!(r#"<light id="sun" type="directional" intensity="1" yaw="{yaw}" pitch="{pitch}"/>"#);
    let dot = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
    let ns = |px: f64, py: f64| surface_through(px, py).map(|(n, _)| dot(n, s));
    let averaged = r#"renderer="pathtrace" pathSamples="64" maxBounces="1" denoise="false""#;
    let mut means = Vec::new();
    for camera in ["", averaged] {
        let Some(r) = planet_seen(camera, material, "", &lights, 0.0) else { return };
        let mut samples = Vec::new();
        for y in 0..SIZE {
            for x in 0..SIZE {
                let (px, py) = (x as f64, y as f64);
                let corners = [(px, py), (px + 1.0, py), (px, py + 1.0), (px + 1.0, py + 1.0)];
                if !corners.iter().all(|&(a, b)| ns(a, b).is_some_and(|v| v >= 0.2)) {
                    continue;
                }
                let (n, v) = surface_through(px + 0.5, py + 0.5).unwrap();
                let h = [v[0] + s[0], v[1] + s[1], v[2] + s[2]];
                let vh = dot(v, h) / dot(h, h).sqrt();
                let kept = 1.0 - (0.04 + 0.96 * (1.0 - vh).powi(5));
                samples.push((kept, r.at(x, y)[1] as f64 / dot(n, s)));
            }
        }
        let m = samples.len() as f64;
        assert!(m > 80.0, "{camera:?}: {m} lit pixels");
        let (gm, km) = samples.iter().fold((0.0, 0.0), |(a, b), (g, k)| (a + g / m, b + k / m));
        let (cov, var) =
            samples.iter().fold((0.0, 0.0), |(c, v), (g, k)| (c + (g - gm) * (k - km), v + (g - gm).powi(2)));
        let slope = cov / var * gm / km;
        assert!(slope.abs() < 0.35, "{camera:?}: the factor over n·s follows 1 − F with slope {slope}");
        if camera.is_empty() {
            let (lo, hi) = samples.iter().fold((f64::INFINITY, 0.0f64), |(a, b), (_, k)| (a.min(*k), b.max(*k)));
            assert!(hi / lo < 1.01, "the factor over n·s runs from {lo} to {hi}");
        }
        means.push(km);
    }
    assert!(
        (means[1] / means[0] - 1.0).abs() < 0.01,
        "the tracer's factor {} against the raster pass's {}",
        means[1],
        means[0]
    );
}

/// [`a_material_without_specular_reflection_shades_as_lambert`] under a uniform dome, which the raster pass lights
/// by its image: a Lambert sphere is as bright at its limb as at its centre.
#[test]
fn a_material_without_specular_reflection_takes_the_whole_dome() {
    let material = r##"<material id="m" baseColor="#FFFFFFFF" metallic="0" roughness="1" specular="0"/>"##;
    let dome = r##"<light id="sky" type="dome" sky="gradient" skyZenith="#808080" skyHorizon="#808080" skyGround="#808080"/>"##;
    let Some(r) = planet_seen("", material, "", dome, 0.0) else { return };
    let mut seen = Vec::new();
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (px, py) = (x as f64, y as f64);
            let inside = [(px, py), (px + 1.0, py), (px, py + 1.0), (px + 1.0, py + 1.0)].iter().all(|&(a, b)| {
                surface_through(a, b).is_some_and(|(n, v)| n.iter().zip(v).map(|(a, b)| a * b).sum::<f64>() > 0.15)
            });
            if inside {
                seen.push(r.at(x, y)[1] as f64);
            }
        }
    }
    let (lo, hi) = seen.iter().fold((f64::INFINITY, 0.0f64), |(a, b), v| (a.min(*v), b.max(*v)));
    assert!(seen.len() > 5000 && lo > 0.0, "{} pixels, darkest {lo}", seen.len());
    assert!(hi / lo < 1.01, "the disc runs from {lo} to {hi}");
}
