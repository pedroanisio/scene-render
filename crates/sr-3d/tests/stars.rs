//! The star field a dome light draws (sr_3d::stars, the star-fields draft's rules 2, 4 and 6): a star's flux spread
//! over a disc of pointSize pixels so that its pixels sum to the flux, and the field's directions at an epoch.

use sr_3d::space::stars::{
    angle, constellation, flux, load_catalogue, moved, radec, scene_direction, star_field, CATALOGUE_DIGEST,
};
use sr_3d::stars::{boundaries, disc_cover, splat, Field, Point, BOUNDARY_STEP_DEG, SCENE_FROM_ICRF_AXES};

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
    // a centre that is not a number, or at infinity, is on no pixel
    for pixel in [[f64::NAN, 4.0], [5.0, f64::NAN], [f64::INFINITY, 4.0], [5.0, f64::NEG_INFINITY]] {
        splat(&mut none, [w, h], &[Point { pixel, rgb: [1.0; 3] }], 2.0);
    }
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

/// The constellation of an ICRF direction (rule 5, at J2000).
fn named(d: [f64; 3]) -> &'static str {
    let (ra, dec) = radec(d).unwrap();
    constellation(ra, dec, 2000.0).unwrap()
}

fn unit(a: [f64; 3]) -> [f64; 3] {
    let n = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    a.map(|v| v / n)
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn along(a: [f64; 3], b: [f64; 3], k: f64) -> [f64; 3] {
    unit([a[0] + k * b[0], a[1] + k * b[1], a[2] + k * b[2]])
}

#[test]
fn the_boundaries_lie_between_constellations() {
    let lines = boundaries();
    assert!(!lines.is_empty());
    let step = BOUNDARY_STEP_DEG.to_radians();
    for line in lines {
        assert!(line.len() >= 2);
        for w in line.windows(2) {
            assert!(angle(w[0], w[1]).unwrap() <= step * 1.000_001, "{w:?}");
            // across the middle of each piece, the constellations on either side differ
            let m = unit([w[0][0] + w[1][0], w[0][1] + w[1][1], w[0][2] + w[1][2]]);
            let n = unit(cross(m, [w[1][0] - w[0][0], w[1][1] - w[0][1], w[1][2] - w[0][2]]));
            assert_ne!(named(along(m, n, 1e-5)), named(along(m, n, -1e-5)), "{m:?}");
        }
    }
}

#[test]
fn neighbouring_directions_in_two_constellations_have_a_boundary_between_them() {
    let points: Vec<[f64; 3]> = boundaries().iter().flatten().copied().collect();
    // a Fibonacci lattice of directions, each with its neighbour 0.2° away toward increasing right ascension
    let (n, gap) = (6000, 0.2f64.to_radians());
    let mut crossings = 0;
    for i in 0..n {
        let z = 1.0 - (2 * i + 1) as f64 / n as f64;
        let phi = i as f64 * std::f64::consts::PI * (3.0 - 5f64.sqrt());
        let r = (1.0 - z * z).sqrt();
        let a = [r * phi.cos(), r * phi.sin(), z];
        let east = unit(cross([0.0, 0.0, 1.0], a));
        let b = along(a, east, gap.tan());
        if named(a) == named(b) {
            continue;
        }
        crossings += 1;
        let near = points.iter().map(|p| angle(*p, a).unwrap().min(angle(*p, b).unwrap())).fold(f64::MAX, f64::min);
        assert!(near <= gap + BOUNDARY_STEP_DEG.to_radians() / 2.0, "{a:?} {} | {b:?} {}", named(a), named(b));
    }
    assert!(crossings > 50, "{crossings}");
    // the 88 constellations each meet a boundary
    let mut seen: Vec<&str> = points.iter().map(|p| named(*p)).collect();
    seen.sort_unstable();
    seen.dedup();
    assert!(seen.len() > 80, "{}", seen.len());
}

#[test]
fn a_label_is_at_the_centroid_of_its_catalogue_stars() {
    let cat = catalogue();
    // a limit that keeps few stars does not move a label: every catalogue star counts
    let field = Field::new(&cat, 2.0, 1.0).unwrap();
    let labels = field.labels(100.0, true, &SCENE_FROM_ICRF_AXES).unwrap();
    assert_eq!(labels.len(), 88);
    for (abbr, d) in &labels {
        assert!((d.iter().map(|v| v * v).sum::<f64>() - 1.0).abs() < 1e-12, "{abbr}: {d:?} is not a direction");
    }
    for abbr in ["CMa", "Ori", "UMi", "Oct", "Ser"] {
        let mut sum = [0.0; 3];
        for s in cat.iter().filter(|s| constellation(s.ra, s.dec, 2000.0).unwrap() == abbr) {
            let d = moved(s, 100.0).unwrap();
            sum = [sum[0] + d[0], sum[1] + d[1], sum[2] + d[2]];
        }
        let want = scene_direction(unit(sum));
        let got = labels.iter().find(|l| l.0 == abbr).unwrap().1;
        assert!(angle(got, want).unwrap() < 1e-12, "{abbr}: {got:?} {want:?}");
    }
    // the bright labels the kit names lie in their own constellations
    for abbr in ["CMa", "Ori", "UMi"] {
        let d = labels.iter().find(|l| l.0 == abbr).unwrap().1;
        let icrf = [d[0], d[2], -d[1]];
        assert_eq!(named(icrf), abbr);
    }
    // without proper motion, the J2000 places
    let still = field.labels(100.0, false, &SCENE_FROM_ICRF_AXES).unwrap();
    assert_ne!(still, labels);
    assert_eq!(still, field.labels(0.0, true, &SCENE_FROM_ICRF_AXES).unwrap());
}

/// The right ascension and declination of ICRF direction `d`, degrees.
fn ra_dec(d: [f64; 3]) -> (f64, f64) {
    (d[1].atan2(d[0]).to_degrees().rem_euclid(360.0), d[2].clamp(-1.0, 1.0).asin().to_degrees())
}

#[test]
fn labels_and_boundaries_agree_with_the_drafts_reference_values() {
    // values from an independent reading of rule 5, within 0.01°
    let cat = catalogue();
    let labels = Field::new(&cat, 6.5, 1.0).unwrap().labels(0.0, true, &SCENE_FROM_ICRF_AXES).unwrap();
    for (abbr, ra, dec) in [("CMa", 103.59, -23.0889), ("Ori", 84.2191, 3.66242), ("UMi", 227.947, 77.487)] {
        let d = labels.iter().find(|l| l.0 == abbr).unwrap().1;
        let got = [d[0], d[2], -d[1]];
        let want = sr_3d::space::stars::direction(ra, dec).unwrap();
        assert!(angle(got, want).unwrap().to_degrees() < 0.01, "{abbr}: {:?}", ra_dec(got));
    }
    // the midpoint of Orion's southern arc, RA 5h27.5m Dec −11° at 1875, lies on a boundary at its J2000 place
    let want = sr_3d::space::stars::direction(83.342, -10.9105).unwrap();
    let nearest = boundaries()
        .iter()
        .flat_map(|l| l.windows(2))
        .map(|w| {
            // the angle from `want` to the great-circle piece w[0]–w[1]
            let n = unit(cross(w[0], w[1]));
            let off = (want[0] * n[0] + want[1] * n[1] + want[2] * n[2]).asin().abs();
            let inside = angle(w[0], want).unwrap().max(angle(want, w[1]).unwrap()) <= angle(w[0], w[1]).unwrap();
            if inside {
                off
            } else {
                angle(w[0], want).unwrap().min(angle(w[1], want).unwrap())
            }
        })
        .fold(f64::MAX, f64::min);
    assert!(nearest.to_degrees() < 0.01, "{}", nearest.to_degrees());
}
