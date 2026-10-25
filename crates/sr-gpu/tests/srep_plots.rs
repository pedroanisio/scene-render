//! The plots draft (sr-core srep/srep-0000-plots.md) at the renderer: an explicit plot draws its curve in the
//! box, a child is clipped to the box, and the tick labels are typeset in the axes' stroke colour.

mod common;
use common::*;

/// The centre and extent of the pixels of a class: (cx, cy, w, h), or None under 12 pixels.
fn measure(r: &Rendered, class: impl Fn([f32; 4]) -> bool, [x0, y0, x1, y1]: [u32; 4]) -> Option<[f64; 4]> {
    let (mut n, mut sx, mut sy) = (0usize, 0f64, 0f64);
    let (mut a, mut b, mut c, mut d) = (u32::MAX, u32::MAX, 0, 0);
    for y in y0..y1 {
        for x in x0..x1 {
            if class(r.at(x, y)) {
                n += 1;
                sx += x as f64;
                sy += y as f64;
                (a, b, c, d) = (a.min(x), b.min(y), c.max(x), d.max(y));
            }
        }
    }
    (n >= 12).then(|| [sx / n as f64 + 0.5, sy / n as f64 + 0.5, (c - a + 1) as f64, (d - b + 1) as f64])
}

fn red(p: [f32; 4]) -> bool {
    p[0] > 0.5 && p[1] < 0.3 && p[2] < 0.3
}

fn green(p: [f32; 4]) -> bool {
    p[1] > 0.5 && p[0] < 0.3 && p[2] < 0.3
}

fn blue(p: [f32; 4]) -> bool {
    p[2] > 0.5 && p[0] < 0.3 && p[1] < 0.3
}

/// The draft's kit shot "axes-sine": −π..π × −1.5..1.5 on a 500 × 300 box centred in the frame, a red sine, here
/// with a 2 px stroke; green axes; a blue 40 px square at data (3, 0) that the box cuts.
const SINE: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<scene version="1.8">
<project width="640" height="360" fps="24" duration="1" background="#000000FF" seed="1"/>
<output id="still" path="out/frame_%04d.png" codec="png-sequence"/>
<composition>
<axes id="ax" x="70" y="30" width="500" height="300" xMin="-3.141592653589793" xMax="3.141592653589793" yMin="-1.5" yMax="1.5" stroke="#00FF00FF">
<plot kind="explicit" y="Math.sin(x)" stroke="#FF0000FF" strokeWidth="2"/>
<shape id="sq" shape="rect" x="3" y="0" width="40" height="40" fill="#0000FFFF"/>
</axes>
</composition>
</scene>"##;

#[test]
fn a_sine_on_axes_draws_in_its_box_with_labels() {
    let d = sr_model::load_str(SINE, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}"));
    let Some(r) = render(&d) else { return };
    assert!(
        r.stats.errors.is_empty() && r.stats.unsupported.is_empty(),
        "{:?} {:?}",
        r.stats.errors,
        r.stats.unsupported
    );
    // the curve: centred on the box, its full width, the 200 px between the peaks plus the stroke
    let got = measure(&r, red, [0, 0, 640, 360]).expect("no curve drawn");
    let want = [320.0, 180.0, 500.0, 202.0];
    assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 2.0), "curve {got:?}, expected {want:?}");
    // the square starts at data x = 3, pixel 70 + 488.7, and the box cuts it at its right edge, 570
    let sq = measure(&r, blue, [0, 0, 640, 360]).expect("no square drawn");
    assert!((sq[2] - 11.3).abs() <= 1.5 && (sq[3] - 40.0).abs() <= 1.0, "square {sq:?}");
    // the label "1" hangs below the x axis' tick at data 1, pixel 70 + 329.6, from 30 + 150 + 3 + 4 down
    let one = measure(&r, green, [385, 187, 415, 210]).expect("no label under the tick at 1");
    assert!((one[0] - 399.6).abs() <= 2.0, "label {one:?}");
    // nothing above the axes' box is drawn: the labels of the x axis are under it
    assert!(measure(&r, |p| p[0] + p[1] + p[2] > 0.3, [0, 0, 640, 25]).is_none());
}
