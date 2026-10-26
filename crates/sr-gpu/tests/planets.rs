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
