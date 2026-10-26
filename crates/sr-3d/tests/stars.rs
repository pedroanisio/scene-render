//! The star field a dome light draws (sr_3d::stars, the star-fields draft's rules 2, 4 and 6): a star's flux spread
//! over a disc of pointSize pixels so that its pixels sum to the flux, and the field's directions at an epoch.

use sr_3d::space::stars::{flux, load_catalogue, moved, scene_direction, star_field, CATALOGUE_DIGEST};
use sr_3d::stars::{disc_cover, splat, Field, Point, SCENE_FROM_ICRF_AXES};

fn catalogue() -> Vec<sr_3d::space::stars::Star> {
    let path =
        std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/corpus/media/bsc5-compact.csv.gz"));
    load_catalogue(path, Some(CATALOGUE_DIGEST)).unwrap()
}

/// The coverage of a pixel by brute force: the share of `n`² sample points of the pixel inside the disc, over the
/// disc's area.
fn sampled(cx: f64, cy: f64, r: f64, px: i64, py: i64, n: u32) -> f64 {
    let mut inside = 0u32;
    for i in 0..n {
        for j in 0..n {
            let x = px as f64 + (i as f64 + 0.5) / n as f64 - cx;
            let y = py as f64 + (j as f64 + 0.5) / n as f64 - cy;
            inside += u32::from(x * x + y * y <= r * r);
        }
    }
    inside as f64 / (n * n) as f64 / (std::f64::consts::PI * r * r)
}

#[test]
fn a_disc_covers_its_pixels_exactly_once() {
    for &(cx, cy) in &[(0.5f64, 0.5f64), (0.0, 0.0), (0.37, 0.91), (12.25, -3.6), (-0.999, 0.001)] {
        for r in [0.2f64, 0.5, 0.75, 1.0, 1.5, 4.0] {
            let (x0, x1) = ((cx - r).floor() as i64 - 1, (cx + r).ceil() as i64 + 1);
            let (y0, y1) = ((cy - r).floor() as i64 - 1, (cy + r).ceil() as i64 + 1);
            let mut sum = 0.0;
            for py in y0..y1 {
                for px in x0..x1 {
                    let c = disc_cover(cx, cy, r, px, py);
                    assert!((0.0..=1.0 / (std::f64::consts::PI * r * r) + 1e-12).contains(&c), "{c}");
                    assert!(
                        (c - sampled(cx, cy, r, px, py, 200)).abs() <= 2e-2 / (r * r).max(1.0),
                        "{cx} {cy} {r} {px} {py}"
                    );
                    sum += c;
                }
            }
            assert!((sum - 1.0).abs() < 1e-12, "centre ({cx}, {cy}) radius {r}: {sum}");
        }
    }
    // a pixel well inside the disc holds 1 / (π r²); one outside holds nothing
    assert!((disc_cover(10.5, 10.5, 4.0, 10, 10) - 1.0 / (std::f64::consts::PI * 16.0)).abs() < 1e-15);
    assert_eq!(disc_cover(10.5, 10.5, 1.0, 13, 10), 0.0);
}

#[test]
fn a_splat_sums_to_the_flux_and_clips_at_the_frame() {
    let (w, h) = (16u32, 8u32);
    let mut buf = vec![[0f32; 3]; (w * h) as usize];
    let p = Point { pixel: [5.3, 4.1], rgb: [2.0, 1.0, 0.5] };
    splat(&mut buf, [w, h], &[p], 1.5);
    let sum = buf.iter().fold([0f64; 3], |s, c| [s[0] + c[0] as f64, s[1] + c[1] as f64, s[2] + c[2] as f64]);
    for (got, want) in sum.iter().zip([2.0, 1.0, 0.5]) {
        assert!((got - want).abs() < 1e-5, "{sum:?}");
    }
    // the pixel holding the centre is the brightest
    let peak = (0..buf.len()).max_by(|&a, &b| buf[a][0].total_cmp(&buf[b][0])).unwrap();
    assert_eq!((peak as u32 % w, peak as u32 / w), (5, 4));
    // a star on the frame's edge keeps only the share that falls inside it
    let mut edge = vec![[0f32; 3]; (w * h) as usize];
    splat(&mut edge, [w, h], &[Point { pixel: [0.0, 4.0], rgb: [1.0; 3] }], 2.0);
    let inside: f64 = edge.iter().map(|c| c[0] as f64).sum();
    assert!((inside - 0.5).abs() < 1e-5, "{inside}");
    // far outside the frame, nothing
    let mut none = vec![[0f32; 3]; (w * h) as usize];
    splat(&mut none, [w, h], &[Point { pixel: [-40.0, 1e9], rgb: [1.0; 3] }], 2.0);
    assert!(none.iter().all(|c| *c == [0.0; 3]));
}

#[test]
fn the_field_is_the_drafts_field_in_scene_directions() {
    let cat = catalogue();
    let field = Field::new(&cat, 6.5, 0.25).unwrap();
    let want = star_field(&cat, 6.5, 25.0, 0.25).unwrap();
    let got = field.at(25.0, true, &SCENE_FROM_ICRF_AXES).unwrap();
    assert_eq!(got.len(), want.len());
    for (g, w) in got.iter().zip(&want) {
        assert_eq!(g.direction, scene_direction(w.direction));
        assert_eq!((g.flux, g.colour), (w.flux, w.colour));
    }
    // without proper motion the directions are the J2000 places at any epoch
    let still = field.at(25.0, false, &SCENE_FROM_ICRF_AXES).unwrap();
    let sirius = cat.iter().position(|s| s.hr == 2491).unwrap();
    let k = want.iter().position(|s| s.hr == 2491).unwrap();
    assert_eq!(still[k].direction, scene_direction(moved(&cat[sirius], 0.0).unwrap()));
    assert_ne!(still[k].direction, got[k].direction);
    assert_eq!(got[k].flux, flux(-1.46, 0.25).unwrap());
    // Polaris is near the scene's up, −y
    let polaris = want.iter().position(|s| s.hr == 424).unwrap();
    assert!(got[polaris].direction[1] < -0.9999, "{:?}", got[polaris].direction);
    assert!(Field::new(&cat, f64::NAN, 1.0).is_err() && Field::new(&cat, 6.5, 0.0).is_err());
    assert!(field.at(2e4, true, &SCENE_FROM_ICRF_AXES).is_err());
    // a space's frame turns the field: under the ecliptic frame the ecliptic pole is up
    let ecliptic = sr_3d::space::mat_mul(&SCENE_FROM_ICRF_AXES, &sr_3d::space::ecliptic_from_icrf());
    let turned = field.at(0.0, false, &ecliptic).unwrap();
    let pole = sr_3d::space::mat_apply(&ecliptic, sr_3d::space::stars::direction(270.0, 66.560708).unwrap());
    assert!(pole[1] < -0.999_999_99, "{pole:?}");
    assert_eq!(turned[k].direction, sr_3d::space::mat_apply(&ecliptic, moved(&cat[sirius], 0.0).unwrap()));
}
