//! CPU port of the path tracer's ray generation helpers and BVH traversal (`pathtrace.wgsl`: `pcg`, `rnd`, `tri_hit`,
//! `box_hit`, `prototype_hit`, `trace`), reading the same data the shader reads from `PtScene`. Constants, epsilons and
//! operation order follow the shader, in f32 throughout.
// The plug-in pass wires this module into the render path; until then only the tests call it.
#![cfg_attr(not(test), allow(dead_code))]

use super::{PtNode, PtScene};

/// The shader's `rng` private variable.
pub(crate) struct Rng(pub u32);

pub(crate) fn pcg(v: u32) -> u32 {
    let s = v.wrapping_mul(747796405).wrapping_add(2891336453);
    let w = ((s >> ((s >> 28) + 4)) ^ s).wrapping_mul(277803737);
    (w >> 22) ^ w
}

impl Rng {
    pub(crate) fn rnd(&mut self) -> f32 {
        self.0 = pcg(self.0);
        (self.0 >> 8) as f32 / 16777216.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Hit {
    pub t: f32,
    pub u: f32,
    pub v: f32,
    pub tri: u32,
    pub instance: u32,
}

const NONE: u32 = 0xffff_ffff;
const PROTOTYPE: u32 = 0xffff_fffe;

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn xyz(p: &[f32; 4]) -> [f32; 3] {
    [p[0], p[1], p[2]]
}

/// `inverse * vec4(v, w)` for the instance record's four rows, which are the matrix's columns.
fn transform(rows: &[[f32; 4]], v: [f32; 3], w: f32) -> [f32; 3] {
    let mut out = [0.0; 3];
    for (i, o) in out.iter_mut().enumerate() {
        *o = rows[0][i] * v[0] + rows[1][i] * v[1] + rows[2][i] * v[2] + rows[3][i] * w;
    }
    out
}

/// The shader's `tri_hit`. A splat (material `0xffffffff`) is not supported here: it leaves the hit unchanged, and the
/// plug-in pass refuses scenes that have splats.
fn tri_hit(scene: &PtScene, k: usize, o: [f32; 3], d: [f32; 3], tmax: f32, h: &mut Hit, instance: u32) {
    if scene.tri_mat[k] == NONE {
        return;
    }
    let p0 = xyz(&scene.pos[k * 3]);
    let e1 = sub(xyz(&scene.pos[k * 3 + 1]), p0);
    let e2 = sub(xyz(&scene.pos[k * 3 + 2]), p0);
    let pv = cross(d, e2);
    let det = dot(e1, pv);
    let mut determinant_scale = 1.0;
    if instance != NONE {
        determinant_scale = scene.instances[&(instance as usize)][13][2];
    }
    if (det * determinant_scale).abs() < 1e-12 {
        return;
    }
    let inv = 1.0 / det;
    let s = sub(o, p0);
    let u = dot(s, pv) * inv;
    if !(0.0..=1.0).contains(&u) {
        return;
    }
    let q = cross(s, e1);
    let v = dot(d, q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return;
    }
    let t = dot(e2, q) * inv;
    if t > 1e-4 && t < tmax.min(h.t) {
        *h = Hit { t, u, v, tri: k as u32, instance };
    }
}

/// The shader's `box_hit`; `inv` is the reciprocal direction.
fn box_hit(n: &PtNode, o: [f32; 3], inv: [f32; 3], tmax: f32) -> bool {
    if n.lo[0] > n.hi[0] || n.lo[1] > n.hi[1] || n.lo[2] > n.hi[2] {
        return false;
    }
    let t0 = [0, 1, 2].map(|i| (n.lo[i] - o[i]) * inv[i]);
    let t1 = [0, 1, 2].map(|i| (n.hi[i] - o[i]) * inv[i]);
    let tn = t0[0].min(t1[0]).max(t0[1].min(t1[1])).max(t0[2].min(t1[2]));
    let tf = t0[0].max(t1[0]).min(t0[1].max(t1[1])).min(t0[2].max(t1[2]));
    tf >= tn.max(0.0) && tn < tmax
}

fn reciprocal(d: [f32; 3]) -> [f32; 3] {
    d.map(|c| 1.0 / if c.abs() < 1e-12 { 1e-12 } else { c })
}

/// The shader's `prototype_hit`: the ray in the instance's local space, walking the shared prototype BVH.
fn prototype_hit(scene: &PtScene, instance: usize, world_o: [f32; 3], world_d: [f32; 3], tmax: f32, h: &mut Hit) {
    let record = &scene.instances[&instance];
    let inverse = &record[5..9];
    let o = transform(inverse, world_o, 1.0);
    // Not normalized: the ray parameter remains the world-space distance.
    let d = transform(inverse, world_d, 0.0);
    let inv = reciprocal(d);
    let mut stack = [0u32; 64];
    stack[0] = record[13][0].to_bits();
    let mut sp = 1usize;
    while sp > 0 {
        sp -= 1;
        let n = &scene.nodes[stack[sp] as usize];
        if !box_hit(n, o, inv, h.t) {
            continue;
        }
        if n.b > 0 {
            for k in n.a..n.a + n.b {
                tri_hit(scene, k as usize, o, d, tmax, h, instance as u32);
            }
        } else if sp < 62 {
            stack[sp] = n.a;
            stack[sp + 1] = n.a - 1;
            sp += 2;
        }
    }
}

/// The shader's `trace`: the nearest hit along the ray within `tmax` (`tri == 0xffffffff` when there is none).
pub(crate) fn trace(scene: &PtScene, o: [f32; 3], d: [f32; 3], tmax: f32) -> Hit {
    let mut h = Hit { t: tmax, u: 0.0, v: 0.0, tri: NONE, instance: NONE };
    let inv = reciprocal(d);
    let mut stack = [0u32; 64];
    let mut sp = 1usize;
    while sp > 0 {
        sp -= 1;
        let n = &scene.nodes[stack[sp] as usize];
        if !box_hit(n, o, inv, h.t) {
            continue;
        }
        if n.b > 0 {
            for k in n.a..n.a + n.b {
                if scene.tri_mat[k as usize] == PROTOTYPE {
                    prototype_hit(scene, k as usize, o, d, tmax, &mut h);
                } else {
                    tri_hit(scene, k as usize, o, d, tmax, &mut h, NONE);
                }
            }
        } else if sp < 62 {
            stack[sp] = n.a;
            stack[sp + 1] = n.a - 1;
            sp += 2;
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcg_and_rnd_follow_the_shader_formula() {
        // pcg outputs computed by hand from the WGSL formula; rnd() is the top 24 bits of the next state over 2^24
        for (seed, first, second) in
            [(0u32, 129708002u32, 817759070u32), (1, 2831084092, 3911864714), (12345, 4099845390, 1433193492)]
        {
            assert_eq!(pcg(seed), first);
            assert_eq!(pcg(first), second);
            let mut r = Rng(first);
            assert_eq!(r.rnd(), (second >> 8) as f32 / 16777216.0);
            assert_eq!(r.0, second);
        }
    }

    fn one_triangle(p: [[f32; 3]; 3]) -> PtScene {
        let mut s = PtScene::default();
        for c in p {
            s.pos.push([c[0], c[1], c[2], 0.0]);
            s.nrm.push([0.0; 4]);
        }
        s.tri_mat.push(0);
        s
    }

    #[test]
    fn tri_hit_hits_misses_and_respects_t_bounds() {
        let s = one_triangle([[0.0, 0.0, 5.0], [4.0, 0.0, 5.0], [0.0, 4.0, 5.0]]);
        let fresh = |tmax| Hit { t: tmax, u: 0.0, v: 0.0, tri: u32::MAX, instance: u32::MAX };
        let d = [0.0, 0.0, 1.0];
        let mut h = fresh(100.0);
        tri_hit(&s, 0, [1.0, 1.0, 0.0], d, 100.0, &mut h, u32::MAX);
        assert_eq!((h.t, h.u, h.v, h.tri), (5.0, 0.25, 0.25, 0));
        // beyond the triangle's edge, behind the origin (no culling: a ray from the other side hits)
        let mut h = fresh(100.0);
        tri_hit(&s, 0, [3.0, 3.0, 0.0], d, 100.0, &mut h, u32::MAX);
        assert_eq!(h.tri, u32::MAX);
        let mut h = fresh(100.0);
        tri_hit(&s, 0, [1.0, 1.0, 10.0], [0.0, 0.0, -1.0], 100.0, &mut h, u32::MAX);
        assert_eq!((h.t, h.tri), (5.0, 0));
        let mut h = fresh(100.0);
        tri_hit(&s, 0, [1.0, 1.0, 10.0], d, 100.0, &mut h, u32::MAX);
        assert_eq!(h.tri, u32::MAX);
        // t bounds: tmax, the current hit, and the 1e-4 near limit
        let mut h = fresh(100.0);
        tri_hit(&s, 0, [1.0, 1.0, 0.0], d, 4.0, &mut h, u32::MAX);
        assert_eq!(h.tri, u32::MAX);
        let mut h = fresh(3.0);
        tri_hit(&s, 0, [1.0, 1.0, 0.0], d, 100.0, &mut h, u32::MAX);
        assert_eq!(h.tri, u32::MAX);
        let mut h = fresh(100.0);
        tri_hit(&s, 0, [1.0, 1.0, 4.99999], d, 100.0, &mut h, u32::MAX);
        assert_eq!(h.tri, u32::MAX);
        // parallel ray: determinant below 1e-12
        let mut h = fresh(100.0);
        tri_hit(&s, 0, [1.0, 1.0, 0.0], [1.0, 0.0, 0.0], 100.0, &mut h, u32::MAX);
        assert_eq!(h.tri, u32::MAX);
    }

    #[test]
    fn trace_equals_a_brute_force_loop_over_random_triangles() {
        let mut rng = Rng(7);
        let mut r = move |lo: f32, hi: f32| lo + (hi - lo) * rng.rnd();
        let mut tris = Vec::new();
        for _ in 0..300 {
            let c = glam::Vec3::new(r(-10.0, 10.0), r(-10.0, 10.0), r(-10.0, 10.0));
            tris.push([0, 1, 2].map(|_| c + glam::Vec3::new(r(-1.5, 1.5), r(-1.5, 1.5), r(-1.5, 1.5))));
        }
        let mut s = PtScene::default();
        let order = super::super::bvh(&tris, &mut s.nodes);
        for &t in &order {
            for p in tris[t] {
                s.pos.push([p.x, p.y, p.z, 0.0]);
                s.nrm.push([0.0; 4]);
            }
            s.tri_mat.push(0);
        }
        let mut hits = 0;
        for _ in 0..2000 {
            let o = [r(-14.0, 14.0), r(-14.0, 14.0), r(-14.0, 14.0)];
            let d = glam::Vec3::new(r(-1.0, 1.0), r(-1.0, 1.0), r(-1.0, 1.0)).normalize().to_array();
            let tmax = 1000.0;
            let mut expected = Hit { t: tmax, u: 0.0, v: 0.0, tri: u32::MAX, instance: u32::MAX };
            for k in 0..s.tri_mat.len() {
                tri_hit(&s, k, o, d, tmax, &mut expected, u32::MAX);
            }
            let got = trace(&s, o, d, tmax);
            hits += (got.tri != u32::MAX) as u32;
            assert_eq!(got, expected);
        }
        assert!(hits > 200, "the rays should hit often enough to mean something: {hits}");
    }
}
