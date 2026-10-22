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

use std::f32::consts::PI; // the shader's 3.14159265358979 rounds to this f32
const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn normalize(a: [f32; 3]) -> [f32; 3] {
    scale(a, 1.0 / dot(a, a).sqrt())
}
/// The shader's `reflect(i, n)`: `i - 2 dot(n, i) n`.
fn reflect(i: [f32; 3], n: [f32; 3]) -> [f32; 3] {
    sub(i, scale(n, 2.0 * dot(n, i)))
}
fn mix(a: f32, b: f32, t: f32) -> f32 {
    a * (1.0 - t) + b * t
}

/// The shader's `d_ggx`.
pub(crate) fn d_ggx(nh: f32, a: f32) -> f32 {
    let a2 = a * a;
    let f = nh * nh * (a2 - 1.0) + 1.0;
    a2 / (PI * f * f + 1e-7)
}

/// The shader's `v_smith`.
pub(crate) fn v_smith(nl: f32, nv: f32, a: f32) -> f32 {
    let a2 = a * a;
    let g_view = nl * (nv * nv * (1.0 - a2) + a2).sqrt();
    let g_light = nv * (nl * nl * (1.0 - a2) + a2).sqrt();
    0.5 / (g_view + g_light).max(1e-7)
}

/// Schlick's Fresnel term, as written in `bsdf` and `p_spec`: `f0 + (1 - f0) (1 - c)^5`.
pub(crate) fn fresnel(f0: [f32; 3], c: f32) -> [f32; 3] {
    let k = (1.0 - c).powf(5.0);
    [f0[0] + (1.0 - f0[0]) * k, f0[1] + (1.0 - f0[1]) * k, f0[2] + (1.0 - f0[2]) * k]
}

/// The shader's `Surf` (opaque surfaces: the transmission weight stays, for the `kd` factor).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Surf {
    pub albedo: [f32; 3],
    pub metallic: f32,
    pub a: f32,
    pub f0: [f32; 3],
    pub specw: f32,
    pub trans: f32,
}

/// The shader's `surf_of`, from the material's `base`, `params` and `extra` vectors.
pub(crate) fn surf_of(base: [f32; 4], params: [f32; 4], extra: [f32; 4]) -> Surf {
    let albedo = [base[0], base[1], base[2]];
    let metallic = params[0];
    let ior = params[3];
    let r = (ior - 1.0) / (ior + 1.0);
    Surf {
        albedo,
        metallic,
        a: (params[1] * params[1]).max(1e-3),
        f0: [mix(r * r, albedo[0], metallic), mix(r * r, albedo[1], metallic), mix(r * r, albedo[2], metallic)],
        specw: mix(extra[0].clamp(0.0, 1.0), 1.0, metallic),
        trans: params[2],
    }
}

/// The shader's `bsdf`.
pub(crate) fn bsdf(s: &Surf, n: [f32; 3], v: [f32; 3], l: [f32; 3], lobes: [f32; 2]) -> [f32; 3] {
    let nl = dot(n, l);
    let nv = dot(n, v);
    if nl <= 0.0 || nv <= 0.0 {
        return [0.0; 3];
    }
    let h = normalize(add(l, v));
    let nh = dot(n, h).max(0.0);
    let vh = dot(v, h).clamp(0.0, 1.0);
    let f = fresnel(s.f0, vh);
    let dv = d_ggx(nh, s.a) * v_smith(nl, nv, s.a) * s.specw;
    let kd = (1.0 - s.metallic) * (1.0 - s.trans);
    let mut out = [0.0; 3];
    for c in 0..3 {
        let spec = f[c] * dv;
        out[c] = (1.0 - f[c]) * kd * s.albedo[c] / PI * lobes[0] + spec * lobes[1];
    }
    out
}

/// The shader's `frame_of`: the tangent, bitangent and normal.
pub(crate) fn frame_of(n: [f32; 3]) -> [[f32; 3]; 3] {
    let up = if n[2].abs() > 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 0.0, 1.0] };
    let t = normalize(cross(up, n));
    [t, cross(n, t), n]
}

fn frame_apply(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    add(add(scale(m[0], v[0]), scale(m[1], v[1])), scale(m[2], v[2]))
}

/// The shader's `p_spec`.
pub(crate) fn p_spec(s: &Surf, n: [f32; 3], v: [f32; 3]) -> f32 {
    let f = fresnel(s.f0, dot(n, v).clamp(0.0, 1.0));
    let ls = dot(f, LUMA) * s.specw;
    let one_minus = [(1.0 - f[0]) * s.albedo[0], (1.0 - f[1]) * s.albedo[1], (1.0 - f[2]) * s.albedo[2]];
    let ld = dot(one_minus, LUMA) * (1.0 - s.metallic) * (1.0 - s.trans);
    if ld <= 0.0 {
        return 1.0;
    }
    (ls / (ls + ld).max(1e-6)).clamp(0.05, 0.999)
}

/// The shader's `g1_smith`.
pub(crate) fn g1_smith(nv: f32, a: f32) -> f32 {
    let a2 = a * a;
    2.0 * nv / (nv + (a2 + (1.0 - a2) * nv * nv).sqrt())
}

/// The shader's `sample_visible_normal`.
pub(crate) fn sample_visible_normal(v: [f32; 3], a: f32, u1: f32, u2: f32) -> [f32; 3] {
    let vh = normalize([a * v[0], a * v[1], v[2]]);
    let lensq = vh[0] * vh[0] + vh[1] * vh[1];
    let t1 = if lensq > 0.0 { scale([-vh[1], vh[0], 0.0], 1.0 / lensq.sqrt()) } else { [1.0, 0.0, 0.0] };
    let t2 = cross(vh, t1);
    let r = u1.sqrt();
    let phi = 2.0 * PI * u2;
    let p1 = r * phi.cos();
    let mut p2 = r * phi.sin();
    let k = 0.5 * (1.0 + vh[2]);
    p2 = (1.0 - k) * (1.0 - p1 * p1).max(0.0).sqrt() + k * p2;
    let w = (1.0 - p1 * p1 - p2 * p2).max(0.0).sqrt();
    let nh = add(add(scale(t1, p1), scale(t2, p2)), scale(vh, w));
    normalize([a * nh[0], a * nh[1], nh[2].max(0.0)])
}

/// The shader's `pdf_of`.
pub(crate) fn pdf_of(s: &Surf, n: [f32; 3], v: [f32; 3], l: [f32; 3]) -> f32 {
    let nl = dot(n, l);
    if nl <= 0.0 {
        return 0.0;
    }
    let h = normalize(add(l, v));
    let nh = dot(n, h).max(0.0);
    let ps = p_spec(s, n, v);
    ps * g1_smith(dot(n, v), s.a) * d_ggx(nh, s.a) / (4.0 * dot(n, v).max(1e-4)) + (1.0 - ps) * nl / PI
}

/// The shader's `sample_dir`: three `rnd()` calls in the order `u1`, `u2`, lobe choice.
pub(crate) fn sample_dir(s: &Surf, n: [f32; 3], v: [f32; 3], rng: &mut Rng) -> [f32; 3] {
    let tbn = frame_of(n);
    let u1 = rng.rnd();
    let u2 = rng.rnd();
    if rng.rnd() < p_spec(s, n, v) {
        let vl = [dot(v, tbn[0]), dot(v, tbn[1]), dot(v, n).max(1e-4)];
        let h = frame_apply(&tbn, sample_visible_normal(vl, s.a, u1, u2));
        return reflect(scale(v, -1.0), h);
    }
    let r = u1.sqrt();
    let ph = 2.0 * PI * u2;
    frame_apply(&tbn, [r * ph.cos(), r * ph.sin(), (1.0 - u1).max(0.0).sqrt()])
}

#[cfg(test)]
mod bsdf_tests {
    use super::*;

    fn close(a: f32, b: f32, rel: f32) -> bool {
        (a - b).abs() <= rel * b.abs().max(1e-6)
    }

    fn dielectric(rough: f32, metallic: f32) -> Surf {
        surf_of([0.6, 0.4, 0.2, 1.0], [metallic, rough, 0.0, 1.5], [1.0, 0.0, 0.0, 0.0])
    }

    #[test]
    fn d_v_and_f_match_hand_computed_values() {
        // a = 1: f = 1 whatever nh, so D = 1 / (pi + 1e-7)
        assert!(close(d_ggx(0.5, 1.0), 1.0 / PI, 1e-5));
        // a = 0.5, nh = 1: f = 0.25, D = 0.25 / (pi / 16)
        assert!(close(d_ggx(1.0, 0.5), 4.0 / PI, 1e-5));
        // a = 0.5, nl = nv = 1: both G terms are 1, V = 0.5 / 2
        assert!(close(v_smith(1.0, 1.0, 0.5), 0.25, 1e-6));
        // a = 1, nl = nv = 0.5: both G terms are 0.5, V = 0.5 / 1
        assert!(close(v_smith(0.5, 0.5, 1.0), 0.5, 1e-6));
        let f0 = [0.04, 0.5, 1.0];
        assert_eq!(fresnel(f0, 1.0), f0);
        assert_eq!(fresnel(f0, 0.0), [1.0; 3]);
        let f = fresnel(f0, 0.5);
        for c in 0..3 {
            assert!(close(f[c], f0[c] + (1.0 - f0[c]) / 32.0, 1e-6));
        }
    }

    #[test]
    fn surf_of_derives_alpha_f0_and_specular_weight() {
        let s = surf_of([0.8, 0.2, 0.1, 1.0], [1.0, 0.0, 0.0, 1.5], [0.3, 0.0, 0.0, 0.0]);
        assert_eq!(s.a, 1e-3);
        assert_eq!(s.f0, [0.8, 0.2, 0.1]);
        assert_eq!(s.specw, 1.0);
        let d = surf_of([0.8, 0.2, 0.1, 1.0], [0.0, 0.5, 0.0, 1.5], [0.3, 0.0, 0.0, 0.0]);
        assert_eq!(d.a, 0.25);
        assert!(close(d.f0[0], 0.04, 1e-5));
        assert_eq!(d.specw, 0.3);
    }

    #[test]
    fn sample_dir_is_unit_length_and_diffuse_stays_in_the_upper_hemisphere() {
        let n = normalize([0.3, -0.5, 0.8]);
        let v = normalize([0.6, 0.2, 0.7]);
        let mut rng = Rng(12345);
        let diffuse = surf_of([0.8, 0.8, 0.8, 1.0], [0.0, 1.0, 0.0, 1.0], [0.0; 4]); // ior 1: f0 = 0, specw 0
        let rough = dielectric(0.6, 0.0);
        let mut below = 0;
        for _ in 0..2000 {
            let d = sample_dir(&diffuse, n, v, &mut rng);
            assert!(close(dot(d, d), 1.0, 1e-4));
            below += (dot(d, n) < -1e-5) as u32;
            let g = sample_dir(&rough, n, v, &mut rng);
            assert!(close(dot(g, g), 1.0, 1e-4));
        }
        // the specular share (p_spec is at least 0.05) is the only way below the horizon
        assert!(below <= 2000 * 6 / 100, "{below} of 2000 diffuse draws below the surface");
    }

    #[test]
    fn mean_of_eval_cos_over_pdf_matches_the_integrated_albedo() {
        let n = [0.0, 0.0, 1.0];
        let v = normalize([0.5, 0.2, 0.8]);
        for (rough, metallic) in [(0.5, 0.0), (0.8, 0.3)] {
            let s = dielectric(rough, metallic);
            let (nt, np) = (400, 800);
            let mut reference = 0.0f64;
            for i in 0..nt {
                let theta = (i as f32 + 0.5) / nt as f32 * (PI / 2.0);
                for j in 0..np {
                    let phi = (j as f32 + 0.5) / np as f32 * 2.0 * PI;
                    let l = [theta.sin() * phi.cos(), theta.sin() * phi.sin(), theta.cos()];
                    let w = (theta.sin() * (PI / 2.0 / nt as f32) * (2.0 * PI / np as f32)) as f64;
                    reference += (bsdf(&s, n, v, l, [1.0, 1.0])[0] * l[2]) as f64 * w;
                }
            }
            let mut rng = Rng(777);
            let mut sum = 0.0f64;
            let count = 20_000;
            for _ in 0..count {
                let l = sample_dir(&s, n, v, &mut rng);
                let pdf = pdf_of(&s, n, v, l);
                if pdf > 0.0 && l[2] > 0.0 {
                    sum += (bsdf(&s, n, v, l, [1.0, 1.0])[0] * l[2] / pdf) as f64;
                }
            }
            let mean = sum / count as f64;
            assert!(((mean - reference) / reference).abs() < 0.02, "rough {rough}: mean {mean}, albedo {reference}");
        }
    }

    #[test]
    fn pdf_of_follows_the_mixture_density_and_integrates_to_one() {
        let n = normalize([0.1, 0.2, 0.9]);
        let v = normalize([0.4, -0.3, 0.8]);
        let s = dielectric(0.7, 0.0);
        let ps = p_spec(&s, n, v);
        let mut rng = Rng(99);
        for _ in 0..200 {
            let l = sample_dir(&s, n, v, &mut rng);
            let h = normalize(add(l, v));
            let spec = g1_smith(dot(n, v), s.a) * d_ggx(dot(n, h).max(0.0), s.a) / (4.0 * dot(n, v));
            let want = if dot(n, l) > 0.0 { ps * spec + (1.0 - ps) * dot(n, l) / PI } else { 0.0 };
            assert!(close(pdf_of(&s, n, v, l), want, 1e-4));
        }
        // the density over the sphere: 1 less the share of the visible normals that reflect below the horizon
        let tbn = frame_of(n);
        let (nt, np) = (400, 800);
        let mut total = 0.0f64;
        for i in 0..nt {
            let theta = (i as f32 + 0.5) / nt as f32 * (PI / 2.0);
            for j in 0..np {
                let phi = (j as f32 + 0.5) / np as f32 * 2.0 * PI;
                let l = frame_apply(&tbn, [theta.sin() * phi.cos(), theta.sin() * phi.sin(), theta.cos()]);
                total += (pdf_of(&s, n, v, l) * theta.sin() * (PI / 2.0 / nt as f32) * (2.0 * PI / np as f32)) as f64;
            }
        }
        assert!((0.9..1.02).contains(&total), "pdf integral {total}");
    }
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
