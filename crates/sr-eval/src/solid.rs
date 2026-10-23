//! Shared procedural solid inputs for render and simulation geometry.
use crate::FrameNode;
use glam::{Quat, Vec3};
use sr_model::element::children;
use std::collections::HashMap;
use std::sync::Arc;

pub struct Clay {
    pub blobs: Vec<sr_3d::clay::Blob>,
    pub finish: sr_3d::clay::Finish,
    pub resolution: u32,
}

/// Resolve each animated blob on the evaluated object's local clock.
pub fn clay(n: &FrameNode) -> Clay {
    clay_inputs(&n.elem, &n.seed_id, &n.props, &n.parts)
}

fn clay_inputs(
    elem: &sr_model::model::Node,
    id: &str,
    values: &crate::Props,
    parts: &[crate::eval::ElementState],
) -> Clay {
    let mut blobs = Vec::new();
    for (k, c) in children(elem).into_iter().filter(|c| c.element_name() == "blob").enumerate() {
        let key = format!("{}/blob[{k}]", id);
        let props = parts.iter().find(|p| *p.key == key).map(|p| &p.props);
        let f = |key, default| {
            props
                .and_then(|p| p.get(key))
                .and_then(crate::Value::as_num)
                .unwrap_or_else(|| crate::sim::num(c, key, default)) as f32
        };
        let text = |key| {
            props
                .and_then(|p| p.get(key))
                .and_then(|v| match v {
                    crate::Value::Str(s) => Some(s.to_string()),
                    _ => None,
                })
                .or_else(|| crate::sim::text(c, key))
        };
        blobs.push(sr_3d::clay::Blob {
            shape: match text("shape").as_deref() {
                Some("box") => sr_3d::clay::BlobShape::Box,
                Some("capsule") => sr_3d::clay::BlobShape::Capsule,
                Some("torus") => sr_3d::clay::BlobShape::Torus,
                _ => sr_3d::clay::BlobShape::Sphere,
            },
            center: Vec3::new(f("x", 0.), f("y", 0.), f("z", 0.)),
            rotation: Quat::from_euler(
                glam::EulerRot::YXZ,
                f("rotationY", 0.).to_radians(),
                f("rotationX", 0.).to_radians(),
                f("rotation", 0.).to_radians(),
            ),
            radius: f("radius", 30.),
            size: Vec3::new(f("width", 60.), f("height", 60.), f("depth", 60.)),
            length: f("length", 60.),
            blend: f("blend", 10.),
            subtract: f("subtract", 0.) != 0. || text("subtract").as_deref() == Some("true"),
        });
    }
    let value = |key, default| {
        values.get(key).and_then(crate::Value::as_num).unwrap_or_else(|| crate::sim::num(elem, key, default))
    };
    let seed = match elem {
        sr_model::model::Node::Object3D(o) => o.seed,
        _ => None,
    }
    .unwrap_or_else(|| crate::rng::hash_str(id));
    Clay {
        blobs,
        finish: sr_3d::clay::Finish { amount: value("fingerprints", 0.) as f32, seed, boil: value("boil", 0.) as f32 },
        resolution: value("resolution", 64.).clamp(8., 256.) as u32,
    }
}

/// A clay object's extracted surface, and the [`sr_3d::clay::key`] of the inputs it was extracted from.
#[derive(Debug)]
pub struct ClayMesh {
    pub key: u64,
    pub mesh: Arc<(Vec<sr_3d::Vertex>, Vec<u32>)>,
}

/// The clay surfaces of the last frame given to [`attach_clay`], by node id.
#[derive(Default)]
pub struct ClayCache(HashMap<Arc<str>, Arc<ClayMesh>>);

/// Whether `n` draws a clay solid.
pub fn is_clay(n: &FrameNode) -> bool {
    n.kind == "object3D"
        && match n.props.get("primitive") {
            Some(crate::Value::Str(s)) => &**s == "clay",
            _ => crate::sim::text(&*n.elem, "primitive").as_deref() == Some("clay"),
        }
}

/// The key of a clay object's surface at this time.
pub fn clay_key(spec: &Clay, n: &FrameNode) -> u64 {
    sr_3d::clay::key(&spec.blobs, &spec.finish, spec.resolution, n.local_time)
}

/// Extracts the surface of `spec` at `n`'s time.
pub fn extract_clay(spec: &Clay, n: &FrameNode) -> Arc<(Vec<sr_3d::Vertex>, Vec<u32>)> {
    let prim = sr_3d::clay::mesh(&spec.blobs, &spec.finish, spec.resolution, n.local_time);
    Arc::new((prim.vertices, prim.indices))
}

/// Gives every drawn clay object of `g` its surface ([`FrameNode::clay_mesh`]): the cached one while its inputs are
/// unchanged, otherwise extracted, the objects in parallel. The cache then holds this frame's surfaces only.
pub fn attach_clay(g: &mut crate::FrameGraph, cache: &mut ClayCache) {
    use rayon::prelude::*;
    let mut todo = Vec::new();
    let mut kept = HashMap::new();
    for (i, n) in g.nodes.iter().enumerate().filter(|(_, n)| n.draw && is_clay(n)) {
        let spec = clay(n);
        let key = clay_key(&spec, n);
        match cache.0.get(&n.id).filter(|m| m.key == key) {
            Some(m) => {
                kept.insert(n.id.clone(), m.clone());
            }
            None => todo.push((i, spec, key)),
        }
    }
    let nodes = &g.nodes;
    let made: Vec<(usize, Arc<ClayMesh>)> = todo
        .par_iter()
        .map(|(i, spec, key)| (*i, Arc::new(ClayMesh { key: *key, mesh: extract_clay(spec, &nodes[*i]) })))
        .collect();
    for (i, m) in made {
        kept.insert(g.nodes[i].id.clone(), m);
    }
    for n in g.nodes.iter_mut() {
        if let Some(m) = kept.get(&n.id) {
            n.clay_mesh = Some(m.clone());
        }
    }
    cache.0 = kept;
}

pub fn clay_mesh(n: &FrameNode, budget: usize) -> Result<sr_3d::Primitive, String> {
    let spec = clay(n);
    sr_3d::clay::mesh_with_budget(&spec.blobs, &spec.finish, spec.resolution, n.local_time, budget)
}

/// Shape and extrude text using the same font discovery and outline path as
/// the renderer. Document font files are resolved against their owning base.
pub fn text_mesh(p: &crate::Program, n: &FrameNode, budget: usize) -> Result<sr_3d::Primitive, String> {
    let value = |key, default| {
        n.props.get(key).and_then(crate::Value::as_num).unwrap_or_else(|| crate::sim::num(&*n.elem, key, default))
    };
    let text = |key| {
        n.props
            .get(key)
            .and_then(|v| match v {
                crate::Value::Str(s) => Some(s.to_string()),
                _ => None,
            })
            .or_else(|| crate::sim::text(&*n.elem, key))
    };
    let content = n.text.as_deref().map(str::to_owned).or_else(|| text("text")).unwrap_or_default();
    text_geometry(
        p,
        TextSolid {
            content,
            family: text("font"),
            height: value("height", 100.),
            tracking: value("tracking", 0.),
            depth: value("depth", 10.) as f32,
            bevel: value("bevel", 0.) as f32,
        },
        budget,
    )
}

struct TextSolid {
    content: String,
    family: Option<String>,
    height: f64,
    tracking: f64,
    depth: f32,
    bevel: f32,
}
fn text_geometry(p: &crate::Program, spec: TextSolid, budget: usize) -> Result<sr_3d::Primitive, String> {
    // SREP 21: under fontPolicy="pinned" only the document's font assets, never the host's fonts
    let pinned = p.fonts_pinned();
    let mut lib = if pinned { sr_text::FontLib::pinned() } else { sr_text::FontLib::new(true) };
    let mut remaining = budget;
    for (doc, scene) in std::iter::once(&p.scene).chain(p.includes.iter().map(|i| &i.1)).enumerate() {
        for font in crate::font_policy::font_assets(scene) {
            let base = p.base_dirs.get(doc).map(|p| p.as_path()).unwrap_or_else(|| std::path::Path::new(""));
            let sr_model::assets::Resolved::Local(path) = sr_model::assets::resolve(&font.src, base) else {
                return Err("text solid font requires a resolved local file".into());
            };
            let bytes = usize::try_from(std::fs::metadata(&path).map_err(|e| e.to_string())?.len())
                .map_err(|_| "text solid font size overflow")?;
            remaining = remaining.checked_sub(bytes).ok_or("text solid font memory budget")?;
            let face = if pinned {
                lib.pin(&path, font.index, &font.family, font.weight, font.italic)
            } else {
                lib.file(&path, font.index)
            };
            if face.is_none() {
                return Err(format!("text solid font cannot be decoded: {}", path.display()));
            }
        }
    }
    let polygons = sr_text::extrusion::outline_polygons_tracked(
        &mut lib,
        &spec.content,
        spec.family.as_deref(),
        spec.height,
        spec.tracking,
        0.25,
        remaining,
    )?;
    let mut polygons: Vec<Vec<glam::Vec2>> = polygons
        .into_iter()
        .map(|p| p.into_iter().map(|q| glam::Vec2::new(q[0] as f32, q[1] as f32)).collect())
        .collect();
    let (lo, hi) = polygons
        .iter()
        .flatten()
        .fold((glam::Vec2::splat(f32::MAX), glam::Vec2::splat(f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    let center = (lo + hi) * 0.5;
    polygons.iter_mut().flatten().for_each(|p| *p -= center);
    sr_3d::prim::extrude(&polygons, spec.depth, spec.bevel)
}

/// Static shape attributes are validated before consumers use this path.
/// Construction is independent of render conditions and activation windows.
pub(crate) fn collider_triangles(
    p: &crate::Program,
    node: &crate::program::InstNode,
    budget: usize,
) -> Result<crate::sim3d::Triangles, String> {
    let e = &*node.elem;
    let text = |key| crate::sim::text(e, key);
    let value = |key, default| crate::sim::num(e, key, default);
    let mesh = match text("primitive").as_deref() {
        Some("clay") => {
            let spec = clay_inputs(e, &node.seed_id, &Default::default(), &[]);
            sr_3d::clay::mesh_with_budget(&spec.blobs, &spec.finish, spec.resolution, 0., budget)?
        }
        Some("text") => text_geometry(
            p,
            TextSolid {
                content: text("text").unwrap_or_default(),
                family: text("font"),
                height: value("height", 100.),
                tracking: value("tracking", 0.),
                depth: value("depth", 10.) as f32,
                bevel: value("bevel", 0.) as f32,
            },
            budget,
        )?,
        Some("extrude") => {
            let path = text("path").ok_or("solid collider path is missing")?;
            if path.len().saturating_mul(1024) > budget {
                return Err("solid collider path exceeds memory budget".into());
            }
            sr_3d::prim::extrude(
                &sr_3d::prim::path_polygons(&path, 0.25)?,
                value("depth", 10.) as f32,
                value("bevel", 0.) as f32,
            )?
        }
        _ => return Err("unsupported procedural solid collider".into()),
    };
    if mesh.vertices.len().saturating_mul(256).saturating_add(mesh.indices.len().saturating_mul(32)) > budget {
        return Err("solid collider topology exceeds memory budget".into());
    }
    Ok((mesh.vertices.iter().map(|v| v.pos.map(f64::from)).collect(), mesh.indices.as_chunks::<3>().0.to_vec()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn clay_seed_retains_all_authored_bits() {
        for seed in [u64::MAX - 1, u64::MAX] {
            let xml = format!(
                r#"<scene version="1.3"><project width="8" height="8" fps="1" duration="1"/>
                <composition><object3D id="clay" primitive="clay" seed="{seed}"><blob/></object3D></composition></scene>"#
            );
            let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
            let ev = crate::Evaluator::new(&doc, &Default::default()).unwrap();
            let frame = ev.evaluate(0.);
            assert_eq!(super::clay(&frame.nodes[0]).finish.seed, seed);
        }
    }

    #[test]
    fn clay_is_extracted_ahead_and_reused_while_its_inputs_hold() {
        // two boiling objects (a new surface every half second) and one that never boils
        let xml = r#"<scene version="1.3"><project width="8" height="8" fps="4" duration="2"/>
            <composition>
              <object3D id="a" primitive="clay" resolution="16" fingerprints="0.4" boil="2"><blob radius="20"/></object3D>
              <object3D id="b" primitive="clay" resolution="20" fingerprints="0.4" boil="2"><blob radius="15"/></object3D>
              <object3D id="still" primitive="clay" resolution="16"><blob radius="10"/></object3D>
              <object3D id="box" primitive="box"/>
            </composition></scene>"#;
        let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
        let ev = crate::Evaluator::new(&doc, &Default::default()).unwrap();
        let mut cache = super::ClayCache::default();
        let frame = |t: f64, cache: &mut super::ClayCache| {
            let mut g = ev.evaluate(t);
            super::attach_clay(&mut g, cache);
            g
        };
        let mesh = |g: &crate::FrameGraph, id: &str| g.nodes.iter().find(|n| &*n.id == id).unwrap().clay_mesh.clone();
        let g0 = frame(0.0, &mut cache);
        assert!(mesh(&g0, "box").is_none());
        for id in ["a", "b", "still"] {
            // what the renderer would extract itself, vertex for vertex
            let n = g0.nodes.iter().find(|n| &*n.id == id).unwrap();
            let spec = super::clay(n);
            let made = mesh(&g0, id).expect("clay gets its surface");
            assert_eq!(made.key, super::clay_key(&spec, n));
            let own = super::extract_clay(&spec, n);
            assert!(!own.1.is_empty());
            assert_eq!(made.mesh.1, own.1);
            assert!(made.mesh.0.iter().zip(&own.0).all(|(p, q)| p.pos == q.pos && p.normal == q.normal));
        }
        // within the boil step every surface is reused; past it, only the still one is
        let g1 = frame(0.25, &mut cache);
        for id in ["a", "b", "still"] {
            assert!(std::sync::Arc::ptr_eq(&mesh(&g0, id).unwrap(), &mesh(&g1, id).unwrap()), "{id}");
        }
        let g2 = frame(0.5, &mut cache);
        assert!(std::sync::Arc::ptr_eq(&mesh(&g0, "still").unwrap(), &mesh(&g2, "still").unwrap()));
        for id in ["a", "b"] {
            assert!(!std::sync::Arc::ptr_eq(&mesh(&g0, id).unwrap(), &mesh(&g2, id).unwrap()), "{id}");
            assert_ne!(mesh(&g0, id).unwrap().key, mesh(&g2, id).unwrap().key);
        }
        // a frame without an object drops its surface from the cache
        let mut g3 = ev.evaluate(0.5);
        g3.nodes.retain(|n| &*n.id != "a");
        super::attach_clay(&mut g3, &mut cache);
        let g4 = frame(0.5, &mut cache);
        assert!(!std::sync::Arc::ptr_eq(&mesh(&g2, "a").unwrap(), &mesh(&g4, "a").unwrap()));
        assert!(std::sync::Arc::ptr_eq(&mesh(&g2, "b").unwrap(), &mesh(&g4, "b").unwrap()));
    }

    #[test]
    fn text_solids_take_their_tracking_from_the_object() {
        // the collider path builds the text mesh through text_geometry: tracking must widen it like the drawn mesh
        let font = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/solid.ttf");
        let xml = format!(
            r#"<scene version="1.3"><project width="8" height="8" fps="1" duration="1"/>
            <assets><font id="face" family="SR Solid Test" src="{}"/></assets>
            <composition><object3D id="t" primitive="text" text="OO" font="SR Solid Test" height="10" tracking="1000"/></composition></scene>"#,
            font.display()
        );
        let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
        let ev = crate::Evaluator::new(&doc, &Default::default()).unwrap();
        let width = |tracking: f64| {
            let spec = super::TextSolid {
                content: "OO".into(),
                family: Some("SR Solid Test".into()),
                height: 10.0,
                tracking,
                depth: 2.0,
                bevel: 0.0,
            };
            let mesh = super::text_geometry(ev.program(), spec, usize::MAX).unwrap();
            let xs = mesh.vertices.iter().map(|v| v.pos[0]);
            xs.clone().fold(f32::MIN, f32::max) - xs.fold(f32::MAX, f32::min)
        };
        // the second glyph moves one em (10 px at height 10) further right
        assert!((width(1000.0) - width(0.0) - 10.0).abs() < 1e-3, "{} {}", width(0.0), width(1000.0));
    }
}
