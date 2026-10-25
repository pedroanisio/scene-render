//! The time-frames draft drawn (rule 7, kit cases space-earth-moon and space-rate of sr-core
//! `srep/srep-0000-time-frames.md`): the Moon's sphere, placed by the excerpt ephemeris about the Earth at
//! `pixelsPerMeter="1e-6"`, is centred within 1 px of its scene position, under an orthographic camera that puts
//! the scene's origin at the frame's centre.

mod common;
use common::*;

use sr_3d::space::{place_body, Frame, Space, Spk, TimeScale};

const SIZE: u32 = 1024;
const DIGEST: &str = "d113bb5117946244cf63de9d06287d5aac008acd1a561d28b760bcc7d4c3d1fd";

fn media() -> std::path::PathBuf {
    std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/corpus/media"))
}

fn document(rate: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.8"><project width="{SIZE}" height="{SIZE}" fps="24" duration="3" background="#000000FF"/><assets><ephemeris id="de" src="de440s-excerpt.bsp" digest="sha256:{DIGEST}"/></assets><materials><material id="white" baseColor="#FFFFFFFF" unlit="true"/></materials><composition><camera id="eye" projection="orthographic" z="-1000"/><object3D id="moon" primitive="sphere" radius="8" material="white" body="301" positionFrom="ephemeris"/></composition><physics pixelsPerMeter="1e-6"/><space epoch="2451545.0" timeScale="tdb" rate="{rate}" origin="399" ephemeris="de"/></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(media()) };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

/// The centre of the lit pixels.
fn centre(r: &Rendered) -> [f64; 2] {
    let (mut n, mut sx, mut sy) = (0usize, 0f64, 0f64);
    for y in 0..r.size[1] {
        for x in 0..r.size[0] {
            if r.at(x, y)[0] > 0.5 {
                n += 1;
                sx += x as f64 + 0.5;
                sy += y as f64 + 0.5;
            }
        }
    }
    assert!(n > 100, "the Moon covers {n} pixels");
    [sx / n as f64, sy / n as f64]
}

/// The pixel of a scene position under the orthographic camera at the origin.
fn pixel(p: [f64; 3]) -> [f64; 2] {
    [SIZE as f64 / 2.0 + p[0], SIZE as f64 / 2.0 + p[1]]
}

fn expected(rate: f64, frame: u64) -> [f64; 2] {
    let spk = Spk::open(&media().join("de440s-excerpt.bsp"), Some(&format!("sha256:{DIGEST}"))).unwrap();
    let s = Space::new("2451545.0", TimeScale::Tdb, rate, 399, Frame::Icrf).unwrap();
    pixel(place_body(&spk, &s, 301, frame, 24.0, 1e-6).unwrap().position)
}

fn rendered(rate: &str, t: f64) -> Option<Rendered> {
    let r = render_times(&document(rate), &[t])?;
    assert!(
        r.stats.errors.is_empty() && r.stats.unsupported.is_empty(),
        "{:?} {:?}",
        r.stats.errors,
        r.stats.unsupported
    );
    Some(r)
}

#[test]
fn the_moon_is_drawn_at_its_place_about_the_earth() {
    let Some(r) = rendered("1", 0.0) else { return };
    let (got, want) = (centre(&r), expected(1.0, 0));
    assert!((got[0] - want[0]).hypot(got[1] - want[1]) <= 1.0, "centre {got:?}, expected {want:?}");
}

#[test]
fn the_moon_moves_by_its_two_hour_arc_over_48_frames() {
    let (Some(first), Some(last)) = (rendered("3600", 0.0), rendered("3600", 2.0)) else { return };
    for (r, f) in [(first, 0), (last, 48)] {
        let (got, want) = (centre(&r), expected(3600.0, f));
        assert!((got[0] - want[0]).hypot(got[1] - want[1]) <= 1.0, "frame {f}: centre {got:?}, expected {want:?}");
    }
    // the two places differ by more than the tolerance, so the clock moved the body
    let (a, b) = (expected(3600.0, 0), expected(3600.0, 48));
    assert!((a[0] - b[0]).hypot(a[1] - b[1]) > 4.0, "{a:?} {b:?}");
}
