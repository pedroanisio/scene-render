//! Connectors (SREP 16): a line between two nodes, or a node and a point, recomputed from the ends' poses as
//! drawn at every evaluated time.
//!
//! The geometry is computed after every pose of the frame is final (layout, transform parents, constraints and
//! simulated bodies), in the connector's space: the space of its siblings, whose transform C is the connector's
//! world transform (a connector has no transform of its own). Per end:
//!
//! * a **node end** maps the target's box (0, 0)–(w, h) by Q = C⁻¹ · W into connector space, a parallelogram;
//!   with anchor `auto` its reference point is the box centre and the end is clipped to the parallelogram;
//!   an anchor keyword or an explicit `fromX`/`fromY` picks a box point and is not clipped;
//! * a **point end** is (`fromX`, `fromY`), `%` of the parent's box, and is never clipped.
//!
//! The route (straight, orthogonal or one cubic, sampled by the 32-sample table of SREP 58) is cut at the first meeting
//! with the from-outline and the last with the to-outline, less the gaps, giving the visible path V. A missing
//! end, a clipped end the route never meets, or ends that cross leave the connector undrawn.

use std::collections::HashMap;
use std::sync::Arc;

use sr_model::element::{AttrValue, Element};
use sr_model::values::Length;

use crate::eval::{Affine, FrameNode};
use crate::path::{arc_table, locate, Seg};
use crate::program::Program;
use crate::value::Value;

/// The control-point distance of a curved route as a fraction of the chord (TikZ `bend` with `looseness=1`).
const BEND_LOOSENESS: f64 = 0.3915;

/// What a connector draws at one time, in connector space.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Connector {
    /// The visible path V: the route between the clipped, gapped ends, in the direction of travel.
    pub path: Vec<[f64; 2]>,
    /// Bounding box of V (x0, y0, x1, y1) before trim: the box `url(#…)` paints fill.
    pub bounds: [f64; 4],
    /// Where the label goes, when the connector has one.
    pub label: Option<LabelPlace>,
    /// The original cubic and its measure, retained so trims and labels evaluate the true curve.
    #[serde(skip_serializing_if = "Option::is_none")]
    curve: Option<CurveMeasure>,
}

/// SREP 58 measure of one curved route, with the visible interval in original route length units.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
struct CurveMeasure {
    controls: [[f64; 2]; 4],
    table: Vec<(f64, usize, f64)>,
    visible: [f64; 2],
}

impl CurveMeasure {
    fn new(controls: [[f64; 2]; 4]) -> Self {
        let [a, b, c, d] = controls;
        let (table, length) = arc_table(&[Seg::Cubic(a, b, c, d)], &[]);
        Self { controls, table, visible: [0.0, length] }
    }

    fn segment(&self) -> Seg {
        let [a, b, c, d] = self.controls;
        Seg::Cubic(a, b, c, d)
    }

    fn at(&self, length: f64) -> ([f64; 2], [f64; 2]) {
        let (_, t) = locate(&self.table, length);
        let seg = self.segment();
        let dir = seg.tangent(t.clamp(0.0, 1.0));
        let norm = libm::hypot(dir[0], dir[1]);
        (seg.at(t.clamp(0.0, 1.0)), if norm > 0.0 { [dir[0] / norm, dir[1] / norm] } else { [0.0, 0.0] })
    }

    /// A crossing of an infinite line, bracketed by two entries of the shared length table.
    fn crossing(&self, mut a: f64, mut b: f64, origin: [f64; 2], normal: [f64; 2]) -> Option<f64> {
        let side = |s| {
            let p = self.at(s).0;
            (p[0] - origin[0]) * normal[0] + (p[1] - origin[1]) * normal[1]
        };
        let fa = side(a);
        let fb = side(b);
        if fa == 0.0 {
            return Some(a);
        }
        if fb == 0.0 {
            return Some(b);
        }
        if fa.signum() == fb.signum() {
            return None;
        }
        for _ in 0..48 {
            let m = (a + b) / 2.0;
            if side(m).signum() == fa.signum() {
                a = m;
            } else {
                b = m;
            }
        }
        Some((a + b) / 2.0)
    }

    /// Outline crossings on the true cubic, expressed in the shared table's length units.
    fn meetings(&self, outline: &[[f64; 2]; 4]) -> Vec<f64> {
        let mut out = Vec::new();
        for w in self.table.windows(2) {
            let (a, b) = (w[0].0, w[1].0);
            let (p, q) = (self.at(a).0, self.at(b).0);
            for k in 0..4 {
                let (e0, e1) = (outline[k], outline[(k + 1) % 4]);
                let e = [e1[0] - e0[0], e1[1] - e0[1]];
                let len2 = e[0] * e[0] + e[1] * e[1];
                if len2 == 0.0 || b <= a {
                    continue;
                }
                let normal = [e[1], -e[0]];
                let side = |p: [f64; 2]| (p[0] - e0[0]) * normal[0] + (p[1] - e0[1]) * normal[1];
                if side(p).abs().max(side(q).abs()) <= 1e-12 * libm::sqrt(len2) {
                    // Collinear: retain the overlap's endpoints, as for a polyline.
                    out.extend([e0, e1].into_iter().filter_map(|end| self.crossing(a, b, end, e)));
                } else if let Some(s) = self.crossing(a, b, e0, normal) {
                    let p = self.at(s).0;
                    let u = ((p[0] - e0[0]) * e[0] + (p[1] - e0[1]) * e[1]) / len2;
                    if (-1e-12..=1.0 + 1e-12).contains(&u) {
                        out.push(s);
                    }
                }
            }
        }
        out
    }

    fn between(&self, start: f64, end: f64) -> Vec<[f64; 2]> {
        let mut pts = vec![self.at(start).0];
        let seg = self.segment();
        pts.extend(self.table.iter().filter(|&&(l, _, _)| l > start && l < end).map(|&(_, _, t)| seg.at(t)));
        pts.push(self.at(end).0);
        pts.dedup();
        pts
    }
}

impl Connector {
    /// Unit directions of travel at the first and last trimmed endpoints of a curved route.
    /// Polyline routes return `None`, so their markers use the existing outgoing segment rule.
    pub fn trimmed_directions(&self, start: f64, end: f64, offset: f64) -> Option<[[f64; 2]; 2]> {
        let c = self.curve.as_ref()?;
        let ranges = sr_vector::measure::ranges(start, end, offset);
        let a = ranges.first()?.0;
        let b = ranges.last()?.1;
        let [s0, s1] = c.visible;
        Some([c.at(s0 + a * (s1 - s0)).1, c.at(s0 + b * (s1 - s0)).1])
    }

    /// The drawn subpaths after trim. Curved endpoints use the original SREP 58 length table and lie on the
    /// exact cubic; straight and orthogonal routes retain their polyline measure. `offset` is in turns.
    pub fn trimmed_path(&self, start: f64, end: f64, offset: f64) -> Vec<Vec<[f64; 2]>> {
        let ranges = sr_vector::measure::ranges(start, end, offset);
        if let Some(curve) = &self.curve {
            let [s0, s1] = curve.visible;
            ranges.into_iter().map(|(a, b)| curve.between(s0 + a * (s1 - s0), s0 + b * (s1 - s0))).collect()
        } else {
            let total = length(&self.path);
            ranges.into_iter().map(|(a, b)| between(&self.path, a * total, b * total)).collect()
        }
    }
}

/// The placement of a connector's label: its box is centred on `center` and turned by `angle` degrees.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct LabelPlace {
    /// P + `labelOffset` · n, in connector space.
    pub center: [f64; 2],
    /// Rotation, degrees clockwise: 0 for `labelOrient="horizontal"`.
    pub angle: f64,
}

/// A connector of the program: its node and the effective ids of its ends and label layer.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectorInst {
    /// The connector's effective id.
    pub id: Arc<str>,
    /// The `from` node's effective id, for a node end.
    pub from: Option<Arc<str>>,
    /// The `to` node's effective id, for a node end.
    pub to: Option<Arc<str>>,
    /// The label layer's effective id.
    pub label: Option<Arc<str>>,
}

/// Attribute reading: the animated value first, then the document's.
struct Attrs<'a> {
    n: &'a FrameNode,
}

impl Attrs<'_> {
    fn num(&self, name: &str, default: f64) -> f64 {
        if let Some(v) = self.n.props.get(name).and_then(Value::as_num) {
            return v;
        }
        match self.n.elem.get_attr(name) {
            Some(AttrValue::Num(v)) => v,
            Some(AttrValue::Length(l)) => l.value,
            _ => default,
        }
    }

    fn len(&self, name: &str) -> Option<Length> {
        match self.n.props.get(name) {
            Some(Value::Len(l)) => return Some(*l),
            Some(Value::Num(x)) => return Some(Length::px(*x)),
            _ => {}
        }
        match self.n.elem.get_attr(name) {
            Some(AttrValue::Length(l)) => Some(l),
            Some(AttrValue::Num(x)) => Some(Length::px(x)),
            _ => None,
        }
    }

    fn str(&self, name: &str) -> Option<String> {
        if let Some(Value::Str(s)) = self.n.props.get(name) {
            return Some(s.to_string());
        }
        self.n.elem.get_attr(name).map(|a| a.to_string())
    }

    fn points(&self) -> Vec<[f64; 2]> {
        match self.n.elem.get_attr("points") {
            Some(AttrValue::Tokens(t)) => t
                .iter()
                .filter_map(|p| {
                    let (x, y) = p.split_once(',')?;
                    Some([x.trim().parse().ok()?, y.trim().parse().ok()?])
                })
                .collect(),
            _ => Vec::new(),
        }
    }
}

/// One end in connector space: its reference point and, for a clipped end, its outline.
struct End {
    at: [f64; 2],
    outline: Option<[[f64; 2]; 4]>,
}

/// The box of a frame node in its own node space (x0, y0, x1, y1): its size, or for a group the union of its own
/// box and the boxes of its sized descendants (the box chromatic aberration measures too).
fn node_box(nodes: &[FrameNode], kids: &[Vec<usize>], i: usize) -> Option<[f64; 4]> {
    let n = &nodes[i];
    if n.kind != "group" {
        return n.size.map(|[w, h]| [0.0, 0.0, w, h]);
    }
    let inv = n.world.inverse()?;
    let mut b = n.size.map(|[w, h]| [0.0, 0.0, w, h]);
    let mut stack: Vec<usize> = kids[i].clone();
    while let Some(k) = stack.pop() {
        if let Some([w, h]) = nodes[k].size {
            let r = inv.then(&nodes[k].world).bbox([0.0, 0.0, w, h]);
            b = Some(match b {
                Some(o) => [o[0].min(r[0]), o[1].min(r[1]), o[2].max(r[2]), o[3].max(r[3])],
                None => r,
            });
        }
        stack.extend(kids[k].iter().copied());
    }
    b
}

/// The box point an anchor keyword names, for the box (x0, y0, x1, y1).
fn anchor_point(kw: &str, b: [f64; 4]) -> [f64; 2] {
    let (x0, y0, x1, y1) = (b[0], b[1], b[2], b[3]);
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    match kw {
        "top" => [cx, y0],
        "right" => [x1, cy],
        "bottom" => [cx, y1],
        "left" => [x0, cy],
        "top-left" => [x0, y0],
        "top-right" => [x1, y0],
        "bottom-right" => [x1, y1],
        "bottom-left" => [x0, y1],
        _ => [cx, cy],
    }
}

#[allow(clippy::too_many_arguments)]
fn end(
    a: &Attrs,
    side: &str,
    target: Option<usize>,
    nodes: &[FrameNode],
    kids: &[Vec<usize>],
    c_inv: &Affine,
    parent_box: [f64; 2],
    frame: [f64; 2],
) -> End {
    let (kx, ky) = (a.len(&format!("{side}X")), a.len(&format!("{side}Y")));
    let Some(t) = target else {
        // a point end
        let x = kx.map_or(0.0, |l| l.resolve(parent_box[0], frame[0], frame[1]));
        let y = ky.map_or(0.0, |l| l.resolve(parent_box[1], frame[0], frame[1]));
        return End { at: [x, y], outline: None };
    };
    let q = c_inv.then(&nodes[t].world);
    let b = node_box(nodes, kids, t).unwrap_or([0.0; 4]);
    let (w, h) = (b[2] - b[0], b[3] - b[1]);
    let keyword = a.str(&format!("{side}Anchor")).unwrap_or_else(|| "auto".into());
    let local = match (kx, ky) {
        (Some(x), Some(y)) => [b[0] + x.resolve(w, frame[0], frame[1]), b[1] + y.resolve(h, frame[0], frame[1])],
        _ => anchor_point(&keyword, b),
    };
    let [a0, b0, c0, d0, _, _] = q.0;
    let singular = (a0 * d0 - b0 * c0).abs() < 1e-12;
    let clipped = keyword == "auto" && kx.is_none() && ky.is_none() && w > 0.0 && h > 0.0 && !singular;
    let outline = clipped.then(|| [[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]].map(|p| q.apply(p)));
    End { at: q.apply(local), outline }
}

fn curve_controls(a: [f64; 2], b: [f64; 2], bend: f64) -> [[f64; 2]; 4] {
    let phi = libm::atan2(b[1] - a[1], b[0] - a[0]);
    let beta = bend.to_radians();
    let l = BEND_LOOSENESS * libm::hypot(b[0] - a[0], b[1] - a[1]);
    [
        a,
        [a[0] + l * libm::cos(phi - beta), a[1] + l * libm::sin(phi - beta)],
        [b[0] - l * libm::cos(phi + beta), b[1] - l * libm::sin(phi + beta)],
        b,
    ]
}

/// The route as a polyline (SREP 16 §3).
fn route(kind: &str, a: [f64; 2], b: [f64; 2], waypoints: &[[f64; 2]], bend: f64) -> Vec<[f64; 2]> {
    let mut pts: Vec<[f64; 2]> = Vec::new();
    match kind {
        "curved" => {
            let curve = CurveMeasure::new(curve_controls(a, b, bend));
            pts = curve.between(0.0, curve.visible[1]);
        }
        "orthogonal" if waypoints.is_empty() => {
            if (b[0] - a[0]).abs() >= (b[1] - a[1]).abs() {
                let m = (a[0] + b[0]) / 2.0;
                pts.extend([a, [m, a[1]], [m, b[1]], b]);
            } else {
                let m = (a[1] + b[1]) / 2.0;
                pts.extend([a, [a[0], m], [b[0], m], b]);
            }
        }
        "orthogonal" => {
            let all: Vec<[f64; 2]> =
                std::iter::once(a).chain(waypoints.iter().copied()).chain(std::iter::once(b)).collect();
            pts.push(a);
            for w in all.windows(2) {
                let (p, q) = (w[0], w[1]);
                let elbow = if (q[0] - p[0]).abs() >= (q[1] - p[1]).abs() { [q[0], p[1]] } else { [p[0], q[1]] };
                pts.extend([elbow, q]);
            }
        }
        _ => {
            pts.push(a);
            pts.extend(waypoints.iter().copied());
            pts.push(b);
        }
    }
    pts.dedup();
    pts
}

/// Arc lengths along `pts` at which the polyline meets the closed outline `o`.
fn meetings(pts: &[[f64; 2]], o: &[[f64; 2]; 4]) -> Vec<f64> {
    let mut out = Vec::new();
    let mut acc = 0.0;
    for w in pts.windows(2) {
        let (p, q) = (w[0], w[1]);
        let d = [q[0] - p[0], q[1] - p[1]];
        let len = libm::hypot(d[0], d[1]);
        for k in 0..4 {
            let (e0, e1) = (o[k], o[(k + 1) % 4]);
            let e = [e1[0] - e0[0], e1[1] - e0[1]];
            let den = d[0] * e[1] - d[1] * e[0];
            let r = [e0[0] - p[0], e0[1] - p[1]];
            if den.abs() > 1e-12 {
                let t = (r[0] * e[1] - r[1] * e[0]) / den;
                let u = (r[0] * d[1] - r[1] * d[0]) / den;
                if (-1e-12..=1.0 + 1e-12).contains(&t) && (-1e-12..=1.0 + 1e-12).contains(&u) {
                    out.push(acc + t.clamp(0.0, 1.0) * len);
                }
            } else if (r[0] * d[1] - r[1] * d[0]).abs() <= 1e-9 * len.max(1.0) && len > 0.0 {
                // collinear: the overlap's ends along the segment
                for pnt in [e0, e1] {
                    let t = ((pnt[0] - p[0]) * d[0] + (pnt[1] - p[1]) * d[1]) / (len * len);
                    if (0.0..=1.0).contains(&t) {
                        out.push(acc + t * len);
                    }
                }
            }
        }
        acc += len;
    }
    out
}

/// The part of `pts` between arc lengths `s0` and `s1`.
fn between(pts: &[[f64; 2]], s0: f64, s1: f64) -> Vec<[f64; 2]> {
    let mut out = Vec::new();
    let mut acc = 0.0;
    for w in pts.windows(2) {
        let (p, q) = (w[0], w[1]);
        let len = libm::hypot(q[0] - p[0], q[1] - p[1]);
        let at = |s: f64| {
            let t = if len > 0.0 { ((s - acc) / len).clamp(0.0, 1.0) } else { 0.0 };
            [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]
        };
        if out.is_empty() && s0 <= acc + len {
            out.push(at(s0));
        }
        if !out.is_empty() {
            if s1 <= acc + len {
                out.push(at(s1));
                break;
            }
            out.push(q);
        }
        acc += len;
    }
    out.dedup();
    out
}

/// Point and unit direction at arc length `s` along `pts` (the outgoing segment at a vertex).
fn at_length(pts: &[[f64; 2]], s: f64) -> Option<([f64; 2], [f64; 2])> {
    let mut acc = 0.0;
    let mut last = None;
    for w in pts.windows(2) {
        let (p, q) = (w[0], w[1]);
        let len = libm::hypot(q[0] - p[0], q[1] - p[1]);
        if len <= 0.0 {
            continue;
        }
        let dir = [(q[0] - p[0]) / len, (q[1] - p[1]) / len];
        if s < acc + len {
            let t = ((s - acc) / len).max(0.0);
            return Some(([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t], dir));
        }
        last = Some((q, dir));
        acc += len;
    }
    last
}

fn length(pts: &[[f64; 2]]) -> f64 {
    pts.windows(2).map(|w| libm::hypot(w[1][0] - w[0][0], w[1][1] - w[0][1])).sum()
}

/// Computes every connector of the frame (and places its label layer) from the poses in `nodes`, which must be
/// final. Run again after anything moves a node (simulated bodies); a connector whose ends are absent, never met
/// or crossed draws nothing.
pub fn resolve(p: &Program, nodes: &mut [FrameNode]) {
    if p.connectors.is_empty() {
        return;
    }
    let index: HashMap<&str, usize> = nodes.iter().enumerate().map(|(i, n)| (&*n.id, i)).collect();
    let mut kids: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for (i, n) in nodes.iter().enumerate() {
        if let Some(par) = n.parent {
            kids[par as usize].push(i);
        }
    }
    let mut results: Vec<(usize, Option<Connector>, Option<usize>)> = Vec::new();
    for c in &p.connectors {
        let Some(&ci) = index.get(&*c.id) else { continue };
        let label_ix = c.label.as_ref().and_then(|l| index.get(&**l).copied());
        results.push((ci, geometry(p, c, ci, nodes, &kids, &index), label_ix));
    }
    for (ci, geom, label_ix) in results {
        if let Some(li) = label_ix {
            match geom.as_ref().and_then(|g| g.label) {
                Some(place) => {
                    let [w, h] = nodes[li].size.unwrap_or([0.0, 0.0]);
                    let local = Affine::translate(place.center[0], place.center[1])
                        .then(&Affine::rotate(place.angle))
                        .then(&Affine::translate(-w / 2.0, -h / 2.0));
                    nodes[li].local = local;
                    nodes[li].world = nodes[ci].world.then(&local);
                }
                None => nodes[li].draw = false,
            }
        }
        nodes[ci].connector = geom.map(Arc::new);
    }
}

fn geometry(
    p: &Program,
    c: &ConnectorInst,
    ci: usize,
    nodes: &[FrameNode],
    kids: &[Vec<usize>],
    index: &HashMap<&str, usize>,
) -> Option<Connector> {
    let n = &nodes[ci];
    let a = Attrs { n };
    let target = |id: &Option<Arc<str>>| -> Result<Option<usize>, ()> {
        match id {
            Some(t) => index.get(&**t).copied().map(Some).ok_or(()),
            None => Ok(None),
        }
    };
    // an end whose target is not in the frame (outside its window, or a condition that does not hold) is absent
    let (from, to) = (target(&c.from).ok()?, target(&c.to).ok()?);
    let c_inv = n.world.inverse()?;
    let mut parent_box = p.size;
    let mut up = n.parent;
    while let Some(k) = up {
        if let Some(s) = nodes[k as usize].size {
            parent_box = s;
            break;
        }
        up = nodes[k as usize].parent;
    }
    let e0 = end(&a, "from", from, nodes, kids, &c_inv, parent_box, p.size);
    let e1 = end(&a, "to", to, nodes, kids, &c_inv, parent_box, p.size);
    let kind = a.str("route").unwrap_or_else(|| "straight".into());
    let pts = route(&kind, e0.at, e1.at, &a.points(), a.num("bend", 30.0));
    let mut curve = (kind == "curved").then(|| CurveMeasure::new(curve_controls(e0.at, e1.at, a.num("bend", 30.0))));
    let total = curve.as_ref().map_or_else(|| length(&pts), |c| c.visible[1]);
    let crossings = |o| curve.as_ref().map_or_else(|| meetings(&pts, o), |c| c.meetings(o));
    let mut s0 = match &e0.outline {
        Some(o) => crossings(o).into_iter().fold(None, |m: Option<f64>, s| Some(m.map_or(s, |m| m.min(s))))?,
        None => 0.0,
    };
    let mut s1 = match &e1.outline {
        Some(o) => crossings(o).into_iter().fold(None, |m: Option<f64>, s| Some(m.map_or(s, |m| m.max(s))))?,
        None => total,
    };
    s0 += a.num("fromGap", 0.0);
    s1 -= a.num("toGap", 0.0);
    if s1 <= s0 {
        return None;
    }
    s0 = s0.clamp(0.0, total);
    s1 = s1.clamp(0.0, total);
    if let Some(c) = &mut curve {
        c.visible = [s0, s1];
    }
    let path = curve.as_ref().map_or_else(|| between(&pts, s0, s1), |c| c.between(s0, s1));
    if path.len() < 2 {
        return None;
    }
    let mut bounds = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for q in &path {
        bounds = [bounds[0].min(q[0]), bounds[1].min(q[1]), bounds[2].max(q[0]), bounds[3].max(q[1])];
    }
    let label = c.label.as_ref().and_then(|_| {
        let s = s0 + a.num("labelAt", 0.5) * (s1 - s0);
        let (pt, dir) = curve.as_ref().map(|c| c.at(s)).or_else(|| at_length(&pts, s))?;
        let off = a.num("labelOffset", 0.0);
        // the normal turned 90° clockwise on screen from the direction of travel
        let center = [pt[0] - dir[1] * off, pt[1] + dir[0] * off];
        let angle = if a.str("labelOrient").as_deref() == Some("along") {
            let theta = libm::atan2(dir[1], dir[0]).to_degrees();
            if theta > -90.0 && theta <= 90.0 {
                theta
            } else {
                theta + 180.0
            }
        } else {
            0.0
        };
        Some(LabelPlace { center, angle })
    });
    Some(Connector { path, bounds, label, curve })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orthogonal_without_waypoints_is_a_z() {
        assert_eq!(
            route("orthogonal", [0.0, 0.0], [10.0, 4.0], &[], 30.0),
            vec![[0.0, 0.0], [5.0, 0.0], [5.0, 4.0], [10.0, 4.0]]
        );
        assert_eq!(
            route("orthogonal", [0.0, 0.0], [4.0, 10.0], &[], 30.0),
            vec![[0.0, 0.0], [0.0, 5.0], [4.0, 5.0], [4.0, 10.0]]
        );
        // aligned ends: the elbows coincide and drop out
        assert_eq!(route("orthogonal", [0.0, 0.0], [10.0, 0.0], &[], 30.0), vec![[0.0, 0.0], [5.0, 0.0], [10.0, 0.0]]);
    }

    #[test]
    fn curved_route_is_the_32_sample_table_on_the_true_cubic() {
        let (a, b) = ([0.0, 0.0], [100.0, 0.0]);
        let pts = route("curved", a, b, &[], 30.0);
        // the first point, then 32 points per piece (SREP 58), the last the end
        assert_eq!(pts.len(), 33);
        assert_eq!(pts[0], a);
        assert_eq!(pts[32], b);
        // control points of the bend (TikZ looseness 1) and the cubic's value at t = k/32, from the Bernstein form
        let l = BEND_LOOSENESS * 100.0;
        let (beta, phi) = (30f64.to_radians(), 0.0f64);
        let c1 = [l * (phi - beta).cos(), l * (phi - beta).sin()];
        let c2 = [100.0 - l * (phi + beta).cos(), -l * (phi + beta).sin()];
        for k in [1usize, 5, 16, 31] {
            let t = k as f64 / 32.0;
            let (m, t2) = (1.0 - t, t * t);
            let x = m * m * m * a[0] + 3.0 * m * m * t * c1[0] + 3.0 * m * t2 * c2[0] + t2 * t * b[0];
            let y = m * m * m * a[1] + 3.0 * m * m * t * c1[1] + 3.0 * m * t2 * c2[1] + t2 * t * b[1];
            assert!((pts[k][0] - x).abs() < 1e-9 && (pts[k][1] - y).abs() < 1e-9, "k = {k}: {:?} vs {x}, {y}", pts[k]);
        }
        // the apex (t = 1/2) is a table point
        assert!((pts[16][0] - 50.0).abs() < 1e-9);
    }

    #[test]
    fn meetings_and_between() {
        let sq = [[1.0, -1.0], [3.0, -1.0], [3.0, 1.0], [1.0, 1.0]];
        let pts = [[0.0, 0.0], [10.0, 0.0]];
        let mut m = meetings(&pts, &sq);
        m.sort_by(f64::total_cmp);
        assert_eq!(m, vec![1.0, 3.0]);
        assert_eq!(between(&[[0.0, 0.0], [4.0, 0.0], [4.0, 4.0]], 2.0, 6.0), vec![[2.0, 0.0], [4.0, 0.0], [4.0, 2.0]]);
        assert_eq!(between(&[[0.0, 0.0], [4.0, 0.0], [4.0, 4.0]], 4.0, 6.0), vec![[4.0, 0.0], [4.0, 2.0]]);
    }

    #[test]
    fn at_length_takes_the_outgoing_segment() {
        let (p, d) = at_length(&[[0.0, 0.0], [4.0, 0.0], [4.0, 4.0]], 4.0).unwrap();
        assert_eq!(p, [4.0, 0.0]);
        assert_eq!(d, [0.0, 1.0]);
        let (p, d) = at_length(&[[0.0, 0.0], [4.0, 0.0], [4.0, 4.0]], 8.0).unwrap();
        assert_eq!(p, [4.0, 4.0]);
        assert_eq!(d, [0.0, 1.0]);
    }
}
