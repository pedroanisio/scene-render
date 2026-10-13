// Adapted from FilmCraft (crates/scopes/src/tests.rs), Copyright (c) 2026 ArtCraft Team and the FilmCraft
// contributors, used under the MIT License; see THIRD-PARTY-NOTICES.md.

//! Scope maths and level checks on generated frames with known values.

use super::*;
use crate::decode::{Plane, VideoFrame};
use crate::PixelLayout;

fn rgba8(w: usize, h: usize, mut f: impl FnMut(usize, usize) -> [u8; 3]) -> Vec<u8> {
    let mut px = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let [r, g, b] = f(x, y);
            px.extend_from_slice(&[r, g, b, 255]);
        }
    }
    px
}

fn signal(w: usize, h: usize, f: impl FnMut(usize, usize) -> [u8; 3]) -> Signal {
    Signal::from_rgba8(w, h, &rgba8(w, h, f)).unwrap()
}

fn solid(w: usize, h: usize, c: [u8; 3]) -> Signal {
    signal(w, h, |_, _| c)
}

/// 75 % colour bars (code 191): white, yellow, cyan, green, magenta, red, blue, black; 32 px each.
const BAR: u8 = 191;
const BARS: [[u8; 3]; 8] =
    [[BAR, BAR, BAR], [BAR, BAR, 0], [0, BAR, BAR], [0, BAR, 0], [BAR, 0, BAR], [BAR, 0, 0], [0, 0, BAR], [0, 0, 0]];

fn bars(level: u8) -> Signal {
    signal(256, 16, |x, _| BARS[x / 32].map(|c| if c == BAR { level } else { 0 }))
}

fn code(v: u8) -> f32 {
    v as f32 / 255.0
}

/// One column per sample of a 256-wide signal.
fn wide() -> Params {
    Params { columns: 256, ..Params::default() }
}

fn levels(s: &Signal) -> Levels {
    let mut scopes = Scopes::new(Params::default(), None).unwrap();
    scopes.add(0, s)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

// ------------------------------------------------------------------------------------ scopes

#[test]
fn single_colour_lands_in_exact_histogram_bins() {
    let s = solid(64, 36, [200, 100, 50]);
    let n = 64 * 36;
    let h = histogram(&s, &Params::default());
    assert_eq!(h.samples, n);
    assert_eq!((h.r[200], h.g[100], h.b[50]), (n, n, n));
    assert_eq!(h.r.iter().sum::<u64>(), n);
    let y = rgb_to_ycbcr(code(200), code(100), code(50), Matrix::Bt709)[0];
    assert_eq!(h.y[(y * 255.0).round() as usize], n);
    assert_eq!((h.below, h.above), ([0; 4], [0; 4]));
}

#[test]
fn ramp_fills_every_bin_and_waveform_row_equals_code_value() {
    let s = signal(256, 10, |x, _| [x as u8; 3]);
    let p = wide();
    let h = histogram(&s, &p);
    for k in 0..256 {
        assert_eq!((h.r[k], h.g[k], h.b[k], h.y[k]), (10, 10, 10, 10), "bin {k}");
    }
    let w = luma_waveform(&s, &p);
    let y = w.trace("Y").unwrap();
    for x in 0..256 {
        assert_eq!(y.rows_in(x), vec![x], "column {x}");
        assert_eq!(y.at(x, x), 10);
    }
    let w = rgb_parade(&s, &p);
    assert_eq!(w.traces.len(), 3);
    for t in &w.traces {
        assert!((0..256).all(|x| t.rows_in(x) == vec![x]), "{}", t.name);
    }
}

#[test]
fn waveform_columns_cover_the_frame_width() {
    // a 1920-wide frame folds onto 480 columns, four samples per column and row
    let s = signal(1920, 2, |x, _| [(x / 4 % 256) as u8; 3]);
    let w = luma_waveform(&s, &Params::default());
    let y = w.trace("Y").unwrap();
    assert_eq!((y.cols, y.rows), (WAVEFORM_COLUMNS, 256));
    assert_eq!(y.rows_in(1), vec![1]);
    assert_eq!(y.at(1, 1), 8);
    assert_eq!(y.total(), 1920 * 2);
}

#[test]
fn colour_bars_parade_levels_and_vectorscope_targets() {
    let s = bars(BAR);
    let p = wide();
    // RGB parade: every bar column holds its channel's code value exactly
    let par = rgb_parade(&s, &p);
    for (ch, name) in ["R", "G", "B"].iter().enumerate() {
        let t = par.trace(name).unwrap();
        for (i, bar) in BARS.iter().enumerate() {
            for x in i * 32..(i + 1) * 32 {
                assert_eq!(t.rows_in(x), vec![bar[ch] as usize], "{name} bar {i}");
                assert_eq!(t.at(x, bar[ch] as usize), 16);
            }
        }
    }
    // vectorscope: each coloured bar on its 75 % target cell, white and black in the centre
    for m in [Matrix::Bt601, Matrix::Bt709, Matrix::Bt2020] {
        let p = Params { matrix: m, ..p };
        let v = vectorscope(&s, &p);
        assert_eq!(v.samples, 256 * 16);
        let bar_samples = 32 * 16;
        for t in targets(m, code(BAR)) {
            let (x, y) = v.cell(t.cb, t.cr).unwrap();
            assert_eq!(v.at(x, y), bar_samples, "{m:?} {}", t.name);
        }
        let (cx, cy) = v.cell(0.0, 0.0).unwrap();
        assert_eq!(v.at(cx, cy), 2 * bar_samples);
        // the densest spots: the centre (two bars), then the six targets
        let pk = peaks(&v, 10, 0.01);
        assert_eq!(pk.len(), 7);
        assert_eq!((pk[0].x, pk[0].y, pk[0].share), (cx, cy, 0.25));
        assert!(pk[1..].iter().all(|k| k.share == 0.125));
    }
}

#[test]
fn vectorscope_positions_of_primaries_are_known() {
    let p = Params::default();
    // BT.709 red: Cb = −0.2126 / 1.8556 = −0.11457, Cr = 0.5
    let v = vectorscope(&solid(8, 8, [255, 0, 0]), &p);
    let pk = peaks(&v, 1, 0.0)[0];
    assert_eq!((pk.x, pk.y), (103, 21));
    assert_eq!((pk.share, v.samples), (1.0, 64));
    assert!((red_angle(Matrix::Bt709) - 102.906).abs() < 0.01, "{}", red_angle(Matrix::Bt709));
    assert!((red_angle(Matrix::Bt601) - 108.645).abs() < 0.01, "{}", red_angle(Matrix::Bt601));
    // the mean chroma of a flat colour is its own
    let [_, cb, cr] = rgb_to_ycbcr(1.0, 0.0, 0.0, Matrix::Bt709);
    let mean = v.mean();
    assert!((mean[0] - cb).abs() < 1e-6 && (mean[1] - cr).abs() < 1e-6);
    // greys sit in the centre for every matrix
    for m in [Matrix::Bt601, Matrix::Bt709, Matrix::Bt2020] {
        let v = vectorscope(&solid(4, 4, [77, 77, 77]), &Params { matrix: m, ..p });
        assert_eq!(v.at(127, 127), 16, "{m:?}");
    }
}

#[test]
fn ycbcr_round_trips_through_rgb() {
    for m in [Matrix::Bt601, Matrix::Bt709, Matrix::Bt2020] {
        for rgb in [[1.0, 0.0, 0.0], [0.2, 0.7, 0.4], [0.0, 0.0, 1.0], [0.5, 0.5, 0.5]] {
            let [y, cb, cr] = rgb_to_ycbcr(rgb[0], rgb[1], rgb[2], m);
            let back = ycbcr_to_rgb(y, cb, cr, m);
            assert!(back.iter().zip(rgb).all(|(a, b)| (a - b).abs() < 1e-5), "{m:?} {rgb:?} {back:?}");
        }
    }
}

#[test]
fn clamp_signal_controls_out_of_range_values() {
    let s = Signal::from_fn(4, 4, |x, _| if x < 2 { [1.2, 0.5, -0.05] } else { [0.5, 0.5, 0.5] });
    let on = Params::default();
    let off = Params { clamp: false, ..on };
    let h = histogram(&s, &on);
    assert_eq!((h.r[255], h.above[0], h.b[0], h.below[2]), (8, 8, 8, 8));
    let h = histogram(&s, &off);
    assert_eq!((h.r[255], h.above[0], h.b[0], h.below[2]), (0, 8, 0, 8));
    // waveform: clamped onto the top row, or plotted above 100 % within −10..110
    let w = rgb_parade(&s, &on);
    assert_eq!(w.trace("R").unwrap().rows_in(0), vec![255]);
    let w = rgb_parade(&s, &off);
    assert_eq!((w.lo, w.hi), (-0.1, 1.1));
    assert!(w.trace("R").unwrap().rows_in(0).is_empty(), "1.2 is outside −10..110");
    let r = w.trace("B").unwrap().rows_in(0);
    assert_eq!(r, vec![row_of(-0.05, -0.1, 1.1, 256).unwrap()]);
}

#[test]
fn nan_and_empty_signals_do_not_panic() {
    let s = Signal::from_fn(3, 3, |x, _| if x == 1 { [f32::NAN; 3] } else { [0.5; 3] });
    let empty = Signal::from_fn(0, 0, |_, _| [0.0; 3]);
    for clamp in [true, false] {
        let p = Params { clamp, ..Params::default() };
        for s in [&s, &empty] {
            let _ = (luma_waveform(s, &p), rgb_parade(s, &p), histogram(s, &p), vectorscope(s, &p));
        }
        let h = histogram(&s, &p);
        assert_eq!((h.r.iter().sum::<u64>(), h.below[0], h.above[0]), (6, 0, 0));
    }
    let mut scopes = Scopes::new(Params::default(), None).unwrap();
    let l = scopes.add(0, &empty);
    assert_eq!((l.samples, l.clipping.black.r, l.saturation.max_chroma), (0, 0.0, None));
    assert!(scopes.summary().channels.is_none());
}

#[test]
fn summaries_report_levels_in_percent() {
    let mut scopes = Scopes::new(Params::default(), None).unwrap();
    scopes.add(0, &solid(8, 8, [200, 100, 50]));
    let st = scopes.summary().channels.unwrap();
    assert_eq!((st.r.mean, st.g.mean, st.b.mean), (78.43, 39.22, 19.61));
    assert_eq!((st.r.min, st.r.max), (78.43, 78.43));
    // a two-level frame: left half black, right half white
    let s = signal(100, 4, |x, _| if x < 50 { [0; 3] } else { [255; 3] });
    let w = luma_waveform(&s, &Params::default());
    let cols = trace_columns(&w, w.trace("Y").unwrap(), 4);
    assert_eq!(cols.len(), 4);
    assert_eq!(cols[0].unwrap(), Level { min: 0.0, max: 0.0, mean: 0.0 });
    assert_eq!(cols[3].unwrap(), Level { min: 100.0, max: 100.0, mean: 100.0 });
    assert_eq!((cols[1].unwrap().max, cols[2].unwrap().min), (0.0, 100.0));
    // the summary's histogram re-bins to 32 bins of fractions and its targets are the 75 % ones
    let mut scopes = Scopes::new(Params::default(), None).unwrap();
    scopes.add(0, &s);
    let sum = scopes.summary();
    assert_eq!(sum.histogram.y.len(), 32);
    assert_eq!((sum.histogram.y[0], sum.histogram.y[31]), (0.5, 0.5));
    assert_eq!(sum.waveform.luma.len(), 8);
    assert_eq!(sum.vectorscope.targets.iter().map(|t| t.name).collect::<Vec<_>>(), ["R", "Mg", "B", "Cy", "G", "Yl"]);
}

// ------------------------------------------------------------------------------------ levels

#[test]
fn black_and_white_frames_clip_completely() {
    let l = levels(&solid(16, 9, [0, 0, 0]));
    let all = Channels { r: 1.0, g: 1.0, b: 1.0, y: 1.0 };
    assert_eq!((l.samples, l.clipping.black, l.clipping.white), (144, all, Channels::default()));
    assert_eq!((l.illegal, l.legal.is_none(), l.float_range.is_none()), (None, true, true));
    assert_eq!((l.saturation.out_of_gamut, l.saturation.above_limit, l.saturation.max_chroma), (0.0, 0.0, Some(0.0)));
    let l = levels(&solid(16, 9, [255, 255, 255]));
    assert_eq!((l.clipping.black, l.clipping.white), (Channels::default(), all));
}

#[test]
fn a_ramp_clips_one_code_value_at_each_end() {
    let l = levels(&signal(256, 4, |x, _| [x as u8; 3]));
    for v in [l.clipping.black, l.clipping.white] {
        assert_eq!(v, Channels { r: 1.0 / 256.0, g: 1.0 / 256.0, b: 1.0 / 256.0, y: 1.0 / 256.0 });
    }
}

#[test]
fn bars_above_the_75_percent_targets_are_oversaturated() {
    let limit = default_chroma_limit(Matrix::Bt709);
    assert!((limit - 0.44675).abs() < 1e-4, "{limit}");
    let l = levels(&bars(BAR));
    assert_eq!((l.saturation.above_limit, l.saturation.out_of_gamut), (0.0, 0.0));
    assert!(close(l.saturation.chroma_limit, limit as f64));
    let l = levels(&bars(255));
    assert_eq!((l.saturation.above_limit, l.saturation.out_of_gamut), (0.75, 0.0));
    // magenta and green at 100 % are the farthest from the centre
    let mg = targets(Matrix::Bt709, 1.0)[1];
    assert!(close(l.saturation.max_chroma.unwrap(), (mg.cb.hypot(mg.cr)) as f64));
    // a caller's limit replaces the default
    let mut scopes = Scopes::new(Params::default(), Some(0.6)).unwrap();
    assert_eq!(scopes.add(0, &bars(255)).saturation.above_limit, 0.0);
}

#[test]
fn float_values_outside_zero_to_one_are_illegal() {
    let s = Signal::from_fn(4, 1, |x, _| match x {
        0 => [-0.1, 0.5, 0.5],
        1 => [0.5, 1.2, 0.5],
        2 => [f32::NAN, 0.5, 0.5],
        _ => [0.5, 0.5, 0.5],
    });
    let l = levels(&s);
    let f = l.float_range.unwrap();
    assert_eq!(
        (f.below, f.above, f.non_finite),
        (Rgb { r: 0.25, g: 0.0, b: 0.0 }, Rgb { r: 0.0, g: 0.25, b: 0.0 }, 0.25)
    );
    assert_eq!(l.illegal, Some(0.75));
    assert_eq!(l.saturation.out_of_gamut, 0.75);
    // beyond 0 and 1 counts as clipped: it clips on delivery
    assert_eq!((l.clipping.black.r, l.clipping.white.g), (0.25, 0.25));
    // inside the EBU R 103 tolerance (−5 %..105 %) a float value is illegal but not out of gamut
    let l = levels(&Signal::from_fn(2, 1, |x, _| if x == 0 { [1.04, 0.5, 0.5] } else { [0.5; 3] }));
    assert_eq!((l.illegal, l.saturation.out_of_gamut), (Some(0.5), 0.0));
}

#[test]
fn limited_range_8_bit_codes_outside_16_235_are_illegal() {
    let s = Signal::from_ycbcr(256, 1, 8, false, Matrix::Bt709, |x, _| [x as u16, 128, 128]).unwrap();
    let l = levels(&s);
    let legal = l.legal.unwrap();
    assert_eq!((legal.depth, legal.y, legal.c), (8, [16, 235], [16, 240]));
    assert_eq!((legal.below.y, legal.above.y), (16.0 / 256.0, 20.0 / 256.0));
    assert_eq!((legal.below.cb, legal.above.cr), (0.0, 0.0));
    assert_eq!(l.illegal, Some(36.0 / 256.0));
    // black is code 16 and below, white 235 and above
    assert_eq!((l.clipping.black.y, l.clipping.white.y), (17.0 / 256.0, 21.0 / 256.0));
    // below −5 % (codes 0..=5) and above 105 % (246..=255) are out of gamut
    assert_eq!(l.saturation.out_of_gamut, 16.0 / 256.0);
    assert!(l.float_range.is_none());
}

#[test]
fn limited_range_10_bit_codes_outside_64_940_are_illegal() {
    let codes = [[64, 512, 512], [940, 512, 512], [63, 512, 512], [941, 512, 512]];
    let s = Signal::from_ycbcr(4, 1, 10, false, Matrix::Bt709, |x, _| codes[x]).unwrap();
    let l = levels(&s);
    let legal = l.legal.unwrap();
    assert_eq!((legal.depth, legal.y, legal.c), (10, [64, 940], [64, 960]));
    assert_eq!((legal.below.y, legal.above.y, l.illegal), (0.25, 0.25, Some(0.5)));
    assert_eq!((l.clipping.black.y, l.clipping.white.y), (0.5, 0.5));
    let chroma = Signal::from_ycbcr(2, 1, 10, false, Matrix::Bt709, |x, _| [512, 63 + 898 * x as u16, 512]).unwrap();
    let legal = levels(&chroma).legal.unwrap();
    assert_eq!((legal.below.cb, legal.above.cb), (0.5, 0.5));
}

#[test]
fn full_range_codes_have_no_legal_range() {
    let s = Signal::from_ycbcr(2, 1, 8, true, Matrix::Bt709, |x, _| [255 * x as u16, 128, 128]).unwrap();
    let l = levels(&s);
    assert_eq!((l.legal.is_none(), l.illegal), (true, None));
    assert_eq!((l.clipping.black.y, l.clipping.white.y), (0.5, 0.5));
    assert!(matches!(
        Signal::from_ycbcr(1, 1, 7, false, Matrix::Bt709, |_, _| [16, 128, 128]),
        Err(ScopeError::Invalid(_))
    ));
}

#[test]
fn legal_codes_can_still_leave_the_gamut() {
    // Y black with Cr at its maximum is a legal code triple and no displayable colour
    let s = Signal::from_ycbcr(1, 1, 8, false, Matrix::Bt709, |_, _| [16, 128, 240]).unwrap();
    let l = levels(&s);
    assert_eq!((l.illegal, l.saturation.out_of_gamut, l.saturation.above_limit), (Some(0.0), 1.0, 1.0));
}

// ------------------------------------------------------------------------------------ frames

fn frame(layout: PixelLayout, planes: Vec<Vec<u8>>, w: u32, h: u32) -> VideoFrame {
    let sizes = layout.planes(w, h);
    let planes =
        planes.into_iter().zip(sizes).map(|(data, (width, height, _))| Plane { width, height, data }).collect();
    VideoFrame { index: 0, width: w, height: h, layout, planes }
}

fn info(depth: u32, full_range: bool) -> VideoInfo {
    VideoInfo { width: 2, height: 2, depth, full_range, ..VideoInfo::default() }
}

#[test]
fn decoded_frames_keep_their_code_values() {
    // 4:2:0, 8 bits: one chroma sample covers the 2 × 2 block
    let f = frame(PixelLayout::Yuv8(1, 1), vec![vec![16, 235, 16, 235], vec![128], vec![128]], 2, 2);
    let s = Signal::from_frame(&f, &info(8, false), Matrix::Bt709).unwrap();
    assert_eq!(s.codes.as_ref().unwrap().ycc, vec![[16, 128, 128], [235, 128, 128], [16, 128, 128], [235, 128, 128]]);
    assert!(s.rgb.iter().all(|c| c.iter().all(|v| (v - c[0]).abs() < 1e-6)));
    assert_eq!((s.rgb[0][0], s.rgb[1][0]), (0.0, 1.0));
    // 10 bits arrive on the full 16-bit scale
    let le = |v: u16| (v << 6).to_le_bytes().to_vec();
    let y = [le(64), le(940), le(63), le(941)].concat();
    let f = frame(PixelLayout::Yuv16(1, 1), vec![y, le(512), le(512)], 2, 2);
    let s = Signal::from_frame(&f, &info(10, false), Matrix::Bt709).unwrap();
    assert_eq!(s.codes.as_ref().unwrap().ycc.iter().map(|c| c[0]).collect::<Vec<_>>(), [64, 940, 63, 941]);
    assert_eq!(levels(&s).illegal, Some(0.5));
    // RGBA layouts
    let f = frame(PixelLayout::Rgba8, vec![vec![255, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255]], 2, 2);
    let s = Signal::from_frame(&f, &info(8, true), Matrix::Bt709).unwrap();
    assert_eq!((s.rgb[0], s.codes.is_none()), ([1.0, 0.0, 0.0], true));
    let f = frame(
        PixelLayout::Rgba16,
        vec![[65535u16, 0, 0, 65535].repeat(4).iter().flat_map(|v| v.to_le_bytes()).collect()],
        2,
        2,
    );
    let s = Signal::from_frame(&f, &info(16, true), Matrix::Bt709).unwrap();
    assert_eq!(s.rgb, vec![[1.0, 0.0, 0.0]; 4]);
}

#[test]
fn a_frame_whose_planes_do_not_match_its_layout_is_rejected() {
    let f = frame(PixelLayout::Yuv8(1, 1), vec![vec![16, 235, 16], vec![128], vec![128]], 2, 2);
    assert!(matches!(Signal::from_frame(&f, &info(8, false), Matrix::Bt709), Err(ScopeError::Frame(_))));
    let f = frame(PixelLayout::Yuv8(1, 1), vec![vec![16; 4], vec![128]], 2, 2);
    assert!(matches!(Signal::from_frame(&f, &info(8, false), Matrix::Bt709), Err(ScopeError::Frame(_))));
    assert!(matches!(Signal::from_rgba8(2, 2, &[0; 12]), Err(ScopeError::Frame(_))));
    assert!(matches!(Signal::from_rgba_f32(2, 1, &[0.0; 4]), Err(ScopeError::Frame(_))));
}

#[test]
fn matrix_follows_the_stream_tag_or_the_frame_height() {
    assert_eq!(Matrix::from_tag("bt709", 480), (Matrix::Bt709, true));
    assert_eq!(Matrix::from_tag("smpte170m", 1080), (Matrix::Bt601, true));
    assert_eq!(Matrix::from_tag("bt470bg", 1080), (Matrix::Bt601, true));
    assert_eq!(Matrix::from_tag("bt2020nc", 480), (Matrix::Bt2020, true));
    assert_eq!(Matrix::from_tag("", 720), (Matrix::Bt709, false));
    assert_eq!(Matrix::from_tag("unknown", 576), (Matrix::Bt601, false));
    assert_eq!(serde_json::to_value(Matrix::Bt2020).unwrap(), serde_json::json!("BT.2020"));
}

#[test]
fn the_worst_frame_of_each_measure_is_kept() {
    let mut scopes = Scopes::new(Params::default(), None).unwrap();
    scopes.add(0, &solid(4, 4, [0, 0, 0]));
    scopes.add(5, &solid(4, 4, [255, 255, 255]));
    scopes.add(9, &solid(4, 4, [255, 255, 255]));
    let w = scopes.worst();
    assert_eq!(w.black_clipped, Some(Worst { frame: 0, value: 1.0 }));
    assert_eq!(w.white_clipped, Some(Worst { frame: 5, value: 1.0 }), "a tie keeps the first frame");
    assert_eq!(w.out_of_gamut, Some(Worst { frame: 0, value: 0.0 }));
    assert_eq!(w.illegal, None);
    assert_eq!((scopes.frames(), scopes.levels().clipping.black.r, scopes.levels().samples), (3, 1.0 / 3.0, 48));
    assert!(matches!(Scopes::new(Params::default(), Some(0.0)), Err(ScopeError::Invalid(_))));
    assert!(matches!(Scopes::new(Params::default(), Some(f32::NAN)), Err(ScopeError::Invalid(_))));
}

// ------------------------------------------------------------------------------------ files

#[test]
fn a_still_image_file_is_analysed_as_one_frame() {
    let dir = tempfile::tempdir().unwrap();
    let png = dir.path().join("half.png");
    let img =
        image::RgbImage::from_fn(4, 2, |x, _| if x < 2 { image::Rgb([0, 0, 0]) } else { image::Rgb([255, 0, 0]) });
    img.save(&png).unwrap();
    let text = dir.path().join("notes.txt");
    std::fs::write(&text, "not an image").unwrap();
    assert!(crate::still::is_still(&png));
    assert!(!crate::still::is_still(&text));
    assert!(!crate::still::is_still(&dir.path().join("missing.png")));

    let r = analyse(&png, 1, None).unwrap();
    assert_eq!(
        (r.schema, r.source.kind, r.source.matrix, r.source.matrix_tagged),
        (SCHEMA, "still", Matrix::Bt709, false)
    );
    assert_eq!((r.source.width, r.source.height, r.source.depth, r.frames.analysed), (4, 2, 8, 1));
    assert_eq!((r.levels.clipping.black.g, r.levels.clipping.white.r, r.levels.illegal), (1.0, 0.5, None));
    assert_eq!(r.worst.white_clipped, Some(Worst { frame: 0, value: 0.5 }));
    let json = serde_json::to_value(&r).unwrap();
    assert_eq!(json["schema"], SCHEMA);
    assert_eq!(json["levels"]["clipping"]["black"]["g"], 1.0);
    assert!(r.human().contains("clipped"));
}

#[test]
fn analysis_rejects_bad_settings_and_missing_files() {
    let dir = tempfile::tempdir().unwrap();
    let png = dir.path().join("grey.png");
    image::RgbImage::from_pixel(2, 2, image::Rgb([128, 128, 128])).save(&png).unwrap();
    assert!(matches!(analyse(&png, 0, None), Err(ScopeError::Invalid(_))));
    assert!(matches!(analyse(&png, 1, Some(-1.0)), Err(ScopeError::Invalid(_))));
    assert!(matches!(analyse(&dir.path().join("missing.mp4"), 1, None), Err(ScopeError::Open { .. })));
}
