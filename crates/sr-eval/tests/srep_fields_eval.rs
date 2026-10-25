//! The fields draft at the evaluator: a `field` node lowers to a path shape per stroked presentation (arrows,
//! streamlines, lines, contours) and a raster per lic and portrait, computed each frame from its source by the rules
//! of `sr_eval::fields` (sr-core srep/srep-0000-fields.md, Semantics 1 to 10).

use sr_eval::fields::{self, ArrowStyle, ChargeLines, Spacing};
use sr_eval::{Evaluator, FrameGraph, FrameNode};

fn document(composition: &str, physics: &str) -> String {
    format!(
        r##"<scene version="1.8"><project width="640" height="360" fps="24" duration="2" background="#000000FF" seed="1"/>
  <parameters><data id="flow" format="csv">x,y,u,v
-2,-1,1,0
2,-1,0,1
-2,1,-1,0
2,1,0,-1</data></parameters><composition>{composition}</composition>{physics}</scene>"##
    )
}

fn evaluator_with(composition: &str, physics: &str) -> Evaluator {
    let xml = document(composition, physics);
    let doc =
        sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}\n{xml}"));
    Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"))
}

fn evaluator(composition: &str) -> Evaluator {
    evaluator_with(composition, "")
}

fn refused(composition: &str) -> String {
    let xml = document(composition, "");
    let doc =
        sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}\n{xml}"));
    match Evaluator::new(&doc, &Default::default()) {
        Ok(_) => panic!("{composition} builds"),
        Err(e) => e.to_string(),
    }
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

/// The box of [`BOX`]: 400 x 200 px over [-2, 2] x [-1, 1], 100 px per data unit on both axes, +y up.
const BOX: &str = r#"id="f" x="10" y="20" width="400" height="200" xMin="-2" xMax="2" yMin="-1" yMax="1""#;
const RANGES: [f64; 4] = [-2.0, -1.0, 2.0, 1.0];
const PPU: f64 = 100.0;

fn px(p: [f64; 2]) -> [f64; 2] {
    [(p[0] + 2.0) * PPU, (1.0 - p[1]) * PPU]
}

#[track_caller]
fn same_polylines(got: &[(Vec<[f64; 2]>, bool)], want: &[Vec<[f64; 2]>]) {
    assert_eq!(got.len(), want.len(), "subpaths: {} against {}", got.len(), want.len());
    for ((g, _), w) in got.iter().zip(want) {
        assert_eq!(g.len(), w.len(), "{g:?} {w:?}");
        for (a, b) in g.iter().zip(w) {
            let b = px(*b);
            assert!((a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6, "{a:?} against {b:?}");
        }
    }
}

/// Rule 5's glyphs as polylines: the shaft, then the head left, tip, right.
fn arrow_polylines(arrows: &[Option<fields::Arrow>]) -> Vec<Vec<[f64; 2]>> {
    arrows.iter().flatten().flat_map(|a| [a.shaft.to_vec(), vec![a.head[1], a.head[0], a.head[2]]]).collect()
}

fn rotation(x: f64, y: f64) -> [f64; 2] {
    [-y, x]
}

#[test]
fn arrows_are_rule_5_glyphs_on_the_grid_mapped_to_the_box() {
    let ev = evaluator(&format!(
        r#"<field {BOX} u="-y" v="x"><arrows cols="5" rows="3" scale="0.25" maxLength="30" headLength="0.4" headWidth="0.3"/></field>"#
    ));
    let g = frame(&ev, 0.0);
    let style = ArrowStyle { scale: 0.25, max_length: 30.0 / PPU, head_length: 0.4, head_width: 0.3 };
    let want = arrow_polylines(&fields::arrow_grid(rotation, RANGES, 5, 3, &style).unwrap());
    let arrows = node(&g, "f/arrows0");
    assert_eq!(arrows.kind, "shape");
    same_polylines(&subpaths(&path_of(arrows)), &want);
    // the field node sits at its x and y
    let w = node(&g, "f").world.0;
    assert_eq!([w[4], w[5]], [10.0, 20.0]);
}

#[test]
fn streamlines_are_rule_6_in_data_units_from_pixel_attributes() {
    let ev = evaluator(&format!(r#"<field {BOX} u="1" v="0.5"><streamlines separation="40" maxLines="20"/></field>"#));
    let g = frame(&ev, 0.0);
    let s = Spacing { d_sep: 0.4, d_test: None, h: None, max_steps: 10_000, max_lines: 20, seed: Some([0.0, 0.0]) };
    let want = fields::evenly_spaced_streamlines(|_, _| [1.0, 0.5], RANGES, &s).unwrap();
    assert!(want.len() > 3);
    same_polylines(&subpaths(&path_of(node(&g, "f/streamlines0"))), &want);
}

#[test]
fn a_seed_test_and_step_reach_rule_6() {
    let ev = evaluator(&format!(
        r#"<field {BOX} u="-y" v="x"><streamlines separation="30" test="10" step="2" seedX="0.5" seedY="0.25" maxSteps="500"/></field>"#
    ));
    let g = frame(&ev, 0.0);
    let s = Spacing {
        d_sep: 0.3,
        d_test: Some(0.1),
        h: Some(0.02),
        max_steps: 500,
        max_lines: 10_000,
        seed: Some([0.5, 0.25]),
    };
    let want = fields::evenly_spaced_streamlines(rotation, RANGES, &s).unwrap();
    same_polylines(&subpaths(&path_of(node(&g, "f/streamlines0"))), &want);
}

#[test]
fn charge_lines_are_rule_10_and_the_charges_are_the_source() {
    let ev = evaluator(&format!(
        r#"<field {BOX}><charge x="-0.5" y="0" q="1"/><charge x="0.5" y="0" q="-1"/><lines perCharge="6" radius="0.05" maxSteps="3000"/><arrows cols="3" rows="3"/></field>"#
    ));
    let g = frame(&ev, 0.0);
    let charges = [[-0.5, 0.0, 1.0], [0.5, 0.0, -1.0]];
    // the step defaults to a quarter of the default separation, 20 px: 5 px
    let c = ChargeLines { per_charge: 6, r0: 0.05, h: 0.05, max_steps: 3000, ranges: RANGES };
    let want = fields::charge_lines(&charges, &c).unwrap();
    same_polylines(&subpaths(&path_of(node(&g, "f/lines0"))), &want);
    let style = ArrowStyle { max_length: 40.0 / PPU, ..ArrowStyle::default() };
    let f = |x, y| fields::charge_field(&charges, [x, y]);
    let want = arrow_polylines(&fields::arrow_grid(f, RANGES, 3, 3, &style).unwrap());
    same_polylines(&subpaths(&path_of(node(&g, "f/arrows0"))), &want);
}

#[test]
fn currents_and_dipoles_sum_with_the_charges() {
    let ev = evaluator(&format!(
        r#"<field {BOX}><current x="0" y="0" i="2"/><dipole x="1" y="0" mx="0" my="1"/><arrows cols="4" rows="3"/></field>"#
    ));
    let g = frame(&ev, 0.0);
    let f = |x, y| {
        let (a, b) =
            (fields::wire_field([x, y], 2.0, [0.0, 0.0]), fields::dipole_field([x, y], [0.0, 1.0], [1.0, 0.0]));
        [a[0] + b[0], a[1] + b[1]]
    };
    let style = ArrowStyle { max_length: 40.0 / PPU, ..ArrowStyle::default() };
    let want = arrow_polylines(&fields::arrow_grid(f, RANGES, 4, 3, &style).unwrap());
    same_polylines(&subpaths(&path_of(node(&g, "f/arrows0"))), &want);
}

#[test]
fn a_complex_function_and_its_w_expressions_draw_alike() {
    let style = ArrowStyle { max_length: 40.0 / PPU, ..ArrowStyle::default() };
    let square = |x: f64, y: f64| [x * x - y * y, 2.0 * x * y];
    let want = arrow_polylines(&fields::arrow_grid(square, RANGES, 4, 3, &style).unwrap());
    for source in [r#"function="square""#, r#"w="x * x - y * y; 2 * x * y""#] {
        let ev = evaluator(&format!(r#"<field {BOX} kind="complex" {source}><arrows cols="4" rows="3"/></field>"#));
        let g = frame(&ev, 0.0);
        same_polylines(&subpaths(&path_of(node(&g, "f/arrows0"))), &want);
    }
}

#[test]
fn a_grid_is_bilinear_between_its_lattice_rows() {
    let ev = evaluator(&format!(r#"<field {BOX} grid="flow"><arrows cols="3" rows="3"/></field>"#));
    let g = frame(&ev, 0.0);
    let lattice = vec![vec![[1.0, 0.0], [0.0, 1.0]], vec![[-1.0, 0.0], [0.0, -1.0]]];
    let f = fields::grid_sampler(&lattice, RANGES).unwrap();
    let style = ArrowStyle { max_length: 40.0 / PPU, ..ArrowStyle::default() };
    let want = arrow_polylines(&fields::arrow_grid(f, RANGES, 3, 3, &style).unwrap());
    same_polylines(&subpaths(&path_of(node(&g, "f/arrows0"))), &want);
}

#[test]
fn a_force_field_is_sampled_in_metres_with_y_up_and_follows_its_animation() {
    let physics = r#"<physics pixelsPerMeter="50"><forceField id="push" type="directional" forceX="2" forceY="1"><animate property="forceX"><key time="0" value="0"/><key time="1" value="2"/></animate></forceField></physics>"#;
    let ev = evaluator_with(&format!(r#"<field {BOX} forceField="push"><arrows cols="2" rows="2"/></field>"#), physics);
    let style = ArrowStyle { max_length: 40.0 / PPU, ..ArrowStyle::default() };
    for (t, fx) in [(0.0, 0.0), (1.0, 2.0)] {
        let g = frame(&ev, t);
        let want = arrow_polylines(&fields::arrow_grid(|_, _| [fx, 1.0], RANGES, 2, 2, &style).unwrap());
        same_polylines(&subpaths(&path_of(node(&g, "f/arrows0"))), &want);
    }
}

#[test]
fn contours_are_closed_level_curves_of_a_scalar_field() {
    let ev = evaluator(&format!(
        r#"<field {BOX} kind="scalar" u="x * x + y * y"><contours levels="0.25 0.64" resolution="80"/></field>"#
    ));
    let g = frame(&ev, 0.0);
    let got = subpaths(&path_of(node(&g, "f/contours0")));
    assert_eq!(got.len(), 2, "{got:?}");
    for ((pts, closed), r) in got.iter().zip([0.5, 0.8]) {
        assert!(closed);
        for p in pts {
            let d = [p[0] / PPU - 2.0, 1.0 - p[1] / PPU];
            assert!((d[0].hypot(d[1]) - r).abs() < 0.01, "{p:?} off the circle of radius {r}");
        }
    }
}

#[test]
fn an_expression_field_reads_time() {
    let ev = evaluator(&format!(r#"<field {BOX} u="time" v="1"><arrows cols="2" rows="2"/></field>"#));
    let style = ArrowStyle { max_length: 40.0 / PPU, ..ArrowStyle::default() };
    let g = frame(&ev, 0.5);
    let want = arrow_polylines(&fields::arrow_grid(|_, _| [0.5, 1.0], RANGES, 2, 2, &style).unwrap());
    same_polylines(&subpaths(&path_of(node(&g, "f/arrows0"))), &want);
}

#[test]
fn in_an_axes_the_box_and_ranges_are_the_axes() {
    let ev = evaluator(
        r#"<axes id="ax" x="10" y="20" width="400" height="200" xMin="-2" xMax="2" yMin="-1" yMax="1"><field id="f" u="-y" v="x"><arrows cols="3" rows="3"/></field></axes>"#,
    );
    let g = frame(&ev, 0.0);
    let style = ArrowStyle { max_length: 40.0 / PPU, ..ArrowStyle::default() };
    let want = arrow_polylines(&fields::arrow_grid(rotation, RANGES, 3, 3, &style).unwrap());
    same_polylines(&subpaths(&path_of(node(&g, "f/arrows0"))), &want);
    // the field sits at the box's origin, not at data (0, 0)
    let w = node(&g, "f").world.0;
    assert_eq!([w[4], w[5]], [10.0, 20.0]);
}

#[test]
fn a_presentation_strokes_with_its_own_paint_or_the_fields() {
    let ev = evaluator(&format!(
        r##"<field {BOX} u="1" v="0" stroke="#FF0000FF" strokeWidth="3" dash="4 2"><arrows/><streamlines stroke="#00FF00FF" strokeWidth="1" markerEnd="arrow"/></field>"##
    ));
    let g = frame(&ev, 0.0);
    let arrows = node(&g, "f/arrows0");
    let lines = node(&g, "f/streamlines0");
    // static paint is the node's element's, not a per-frame property
    let get = |n: &FrameNode, k: &str| (&*n.elem as &dyn sr_model::element::Element).get_attr(k).map(|v| v.to_string());
    assert_eq!(get(arrows, "stroke").as_deref(), Some("#ff0000ff"));
    assert_eq!(get(arrows, "strokeWidth").as_deref(), Some("3"));
    assert_eq!(get(arrows, "dash").as_deref(), Some("4 2"));
    assert_eq!(get(lines, "stroke").as_deref(), Some("#00ff00ff"));
    assert_eq!(get(lines, "strokeWidth").as_deref(), Some("1"));
    assert_eq!(get(lines, "markerEnd").as_deref(), Some("arrow"));
}

#[test]
fn lic_is_rule_7_over_d24_noise_rows_from_the_top() {
    let ev = evaluator(&format!(
        r#"<field {BOX} u="1" v="0.25"><lic length="20" step="2" resolution="8" seed="3" opacity="0.5"/></field>"#
    ));
    let g = frame(&ev, 0.0);
    let r = node(&g, "f/lic0").raster.clone().expect("a lic raster");
    // 8 columns across the 2:1 box, 4 rows
    assert_eq!((r.cols, r.rows, r.opacity), (8, 4, 0.5));
    let noise: Vec<Vec<f64>> =
        (0..4).map(|j| (0..8).map(|i| sr_eval::rng::d24_unit(3, j as u64, i as u64)).collect()).collect();
    let want = fields::lic(|_, _| [1.0, 0.25], &noise, RANGES, 0.2, 0.02).unwrap();
    for (j, row) in want.iter().enumerate() {
        for (i, v) in row.iter().enumerate() {
            // raster row 0 is the box's top, yMax; the rule's row 0 is yMin
            let got = r.rgba[(3 - j) * 8 + i];
            assert!(
                (got[0] as f64 - v).abs() < 1e-6 && got[0] == got[1] && got[1] == got[2] && got[3] == 1.0,
                "{got:?} {v}"
            );
        }
    }
}

#[test]
fn a_portrait_is_rule_8_per_pixel_of_the_box() {
    let ev = evaluator(&format!(r#"<field {BOX} kind="complex" function="square"><portrait base="3"/></field>"#));
    let g = frame(&ev, 0.0);
    let r = node(&g, "f/portrait0").raster.clone().expect("a portrait raster");
    assert_eq!((r.cols, r.rows), (400, 200));
    for (i, j) in [(0, 0), (123, 45), (399, 199), (250, 100)] {
        let p = [-2.0 + (i as f64 + 0.5) / PPU, 1.0 - (j as f64 + 0.5) / PPU];
        let w = [p[0] * p[0] - p[1] * p[1], 2.0 * p[0] * p[1]];
        let (h, l) = (fields::phase_hue(w), 0.5 - 0.25 * fields::modulus_shade(w, 3.0));
        let want = hsl(h, l);
        let got = r.rgba[j * 400 + i];
        for k in 0..3 {
            assert!((got[k] as f64 - want[k]).abs() < 1e-6, "pixel ({i}, {j}): {got:?} against {want:?}");
        }
    }
}

/// HSL with full saturation to sRGB.
fn hsl(h: f64, l: f64) -> [f64; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * 1.0;
    let hp = h * 6.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [r + m, g + m, b + m]
}

#[test]
fn a_field_expression_that_does_not_compile_is_an_e01() {
    let err = refused(&format!(r#"<field {BOX} u="x +" v="y"><arrows/></field>"#));
    assert!(err.contains("E01") && err.contains("f@u"), "{err}");
}

#[test]
fn sources_that_do_not_fit_the_kind_are_flde1() {
    for bad in [
        format!(r#"<field {BOX} u="x"><arrows/></field>"#),
        format!(r#"<field {BOX} function="exp"><arrows/></field>"#),
        format!(r#"<field {BOX} kind="scalar" function="exp"><contours levels="0"/></field>"#),
        format!(r#"<field {BOX} kind="complex" u="x"><portrait/></field>"#),
        format!(r#"<field {BOX} kind="complex"><charge x="0" y="0" q="1"/><portrait/></field>"#),
        format!(r#"<field {BOX} kind="complex" function="mobius" coefficients="1 0 0"><portrait/></field>"#),
        format!(r#"<field {BOX} kind="complex" function="exp"><portrait base="1"/></field>"#),
        r#"<field id="f" u="-y" v="x"><arrows/></field>"#.to_string(),
        format!(r#"<field {BOX} grid="nowhere"><arrows/></field>"#).replace(r#"grid="nowhere""#, r#"grid="f""#),
        // an id that is not a forceField of physics
        format!(r#"<field {BOX} forceField="f"><arrows/></field>"#),
    ] {
        let err = refused(&bad);
        assert!(err.contains("FLDE1"), "{bad}: {err}");
    }
}
