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
    // under a physical sky's sun the shadow darkens the bake's diffuse light, and its specular light on a ground that
    // reflects is named as not drawn
    let sky = r#"<light id="sky" type="dome" sky="physical" sunIntensity="1" date="2024-06-21T12:00:00Z" latitude="0" moon="false"/>"#;
    let note = "p: the clouds' and rings' shadows of a physical sky's sun on the ground's specular light are not drawn by the raster pass";
    // the path tracer's rays toward the sun meet the shell, which shadows the ground whatever `cloudShadow` says
    let traced = "p: cloudShadow=\"false\" under a physical sky's sun is not drawn by the path tracer, whose rays toward the sun meet the cloud shell";
    let lambert = r##"<material id="m" baseColor="#FFFFFFFF" specular="0" roughness="1"/>"##;
    let glossy = r##"<material id="m" baseColor="#FFFFFFFF" specular="0.5" roughness="0.4"/>"##;
    let (raster, tracer) = ("", r#"renderer="pathtrace" pathSamples="1""#);
    let off = r#" cloudShadow="false""#;
    for (camera, material, more, light, named) in [
        (raster, lambert, "", sky, None),
        (raster, glossy, "", sky, Some(note)),
        (raster, glossy, off, sky, None),
        (raster, glossy, "", light, None),
        (tracer, glossy, "", sky, None),
        (tracer, lambert, off, sky, Some(traced)),
        (tracer, lambert, off, light, None),
    ] {
        let Some(r) = planet_seen(camera, material, &format!("{attrs}{more}"), light, 0.0) else { return };
        let u = &r.stats.unsupported;
        let named_ones: Vec<_> = u.iter().filter(|m| m.contains("physical sky")).collect();
        assert_eq!(named_ones, named.iter().collect::<Vec<_>>(), "{camera} {material}{more}: {u:?}");
    }
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

/// A physical sky at 40° N on the March equinox at noon: the sun 50° up in the south, the scene's −z, toward the eye.
const NOON_40N: (&str, f64) = ("2024-03-20T12:00:00Z", 40.0);

/// A physical sky at `latitude` on the meridian of Greenwich at `date`, unturned, with no air between the eye and the
/// planet, its sun `sun_size` degrees across.
fn physical_sky((date, latitude): (&str, f64), sun_size: &str) -> String {
    format!(
        r#"<light id="sky" type="dome" sky="physical" sunIntensity="1" date="{date}" latitude="{latitude}" longitude="0" moon="false" aerialPerspective="false" sunSize="{sun_size}"/>"#
    )
}

/// That sky's sun in the scene's axes (the dome's, unturned).
fn physical_sun((date, latitude): (&str, f64)) -> [f64; 3] {
    let o = sr_3d::space::sun::Observer {
        latitude,
        longitude: 0.0,
        altitude: 0.0,
        pressure: 1013.25,
        temperature: 10.0,
        delta_t: None,
    };
    sr_3d::space::sun::sun_from_document(date, &o).unwrap().1
}

/// Whether `got` is `want` to a share `tol` of the sun's light `sun` and a hundredth of `want`, channel by channel.
fn within(got: [f32; 4], want: [f32; 4], sun: [f32; 4], tol: f64) -> bool {
    (0..3).all(|c| ((got[c] - want[c]) as f64).abs() <= tol * (sun[c] as f64).abs() + 0.01 * (want[c] as f64).abs())
}

#[test]
fn the_clouds_shadow_takes_a_physical_skys_sun_from_the_ground() {
    // the patch north of the point the eye sees the sun's ray leave: the ground under it keeps the sky's light alone,
    // the frame of the same sky with no disc (sunSize 0), and the ground clear of it the whole sun's
    let (h, (lo, hi), (south, north)) = (0.2, (-110.0, -70.0), (25.0, 70.0));
    let map = cloud_band("planet-cloud-north.png", [0, 0, 0], (lo, hi), (south, north));
    let material = r##"<material id="m" baseColor="#FFFFFFFF" specular="0" roughness="1"/>"##;
    let clouds = |shadow: bool| format!(r#"cloudMap="{}" cloudHeight="{h}" cloudShadow="{shadow}""#, map.display());
    let (Some(r), Some(bare), Some(sky)) = (
        planet_at(material, &clouds(true), &physical_sky(NOON_40N, "0.53"), 0.0),
        planet_at(material, &clouds(false), &physical_sky(NOON_40N, "0.53"), 0.0),
        planet_at(material, &clouds(true), &physical_sky(NOON_40N, "0"), 0.0),
    ) else {
        return;
    };
    assert!(r.stats.unsupported.iter().all(|u| !u.contains("shadow")), "{:?}", r.stats.unsupported);
    let sun = physical_sun(NOON_40N);
    let alpha = |q: [f64; 3]| {
        let (lon, lat) = (longitude(q), geodetic_latitude(q, 0.0));
        let near = |v: f64, e: f64| (v - e).abs() < 1.5;
        (!(near(lon, lo) || near(lon, hi) || near(lat, south) || near(lat, north)))
            .then_some(if (lo..hi).contains(&lon) && (south..north).contains(&lat) { 1.0 } else { 0.0 })
    };
    let (mut shaded, mut clear) = (0, 0);
    for y in (0..SIZE).step_by(2) {
        for x in (0..SIZE).step_by(2) {
            let (Some(p), Some(qv)) = (crossing(x, y, 100.0), crossing(x, y, 100.0 * (1.0 + h))) else { continue };
            // seen through the clear layer, on the day side away from the limb
            if alpha(qv) != Some(0.0) || p[2] > -30.0 {
                continue;
            }
            let through = match shadow_point(p, sun, 100.0, h, 0.0) {
                Some(q) => match alpha(q) {
                    Some(a) => 1.0 - a,
                    None => continue,
                },
                None => 1.0,
            };
            let (got, full, alone) = (r.at(x, y), bare.at(x, y), sky.at(x, y));
            let share = [full[0] - alone[0], full[1] - alone[1], full[2] - alone[2], 0.0];
            if share[0] < 0.2 * full[0] {
                continue;
            }
            let want = if through == 0.0 { alone } else { full };
            assert!(within(got, want, share, 0.02), "({x}, {y}) through {through}: {got:?}, want {want:?}");
            shaded += (through == 0.0) as u32;
            clear += (through == 1.0) as u32;
        }
    }
    assert!(shaded > 100 && clear > 100, "{shaded} in the shadow, {clear} clear");
    // off, the shadow leaves the bake whole
    let far = r.px.iter().zip(&bare.px).filter(|(p, q)| (0..3).any(|c| (p[c] - q[c]).abs() > 1e-6)).count();
    assert!(far > 100, "{far} pixels differ from the frame without the shadow");
}

/// A physical sky over the equator in the afternoon of 23 February: the sun 23° up in the west-southwest, about 27°
/// north of the ring frames' plane, toward the body's −x.
const AFTERNOON: (&str, f64) = ("2024-02-23T16:40:00Z", 0.0);

/// The ring frames' planet (`RING_TILT`, radius 100, the ring from 1.3 to 2.2 radii, a white ground) under a physical
/// sky `when` with its disc and without, and without the ring under both.
fn ringed_under(tilt: f64, when: (&str, f64)) -> Option<[Rendered; 4]> {
    let strip = ring_strip(&RING_STRIP);
    let material = r##"<material id="m" baseColor="#FFFFFFFF" specular="0" roughness="1"/>"##;
    let body = format!(r#"rotationX="{tilt}""#);
    let ring = format!(r#"{body} ringMap="{}" ringInner="1.3" ringOuter="2.2""#, strip.display());
    let (sunlit, alone) = (physical_sky(when, "0.53"), physical_sky(when, "0"));
    let frames = [
        planet_at(material, &ring, &sunlit, 0.0)?,
        planet_at(material, &ring, &alone, 0.0)?,
        planet_at(material, &body, &sunlit, 0.0)?,
        planet_at(material, &body, &alone, 0.0)?,
    ];
    assert!(frames[0].stats.unsupported.iter().all(|u| !u.contains("shadow")), "{:?}", frames[0].stats.unsupported);
    Some(frames)
}

fn share(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2], 0.0]
}

#[test]
fn the_rings_shadows_take_a_physical_skys_sun() {
    let (inner, outer) = (130.0, 220.0);
    // the ring in the planet's shadow keeps the sky's light alone, the frame of the same sky with no disc: its sun's
    // share over the strip's premultiplied colour is one factor where it is lit and none in the shadow
    let Some([r, sky, _, _]) = ringed_under(RING_TILT, AFTERNOON) else { return };
    let sun = to_body(physical_sun(AFTERNOON));
    let (mut lit, mut dark) = (Vec::new(), Vec::new());
    for y in 0..SIZE {
        for x in 0..SIZE {
            let meets = |px: f64, py: f64| {
                ring_ray(px, py, 0.0).and_then(|(q, hidden)| {
                    let rho = q[0].hypot(q[2]);
                    ((inner + 2.0..=outer - 2.0).contains(&rho) && !hidden).then(|| ring_in_shadow(q, sun, 0.0))
                })
            };
            let (fx, fy) = (x as f64, y as f64);
            let m = meets(fx + 0.5, fy + 0.5);
            let corners = [(-0.5, -0.5), (1.5, -0.5), (-0.5, 1.5), (1.5, 1.5)];
            if m.is_none() || corners.iter().any(|(dx, dy)| meets(fx + dx, fy + dy) != m) {
                continue;
            }
            let (q, _) = ring_ray(fx + 0.5, fy + 0.5, 0.0).unwrap();
            let s = strip_at(&RING_STRIP, q[0].hypot(q[2]), inner, outer).unwrap();
            let premultiplied = s[0] * s[3];
            if premultiplied < 0.05 {
                continue;
            }
            let d = share(r.at(x, y), sky.at(x, y))[0] as f64 / premultiplied;
            if m == Some(true) {
                dark.push((x, y, d))
            } else {
                lit.push(d)
            }
        }
    }
    lit.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let k = lit.get(lit.len() / 2).copied().unwrap_or(0.0);
    assert!(lit.len() > 500 && dark.len() > 100 && k > 0.05, "{} lit, {} shadowed, factor {k}", lit.len(), dark.len());
    for (x, y, d) in &dark {
        assert!(d.abs() < 0.02 * k, "ring ({x}, {y}) in the planet's shadow keeps {d} of the sun's {k}");
    }
    // the ground the eye sees bare under the ring's shadow: 1 − alpha of the sun's share, the frame without the ring
    // less that share. The body is tilted the other way, so that the noon sun, toward the eye, casts the shadow on the
    // ground the eye sees: the frame is the ring frames' mirrored through the world's y, its sun's y negated, row y
    // read at SIZE − 1 − y
    let Some([r, _, bare, bare_sky]) = ringed_under(-RING_TILT, NOON_40N) else { return };
    let [sx, sy, sz] = physical_sun(NOON_40N);
    let sun = to_body([sx, -sy, sz]);
    let toward_eye = to_body([0.0, 0.0, -1.0]);
    let (mut shaded, mut clear) = (0, 0);
    for y in (0..SIZE).step_by(2) {
        for x in (0..SIZE).step_by(2) {
            let Some((p, behind)) = ground_ray(x as f64 + 0.5, y as f64 + 0.5, 0.0, inner, outer) else { continue };
            // clear of the limb, where a pixel straddles the ground and what lies beyond
            if behind || toward_eye.iter().zip(p).map(|(a, b)| a * b).sum::<f64>() < 30.0 {
                continue;
            }
            let through = ring_light(p, sun, inner, outer);
            let at = |f: &Rendered| f.at(x, SIZE - 1 - y);
            let (full, alone) = (at(&bare), at(&bare_sky));
            let sun_share = share(full, alone);
            if sun_share[0] < 0.2 * full[0] {
                continue;
            }
            let want = [0, 1, 2, 3].map(|c| alone[c] + through as f32 * sun_share[c]);
            let got = at(&r);
            assert!(within(got, want, sun_share, 0.02), "ground ({x}, {y}) through {through}: {got:?}, want {want:?}");
            shaded += (through < 0.5) as u32;
            clear += (through == 1.0) as u32;
        }
    }
    assert!(shaded > 50 && clear > 100, "{shaded} in the ring's shadow, {clear} clear");
}

#[test]
fn the_path_tracer_casts_the_clouds_shadow_of_a_physical_skys_sun() {
    // the raster frame's patch, traced under a sun 10° wide that the tracer's rays meet in the sky's image, and stop
    // at in the shell: the ground under the shadow keeps the sky's light alone, the frame of the same sky with no disc
    // that traces the same rays, and the ground clear of it the sun's share of the frame without clouds
    let cam = r#"renderer="pathtrace" pathSamples="64" maxBounces="1" denoise="false""#;
    let (h, (lo, hi), (south, north)) = (0.2, (-110.0, -70.0), (25.0, 70.0));
    let map = cloud_band("planet-cloud-north.png", [0, 0, 0], (lo, hi), (south, north));
    let material = r##"<material id="m" baseColor="#FFFFFFFF" specular="0" roughness="1"/>"##;
    let clouds = format!(r#"cloudMap="{}" cloudHeight="{h}""#, map.display());
    let traced = |attrs: &str, size: &str| planet_seen(cam, material, attrs, &physical_sky(NOON_40N, size), 0.0);
    let (Some(r), Some(sky), Some(bare), Some(bare_sky)) =
        (traced(&clouds, "10"), traced(&clouds, "0"), traced("", "10"), traced("", "0"))
    else {
        return;
    };
    assert!(r.stats.unsupported.iter().all(|u| !u.contains("shadow")), "{:?}", r.stats.unsupported);
    let sun = physical_sun(NOON_40N);
    let alpha = |q: [f64; 3]| {
        let (lon, lat) = (longitude(q), geodetic_latitude(q, 0.0));
        let near = |v: f64, e: f64| (v - e).abs() < 3.0;
        (!(near(lon, lo) || near(lon, hi) || near(lat, south) || near(lat, north)))
            .then_some(if (lo..hi).contains(&lon) && (south..north).contains(&lat) { 1.0 } else { 0.0 })
    };
    let (mut shaded, mut clear) = (0, 0);
    let (mut got_share, mut want_share, mut shaded_lit) = (0.0, 0.0, 0.0);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (Some(p), Some(qv)) = (crossing(x, y, 100.0), crossing(x, y, 100.0 * (1.0 + h))) else { continue };
            if alpha(qv) != Some(0.0) || p[2] > -30.0 {
                continue;
            }
            let through = match shadow_point(p, sun, 100.0, h, 0.0) {
                Some(q) => match alpha(q) {
                    Some(a) => 1.0 - a,
                    None => continue,
                },
                None => 1.0,
            };
            let (got, alone) = (r.at(x, y), sky.at(x, y));
            if through == 0.0 {
                assert!((got[0] - alone[0]).abs() <= 0.01 * alone[0], "({x}, {y}): {got:?}, the sky's {alone:?}");
                shaded += 1;
                shaded_lit += (bare.at(x, y)[0] - bare_sky.at(x, y)[0]) as f64;
            } else if through == 1.0 {
                got_share += (got[0] - alone[0]) as f64;
                want_share += (bare.at(x, y)[0] - bare_sky.at(x, y)[0]) as f64;
                clear += 1;
            }
        }
    }
    assert!(shaded > 100 && clear > 100, "{shaded} in the shadow, {clear} clear");
    // without the clouds the sun reaches the shaded ground as it does the clear
    let (shaded_lit, clear_lit) = (shaded_lit / shaded as f64, want_share / clear as f64);
    assert!(
        shaded_lit > 0.5 * clear_lit,
        "the shaded ground's sun {shaded_lit} without the clouds, the clear's {clear_lit}"
    );
    let ratio = got_share / want_share;
    assert!(
        want_share > 10.0 && (ratio - 1.0).abs() < 0.1,
        "the clear ground's sun {got_share}, without clouds {want_share}"
    );
}

#[test]
fn the_path_tracer_casts_the_rings_shadows_of_a_physical_skys_sun() {
    // the ring frames traced under a sun 3° wide, which the tracer's rays meet in the sky's image and stop at in the
    // body and, by the strip's alpha, in the ring; the sun's share is each frame less the frame of the same sky with
    // no disc. Its penumbra spreads the shadows' edges, so regions are compared, not pixels
    let cam = r#"renderer="pathtrace" pathSamples="64" maxBounces="1" denoise="false""#;
    let material = r##"<material id="m" baseColor="#FFFFFFFF" specular="0" roughness="1"/>"##;
    let strip = ring_strip(&RING_STRIP);
    let (inner, outer) = (130.0, 220.0);
    let traced = |tilt: f64, ring: bool, size: &str, when| {
        let body = format!(r#"rotationX="{tilt}""#);
        let attrs = if ring {
            format!(r#"{body} ringMap="{}" ringInner="1.3" ringOuter="2.2""#, strip.display())
        } else {
            body
        };
        planet_seen(cam, material, &attrs, &physical_sky(when, size), 0.0)
    };
    let share = |a: &Rendered, b: &Rendered, x: u32, y: u32| (a.at(x, y)[0] - b.at(x, y)[0]) as f64;
    // the ring in the body's shadow keeps a small part of the sun's share where it is lit
    let (Some(r), Some(sky)) = (traced(RING_TILT, true, "3", AFTERNOON), traced(RING_TILT, true, "0", AFTERNOON))
    else {
        return;
    };
    let sun = to_body(physical_sun(AFTERNOON));
    let (mut lit, mut dark) = ((0.0, 0), (0.0, 0));
    for y in 0..SIZE {
        for x in 0..SIZE {
            let Some((q, hidden)) = ring_ray(x as f64 + 0.5, y as f64 + 0.5, 0.0) else { continue };
            if hidden || !(inner + 2.0..=outer - 2.0).contains(&q[0].hypot(q[2])) {
                continue;
            }
            let side = if ring_in_shadow(q, sun, 0.0) { &mut dark } else { &mut lit };
            *side = (side.0 + share(&r, &sky, x, y), side.1 + 1);
        }
    }
    let (lit_mean, dark_mean) = (lit.0 / lit.1 as f64, dark.0 / dark.1 as f64);
    assert!(lit.1 > 500 && dark.1 > 100 && lit_mean > 0.005, "{lit:?} lit, {dark:?} in the shadow");
    assert!(dark_mean < 0.1 * lit_mean, "the ring in the body's shadow keeps {dark_mean} of the lit ring's {lit_mean}");
    // the ground the eye sees bare (the mirrored noon frame of the raster pass's test): the sun's share, 1 − alpha of
    // the strip where the ray toward it crosses the ring of the frame without the ring's
    let frames = (
        traced(-RING_TILT, true, "3", NOON_40N),
        traced(-RING_TILT, true, "0", NOON_40N),
        traced(-RING_TILT, false, "3", NOON_40N),
        traced(-RING_TILT, false, "0", NOON_40N),
    );
    let (Some(r), Some(sky), Some(bare), Some(bare_sky)) = frames else { return };
    let [sx, sy, sz] = physical_sun(NOON_40N);
    let sun = to_body([sx, -sy, sz]);
    let (mut got, mut want, mut whole) = ([0.0; 2], [0.0; 2], [0.0; 2]);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let Some((p, behind)) = ground_ray(x as f64 + 0.5, y as f64 + 0.5, 0.0, inner, outer) else { continue };
            if behind {
                continue;
            }
            let through = ring_light(p, sun, inner, outer);
            let y = SIZE - 1 - y;
            let (g, alone) = (share(&r, &sky, x, y), share(&bare, &bare_sky, x, y));
            for k in 0..=usize::from(through < 0.5) {
                got[k] += g;
                want[k] += through * alone;
                whole[k] += alone;
            }
        }
    }
    assert!((got[0] / want[0] - 1.0).abs() < 0.05, "the ground's sun {}, the strip's share {}", got[0], want[0]);
    assert!(
        want[1] < 0.6 * whole[1] && got[1] < 1.3 * want[1],
        "the ground in the ring's shadow: {} of the sun, the strip's share {}, {} without the ring",
        got[1],
        want[1],
        whole[1]
    );
}

#[test]
fn a_spinning_planets_frame_rendered_alone_is_the_frame_played() {
    // the draft's rule 7 (SREP 14): the Earth turned with its rotation elements (orientation="bodyFixed", 75° by
    // frame 120 at 3600 scene seconds per second) under clouds drifting 0.01° per scene second (180° by then), its
    // body and layer marked by bands; frame 120 rendered alone is the frame reached by playing from 0
    // stripes 20° wide every 60° of longitude: red on a white ground, and blue, offset by 30°, in a clear layer
    let stripes = |name: &str, on: [u8; 4], off: [u8; 4], offset: u32| {
        let img =
            image::RgbaImage::from_fn(360, 180, |x, _| image::Rgba(if (x + offset) % 60 < 20 { on } else { off }));
        write_fixture_png(name, &img)
    };
    let ground = stripes("planet-seek-ground.png", [255, 0, 0, 255], [255; 4], 0);
    let clouds = stripes("planet-seek-clouds.png", [0, 0, 255, 255], [0; 4], 30);
    let xml = format!(
        r##"<scene version="1.8"><project width="{SIZE}" height="{SIZE}" fps="24" duration="6" background="#000000FF"/><materials><material id="m" baseColor="#FFFFFFFF" baseColorMap="{}" specular="0" roughness="1"/></materials><composition><camera id="eye" x="0" y="0" z="0" fov="60"/><object3D id="p" primitive="planet" material="m" body="399" orientation="bodyFixed" z="400" segments="128" cloudMap="{}" cloudRotation="0.01"/></composition><lights><light id="sun" type="directional" yaw="-60" intensity="3"/></lights><physics pixelsPerMeter="1.5679e-5"/><space epoch="2451545.0" timeScale="tdb" origin="10" rate="3600"/></scene>"##,
        ground.display(),
        clouds.display()
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    let doc = sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"));
    let frames: Vec<f64> = (0..=120).map(|f| f as f64 / 24.0).collect();
    let (Some(alone), Some(played), Some(first)) =
        (render_times(&doc, &frames[120..]), render_times(&doc, &frames), render_times(&doc, &frames[..1]))
    else {
        return;
    };
    for r in [&alone, &played] {
        assert!(
            r.stats.errors.is_empty() && r.stats.unsupported.is_empty(),
            "{:?} {:?}",
            r.stats.errors,
            r.stats.unsupported
        );
    }
    let moved = first.px.iter().zip(&alone.px).filter(|(p, q)| (0..3).any(|c| (p[c] - q[c]).abs() > 0.05)).count();
    assert!(moved > 1000, "{moved} pixels differ from frame 0");
    let off = alone.px.iter().zip(&played.px).filter(|(p, q)| p != q).count();
    assert_eq!(off, 0, "{off} pixels of frame 120 alone differ from frame 120 played");
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
    // under a physical sky's sun both ring shadows darken the bake's diffuse light; the ground's specular light keeps
    // the sun's whole share, which is named
    let attrs = format!(r#"rotationX="{RING_TILT}" ringMap="{}""#, ring_strip(&RING_STRIP).display());
    let note = "p: the clouds' and rings' shadows of a physical sky's sun on the ground's specular light are not drawn by the raster pass";
    let sky = r#"<light id="sky" type="dome" sky="physical" sunIntensity="1" date="2024-06-21T12:00:00Z" latitude="0" moon="false"/>"#;
    let light = r#"<light id="sun" type="directional" yaw="-90" intensity="3"/>"#;
    let lambert = r##"<material id="m" baseColor="#FFFFFFFF" specular="0" roughness="1"/>"##;
    let glossy = r##"<material id="m" baseColor="#FFFFFFFF" specular="0.5" roughness="0.4"/>"##;
    // the path tracer's rays meet the ring and the body, which cast both shadows
    let tracer = r#"renderer="pathtrace" pathSamples="1""#;
    for (camera, material, light, named) in
        [("", lambert, sky, false), ("", glossy, sky, true), ("", glossy, light, false), (tracer, glossy, sky, false)]
    {
        let Some(r) = planet_seen(camera, material, &attrs, light, 0.0) else { return };
        let u = &r.stats.unsupported;
        assert!(u.iter().all(|m| !m.contains("physical sky")) || named, "{camera} {material}: {u:?}");
        assert_eq!(u.iter().any(|m| m == note), named, "{camera} {material}: {u:?}");
    }
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

/// Rule 6's frame: Earth (equatorial radius 63.78137 scene units at 1e-5 px/m) under `atmosphere="earth"` of 100 km,
/// seen from 400 km over its equator through 10° across the limb. The nadir lies in the frame's middle row, `NADIR`
/// degrees left of the view axis, so that the axis is midway between the limb and the grazing ray of the shell's top.
/// The ground is black and unlit, and the sun is 48° from the eye's zenith. Returns the frame and the nadir and sun.
const AIR_W: u32 = 320;
const AIR_H: u32 = 180;
const AIR_PPM: f64 = 1e-5;
const AIR_ALTITUDE: f64 = 400_000.0;
const AIR_HEIGHT: f64 = 100_000.0;
const EARTH_A: f64 = 6_378_137.0;

fn air_band() -> (f64, f64) {
    let b = sr_3d::planet::shell_band(EARTH_A + AIR_ALTITUDE, EARTH_A, AIR_HEIGHT).unwrap();
    (b.planet.unwrap(), b.shell.unwrap())
}

fn air_geometry() -> ([f64; 3], [f64; 3]) {
    let (ground, top) = air_band();
    let m = ((ground + top) / 2.0).to_radians();
    let nadir = [-m.sin(), 0.0, m.cos()];
    let up = nadir.map(|v| -v);
    // 48° from the zenith, toward the view's side of the vertical and the frame's top
    let e1 = [m.cos(), 0.0, m.sin()];
    let (c, s) = (48f64.to_radians().cos(), 48f64.to_radians().sin());
    let sun: [f64; 3] = std::array::from_fn(|i| c * up[i] + s * (0.6 * e1[i] + 0.8 * [0.0, -1.0, 0.0][i]));
    (nadir, sun)
}

/// Rule 3's sun's irradiance in [`air_frame`].
const AIR_IRRADIANCE: f64 = 2.0;

/// The directional light of rule 3's sun in [`air_frame`], of irradiance [`AIR_IRRADIANCE`].
fn air_sun() -> String {
    let (_, sun) = air_geometry();
    let (yaw, pitch) = ((-sun[0]).atan2(-sun[2]).to_degrees(), sun[1].asin().to_degrees());
    format!(r#"<light id="sun" type="directional" intensity="{AIR_IRRADIANCE}" yaw="{yaw}" pitch="{pitch}"/>"#)
}

/// A uniform dome of #808080 (0.2158 linear), visible behind everything.
const AIR_DOME: &str = r##"<light id="sky" type="dome" sky="gradient" skyZenith="#808080" skyHorizon="#808080" skyGround="#808080" environmentVisible="true"/>"##;

fn air_frame(camera: &str, lights: &str) -> Option<Rendered> {
    air_frame_with(camera, lights, 1.0, "#000000FF", "")
}

/// [`air_frame`] under a space of `space`'s attributes.
fn air_frame_in(space: &str, lights: &str) -> Option<Rendered> {
    air_doc("", lights, 1.0, "#000000FF", "", &format!("<space {space}/>"))
}

/// [`air_frame`] with the planet scaled by `scale` at 1/`scale` of the pixels per metre (the same frame), over the
/// project's `background` and with `more` in the composition.
fn air_frame_with(camera: &str, lights: &str, scale: f64, background: &str, more: &str) -> Option<Rendered> {
    air_doc(camera, lights, scale, background, more, "")
}

fn air_doc(camera: &str, lights: &str, scale: f64, background: &str, more: &str, after: &str) -> Option<Rendered> {
    let (nadir, _) = air_geometry();
    let k = (EARTH_A + AIR_ALTITUDE) * AIR_PPM;
    air_doc_at(nadir.map(|v| v * k), camera, lights, scale, background, more, after)
}

/// [`air_doc`] with the planet's centre at `centre` (scene units).
fn air_doc_at(
    centre: [f64; 3],
    camera: &str,
    lights: &str,
    scale: f64,
    background: &str,
    more: &str,
    after: &str,
) -> Option<Rendered> {
    let ppm = AIR_PPM / scale;
    let xml = format!(
        r##"<scene version="1.8"><project width="{AIR_W}" height="{AIR_H}" fps="24" duration="1" background="{background}"/><materials><material id="m" baseColor="#000000FF" unlit="true"/></materials><composition><camera id="eye" x="0" y="0" z="0" fov="10" far="1000" {camera}/><object3D id="p" primitive="planet" body="399" material="m" x="{}" y="{}" z="{}" scaleX="{scale}" scaleY="{scale}" scaleZ="{scale}" segments="512" atmosphere="earth" atmosphereHeight="{AIR_HEIGHT}"/>{more}</composition><lights>{lights}</lights><physics pixelsPerMeter="{ppm}"/>{after}</scene>"##,
        centre[0], centre[1], centre[2]
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    let doc = sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"));
    let r = render(&doc)?;
    assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
    Some(r)
}

const LUMA: [f64; 3] = [0.2126, 0.7152, 0.0722];

/// The unit ray through the middle row at column x + 0.5 (camera at the origin looking down +z, x to the right).
fn air_ray(x: f64) -> [f64; 3] {
    air_ray_at(x, AIR_H as f64 / 2.0)
}

/// The unit ray through the point (x, y) of the frame, in pixels.
fn air_ray_at(x: f64, y: f64) -> [f64; 3] {
    let f = AIR_W as f64 / 2.0 / 5f64.to_radians().tan();
    let d = [(x - AIR_W as f64 / 2.0) / f, (y - AIR_H as f64 / 2.0) / f, 1.0];
    let n = (d[0] * d[0] + d[1] * d[1] + 1.0).sqrt();
    d.map(|v| v / n)
}

/// The reference along the middle row: the luminance of rule 9's sky where the ray misses the planet and of rule 10's
/// aerial perspective of the black ground where it meets it, and the column's angle from the nadir.
fn air_reference(x: f64) -> (f64, f64) {
    let (angle, l, _) = air_reference_rgb(x);
    (angle, (0..3).map(|i| l[i] * LUMA[i]).sum())
}

/// [`air_reference`]'s angle and radiance per channel, and the transmittance to space of a ray that misses the planet.
fn air_reference_rgb(x: f64) -> (f64, [f64; 3], [f64; 3]) {
    air_reference_through(air_ray(x))
}

/// [`air_reference_rgb`] along the unit ray d.
fn air_reference_through(d: [f64; 3]) -> (f64, [f64; 3], [f64; 3]) {
    let (nadir, sun) = air_geometry();
    let s = sr_3d::atmosphere::Scattering::new(
        sr_3d::atmosphere::atmosphere(&sr_3d::atmosphere::AtmosphereOptions {
            planet_radius: EARTH_A,
            top_radius: EARTH_A + AIR_HEIGHT,
            sun_irradiance: [AIR_IRRADIANCE; 3],
            ..Default::default()
        })
        .unwrap(),
    );
    let obs = nadir.map(|v| -v * (EARTH_A + AIR_ALTITUDE));
    let b = obs.iter().zip(d).map(|(o, d)| o * d).sum::<f64>();
    let c = obs.iter().map(|o| o * o).sum::<f64>() - EARTH_A * EARTH_A;
    let (l, t) = if b * b - c >= 0.0 && -b - (b * b - c).sqrt() > 0.0 {
        (s.aerial_perspective(obs, d, -b - (b * b - c).sqrt(), [0.0; 3], sun).unwrap(), [0.0; 3])
    } else {
        (s.sky_radiance(obs, d, sun).unwrap(), s.atmosphere.transmittance(obs, d).unwrap())
    };
    let angle = d.iter().zip(nadir).map(|(d, n)| d * n).sum::<f64>().clamp(-1.0, 1.0).acos().to_degrees();
    (angle, l, t)
}

/// The kit's measures of a profile over increasing angles: the peak's angle and luminance, the largest angle that
/// still reaches 1 % of the peak, and the angle of the largest rise between neighbours (the limb).
fn air_measures(profile: &[(f64, f64)]) -> (f64, f64, f64, f64) {
    let (peak_angle, peak) = profile.iter().fold((0.0, 0.0f64), |b, &(a, l)| if l > b.1 { (a, l) } else { b });
    let level = peak * 0.01;
    let mut edge = f64::NAN;
    for w in profile.windows(2) {
        let ((a0, l0), (a1, l1)) = (w[0], w[1]);
        if l0 >= level && level > l1 {
            edge = a0 + (a1 - a0) * (l0 - level) / (l0 - l1);
        } else if l1 >= level {
            edge = a1;
        }
    }
    let (mut jump, mut limb) = (f64::NEG_INFINITY, f64::NAN);
    for w in profile.windows(2) {
        if w[1].1 - w[0].1 > jump {
            jump = w[1].1 - w[0].1;
            limb = (w[0].0 + w[1].0) / 2.0;
        }
    }
    (peak_angle, peak, edge, limb)
}

#[test]
fn an_earths_atmosphere_shell_is_the_physical_skys_from_orbit() {
    let Some(r) = air_frame("", &air_sun()) else { return };
    air_from_orbit(&r, 0.01);
}

/// The path tracer's camera of 64 samples a pixel, their mean undenoised: their spread over a pixel near the shell's
/// top, where the light falls by a sixth from one pixel to the next, leaves about 1.5 % of noise.
const AIR_TRACED: &str = r#"renderer="pathtrace" pathSamples="64" maxBounces="1" denoise="false""#;

#[test]
fn the_path_tracer_draws_the_shell_as_the_physical_skys_from_orbit() {
    // its samples spread over each pixel where the raster pass reads the pixel's centre
    let Some(r) = air_frame(AIR_TRACED, &air_sun()) else { return };
    assert!(!r.stats.unsupported.iter().any(|m| m.contains("atmosphere shell")), "{:?}", r.stats.unsupported);
    air_from_orbit(&r, 0.02);
}

/// [`air_frame`]'s measures against the physical sky's reference, the band's pixels within `tol` of it.
fn air_from_orbit(r: &Rendered, tol: f64) {
    let (ground, top) = air_band();
    let rows = AIR_H / 2 - 2..=AIR_H / 2 + 2;
    let measured: Vec<(f64, f64)> = (0..AIR_W)
        .map(|x| {
            let l = rows.clone().map(|y| (0..3).map(|i| r.at(x, y)[i] as f64 * LUMA[i]).sum::<f64>()).sum::<f64>();
            (air_reference(x as f64 + 0.5).0, l / 5.0)
        })
        .collect();
    let reference: Vec<(f64, f64)> = (0..AIR_W).map(|x| air_reference(x as f64 + 0.5)).collect();
    let (pa, pl, ea, la) = air_measures(&measured);
    let (ra, rl, re, rla) = air_measures(&reference);
    assert!(pl > 0.0, "the shell is drawn");
    assert!((pa - ra).abs() <= 0.1, "the brightest column at {pa:.3}°, the reference's {ra:.3}°");
    assert!((ea - re).abs() <= 0.1, "the 1 % edge at {ea:.3}°, the reference's {re:.3}°");
    assert!(
        (la - ground).abs() <= 0.1 && (rla - ground).abs() <= 0.1,
        "the limb at {la:.3}° ({rla:.3}°), {ground:.3}°"
    );
    assert!((pl / rl - 1.0).abs() < 0.05, "the peak's luminance {pl} against the reference's {rl}: the sun's light");
    let above = measured.iter().filter(|(a, _)| *a >= top + 0.1).map(|(_, l)| *l).fold(0.0, f64::max);
    assert!(above == 0.0, "above the top: {above} of the peak {pl}");
    // the band's light pixel by pixel, on the middle row and off it, where the azimuth from the sun's vertical turns
    let mut lit = 0;
    for y in [12, AIR_H / 2, AIR_H - 12] {
        for x in 0..AIR_W {
            let (angle, l, _) = air_reference_through(air_ray_at(x as f64 + 0.5, y as f64 + 0.5));
            if angle < ground + 0.3 || angle > top - 0.3 || (0..3).map(|i| l[i] * LUMA[i]).sum::<f64>() < 0.01 * rl {
                continue;
            }
            let got = r.at(x, y);
            for c in 0..3 {
                assert!(
                    (got[c] as f64 - l[c]).abs() <= tol * l[c] + 1e-4,
                    "({x}, {y}) at {angle:.3}°: {got:?}, the reference's {l:?}"
                );
            }
            lit += 1;
        }
    }
    assert!(lit > 80, "{lit} pixels of the band");
    // the black ground below the limb shows the air between it and the eye (rule 10)
    let ground_cols: Vec<usize> = (0..AIR_W as usize).filter(|&x| measured[x].0 < ground - 0.3).collect();
    assert!(ground_cols.len() > 20, "{} columns of ground", ground_cols.len());
    for &x in &ground_cols {
        let (got, want) = (measured[x].1, reference[x].1);
        assert!(
            want > 0.0 && (got / want - 1.0).abs() < 0.02,
            "the ground at {:.3}°: {got}, the reference's {want}",
            measured[x].0
        );
    }
}

/// The middle row's radiance per channel at column x, averaged over five rows.
fn air_row(r: &Rendered, x: u32) -> [f64; 3] {
    std::array::from_fn(|c| (AIR_H / 2 - 2..=AIR_H / 2 + 2).map(|y| r.at(x, y)[c] as f64).sum::<f64>() / 5.0)
}

/// Every column of the middle row whose ray passes through the shell clear of its limb and its top, with what the
/// reference says lies there: the dome of `dome` linear seen through the air, plus the air's light when `lit`.
fn air_through_dome(r: &Rendered, dome: f64, lit: bool) -> usize {
    let (ground, top) = air_band();
    let mut seen = 0;
    for x in 0..AIR_W {
        let (angle, l, t) = air_reference_rgb(x as f64 + 0.5);
        if angle < ground + 0.3 || angle > top - 0.1 {
            let got = air_row(r, x);
            if angle > top + 0.1 {
                for c in 0..3 {
                    assert!((got[c] - dome).abs() < 1e-3, "above the top at {angle:.3}°: {got:?}, the dome {dome}");
                }
            }
            continue;
        }
        let got = air_row(r, x);
        for c in 0..3 {
            let want = dome * t[c] + if lit { l[c] } else { 0.0 };
            assert!(
                (got[c] - want).abs() < 0.03 * want + 2e-3,
                "at {angle:.3}°, channel {c}: {got:?}, the reference's {want}"
            );
        }
        seen += 1;
    }
    seen
}

#[test]
fn the_shell_dims_the_dome_behind_it_and_adds_its_light() {
    let Some(r) = air_frame("", &format!("{}{AIR_DOME}", air_sun())) else { return };
    assert!(air_through_dome(&r, srgb_linear(128.0 / 255.0), true) > 40);
}

#[test]
fn the_path_tracer_sees_the_dome_through_the_shell() {
    let dome = srgb_linear(128.0 / 255.0);
    let Some(r) = air_frame(AIR_TRACED, &format!("{}{AIR_DOME}", air_sun())) else { return };
    assert!(air_through_dome(&r, dome, true) > 40);
    let Some(r) = air_frame(AIR_TRACED, AIR_DOME) else { return };
    assert!(air_through_dome(&r, dome, false) > 40);
}

#[test]
fn without_a_sun_the_shell_only_dims_what_lies_beyond_it() {
    let Some(r) = air_frame("", AIR_DOME) else { return };
    assert!(air_through_dome(&r, srgb_linear(128.0 / 255.0), false) > 40);
    // the space's Sun gives the night lights a direction but the air no light: rule 6's irradiance is a light's
    let Some(r) = air_frame_in(r#"epoch="2027-03-15T00:00:00Z" origin="3""#, AIR_DOME) else { return };
    assert!(air_through_dome(&r, srgb_linear(128.0 / 255.0), false) > 40);
}

fn srgb_linear(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[test]
fn the_raster_pass_names_the_shells_it_does_not_draw() {
    let notes = |camera: &str, lights: &str| air_frame(camera, lights).map(|r| r.stats.unsupported);
    // one atmosphere per document: under a dome with a physical sky the sky's is the one
    let physical = r#"<light id="sky" type="dome" sky="physical" sunIntensity="1" date="2024-06-21T12:00:00Z" latitude="0" moon="false"/>"#;
    let Some(u) = notes("", physical) else { return };
    assert!(u.iter().any(|m| m == r#"p: the atmosphere shell under a dome with sky="physical" is not drawn"#), "{u:?}");
    // 400 km up the eye's orthographic rays start in the plane across the axis through it, 2150 km from the centre
    let Some(u) = notes(r#"projection="orthographic" orthoHeight="10""#, &air_sun()) else { return };
    let air = "p: the atmosphere shell is not drawn through an orthographic camera whose rays start in its air";
    assert!(u.iter().any(|m| m == air), "{u:?}");
    for camera in ["", TRACED] {
        let Some(u) = notes(camera, &air_sun()) else { return };
        assert!(!u.iter().any(|m| m.contains("atmosphere shell")), "{camera}: {u:?}");
    }
    // a second planet with air, behind the eye: the first is drawn and the second named
    let second = r#"<object3D id="q" primitive="planet" radius="1" z="-50" material="m" atmosphere="earth"/>"#;
    let Some(r) = air_frame_with("", &air_sun(), 1.0, "#000000FF", second) else { return };
    let u = &r.stats.unsupported;
    assert!(u.iter().any(|m| m == "q: the atmosphere shell is drawn for the first planet that has one alone"), "{u:?}");
    assert!(air_row(&r, AIR_W / 2).iter().any(|v| *v > 0.01), "the first planet's shell is drawn");
}

#[test]
fn a_scaled_planet_takes_its_air_at_its_scale() {
    // twice the scale at half the pixels per metre is the same planet in the same frame
    let (Some(a), Some(b)) = (air_frame("", &air_sun()), air_frame_with("", &air_sun(), 2.0, "#000000FF", "")) else {
        return;
    };
    for x in (0..AIR_W).step_by(4) {
        let (p, q) = (air_row(&a, x), air_row(&b, x));
        for c in 0..3 {
            assert!((p[c] - q[c]).abs() <= 0.01 * p[c] + 1e-4, "column {x}: {p:?} and scaled {q:?}");
        }
    }
}

#[test]
fn the_shell_adds_its_light_over_the_backdrop() {
    // without a visible dome nothing of the 3D pass lies beyond the shell: its light adds to what the pass is
    // composited over, and above its top that shows through untouched; alike in both renderers
    for camera in ["", AIR_TRACED] {
        let Some(r) = air_frame_with(camera, &air_sun(), 1.0, "#336699FF", "") else { return };
        air_over_backdrop(&r);
    }
}

fn air_over_backdrop(r: &Rendered) {
    let bg = [0x33, 0x66, 0x99].map(|v| srgb_linear(v as f64 / 255.0));
    let (ground, top) = air_band();
    let mut seen = 0;
    for x in 0..AIR_W {
        let (angle, l, _) = air_reference_rgb(x as f64 + 0.5);
        let got = air_row(r, x);
        if angle > top + 0.1 {
            for c in 0..3 {
                assert!((got[c] - bg[c]).abs() < 2e-3, "above the top at {angle:.3}°: {got:?}, the backdrop {bg:?}");
            }
        } else if angle > ground + 0.3 && angle < top - 0.1 {
            for c in 0..3 {
                let want = bg[c] + l[c];
                assert!((got[c] - want).abs() < 0.03 * want + 2e-3, "at {angle:.3}°, channel {c}: {got:?}, {want}");
            }
            seen += 1;
        }
    }
    assert!(seen > 40, "{seen} columns in the band");
}

/// An orthographic frame 4 units high whose axis passes `AIR_ORTHO_IMPACT` metres from the planet's centre, 100 units
/// ahead, the limb crossing the frame upright: rays left of the limb meet the ground, rays right of it the band.
const AIR_ORTHO_IMPACT: f64 = EARTH_A + 50_000.0;
const AIR_ORTHO_HEIGHT: f64 = 4.0;

fn air_ortho_centre() -> [f64; 3] {
    [-AIR_ORTHO_IMPACT * AIR_PPM, 0.0, 100.0]
}

/// The sun of [`air_ortho`]: toward the eye's side of the limb, 40° off the axis.
fn air_ortho_sun() -> ([f64; 3], String) {
    let (c, s) = (40f64.to_radians().cos(), 40f64.to_radians().sin());
    let sun = [0.8 * s, -0.6 * s, -c];
    let (yaw, pitch) = ((-sun[0]).atan2(-sun[2]).to_degrees(), sun[1].asin().to_degrees());
    (sun, format!(r#"<light id="sun" type="directional" intensity="{AIR_IRRADIANCE}" yaw="{yaw}" pitch="{pitch}"/>"#))
}

fn air_ortho(camera: &str, background: &str) -> Option<Rendered> {
    let camera = format!(r#"projection="orthographic" orthoHeight="{AIR_ORTHO_HEIGHT}" {camera}"#);
    air_doc_at(air_ortho_centre(), &camera, &air_ortho_sun().1, 1.0, background, "", "")
}

/// The reference at pixel (x, y) of [`air_ortho`]: the ray's impact on the shell in metres, its radiance (the sky's
/// where it misses the planet, the black ground's aerial perspective where it meets it) and transmittance.
fn air_ortho_reference(x: f64, y: f64) -> (f64, [f64; 3], [f64; 3]) {
    let px = AIR_ORTHO_HEIGHT / AIR_H as f64;
    let p = [(x - AIR_W as f64 / 2.0) * px, (y - AIR_H as f64 / 2.0) * px, 0.0];
    let c = air_ortho_centre();
    let o: [f64; 3] = std::array::from_fn(|i| (p[i] - c[i]) / AIR_PPM);
    let d = [0.0, 0.0, 1.0];
    let s = sr_3d::atmosphere::Scattering::new(
        sr_3d::atmosphere::atmosphere(&sr_3d::atmosphere::AtmosphereOptions {
            planet_radius: EARTH_A,
            top_radius: EARTH_A + AIR_HEIGHT,
            sun_irradiance: [AIR_IRRADIANCE; 3],
            ..Default::default()
        })
        .unwrap(),
    );
    let sun = air_ortho_sun().0;
    let rho = (o[0] * o[0] + o[1] * o[1]).sqrt();
    if rho < EARTH_A {
        let ground = -o[2] - (EARTH_A * EARTH_A - rho * rho).sqrt();
        (rho, s.aerial_perspective(o, d, ground, [0.0; 3], sun).unwrap(), [0.0; 3])
    } else {
        (rho, s.sky_radiance(o, d, sun).unwrap(), s.atmosphere.transmittance(o, d).unwrap())
    }
}

/// [`air_ortho`]'s frame against the reference on three rows: the band within `tol`, nothing past the top, the ground
/// more than 50 km inside the limb within `ground_tol`. The march's 65 columns span the frame, 11 km of impact apart
/// (the kernel's own volume is within 1.6 % of the reference there), and the 512-segment mesh's ground lies up to
/// 120 m inside the spheroid, which lengthens the grazing rays' air.
fn air_ortho_matches(r: &Rendered, tol: f64, ground_tol: f64) {
    let (mut band, mut ground, mut clear) = (0, 0, 0);
    let mut peak = 0.0f64;
    for y in [12, AIR_H / 2, AIR_H - 12] {
        for x in 0..AIR_W {
            let (rho, l, _) = air_ortho_reference(x as f64 + 0.5, y as f64 + 0.5);
            let got = r.at(x, y);
            peak = peak.max(l[2]);
            if rho > EARTH_A + AIR_HEIGHT + 3_000.0 {
                assert!(got[..3].iter().all(|v| *v == 0.0), "({x}, {y}) past the top: {got:?}");
                clear += 1;
            } else if rho > EARTH_A + 3_000.0 && rho < EARTH_A + AIR_HEIGHT - 3_000.0 && l[2] > 1e-4 {
                for c in 0..3 {
                    assert!(
                        (got[c] as f64 - l[c]).abs() <= tol * l[c] + 1e-4,
                        "({x}, {y}) {:.1} km up: {got:?}, the reference's {l:?}",
                        (rho - EARTH_A) / 1e3
                    );
                }
                band += 1;
            } else if rho < EARTH_A - 50_000.0 {
                for c in 0..3 {
                    assert!(
                        (got[c] as f64 - l[c]).abs() <= ground_tol * l[c] + 1e-4,
                        "the ground at ({x}, {y}), {:.1} km in: {got:?}, the reference's {l:?}",
                        (EARTH_A - rho) / 1e3
                    );
                }
                ground += 1;
            }
        }
    }
    assert!(peak > 0.01, "the band is lit: {peak}");
    assert!(band > 60 && ground > 60 && clear > 60, "{band} band, {ground} ground and {clear} clear pixels");
}

#[test]
fn an_orthographic_camera_sees_the_shell_by_each_rays_impact() {
    let Some(r) = air_ortho("", "#000000FF") else { return };
    assert!(!r.stats.unsupported.iter().any(|m| m.contains("atmosphere shell")), "{:?}", r.stats.unsupported);
    air_ortho_matches(&r, 0.01, 0.04);
}

#[test]
fn the_path_tracer_sees_the_shell_through_an_orthographic_camera() {
    let Some(r) = air_ortho(AIR_TRACED, "#000000FF") else { return };
    air_ortho_matches(&r, 0.02, 0.04);
}
