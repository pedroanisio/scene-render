//! The acceleration structure that hardware ray queries traverse in place of the tracer's own BVH.
//!
//! Its top level holds two bottom-level structures with an entry for every primitive of the scene's top level, in the
//! order the packed records have: one of triangles, where splats and instances are inactive, and one of boxes, where
//! the triangles are. The primitive index a query reports is then the record's own index, and the shader intersects a
//! box's splat or instance as the software walk does. Prototype triangles stay out: instances reach them through their
//! own BVHs in the packed nodes.

use super::{PtNode, PtScene};

/// The record markers of a splat and of an instance in `tri_mat`; anything else is a triangle.
const SPLAT: u32 = u32::MAX;
const INSTANCE: u32 = u32::MAX - 1;

/// The structures of one pass and the bind group (group 2) that hands the top level to the trace kernels.
pub(super) struct Rays {
    pub group: wgpu::BindGroup,
}

/// How many primitives the top-level BVH reaches: they come first in the packed records, in leaf order, and the
/// prototypes' triangles after them.
pub(super) fn top_level(nodes: &[PtNode], primitives: usize) -> usize {
    let mut end = 0usize;
    let mut stack = vec![0u32];
    while let Some(i) = stack.pop() {
        let Some(n) = nodes.get(i as usize) else { continue };
        if n.b > 0 {
            end = end.max(n.a as usize + n.b as usize);
        } else if n.a > 0 {
            stack.push(n.a);
            stack.push(n.a - 1);
        }
    }
    end.min(primitives)
}

/// What the structures of a scene are built from.
pub(super) struct Geometry {
    /// The vertices of the triangle structure: three per primitive, NaN for an inactive one.
    vertices: Vec<[f32; 4]>,
    /// Whether any primitive is an active triangle.
    triangles: bool,
    /// The boxes of the box structure, min then max, one per primitive, NaN for an inactive one.
    boxes: Vec<[f32; 6]>,
    /// Whether any primitive is an active box.
    others: bool,
}

/// The geometry of the primitives the top level of `data`'s BVH reaches.
pub(super) fn geometry(data: &PtScene) -> Geometry {
    let top = top_level(&data.nodes, data.tri_mat.len());
    let mut vertices = vec![[f32::NAN, 0.0, 0.0, 0.0]; top * 3];
    let mut boxes = vec![[f32::NAN, 0.0, 0.0, 0.0, 0.0, 0.0]; top];
    let (mut triangles, mut others) = (false, false);
    for (k, &material) in data.tri_mat.iter().take(top).enumerate() {
        let corners = &data.pos[3 * k..3 * k + 3];
        if material == SPLAT || material == INSTANCE {
            let lo = std::array::from_fn::<f32, 3, _>(|a| corners.iter().map(|c| c[a]).fold(f32::INFINITY, f32::min));
            let hi =
                std::array::from_fn::<f32, 3, _>(|a| corners.iter().map(|c| c[a]).fold(f32::NEG_INFINITY, f32::max));
            boxes[k] = [lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]];
            others = true;
        } else {
            for (c, corner) in corners.iter().enumerate() {
                vertices[3 * k + c] = [corner[0], corner[1], corner[2], 0.0];
            }
            triangles = true;
        }
    }
    Geometry { vertices, triangles, boxes, others }
}

/// Builds the structures of `data` into `enc`, before the passes that trace them. None when the scene has nothing to
/// hit, or more primitives than the device's structures hold: the pass then walks its own BVH.
pub(super) fn build(
    d: &wgpu::Device,
    enc: &mut wgpu::CommandEncoder,
    data: &PtScene,
    layout: &wgpu::BindGroupLayout,
) -> Option<Rays> {
    let Geometry { vertices, triangles, boxes, others } = geometry(data);
    if !(triangles || others) || boxes.len() as u64 > u64::from(d.limits().max_blas_primitive_count) {
        return None;
    }
    use wgpu::util::DeviceExt;
    let input = |label: &str, bytes: &[u8]| {
        d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytes,
            usage: wgpu::BufferUsages::BLAS_INPUT,
        })
    };
    let structure = |label: &str, sizes: wgpu::BlasGeometrySizeDescriptors| {
        d.create_blas(
            &wgpu::CreateBlasDescriptor {
                label: Some(label),
                flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                update_mode: wgpu::AccelerationStructureUpdateMode::Build,
            },
            sizes,
        )
    };
    let triangle_size = wgpu::BlasTriangleGeometrySizeDescriptor {
        vertex_format: wgpu::VertexFormat::Float32x3,
        vertex_count: vertices.len() as u32,
        index_format: None,
        index_count: None,
        flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
    };
    let box_size = wgpu::BlasAABBGeometrySizeDescriptor {
        primitive_count: boxes.len() as u32,
        flags: wgpu::AccelerationStructureGeometryFlags::empty(),
    };
    let triangle_input = triangles.then(|| input("pt-ray-triangles", bytemuck::cast_slice(&vertices)));
    let box_input = others.then(|| input("pt-ray-boxes", bytemuck::cast_slice(&boxes)));
    let triangle_blas = triangles.then(|| {
        structure(
            "pt-ray-triangles",
            wgpu::BlasGeometrySizeDescriptors::Triangles { descriptors: vec![triangle_size.clone()] },
        )
    });
    let box_blas = others.then(|| {
        structure("pt-ray-boxes", wgpu::BlasGeometrySizeDescriptors::AABBs { descriptors: vec![box_size.clone()] })
    });
    let mut entries = Vec::new();
    if let (Some(blas), Some(buffer)) = (&triangle_blas, &triangle_input) {
        entries.push(wgpu::BlasBuildEntry {
            blas,
            geometry: wgpu::BlasGeometries::TriangleGeometries(vec![wgpu::BlasTriangleGeometry {
                size: &triangle_size,
                vertex_buffer: buffer,
                first_vertex: 0,
                vertex_stride: 16,
                index_buffer: None,
                first_index: None,
                transform_buffer: None,
                transform_buffer_offset: None,
            }]),
        });
    }
    if let (Some(blas), Some(buffer)) = (&box_blas, &box_input) {
        entries.push(wgpu::BlasBuildEntry {
            blas,
            geometry: wgpu::BlasGeometries::AabbGeometries(vec![wgpu::BlasAabbGeometry {
                size: &box_size,
                stride: wgpu::AABB_GEOMETRY_MIN_STRIDE,
                aabb_buffer: buffer,
                primitive_offset: 0,
            }]),
        });
    }
    let mut tlas = d.create_tlas(&wgpu::CreateTlasDescriptor {
        label: Some("pt-rays"),
        max_instances: entries.len() as u32,
        flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
        update_mode: wgpu::AccelerationStructureUpdateMode::Build,
    });
    const IDENTITY: [f32; 12] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    for (i, entry) in entries.iter().enumerate() {
        tlas[i] = Some(wgpu::TlasInstance::new(entry.blas, IDENTITY, 0, 0xff));
    }
    enc.build_acceleration_structures(&entries, [&tlas]);
    let group = d.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("pathtrace-rays"),
        layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: tlas.as_binding() }],
    });
    Some(Rays { group })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(a: u32, b: u32) -> PtNode {
        PtNode { lo: [0.0; 3], a, hi: [1.0; 3], b }
    }

    #[test]
    fn the_top_level_ends_where_the_root_bvh_s_last_leaf_does() {
        // root 0 with children 2 and 1 (`a` and `a - 1`), leaves over records 0..3 and 3..5; a prototype BVH after it
        let nodes = [node(2, 0), node(3, 2), node(0, 3), node(5, 4)];
        assert_eq!(top_level(&nodes, 9), 5, "the prototype's leaf over 5..9 is not reached from the root");
        assert_eq!(top_level(&[node(0, 4)], 4), 4, "a root that is a leaf");
        assert_eq!(top_level(&[node(0, 1)], 0), 0, "the empty scene's root leaf has no record");
        assert_eq!(top_level(&[], 3), 0);
    }

    #[test]
    fn boxes_bound_splats_and_instances_and_triangles_keep_their_corners() {
        let data = PtScene {
            nodes: vec![node(0, 3)],
            tri_mat: vec![2, SPLAT, INSTANCE, 0],
            pos: vec![
                [0.0, 0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [-1.0, -2.0, -3.0, 0.0],
                [1.0, 2.0, 3.0, 0.0],
                [0.0, 0.0, 0.0, 0.0],
                [4.0, 5.0, 6.0, 0.0],
                [3.0, 4.0, 5.0, 0.0],
                [0.0, 0.0, 0.0, 0.0],
                [9.0, 9.0, 9.0, 0.0],
                [9.0, 9.0, 9.0, 0.0],
                [9.0, 9.0, 9.0, 0.0],
            ],
            ..PtScene::default()
        };
        let Geometry { vertices, triangles, boxes, others } = geometry(&data);
        assert!(triangles && others);
        assert_eq!(vertices.len(), 9, "the prototype triangle after the top level is left out");
        assert_eq!(&vertices[..3], &[[0.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]]);
        assert!(vertices[3..].iter().all(|v| v[0].is_nan()), "splats and instances are no triangles");
        assert!(boxes[0][0].is_nan(), "a triangle is no box");
        assert_eq!(boxes[1], [-1.0, -2.0, -3.0, 1.0, 2.0, 3.0]);
        assert_eq!(boxes[2], [0.0, 0.0, 0.0, 4.0, 5.0, 6.0]);
    }
}
