//! The plots draft at the evaluator: an `axes` node lowers to path shapes for its grid, axis lines, ticks and plots,
//! a clip group of its box that holds its children in data units, and its labels (sr-core srep/srep-0000-plots.md,
//! Semantics 1 to 9 and 13).

use sr_eval::plots::{self, Axes};
use sr_eval::{Evaluator, FrameGraph, FrameNode};

fn evaluator(composition: &str) -> Evaluator {
    let xml = format!(
        r##"<scene version="1.8"><project width="640" height="360" fps="24" duration="2" background="#000000FF" seed="1"/>
  <composition>{composition}</composition></scene>"##
    );
    let doc =
        sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}\n{xml}"));
    Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"))
}

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

fn near(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6
}

const AX: &str = r#"id="ax" x="10" y="20" width="400" height="300""#;

#[test]
fn a_child_is_placed_in_data_units_and_keeps_its_pixel_size() {
    let ev = evaluator(&format!(
        r##"<axes {AX}><shape id="dot" shape="ellipse" width="8" height="8" x="0.5" y="0.5" anchorX="0" anchorY="0" fill="#FF0000FF"/></axes>"##
    ));
    let g = frame(&ev, 0.0);
    let dot = node(&g, "dot");
    // rule 1 on the default ranges −1..1: (0.5, 0.5) is pixel (300, 75) of the box at (10, 20)
    let w = dot.world.0;
    assert!(near([w[4], w[5]], [310.0, 95.0]), "{w:?}");
    assert!((w[0] - 1.0).abs() < 1e-12 && (w[3] - 1.0).abs() < 1e-12, "the size stays in pixels: {w:?}");
    assert_eq!(dot.size, Some([8.0, 8.0]));
}

#[test]
fn an_explicit_plot_is_the_reference_sampling_mapped_and_clipped() {
    let ev = evaluator(&format!(
        r#"<axes {AX} xMin="0" xMax="1" yMin="0" yMax="0.5"><plot kind="explicit" y="x * x" samples="5"/></axes>"#
    ));
    let g = frame(&ev, 0.0);
    let plot = node(&g, "ax/plot0");
    let a = Axes::new(0.0, 1.0, 0.0, 0.5, 400.0, 300.0).unwrap();
    let pts: Vec<[f64; 2]> = plots::sample_explicit(|x| x * x, 0.0, 1.0, 5)
        .unwrap()
        .iter()
        .map(|p| plots::to_pixel(&a, p[0], p[1]))
        .collect();
    let want = plots::clip_polyline(&pts, 0.0, 0.0, 400.0, 300.0);
    let got = subpaths(&path_of(plot));
    assert_eq!(got.len(), want.len(), "{got:?}");
    for ((g, closed), w) in got.iter().zip(&want) {
        assert!(!closed);
        assert_eq!(g.len(), w.len(), "{g:?} {w:?}");
        assert!(g.iter().zip(w).all(|(a, b)| near(*a, *b)), "{g:?} {w:?}");
    }
    // the curve leaves the box at y = 0.5, x = √0.5: the clipped end is on the top edge
    assert_eq!(got[0].0.last().unwrap()[1], 0.0);
    // the plot's defaults: a 3 px #FFCC00 stroke
    assert_eq!(plot.kind, "shape");
}

#[test]
fn parametric_polar_and_implicit_plots_close_their_loops() {
    let ev = evaluator(&format!(
        r#"<axes {AX} xMin="-2" xMax="2" yMin="-2" yMax="2"><plot kind="parametric" x="Math.cos(t)" y="Math.sin(t)" t1="6.283185307179586" closed="true" samples="64"/><plot kind="polar" r="1" samples="65"/><plot kind="implicit" f="x * x + y * y - 1" resolution="32"/></axes>"#
    ));
    let g = frame(&ev, 0.0);
    let a = Axes::new(-2.0, 2.0, -2.0, 2.0, 400.0, 300.0).unwrap();
    let on_circle = |p: [f64; 2]| {
        let d = plots::from_pixel(&a, p[0], p[1]);
        (d[0].hypot(d[1]) - 1.0).abs()
    };
    let par = subpaths(&path_of(node(&g, "ax/plot0")));
    assert_eq!(par.len(), 1);
    assert!(par[0].1, "a closed parametric plot is closed");
    assert_eq!(par[0].0.len(), 64);
    assert!(par[0].0.iter().all(|p| on_circle(*p) < 1e-9));
    let polar = subpaths(&path_of(node(&g, "ax/plot1")));
    assert_eq!((polar.len(), polar[0].0.len()), (1, 65));
    assert!(polar[0].0.iter().all(|p| on_circle(*p) < 1e-9));
    let imp = subpaths(&path_of(node(&g, "ax/plot2")));
    assert_eq!(imp.len(), 1);
    assert!(imp[0].1, "the implicit circle is one closed polyline");
    assert!(imp[0].0.iter().all(|p| on_circle(*p) < 0.01), "{:?}", imp[0].0);
}

#[test]
fn ticks_labels_and_grid_follow_the_nice_steps() {
    let ev = evaluator(&format!(r#"<axes {AX} xMin="0" xMax="10" yMin="0" yMax="1" grid="true"/>"#));
    let g = frame(&ev, 0.0);
    let ax = node(&g, "ax");
    let f = ax.axes.as_ref().expect("the axes' frame");
    let texts: Vec<&str> = f.labels.iter().map(|l| l.tex.as_str()).collect();
    // x: 1..10 by 1 (0 omitted at the origin); y: 0.1..1 by 0.1
    let mut want: Vec<String> = (1..=10).map(|k| k.to_string()).collect();
    want.extend(["0.1", "0.2", "0.3", "0.4", "0.5", "0.6", "0.7", "0.8", "0.9", "1"].map(String::from));
    assert_eq!(texts, want);
    // an x label hangs centred below its tick, 4 px under the tick's end
    let two = &f.labels[1];
    assert!(near(two.at, [80.0, 300.0 + 3.0 + 4.0]), "{:?}", two.at);
    assert_eq!(two.align, sr_eval::axes::Align::Below);
    // the grid: one line per tick of both axes (11 + 11), under the axis lines
    assert_eq!(subpaths(&path_of(node(&g, "ax/grid"))).len(), 22);
    assert_eq!(subpaths(&path_of(node(&g, "ax/axis"))).len(), 2);
    assert_eq!(subpaths(&path_of(node(&g, "ax/ticks"))).len(), 22);
    let order: Vec<&str> = g.nodes.iter().filter(|n| n.id.starts_with("ax/")).map(|n| &*n.id).collect();
    assert_eq!(order, ["ax/grid", "ax/axis", "ax/ticks", "ax/content", "ax/labels"]);
}

#[test]
fn a_complex_plane_writes_its_imaginary_labels_with_i() {
    let ev =
        evaluator(&format!(r#"<axes {AX} kind="complex" xMin="-2" xMax="2" yMin="-2" yMax="2" xStep="1" yStep="1"/>"#));
    let g = frame(&ev, 0.0);
    let f = node(&g, "ax").axes.clone().unwrap();
    let texts: Vec<&str> = f.labels.iter().map(|l| l.tex.as_str()).collect();
    assert_eq!(texts, ["-2", "-1", "1", "2", "-2i", "-i", "i", "2i", "\\mathrm{Re}", "\\mathrm{Im}"]);
}

#[test]
fn a_number_line_labels_its_origin_and_has_no_y_axis() {
    let ev = evaluator(&format!(r#"<axes {AX} kind="number-line" xMin="0" xMax="4" xStep="1"/>"#));
    let g = frame(&ev, 0.0);
    let f = node(&g, "ax").axes.clone().unwrap();
    let texts: Vec<&str> = f.labels.iter().map(|l| l.tex.as_str()).collect();
    assert_eq!(texts, ["0", "1", "2", "3", "4"]);
    let axis = subpaths(&path_of(node(&g, "ax/axis")));
    assert_eq!(axis.len(), 1);
    assert!(near(axis[0].0[0], [0.0, 150.0]) && near(axis[0].0[1], [400.0, 150.0]), "{axis:?}");
}

#[test]
fn an_animated_range_moves_the_children_and_the_plots() {
    let ev = evaluator(&format!(
        r##"<axes {AX} xMin="0" xMax="1"><animate property="xMax"><key time="0" value="1"/><key time="1" value="2"/></animate><shape id="dot" shape="rect" width="2" height="2" x="1" y="0" fill="#FFFFFFFF"/><plot kind="explicit" y="0" samples="2"/></axes>"##
    ));
    let at = |t: f64| {
        let g = frame(&ev, t);
        let x = node(&g, "dot").world.0[4];
        let end = subpaths(&path_of(node(&g, "ax/plot0")))[0].0.last().copied().unwrap();
        (x, end)
    };
    assert!((at(0.0).0 - 410.0).abs() < 1e-9);
    assert!((at(1.0).0 - 210.0).abs() < 1e-9, "x = 1 of 0..2 is the middle of the box");
    // the plot samples the x range of each frame: its last point is the box's right edge
    assert!(near(at(1.0).1, [400.0, 150.0]));
}

#[test]
fn a_plot_expression_that_does_not_compile_is_an_e01() {
    let xml = r##"<scene version="1.8"><project width="64" height="64" fps="24" duration="1" background="#000000FF"/><composition><axes id="ax" width="40" height="30"><plot kind="explicit" y="x +"/></axes></composition></scene>"##;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let err = match Evaluator::new(&doc, &Default::default()) {
        Ok(_) => panic!("a broken plot expression compiles"),
        Err(e) => e.to_string(),
    };
    assert!(err.contains("E01") && err.contains("ax/plot[0]@y"), "{err}");
}

#[test]
fn an_anchor_is_an_offset_in_data_units() {
    let ev = evaluator(&format!(
        r##"<axes {AX}><shape id="dot" shape="rect" width="8" height="8" x="0" y="0" anchorX="0.5" anchorY="0.5" fill="#FF0000FF"/></axes>"##
    ));
    let g = frame(&ev, 0.0);
    let dot = node(&g, "dot");
    // the anchor (0.5, 0.5) of the default ranges −1..1 is 100 px right and 75 px up: the dot's origin lands at
    // data (0, 0), pixel (200, 150), less that offset
    let w = dot.world.0;
    assert!(near([w[4], w[5]], [10.0 + 200.0 - 100.0, 20.0 + 150.0 + 75.0]), "{w:?}");
    assert!(near(dot.anchor, [100.0, -75.0]), "{:?}", dot.anchor);
}

#[test]
fn a_range_that_closes_at_a_frame_is_a_problem_and_draws_nothing() {
    let ev = evaluator(&format!(
        r##"<axes {AX} xMin="0" xMax="1"><animate property="xMax"><key time="0" value="1"/><key time="1" value="-1"/></animate><shape id="dot" shape="rect" width="2" height="2" x="0.5" y="0" fill="#FFFFFFFF"/><plot kind="explicit" y="x" samples="2"/></axes>"##
    ));
    let g = ev.evaluate(1.0);
    assert!(g.problems.iter().any(|p| p.starts_with("ax:")), "{:?}", g.problems);
    assert!(node(&g, "ax").axes.is_none());
    assert!(node(&g, "ax/plot0").props.get("path").is_none());
    // the child is left in pixels: x = 0.5 px from the box's left
    assert!((node(&g, "dot").world.0[4] - 10.5).abs() < 1e-9);
}

const HELIX3D: &str = r#"<object3D id="o" primitive="axes"><axes3D width="2" height="2" depth="2" xMin="-1" xMax="1" yMin="-1" yMax="1" zMin="0" zMax="2"/><plot3D x="Math.cos(t)" y="Math.sin(t)" z="t / Math.PI" t1="6.283185307179586" samples="32" tubeRadius="0.05" tubeSegments="6"/></object3D>"#;

fn linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

#[test]
fn an_axes_object_is_its_axis_cylinders_and_its_plots_tubes() {
    let ev = evaluator(HELIX3D);
    let g = frame(&ev, 0.0);
    let o = node(&g, "o");
    let mesh = &o.param_mesh.as_ref().expect("the axes object's mesh").mesh;
    let yellow = [1.0, linear(0.8), 0.0, 1.0];
    let tube: Vec<[f32; 3]> = mesh.vertices.iter().filter(|v| v.color == yellow).map(|v| v.pos).collect();
    // rule 11: samples × tubeSegments vertices, each tubeRadius from its point of the mapped curve (rule 10)
    assert_eq!(tube.len(), 32 * 6);
    let a = plots::Axes3D::new([-1.0, 1.0], [-1.0, 1.0], [0.0, 2.0], [2.0, 2.0, 2.0]).unwrap();
    let curve: Vec<[f64; 3]> = plots::grid(0.0, std::f64::consts::TAU, 32, false)
        .unwrap()
        .iter()
        .map(|&t| plots::to_scene(&a, t.cos(), t.sin(), t / std::f64::consts::PI))
        .collect();
    for v in &tube {
        let d = curve
            .iter()
            .map(|c| (0..3).map(|k| (v[k] as f64 - c[k]).powi(2)).sum::<f64>().sqrt())
            .fold(f64::INFINITY, f64::min);
        assert!((d - 0.05).abs() < 1e-4, "{v:?} is {d} from the curve");
    }
    // the axis lines and ticks are white, and every vertex is finite
    let white = mesh.vertices.iter().filter(|v| v.color == [1.0; 4]).count();
    assert!(white > 0 && white + tube.len() == mesh.vertices.len(), "{white} {}", mesh.vertices.len());
    assert!(mesh.vertices.iter().all(|v| v.pos.iter().all(|c| c.is_finite())));
    // the x axis runs through data y = 0 (scene y 0) and z = 0, the z range's lower end (scene z −1)
    assert!(mesh
        .vertices
        .iter()
        .any(|v| v.color == [1.0; 4] && (v.pos[0] - 1.0).abs() < 1e-6 && (v.pos[2] + 1.0).abs() < 0.03));
}

#[test]
fn a_plot3d_expression_that_does_not_compile_is_an_e01() {
    let xml = format!(
        r##"<scene version="1.8"><project width="64" height="64" fps="24" duration="1" background="#000000FF"/><composition>{}</composition></scene>"##,
        HELIX3D.replace(r#"z="t / Math.PI""#, r#"z="t /""#)
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let err = match Evaluator::new(&doc, &Default::default()) {
        Ok(_) => panic!("a broken plot3D expression compiles"),
        Err(e) => e.to_string(),
    };
    assert!(err.contains("E01") && err.contains("o/plot3D[0]@z"), "{err}");
}

#[test]
fn a_plot3d_with_a_material_is_a_part_of_its_own() {
    // plot3D's `material` is as the object's other children have it (the draft's syntax 3): such a plot leaves the
    // object's mesh for a part drawn with that material; plots without one stay in the object's mesh
    let xml = format!(
        r##"<scene version="1.8"><project width="640" height="360" fps="24" duration="2" background="#000000FF" seed="1"/>
  <materials><material id="gold" baseColor="#FFD700FF"/></materials>
  <composition>{}</composition></scene>"##,
        HELIX3D.replace(
            "</object3D>",
            r#"<plot3D x="t" y="0" z="1" t0="-1" t1="1" samples="8" tubeSegments="4" material="gold"/><plot3D x="0" y="t" z="1" t0="-1" t1="1" samples="8" tubeSegments="4" material="gold"/></object3D>"#
        )
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
    let ev = Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"));
    let g = frame(&ev, 0.0);
    let o = node(&g, "o");
    let mesh = &o.param_mesh.as_ref().expect("the axes object's mesh").mesh;
    let yellow = [1.0, linear(0.8), 0.0, 1.0];
    let white = mesh.vertices.iter().filter(|v| v.color == [1.0; 4]).count();
    assert_eq!(mesh.vertices.iter().filter(|v| v.color == yellow).count(), 32 * 6);
    assert_eq!(white + 32 * 6, mesh.vertices.len(), "the object's mesh holds only the axes and the helix");
    // the two gold plots are one part of 2 × 8 × 4 vertices, in their colour, the default yellow
    assert_eq!(o.param_parts.len(), 1, "{:?}", o.param_parts.iter().map(|p| &p.0).collect::<Vec<_>>());
    let (material, part) = &o.param_parts[0];
    assert_eq!(&**material, "gold");
    assert_eq!(part.mesh.vertices.len(), 2 * 8 * 4);
    assert!(part.mesh.vertices.iter().all(|v| v.color == yellow));
}
