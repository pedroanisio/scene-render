//! The orbits draft drawn (rules 2, 5 and 6, kit cases orbit-ellipse and trail of sr-core
//! `srep/srep-0000-orbits.md`): an orbiting sphere is centred within 1 px of its scene position, and its trail is a
//! ribbon along the orbit in the trail's colour, opaque at the object and faded to nothing at its oldest end, under
//! an orthographic camera that puts the scene's origin at the frame's centre. The orbit is inclined 90° so that it
//! lies in the scene's xy plane, facing the camera.

mod common;
use common::*;

use sr_3d::space::orbits::{orbit, orbit_state, period, GM_SUN};

const SIZE: u32 = 512;
/// 1 au is 200 px.
const PPM: f64 = 200.0 / 149_597_870_700.0;

fn document(trail: &str) -> sr_model::Document {
    let p = period(GM_SUN, 149_597_870_700.0).unwrap();
    let xml = format!(
        r##"<scene version="1.8"><project width="{SIZE}" height="{SIZE}" fps="24" duration="1" background="#000000FF"/><materials><material id="white" baseColor="#FFFFFFFF" unlit="true"/></materials><composition><camera id="eye" projection="orthographic" z="-1000"/><object3D id="sun" primitive="sphere" radius="1" body="10" visible="false"/><object3D id="earth" primitive="sphere" radius="6" material="white"><orbit around="sun" a="149597870700" e="0.3" i="90" argp="40"/>{trail}</object3D></composition><physics pixelsPerMeter="{PPM}"/><space epoch="2451545.0" timeScale="tdb" rate="{p}"/></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

fn rendered(trail: &str, t: f64) -> Option<Rendered> {
    let r = render_times(&document(trail), &[t])?;
    assert!(
        r.stats.errors.is_empty() && r.stats.unsupported.is_empty(),
        "{:?} {:?}",
        r.stats.errors,
        r.stats.unsupported
    );
    Some(r)
}

/// The pixel of a scene position under the orthographic camera at the origin.
fn pixel(p: [f64; 3]) -> [f64; 2] {
    [SIZE as f64 / 2.0 + p[0], SIZE as f64 / 2.0 + p[1]]
}

/// The scene position of the orbit at `t` scene seconds (a period each).
fn place(t: f64) -> [f64; 3] {
    let au = 149_597_870_700.0;
    let o = orbit(GM_SUN, au, 0.3, 90.0, 0.0, 40.0, 0.0, 0.0).unwrap();
    let r = orbit_state(&o, t * period(GM_SUN, au).unwrap()).unwrap().0;
    [r[0] * PPM, -r[2] * PPM, r[1] * PPM]
}

/// The centre of the white pixels.
fn centre(r: &Rendered) -> [f64; 2] {
    let (mut n, mut sx, mut sy) = (0usize, 0f64, 0f64);
    for y in 0..r.size[1] {
        for x in 0..r.size[0] {
            let c = r.at(x, y);
            if c[0] > 0.5 && c[1] > 0.5 {
                n += 1;
                sx += x as f64 + 0.5;
                sy += y as f64 + 0.5;
            }
        }
    }
    assert!(n > 50, "the sphere covers {n} pixels");
    [sx / n as f64, sy / n as f64]
}

/// The most red of the pixels within 2 px of the pixel of `p`.
fn red_near(r: &Rendered, p: [f64; 3]) -> f32 {
    let [cx, cy] = pixel(p);
    let mut best = 0f32;
    for dy in -2..=2 {
        for dx in -2..=2 {
            let (x, y) = ((cx as i64 + dx) as u32, (cy as i64 + dy) as u32);
            let c = r.at(x, y);
            best = best.max(c[0] - c[1].max(c[2]));
        }
    }
    best
}

#[test]
fn an_orbiting_sphere_is_drawn_at_its_place() {
    for t in [0.0, 0.25, 0.5] {
        let Some(r) = rendered("", t) else { return };
        let (got, want) = (centre(&r), pixel(place(t)));
        assert!((got[0] - want[0]).hypot(got[1] - want[1]) <= 1.0, "t {t}: centre {got:?}, expected {want:?}");
    }
}

#[test]
fn a_trail_fades_along_the_orbit_behind_its_object() {
    let p = period(GM_SUN, 149_597_870_700.0).unwrap();
    let trail = format!(r##"<trail length="{}" samples="64" width="4" color="#FF0000FF"/>"##, p / 4.0);
    let Some(r) = rendered(&trail, 0.5) else { return };
    // along the last quarter period: strong just behind the sphere, weaker further back, nothing at the oldest end
    let (near, middle, old) = (red_near(&r, place(0.47)), red_near(&r, place(0.375)), red_near(&r, place(0.252)));
    assert!(near > 0.6, "near {near}");
    assert!(middle > 0.1 && middle < near, "middle {middle}, near {near}");
    assert!(old < 0.1, "old {old}");
    // and nothing ahead of the object, where the trail has not been
    assert!(red_near(&r, place(0.6)) < 0.02);
    // unfaded, the oldest end is as strong as the newest
    let Some(r) = rendered(&trail.replace("/>", r#" fade="false"/>"#), 0.5) else { return };
    let (near, old) = (red_near(&r, place(0.47)), red_near(&r, place(0.26)));
    assert!(near > 0.6 && (near - old).abs() < 0.05, "near {near}, old {old}");
}
