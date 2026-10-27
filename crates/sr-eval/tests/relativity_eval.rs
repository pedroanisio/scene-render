//! The relativity draft at the evaluator (sr-core srep/srep-0000-relativity.md, Semantics 1 and 5): a `spacetime`
//! in an `axes` lowers to path shapes for its boosted grid, light cones, worldlines, light rays and event points,
//! every event mapped by `diagram_pixel`; a keyed `boost` interpolates in rapidity, so the frame's boost is
//! `frame_beta` and a frame evaluated alone equals the frame played to.

use sr_3d::relativity::{self as rel, AxesBox, Compact};
use sr_eval::{Evaluator, FrameGraph, FrameNode};

fn evaluator(spacetime: &str) -> Evaluator {
    let xml = format!(
        r##"<scene version="1.8"><project width="640" height="360" fps="24" duration="10" background="#000000FF" seed="1"/>
  <composition><axes {AX} xMin="-2" xMax="2" yMin="-1.5" yMax="1.5">{spacetime}</axes></composition></scene>"##
    );
    let doc =
        sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}\n{xml}"));
    Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"))
}

const AX: &str = r#"id="ax" x="100" y="50" width="400" height="300""#;

/// The box of the draft's last numeric test, in the box's own pixel space (the parts draw there).
const BOX: AxesBox =
    AxesBox { x: 0.0, y: 0.0, width: 400.0, height: 300.0, x_min: -2.0, x_max: 2.0, y_min: -1.5, y_max: 1.5 };

fn frame(ev: &Evaluator, t: f64) -> FrameGraph {
    let g = ev.evaluate(t);
    assert!(g.failures.is_empty() && g.problems.is_empty(), "{:?} {:?}", g.failures, g.problems);
    g
}

fn node<'a>(g: &'a FrameGraph, id: &str) -> &'a FrameNode {
    g.nodes
        .iter()
        .find(|n| &*n.id == id)
        .unwrap_or_else(|| panic!("no node {id}: {:?}", g.nodes.iter().map(|n| &*n.id).collect::<Vec<_>>()))
}

fn path_of(n: &FrameNode) -> String {
    match n.props.get("path") {
        Some(sr_eval::value::Value::Str(s)) => s.to_string(),
        other => panic!("{}: no path property: {other:?}", n.id),
    }
}

/// The points of M/L/Z path data, one list per subpath, and whether each is closed.
fn subpaths(d: &str) -> Vec<(Vec<[f64; 2]>, bool)> {
    let mut out: Vec<(Vec<[f64; 2]>, bool)> = Vec::new();
    let toks: Vec<&str> = d.split_whitespace().collect();
    let mut k = 0;
    while k < toks.len() {
        match toks[k] {
            "M" | "L" => {
                let p = [toks[k + 1].parse().unwrap(), toks[k + 2].parse().unwrap()];
                if toks[k] == "M" {
                    out.push((vec![p], false));
                } else {
                    out.last_mut().unwrap().0.push(p);
                }
                k += 3;
            }
            "Z" => {
                out.last_mut().unwrap().1 = true;
                k += 1;
            }
            t => panic!("unexpected token {t:?} in {d}"),
        }
    }
    out
}

/// The centre of an event's point: the mean of its circle's vertices.
fn centre(g: &FrameGraph, id: &str) -> [f64; 2] {
    let sp = subpaths(&path_of(node(g, id)));
    assert_eq!(sp.len(), 1, "{sp:?}");
    let (pts, closed) = &sp[0];
    assert!(closed);
    let n = pts.len() as f64;
    [pts.iter().map(|p| p[0]).sum::<f64>() / n, pts.iter().map(|p| p[1]).sum::<f64>() / n]
}

fn pixel(x: f64, t: f64, beta: f64) -> [f64; 2] {
    let (px, py) = rel::diagram_pixel(x, t, &BOX, beta, Compact::None).unwrap();
    [px, py]
}

fn near(a: [f64; 2], b: [f64; 2], tol: f64) -> bool {
    (a[0] - b[0]).abs() <= tol && (a[1] - b[1]).abs() <= tol
}

const BOOSTED: &str = r#"<spacetime><animate property="boost"><key time="0" value="0"/><key time="10" value="0.8"/></animate><event x="0" t="0"/><event x="0.3" t="0.5"/></spacetime>"#;

#[test]
fn an_event_is_drawn_at_its_boosted_pixel() {
    let ev = evaluator(r#"<spacetime boost="0.6"><event x="1" t="0" markerSize="6"/></spacetime>"#);
    let g = frame(&ev, 0.0);
    // the draft's last numeric test: (1, 0) at 0.6 is pixel (425, 275) of the box at (100, 50)
    assert!(near(centre(&g, "ax/event0"), [325.0, 225.0], 1e-9), "{:?}", centre(&g, "ax/event0"));
    assert_eq!(node(&g, "ax/event0").kind, "shape");
}

#[test]
fn a_keyed_boost_interpolates_in_rapidity_at_every_frame() {
    let ev = evaluator(BOOSTED);
    for f in 0..=240u64 {
        let beta = rel::frame_beta(f, 24.0, 0.0, 0.8, 0.0, 10.0).unwrap();
        let g = frame(&ev, f as f64 / 24.0);
        let (o, e) = (centre(&g, "ax/event0"), centre(&g, "ax/event1"));
        assert!(near(e, pixel(0.3, 0.5, beta), 1e-9), "frame {f}: {e:?}");
        // the pixel separation maps back to the interval 0.16 (100 px per unit on both axes)
        let (dx, dt) = ((e[0] - o[0]) / 100.0, (o[1] - e[1]) / 100.0);
        assert!((dt * dt - dx * dx - 0.16).abs() < 1e-9, "frame {f}: {}", dt * dt - dx * dx);
    }
    // frame 120 of 0 to 0.8 over 10 s stands at 0.5
    let g = frame(&ev, 5.0);
    assert!(near(centre(&g, "ax/event1"), pixel(0.3, 0.5, 0.5), 1e-9));
}

#[test]
fn a_frame_alone_equals_the_frame_played_to() {
    let played = evaluator(BOOSTED);
    let ahead: Vec<String> =
        (0..=240u64).map(|f| path_of(node(&frame(&played, f as f64 / 24.0), "ax/event1"))).collect();
    for f in [240u64, 0, 121, 17, 200] {
        let alone = evaluator(BOOSTED);
        assert_eq!(path_of(node(&frame(&alone, f as f64 / 24.0), "ax/event1")), ahead[f as usize], "frame {f}");
    }
}

#[test]
fn an_expression_gives_the_boost_itself() {
    let ev = evaluator(
        r#"<spacetime><expression property="boost">0.06 * time</expression><event x="1" t="0"/></spacetime>"#,
    );
    let g = frame(&ev, 10.0);
    assert!(near(centre(&g, "ax/event0"), pixel(1.0, 0.0, 0.6), 1e-9));
}

#[test]
fn light_cones_keep_their_45_degree_edges_under_a_boost() {
    let ev =
        evaluator(r#"<spacetime boost="0.9"><event x="0.2" t="-0.1" lightCone="true" extent="0.15"/></spacetime>"#);
    let g = frame(&ev, 0.0);
    for (id, half) in [("ax/cone0Future", "future"), ("ax/cone0Past", "past")] {
        let sp = subpaths(&path_of(node(&g, id)));
        assert_eq!(sp.len(), 1, "{id}: {sp:?}");
        let (pts, closed) = &sp[0];
        assert!(*closed && pts.len() == 3, "{id} {half}: {pts:?}");
        let apex = pts[0];
        assert!(near(apex, pixel(0.2, -0.1, 0.9), 1e-9), "{id}: {apex:?}");
        for p in &pts[1..] {
            let angle = ((p[1] - apex[1]) / (p[0] - apex[0])).atan().to_degrees().abs();
            assert!((angle - 45.0).abs() < 1e-9, "{id}: {angle}");
        }
    }
    // the fills are the event's future and past paints, the defaults #FFCC0040 and #4080FF40
    let fill = |id: &str| {
        use sr_model::element::Element;
        let n = ev.program().nodes.iter().find(|n| &*n.id == id).unwrap_or_else(|| panic!("no node {id}"));
        n.elem.get_attr("fill").map(|v| v.to_string().to_uppercase())
    };
    assert_eq!(fill("ax/cone0Future").as_deref(), Some("#FFCC0040"));
    assert_eq!(fill("ax/cone0Past").as_deref(), Some("#4080FF40"));
}

/// Every vertex of a part, which lies inside the box, is within `tol` of a point of `curve` sampled densely.
fn on_curve(pts: &[[f64; 2]], curve: &[[f64; 2]], tol: f64) -> bool {
    pts.iter().all(|p| {
        curve.windows(2).any(|w| {
            let (a, b) = (w[0], w[1]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let l2 = dx * dx + dy * dy;
            let u = if l2 == 0.0 { 0.0 } else { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0) };
            let q = [a[0] + u * dx, a[1] + u * dy];
            near(*p, q, tol)
        })
    })
}

#[test]
fn worldlines_follow_the_reference_events() {
    let ev = evaluator(
        r#"<spacetime boost="0.3"><worldline kind="inertial" velocity="0.5" x0="0.1" t0="0.2"/><worldline kind="accelerated" acceleration="2" x0="-0.5" tau0="-0.6" tau1="0.6" samples="64"/><worldline kind="parametric" x="0.3 * Math.sin(tau)" t="tau" tau0="-1" tau1="1" samples="128"/></spacetime>"#,
    );
    let g = frame(&ev, 0.0);
    let reference = |f: &dyn Fn(f64) -> (f64, f64), lo: f64, hi: f64| -> Vec<[f64; 2]> {
        (0..=4000).map(|i| f(lo + (hi - lo) * i as f64 / 4000.0)).map(|(x, t)| pixel(x, t, 0.3)).collect()
    };
    let inertial = reference(&|t| rel::worldline_inertial(0.1, 0.2, 0.5, t).unwrap(), -10.0, 10.0);
    let accelerated = reference(
        &|tau| {
            let (x, t) = rel::worldline_accelerated(2.0, tau).unwrap();
            (x - 0.5, t)
        },
        -0.6,
        0.6,
    );
    let parametric = reference(&|tau| (0.3 * tau.sin(), tau), -1.0, 1.0);
    for (id, curve) in [("ax/worldline0", inertial), ("ax/worldline1", accelerated), ("ax/worldline2", parametric)] {
        let sp = subpaths(&path_of(node(&g, id)));
        assert!(!sp.is_empty(), "{id} draws nothing");
        for (pts, closed) in &sp {
            assert!(!closed);
            assert!(pts
                .iter()
                .all(|p| (-1e-9..=400.0 + 1e-9).contains(&p[0]) && (-1e-9..=300.0 + 1e-9).contains(&p[1])));
            assert!(on_curve(pts, &curve, 0.05), "{id}: {pts:?}");
        }
    }
    // the accelerated line's samples are its own: 64 vertices from τ = −0.6 to 0.6, all inside the box
    let acc = subpaths(&path_of(node(&g, "ax/worldline1")));
    assert_eq!(acc.len(), 1);
    assert_eq!(acc[0].0.len(), 64);
}

#[test]
fn a_parametric_worldline_that_is_not_timelike_is_a_problem() {
    let ev = evaluator(r#"<spacetime><worldline kind="parametric" x="2 * tau" t="tau"/></spacetime>"#);
    let g = ev.evaluate(0.0);
    assert!(g.problems.iter().any(|p| p.contains("worldline0") && p.contains("timelike")), "{:?}", g.problems);
}

#[test]
fn a_light_ray_runs_at_45_degrees_from_its_event() {
    let ev = evaluator(r#"<spacetime boost="-0.7"><lightRay x="0.5" t="-1" direction="-1" length="1"/></spacetime>"#);
    let g = frame(&ev, 0.0);
    let sp = subpaths(&path_of(node(&g, "ax/lightRay0")));
    assert_eq!(sp.len(), 1);
    let pts = &sp[0].0;
    assert!(near(pts[0], pixel(0.5, -1.0, -0.7), 1e-9));
    assert!(near(*pts.last().unwrap(), pixel(-0.5, 0.0, -0.7), 1e-9));
    let angle = ((pts[1][1] - pts[0][1]) / (pts[1][0] - pts[0][0])).atan().to_degrees();
    assert!((angle - 45.0).abs() < 1e-9, "{angle}");
}

#[test]
fn the_grid_draws_the_box_frame_s_lines_where_the_boost_carries_them() {
    // at rest the lines x = k and t = k at the tick step (0.5) are the axes' own: 9 vertical and 7 horizontal
    let g = frame(&evaluator(r#"<spacetime grid="true"/>"#), 0.0);
    let sp = subpaths(&path_of(node(&g, "ax/spacetimeGrid")));
    assert_eq!(sp.len(), 16, "{sp:?}");
    let vertical = sp.iter().filter(|(p, _)| p.iter().all(|q| (q[0] - p[0][0]).abs() < 1e-9)).count();
    let horizontal = sp.iter().filter(|(p, _)| p.iter().all(|q| (q[1] - p[0][1]).abs() < 1e-9)).count();
    assert_eq!((vertical, horizontal), (9, 7));
    // under a boost of 0.6 they shear: the box frame's events on x = k and t = k are drawn on them
    let g = frame(&evaluator(r#"<spacetime boost="0.6" grid="true"/>"#), 0.0);
    let sp = subpaths(&path_of(node(&g, "ax/spacetimeGrid")));
    let on_some = |p: [f64; 2]| sp.iter().any(|(pts, _)| on_curve(&[p], pts, 1e-6));
    for k in [-1.0, -0.5, 0.0, 0.5, 1.0] {
        for s in [-0.2, 0.0, 0.3] {
            assert!(on_some(pixel(k, s, 0.6)), "x = {k} at t = {s}");
            assert!(on_some(pixel(s, k, 0.6)), "t = {k} at x = {s}");
        }
    }
    // and no line is axis-aligned any more
    assert!(sp
        .iter()
        .all(|(p, _)| p.len() >= 2 && (p[1][0] - p[0][0]).abs() > 1e-6 && (p[1][1] - p[0][1]).abs() > 1e-6));
}

#[test]
fn c_scales_time_and_divides_the_y_labels() {
    let ev = evaluator(r#"<spacetime c="2"><event x="0" t="0.5"/></spacetime>"#);
    let g = frame(&ev, 0.0);
    // t = 0.5 at c = 2 is ct = 1
    assert!(near(centre(&g, "ax/event0"), pixel(0.0, 1.0, 0.0), 1e-9));
    let f = node(&g, "ax").axes.clone().expect("the axes' frame");
    let y: Vec<&str> =
        f.labels.iter().filter(|l| l.align == sr_eval::axes::Align::Left).map(|l| l.tex.as_str()).collect();
    // ticks at ct = −1.5, −1, −0.5, 0.5, 1, 1.5 read t = ct / 2
    assert_eq!(y, ["-0.75", "-0.5", "-0.25", "0.25", "0.5", "0.75"]);
}

#[test]
fn event_and_worldline_labels_join_the_axes_labels() {
    let ev = evaluator(
        r#"<spacetime><event x="1" t="0" label="P"/><worldline kind="inertial" velocity="0.5" label="ship"/></spacetime>"#,
    );
    let g = frame(&ev, 0.0);
    let f = node(&g, "ax").axes.clone().expect("the axes' frame");
    let texts: Vec<&str> = f.labels.iter().map(|l| l.tex.as_str()).collect();
    assert!(texts.contains(&"\\text{P}") && texts.contains(&"\\text{ship}"), "{texts:?}");
}

#[test]
fn a_boost_key_at_the_speed_of_light_is_an_e04() {
    let xml = r##"<scene version="1.8"><project width="64" height="64" fps="24" duration="1" background="#000000FF"/><composition><axes id="ax" width="40" height="30"><spacetime><animate property="boost"><key time="0" value="0"/><key time="1" value="1"/></animate></spacetime></axes></composition></scene>"##;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let err = match Evaluator::new(&doc, &Default::default()) {
        Err(e) => e,
        Ok(_) => panic!("refused"),
    };
    assert!(err.to_string().contains("E04") && err.to_string().contains("boost"), "{err}");
    let additive = xml
        .replace(r#"property="boost""#, r#"property="boost" additive="true""#)
        .replace(r#"value="1""#, r#"value="0.5""#);
    let doc = sr_model::load_str(&additive, &sr_model::LoadOptions::without_assets()).unwrap();
    let err = match Evaluator::new(&doc, &Default::default()) {
        Err(e) => e,
        Ok(_) => panic!("refused"),
    };
    assert!(err.to_string().contains("E04") && err.to_string().contains("rapidity"), "{err}");
}

#[test]
fn the_minkowski_diamond_compactifies_every_part() {
    let ev = evaluator(
        r#"<spacetime boost="0.3" compact="minkowski" grid="true"><event x="0.5" t="0.25"/><worldline kind="inertial" velocity="0"/></spacetime>"#,
    );
    let g = frame(&ev, 0.0);
    let (px, py) = rel::diagram_pixel(0.5, 0.25, &BOX, 0.3, Compact::Minkowski).unwrap();
    assert!(near(centre(&g, "ax/event0"), [px, py], 1e-9));
    // the line x = 0 of the rest frame, boosted and compactified, sampled densely
    let curve: Vec<[f64; 2]> = (-4000..=4000)
        .map(|i| {
            let t = (i as f64 / 4000.0 * (std::f64::consts::FRAC_PI_2 - 1e-4)).tan();
            let (px, py) = rel::diagram_pixel(0.0, t, &BOX, 0.3, Compact::Minkowski).unwrap();
            [px, py]
        })
        .collect();
    let line = subpaths(&path_of(node(&g, "ax/worldline0")));
    assert!(!line.is_empty());
    for (pts, _) in &line {
        // a straight line bends: many vertices, every one on the compactified curve
        assert!(pts.len() > 100, "{}", pts.len());
        assert!(on_curve(pts, &curve, 0.05));
    }
    // the grid's lines are curves inside the box, through the compactified events of the box frame's lines
    let grid = subpaths(&path_of(node(&g, "ax/spacetimeGrid")));
    assert!(!grid.is_empty());
    assert!(grid.iter().flat_map(|(p, _)| p).all(|q| (0.0..=400.0).contains(&q[0]) && (0.0..=300.0).contains(&q[1])));
    let (gx, gy) = rel::diagram_pixel(0.5, 0.1, &BOX, 0.3, Compact::Minkowski).unwrap();
    assert!(grid.iter().any(|(pts, _)| on_curve(&[[gx, gy]], pts, 0.05)), "x = 0.5 at t = 0.1");
}

#[test]
fn the_schwarzschild_diagram_takes_r_s_from_its_black_hole() {
    let xml = r##"<scene version="1.8"><project width="640" height="360" fps="24" duration="1" background="#000000FF"/><composition><blackHole id="h" mass="1"/><axes id="ax" width="400" height="300" xMin="-2" xMax="2" yMin="-1.5" yMax="1.5"><spacetime compact="schwarzschild" blackHole="h"><event x="3" t="0.5"/></spacetime></axes></composition></scene>"##;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
    let ev = Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"));
    let g = frame(&ev, 0.0);
    // r_s = 2 mass = 2
    let (px, py) = rel::diagram_pixel(3.0, 0.5, &BOX, 0.0, Compact::Schwarzschild(2.0)).unwrap();
    assert!(near(centre(&g, "ax/event0"), [px, py], 1e-9), "{:?} {:?}", centre(&g, "ax/event0"), [px, py]);
}

#[test]
fn a_label_with_unpaired_braces_is_a_problem() {
    let ev = evaluator(r#"<spacetime><event x="0" t="0" label="P}"/></spacetime>"#);
    let g = ev.evaluate(0.0);
    assert!(g.problems.iter().any(|p| p.contains("unpaired braces")), "{:?}", g.problems);
    // a label whose braces pair is taken as written
    let g = frame(&evaluator(r#"<spacetime><event x="0" t="0" label="{P}"/></spacetime>"#), 0.0);
    let f = node(&g, "ax").axes.clone().unwrap();
    assert!(f.labels.iter().any(|l| l.tex == "\\text{{P}}"));
}

#[test]
fn an_animated_grid_appears_at_its_key() {
    let ev = evaluator(r#"<spacetime><expression property="grid">time > 1</expression></spacetime>"#);
    assert_eq!(path_of(node(&frame(&ev, 0.5), "ax/spacetimeGrid")), "");
    assert!(!path_of(node(&frame(&ev, 1.5), "ax/spacetimeGrid")).is_empty());
}

#[test]
fn an_event_s_point_is_its_marker_size_across_and_only_inside_the_box() {
    let ev = evaluator(r#"<spacetime><event x="0.5" t="0.5" markerSize="10"/><event x="5" t="0"/></spacetime>"#);
    let g = frame(&ev, 0.0);
    let c = centre(&g, "ax/event0");
    let sp = subpaths(&path_of(node(&g, "ax/event0")));
    assert!(sp[0].0.iter().all(|p| ((p[0] - c[0]).hypot(p[1] - c[1]) - 5.0).abs() < 1e-9));
    // (5, 0) is right of the box: it draws no point
    assert_eq!(path_of(node(&g, "ax/event1")), "");
}

#[test]
fn a_worldline_without_tau_runs_over_the_box_s_extent_both_ways() {
    // τ from −3 to 3 (yMax − yMin) on x = 0, t = τ: the line crosses the whole box
    let g = frame(&evaluator(r#"<spacetime><worldline kind="parametric" x="0" t="tau"/></spacetime>"#), 0.0);
    let sp = subpaths(&path_of(node(&g, "ax/worldline0")));
    let ys: Vec<f64> = sp.iter().flat_map(|(p, _)| p.iter().map(|q| q[1])).collect();
    let (lo, hi) = (ys.iter().cloned().fold(f64::INFINITY, f64::min), ys.iter().cloned().fold(0.0, f64::max));
    assert!(lo.abs() < 1e-9 && (hi - 300.0).abs() < 1e-9, "{lo} {hi}");
}

#[test]
fn c_scales_a_cone_s_apex_and_reach() {
    let g = frame(
        &evaluator(r#"<spacetime c="2"><event x="0" t="0.25" lightCone="true" extent="0.25"/></spacetime>"#),
        0.0,
    );
    let sp = subpaths(&path_of(node(&g, "ax/cone0Future")));
    let pts = &sp[0].0;
    // the apex at ct = 0.5, the ends c · 0.25 = 0.5 away in x and ct
    assert!(near(pts[0], pixel(0.0, 0.5, 0.0), 1e-9), "{pts:?}");
    assert!(near(pts[1], pixel(-0.5, 1.0, 0.0), 1e-9) || near(pts[1], pixel(0.5, 1.0, 0.0), 1e-9), "{pts:?}");
}

#[test]
fn a_label_whose_braces_close_before_they_open_is_a_problem() {
    let g = evaluator(r#"<spacetime><event x="0" t="0" label="}P{"/></spacetime>"#).evaluate(0.0);
    assert!(g.problems.iter().any(|p| p.contains("unpaired braces")), "{:?}", g.problems);
}

#[test]
fn a_worldline_s_label_sits_by_the_last_point_it_draws() {
    let g = frame(
        &evaluator(
            r#"<spacetime><worldline kind="parametric" x="0" t="tau" tau0="-1" tau1="1" label="ship"/></spacetime>"#,
        ),
        0.0,
    );
    let f = node(&g, "ax").axes.clone().unwrap();
    let l = f.labels.iter().find(|l| l.tex == "\\text{ship}").unwrap();
    // τ = 1 is pixel (200, 50); the label is LABEL_GAP below right of it
    assert!(near(l.at, [204.0, 54.0], 1e-9), "{:?}", l.at);
}

#[test]
fn a_cone_that_leaves_the_box_is_clipped_to_it() {
    // the future cone's right end (2.005, 1) is half a pixel right of the box
    let g = frame(&evaluator(r#"<spacetime><event x="1.005" t="0" lightCone="true" extent="1"/></spacetime>"#), 0.0);
    let sp = subpaths(&path_of(node(&g, "ax/cone0Future")));
    assert_eq!(sp.len(), 1);
    let (pts, closed) = &sp[0];
    assert!(*closed);
    assert!(pts.iter().all(|p| (0.0..=400.0).contains(&p[0]) && (0.0..=300.0).contains(&p[1])), "{pts:?}");
    // the right edge cuts the triangle into a quadrilateral with two vertices on x = 400
    assert_eq!(pts.len(), 4, "{pts:?}");
    assert_eq!(pts.iter().filter(|p| (p[0] - 400.0).abs() < 1e-9).count(), 2, "{pts:?}");
}

#[test]
fn a_light_ray_s_length_is_in_units_of_t() {
    // c = 2: length 0.5 of t reaches 1 drawn unit along (1, 1) from (0, c · -0.25)
    let g = frame(&evaluator(r#"<spacetime c="2"><lightRay x="0" t="-0.25" length="0.5"/></spacetime>"#), 0.0);
    let pts = subpaths(&path_of(node(&g, "ax/lightRay0"))).remove(0).0;
    assert!(near(pts[0], pixel(0.0, -0.5, 0.0), 1e-9), "{pts:?}");
    assert!(near(*pts.last().unwrap(), pixel(1.0, 0.5, 0.0), 1e-9), "{pts:?}");
}

#[test]
fn an_expression_that_reaches_the_speed_of_light_is_a_frame_problem() {
    let ev =
        evaluator(r#"<spacetime><expression property="boost">0.2 * time</expression><event x="0" t="0"/></spacetime>"#);
    assert!(ev.evaluate(1.0).problems.is_empty());
    // at t = 5 the expression gives beta = 1: it is not clamped, nothing is drawn with it, and the frame says so
    let g = ev.evaluate(5.0);
    assert!(
        g.problems.iter().any(|p| p.contains("ax/event0") && p.contains("boost must lie strictly between -1 and 1")),
        "{:?}",
        g.problems
    );
    assert!(node(&g, "ax/event0").props.get("path").is_none());
}
