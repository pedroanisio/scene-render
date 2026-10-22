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

/// The dome as the shader reads it: mip 0 of the prefiltered chain, f16-rounded, sampled bilinearly with repeat
/// addressing; `rot` is `env_rot` in column-major order and `intensity` is `env.y`.
pub(crate) struct EnvMap {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<[f32; 4]>,
    pub rot: [[f32; 4]; 4],
    pub intensity: f32,
}

impl EnvMap {
    /// Rounds rgb texels to f16 as the rgba16float upload does (alpha 1).
    pub(crate) fn from_rgb(w: u32, h: u32, rgb: &[[f32; 3]], rot: [[f32; 4]; 4], intensity: f32) -> EnvMap {
        let f = |x: f32| half::f16::from_f32(x).to_f32();
        let rgba = rgb.iter().map(|p| [f(p[0]), f(p[1]), f(p[2]), 1.0]).collect();
        EnvMap { w, h, rgba, rot, intensity }
    }

    fn texel(&self, x: i64, y: i64) -> [f32; 4] {
        let xi = x.rem_euclid(self.w as i64) as usize;
        let yi = y.rem_euclid(self.h as i64) as usize;
        self.rgba[yi * self.w as usize + xi]
    }

    /// The shader's `env_radiance` environment term (no ambient lights) for world direction `d`.
    pub(crate) fn radiance(&self, d: [f32; 3]) -> [f32; 3] {
        let m = &self.rot;
        let rotated = [
            m[0][0] * d[0] + m[1][0] * d[1] + m[2][0] * d[2],
            m[0][1] * d[0] + m[1][1] * d[1] + m[2][1] * d[2],
            m[0][2] * d[0] + m[1][2] * d[1] + m[2][2] * d[2],
        ];
        let r = normalize(rotated);
        let u = (r[0].atan2(r[2]) + PI) / (2.0 * PI);
        let v = (-r[1]).clamp(-1.0, 1.0).acos() / PI;
        let x = u * self.w as f32 - 0.5;
        let y = v * self.h as f32 - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let top = mix4(self.texel(x0, y0), self.texel(x0 + 1, y0), fx);
        let bottom = mix4(self.texel(x0, y0 + 1), self.texel(x0 + 1, y0 + 1), fx);
        let c = mix4(top, bottom, fy);
        [c[0] * self.intensity, c[1] * self.intensity, c[2] * self.intensity]
    }
}

/// Next-event sample of a directional light (the sun) as `light_sample` does: the shader has no angular radius, so
/// the direction is exactly `-dir` and the distance 1e30; the radiance is the light colour (`light_radiance`, no IES).
pub(crate) fn sun_sample(dir: [f32; 3]) -> ([f32; 3], f32) {
    ([-dir[0], -dir[1], -dir[2]], 1e30)
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
    if scene.nodes.is_empty() {
        return h;
    }
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

    const IDENT: [[f32; 4]; 4] = [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.], [0., 0., 0., 1.]];

    #[test]
    fn env_lookup_matches_hand_bilinear_values() {
        // texel (x, y) = x*x + 10*y on a 4x2 map
        let rgb: Vec<[f32; 3]> = (0..8).map(|k| [((k % 4) * (k % 4) + 10 * (k / 4)) as f32, 0.0, 0.0]).collect();
        let env = EnvMap::from_rgb(4, 2, &rgb, IDENT, 1.0);
        let cases = [
            ([1., 0., 0.], 11.5),
            ([-1., 0., 0.], 5.5),
            ([0., 1., 0.], 7.5),
            ([0., -1., 0.], 7.5),
            ([0., 0., 1.], 7.5),
            ([0., 0., -1.], 9.5),
        ];
        for (d, want) in cases {
            let got = env.radiance(d)[0];
            assert!((got - want).abs() < 1e-3, "{d:?}: {got} vs {want}");
        }
    }

    #[test]
    fn constant_env_is_constant_and_f16_rounded() {
        let env = EnvMap::from_rgb(4, 2, &[[0.1, 0.2, 0.3]; 8], IDENT, 1.0);
        let want = [0.1, 0.2, 0.3].map(|x| half::f16::from_f32(x).to_f32());
        assert_ne!(want[0], 0.1);
        let mut rng = Rng(7);
        for _ in 0..100 {
            let d = normalize([rng.rnd() - 0.5, rng.rnd() - 0.5, rng.rnd() - 0.5]);
            let got = env.radiance(d);
            for k in 0..3 {
                assert!((got[k] - want[k]).abs() < 1e-6, "{d:?}: {got:?}");
            }
        }
    }

    #[test]
    fn sun_sample_is_the_exact_reversed_direction() {
        // the shader's directional light has no angular radius: no cone to sample
        let (l, dist) = sun_sample([0.0, -0.6, 0.8]);
        assert_eq!((l, dist), ([0.0, 0.6, -0.8], 1e30));
    }

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

/// The shader's `address_index`.
pub(crate) fn address_index(x: i32, size: i32, mode: u32) -> i32 {
    if mode == 1 {
        return x.clamp(0, size - 1);
    }
    if mode == 2 {
        let v = ((x % (size * 2)) + size * 2) % (size * 2);
        return if v < size { v } else { size * 2 - 1 - v };
    }
    if mode == 3 && (x < 0 || x >= size) {
        return -1;
    }
    ((x % size) + size) % size
}

fn address_coord(v: f32, mode: u32) -> f32 {
    match mode {
        1 => v.clamp(0.0, 1.0),
        2 => v - (v / 2.0).floor() * 2.0,
        3 => v.clamp(-1.0, 2.0),
        _ => v - v.floor(),
    }
}

fn sample_position(uv: [f32; 2], size: [f32; 2], flags: u32) -> [f32; 2] {
    [address_coord(uv[0], (flags >> 3) & 3) * size[0] - 0.5, address_coord(uv[1], (flags >> 5) & 3) * size[1] - 0.5]
}

fn cubic_weights(t: f32) -> [f32; 4] {
    let t2 = t * t;
    let t3 = t2 * t;
    [-0.5 * t + t2 - 0.5 * t3, 1.0 - 2.5 * t2 + 1.5 * t3, 0.5 * t + 2.0 * t2 - 1.5 * t3, -0.5 * t2 + 0.5 * t3]
}

fn srgb_channel(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// The shader's `texel`.
fn texel(pixels: &[u32], info: [u32; 4], x: i32, y: i32, border: [f32; 4]) -> [f32; 4] {
    let qx = address_index(x, info[1] as i32, (info[3] >> 3) & 3);
    let qy = address_index(y, info[2] as i32, (info[3] >> 5) & 3);
    if qx < 0 || qy < 0 {
        return border;
    }
    let index = info[0] as usize + qy as usize * info[1] as usize + qx as usize;
    let c = pixels[index].to_le_bytes().map(|b| b as f32 / 255.0);
    if info[3] & 1 == 0 {
        return c;
    }
    [srgb_channel(c[0]), srgb_channel(c[1]), srgb_channel(c[2]), c[3]]
}

fn mix4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [0, 1, 2, 3].map(|i| a[i] * (1.0 - t) + b[i] * t)
}

/// The shader's `map_sample`: `info` is (offset, width, height, flags) into `pixels`.
pub(crate) fn map_sample(pixels: &[u32], info: [u32; 4], uv: [f32; 2], border: [f32; 4]) -> [f32; 4] {
    if info[1] == 0 || info[2] == 0 {
        return [1.0; 4];
    }
    let p = sample_position(uv, [info[1] as f32, info[2] as f32], info[3]);
    let (qx, qy) = (p[0].floor() as i32, p[1].floor() as i32);
    let f = [p[0] - p[0].floor(), p[1] - p[1].floor()];
    let at = |x: i32, y: i32| texel(pixels, info, x, y, border);
    match (info[3] >> 1) & 3 {
        1 => at((p[0] + 0.5).floor() as i32, (p[1] + 0.5).floor() as i32),
        2 => {
            let (wx, wy) = (cubic_weights(f[0]), cubic_weights(f[1]));
            let mut out = [0.0f32; 4];
            for (y, wy) in wy.into_iter().enumerate() {
                for (x, wx) in wx.into_iter().enumerate() {
                    let t = at(qx + x as i32 - 1, qy + y as i32 - 1);
                    for (o, c) in out.iter_mut().zip(t) {
                        *o += c * wx * wy;
                    }
                }
            }
            out
        }
        _ => mix4(mix4(at(qx, qy), at(qx + 1, qy), f[0]), mix4(at(qx, qy + 1), at(qx + 1, qy + 1), f[0]), f[1]),
    }
}

/// A hit's material and shading normals, as `radiance` assembles them.
#[allow(dead_code)] // the plug-in pass reads every field; the tests read only some
#[derive(Clone, Copy, Debug)]
pub(crate) struct Shaded {
    pub base: [f32; 4],
    pub params: [f32; 4],
    pub emissive: [f32; 4],
    pub extra: [f32; 4],
    pub n: [f32; 3],
    pub ng: [f32; 3],
    /// `1 + strength * (occlusion map red - 1)`, which the shader multiplies into the throughput.
    pub occlusion: f32,
}

/// The shader's `radiance` material assembly for a hit (`hit_material`, `hit_position`, `hit_normal`), without the
/// normal-map branch.
pub(crate) fn shade_hit(scene: &PtScene, hit: &Hit) -> Shaded {
    let k = hit.tri as usize;
    let record = (hit.instance != NONE).then(|| &scene.instances[&(hit.instance as usize)]);
    let mat = match record {
        Some(r) => r[13][1].to_bits(),
        None => scene.tri_mat[k],
    } as usize;
    let mut m = scene.mats[mat];
    let bary = [1.0 - hit.u - hit.v, hit.u, hit.v];
    let uv = |slot: usize| {
        let c = |i: usize| scene.uv[k * 3 + i][slot];
        [0, 1].map(|a| c(0)[a] * bary[0] + c(1)[a] * bary[1] + c(2)[a] * bary[2])
    };
    let base_texel = map_sample(&scene.pixels, m.maps[0], uv(0), m.borders[0]);
    let color = [0, 1, 2, 3].map(|a| {
        scene.colors[k * 3][a] * bary[0] + scene.colors[k * 3 + 1][a] * bary[1] + scene.colors[k * 3 + 2][a] * bary[2]
    });
    m.base = [0, 1, 2, 3].map(|a| m.base[a] * base_texel[a] * color[a]);
    let mr = map_sample(&scene.pixels, m.maps[2], uv(2), m.borders[2]);
    m.params[0] *= mr[2];
    m.params[1] *= mr[1];
    let e = map_sample(&scene.pixels, m.maps[4], uv(4), m.borders[4]);
    m.emissive = [m.emissive[0] * e[0], m.emissive[1] * e[1], m.emissive[2] * e[2], m.emissive[3]];
    if m.emissive[3] > 0.5 {
        m.emissive = [m.base[0], m.base[1], m.base[2], m.emissive[3]];
    }
    if m.texture_params[1] == 0.0 {
        m.base[3] = scene.mats[mat].base[3];
    }
    if m.texture_params[1] == 1.0 {
        m.base[3] = if m.base[3] >= m.texture_params[0] { 1.0 } else { 0.0 };
    }
    let position = |c: usize| {
        let p = xyz(&scene.pos[k * 3 + c]);
        match record {
            Some(r) => transform(&r[1..5], p, 1.0),
            None => p,
        }
    };
    let normal = |c: usize| {
        let n = xyz(&scene.nrm[k * 3 + c]);
        match record {
            Some(r) => {
                let t = transform(&r[9..13], n, 0.0);
                if dot(t, t) > 0.0 {
                    normalize(t)
                } else {
                    [0.0; 3]
                }
            }
            None => n,
        }
    };
    let p0 = position(0);
    let ng = normalize(cross(sub(position(1), p0), sub(position(2), p0)));
    let (n0, n1, n2) = (normal(0), normal(1), normal(2));
    let bw = 1.0 - hit.u - hit.v;
    let n = [0, 1, 2].map(|a| n0[a] * bw + n1[a] * hit.u + n2[a] * hit.v);
    let n = if dot(n, n) < 1e-8 { ng } else { normalize(n) };
    let ao = map_sample(&scene.pixels, m.maps[3], uv(3), m.borders[3]);
    Shaded {
        base: m.base,
        params: m.params,
        emissive: m.emissive,
        extra: m.extra,
        n,
        ng,
        occlusion: 1.0 + m.texture_params[3] * (ao[0] - 1.0),
    }
}

#[cfg(test)]
mod texture_tests {
    use super::*;
    use crate::pathtrace::PtMat;

    fn pack(texels: &[[u8; 4]]) -> Vec<u32> {
        texels.iter().map(|t| u32::from_le_bytes(*t)).collect()
    }
    fn gray(v: u8) -> [u8; 4] {
        [v, v, v, 255]
    }
    /// 2x2 red channel: (0,0)=0, (1,0)=1, (0,1)=1, (1,1)=0.
    fn checker() -> Vec<u32> {
        pack(&[gray(0), gray(255), gray(255), gray(0)])
    }
    const BORDER: [f32; 4] = [0.0; 4];

    #[test]
    fn nearest_picks_the_rounded_texel_in_each_address_mode() {
        let px = checker();
        let nearest = 2;
        // wrap: fract(1.75) = 0.75 -> p = 1.0 -> texel (1, 0)
        assert_eq!(map_sample(&px, [0, 2, 2, nearest], [1.75, 0.25], BORDER)[0], 1.0);
        // clamp: x past 1 -> p = 1.5 -> floor(2.0) = 2 -> clamped to texel 1
        assert_eq!(map_sample(&px, [0, 2, 2, nearest | (1 << 3)], [1.75, 0.25], BORDER)[0], 1.0);
        // clamp below 0 -> texel (0, 0)
        assert_eq!(map_sample(&px, [0, 2, 2, nearest | (1 << 3)], [-0.5, 0.25], BORDER)[0], 0.0);
        // mirror: 1.25 -> p = 2.0 -> floor(2.5) = 2 -> mirrored to texel 1
        assert_eq!(map_sample(&px, [0, 2, 2, nearest | (2 << 3)], [1.25, 0.25], BORDER)[0], 1.0);
        // border mode: texel -1 gives the border colour
        let border = [0.25, 0.5, 0.75, 1.0];
        assert_eq!(map_sample(&px, [0, 2, 2, nearest | (3 << 3)], [-0.1, 0.25], border), border);
    }

    #[test]
    fn address_index_modes() {
        assert_eq!(address_index(-1, 4, 0), 3);
        assert_eq!(address_index(5, 4, 0), 1);
        assert_eq!(address_index(-3, 4, 1), 0);
        assert_eq!(address_index(9, 4, 1), 3);
        assert_eq!(address_index(4, 4, 2), 3);
        assert_eq!(address_index(-1, 4, 2), 0);
        assert_eq!(address_index(8, 4, 2), 0);
        assert_eq!(address_index(-1, 4, 3), -1);
        assert_eq!(address_index(4, 4, 3), -1);
        assert_eq!(address_index(2, 4, 3), 2);
    }

    #[test]
    fn bilinear_mixes_the_four_texels() {
        let px = checker();
        let c = map_sample(&px, [0, 2, 2, 0], [0.5, 0.5], BORDER);
        assert_eq!(c, [0.5, 0.5, 0.5, 1.0]);
        // uv (0.25, 0.5) -> p = (0, 0.5): x weight 0 reads column 0 only, mix(0, 1, 0.5) = 0.5
        assert_eq!(map_sample(&px, [0, 2, 2, 0], [0.25, 0.5], BORDER)[0], 0.5);
    }

    #[test]
    fn bicubic_reproduces_a_linear_ramp_and_a_constant() {
        let ramp: Vec<[u8; 4]> = (0..16).map(|i| gray(85 * (i % 4) as u8)).collect();
        let c = map_sample(&pack(&ramp), [0, 4, 4, 4], [0.5, 0.5], BORDER);
        assert!((c[0] - 0.5).abs() < 1e-6, "{c:?}");
        let ones = pack(&[gray(255); 16]);
        let c = map_sample(&ones, [0, 4, 4, 4], [0.37, 0.61], BORDER);
        assert!((c[1] - 1.0).abs() < 1e-6, "{c:?}");
    }

    #[test]
    fn srgb_is_decoded_before_the_bilinear_mix() {
        let px = pack(&[gray(0), gray(255), gray(255), gray(0)]);
        let c = map_sample(&px, [0, 2, 2, 1], [0.5, 0.5], BORDER);
        // the mean of the linear values 0 and 1, not the linear value of the mean code (about 0.214)
        assert_eq!(c[..3], [0.5; 3]);
        assert_eq!(c[3], 1.0);
        let mid = map_sample(&pack(&[gray(128); 4]), [0, 2, 2, 1], [0.5, 0.5], BORDER);
        let expected = ((128.0f32 / 255.0 + 0.055) / 1.055).powf(2.4);
        assert!((mid[0] - expected).abs() < 1e-6, "{mid:?}");
    }

    #[test]
    fn base_colour_is_material_times_texel_times_vertex_colour() {
        let mut s = PtScene {
            pixels: pack(&[[51, 102, 153, 255], [204, 51, 102, 255], [255, 255, 0, 255], [0, 0, 0, 255]]),
            ..Default::default()
        };
        let corners = [[0.25, 0.25], [0.75, 0.25], [0.25, 0.75]];
        for (c, p) in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]].iter().enumerate() {
            s.pos.push([p[0], p[1], p[2], 0.0]);
            s.nrm.push([0.0, 0.0, 1.0, 0.0]);
            let mut uv = [[0.0; 2]; 6];
            uv[0] = corners[c];
            s.uv.push(uv);
        }
        s.colors = vec![[1.0, 0.5, 0.25, 1.0], [0.5, 1.0, 0.5, 1.0], [0.25, 0.5, 1.0, 1.0]];
        s.tri_mat = vec![0];
        let mut m: PtMat = bytemuck::Zeroable::zeroed();
        m.base = [0.8, 0.6, 0.4, 0.5];
        m.params = [1.0, 0.5, 0.0, 1.5];
        m.maps[0] = [0, 2, 2, 2];
        s.mats = vec![m];
        let texel = |i: usize| s.pixels[i].to_le_bytes().map(|b| b as f32 / 255.0);
        for (c, (u, v)) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)].into_iter().enumerate() {
            let hit = Hit { t: 1.0, u, v, tri: 0, instance: u32::MAX };
            let got = shade_hit(&s, &hit);
            let (t, col) = (texel(c), s.colors[c]);
            for k in 0..3 {
                assert_eq!(got.base[k], m.base[k] * t[k] * col[k], "corner {c} channel {k}");
            }
            assert_eq!(got.base[3], 0.5, "opaque-mode alpha stays the material's");
            assert_eq!(got.params[..2], [1.0, 0.5], "unmapped metallic/roughness are unchanged");
            assert_eq!(got.n, [0.0, 0.0, 1.0]);
        }
    }
}

/// The shader's `PLight` fields that `light_sample`, `light_radiance` and `light_lobes` read for a directional
/// light (type 1): `pos.w` the kind, `dir.xyz` the direction of travel, `color.rgb` the radiance, `size.zw` the IES
/// pixel offset and width (`ies_value`), `right.w` the lobe bits. `spot` is read only by other kinds.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PLight {
    pub pos: [f32; 4],
    pub dir: [f32; 4],
    pub color: [f32; 4],
    pub size: [f32; 4],
    pub right: [f32; 4],
}

/// The uniform inputs of `radiance` and `cs_trace`'s per-pixel part (`pathtrace.wgsl:587-851`). Matrices are
/// column-major like `EnvMap::rot`.
#[derive(Clone, Debug)]
pub(crate) struct RadianceParams {
    /// Whole-image size in pixels (`size`), for `global_pix` and the NDC mapping.
    pub size: [u32; 2],
    pub inv_view_proj: [[f32; 4]; 4],
    pub cam_to_world: [[f32; 4]; 4],
    /// Lens distortion `x`, depth-of-field aperture `y`, focus distance `z`, max bounces `w` (read as `u32(cam.w)`).
    pub cam: [f32; 4],
    /// `x` > 1.5 shows the dome to camera rays, `z` > 0.5 selects an orthographic camera.
    pub env: [f32; 4],
    pub lights: Vec<PLight>,
}

fn mat_vec(m: &[[f32; 4]; 4], v: [f32; 4]) -> [f32; 4] {
    [0, 1, 2, 3].map(|i| m[0][i] * v[0] + m[1][i] * v[1] + m[2][i] * v[2] + m[3][i] * v[3])
}

fn mul3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}

/// The shader's `radiance` for the camera ray through `px` (pixel coordinates with the jitter added); alpha in `[3]`.
/// Ports the supported subset (see `supported`): every `rnd()` is drawn where the shader draws it. The guide buffers,
/// media, water, backdrop, splats, subsurface, transmission, depth of field and ambient lights are not ported.
pub(crate) fn radiance(scene: &PtScene, env: &EnvMap, p: &RadianceParams, px: [f32; 2], rng: &mut Rng) -> [f32; 4] {
    let (w, h) = (p.size[0] as f32, p.size[1] as f32);
    let mut ndc = [px[0] / w * 2.0 - 1.0, 1.0 - px[1] / h * 2.0];
    let k = 1.0 + p.cam[0] * (ndc[0] * ndc[0] + ndc[1] * ndc[1]);
    ndc = [ndc[0] * k, ndc[1] * k];
    let a = mat_vec(&p.inv_view_proj, [ndc[0], ndc[1], 1.0, 1.0]);
    let b = mat_vec(&p.inv_view_proj, [ndc[0], ndc[1], 0.0, 1.0]);
    let near = [a[0] / a[3], a[1] / a[3], a[2] / a[3]];
    let far = [b[0] / b[3], b[1] / b[3], b[2] / b[3]];
    let mut o = xyz(&mat_vec(&p.cam_to_world, [0.0, 0.0, 0.0, 1.0]));
    if p.env[2] > 0.5 {
        o = near;
    }
    let mut d = normalize(sub(far, near));
    let mut thr = [1.0f32; 3];
    let mut col = [0.0f32; 3];
    let mut alpha = 0.0f32;
    let mut bounce = 0u32;
    let mut met = 0u32;
    let max_b = p.cam[3] as u32;
    for _ in 0..64 {
        let hit = trace(scene, o, d, 1e30);
        if hit.tri == NONE {
            if met == 0 {
                if p.env[0] > 1.5 {
                    col = add(col, mul3(thr, env_term(env, p, d)));
                    alpha = 1.0;
                }
                break;
            }
            col = add(col, mul3(thr, env_term(env, p, d)));
            break;
        }
        let m = shade_hit(scene, &hit);
        let pt = add(o, scale(d, hit.t));
        if m.base[3] < 1.0 && rng.rnd() > m.base[3] {
            o = add(pt, scale(d, 1e-3));
            continue;
        }
        alpha = 1.0;
        met += 1;
        let ng0 = m.ng;
        let ng = if dot(d, ng0) < 0.0 { ng0 } else { scale(ng0, -1.0) };
        let mut n = m.n;
        if dot(n, ng) < 0.0 {
            n = scale(n, -1.0);
        }
        col = add(col, mul3(thr, xyz(&m.emissive)));
        if m.emissive[3] > 0.5 {
            break;
        }
        let s = surf_of(m.base, m.params, m.extra);
        let v = scale(d, -1.0);
        if bounce >= max_b {
            break;
        }
        for lt in &p.lights {
            let ty = lt.pos[3] as u32;
            if ty == 0 || lt.pos[3] < 0.0 {
                continue;
            }
            let (l, dist) = sun_sample(xyz(&lt.dir));
            let nl = dot(n, l);
            if nl <= 0.0 {
                continue;
            }
            let rad = xyz(&lt.color);
            if rad[0].max(rad[1].max(rad[2])) <= 0.0 {
                continue;
            }
            let mut visible = 1.0;
            if lt.size[1] > 0.5 && m.extra[2] > 0.5 {
                visible = visibility(scene, add(pt, scale(ng, 1e-2)), l, dist - 2e-2);
            }
            let flags = lt.right[3] as u32;
            let lobes = [(flags & 1) as f32, ((flags >> 1) & 1) as f32];
            let f = bsdf(&s, n, v, l, lobes);
            let mut c = [0, 1, 2].map(|i| thr[i] * f[i] * nl * rad[i] * visible);
            if bounce > 0 {
                c = c.map(|x| x.min(20.0));
            }
            col = add(col, c);
        }
        let l = sample_dir(&s, n, v, rng);
        let pdf = pdf_of(&s, n, v, l);
        let nl = dot(n, l);
        if pdf <= 1e-8 || nl <= 0.0 {
            break;
        }
        let diffuse = bsdf(&s, n, v, l, [1.0, 0.0]);
        let specular = bsdf(&s, n, v, l, [0.0, 1.0]);
        for i in 0..3 {
            thr[i] *= (diffuse[i] + specular[i]) * nl / pdf * m.occlusion;
        }
        bounce += 1;
        if bounce > 2 {
            let q = thr[0].max(thr[1].max(thr[2])).min(0.95);
            if rng.rnd() > q {
                break;
            }
            thr = [thr[0] / q, thr[1] / q, thr[2] / q];
        }
        o = add(pt, scale(ng, 1e-3));
        d = l;
    }
    [col[0], col[1], col[2], alpha]
}

/// The dome as `env_radiance` returns it with no ambient lights: black unless `env.x > 0.5`.
fn env_term(env: &EnvMap, p: &RadianceParams, d: [f32; 3]) -> [f32; 3] {
    if p.env[0] > 0.5 {
        env.radiance(d)
    } else {
        [0.0; 3]
    }
}

/// The shader's `visibility`: transmittance of casters along `d` for `dist`.
pub(crate) fn visibility(scene: &PtScene, o: [f32; 3], d: [f32; 3], dist: f32) -> f32 {
    let mut o = o;
    let mut remaining = dist;
    let mut result = 1.0f32;
    for _ in 0..64 {
        let hit = trace(scene, o, d, remaining);
        if hit.tri == NONE {
            return result;
        }
        // `shade_hit`'s alpha is the shader's `opacity`: the material's for mode 0, times the map and vertex alpha
        // otherwise, cut off for mode 1
        let m = shade_hit(scene, &hit);
        if m.extra[1] > 0.5 {
            result *= 1.0 - m.base[3].clamp(0.0, 1.0);
        }
        if result < 1e-5 {
            return 0.0;
        }
        let advance = hit.t + 1e-3;
        o = add(o, scale(d, advance));
        remaining -= advance;
        if remaining <= 0.0 {
            return result;
        }
    }
    0.0
}

/// `cs_trace`'s per-pixel part: samples `start..start + count` of pixel `(x, y)` summed (not divided).
pub(crate) fn pixel_sum(
    scene: &PtScene,
    env: &EnvMap,
    p: &RadianceParams,
    x: u32,
    y: u32,
    start: u32,
    count: u32,
) -> [f32; 4] {
    let global_pix = y.wrapping_mul(p.size[0]).wrapping_add(x);
    let mut sum = [0.0f32; 4];
    for s in 0..count {
        let si = start + s;
        let mut rng = Rng(pcg(global_pix.wrapping_mul(9781).wrapping_add(pcg(si.wrapping_mul(6271).wrapping_add(1)))));
        let jx = rng.rnd();
        let jy = rng.rnd();
        let r = radiance(scene, env, p, [x as f32 + jx, y as f32 + jy], &mut rng);
        for (acc, c) in sum.iter_mut().zip(r) {
            *acc += c;
        }
    }
    sum
}

/// Whether the CPU loop reproduces the shader for this scene: false for any light that is not a plain directional one
/// (other kinds, IES profiles), subsurface or transmissive materials, normal maps, splats and depth of field. Media,
/// water, procedural skies and glass backdrops are not part of `RadianceParams`; the caller rejects them.
pub(crate) fn supported(scene: &PtScene, p: &RadianceParams) -> bool {
    p.cam[1] <= 0.0
        && scene.splats.is_empty()
        && p.lights.iter().all(|l| l.pos[3] == 1.0 && l.size[3] == 0.0)
        && scene.mats.iter().all(|m| m.params[2] <= 0.0 && m.sss[3] <= 0.0 && m.maps[1][1] == 0)
}

#[cfg(test)]
mod radiance_tests {
    use super::super::PtMat;
    use super::*;

    const SUN_COLOUR: [f32; 3] = [2.0, 1.5, 1.0];

    fn env_of(c: [f32; 3], intensity: f32) -> EnvMap {
        let id = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
        EnvMap::from_rgb(4, 2, &[c; 8], id, intensity)
    }

    /// Orthographic camera at z = 4 looking down -z, +-5 units across; world z = 3 + ndc z.
    fn params(env_x: f32, max_bounces: u32, lights: Vec<PLight>) -> RadianceParams {
        let id = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
        RadianceParams {
            size: [64, 64],
            inv_view_proj: [[5.0, 0.0, 0.0, 0.0], [0.0, 5.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 3.0, 1.0]],
            cam_to_world: id,
            cam: [0.0, 0.0, 0.0, max_bounces as f32],
            env: [env_x, 1.0, 1.0, 0.0],
            lights,
        }
    }

    fn sun() -> PLight {
        PLight {
            pos: [0.0, 0.0, 0.0, 1.0],
            dir: [0.0, 0.0, -1.0, 0.0],
            color: [SUN_COLOUR[0], SUN_COLOUR[1], SUN_COLOUR[2], 0.0],
            size: [0.0, 1.0, 0.0, 0.0],
            right: [0.0, 0.0, 0.0, 1.0], // diffuse lobe only
        }
    }

    fn material(base: [f32; 4], cast: f32, receive: f32) -> PtMat {
        let mut m: PtMat = bytemuck::Zeroable::zeroed();
        m.base = base;
        m.params = [0.0, 1.0, 0.0, 1.0]; // dielectric, rough, ior 1: f0 = 0, so F = 0 where v.h = 1
        m.extra = [0.0, cast, receive, 0.0];
        m
    }

    /// Large quads facing +z at the given heights, one material each.
    fn quads(list: &[(f32, PtMat)]) -> PtScene {
        let mut s = PtScene::default();
        let mut tris = Vec::new();
        let mut mat_of = Vec::new();
        for (i, (z, _)) in list.iter().enumerate() {
            let c = |x: f32, y: f32| glam::Vec3::new(x, y, *z);
            tris.push([c(-50.0, -50.0), c(50.0, -50.0), c(-50.0, 50.0)]);
            tris.push([c(50.0, -50.0), c(50.0, 50.0), c(-50.0, 50.0)]);
            mat_of.extend([i as u32; 2]);
        }
        let order = super::super::bvh(&tris, &mut s.nodes);
        for &t in &order {
            for p in tris[t] {
                s.pos.push([p.x, p.y, p.z, 0.0]);
                s.nrm.push([0.0, 0.0, 1.0, 0.0]);
                s.uv.push([[0.0; 2]; 6]);
                s.colors.push([1.0; 4]);
            }
            s.tri_mat.push(mat_of[t]);
        }
        s.mats = list.iter().map(|(_, m)| *m).collect();
        s
    }

    fn centre(s: &PtScene, env: &EnvMap, p: &RadianceParams) -> [f32; 4] {
        radiance(s, env, p, [32.0, 32.0], &mut Rng(12345))
    }

    fn assert_close(got: [f32; 4], want: [f32; 4]) {
        for k in 0..4 {
            assert!((got[k] - want[k]).abs() <= 1e-4, "channel {k}: got {got:?}, want {want:?}");
        }
    }

    fn lit_floor() -> [f32; 4] {
        let base = [0.8, 0.6, 0.4];
        let c = |k: usize| base[k] / PI * SUN_COLOUR[k];
        [c(0), c(1), c(2), 1.0]
    }

    #[test]
    fn empty_scene_shows_the_constant_dome_to_camera_rays() {
        let env = env_of([0.5, 0.25, 0.125], 1.0);
        let p = params(2.0, 4, vec![]);
        assert_close(centre(&PtScene::default(), &env, &p), [0.5, 0.25, 0.125, 1.0]);
    }

    #[test]
    fn diffuse_quad_under_the_sun_gives_base_over_pi_times_colour() {
        let s = quads(&[(0.0, material([0.8, 0.6, 0.4, 1.0], 1.0, 1.0))]);
        let p = params(0.0, 1, vec![sun()]);
        assert_close(centre(&s, &env_of([0.0; 3], 1.0), &p), lit_floor());
    }

    #[test]
    fn a_shadow_casting_quad_between_floor_and_sun_blocks_the_light() {
        let floor = material([0.8, 0.6, 0.4, 1.0], 1.0, 1.0);
        let caster = |cast| material([1.0; 4], cast, 1.0);
        let env = env_of([0.0; 3], 1.0);
        let p = params(0.0, 1, vec![sun()]);
        assert_close(centre(&quads(&[(0.0, floor), (6.0, caster(1.0))]), &env, &p), [0.0, 0.0, 0.0, 1.0]);
        assert_close(centre(&quads(&[(0.0, floor), (6.0, caster(0.0))]), &env, &p), lit_floor());
    }

    #[test]
    fn visibility_through_a_half_transparent_caster_is_one_half() {
        let s = quads(&[(5.0, material([1.0, 1.0, 1.0, 0.5], 1.0, 1.0))]);
        assert_eq!(visibility(&s, [0.0, 0.0, 0.0], [0.0, 0.0, 1.0], 100.0), 0.5);
        assert_eq!(visibility(&PtScene::default(), [0.0, 0.0, 0.0], [0.0, 0.0, 1.0], 100.0), 1.0);
    }

    #[test]
    fn supported_rejects_point_lights_transmission_and_dof_and_accepts_dome_plus_sun() {
        let plain = quads(&[(0.0, material([0.8, 0.6, 0.4, 1.0], 1.0, 1.0))]);
        assert!(supported(&plain, &params(2.0, 4, vec![sun()])));
        let mut point = sun();
        point.pos[3] = 2.0;
        assert!(!supported(&plain, &params(2.0, 4, vec![point])));
        let mut glass = quads(&[(0.0, material([0.8, 0.6, 0.4, 1.0], 1.0, 1.0))]);
        glass.mats[0].params[2] = 0.5;
        assert!(!supported(&glass, &params(2.0, 4, vec![sun()])));
        let mut dof = params(2.0, 4, vec![sun()]);
        dof.cam[1] = 0.1;
        assert!(!supported(&plain, &dof));
    }

    #[test]
    fn pixel_sum_seeds_the_rng_per_pixel_and_sample_and_draws_the_jitter_first() {
        let env = env_of([0.5, 0.25, 0.125], 1.0);
        let p = params(2.0, 4, vec![]);
        let s = PtScene::default();
        let (x, y, si) = (5u32, 7u32, 3u32);
        let mut rng = Rng(pcg((y * 64 + x) * 9781 + pcg(si * 6271 + 1)));
        let (jx, jy) = (rng.rnd(), rng.rnd());
        let want = radiance(&s, &env, &p, [x as f32 + jx, y as f32 + jy], &mut rng);
        assert_eq!(pixel_sum(&s, &env, &p, x, y, si, 1), want);
        assert_eq!(want, [0.5, 0.25, 0.125, 1.0]);
    }
}
