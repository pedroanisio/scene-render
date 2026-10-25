//! Per-frame evaluation: `evaluate(program, t) → FrameGraph`.
//!
//! A frame is a pure function of the program and the time. Evaluation runs
//! three passes: node timelines top-down (group offsets, repeat staggers,
//! symbol clocks), property slots in dependency order (keyframes, links,
//! expressions), then transforms, opacity and activity into the flat,
//! paint-ordered FrameGraph.

use std::borrow::Cow;
use std::sync::Arc;

use serde::ser::{SerializeMap, Serializer};
use sr_model::model::{Node, TimeBase};
use sr_model::values::Length;

use crate::expr::vm::{self, Band, Host, LoopKind, Var, V};
use crate::program::{marker_time, tfi, Clock, InstNode, Kind, LinkSource, Owner, Program};
use crate::rng;
use crate::value::Value;

/// An evaluated opacity held to the range the schema gives it, 0 to 1 (NaN counts as 0).
fn unit_opacity(x: f64) -> f64 {
    if x.is_nan() {
        0.0
    } else {
        x.clamp(0.0, 1.0)
    }
}

/// A 2D affine transform `[a, b, c, d, e, f]`: x' = a·x + c·y + e, y' = b·x + d·y + f.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Affine(pub [f64; 6]);

impl Affine {
    /// Identity.
    pub const IDENTITY: Affine = Affine([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    /// `self · other` (apply `other` first).
    pub fn then(&self, other: &Affine) -> Affine {
        let [a, b, c, d, e, f] = self.0;
        let [a2, b2, c2, d2, e2, f2] = other.0;
        Affine([
            a * a2 + c * b2,
            b * a2 + d * b2,
            a * c2 + c * d2,
            b * c2 + d * d2,
            a * e2 + c * f2 + e,
            b * e2 + d * f2 + f,
        ])
    }

    /// Translation.
    pub fn translate(x: f64, y: f64) -> Affine {
        Affine([1.0, 0.0, 0.0, 1.0, x, y])
    }

    /// Rotation in degrees, clockwise on screen (+y down).
    pub fn rotate(deg: f64) -> Affine {
        if deg == 0.0 {
            return Affine::IDENTITY;
        }
        let r = deg.to_radians();
        let (s, c) = (libm::sin(r), libm::cos(r));
        Affine([c, s, -s, c, 0.0, 0.0])
    }

    /// Scale.
    pub fn scale(x: f64, y: f64) -> Affine {
        Affine([x, 0.0, 0.0, y, 0.0, 0.0])
    }

    /// Skew in degrees along x and y.
    pub fn skew(x: f64, y: f64) -> Affine {
        if x == 0.0 && y == 0.0 {
            return Affine::IDENTITY;
        }
        Affine([1.0, libm::tan(y.to_radians()), libm::tan(x.to_radians()), 1.0, 0.0, 0.0])
    }

    /// Transforms a point.
    pub fn apply(&self, p: [f64; 2]) -> [f64; 2] {
        let [a, b, c, d, e, f] = self.0;
        [a * p[0] + c * p[1] + e, b * p[0] + d * p[1] + f]
    }

    /// Inverse transform, when the matrix is invertible.
    pub fn inverse(&self) -> Option<Affine> {
        let [a, b, c, d, e, f] = self.0;
        let det = a * d - b * c;
        if det.abs() < 1e-12 || !det.is_finite() {
            return None;
        }
        let (ia, ib, ic, id) = (d / det, -b / det, -c / det, a / det);
        Some(Affine([ia, ib, ic, id, -(ia * e + ic * f), -(ib * e + id * f)]))
    }

    /// Axis-aligned bounds of a transformed rectangle x0, y0, x1, y1.
    pub fn bbox(&self, r: [f64; 4]) -> [f64; 4] {
        let pts =
            [self.apply([r[0], r[1]]), self.apply([r[2], r[1]]), self.apply([r[2], r[3]]), self.apply([r[0], r[3]])];
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for q in pts {
            b[0] = b[0].min(q[0]);
            b[1] = b[1].min(q[1]);
            b[2] = b[2].max(q[0]);
            b[3] = b[3].max(q[1]);
        }
        b
    }
}

/// Property values of one element, serialised as a map.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Props(pub Vec<(Arc<str>, Value)>);

impl Props {
    /// Value of a property.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.0.iter().find(|(k, _)| &**k == name).map(|(_, v)| v)
    }
}

impl serde::Serialize for Props {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(Some(self.0.len()))?;
        for (k, v) in &self.0 {
            m.serialize_entry(&**k, v)?;
        }
        m.end()
    }
}

/// Animated state of a non-node element (paint, effect, mask, modifier, …).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ElementState {
    /// Id, or path relative to its owner.
    pub key: Arc<str>,
    /// Element name.
    pub element: &'static str,
    /// Current values of its animated properties.
    pub props: Props,
}

/// Named joint frames in column-major scene coordinates.
pub type JointFrames = Vec<(String, [f64; 16])>;

/// One node of the frame, in paint order.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FrameNode {
    /// Effective id.
    pub id: Arc<str>,
    /// The id values derived from the node's id use (SREP 87, Semantics 7): see [`crate::program::InstNode::seed_id`].
    pub seed_id: Arc<str>,
    /// The scope the node's references (`@parent`, `@target`, `@blackHole`...) resolve in: the instance, repeat copy
    /// or include it is in ("" at the top level). See [`resolve_in`].
    #[serde(skip)]
    pub scope: Arc<str>,
    /// The document the node is written in: 0 for the main document, then the includes in the order of
    /// [`crate::Program::includes`]. Its references to assets, effects and materials name that document's own
    /// (an include merges them under its namespace; see [`crate::Program::doc_key`]).
    #[serde(skip)]
    pub doc: u16,
    /// Element name (`copy` for repeat copies).
    pub kind: &'static str,
    /// Index of the container node in `FrameGraph::nodes`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<u32>,
    /// Transform parent (`@parent` when present, otherwise the container).
    #[serde(skip)]
    pub transform_parent: Option<u32>,
    /// Tree depth.
    pub depth: u32,
    /// The node's stacking `z` at this frame (0 for the 3D kinds): siblings paint in this order, and captions take
    /// their place among the top-level nodes by it.
    #[serde(skip)]
    pub z: i32,
    /// Local transform.
    pub local: Affine,
    /// Node space to frame space.
    pub world: Affine,
    /// Own opacity.
    pub opacity: f64,
    /// Opacity including all containers.
    pub world_opacity: f64,
    /// Time on the node's timeline.
    pub timeline_time: f64,
    /// Seconds since the node's start.
    pub local_time: f64,
    /// Source media or symbol time (layers).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_time: Option<f64>,
    /// Asset key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset: Option<Arc<str>>,
    /// Resolved text of a text layer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Arc<str>>,
    /// Whether the node's own content is drawn (hidden, or a matte source only).
    pub draw: bool,
    /// Index of the matte source node.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matte: Option<u32>,
    /// Used as a matte by another node.
    pub is_matte: bool,
    /// Repeat copy index and count, for nodes inside a repeat.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat: Option<[u32; 2]>,
    /// 2.5D placement: zDepth, rotationX, rotationY (when `threeD`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub three_d: Option<[f64; 3]>,
    /// Box in local space (width, height), when the node has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<[f64; 2]>,
    /// Anchor point in local space, pixels.
    pub anchor: [f64; 2],
    /// Media placement of a layer inside its box.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<crate::layout::Content>,
    /// Children are clipped to the box.
    pub clip: bool,
    /// The plots draft: an `axes` node's frame (its mapping, parts and labels), on the `axes` and its labels node.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub axes: Option<Arc<crate::axes::AxesFrame>>,
    /// Current values of the node's animated attributes.
    pub props: Props,
    /// Animated parts inside the node.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<ElementState>,
    /// Bone poses of a skeleton node, in bone order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub bones: Vec<BonePose>,
    /// Explicit skin weights of a skeleton (`@weights`), in skeleton space.
    #[serde(skip)]
    pub skin: Option<Arc<SkinWeights>>,
    /// Soft-body lattice displacement in node space.
    #[serde(skip)]
    pub soft: Option<Arc<crate::sim::SoftWarp>>,
    /// Live particles of an emitter or agents of a flock, in frame space.
    #[serde(skip)]
    pub particles: Option<Arc<crate::sim::ParticleFrame>>,
    /// World-space native 3D particles and instance state.
    #[serde(skip)]
    pub particles3d: Option<Arc<crate::particles3d::SimParticles3D>>,
    #[serde(skip)]
    pub sim_ocean: Option<Arc<crate::ocean::SimOcean>>,
    /// Active fracture replaces the source mesh with these rigid pieces.
    #[serde(skip)]
    pub fracture: Option<Arc<crate::fracture::SimFracture>>,
    /// The picture of a grid simulation (fluid, slime, erosion), filling the node's box.
    #[serde(skip)]
    pub sim_image: Option<Arc<crate::agents::SimImage>>,
    /// Native participating-medium simulation, in the object's local 3D frame.
    #[serde(skip)]
    pub sim_volume: Option<Arc<crate::pyro::SimVolume>>,
    /// A crater that grows from an impact, once the impact has happened (`crater@source`).
    #[serde(skip)]
    pub crater_impact: Option<Arc<crate::crater::ImpactCrater>>,
    /// An object of cells that can be cut, with the cuts it has had and the pieces that have come away.
    #[serde(skip)]
    pub voxels: Option<Arc<crate::voxel_cut::SimVoxels>>,
    /// A 3D rigid body's pose: the object's world matrix (column-major, scene space), replacing
    /// its own transform and parent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pose3: Option<[f64; 16]>,
    /// For an object that draws an imported model and has joint sockets: each joint's frame in the object's own frame
    /// (column-major, scene space), at the pose the object draws.
    #[serde(skip)]
    pub joints: Option<Arc<JointFrames>>,
    /// A connector's geometry at this time (SREP 16): None when it draws nothing, or for other nodes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connector: Option<Arc<crate::connector::Connector>>,
    /// The mesh of a parametric surface or heightfield at this time (SREP 70), in the object's local space.
    #[serde(skip)]
    pub param_mesh: Option<Arc<crate::parametric::ParamMesh>>,
    /// The meshes an axes object draws with other materials than its own (the plots draft): (material id, mesh).
    #[serde(skip)]
    pub param_parts: Vec<(Arc<str>, Arc<crate::parametric::ParamMesh>)>,
    /// The space-transforms draft: the node's `map` modifiers at this frame, by their part key
    /// (`{id}/deform[i]/modifierType[j]`); [`FrameGraph::point_map`] turns one into its point function.
    #[serde(skip)]
    pub deform_maps: Vec<(Arc<str>, MapSpec)>,
    /// A clay object's surface at this time, extracted ahead of rendering ([`crate::solid::attach_clay`]); None when
    /// the renderer extracts it itself.
    #[serde(skip)]
    pub clay_mesh: Option<Arc<crate::solid::ClayMesh>>,
    /// The fields draft: the raster a `fieldRaster` node (a lic or a portrait) draws over its field's box.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raster: Option<Arc<crate::field_node::FieldRaster>>,
    /// The box of a parametric shape at this time (SREP 70): the bounding box `[x0, y0, x1, y1]` of its finite
    /// points, in local pixels. Its outline is the `path` property.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param_box: Option<[f64; 4]>,
    /// The node's element after templating (static attributes).
    #[serde(skip)]
    pub elem: Arc<Node>,
}

/// A bone of a skeleton at this frame.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct BonePose {
    pub id: Arc<str>,
    /// Bone space (origin at the joint, x along the bone) → frame space, animated and constrained.
    pub world: Affine,
    /// The same at rest (document attribute values), under the skeleton's current transform.
    pub rest: Affine,
    pub length: f64,
}

/// Skin weights sampled at points in skeleton space: (point, [(bone index, weight)]).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SkinWeights(pub Vec<SkinSample>);

/// One skin-weight sample: a point in skeleton space and its (bone index, weight) pairs.
pub type SkinSample = ([f64; 2], Vec<(usize, f64)>);

/// An active transition.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct FrameTransition {
    /// Transition type.
    pub kind: Arc<str>,
    /// Outgoing node index.
    pub from: Option<u32>,
    /// Incoming node index.
    pub to: Option<u32>,
    /// Eased progress in [0, 1].
    pub progress: f64,
    /// d(eased progress)/dt in 1/s (0 when unknown).
    #[serde(default)]
    pub velocity: f64,
    /// Animated transition parameters.
    pub props: Props,
    /// The transition element (static attributes), when the transition is authored.
    #[serde(skip)]
    pub elem: Option<Arc<Node>>,
}

/// Wall-clock seconds the simulation runtimes spent advancing one frame, for statistics.
/// Each figure is inclusive: particle emitters that read rigid bodies include that work, and
/// the first frame includes the one-time build of the rigid world.
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize)]
pub struct SimSeconds {
    /// Rigid bodies (building the world on the first frame, then stepping it).
    pub rigid: f64,
    /// Ocean surface solver.
    pub ocean: f64,
    /// Participating-medium (smoke) solver.
    pub smoke: f64,
    /// 2D emitters and native 3D particles.
    pub particles: f64,
}

/// Everything the compositor needs to draw one frame, without pixels.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FrameGraph {
    /// Composition time in seconds.
    pub time: f64,
    /// Frame index.
    pub frame: i64,
    /// Frame size in pixels.
    pub size: [f64; 2],
    /// Project background.
    pub background: Value,
    /// Nodes in paint order (containers before their children, children by z).
    pub nodes: Vec<FrameNode>,
    /// Active transitions.
    pub transitions: Vec<FrameTransition>,
    /// Active camera node.
    pub camera: Option<u32>,
    /// Animated elements outside the composition.
    pub elements: Vec<ElementState>,
    /// Problems found while simulating (physics, particles), reported by the renderer.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<String>,
    /// The part of `problems` that are simulations authored but unable to run (a resource limit, a
    /// solver error): the renderer reports each as an error, never as a note, since the frame lacks
    /// what was asked for. The rest of `problems` are notes about authorised fallbacks.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<String>,
    /// Time the simulation runtimes took to produce this frame.
    #[serde(skip)]
    pub sim_seconds: SimSeconds,
    /// The project seed (the default for seeded behaviour such as camera shake).
    #[serde(skip)]
    pub seed: u64,
    /// The compiled program the graph was evaluated from, which [`FrameGraph::point_map`] runs: set by
    /// [`crate::Evaluator::evaluate`], absent from a graph evaluated from a borrowed program.
    #[serde(skip)]
    pub program: SharedProgram,
}

/// The program behind a frame graph, shared with the point functions made from it.
#[derive(Clone, Default)]
pub struct SharedProgram(pub Option<Arc<Program>>);

impl std::fmt::Debug for SharedProgram {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() { "SharedProgram(Some(..))" } else { "SharedProgram(None)" })
    }
}

/// A `map` modifier at one frame (the space-transforms draft, Semantics 2): its compiled `x` and `y` expressions
/// and the times they read.
#[derive(Debug, Clone, PartialEq)]
pub struct MapSpec {
    /// The part key, `{id}/deform[i]/modifierType[j]`.
    pub key: Arc<str>,
    exprs: [u32; 2],
    comp_t: f64,
    /// Time on the node's timeline.
    t: f64,
}

impl MapSpec {
    /// The values of `x` and `y` at (u, v), each NaN when its expression gives no number. A list of one number is
    /// that number, as a number property reads it: a value asset's state is a list (the reactive-values draft).
    fn point(&self, p: &Program, u: f64, v: f64, memo: &Memo) -> [f64; 2] {
        let mut regs = Vec::new();
        self.exprs.map(|x| {
            let mut h = ExprHost {
                p,
                frame: None,
                comp_t: self.comp_t,
                t: self.t,
                slot: None,
                expr: x,
                value: Value::Num(0.0),
                depth: 0,
                memo,
                sample: [u, v],
                fold: None,
            };
            let r = vm::run(&p.exprs[x as usize].code, &mut h, &mut regs);
            match &r {
                V::Num(_) | V::Bool(_) => r.num(),
                V::Arr(a) if a.len() == 1 && matches!(a[0], V::Num(_) | V::Bool(_)) => a[0].num(),
                _ => f64::NAN,
            }
        })
    }
}

impl FrameGraph {
    /// The point function of a `map` modifier of this frame: (u, v) in data units to the values of its `x` and `y`,
    /// each NaN when its expression gives no number. It holds the program, so it outlives the graph and runs on any
    /// thread, each call with its own registers and memo. Refused when the graph was evaluated without its program.
    pub fn point_map(&self, spec: &MapSpec) -> Result<impl Fn(f64, f64) -> [f64; 2] + Send + Sync + 'static, String> {
        let p = self
            .program
            .0
            .clone()
            .ok_or_else(|| format!("{}: the frame was evaluated without its program", spec.key))?;
        let spec = spec.clone();
        Ok(move |u: f64, v: f64| {
            let memo = Memo::default();
            let xy = spec.point(&p, u, v, &memo);
            // a point that read a failed fold (a non-finite state) is no point; the frame reported the failure
            if memo.failures.borrow().is_empty() {
                xy
            } else {
                [f64::NAN; 2]
            }
        })
    }

    /// Records a simulation that could not run, as a problem and as a failure.
    pub fn fail(&mut self, message: String) {
        self.problems.push(message.clone());
        self.failures.push(message);
    }

    /// Whether the simulation of node `id` failed to run (its entry in `failures` starts with the id).
    pub fn failed(&self, id: &str) -> bool {
        self.failures.iter().any(|f| f.strip_prefix(id).is_some_and(|rest| rest.starts_with(": ")))
    }
}

pub(crate) struct Frame<'p> {
    /// Discovery/endpoint snapshots retain conditionally hidden nodes; ordinary
    /// frames still determine simulation participation at each step boundary.
    include_inactive: bool,
    p: &'p Program,
    t: f64,
    /// Time on each node's own timeline.
    tl: Vec<f64>,
    /// Time each node gives its children.
    child: Vec<f64>,
    values: Vec<Value>,
    regs: Vec<V>,
    /// Layout and alignment adjustments, applied before a node's local transform.
    adjust: Vec<Option<Affine>>,
    /// Whether each node and its ancestors are inside their windows (conditions not yet applied).
    alive: Vec<bool>,
    memo: Memo,
    /// The frames of the live `axes` nodes (the plots draft), by node.
    axes: Vec<(u32, Arc<crate::axes::AxesFrame>)>,
}

/// Values of linked and computed slots away from the frame's own time, by slot, time and depth:
/// what delayed and smoothed links reach, each computed once a frame however many readers it has.
/// Slot values computed off the frame's own time, by (slot, composition time, depth), and the failures of the folds
/// computed for them (the reactive-values draft: a non-finite state is a render error).
#[derive(Default)]
pub(crate) struct Memo {
    values: std::cell::RefCell<std::collections::HashMap<(u32, u64, u32), Value>>,
    failures: std::cell::RefCell<Vec<String>>,
}

impl Memo {
    /// Records a failure once.
    pub(crate) fn fail(&self, message: String) {
        let mut f = self.failures.borrow_mut();
        if !f.contains(&message) {
            f.push(message);
        }
    }

    /// Takes over the failures of `other`.
    pub(crate) fn absorb(&self, other: Memo) {
        for m in other.failures.into_inner() {
            self.fail(m);
        }
    }
}

/// A node's local transform and the quantities layout needs.
struct Local {
    aff: Affine,
    opacity: f64,
    pos: [f64; 2],
    anchor: [f64; 2],
    /// The rotation, degrees, with a motion path's tangent when it orients the node.
    rot: f64,
}

/// Whether a plain node is inside its own window at `tl` on its timeline, `t` being the composition time. Before time 0,
/// which a shutter opening early samples for the first frame (SREP 55), a window that opens at or before 0 is open: the
/// programme's start is not a cut, so the first frame blurs over the scene it starts with.
fn in_own_window(node: &InstNode, tl: f64, t: f64) -> bool {
    (tl >= node.vis_start || (t < 0.0 && node.vis_start <= 0.0)) && node.vis_end.is_none_or(|e| tl < e)
}

/// Whether node `n` is inside its time window at composition time `t`, and so is every ancestor, each
/// ancestor's clock mapping the time for its children: the node is then in the frame graph unless a
/// condition hides it (conditions are not evaluated here).
pub(crate) fn in_window(p: &Program, n: u32, t: f64) -> bool {
    let mut chain = vec![n];
    while let Some(parent) = p.nodes[*chain.last().expect("chain") as usize].parent {
        chain.push(parent);
    }
    let mut tl = t;
    for &k in chain.iter().rev() {
        let node = &p.nodes[k as usize];
        let on = match node.kind {
            Kind::RepeatCopy { .. } => true,
            Kind::Transition(_) => false,
            Kind::Plain => in_own_window(node, tl, t),
        };
        if !on {
            return false;
        }
        tl = clock_map(&node.clock, tl);
    }
    true
}

pub(crate) fn clock_map(c: &Clock, t: f64) -> f64 {
    match c {
        Clock::Same => t,
        Clock::Affine { origin, offset, scale } => origin + (t - origin - offset) * scale,
        Clock::Media(m) => m.map(t),
    }
}

impl Program {
    /// Time on node `n`'s timeline at composition time `t`.
    pub fn timeline_at(&self, n: u32, t: f64) -> f64 {
        match self.nodes[n as usize].parent {
            None => t,
            Some(p) => clock_map(&self.nodes[p as usize].clock, self.timeline_at(p, t)),
        }
    }

    fn slot_time(&self, slot: u32, comp_t: f64, tl: Option<&[f64]>) -> f64 {
        let s = &self.slots[slot as usize];
        let at = |n: u32| match tl {
            Some(tl) => tl[n as usize],
            None => self.timeline_at(n, comp_t),
        };
        match (s.time_node, s.clock) {
            (Some(n), _) => at(n),
            // a definition an include merges: the time of the include's content
            (None, Some(inc)) => clock_map(&self.nodes[inc as usize].clock, at(inc)),
            (None, None) => comp_t,
        }
    }

    fn channel_time(&self, slot: u32, base: TimeBase, t: f64) -> f64 {
        let Some(n) = self.slots[slot as usize].time_node else { return t };
        let node = &self.nodes[n as usize];
        match base {
            TimeBase::Composition => t,
            TimeBase::Local => t - node.start,
            TimeBase::Normalized => {
                let end = node.end.unwrap_or(self.duration);
                let d = end - node.start;
                if d > 0.0 {
                    (t - node.start) / d
                } else {
                    0.0
                }
            }
        }
    }

    /// Keyframed value of a slot at time `t` on its timeline.
    fn channels_at(&self, slot: u32, t: f64) -> Value {
        let s = &self.slots[slot as usize];
        let mut v = s.base.clone();
        for &c in &s.channels {
            let ch = &self.channels[c as usize];
            let cv = ch.eval(self.channel_time(slot, ch.time_base, t));
            v = if ch.additive { v.add(&cv) } else { cv };
        }
        v
    }

    fn beat_at(&self, t: f64) -> f64 {
        if let Some(beats) = &self.analysis.beats {
            if !beats.is_empty() {
                let i = beats.partition_point(|b| *b <= t);
                if i == 0 {
                    return (t - beats[0]) / beats.get(1).map(|b1| b1 - beats[0]).unwrap_or(1.0);
                }
                let (a, b) = (beats[i - 1], beats.get(i).copied().unwrap_or(beats[i - 1] + 1.0));
                return (i - 1) as f64 + (t - a) / (b - a).max(1e-9);
            }
        }
        match self.beat {
            Some(g) => (t - g.offset) * g.bpm / 60.0,
            None => 0.0,
        }
    }

    fn link_value(
        &self,
        slot: u32,
        l: u32,
        comp_t: f64,
        frame: Option<&Frame>,
        depth: u32,
        memo: &Memo,
    ) -> Option<Value> {
        let link = &self.links[l as usize];
        let kind = self.slots[slot as usize].kind;
        let template = &self.slots[slot as usize].base;
        let sample = |ct: f64| -> V {
            match &link.source {
                LinkSource::Param(v) => v.clone(),
                LinkSource::Audio(track, band) => V::Num(self.analysis.amplitude(track, *band, ct)),
                LinkSource::Marker(t0, d) => V::Num(if ct < *t0 {
                    0.0
                } else if *d <= 0.0 || ct >= t0 + d {
                    1.0
                } else {
                    (ct - t0) / d
                }),
                LinkSource::Prop(p) => match frame {
                    Some(f) if ct == f.t => f.values[*p as usize].to_v(),
                    _ => self.value_at(*p, ct, depth + 1, memo).to_v(),
                },
            }
        };
        let t0 = comp_t - link.delay;
        let src = if let Some(follower) = &link.follower {
            // the filter of the source's history: sum of weight x source at (t0 - age), the source holding its value
            // from before time 0 (a constant source gives that constant)
            let step = follower.window / follower.weights.len() as f64;
            let mut acc: Option<Vec<f64>> = None;
            for (i, w) in follower.weights.iter().enumerate() {
                let c = sample((t0 - (i as f64 + 0.5) * step).max(0.0)).components().unwrap_or_default();
                match &mut acc {
                    None => acc = Some(c.iter().map(|x| x * w).collect()),
                    Some(a) => a.iter_mut().zip(&c).for_each(|(x, y)| *x += y * w),
                }
            }
            V::nums(&acc.unwrap_or_default())
        } else if link.smoothing > 0.0 {
            let n = ((link.smoothing * self.fps.as_f64()).round() as usize).clamp(1, 240);
            let mut acc: Option<Vec<f64>> = None;
            for i in 0..n {
                let ct = t0 - link.smoothing * i as f64 / n as f64;
                let c = sample(ct).components().unwrap_or_default();
                match &mut acc {
                    None => acc = Some(c),
                    Some(a) => a.iter_mut().zip(&c).for_each(|(x, y)| *x += y),
                }
            }
            V::nums(&acc.unwrap_or_default().iter().map(|x| x / n as f64).collect::<Vec<_>>())
        } else {
            sample(t0)
        };
        let comps = src.components()?;
        let out: Vec<f64> = comps
            .iter()
            .map(|x| {
                let mut y = x * link.scale + link.offset;
                if let Some(lo) = link.min {
                    y = y.max(lo);
                }
                if let Some(hi) = link.max {
                    y = y.min(hi);
                }
                y
            })
            .collect();
        let v = if out.len() == 1 { V::Num(out[0]) } else { V::nums(&out) };
        kind.from_v(&v, template)
    }

    /// Full value of a slot (keys, link, expression, fold) at composition time
    /// `comp_t`, computed without the frame cache (link delays, smoothing).
    pub(crate) fn value_at(&self, slot: u32, comp_t: f64, depth: u32, memo: &Memo) -> Value {
        let s = &self.slots[slot as usize];
        if depth > 32 || (s.link.is_none() && s.expr.is_none() && s.fold.is_none()) {
            let v = self.channels_at(slot, self.slot_time(slot, comp_t, None));
            return if depth > 32 { v } else { s.kind.clamp(v) };
        }
        let key = (slot, comp_t.to_bits(), depth);
        if let Some(known) = memo.values.borrow().get(&key) {
            return known.clone();
        }
        let v = match s.fold {
            Some(f) => self.fold_value(f, comp_t, depth, memo),
            None => self.unfolded_at(slot, comp_t, depth, memo),
        };
        let v = s.kind.clamp(v);
        memo.values.borrow_mut().insert(key, v.clone());
        v
    }

    /// The value of a slot from its keys, link and expression at composition time `comp_t`, before a fold and
    /// unclamped: a fold's keyed value.
    pub(crate) fn unfolded_at(&self, slot: u32, comp_t: f64, depth: u32, memo: &Memo) -> Value {
        let s = &self.slots[slot as usize];
        let t = self.slot_time(slot, comp_t, None);
        let mut v = self.channels_at(slot, t);
        if let Some(l) = s.link {
            if let Some(lv) = self.link_value(slot, l, comp_t, None, depth, memo) {
                v = lv;
            }
        }
        if let Some(x) = s.expr {
            let mut regs = Vec::new();
            let mut h = ExprHost {
                p: self,
                frame: None,
                comp_t,
                t,
                slot: Some(slot),
                expr: x,
                value: v.clone(),
                depth,
                memo,
                sample: [0.0; 2],
                fold: None,
            };
            let r = vm::run(&self.exprs[x as usize].code, &mut h, &mut regs);
            if let Some(nv) = s.kind.from_v(&r, &v) {
                v = nv;
            }
        }
        v
    }
}

pub(crate) struct ExprHost<'a, 'f> {
    pub(crate) p: &'a Program,
    pub(crate) frame: Option<&'f Frame<'a>>,
    pub(crate) comp_t: f64,
    /// Time on the owner's timeline.
    pub(crate) t: f64,
    pub(crate) slot: Option<u32>,
    pub(crate) expr: u32,
    pub(crate) value: Value,
    pub(crate) depth: u32,
    pub(crate) memo: &'f Memo,
    /// The sampling variables of a parametric geometry expression (SREP 70).
    pub(crate) sample: [f64; 2],
    /// The step of a value's `update` (the reactive-values draft).
    pub(crate) fold: Option<FoldCx<'f>>,
}

/// What a value's `update` sees besides `value` and `time` (the reactive-values draft, Semantics 2 and 5).
pub(crate) struct FoldCx<'f> {
    /// The step or iteration index from 0.
    pub(crate) index: u64,
    /// The step number (index + 1) of a time fold, the `count` of an index fold.
    pub(crate) count: u64,
    /// The `step` of a time fold, 0 in an index fold.
    pub(crate) dt: f64,
    /// A time fold's system: its folds' slots and their states before the step, which reads of them see.
    pub(crate) system: Option<(&'f [u32], &'f [Value])>,
}

impl Host for ExprHost<'_, '_> {
    fn var(&mut self, v: Var) -> V {
        let node = self.p.exprs[self.expr as usize].node.map(|n| &self.p.nodes[n as usize]);
        if let Some(f) = &self.fold {
            match v {
                Var::Index => return V::Num(f.index as f64),
                Var::Count => return V::Num(f.count as f64),
                Var::Dt => return V::Num(f.dt),
                _ => {}
            }
        }
        match v {
            Var::Dt => V::Undef,
            Var::Time => V::Num(self.t),
            Var::Frame => V::Num(libm::floor(self.t * self.p.fps.as_f64() + 1e-9)),
            Var::Value => self.value.to_v(),
            Var::Index => V::Num(node.and_then(|n| n.repeat.as_ref()).map(|r| r.0 as f64).unwrap_or(0.0)),
            Var::Count => V::Num(node.and_then(|n| n.repeat.as_ref()).map(|r| r.1 as f64).unwrap_or(1.0)),
            Var::Seed => V::Num(self.p.exprs[self.expr as usize].seed as f64),
            Var::TextIndex => V::Num(0.0),
            Var::TextTotal => V::Num(1.0),
            Var::Fps => V::Num(self.p.fps.as_f64()),
            Var::Duration => V::Num(self.p.duration),
            Var::Sample(k) => V::Num(self.sample.get(k as usize).copied().unwrap_or(f64::NAN)),
            Var::PointX | Var::PointY | Var::PointAngle | Var::PointU | Var::PointRandom => {
                let Some((g, i)) = node.and_then(|n| n.point) else { return V::Undef };
                let gen = &self.p.points[g as usize];
                match v {
                    Var::PointU => V::Num(gen.u(i)),
                    Var::PointRandom => V::Num(gen.random(i)),
                    _ => {
                        // the sizes at this expression's time, read independently of the slot order
                        let (p, comp_t, depth, memo) = (self.p, self.comp_t, self.depth, self.memo);
                        let live = std::array::from_fn(|k| match gen.slots[k] {
                            Some(s) => p.value_at(s, comp_t, depth + 1, memo).as_num().unwrap_or(gen.base[k]),
                            None => gen.base[k],
                        });
                        let q = gen.point(i, live);
                        V::Num(match v {
                            Var::PointX => q.x,
                            Var::PointY => q.y,
                            _ => q.direction,
                        })
                    }
                }
            }
        }
    }

    fn prop(&mut self, slot: u32) -> V {
        // a time fold reads the folds of its system before the step (gather form)
        if let Some((slots, states)) = self.fold.as_ref().and_then(|f| f.system) {
            if let Some(i) = slots.iter().position(|&s| s == slot) {
                return crate::fold::shown(&states[i]);
            }
        }
        match self.frame {
            Some(f) if self.comp_t == f.t => f.values[slot as usize].to_v(),
            _ => self.p.value_at(slot, self.comp_t, self.depth + 1, self.memo).to_v(),
        }
    }

    fn prop_at(&mut self, slot: u32, t: f64) -> V {
        self.p.value_at(slot, t, self.depth + 1, self.memo).to_v()
    }

    fn value_at_time(&mut self, t: f64) -> V {
        match self.slot {
            Some(s) => self.p.channels_at(s, t).to_v(),
            None => V::Undef,
        }
    }

    fn param(&mut self, name: &str) -> V {
        if let Some(n) = self.p.exprs[self.expr as usize].node {
            if let Some((i, c, var, item)) = &self.p.nodes[n as usize].repeat {
                let (root, field) = name.split_once('.').map(|(a, b)| (a, Some(b))).unwrap_or((name, None));
                if root == &**var {
                    return match (field, item) {
                        (None, v) => v.clone(),
                        (Some(f), V::Obj(o)) => o.get(f).cloned().unwrap_or_default(),
                        _ => V::Undef,
                    };
                }
                match name {
                    "index" => return V::Num(*i as f64),
                    "count" => return V::Num(*c as f64),
                    _ => {}
                }
            }
        }
        let (root, field) = name.split_once('.').map(|(a, b)| (a, Some(b))).unwrap_or((name, None));
        match (self.p.doc_params(self.p.exprs[self.expr as usize].doc).get(root), field) {
            (Some(v), None) => v.clone(),
            (Some(V::Obj(o)), Some(f)) => o.get(f).cloned().unwrap_or_default(),
            _ => V::Undef,
        }
    }

    fn loop_value(&mut self, out: bool, kind: LoopKind, keys: usize) -> V {
        let Some(s) = self.slot else { return V::Undef };
        let slot = &self.p.slots[s as usize];
        match slot.channels.last() {
            Some(&c) => {
                let ch = &self.p.channels[c as usize];
                ch.eval_loop(self.p.channel_time(s, ch.time_base, self.t), out, kind, keys).to_v()
            }
            None => self.value.to_v(),
        }
    }

    fn audio(&mut self, track: &str, band: Band) -> f64 {
        self.p.analysis.amplitude(track, band, self.comp_t)
    }

    fn geo(&mut self, map: &str, lon: f64, lat: f64) -> Option<([f64; 2], bool)> {
        // the map of the expression's own document (in an include, the include's own), also for an expression on one
        // of its definitions, which has no node
        let doc = self.p.exprs[self.expr as usize].doc;
        let mp = crate::geo::map_asset(self.p, doc, map)?;
        let cam = crate::geo::camera(self.p, doc, mp).ok()?;
        let key = self.p.doc_key(doc, map);
        let target = self.p.elements.iter().find(|e| *e.key == *key);
        let (p, comp_t, depth, memo) = (self.p, self.comp_t, self.depth, self.memo);
        let animated = |name: &str| {
            let slot = target?.slots.iter().copied().find(|&s| &*p.slots[s as usize].prop == name)?;
            p.value_at(slot, comp_t, depth + 1, memo).as_num()
        };
        let v = crate::geo::view(&cam, mp, &animated, comp_t);
        Some(crate::geo::locate(&cam, &v, lon, lat))
    }

    fn beat(&mut self) -> f64 {
        self.p.beat_at(self.comp_t)
    }

    fn random(&mut self, site: u32, component: u32) -> f64 {
        if let Some(f) = &self.fold {
            let index = f.index;
            return crate::fold::random(self.noise_seed(), index, site, self.noise_channel(), component);
        }
        // splitmix64 hash of (seed, frame, call site, property): the lattice hash of `rng`
        // with the frame as the channel; an array's component k adds k · 2⁴⁸
        let frame = libm::floor(self.t * self.p.fps.as_f64() + 1e-9) as i64;
        let index = site as u64 + (self.noise_channel() << 32) + ((component as u64) << 48);
        rng::d24_unit(self.noise_seed(), frame as u64, index)
    }

    fn draw(&mut self, channel: f64, i: f64) -> f64 {
        match &self.fold {
            Some(f) => crate::fold::draw(self.p.exprs[self.expr as usize].seed, f.index, channel, i),
            None => f64::NAN,
        }
    }

    fn noise_seed(&mut self) -> u64 {
        self.p.exprs[self.expr as usize].seed
    }

    fn noise_channel(&mut self) -> u64 {
        match self.slot {
            Some(s) => self.p.slots[s as usize].noise_channel,
            None => vm::OTHER_CHANNEL,
        }
    }
}

fn resolve_len(l: Length, parent: f64, frame: [f64; 2]) -> f64 {
    l.resolve(parent, frame[0], frame[1])
}

fn component(v: &Value, c: Option<u8>) -> Option<Value> {
    match (v, c) {
        (Value::Pair(p), Some(i)) => Some(Value::Len(p[i as usize])),
        (v, None) => Some(v.clone()),
        _ => None,
    }
}

impl<'p> Frame<'p> {
    fn timelines(&mut self, clocks: &[(u32, f64)]) {
        let p = self.p;
        let mut stack: Vec<(u32, f64, bool)> = p.roots.iter().rev().map(|&r| (r, self.t, true)).collect();
        while let Some((n, t, up)) = stack.pop() {
            self.tl[n as usize] = t;
            let node = &p.nodes[n as usize];
            let clock_override = clocks.iter().find(|(k, _)| *k == n).map(|(_, t)| *t);
            let alive = up
                && (clock_override.is_some()
                    || match node.kind {
                        Kind::Plain => in_own_window(node, t, self.t),
                        _ => true,
                    });
            self.alive[n as usize] = alive;
            let ct = clock_override.unwrap_or_else(|| clock_map(&node.clock, t));
            self.child[n as usize] = ct;
            for &c in node.children.iter().rev() {
                stack.push((c, ct, alive));
            }
        }
    }

    /// Whether slot s can be left unevaluated this frame: nothing reads it and its owner is off screen
    /// (outside its window or under a node that is), so no drawn pixel depends on its expression.
    fn idle(&self, s: u32) -> bool {
        let p = self.p;
        if p.slot_read[s as usize] {
            return false;
        }
        let node = match p.slots[s as usize].owner {
            Owner::Node(n) => Some(n),
            Owner::Element(e) => p.elements[e as usize].node,
        };
        node.is_some_and(|n| !self.alive[n as usize])
    }

    /// Whether slot s can be left uncomputed this frame (keys, link and expression): it is [`Self::idle`] and its
    /// node is not one whose properties are read off screen ([`crate::Program::pinned`]). Its value is never read.
    fn unread(&self, s: u32) -> bool {
        let p = self.p;
        self.idle(s)
            && match p.slots[s as usize].owner {
                Owner::Node(n) => !p.pinned[n as usize],
                Owner::Element(e) => p.elements[e as usize].node.is_some_and(|n| !p.pinned[n as usize]),
            }
    }

    fn slots(&mut self, definition_time: Option<f64>) {
        let p = self.p;
        for &s in &p.order {
            let slot = &p.slots[s as usize];
            if self.unread(s) {
                continue;
            }
            #[cfg(test)]
            SLOTS_COMPUTED.with(|c| c.set(c.get() + 1));
            let t = if slot.time_node.is_none() {
                definition_time.unwrap_or_else(|| p.slot_time(s, self.t, Some(&self.tl)))
            } else {
                p.slot_time(s, self.t, Some(&self.tl))
            };
            let mut v = p.channels_at(s, t);
            if let Some(l) = slot.link {
                if let Some(lv) = p.link_value(s, l, self.t, Some(self), 0, &self.memo) {
                    v = lv;
                }
            }
            if let Some(x) = slot.expr.filter(|_| !self.idle(s)) {
                let mut regs = std::mem::take(&mut self.regs);
                let r = {
                    let mut h = ExprHost {
                        p,
                        frame: Some(self),
                        comp_t: self.t,
                        t,
                        slot: Some(s),
                        expr: x,
                        value: v.clone(),
                        depth: 0,
                        memo: &self.memo,
                        sample: [0.0; 2],
                        fold: None,
                    };
                    vm::run(&p.exprs[x as usize].code, &mut h, &mut regs)
                };
                self.regs = regs;
                if let Some(nv) = slot.kind.from_v(&r, &v) {
                    v = nv;
                }
            }
            if let Some(f) = slot.fold {
                v = p.fold_value(f, self.t, 0, &self.memo);
            }
            self.values[s as usize] = slot.kind.clamp(v);
        }
    }

    fn condition(&mut self, n: u32) -> bool {
        let p = self.p;
        let Some(x) = p.nodes[n as usize].cond else { return true };
        let mut regs = std::mem::take(&mut self.regs);
        let r = {
            let mut h = ExprHost {
                p,
                frame: Some(self),
                comp_t: self.t,
                t: self.tl[n as usize],
                slot: None,
                expr: x,
                value: Value::Bool(true),
                depth: 0,
                memo: &self.memo,
                sample: [0.0; 2],
                fold: None,
            };
            vm::run(&p.exprs[x as usize].code, &mut h, &mut regs)
        };
        self.regs = regs;
        r.truthy()
    }

    /// Runs the parametric geometry expression `x` of node `n` (SREP 70) once per sample, with the sampling variables
    /// of each, at this frame. `out` receives one value per sample: the number the expression gives, NaN when it
    /// gives none.
    pub(crate) fn sampled(&mut self, n: u32, x: u32, samples: &[[f64; 2]], out: &mut Vec<f64>) {
        let p = self.p;
        let mut regs = std::mem::take(&mut self.regs);
        out.clear();
        out.reserve(samples.len());
        for s in samples {
            let mut h = ExprHost {
                p,
                frame: Some(self),
                comp_t: self.t,
                t: self.tl[n as usize],
                slot: None,
                expr: x,
                value: Value::Num(0.0),
                depth: 0,
                memo: &self.memo,
                sample: *s,
                fold: None,
            };
            let v = vm::run(&p.exprs[x as usize].code, &mut h, &mut regs);
            out.push(match v {
                V::Num(_) | V::Bool(_) => v.num(),
                _ => f64::NAN,
            });
        }
        self.regs = regs;
    }

    /// The plots draft: computes the frame of every live `axes` from its attributes' values at this frame. An
    /// `axes` whose values the rules refuse (an animated range that closes, say) has no frame: its parts draw
    /// nothing, its children stay in pixels, and the refusal is returned as the frame's problem.
    fn axes_frames(&mut self) -> Vec<String> {
        let p = self.p;
        let mut problems = Vec::new();
        for &n in &p.axes_nodes {
            if !self.alive[n as usize] {
                continue;
            }
            let node = &p.nodes[n as usize];
            let Node::Axes(a) = &*node.elem else { continue };
            let props = self.props(&node.slots);
            let num = |k: &str, d: f64| props.get(k).and_then(Value::as_num).unwrap_or(d);
            let step = |k: &str, d: Option<f64>| props.get(k).and_then(Value::as_num).or(d);
            let flag = |k: &str, d: bool| match props.get(k) {
                Some(Value::Bool(b)) => *b,
                _ => d,
            };
            let spec = crate::axes::Spec {
                kind: match a.kind {
                    sr_model::model::AxesKind::Cartesian => crate::axes::AxesKind::Cartesian,
                    sr_model::model::AxesKind::Complex => crate::axes::AxesKind::Complex,
                    sr_model::model::AxesKind::Polar => crate::axes::AxesKind::Polar,
                    sr_model::model::AxesKind::NumberLine => crate::axes::AxesKind::NumberLine,
                },
                width: num("width", a.width.get()),
                height: num("height", a.height.get()),
                ranges: [num("xMin", a.x_min), num("xMax", a.x_max), num("yMin", a.y_min), num("yMax", a.y_max)],
                steps: [step("xStep", a.x_step.map(|s| s.get())), step("yStep", a.y_step.map(|s| s.get()))],
                ticks: flag("ticks", a.ticks),
                labels: flag("labels", a.labels),
                grid: flag("grid", a.grid),
                tick_length: num("tickLength", a.tick_length.get()),
            };
            match crate::axes::frame(&spec) {
                Ok(af) => self.axes.push((n, Arc::new(af))),
                Err(e) => problems.push(format!("{}: {e}", node.id)),
            }
        }
        problems
    }

    /// The frame of `axes` node `n`, when it is live and its values are valid.
    pub(crate) fn axes_of(&self, n: u32) -> Option<Arc<crate::axes::AxesFrame>> {
        self.axes.iter().find(|(k, _)| *k == n).map(|(_, af)| af.clone())
    }

    /// The path of a synthetic part of `axes` node `a`, drawn by node `n`, at this frame.
    fn axes_part(
        &mut self,
        n: u32,
        a: u32,
        part: crate::program::AxesPart,
        out: &mut FrameNode,
        problems: &mut Vec<String>,
    ) {
        use crate::program::AxesPart;
        let Some(af) = self.axes_of(a) else { return };
        let d = match part {
            AxesPart::Grid => crate::axes::lines(&af.grid),
            AxesPart::Axis => crate::axes::lines(&af.axis),
            AxesPart::Ticks => crate::axes::lines(&af.ticks),
            AxesPart::Labels => return,
            AxesPart::Plot(k) => {
                let p = self.p;
                let clip = matches!(&*p.nodes[a as usize].elem, Node::Axes(x) if x.clip);
                match crate::axes::plot_path(self, n, &p.plots[k as usize], &af, clip) {
                    Ok(d) => d,
                    Err(e) => {
                        problems.push(format!("{}: {e}", out.id));
                        return;
                    }
                }
            }
        };
        match out.props.0.iter_mut().find(|(k, _)| &**k == "path") {
            Some(slot) => slot.1 = Value::Str(d.into()),
            None => out.props.0.push((Arc::from("path"), Value::Str(d.into()))),
        }
    }

    /// The fields draft: the box of field `fi` at this frame, its enclosing axes' or its own; `None` when the axes
    /// has no frame (its problem is reported once, as the axes').
    fn field_box(&self, fi: &crate::field_node::FieldInst) -> Option<Result<crate::field_node::FieldBox, String>> {
        use crate::field_node::FieldBox;
        if let Some(a) = fi.axes {
            let af = self.axes_of(a)?;
            let a = &af.axes;
            return Some(FieldBox::new(a.width, a.height, [a.x_min, a.y_min, a.x_max, a.y_max]));
        }
        let node = &self.p.nodes[fi.node as usize];
        let Node::Field(fl) = &*node.elem else { return None };
        let props = self.props(&node.slots);
        let num = |k: &str, d: f64| props.get(k).and_then(Value::as_num).unwrap_or(d);
        let width = num("width", fl.width.map_or(f64::NAN, |w| w.get()));
        let height = num("height", fl.height.map_or(f64::NAN, |h| h.get()));
        let ranges = [
            num("xMin", fl.x_min.unwrap_or(-1.0)),
            num("yMin", fl.y_min.unwrap_or(-1.0)),
            num("xMax", fl.x_max.unwrap_or(1.0)),
            num("yMax", fl.y_max.unwrap_or(1.0)),
        ];
        Some(FieldBox::new(width, height, ranges))
    }

    /// The fields draft: sets the path or the raster of part `j` of field `k`, drawn by `out`, at this frame.
    fn field_part(&mut self, k: u32, j: u32, out: &mut FrameNode) -> Result<(), String> {
        use crate::field_node::{self as fnode, Kind, Part, Source};
        let p = self.p;
        let fi = &p.fields[k as usize];
        let b = match self.field_box(fi) {
            None => return Ok(()),
            Some(b) => b?,
        };
        let n = fi.node;
        // the force field's values at this frame, read before the frame is lent to the samplers
        let force = match &fi.source {
            Source::ForceField(key) => {
                let props = p.elements.iter().find(|e| *e.key == **key).map(|e| self.props(&e.slots));
                let get = |name: &str| props.as_ref().and_then(|pr| pr.get(name)).and_then(Value::as_num);
                Some(crate::sim::force_field_at(p, key, self.t, &get))
            }
            _ => None,
        };
        let frame = std::cell::RefCell::new(&mut *self);
        let one = |e: u32, x: f64, y: f64| {
            let mut v = Vec::with_capacity(1);
            frame.borrow_mut().sampled(n, e, &[[x, y]], &mut v);
            v[0]
        };
        // the field as a vector function of data (x, y): (u, v), the gradient of a scalar, or (Re w, Im w)
        let vector: Box<dyn Fn(f64, f64) -> [f64; 2] + '_> = match (&fi.source, fi.kind) {
            (Source::Exprs { u, v: Some(v) }, _) => Box::new(|x, y| [one(*u, x, y), one(*v, x, y)]),
            (Source::Exprs { u, v: None }, _) => Box::new(fnode::gradient(move |x, y| one(*u, x, y))),
            (Source::W([re, im]), _) => Box::new(|x, y| [one(*re, x, y), one(*im, x, y)]),
            (Source::Function { f, coefficients }, _) => Box::new(|x, y| fnode::complex_at(*f, coefficients, x, y)),
            (Source::Grid { lattice, ranges }, kind) => {
                let g = crate::fields::grid_sampler(lattice, *ranges).map_err(|e| e.to_string())?;
                if kind == Kind::Scalar {
                    Box::new(fnode::gradient(move |x, y| g(x, y)[0]))
                } else {
                    Box::new(g)
                }
            }
            (Source::ForceField(_), _) => match force.clone().flatten() {
                Some((field, ppm)) => {
                    let t = frame.borrow().t;
                    Box::new(move |x, y| {
                        let a = field.accel([x * ppm, -y * ppm], [0.0, 0.0], t);
                        [a[0] / ppm, -a[1] / ppm]
                    })
                }
                None => Box::new(|_, _| [0.0, 0.0]),
            },
            (Source::Sources { charges, currents, dipoles }, _) => Box::new(move |x, y| {
                let mut f = crate::fields::charge_field(charges, [x, y]);
                for c in currents {
                    let w = crate::fields::wire_field([x, y], c[2], [c[0], c[1]]);
                    f = [f[0] + w[0], f[1] + w[1]];
                }
                for d in dipoles {
                    let w = crate::fields::dipole_field([x, y], [d[2], d[3]], [d[0], d[1]]);
                    f = [f[0] + w[0], f[1] + w[1]];
                }
                f
            }),
        };
        // a scalar field draws only its contours and the lic of its gradient (rule 1)
        let drawn = fi.kind != Kind::Scalar;
        let d = match &fi.parts[j as usize] {
            Part::Arrows { .. } | Part::Streamlines { .. } | Part::Lines { .. } if !drawn => String::new(),
            Part::Arrows { cols, rows, style } => {
                fnode::arrows_path(&vector, &b, *cols, *rows, style).map_err(|e| e.to_string())?
            }
            &Part::Streamlines { separation, test, step, max_steps, max_lines, seed } => {
                fnode::streamlines_path(&vector, &b, separation, test, step, max_steps, max_lines, seed)
                    .map_err(|e| e.to_string())?
            }
            &Part::Lines { per_charge, radius, step, max_steps } => {
                let Source::Sources { charges, .. } = &fi.source else { return Ok(()) };
                fnode::lines_path(charges, &b, per_charge, radius, step, max_steps).map_err(|e| e.to_string())?
            }
            Part::Contours { levels, resolution } => {
                let sample = |at: &[[f64; 2]]| -> Vec<f64> {
                    match &fi.source {
                        Source::Exprs { u, .. } => {
                            let mut v = Vec::new();
                            frame.borrow_mut().sampled(n, *u, at, &mut v);
                            v
                        }
                        Source::Grid { lattice, ranges } => match crate::fields::grid_sampler(lattice, *ranges) {
                            Ok(g) => at.iter().map(|q| g(q[0], q[1])[0]).collect(),
                            Err(_) => vec![f64::NAN; at.len()],
                        },
                        _ => vec![f64::NAN; at.len()],
                    }
                };
                fnode::contours_path(sample, &b, levels, *resolution).map_err(|e| e.to_string())?
            }
            &Part::Lic { length, step, resolution, seed, opacity } => {
                let r = fnode::lic_raster(&vector, &b, length, step, resolution, seed, opacity)
                    .map_err(|e| e.to_string())?;
                out.raster = Some(Arc::new(r));
                out.size = Some([b.width, b.height]);
                return Ok(());
            }
            &Part::Portrait { modulus, base, opacity } => {
                out.raster = Some(Arc::new(fnode::portrait_raster(&vector, &b, modulus, base, opacity)));
                out.size = Some([b.width, b.height]);
                return Ok(());
            }
        };
        match out.props.0.iter_mut().find(|(k, _)| &**k == "path") {
            Some(slot) => slot.1 = Value::Str(d.into()),
            None => out.props.0.push((Arc::from("path"), Value::Str(d.into()))),
        }
        Ok(())
    }

    fn num(&self, n: u32, k: usize) -> f64 {
        let node = &self.p.nodes[n as usize];
        match node.tf_slots.num[k] {
            Some((s, c)) => component(&self.values[s as usize], c).and_then(|v| v.as_num()).unwrap_or(node.tf.num[k]),
            None => node.tf.num[k],
        }
    }

    fn len(&self, n: u32, k: usize) -> Length {
        let node = &self.p.nodes[n as usize];
        match node.tf_slots.pos[k] {
            Some((s, c)) => match component(&self.values[s as usize], c) {
                Some(Value::Len(l)) => l,
                Some(Value::Num(x)) => Length::px(x),
                _ => node.tf.pos[k],
            },
            None => node.tf.pos[k],
        }
    }

    fn local(&self, n: u32, bx: [f64; 2]) -> Local {
        let p = self.p;
        let node = &p.nodes[n as usize];
        let fs = p.size;
        let mut x = resolve_len(self.len(n, tfi::X), bx[0], fs);
        let mut y = resolve_len(self.len(n, tfi::Y), bx[1], fs);
        let mut ax = resolve_len(self.len(n, tfi::AX), bx[0], fs);
        let mut ay = resolve_len(self.len(n, tfi::AY), bx[1], fs);
        let mut rot = self.num(n, tfi::ROT);
        let (sx, sy) = (self.num(n, tfi::SX), self.num(n, tfi::SY));
        let (kx, ky) = (self.num(n, tfi::KX), self.num(n, tfi::KY));
        // XSD: from 0 to 1. An overshooting easing, a spring or an expression can leave the range
        let mut opacity = unit_opacity(self.num(n, tfi::OPACITY));
        if let Some(m) = &node.motion {
            let t = self.tl[n as usize];
            let end = m.end.or(node.end).unwrap_or(p.duration);
            let prog = match m.progress {
                Some(s) => self.values[s as usize].as_num().unwrap_or(0.0),
                None => {
                    let d = end - m.start;
                    let u = if d > 0.0 { ((t - m.start) / d).clamp(0.0, 1.0) } else { 1.0 };
                    m.ease.apply(u)
                }
            };
            let (pt, ang) = m.path.sample(prog, m.constant_speed);
            if m.additive {
                // an offset: how far the path has gone from its first point
                let start = m.path.sample(0.0, m.constant_speed).0;
                x += pt[0] - start[0];
                y += pt[1] - start[1];
            } else {
                x = pt[0];
                y = pt[1];
            }
            if m.auto_orient {
                rot += ang + m.orient_offset;
            }
        }
        let data = [x, y];
        if let Some(af) = node.data_axes.and_then(|a| self.axes_of(a)) {
            // rule 9: the child's position and anchor are in data units; its other attributes stay in pixels
            [x, y] = af.place(x, y);
            [ax, ay] = af.offset(ax, ay);
        }
        let mut aff = Affine::translate(x, y)
            .then(&Affine::rotate(rot))
            .then(&Affine::skew(kx, ky))
            .then(&Affine::scale(sx, sy))
            .then(&Affine::translate(-ax, -ay));
        if let Some([dx, dy, r, s, o]) = node.copy {
            let mut steps = Affine::translate(dx, dy).then(&Affine::rotate(r)).then(&Affine::scale(s, s));
            // SREP 26: T(x, y) · R(θ) of the copy's point, before its step offsets
            if let Some((g, i)) = node.point {
                let gen = &p.points[g as usize];
                if node.parent == Some(gen.node) {
                    let live = std::array::from_fn(|k| match gen.slots[k] {
                        Some(s) => self.values[s as usize].as_num().unwrap_or(gen.base[k]),
                        None => gen.base[k],
                    });
                    let q = gen.point(i, live);
                    steps = Affine::translate(q.x, q.y).then(&Affine::rotate(q.theta)).then(&steps);
                }
            }
            aff = steps.then(&aff);
            opacity *= unit_opacity(o);
        }
        if let Some(adj) = self.adjust[n as usize] {
            aff = adj.then(&aff);
        }
        // the properties of a data-unit child read its data values; its drawn anchor is in pixels
        let pos = if node.data_axes.is_some() { data } else { [x, y] };
        Local { aff, opacity, pos, anchor: [ax, ay], rot }
    }

    /// Box size and media placement of a node in its local space.
    fn size(&self, n: u32, bx: [f64; 2]) -> (Option<[f64; 2]>, Option<crate::layout::Content>) {
        let p = self.p;
        let node = &p.nodes[n as usize];
        let fs = p.size;
        if let Some(fit) = &node.fit {
            let bsize = match (fit.fit, fit.box_w, fit.box_h) {
                (sr_model::model::Fit::None, _, _) => None,
                (_, Some(w), Some(h)) => Some([resolve_len(w, bx[0], fs), resolve_len(h, bx[1], fs)]),
                _ => None,
            };
            return match node.asset_size {
                Some(src) => {
                    let (size, c) = crate::layout::place(src, fit, bsize);
                    (Some(size), Some(c))
                }
                None => (bsize, None),
            };
        }
        if let Some([w, h]) = node.shape_size {
            let (w, h) = (self.live_len(n, "width", w), self.live_len(n, "height", h));
            return (Some([resolve_len(w, bx[0], fs), resolve_len(h, bx[1], fs)]), None);
        }
        if let Some([w, h]) = node.box_size {
            let (w, h) = (self.live_len(n, "width", w), self.live_len(n, "height", h));
            return (Some([resolve_len(w, bx[0], fs), resolve_len(h, bx[1], fs)]), None);
        }
        (None, None)
    }

    /// The current value of an animated length property of node `n` (width, height), else `base`.
    fn live_len(&self, n: u32, prop: &str, base: Length) -> Length {
        let p = self.p;
        p.nodes[n as usize]
            .slots
            .iter()
            .find(|&&s| &*p.slots[s as usize].prop == prop)
            .and_then(|&s| match &self.values[s as usize] {
                Value::Len(l) => Some(*l),
                Value::Num(x) => Some(Length::px(*x)),
                _ => None,
            })
            .unwrap_or(base)
    }

    fn active(&mut self, n: u32) -> bool {
        let node = &self.p.nodes[n as usize];
        let tl = self.tl[n as usize];
        let on = match node.kind {
            Kind::RepeatCopy { .. } => true,
            Kind::Transition(_) => false,
            Kind::Plain => in_own_window(node, tl, self.t),
        };
        on && self.condition(n)
    }

    /// Flex/grid layout of a container's children and alignment of each child.
    fn arrange(
        &mut self,
        kids: &[u32],
        layout: Option<(&crate::layout::LayoutSpec, Option<[f64; 2]>)>,
        cbox: [f64; 2],
        cworld: Affine,
    ) {
        let p = self.p;
        let fs = p.size;
        // a connector has no box and takes no slot (SREP 16 §1.2)
        let active: Vec<u32> =
            kids.iter().copied().filter(|&k| p.nodes[k as usize].name != "connector" && self.active(k)).collect();
        if let Some((spec, own)) = layout {
            let mut placed = Vec::new();
            let mut sizes = Vec::new();
            for &k in &active {
                self.adjust[k as usize] = None;
                let (size, _) = self.size(k, cbox);
                let Some([w, h]) = size else { continue };
                let l = self.local(k, cbox);
                let bb = l.aff.bbox([0.0, 0.0, w, h]);
                placed.push((k, bb, l.pos));
                sizes.push([bb[2] - bb[0], bb[3] - bb[1]]);
            }
            let slots = crate::layout::arrange(
                spec,
                own,
                resolve_len(spec.gap, cbox[0], fs),
                resolve_len(spec.padding, cbox[0], fs),
                &sizes,
            );
            for ((k, bb, pos), slot) in placed.into_iter().zip(slots) {
                let target = [slot.at[0] + pos[0], slot.at[1] + pos[1]];
                let adj = Affine::translate(target[0], target[1])
                    .then(&Affine::scale(slot.stretch[0], slot.stretch[1]))
                    .then(&Affine::translate(-bb[0], -bb[1]));
                self.adjust[k as usize] = Some(adj);
            }
        }
        for &k in &active {
            let Some(spec) = p.nodes[k as usize].align else { continue };
            let (size, _) = self.size(k, cbox);
            let Some([w, h]) = size else { continue };
            let l = self.local(k, cbox);
            let bb = l.aff.bbox([0.0, 0.0, w, h]);
            let frame_rect = |inset: [f64; 4]| {
                [inset[3] * fs[0], inset[0] * fs[1], fs[0] * (1.0 - inset[1]), fs[1] * (1.0 - inset[2])]
            };
            let rect = match spec.to {
                sr_model::model::AlignTo::Parent => [0.0, 0.0, cbox[0], cbox[1]],
                sr_model::model::AlignTo::Frame => {
                    cworld.inverse().map(|i| i.bbox(frame_rect([0.0; 4]))).unwrap_or([0.0, 0.0, fs[0], fs[1]])
                }
                sr_model::model::AlignTo::SafeArea => {
                    cworld.inverse().map(|i| i.bbox(frame_rect(p.safe_area))).unwrap_or(frame_rect(p.safe_area))
                }
            };
            let slot = crate::layout::align(&spec, bb, rect, resolve_len(spec.margin, cbox[0], fs));
            let adj = Affine::translate(slot.at[0], slot.at[1])
                .then(&Affine::scale(slot.stretch[0], slot.stretch[1]))
                .then(&Affine::translate(-bb[0], -bb[1]));
            let prev = self.adjust[k as usize].unwrap_or(Affine::IDENTITY);
            self.adjust[k as usize] = Some(adj.then(&prev));
        }
    }

    /// Siblings in this frame's paint order: document order stably sorted by the current `z`
    /// (an integer property, so its animated value is already rounded).
    fn stacked(&self, doc: &[u32]) -> Cow<'p, [u32]> {
        let mut v = doc.to_vec();
        v.sort_by_key(|&k| self.z_of(k));
        Cow::Owned(v)
    }

    /// The current stacking `z` of node `k`: its animated value (an integer property, so already rounded), else its
    /// written one.
    fn z_of(&self, k: u32) -> i32 {
        let n = &self.p.nodes[k as usize];
        n.z_slot.and_then(|s| self.values[s as usize].as_num()).filter(|z| z.is_finite()).map_or(n.z, |z| z as i32)
    }

    fn props(&self, slots: &[u32]) -> Props {
        Props(slots.iter().map(|&s| (self.p.slots[s as usize].prop.clone(), self.values[s as usize].clone())).collect())
    }
}

/// Evaluates one frame at composition time `t` (seconds).
pub fn evaluate(p: &Program, t: f64) -> FrameGraph {
    evaluate_with_clocks(p, t, &[])
}

/// Samples animated definitions at a consumer's local time, including definitions
/// from included documents. Used when a sprite samples a generator at layer time.
/// Node properties referenced by expressions retain their ordinary timeline mapping.
pub fn elements_at(p: &Program, t: f64) -> Vec<ElementState> {
    let n = p.nodes.len();
    let mut f = Frame {
        include_inactive: false,
        p,
        t,
        tl: vec![0.0; n],
        child: vec![0.0; n],
        values: vec![Value::Num(0.0); p.slots.len()],
        regs: Vec::new(),
        adjust: vec![None; n],
        alive: vec![false; n],
        memo: Memo::default(),
        axes: Vec::new(),
    };
    f.timelines(&[]);
    f.slots(Some(t));
    p.elements
        .iter()
        .filter(|e| e.node.is_none())
        .map(|e| ElementState { key: e.key.clone(), element: e.name, props: f.props(&e.slots) })
        .collect()
}

/// Sample source history that a remap jumps over or a freeze holds. Composition
/// properties still use `t`; the supplied container clocks drive their children.
pub(crate) fn evaluate_with_clocks(p: &Program, t: f64, clocks: &[(u32, f64)]) -> FrameGraph {
    evaluate_inner(p, t, clocks, false)
}

/// Discover physics bodies even when their render windows have not started yet.
pub(crate) fn evaluate_for_physics(p: &Program, t: f64) -> FrameGraph {
    evaluate_inner(p, t, &[], true)
}

/// Pose sampling across a window edge, preserving an overridden source clock.
/// Callers decide which bodies are active from the ordinary frame at step start.
pub(crate) fn evaluate_pose_with_clocks(p: &Program, t: f64, clocks: &[(u32, f64)]) -> FrameGraph {
    evaluate_inner(p, t, clocks, true)
}

fn evaluate_inner(p: &Program, t: f64, clocks: &[(u32, f64)], include_inactive: bool) -> FrameGraph {
    let n = p.nodes.len();
    let mut f = Frame {
        include_inactive,
        p,
        t,
        tl: vec![0.0; n],
        child: vec![0.0; n],
        values: vec![Value::Num(0.0); p.slots.len()],
        regs: Vec::new(),
        adjust: vec![None; n],
        alive: vec![false; n],
        memo: Memo::default(),
        axes: Vec::new(),
    };
    f.timelines(clocks);
    if include_inactive {
        f.alive.fill(true);
    }
    f.slots(None);
    let axes_problems = f.axes_frames();

    struct Out {
        inst: Vec<u32>,
        nodes: Vec<FrameNode>,
        world: Vec<Option<(Affine, f64)>>,
        out_ix: Vec<Option<u32>>,
        problems: Vec<String>,
    }
    // every drawn node is alive, so the alive count bounds the output without reserving for the whole document
    let alive = f.alive.iter().filter(|&&a| a).count();
    let mut out = Out {
        inst: Vec::with_capacity(alive),
        nodes: Vec::with_capacity(alive),
        world: vec![None; n],
        out_ix: vec![None; n],
        problems: axes_problems,
    };

    // depth-first, z-sorted, active nodes only
    #[allow(clippy::too_many_arguments)]
    fn visit(
        f: &mut Frame,
        o: &mut Out,
        n: u32,
        parent_out: Option<u32>,
        depth: u32,
        bx: [f64; 2],
        pw: (Affine, f64),
        draw_parent: bool,
    ) {
        let p = f.p;
        let node = &p.nodes[n as usize];
        let tl = f.tl[n as usize];
        let active = match node.kind {
            Kind::RepeatCopy { .. } => true,
            Kind::Transition(_) => false,
            Kind::Plain => f.alive[n as usize],
        };
        if !active || (!f.include_inactive && !f.condition(n)) {
            return;
        }
        let lc = f.local(n, bx);
        let (local, opacity) = (lc.aff, lc.opacity);
        let (size, content) = f.size(n, bx);
        let base = match node.parent_link {
            Some(pl) => world_of(f, o, pl).map(|w| w.0).unwrap_or(pw.0),
            None => pw.0,
        };
        let world = base.then(&local);
        let wop = pw.1 * opacity;
        o.world[n as usize] = Some((world, wop));
        let ix = o.nodes.len() as u32;
        o.out_ix[n as usize] = Some(ix);
        o.inst.push(n);
        let draw = draw_parent && node.visible;
        let three = node.three_d.then(|| [f.num(n, tfi::ZDEPTH), f.num(n, tfi::RX), f.num(n, tfi::RY)]);
        let parts = node
            .parts
            .iter()
            .map(|&e| ElementState {
                key: p.elements[e as usize].key.clone(),
                element: p.elements[e as usize].name,
                props: f.props(&p.elements[e as usize].slots),
            })
            .collect();
        o.nodes.push(FrameNode {
            id: node.id.clone(),
            seed_id: node.seed_id.clone(),
            scope: node.scope.clone(),
            doc: node.doc,
            kind: node.name,
            parent: parent_out,
            transform_parent: parent_out,
            depth,
            z: f.z_of(n),
            local,
            world,
            opacity,
            world_opacity: wop,
            timeline_time: tl,
            local_time: tl - node.start,
            source_time: node.media.as_ref().map(|m| m.map(tl)).or(match &node.clock {
                Clock::Media(m) => Some(m.map(tl)),
                _ => None,
            }),
            asset: node.asset.clone(),
            text: node.text.clone(),
            draw,
            matte: None,
            is_matte: false,
            repeat: node.repeat.as_ref().map(|r| [r.0, r.1]),
            three_d: three,
            size,
            anchor: lc.anchor,
            content,
            clip: node.clip,
            axes: match (node.name, node.axes_part) {
                ("axes", _) => f.axes_of(n),
                (_, Some((a, crate::program::AxesPart::Labels))) => f.axes_of(a),
                _ => None,
            },
            props: {
                let mut props = f.props(&node.slots);
                if node.motion.is_some() {
                    // A motion path places the node through its local transform. Its x and y (and its rotation, when
                    // the path orients it) are also the node's properties, so that what reads a pose from them (a
                    // camera's eye, an object3D, a rigid body's start) follows the path instead of the attributes the
                    // path replaced.
                    let turn = node.motion.as_ref().is_some_and(|m| m.auto_orient).then_some(("rotation", lc.rot));
                    for (k, v) in [("x", lc.pos[0]), ("y", lc.pos[1])].into_iter().chain(turn) {
                        match props.0.iter_mut().find(|(name, _)| &**name == k) {
                            Some(slot) => slot.1 = Value::Num(v),
                            None => props.0.push((Arc::from(k), Value::Num(v))),
                        }
                    }
                }
                props
            },
            parts,
            bones: Vec::new(),
            skin: None,
            soft: None,
            particles: None,
            particles3d: None,
            sim_ocean: None,
            fracture: None,
            sim_image: None,
            sim_volume: None,
            crater_impact: None,
            voxels: None,
            pose3: None,
            joints: None,
            connector: None,
            param_mesh: None,
            param_parts: Vec::new(),
            deform_maps: node
                .deform_maps
                .iter()
                .map(|(k, x)| {
                    let spec = MapSpec { key: k.clone(), exprs: *x, comp_t: f.t, t: tl };
                    // one point with the frame's memo, so a fold the map reads at another time reports its failure
                    spec.point(p, 0.0, 0.0, &f.memo);
                    (k.clone(), spec)
                })
                .collect(),
            clay_mesh: None,
            raster: None,
            param_box: None,
            elem: node.elem.clone(),
        });
        if let Some(g) = node.geom {
            if let Some(out) = o.nodes.last_mut() {
                crate::parametric::attach(f, n, &p.geoms[g as usize], out);
            }
        }
        if let Some((a, part)) = node.axes_part {
            if let Some(out) = o.nodes.last_mut() {
                f.axes_part(n, a, part, out, &mut o.problems);
            }
        }
        if let Some((k, j)) = node.field_part {
            if let Some(out) = o.nodes.last_mut() {
                if let Err(e) = f.field_part(k, j, out) {
                    o.problems.push(format!("{}: {e}", out.id));
                }
            }
        }
        if let Some(k) = node.axes3d {
            if let Some(out) = o.nodes.last_mut() {
                match crate::axes::axes3d_mesh(f, n, &p.axes3d[k as usize], &mut o.problems, &node.id) {
                    Ok(m) => {
                        out.param_mesh = Some(Arc::new(m.own));
                        out.param_parts = m.parts.into_iter().map(|(id, mesh)| (id, Arc::new(mesh))).collect();
                    }
                    Err(e) => o.problems.push(format!("{}: {e}", node.id)),
                }
            }
        }
        if node.name == "shape" {
            if let Some(shape) = o.nodes.last_mut() {
                crate::stroke_font::attach(p, shape, &mut o.problems);
            }
        }
        let own_box = node.box_size.map(|[w, h]| [resolve_len(w, bx[0], p.size), resolve_len(h, bx[1], p.size)]);
        let cbox = own_box.unwrap_or(bx);
        // A fitted instance places the symbol's coordinate system inside its box.
        let kid_world = child_world(f, n, bx, world);
        if node.layout.is_some() || node.children.iter().any(|&k| p.nodes[k as usize].align.is_some()) {
            f.arrange(&node.doc_children, node.layout.as_ref().map(|l| (l, own_box)), cbox, kid_world);
        }
        let kids = if node.restack { f.stacked(&node.doc_children) } else { Cow::Borrowed(&node.children[..]) };
        for &k in kids.iter() {
            visit(f, o, k, Some(ix), depth + 1, cbox, (kid_world, wop), draw);
        }
    }

    fn child_world(f: &Frame, n: u32, bx: [f64; 2], world: Affine) -> Affine {
        let node = &f.p.nodes[n as usize];
        match (node.name, node.asset_size) {
            ("instance", Some(src)) => match f.size(n, bx).1 {
                Some(c) => world.then(&Affine(crate::layout::symbol_transform(src, &c))),
                None => world,
            },
            _ => world,
        }
    }

    // Percentages within symbols use their natural box, including a transform parent visited lazily.
    fn child_box(f: &Frame, n: Option<u32>) -> [f64; 2] {
        let Some(n) = n else { return f.p.size };
        let node = &f.p.nodes[n as usize];
        let bx = child_box(f, node.parent);
        node.box_size.map(|[w, h]| [resolve_len(w, bx[0], f.p.size), resolve_len(h, bx[1], f.p.size)]).unwrap_or(bx)
    }

    fn world_of(f: &mut Frame, o: &mut Out, n: u32) -> Option<(Affine, f64)> {
        if let Some(w) = o.world[n as usize] {
            return Some(w);
        }
        // parent referenced before it was visited: compute its chain directly
        let p = f.p;
        let node = &p.nodes[n as usize];
        let (pw, bx) = match node.parent.filter(|_| node.parent_link.is_none()) {
            Some(c) => {
                let pw = world_of(f, o, c)?;
                let bx = child_box(f, p.nodes[c as usize].parent);
                ((child_world(f, c, bx, pw.0), pw.1), child_box(f, Some(c)))
            }
            None => ((Affine::IDENTITY, 1.0), child_box(f, node.parent)),
        };
        let lc = f.local(n, bx);
        let base = match node.parent_link {
            Some(pl) if pl != n => world_of(f, o, pl).map(|w| w.0).unwrap_or(pw.0),
            _ => pw.0,
        };
        let mut opacity = lc.opacity;
        if node.parent_link.is_some() {
            // Opacity follows containment, independently of transform parenting.
            let mut parent = node.parent;
            while let Some(k) = parent {
                opacity *= f.local(k, p.size).opacity;
                parent = p.nodes[k as usize].parent;
            }
        } else {
            opacity *= pw.1;
        }
        Some((base.then(&lc.aff), opacity))
    }

    if p.roots.iter().any(|&k| p.nodes[k as usize].align.is_some()) {
        let doc_roots = p.roots.clone();
        f.arrange(&doc_roots, None, p.size, Affine::IDENTITY);
    }
    let roots = if p.restack_roots { f.stacked(&p.doc_roots) } else { Cow::Borrowed(&p.roots[..]) };
    for &r in roots.iter() {
        visit(&mut f, &mut out, r, None, 0, p.size, (Affine::IDENTITY, 1.0), true);
    }

    // mattes: map instance indices to output indices; matte sources draw only when asked
    let mut hidden = vec![false; out.nodes.len()];
    for i in 0..out.nodes.len() {
        let node = &p.nodes[out.inst[i] as usize];
        out.nodes[i].transform_parent = node.parent_link.and_then(|k| out.out_ix[k as usize]).or(out.nodes[i].parent);
        if let Some(m) = node.matte.and_then(|m| out.out_ix[m as usize]) {
            out.nodes[i].matte = Some(m);
            out.nodes[m as usize].is_matte = true;
            if !node.matte_visible {
                hidden[m as usize] = true;
            }
        }
    }
    // a luma transition's matte sibling is not drawn itself
    for tr in &p.transitions {
        if let Some(m) = tr.matte.and_then(|m| out.out_ix[m as usize]) {
            hidden[m as usize] = true;
        }
    }
    for (i, h) in hidden.into_iter().enumerate() {
        if h {
            out.nodes[i].draw = false;
        }
    }

    crate::rig::post_pass(p, &mut out.nodes, &out.inst, t);
    // shapes on pdf regions follow their layer, and connectors read their ends as drawn, once every pose is final
    crate::region::resolve(p, &mut out.nodes);
    crate::connector::resolve(p, &mut out.nodes);

    // the cameras inside a viewport3D view only its scene (SREP 74 §2), and never become the document's camera
    let camera = out
        .nodes
        .iter()
        .enumerate()
        .rev()
        .find(|(i, n)| n.kind == "camera" && n.draw && viewport_of(&out.nodes, *i).is_none())
        .map(|(i, _)| i as u32);

    let mut transitions = Vec::new();
    for tr in &p.transitions {
        let tt = match tr.container {
            Some(c) => f.child[c as usize],
            None => t,
        };
        let (w0, w1) = tr.window;
        if tt < w0 || tt >= w1 {
            continue;
        }
        let u = if w1 > w0 { (tt - w0) / (w1 - w0) } else { 1.0 };
        // central difference of the eased curve (the `velocity` variable)
        let velocity = if w1 > w0 {
            let (lo, hi) = ((u - 1e-3).max(0.0), (u + 1e-3).min(1.0));
            if hi > lo {
                (tr.ease.apply(hi) - tr.ease.apply(lo)) / (hi - lo) / (w1 - w0)
            } else {
                0.0
            }
        } else {
            0.0
        };
        let props = match tr.node {
            Some(n) => f.props(&p.nodes[n as usize].slots),
            None => Props::default(),
        };
        transitions.push(FrameTransition {
            kind: tr.kind.clone(),
            from: tr.from.and_then(|x| out.out_ix[x as usize]),
            to: tr.to.and_then(|x| out.out_ix[x as usize]),
            progress: tr.ease.apply(u).clamp(0.0, 1.0),
            velocity,
            props,
            elem: tr.node.map(|n| p.nodes[n as usize].elem.clone()),
        });
    }

    let elements = p
        .elements
        .iter()
        .filter(|e| e.node.is_none() && !e.slots.is_empty())
        .map(|e| ElementState { key: e.key.clone(), element: e.name, props: f.props(&e.slots) })
        .collect();

    let background = sr_model::element::Element::get_attr(&p.scene.project, "background")
        .map(|a| crate::value::PropKind::Paint.from_attr(&a, &|_| None))
        .unwrap_or(Value::Color([0.0, 0.0, 0.0, 1.0]));
    let background = match (&p.scene.project.background, &background) {
        (sr_model::values::Paint::Color(sr_model::values::Color::Token(tk)), _) => {
            token_color(p, tk).map(Value::Color).unwrap_or(background)
        }
        _ => background,
    };

    let mut graph = FrameGraph {
        time: t,
        frame: libm::floor(t * p.fps.as_f64() + 1e-9) as i64,
        size: p.size,
        background,
        nodes: out.nodes,
        transitions,
        camera,
        elements,
        problems: std::mem::take(&mut out.problems),
        failures: Vec::new(),
        sim_seconds: SimSeconds::default(),
        program: SharedProgram::default(),
        seed: p.seed,
    };
    // the reactive-values draft: a fold whose update produced a non-finite state is a render error
    for m in f.memo.failures.take() {
        graph.fail(m);
    }
    crate::joints::attach(p, &mut graph);
    graph
}

/// The `viewport3D` node that node `i` is inside, if any: the 3D scene `i` belongs to (SREP 74).
pub fn viewport_of(nodes: &[FrameNode], i: usize) -> Option<usize> {
    let mut at = nodes[i].parent;
    while let Some(p) = at {
        if nodes[p as usize].kind == "viewport3D" {
            return Some(p as usize);
        }
        at = nodes[p as usize].parent;
    }
    None
}

/// The camera that views the scene of viewport `vp` at this frame (SREP 74 §2): the camera child its `camera`
/// attribute names, else its last active camera child, else none (the viewport's implicit camera).
pub fn viewport_camera(g: &FrameGraph, vp: usize) -> Option<usize> {
    let mine = |i: usize| g.nodes[i].kind == "camera" && g.nodes[i].parent == Some(vp as u32);
    if let Some(sr_model::element::AttrValue::Str(id)) =
        sr_model::element::Element::get_attr(&*g.nodes[vp].elem, "camera")
    {
        // ids in the frame graph carry the scope of instances and includes; the named camera is a child
        let named = (0..g.nodes.len()).find(|&i| mine(i) && local_id(&g.nodes[i].id) == id.as_str());
        if named.is_some() {
            return named;
        }
    }
    (0..g.nodes.len()).rev().find(|&i| mine(i) && g.nodes[i].draw)
}

/// The node that the id `id`, written in scope `scope` (a node's [`FrameNode::scope`]), names at this frame: `id`
/// in that scope, else in each enclosing scope out to the top level of the document it is written in, as the
/// evaluator resolves ids. Ids of an included document, a symbol instance or a repeat copy carry its scope
/// (`inc/hole`) while the references written inside it do not (`blackHole="hole"`). A lookup inside an include stops
/// at the include: the included document's references never name the including document's nodes. An id the compiler
/// has already resolved (an effective id such as `inc/carrier`, which an id as written never is: an IDREF holds no
/// `/`) names that node and no other, so it is matched as it is and never read relative to `scope`, where it could
/// name a node of a nested include of the same id (`inc/inc/carrier`).
pub fn resolve_in(nodes: &[FrameNode], scope: &str, id: &str) -> Option<usize> {
    if id.contains('/') {
        return nodes.iter().position(|n| *n.id == *id);
    }
    let mut s = scope;
    loop {
        let key = if s.is_empty() { id.to_string() } else { format!("{s}/{id}") };
        let found = nodes.iter().position(|n| *n.id == *key);
        if found.is_some() || s.is_empty() {
            return found;
        }
        if is_include(nodes, s) {
            break;
        }
        s = s.rfind('/').map(|i| &s[..i]).unwrap_or("");
    }
    None
}

/// Whether a reference written in `scope` is inside an included document, whose references never name the including
/// document's nodes, lights or other definitions.
pub fn inside_include(nodes: &[FrameNode], scope: &str) -> bool {
    let mut s = scope;
    while !s.is_empty() {
        if is_include(nodes, s) {
            return true;
        }
        s = s.rfind('/').map(|i| &s[..i]).unwrap_or("");
    }
    false
}

fn is_include(nodes: &[FrameNode], id: &str) -> bool {
    nodes.iter().any(|n| *n.id == *id && n.kind == "include")
}

/// An id without the scope prefix of the instance or include it is in.
fn local_id(id: &str) -> &str {
    id.rsplit('/').next().unwrap_or(id)
}

fn token_color(p: &Program, name: &str) -> Option<[f64; 4]> {
    let styles = p.scene.styles.as_ref()?;
    let mut cur = name.to_string();
    for _ in 0..8 {
        let v = styles.children.iter().find_map(|c| match c {
            sr_model::model::StylesChild::Token(t) if t.name == cur => Some(t.value.clone()),
            _ => None,
        })?;
        match <sr_model::values::Color as sr_model::parse::ParseValue>::parse_value(v.trim()).ok()? {
            sr_model::values::Color::Rgba(c) => return Some([c.r as f64, c.g as f64, c.b as f64, c.a as f64]),
            sr_model::values::Color::Token(t) => cur = t,
        }
    }
    None
}

/// Marker time lookup exposed for callers (includes `beat.N`/`bar.N`).
pub fn marker(p: &Program, id: &str) -> Option<f64> {
    marker_time(&p.markers, p.beat, id)
}

#[cfg(test)]
thread_local! {
    /// Slots [`Frame::slots`] computed.
    static SLOTS_COMPUTED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests {
    use super::SLOTS_COMPUTED;

    fn frame(body: &str, t: f64) -> (crate::FrameGraph, usize) {
        let xml = format!(
            r##"<scene version="1.1"><project width="200" height="200" fps="10" duration="6"/><assets><image id="img" src="img.png" width="8" height="8"/></assets><composition>{body}</composition></scene>"##
        );
        let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).expect("valid");
        let ev = crate::Evaluator::new(&doc, &crate::EvalOptions::default()).unwrap_or_else(|r| panic!("{r}"));
        SLOTS_COMPUTED.with(|c| c.set(0));
        let g = ev.evaluate_layout(t);
        (g, SLOTS_COMPUTED.with(|c| c.get()))
    }

    fn held(prop: &str, v: f64) -> String {
        format!(r#"<animate property="{prop}"><key time="0" value="{v}"/><key time="10" value="{v}"/></animate>"#)
    }

    #[test]
    fn keyframes_of_an_off_screen_node_nobody_reads_are_not_evaluated() {
        let on = format!(r#"<shape id="on" shape="rect" width="4" height="4">{}</shape>"#, held("x", 30.0));
        let (_, alone) = frame(&on, 0.5);
        let later = format!(
            r#"<shape id="later" shape="rect" width="4" height="4" start="2">{}{}</shape>"#,
            held("x", 1.0),
            held("y", 2.0)
        );
        let (g, with_later) = frame(&format!("{on}{later}"), 0.5);
        assert!(g.nodes.iter().all(|n| &*n.id != "later"), "not on screen yet");
        assert_eq!(with_later, alone, "the later shape's keyframes wait for its window");
        let (g, shown) = frame(&format!("{on}{later}"), 2.5);
        let n = g.nodes.iter().find(|n| &*n.id == "later").expect("on screen");
        assert_eq!((n.world.0[4], n.world.0[5]), (1.0, 2.0));
        assert!(shown > alone);
    }

    #[test]
    fn a_node_parented_to_an_off_screen_node_reads_its_keyframes() {
        let body = format!(
            r#"<shape id="later" shape="rect" width="4" height="4" start="2">{}</shape><shape id="kid" shape="rect" width="4" height="4" parent="later" x="10"/>"#,
            held("x", 100.0)
        );
        let (g, _) = frame(&body, 0.5);
        let kid = g.nodes.iter().find(|n| &*n.id == "kid").expect("kid");
        assert_eq!(kid.world.0[4], 110.0, "the parent's animated x, though the parent is not drawn yet");
    }

    #[test]
    fn a_transition_reads_its_own_keyframes() {
        let body = format!(
            r#"<layer id="a" asset="img" end="2"/><layer id="b" asset="img" start="2"/><transition type="wipe" from="a" to="b" duration="1" curve="linear">{}</transition>"#,
            held("softness", 0.6)
        );
        let (g, _) = frame(&body, 2.0);
        assert_eq!(g.transitions.len(), 1);
        assert_eq!(g.transitions[0].props.get("softness").and_then(|v| v.as_num()), Some(0.6));
    }

    #[test]
    fn a_transition_outliving_its_group_reads_its_own_keyframes() {
        let body = format!(
            r#"<group id="g" end="1"><layer id="a" asset="img" end="2"/><layer id="b" asset="img" start="2"/><transition type="wipe" from="a" to="b" duration="1" curve="linear">{}</transition></group>"#,
            held("softness", 0.6)
        );
        let (g, _) = frame(&body, 2.0);
        assert_eq!(g.transitions.len(), 1);
        assert_eq!(g.transitions[0].props.get("softness").and_then(|v| v.as_num()), Some(0.6));
    }
}
