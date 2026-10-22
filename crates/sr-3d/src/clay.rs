//! Clay: solids defined by signed distance fields, joined by smooth unions (Quilez's polynomial
//! smooth minimum), carved by smooth subtraction, roughened by fingerprint-like displacement
//! and meshed every frame by naive surface nets (Gibson 1998, "Constrained elastic surface
//! nets"): one vertex per grid cell the surface crosses, at the mean of its edge crossings,
//! and one quad per crossed grid edge. Because the surface is re-extracted from the field,
//! blobs can merge and split as they animate, which a fixed mesh cannot.
//!
//! "Boil" re-seeds the displacement a number of times per second, the jitter of hand-animated
//! clay between frames.

use glam::{Quat, Vec3};
use rayon::prelude::*;

use crate::{Primitive, Vertex};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BlobShape {
    Sphere,
    /// Width, height, depth.
    Box,
    /// Along local y, `length` between the cap centres.
    Capsule,
    /// In the local xz plane: ring `radius`, tube `width` / 2.
    Torus,
}

/// One solid, in the object's local scene units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blob {
    pub shape: BlobShape,
    pub center: Vec3,
    pub rotation: Quat,
    pub radius: f32,
    pub size: Vec3,
    pub length: f32,
    /// Smooth-union (or subtraction) width.
    pub blend: f32,
    pub subtract: bool,
}

/// Surface roughness: `amount` (0–1) of fingerprint-like displacement, re-seeded `boil` times a
/// second (0: never).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Finish {
    pub amount: f32,
    pub seed: u64,
    pub boil: f32,
}

fn smin(a: f32, b: f32, k: f32) -> f32 {
    if k <= 0.0 {
        return a.min(b);
    }
    let h = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
    b + (a - b) * h - k * h * (1.0 - h)
}

/// `base` with `cut` carved out, smoothly over `k`.
fn ssub(base: f32, cut: f32, k: f32) -> f32 {
    if k <= 0.0 {
        return base.max(-cut);
    }
    let h = (0.5 - 0.5 * (base + cut) / k).clamp(0.0, 1.0);
    base + (-cut - base) * h + k * h * (1.0 - h)
}

impl Blob {
    /// Signed distance from `p` (object space) to this solid.
    pub fn sdf(&self, p: Vec3) -> f32 {
        let q = self.rotation.inverse() * (p - self.center);
        match self.shape {
            BlobShape::Sphere => q.length() - self.radius,
            BlobShape::Box => {
                let d = q.abs() - self.size * 0.5;
                d.max(Vec3::ZERO).length() + d.x.max(d.y).max(d.z).min(0.0)
            }
            BlobShape::Capsule => {
                let h = self.length * 0.5;
                let y = q.y.clamp(-h, h);
                (q - Vec3::new(0.0, y, 0.0)).length() - self.radius
            }
            BlobShape::Torus => {
                let ring = (q.x * q.x + q.z * q.z).sqrt() - self.radius;
                (ring * ring + q.y * q.y).sqrt() - self.size.x * 0.5
            }
        }
    }

    /// A box containing the solid (with its blend width).
    fn bounds(&self) -> (Vec3, Vec3) {
        let r = match self.shape {
            BlobShape::Sphere => self.radius,
            BlobShape::Box => self.size.length() * 0.5,
            BlobShape::Capsule => self.radius + self.length * 0.5,
            BlobShape::Torus => self.radius + self.size.x * 0.5,
        } + self.blend;
        (self.center - Vec3::splat(r), self.center + Vec3::splat(r))
    }
}

/// The joined field: blobs in order, each unioned (or subtracted) into the ones before it.
pub fn field(blobs: &[Blob], p: Vec3) -> f32 {
    let mut d = f32::MAX;
    for (k, b) in blobs.iter().enumerate() {
        let s = b.sdf(p);
        d = if k == 0 {
            if b.subtract {
                f32::MAX
            } else {
                s
            }
        } else if b.subtract {
            ssub(d, s, b.blend)
        } else {
            smin(d, s, b.blend)
        };
    }
    d
}

fn hash3(seed: u64, x: i64, y: i64, z: i64) -> f32 {
    let mut h = seed ^ (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= (z as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 29;
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// Trilinear value noise in [0, 1].
fn noise(seed: u64, p: Vec3) -> f32 {
    let f = p.floor();
    let t = p - f;
    let s = t * t * (Vec3::splat(3.0) - 2.0 * t);
    let (x, y, z) = (f.x as i64, f.y as i64, f.z as i64);
    let mut v = 0.0;
    for dz in 0..2 {
        for dy in 0..2 {
            for dx in 0..2 {
                let w = (if dx == 1 { s.x } else { 1.0 - s.x })
                    * (if dy == 1 { s.y } else { 1.0 - s.y })
                    * (if dz == 1 { s.z } else { 1.0 - s.z });
                v += w * hash3(seed, x + dx, y + dy, z + dz);
            }
        }
    }
    v
}

/// Displacement of the surface at `p` (scene units): smudges with ridges like fingerprints.
fn roughness(f: &Finish, seed: u64, p: Vec3) -> f32 {
    if f.amount <= 0.0 {
        return 0.0;
    }
    let broad = noise(seed, p / 14.0) - 0.5;
    // ridged bands: fingerprint whorls on a smaller scale
    let ridges = 1.0 - (2.0 * noise(seed ^ 0x51, p / 3.0) - 1.0).abs();
    f.amount * (2.5 * broad + 0.6 * ridges)
}

/// The clay's surface as a triangle mesh at time `t` (seconds, for the boil).
pub fn mesh(blobs: &[Blob], finish: &Finish, resolution: u32, t: f64) -> Primitive {
    mesh_with_budget(blobs, finish, resolution, t, usize::MAX).unwrap_or_default()
}

/// Extract the same surface with conservative grid/output admission before
/// allocation. Caller-owned blobs are outside this component allowance.
pub fn mesh_with_budget(
    blobs: &[Blob],
    finish: &Finish,
    resolution: u32,
    t: f64,
    max_bytes: usize,
) -> Result<Primitive, String> {
    extract(blobs, finish, resolution, t, max_bytes, true).map(|(prim, _)| prim)
}

/// [`mesh_with_budget`], with the roughness evaluated near the surface only (`banded`) or at every grid point, and the
/// number of grid points it was evaluated at.
fn extract(
    blobs: &[Blob],
    finish: &Finish,
    resolution: u32,
    t: f64,
    max_bytes: usize,
    banded: bool,
) -> Result<(Primitive, usize), String> {
    if resolution > 256
        || !t.is_finite()
        || !finish.amount.is_finite()
        || !finish.boil.is_finite()
        || !(0.0..=1.0).contains(&finish.amount)
        || finish.boil < 0.
        || blobs.iter().any(|b| {
            !b.center.is_finite()
                || !b.rotation.is_finite()
                || b.rotation.length_squared() == 0.
                || !b.size.is_finite()
                || b.size.min_element() <= 0.
                || !b.radius.is_finite()
                || b.radius <= 0.
                || !b.length.is_finite()
                || b.length < 0.
                || !b.blend.is_finite()
                || b.blend < 0.
        })
    {
        return Err("invalid clay field, time or resolution".into());
    }
    let mut prim = Primitive::default();
    if blobs.iter().all(|b| b.subtract) {
        return Ok((prim, 0));
    }
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for b in blobs.iter().filter(|b| !b.subtract) {
        let (l, h) = b.bounds();
        lo = lo.min(l);
        hi = hi.max(h);
    }
    let pad = 4.0 * finish.amount + 2.0;
    lo -= Vec3::splat(pad);
    hi += Vec3::splat(pad);
    let cell = (hi - lo).max_element() / resolution.max(4) as f32;
    if !lo.is_finite() || !hi.is_finite() || !cell.is_finite() || cell <= 0. {
        return Err("clay grid extent exceeds numeric precision".into());
    }
    let dims = ((hi - lo) / cell).ceil().as_uvec3() + glam::UVec3::ONE;
    let (nx, ny, nz) = (dims.x as usize, dims.y as usize, dims.z as usize);
    let points = nx.checked_mul(ny).and_then(|n| n.checked_mul(nz)).ok_or("clay grid size overflow")?;
    // At most one vertex per cell and six triangles per cell; allow Vec
    // growth capacity as well as both scalar/index grids during construction.
    let charge =
        points.checked_mul(8 + 2 * std::mem::size_of::<Vertex>() + 144).ok_or("clay memory budget overflow")?;
    if charge > max_bytes {
        return Err("clay grid and surface exceed memory budget".into());
    }
    if max_bytes != usize::MAX && points.saturating_mul(blobs.len()).saturating_mul(8) > 100_000_000 {
        return Err("clay field sampling exceeds work budget".into());
    }
    let seed = if finish.boil > 0.0 {
        finish.seed ^ ((t * finish.boil as f64).floor() as u64).wrapping_mul(0x2545_F491_4F6C_DD1D)
    } else {
        finish.seed
    };
    let sample = |p: Vec3| field(blobs, p) - roughness(finish, seed, p);
    // field at the grid points
    let at = |i: usize, j: usize, k: usize| lo + Vec3::new(i as f32, j as f32, k as f32) * cell;
    // Every pass below works on independent z slabs in parallel and joins them in slab order, so
    // the mesh is the same, vertex for vertex and index for index, as a serial sweep.
    let mut d = vec![0.0f32; nx * ny * nz];
    // The roughness moves the surface by at most 1.85 `amount` (|2.5 broad + 0.6 ridges| with broad in [-0.5, 0.5] and
    // ridges in [0, 1]), and the joined field is 1-Lipschitz (exact distances joined by smooth minima, whose gradients
    // are convex mixtures). A corner of a cell the surface crosses is therefore within that reach and a cell diagonal of
    // it; beyond twice the diagonal the smooth field alone has the sign of the rough one, and only signs are read there,
    // so the costly roughness is evaluated near the surface only.
    let reach = if banded { 1.85 * finish.amount.max(0.0) * 1.01 + 2.0 * 3f32.sqrt() * cell } else { f32::INFINITY };
    let rough = finish.amount > 0.0;
    let roughened: usize = d
        .par_chunks_mut(nx * ny)
        .enumerate()
        .map(|(k, slab)| {
            let mut roughened = 0;
            for j in 0..ny {
                for i in 0..nx {
                    let p = at(i, j, k);
                    let smooth = field(blobs, p);
                    slab[j * nx + i] = if smooth.abs() <= reach {
                        roughened += usize::from(rough);
                        smooth - roughness(finish, seed, p)
                    } else {
                        smooth
                    };
                }
            }
            roughened
        })
        .sum();
    let idx = |i: usize, j: usize, k: usize| (k * ny + j) * nx + i;
    // one vertex per cell crossed by the surface
    let (cx, cy, cz) = (nx - 1, ny - 1, nz - 1);
    let corners = [(0, 0, 0), (1, 0, 0), (0, 1, 0), (1, 1, 0), (0, 0, 1), (1, 0, 1), (0, 1, 1), (1, 1, 1)];
    let edges = [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)];
    let grad = |p: Vec3| {
        let e = cell * 0.5;
        Vec3::new(
            sample(p + Vec3::X * e) - sample(p - Vec3::X * e),
            sample(p + Vec3::Y * e) - sample(p - Vec3::Y * e),
            sample(p + Vec3::Z * e) - sample(p - Vec3::Z * e),
        )
        .normalize_or_zero()
    };
    let slabs: Vec<Vec<(usize, Vertex)>> = (0..cz)
        .into_par_iter()
        .map(|k| {
            let mut slab = Vec::new();
            for j in 0..cy {
                for i in 0..cx {
                    let v: [f32; 8] = std::array::from_fn(|c| {
                        let (a, b, e) = corners[c];
                        d[idx(i + a, j + b, k + e)]
                    });
                    let inside = v.iter().filter(|x| **x < 0.0).count();
                    if inside == 0 || inside == 8 {
                        continue;
                    }
                    let (mut sum, mut count) = (Vec3::ZERO, 0.0);
                    for (a, b) in edges {
                        if (v[a] < 0.0) != (v[b] < 0.0) {
                            let s = v[a] / (v[a] - v[b]);
                            let (pa, pb) = (corners[a], corners[b]);
                            let pa = Vec3::new(pa.0 as f32, pa.1 as f32, pa.2 as f32);
                            let pb = Vec3::new(pb.0 as f32, pb.1 as f32, pb.2 as f32);
                            sum += pa + (pb - pa) * s;
                            count += 1.0;
                        }
                    }
                    let p = lo + (Vec3::new(i as f32, j as f32, k as f32) + sum / count) * cell;
                    slab.push((
                        (k * cy + j) * cx + i,
                        Vertex {
                            pos: p.into(),
                            normal: grad(p).into(),
                            uv: [0.0, 0.0],
                            tangent: [1.0, 0.0, 0.0, 1.0],
                            ..Default::default()
                        },
                    ));
                }
            }
            slab
        })
        .collect();
    let mut vert_of = vec![u32::MAX; cx * cy * cz];
    prim.vertices.reserve(slabs.iter().map(Vec::len).sum());
    for (c, vertex) in slabs.into_iter().flatten() {
        vert_of[c] = prim.vertices.len() as u32;
        prim.vertices.push(vertex);
    }
    // one quad per grid edge the surface crosses, between the four cells around it
    let cell_vert = |i: usize, j: usize, k: usize| vert_of[(k * cy + j) * cx + i];
    let quads: Vec<Vec<u32>> = (0..nz - 1)
        .into_par_iter()
        .map(|k| {
            let mut indices = Vec::new();
            for j in 0..ny - 1 {
                for i in 0..nx - 1 {
                    let here = d[idx(i, j, k)];
                    for axis in 0..3 {
                        // A crossed edge may start on the first grid plane along
                        // its own axis. Only its two transverse coordinates need
                        // preceding cells; skipping all first planes opens holes.
                        if match axis {
                            0 => j == 0 || k == 0,
                            1 => i == 0 || k == 0,
                            _ => i == 0 || j == 0,
                        } {
                            continue;
                        }
                        let (ni, nj, nk) = match axis {
                            0 => (i + 1, j, k),
                            1 => (i, j + 1, k),
                            _ => (i, j, k + 1),
                        };
                        if ni >= nx || nj >= ny || nk >= nz {
                            continue;
                        }
                        let there = d[idx(ni, nj, nk)];
                        if (here < 0.0) == (there < 0.0) {
                            continue;
                        }
                        // the four cells sharing the edge (i, j, k) → (ni, nj, nk)
                        let q = match axis {
                            0 => [
                                cell_vert(i, j - 1, k - 1),
                                cell_vert(i, j, k - 1),
                                cell_vert(i, j, k),
                                cell_vert(i, j - 1, k),
                            ],
                            1 => [
                                cell_vert(i - 1, j, k - 1),
                                cell_vert(i - 1, j, k),
                                cell_vert(i, j, k),
                                cell_vert(i, j, k - 1),
                            ],
                            _ => [
                                cell_vert(i - 1, j - 1, k),
                                cell_vert(i, j - 1, k),
                                cell_vert(i, j, k),
                                cell_vert(i - 1, j, k),
                            ],
                        };
                        if q.contains(&u32::MAX) {
                            continue;
                        }
                        // wind outward: inside → outside along the axis keeps the order
                        let (a, b, c, e) = if here < 0.0 { (q[0], q[1], q[2], q[3]) } else { (q[0], q[3], q[2], q[1]) };
                        indices.extend_from_slice(&[a, b, c, a, c, e]);
                    }
                }
            }
            indices
        })
        .collect();
    prim.indices = quads.concat();
    Ok((prim, roughened))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sphere(x: f32, r: f32, blend: f32) -> Blob {
        Blob {
            shape: BlobShape::Sphere,
            center: Vec3::new(x, 0.0, 0.0),
            rotation: Quat::IDENTITY,
            radius: r,
            size: Vec3::splat(60.0),
            length: 60.0,
            blend,
            subtract: false,
        }
    }

    const SMOOTH: Finish = Finish { amount: 0.0, seed: 1, boil: 0.0 };

    #[test]
    fn bounded_meshing_rejects_grid_allocation_before_building() {
        let blobs = [sphere(0., 3., 0.)];
        assert!(mesh_with_budget(&blobs, &SMOOTH, 256, 0., 1 << 20).unwrap_err().contains("budget"));
        assert!(mesh_with_budget(&blobs, &SMOOTH, u32::MAX, 0., usize::MAX).is_err());
        let mesh = mesh_with_budget(&blobs, &SMOOTH, 32, 0., 32 << 20).unwrap();
        let sphere_volume = 4. / 3. * std::f32::consts::PI * 27.;
        assert!((volume(&mesh) / sphere_volume - 1.).abs() < 0.02);
        assert!(mesh_with_budget(&blobs, &SMOOTH, 10, f64::NAN, 1 << 20).is_err());
    }

    #[test]
    fn clay_sphere_is_a_closed_fracture_source_across_sizes() {
        for radius in [3., 18., 30.] {
            let mesh = mesh(&[sphere(0., radius, 0.)], &SMOOTH, 10, 0.);
            let points: Vec<_> = mesh.vertices.iter().map(|v| v.pos.map(f64::from)).collect();
            let triangles = mesh.indices.as_chunks::<3>().0;
            let pieces = crate::fracture::fracture(
                &points,
                triangles,
                crate::fracture::Spec { pieces: 4, seed: 42, ..Default::default() },
            );
            assert!(pieces.is_ok(), "radius={radius}: {pieces:?}");
        }
    }

    /// FNV-1a over the mesh's vertex and index bytes.
    fn digest(p: &Primitive) -> u64 {
        let bytes = bytemuck::cast_slice::<Vertex, u8>(&p.vertices).iter().chain(bytemuck::cast_slice(&p.indices));
        bytes.fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
    }

    /// Pinned from the serial extraction: how the work is split must not change a single vertex,
    /// normal or index, nor their order.
    #[test]
    fn extraction_reproduces_the_serial_mesh_bit_for_bit() {
        let mut ear = sphere(-30., 14., 12.);
        ear.shape = BlobShape::Capsule;
        ear.center.y = -60.;
        ear.rotation = Quat::from_rotation_z(-0.4);
        let mut socket = sphere(-12., 9., 4.);
        socket.center.z = -28.;
        socket.subtract = true;
        let blobs = [sphere(0., 40., 0.), sphere(30., 20., 18.), ear, socket];
        let finish = Finish { amount: 0.55, seed: 9, boil: 12.0 };
        let m = mesh(&blobs, &finish, 72, 0.25);
        assert_eq!((m.vertices.len(), m.indices.len()), (8127, 48756));
        assert_eq!(digest(&m), 5_028_264_707_182_578_705);
    }

    /// The roughness evaluated near the surface only gives the mesh it gives evaluated everywhere, over every shape (turned
    /// boxes, tori and capsules), carving, wide and zero blends, several roughness amounts and grid resolutions.
    #[test]
    fn roughness_near_the_surface_only_gives_the_same_mesh() {
        let turned = |shape: BlobShape, x: f32, y: f32, r: f32, blend: f32, subtract: bool, angle: f32| Blob {
            shape,
            center: Vec3::new(x, y, 0.3 * x),
            rotation: Quat::from_euler(glam::EulerRot::YXZ, angle, 0.7 * angle, -0.4 * angle),
            radius: r,
            size: Vec3::new(1.6 * r, 0.9 * r, 1.2 * r),
            length: 1.5 * r,
            blend,
            subtract,
        };
        let shapes = [
            turned(BlobShape::Sphere, 0., 0., 30., 0., false, 0.),
            turned(BlobShape::Box, 26., 10., 18., 14., false, 0.6),
            turned(BlobShape::Torus, -24., -6., 20., 22., false, 1.1),
            turned(BlobShape::Capsule, 4., -30., 9., 30., false, -0.8),
            turned(BlobShape::Sphere, -8., 6., 12., 4., true, 0.),
            turned(BlobShape::Box, 12., -12., 8., 0., true, 0.3),
        ];
        let mut checked = 0;
        for amount in [0.0, 0.25, 0.55, 1.0] {
            for resolution in [24, 61, 96] {
                for (seed, t) in [(9, 0.0), (31, 0.4)] {
                    let finish = Finish { amount, seed, boil: 12.0 };
                    let (banded, _) = extract(&shapes, &finish, resolution, t, usize::MAX, true).unwrap();
                    let (everywhere, _) = extract(&shapes, &finish, resolution, t, usize::MAX, false).unwrap();
                    assert!(!everywhere.indices.is_empty());
                    assert!(
                        digest(&banded) == digest(&everywhere),
                        "amount {amount}, resolution {resolution}, seed {seed}: the meshes differ"
                    );
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, 24);
    }

    /// A performance contract: the roughness is evaluated near the surface only, at under a quarter of the grid's points,
    /// for the seven-blob fingerprinted body of the clay showcase.
    #[test]
    fn roughness_is_evaluated_near_the_surface_only() {
        let at = |x: f32, y: f32, z: f32, r: f32, blend: f32, subtract: bool| Blob {
            center: Vec3::new(x, y, z),
            radius: r,
            blend,
            subtract,
            ..sphere(0., r, blend)
        };
        let blobs = [
            at(0., 0., 0., 95., 10., false),
            at(0., -118., 0., 64., 34., false),
            at(-38., -188., 0., 15., 14., false),
            at(38., -188., 0., 15., 14., false),
            at(-24., -128., -56., 17., 6., true),
            at(24., -128., -56., 17., 6., true),
            at(180., 64., 0., 38., 42., false),
        ];
        let finish = Finish { amount: 0.55, seed: 9, boil: 12.0 };
        // the grid's points: the extent of the solids with their blends and the roughness pad, over the cell
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for b in blobs.iter().filter(|b| !b.subtract) {
            let (l, h) = b.bounds();
            (lo, hi) = (lo.min(l), hi.max(h));
        }
        let (lo, hi) = (lo - Vec3::splat(4.0 * 0.55 + 2.0), hi + Vec3::splat(4.0 * 0.55 + 2.0));
        let cell = (hi - lo).max_element() / 160.0;
        let points = ((hi - lo) / cell).ceil().as_uvec3() + glam::UVec3::ONE;
        let points = (points.x * points.y * points.z) as usize;
        let (m, calls) = extract(&blobs, &finish, 160, 0.0, usize::MAX, true).unwrap();
        assert!(!m.indices.is_empty());
        let (_, everywhere) = extract(&blobs, &finish, 160, 0.0, usize::MAX, false).unwrap();
        assert_eq!(everywhere, points, "evaluated everywhere, it is evaluated at every grid point");
        println!("roughness at {calls} of {points} grid points");
        assert!(calls < points / 4, "roughness at {calls} of {points} grid points");
    }

    /// Signed volume by the divergence theorem (positive for outward winding).
    fn volume(p: &Primitive) -> f32 {
        p.indices
            .chunks_exact(3)
            .map(|t| {
                let v = |k: usize| Vec3::from(p.vertices[t[k] as usize].pos);
                v(0).dot(v(1).cross(v(2))) / 6.0
            })
            .sum()
    }

    /// Connected components of the mesh (by shared vertices).
    fn components(p: &Primitive) -> usize {
        let mut parent: Vec<usize> = (0..p.vertices.len()).collect();
        fn find(p: &mut [usize], x: usize) -> usize {
            let mut x = x;
            while p[x] != x {
                p[x] = p[p[x]];
                x = p[x];
            }
            x
        }
        for t in p.indices.chunks_exact(3) {
            for k in 1..3 {
                let (a, b) = (find(&mut parent, t[0] as usize), find(&mut parent, t[k] as usize));
                parent[a] = b;
            }
        }
        let used: std::collections::BTreeSet<usize> =
            p.indices.iter().map(|&i| find(&mut parent, i as usize)).collect();
        used.len()
    }

    #[test]
    fn a_sphere_meshes_closed_with_the_right_volume_and_outward_normals() {
        let m = mesh(&[sphere(0.0, 40.0, 0.0)], &SMOOTH, 64, 0.0);
        let v = volume(&m);
        let want = 4.0 / 3.0 * std::f32::consts::PI * 40.0f32.powi(3);
        assert!((v - want).abs() / want < 0.02, "volume {v} vs {want}");
        for vx in &m.vertices {
            let p = Vec3::from(vx.pos);
            assert!((p.length() - 40.0).abs() < 1.0, "on the surface: {}", p.length());
            assert!(Vec3::from(vx.normal).dot(p.normalize()) > 0.9, "outward normal");
        }
        // every edge is shared by exactly two triangles: closed
        let mut edges = std::collections::HashMap::new();
        for t in m.indices.chunks_exact(3) {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                *edges.entry((a.min(b), a.max(b))).or_insert(0) += 1;
            }
        }
        assert!(edges.values().all(|&c| c == 2), "open edges");
    }

    #[test]
    fn blobs_merge_when_they_meet_and_split_when_they_part() {
        // apart: two pieces; close with a smooth blend: one piece with a neck
        let apart = mesh(&[sphere(-50.0, 25.0, 10.0), sphere(50.0, 25.0, 10.0)], &SMOOTH, 64, 0.0);
        let near = mesh(&[sphere(-26.0, 25.0, 10.0), sphere(26.0, 25.0, 10.0)], &SMOOTH, 64, 0.0);
        assert_eq!(components(&apart), 2);
        assert_eq!(components(&near), 1);
        // subtracting a sphere carves a hole: less volume
        let mut cut = sphere(20.0, 20.0, 4.0);
        cut.subtract = true;
        let whole = volume(&mesh(&[sphere(0.0, 40.0, 0.0)], &SMOOTH, 64, 0.0));
        let carved = volume(&mesh(&[sphere(0.0, 40.0, 0.0), cut], &SMOOTH, 64, 0.0));
        assert!(carved < whole * 0.95, "{carved} vs {whole}");
    }

    #[test]
    fn fingerprints_roughen_and_boil_changes_the_surface_between_frames() {
        let clay = Finish { amount: 0.6, seed: 9, boil: 12.0 };
        let a = mesh(&[sphere(0.0, 40.0, 0.0)], &clay, 64, 0.0);
        let b = mesh(&[sphere(0.0, 40.0, 0.0)], &clay, 64, 0.04);
        let c = mesh(&[sphere(0.0, 40.0, 0.0)], &clay, 64, 0.1);
        // within one boil frame (1/12 s) the surface holds; across it, it changes
        assert_eq!(a.vertices.len(), b.vertices.len());
        assert!(a.vertices.iter().zip(&b.vertices).all(|(p, q)| p.pos == q.pos));
        assert!(
            a.vertices.len() != c.vertices.len() || a.vertices.iter().zip(&c.vertices).any(|(p, q)| p.pos != q.pos)
        );
        // the radius now varies around the sphere
        let r: Vec<f32> = a.vertices.iter().map(|v| Vec3::from(v.pos).length()).collect();
        let (lo, hi) = r.iter().fold((f32::MAX, f32::MIN), |(l, h), &x| (l.min(x), h.max(x)));
        assert!(hi - lo > 1.0, "{lo}..{hi}");
    }
}
