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

/// TEMPORARY N2 timing probe.
pub(crate) fn probe(scene: &PtScene, ivp: [[f32; 4]; 4], w: usize, h: usize) {
    use rayon::prelude::*;
    use std::time::Instant;
    let un = |x: f32, y: f32, z: f32| {
        let mut r = [0.0f32; 4];
        for (i, o) in r.iter_mut().enumerate() {
            *o = ivp[0][i] * x + ivp[1][i] * y + ivp[2][i] * z + ivp[3][i];
        }
        [r[0] / r[3], r[1] / r[3], r[2] / r[3]]
    };
    let n = w * h;
    let t0 = Instant::now();
    let prim: Vec<([f32; 3], [f32; 3], u32)> = (0..n)
        .into_par_iter()
        .map(|i| {
            let mut rng = Rng(pcg(i as u32 + 1));
            let nx = ((i % w) as f32 + rng.rnd()) / w as f32 * 2.0 - 1.0;
            let ny = 1.0 - ((i / w) as f32 + rng.rnd()) / h as f32 * 2.0;
            let a = un(nx, ny, 1.0);
            let b = un(nx, ny, 0.0);
            let d = sub(a, b);
            let l = dot(d, d).sqrt();
            (b, [d[0] / l, d[1] / l, d[2] / l], rng.0)
        })
        .collect();
    let t_gen = t0.elapsed().as_secs_f64();
    let t1 = Instant::now();
    let hits: Vec<Hit> = prim.par_iter().map(|(o, d, _)| trace(scene, *o, *d, 1e30)).collect();
    let t_prim = t1.elapsed().as_secs_f64();
    let nh = hits.iter().filter(|h| h.tri != NONE).count();
    let t2 = Instant::now();
    let res: Vec<(u8, u8)> = prim
        .par_iter()
        .zip(hits.par_iter())
        .map(|((o, d, s), hit)| {
            if hit.tri == NONE {
                return (0, 0);
            }
            let p = [o[0] + d[0] * hit.t, o[1] + d[1] * hit.t, o[2] + d[2] * hit.t];
            let k = hit.tri as usize;
            let mut nrm = if hit.instance == NONE {
                let p0 = xyz(&scene.pos[k * 3]);
                cross(sub(xyz(&scene.pos[k * 3 + 1]), p0), sub(xyz(&scene.pos[k * 3 + 2]), p0))
            } else {
                [0.0, 1.0, 0.0]
            };
            let l = dot(nrm, nrm).sqrt().max(1e-20);
            nrm = [nrm[0] / l, nrm[1] / l, nrm[2] / l];
            if dot(nrm, *d) > 0.0 {
                nrm = [-nrm[0], -nrm[1], -nrm[2]];
            }
            let o2 = [p[0] + nrm[0] * 1e-3, p[1] + nrm[1] * 1e-3, p[2] + nrm[2] * 1e-3];
            let mut rng = Rng(*s);
            let (r1, r2) = (rng.rnd(), rng.rnd());
            let (r, phi) = (r1.sqrt(), 6.2831853 * r2);
            let tan = if nrm[0].abs() > 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
            let mut t = cross(nrm, tan);
            let tl = dot(t, t).sqrt();
            t = [t[0] / tl, t[1] / tl, t[2] / tl];
            let bt = cross(nrm, t);
            let z = (1.0 - r1).max(0.0).sqrt();
            let dir = [0, 1, 2].map(|i| t[i] * r * phi.cos() + bt[i] * r * phi.sin() + nrm[i] * z);
            let h2 = trace(scene, o2, dir, 1e30);
            (1, (h2.tri != NONE) as u8)
        })
        .collect();
    let t_sec = t2.elapsed().as_secs_f64();
    let n_sec = res.iter().filter(|r| r.0 == 1).count();
    let h_sec = res.iter().filter(|r| r.1 == 1).count();
    let t3 = Instant::now();
    let sun = {
        let v = [0.3f32, 0.8, 0.5];
        let l = dot(v, v).sqrt();
        [v[0] / l, v[1] / l, v[2] / l]
    };
    let sh: Vec<u8> = prim
        .par_iter()
        .zip(hits.par_iter())
        .map(|((o, d, _), hit)| {
            if hit.tri == NONE {
                return 0;
            }
            let p = [o[0] + d[0] * hit.t, o[1] + d[1] * hit.t, o[2] + d[2] * hit.t];
            let o2 = [p[0] + sun[0] * 1e-3, p[1] + sun[1] * 1e-3, p[2] + sun[2] * 1e-3];
            (trace(scene, o2, sun, 1e30).tri != NONE) as u8
        })
        .collect();
    let t_sh = t3.elapsed().as_secs_f64();
    let h_sh = sh.iter().filter(|&&x| x == 1).count();
    let total = t_prim + t_sec + t_sh;
    let rays = n + n_sec + nh;
    eprintln!("N2 probe: threads={} tris={} nodes={} instances={}", rayon::current_num_threads(), scene.tri_mat.len(), scene.nodes.len(), scene.instances.len());
    eprintln!("N2 probe: gen {t_gen:.4}s");
    eprintln!("N2 probe: primary rays={n} hits={nh} {t_prim:.4}s ({:.2} Mrays/s)", n as f64 / t_prim / 1e6);
    eprintln!("N2 probe: secondary rays={n_sec} hits={h_sec} {t_sec:.4}s ({:.2} Mrays/s)", n_sec as f64 / t_sec / 1e6);
    eprintln!("N2 probe: shadow rays={nh} hits={h_sh} {t_sh:.4}s ({:.2} Mrays/s)", nh as f64 / t_sh / 1e6);
    eprintln!("N2 probe: total rays={rays} {total:.4}s ({:.2} Mrays/s)", rays as f64 / total / 1e6);
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
