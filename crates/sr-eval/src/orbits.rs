//! The orbits draft at the evaluator (sr-core `srep/srep-0000-orbits.md`, rules 2 to 4 and 6): the `orbit` and
//! `nbody` of a document's `space`, read once per program, and the `object3D` nodes they place at each frame. An
//! orbit's object stands at its central object's place at the frame plus the orbit's state at the frame's time of the
//! space clock (scene seconds × rate); an nbody member stands at the integrated system's state, and a `cluster`'s
//! bodies are spheres named by its prefix, added to the composition when the document is templated. Positions are
//! metres in the space frame until they map to the scene as (x, −z, y) × pixelsPerMeter about the space's origin.

use std::collections::HashMap;
use std::sync::Mutex;

use sr_model::element::{walk, Element};
use sr_model::model as m;
use sr_sim::nbody::{integrate, integrate_adaptive, plummer, system, Integrator, System, MAX_STEPS};
use sr_space::orbits::{body_gm, orbit, orbit_state, Orbit, Vec3};
use sr_space::{Positions, Space};

use crate::eval::FrameNode;
use crate::program::Program;

/// The seconds either side of the epoch over which a member's velocity is differenced from the approximate
/// elements, which give positions only.
const VELOCITY_SPAN: f64 = 60.0;

/// What moves the objects of one document: its orbits by object id and its N-body system.
#[derive(Debug, Default)]
pub(crate) struct Motion {
    rate: f64,
    pixels_per_meter: f64,
    /// Each orbiting object's central object and orbit, by object id.
    orbits: HashMap<String, (String, Orbit)>,
    nbody: Option<Bodies>,
}

#[derive(Debug)]
struct Bodies {
    start: System,
    integrator: Integrator,
    step: f64,
    tolerance: Option<f64>,
    /// The object each body places, by body index.
    ids: Vec<String>,
    /// The last state reached, from which a later frame steps on: the fixed-step leapfrog and Wisdom–Holman schemes
    /// carry nothing else between steps, so the step sequence and its result are those from the epoch.
    last: Mutex<Option<System>>,
}

/// Rule 6: metres in the space frame to scene units.
fn scene(p: Vec3, pixels_per_meter: f64) -> [f64; 3] {
    [p[0] * pixels_per_meter, -p[2] * pixels_per_meter, p[1] * pixels_per_meter]
}

fn objects(scene: &m::Scene) -> HashMap<&str, &m::Object3D> {
    let mut out = HashMap::new();
    walk(&scene.composition, &mut |e| {
        if let Some(o) = e.as_any().downcast_ref::<m::Object3D>() {
            out.insert(o.id.as_str(), o);
        }
    });
    out
}

fn members(n: &m::Nbody) -> impl Iterator<Item = &m::NbodyMember> {
    n.children.iter().filter_map(|c| match c {
        m::NbodyChild::Member(x) => Some(x),
        m::NbodyChild::Cluster(_) => None,
    })
}

fn clusters(n: &m::Nbody) -> impl Iterator<Item = &m::NbodyCluster> {
    n.children.iter().filter_map(|c| match c {
        m::NbodyChild::Cluster(x) => Some(x),
        m::NbodyChild::Member(_) => None,
    })
}

/// A cluster body's id.
fn body_id(c: &m::NbodyCluster, i: u64) -> String {
    format!("{}{i}", c.prefix)
}

/// Adds each cluster's bodies to the end of the composition as spheres of the cluster's radius named prefix +
/// index; a name another element already has is an error.
pub(crate) fn add_clusters(scene: &mut m::Scene) -> Result<(), String> {
    let Some(n) = scene.space.as_ref().and_then(|s| s.nbody.as_ref()) else { return Ok(()) };
    let mut taken = std::collections::HashSet::new();
    walk(&*scene, &mut |e| {
        if let Some(id) = e.element_id() {
            taken.insert(id.to_string());
        }
    });
    let mut added = Vec::new();
    for c in clusters(n) {
        for i in 0..c.count {
            let id = body_id(c, i);
            if !taken.insert(id.clone()) {
                return Err(format!("nbody: the cluster body {id:?} is named like another element"));
            }
            let xml = format!(r#"<object3D id="{id}" primitive="sphere" radius="{}"/>"#, c.radius.get());
            added.push(m::Node::from_fragment(&xml).map_err(|e| format!("nbody: cluster body {id:?}: {}", e.message))?);
        }
    }
    scene.composition.children.extend(added);
    Ok(())
}

impl Motion {
    /// The orbits and N-body system of a document whose space is `s`, its clock and frame `space` and its positions
    /// `positions`; why they cannot be read (a gravitational parameter or a system the rules refuse) otherwise.
    pub(crate) fn read(
        scene: &m::Scene,
        s: &m::Space,
        space: &Space,
        positions: Positions<'_>,
        pixels_per_meter: f64,
    ) -> Result<Motion, String> {
        let objects = objects(scene);
        let g = s.nbody.as_ref().map_or(sr_space::orbits::G, |n| n.g.get());
        let member = |id: &str| s.nbody.as_ref().and_then(|n| members(n).find(|m| m.r#ref == id));
        let mut orbits = HashMap::new();
        for (id, o) in &objects {
            let Some(e) = o.children.iter().find_map(|c| match c {
                m::Object3DChild::Orbit(e) => Some(e),
                _ => None,
            }) else {
                continue;
            };
            let centre = objects.get(e.around.as_str()).filter(|c| c.id != *id);
            let mu = match (e.mu, member(&e.around), centre) {
                (Some(mu), _, _) => Some(mu.get()),
                (None, Some(m), _) if m.mass.is_some() => m.mass.map(|x| g * x.get()),
                (None, Some(m), _) if m.body.is_some() => m.body.and_then(body_gm),
                (None, _, Some(c)) => c.body.and_then(body_gm),
                _ => None,
            };
            let mu =
                mu.ok_or_else(|| format!("orbit of {id:?}: {:?} gives no gravitational parameter (OR1)", e.around))?;
            let orbit = orbit(mu, e.a, e.e.get(), e.i, e.raan, e.argp, e.anomaly, e.epoch)
                .map_err(|err| format!("orbit of {id:?}: {err}"))?;
            orbits.insert(id.to_string(), (e.around.clone(), orbit));
        }
        let nbody = match &s.nbody {
            Some(n) => Some(Bodies::read(n, space, positions)?),
            None => None,
        };
        Ok(Motion { rate: s.rate.get(), pixels_per_meter, orbits, nbody })
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.orbits.is_empty() && self.nbody.is_none()
    }
}

impl Bodies {
    fn read(n: &m::Nbody, space: &Space, positions: Positions<'_>) -> Result<Bodies, String> {
        let g = n.g.get();
        let (mut masses, mut p, mut v, mut ids) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let et = space.time_at(0.0).map_err(|e| e.to_string())?;
        let frame = space.frame_matrix(et).map_err(|e| e.to_string())?;
        for x in members(n) {
            let mass = match (x.mass, x.body) {
                (Some(mass), _) => mass.get(),
                (None, Some(b)) => {
                    body_gm(b).ok_or_else(|| format!("nbody member {:?}: no GM for body {b} (OR3)", x.r#ref))? / g
                }
                (None, None) => return Err(format!("nbody member {:?} has neither a mass nor a body (OR3)", x.r#ref)),
            };
            let (r, u) = match x.body {
                Some(b) => {
                    let (r, u) = ephemeris_state(positions, b, space.origin, et)
                        .map_err(|e| format!("nbody member {:?}: {e}", x.r#ref))?;
                    // km and km/s in ICRF axes to metres and metres per second in the space frame
                    let to = |a: [f64; 3]| sr_space::mat_apply(&frame, a).map(|c| c * 1000.0);
                    (to(r), to(u))
                }
                None => ([x.x, x.y, x.z], [x.vx, x.vy, x.vz]),
            };
            masses.push(mass);
            p.push(r);
            v.push(u);
            ids.push(x.r#ref.clone());
        }
        for c in clusters(n) {
            let count = usize::try_from(c.count).map_err(|e| e.to_string())?;
            let s = plummer(c.seed as u64, count, c.mass.get(), c.scale.get(), g)
                .map_err(|e| format!("nbody cluster {:?}: {e}", c.prefix))?;
            masses.extend(s.masses);
            p.extend(s.positions.iter().map(|q| [q[0] + c.x, q[1] + c.y, q[2] + c.z]));
            v.extend(s.velocities.iter().map(|q| [q[0] + c.vx, q[1] + c.vy, q[2] + c.vz]));
            ids.extend((0..c.count).map(|i| body_id(c, i)));
        }
        let start = system(&masses, &p, &v, g, n.softening.get()).map_err(|e| format!("nbody: {e}"))?;
        let integrator = match n.integrator {
            m::Integrator::Leapfrog => Integrator::Leapfrog,
            m::Integrator::WisdomHolman => Integrator::WisdomHolman,
            m::Integrator::Ias15 => Integrator::Ias15,
        };
        Ok(Bodies {
            start,
            integrator,
            step: n.step.get(),
            tolerance: n.tolerance.map(|t| t.get()),
            ids,
            last: Mutex::new(None),
        })
    }

    /// Rule 6: the system `t` seconds of the space clock past its epoch. A fixed-step leapfrog or Wisdom–Holman system
    /// steps on from the last state reached when that is not later; IAS15, whose predictor carries between steps,
    /// and the adaptive step, whose sequence depends on the times landed on, integrate from the epoch.
    fn at(&self, t: f64) -> Result<System, String> {
        if let Some(tolerance) = self.tolerance {
            return integrate_adaptive(&self.start, t, self.step, tolerance, MAX_STEPS)
                .map(|(s, _)| s)
                .map_err(|e| format!("nbody: {e}"));
        }
        let cached = self.integrator != Integrator::Ias15;
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        let from = match last.as_ref() {
            Some(s) if cached && s.t <= t => s.clone(),
            _ => self.start.clone(),
        };
        let s = integrate(&from, self.integrator, self.step, t - from.t).map_err(|e| format!("nbody: {e}"))?;
        if cached {
            *last = Some(s.clone());
        }
        Ok(s)
    }
}

/// A body's position and velocity about `origin` at ephemeris time `et`, km and km/s in ICRF axes: the file's
/// state, or the approximate elements' position with its velocity differenced over ±[`VELOCITY_SPAN`] seconds.
fn ephemeris_state(positions: Positions<'_>, body: i32, origin: i32, et: f64) -> Result<([f64; 3], [f64; 3]), String> {
    match positions {
        Positions::File(spk) => spk.state(body, origin, et).map_err(|e| e.to_string()),
        Positions::Approximate => {
            let at = |t: f64| positions.position(body, origin, t).map_err(|e| e.to_string());
            let (before, after) = (at(et - VELOCITY_SPAN)?, at(et + VELOCITY_SPAN)?);
            let v = std::array::from_fn(|k| (after[k] - before[k]) / (2.0 * VELOCITY_SPAN));
            Ok((at(et)?, v))
        }
    }
}

/// A node's place: its x, y and z at the frame, else as written.
fn place_of(n: &FrameNode) -> [f64; 3] {
    ["x", "y", "z"].map(|k| {
        n.props
            .get(k)
            .and_then(|v| v.as_num())
            .or_else(|| n.elem.get_attr(k).and_then(|v| v.to_string().parse().ok()))
            .unwrap_or(0.0)
    })
}

fn set_place(n: &mut FrameNode, p: [f64; 3]) {
    for (k, v) in ["x", "y", "z"].into_iter().zip(p) {
        n.props.set(k, v);
    }
}

/// Places the orbiting and N-body objects of the frame at composition time `t`, once every other pose is final;
/// what cannot be placed (an orbit about its own descendant in the chain of centres, a system the integrator
/// refuses) is a failure of the frame.
pub(crate) fn resolve(p: &Program, nodes: &mut [FrameNode], t: f64, failures: &mut Vec<String>) {
    for (doc, sky) in p.skies.iter().enumerate() {
        let Some(Ok(sky)) = sky else { continue };
        let motion = sky.motion();
        if motion.is_empty() {
            continue;
        }
        let doc = doc as u16;
        let index: HashMap<std::sync::Arc<str>, usize> =
            nodes.iter().enumerate().filter(|(_, n)| n.doc == doc).map(|(i, n)| (n.id.clone(), i)).collect();
        let find = |id: &str| index.get(&*p.doc_key(doc, id)).copied();
        let tau = t * motion.rate;
        let mut placed: Vec<(usize, [f64; 3])> = Vec::new();
        if let Some(b) = &motion.nbody {
            match b.at(tau) {
                Ok(s) => placed.extend(
                    b.ids
                        .iter()
                        .zip(&s.positions)
                        .filter_map(|(id, r)| find(id).map(|i| (i, scene(*r, motion.pixels_per_meter)))),
                ),
                Err(e) => failures.push(e),
            }
        }
        for (i, q) in placed {
            set_place(&mut nodes[i], q);
        }
        // each orbit after its centre's, following the chain of centres up to one already placed or not an orbit
        let mut done: HashMap<String, [f64; 3]> = HashMap::new();
        let mut cycles: Vec<&str> = Vec::new();
        let mut ids: Vec<&String> = motion.orbits.keys().collect();
        ids.sort();
        for id in ids {
            let mut chain: Vec<&str> = Vec::new();
            let mut at = id.as_str();
            while motion.orbits.contains_key(at) && !done.contains_key(at) && !chain.contains(&at) {
                chain.push(at);
                at = &motion.orbits[at].0;
            }
            if chain.contains(&at) || cycles.contains(&at) {
                // each cycle is reported once, from the first of its objects in id order
                if !cycles.contains(&at) {
                    chain.push(at);
                    failures.push(format!("{id}: orbit: the centres form a cycle ({})", chain.join(" → ")));
                }
                cycles.extend(chain);
                continue;
            }
            // place down the chain, from the outermost centre in
            for object in chain.into_iter().rev() {
                let (around, o) = &motion.orbits[object];
                let centre = done.get(around).copied().or_else(|| find(around).map(|c| place_of(&nodes[c])));
                let (Some(centre), Some(i)) = (centre, find(object)) else { continue };
                match orbit_state(o, tau) {
                    Ok((r, _)) => {
                        let s = scene(r, motion.pixels_per_meter);
                        let q = [centre[0] + s[0], centre[1] + s[1], centre[2] + s[2]];
                        set_place(&mut nodes[i], q);
                        done.insert(object.to_string(), q);
                    }
                    Err(e) => failures.push(format!("{object}: orbit: {e}")),
                }
            }
        }
    }
}
