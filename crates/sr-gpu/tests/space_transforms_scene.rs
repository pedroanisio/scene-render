//! The space-transforms draft in the vector builder, without a device: the modifier types matrix, map and complex
//! map a shape's path before it is filled and stroked (Semantics 4 and 5), and a group's deform maps its
//! descendants in the group's space after their own (Syntax 2, Semantics 9).

use sr_gpu::vector::{node_deformers, shape_scene};
use sr_vector::scene::{Cmd, Paint, Scene};

fn graph(composition: &str, t: f64) -> sr_eval::FrameGraph {
    let xml = format!(
        r##"<scene version="1.8"><project width="400" height="400" fps="24" duration="2" background="#000000FF"/>
  <composition>{composition}</composition></scene>"##
    );
    let doc =
        sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}\n{xml}"));
    sr_eval::Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}")).evaluate(t)
}

fn index(g: &sr_eval::FrameGraph, id: &str) -> usize {
    g.nodes.iter().position(|n| &*n.id == id).unwrap_or_else(|| panic!("no node {id}"))
}

/// The scene of shape `id`, under its own and its groups' deforms.
fn scene(g: &sr_eval::FrameGraph, id: &str) -> Scene {
    let i = index(g, id);
    let n = &g.nodes[i];
    let ds = node_deformers(i, g, n.size.unwrap_or([0.0, 0.0])).unwrap();
    let mut white = |_: &sr_eval::Value, _: [f64; 4]| Some(Paint::Solid { rgba: [1.0; 4], srgb: false });
    shape_scene(n, &mut white, 0.05, &ds).unwrap()
}

/// The points of every fill, one list per command.
fn fills(s: &Scene) -> Vec<Vec<[f64; 2]>> {
    s.cmds
        .iter()
        .filter_map(|c| match c {
            Cmd::Fill { polys, .. } => Some(polys.iter().flat_map(|q| q.pts.iter().map(|r| [r.x, r.y])).collect()),
            _ => None,
        })
        .collect()
}

fn bounds(pts: &[[f64; 2]]) -> [f64; 4] {
    pts.iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, q| {
        [b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[0]), b[3].max(q[1])]
    })
}

fn near(a: [f64; 4], b: [f64; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6)
}

#[test]
fn a_matrix_maps_the_path_and_the_stroke_keeps_its_width() {
    let g = graph(
        r##"<shape id="s" shape="rect" width="100" height="50" fill="#FFFFFFFF" stroke="#FF0000FF" strokeWidth="4">
            <deform><modifier type="matrix" a="2" centerX="0" centerY="0"/></deform></shape>"##,
        0.0,
    );
    let f = fills(&scene(&g, "s"));
    assert_eq!(f.len(), 2, "a fill and a stroke");
    assert!(near(bounds(&f[0]), [0.0, 0.0, 200.0, 50.0]), "{:?}", bounds(&f[0]));
    // the stroke is drawn on the mapped path: 4 px wide on every side, not 8 on the stretched ones
    assert!(near(bounds(&f[1]), [-2.0, -2.0, 202.0, 52.0]), "{:?}", bounds(&f[1]));
}

#[test]
fn a_pole_breaks_the_path_into_finite_pieces() {
    // 1/z about the line's midpoint: the line through the pole maps to two rays, cut where they leave every bound
    let g = graph(
        r##"<shape id="s" shape="path" path="M 0 20 L 40 20" width="40" height="40" stroke="#FFFFFFFF" strokeWidth="1">
            <deform><modifier type="complex" function="inverse" centerX="20" centerY="20" unit="10"/></deform></shape>"##,
        0.0,
    );
    let s = scene(&g, "s");
    let polys: Vec<_> = s
        .cmds
        .iter()
        .flat_map(|c| match c {
            Cmd::Fill { polys, .. } => polys.clone(),
            _ => Vec::new(),
        })
        .collect();
    assert!(polys.len() >= 2, "{} pieces", polys.len());
    assert!(polys.iter().flat_map(|q| &q.pts).all(|r| r.x.is_finite() && r.y.is_finite()));
}

#[test]
fn a_map_reads_its_frame_time() {
    let at = |t: f64| {
        let g = graph(
            r##"<shape id="s" shape="rect" width="10" height="10" fill="#FFFFFFFF">
                <deform><modifier type="map" x="u + time" y="v" centerX="0" centerY="0" unit="10"/></deform></shape>"##,
            t,
        );
        bounds(&fills(&scene(&g, "s"))[0])
    };
    assert!(near(at(0.0), [0.0, 0.0, 10.0, 10.0]), "{:?}", at(0.0));
    // one data unit is 10 px
    assert!(near(at(1.0), [10.0, 0.0, 20.0, 10.0]), "{:?}", at(1.0));
}

#[test]
fn a_group_deform_maps_its_descendants_in_its_space_after_their_own() {
    // the inner group scales by 2: the outer group's 10 px shift is 5 px of the shape's own space; the shape's own
    // matrix doubles x first
    let g = graph(
        r##"<group id="g" x="50" y="50">
              <deform><modifier type="matrix" e="10" centerX="0" centerY="0"/></deform>
              <group id="inner" scaleX="2" scaleY="2"><shape id="s" shape="rect" width="10" height="10" fill="#FFFFFFFF">
                <deform><modifier type="matrix" a="2" centerX="0" centerY="0"/></deform></shape></group>
            </group>"##,
        0.0,
    );
    let b = bounds(&fills(&scene(&g, "s"))[0]);
    assert!(near(b, [5.0, 0.0, 25.0, 10.0]), "{b:?}");
}

#[test]
fn a_twist_alone_keeps_the_old_post_stroke_deform() {
    // without one of the three types the chain deforms the drawn scene: a twist of 0 changes nothing but splits edges
    let g = graph(
        r##"<shape id="s" shape="rect" width="100" height="50" fill="#FFFFFFFF">
            <deform><modifier type="twist" amount="0"/></deform></shape>"##,
        0.0,
    );
    let f = fills(&scene(&g, "s"));
    assert!(near(bounds(&f[0]), [0.0, 0.0, 100.0, 50.0]), "{:?}", bounds(&f[0]));
    assert!(f[0].len() > 4, "subdivided: {}", f[0].len());
}

#[test]
fn a_graph_without_its_program_cannot_build_a_map() {
    let xml = r##"<scene version="1.8"><project width="64" height="64" fps="24" duration="1"/><composition>
        <shape id="s" shape="rect" width="10" height="10"><deform><modifier type="map" x="u" y="v"/></deform></shape>
        </composition></scene>"##;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let g = sr_eval::eval::evaluate(ev.program(), 0.0);
    let i = index(&g, "s");
    let e = node_deformers(i, &g, [10.0, 10.0]).unwrap_err();
    assert!(e.contains("s/deform[0]/modifierType[0]"), "{e}");
}
