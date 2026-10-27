// Sampler bits are defined by sr_3d::sampling::TextureSampler::flags.
fn address_index(x: i32, size: i32, mode: u32) -> i32 {
    if (mode == 1u) { return clamp(x, 0, size - 1); }
    if (mode == 2u) {
        let v = ((x % (size * 2)) + size * 2) % (size * 2);
        return select(size * 2 - 1 - v, v, v < size);
    }
    if (mode == 3u && (x < 0 || x >= size)) { return -1; }
    return ((x % size) + size) % size;
}
fn address_coord(v: f32, mode: u32) -> f32 {
    if (mode == 1u) { return clamp(v, 0.0, 1.0); }
    if (mode == 2u) { return v - floor(v / 2.0) * 2.0; }
    if (mode == 3u) { return clamp(v, -1.0, 2.0); }
    return fract(v);
}
fn sample_position(uv: vec2<f32>, size: vec2<f32>, flags: u32) -> vec2<f32> {
    return vec2(address_coord(uv.x, (flags >> 3u) & 3u), address_coord(uv.y, (flags >> 5u) & 3u)) * size - 0.5;
}
fn cubic_weights(t: f32) -> vec4<f32> {
    let t2 = t*t; let t3 = t2*t;
    return vec4(-0.5*t+t2-0.5*t3, 1.0-2.5*t2+1.5*t3, 0.5*t+2.0*t2-1.5*t3, -0.5*t2+0.5*t3);
}

// ---------------------------------------------------------------- SREP 71, shared by the raster and path-traced 3D passes

// The procedural sky (sr_3d::sky::Sky::radiance) at a unit direction of the dome's space, for the five vec4 of
// sr_3d::sky::Sky::uniforms: zenith (rgb, exponent), horizon (rgb, 1), ground, sun direction (xyz, cos of half its
// size), sun radiance.
fn sky_radiance(d: vec3<f32>, p: array<vec4<f32>, 5>) -> vec3<f32> {
    let s = -d.y;
    var far = p[0].rgb;
    var t = s;
    if (s < 0.0) { far = p[2].rgb; t = -s; }
    let w = select(pow(min(t, 1.0), p[0].w), 0.0, t <= 0.0);
    var c = p[1].rgb + (far - p[1].rgb) * w;
    if (any(p[4].rgb > vec3(0.0)) && dot(d, p[3].xyz) >= p[3].w) { c += p[4].rgb; }
    return c;
}

// The discs of a physical sky (sr_3d::physical_sky::PhysicalSky::radiance less its sky) at a unit direction of the
// dome's space, for the five vec4 of sr_3d::physical_sky::PhysicalSky::uniforms: moon direction (xyz, cos of half
// its size), mode (w = 2), moon radiance, sun direction (xyz, cos of half its size), sun radiance (rgb, the sine of
// the horizon's elevation). Each disc is drawn above the horizon only.
fn sky_discs(d: vec3<f32>, p: array<vec4<f32>, 5>) -> vec3<f32> {
    var c = vec3(0.0);
    if (-d.y < p[4].w) { return c; }
    if (any(p[4].rgb > vec3(0.0)) && dot(d, p[3].xyz) >= p[3].w) { c += p[4].rgb; }
    if (any(p[2].rgb > vec3(0.0)) && dot(d, p[0].xyz) >= p[0].w) { c += p[2].rgb; }
    return c;
}

// The subsurface lobe (sr_3d::subsurface::lobe): the mean clamped cosine over the rings at the profile's quantiles
// (sr_3d::subsurface::QUANTILES, in units of the shape parameter d) on a surface of curvature 1 / radius.
const SSS_Q = array<f32, 16>(0.06383456, 0.20014349, 0.34958202, 0.51450014, 0.69787014, 0.90351087, 1.1364199, 1.4032811, 1.7132746, 2.0794415, 2.5211546, 3.0690293, 3.776038, 4.7477603, 6.25375, 9.535895);
fn sss_ring(c: f32, alpha: f32) -> f32 {
    let cc = clamp(c, -1.0, 1.0);
    let a = cc * cos(alpha);
    let b = sqrt(max(1.0 - cc * cc, 0.0)) * abs(sin(alpha));
    if (a >= b) { return a; }
    if (a <= -b) { return 0.0; }
    return (a * acos(clamp(-a / b, -1.0, 1.0)) + sqrt(max(b * b - a * a, 0.0))) * 0.318309886183791;
}
fn sss_lobe(c: f32, curvature: f32, d: f32) -> f32 {
    // a variable, which may be indexed by a value that is not constant
    var q = SSS_Q;
    var e = 0.0;
    for (var k = 0u; k < 16u; k++) {
        let half = min(q[k] * d * curvature * 0.5, 1.0);
        e += sss_ring(c, 2.0 * asin(half));
    }
    return e / 16.0;
}
fn sss_lobe3(c: f32, curvature: f32, d: vec3<f32>) -> vec3<f32> {
    return vec3(sss_lobe(c, curvature, d.x), sss_lobe(c, curvature, d.y), sss_lobe(c, curvature, d.z));
}

// ---------------------------------------------------------------- a planet's atmosphere shell (the planets draft), shared by both 3D passes

// sr_3d::planet_shell::PlanetShell's table: rows x, y and up of the eye's frame (w: the rows' first and last
// coordinates and the sun's azimuth), [rows, columns, 1 for a parallel view], the eye in its frame (metres from the
// centre), then the radiance nodes row by row from the limb and the transmittance of every row. Rows are spaced by
// the square of their index over the elevation from the eye, or over the impact radius of a parallel view's rays.
const SHELL_HEADER: u32 = 5u;
// Each pass that includes this reads row i of the table it binds as shell_at(i).

// The row coordinate of the ray along the world direction d whose origin lies `o` metres from the eye (world axes):
// its elevation, or its impact radius in a parallel view; its direction or point in the eye's frame in `l`.
fn shell_coordinate(o: vec3<f32>, d: vec3<f32>, l: ptr<function, vec3<f32>>) -> f32 {
    let x = shell_at(0).xyz;
    let y = shell_at(1).xyz;
    let up = shell_at(2).xyz;
    if (shell_at(3).z > 0.5) {
        *l = vec3(dot(x, o), dot(y, o), dot(up, o)) + shell_at(4).xyz;
        return length((*l).xy);
    }
    *l = vec3(dot(x, d), dot(y, d), dot(up, d));
    return asin(clamp((*l).z, -1.0, 1.0));
}

// Metres from a parallel view's ray origin `o` metres off the eye to where it enters the shell's top, past `start`
// metres to the plane tangent to the top; 0 in a view from the eye.
fn shell_entry(o: vec3<f32>, start: f32) -> f32 {
    if (shell_at(3).z < 0.5) { return 0.0; }
    var l: vec3<f32>;
    let rho = shell_coordinate(o, vec3(0.0), &l);
    let top = shell_at(1).w;
    return start + top - sqrt(max((top - rho) * (top + rho), 0.0));
}

// The fractional row of coordinate e, or −1 outside the rows.
fn shell_row(e: f32) -> f32 {
    let h = shell_at(0).w;
    let top = shell_at(1).w;
    if (e < h || e > top) { return -1.0; }
    return sqrt(clamp((e - h) / (top - h), 0.0, 1.0)) * (shell_at(3).x - 1.0);
}

// Rule 9's radiance along the world direction d of a ray from `o` metres off the eye that meets no surface: zero
// toward the planet and past the shell's top, bilinear between the nodes.
fn shell_radiance(o: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    var l: vec3<f32>;
    let fi = shell_row(shell_coordinate(o, d, &l));
    if (fi < 0.0) { return vec3(0.0); }
    let rows = u32(shell_at(3).x);
    let cols = u32(shell_at(3).y);
    var a = 0.0;
    if (l.x != 0.0 || l.y != 0.0) { a = atan2(l.y, l.x) - shell_at(2).w; }
    // the angle from the sun's side, 0 to π
    a = a - 2.0 * PI * floor(a / (2.0 * PI));
    if (a > PI) { a = 2.0 * PI - a; }
    let fj = a / PI * f32(cols - 1u);
    let i0 = min(u32(floor(fi)), rows - 2u);
    let j0 = min(u32(floor(fj)), cols - 2u);
    let ti = fi - f32(i0);
    let tj = fj - f32(j0);
    let at = SHELL_HEADER + i0 * cols + j0;
    let lo = mix(shell_at(at).xyz, shell_at(at + 1u).xyz, tj);
    let hi = mix(shell_at(at + cols).xyz, shell_at(at + cols + 1u).xyz, tj);
    return mix(lo, hi, ti);
}

// The transmittance to space along that ray: zero toward the planet, one past the shell's top.
fn shell_transmittance(o: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    var l: vec3<f32>;
    let e = shell_coordinate(o, d, &l);
    let fi = shell_row(e);
    if (fi < 0.0) { return select(vec3(0.0), vec3(1.0), e > shell_at(1).w); }
    let rows = u32(shell_at(3).x);
    let cols = u32(shell_at(3).y);
    let i = min(u32(floor(fi)), rows - 2u);
    let at = SHELL_HEADER + rows * cols + i;
    return mix(shell_at(at).xyz, shell_at(at + 1u).xyz, fi - f32(i));
}
