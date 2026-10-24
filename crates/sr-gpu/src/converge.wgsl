// The largest change between two textures (SREP 67 `until="converged"`): the greatest |a − b| over every channel of
// every texel, kept as the bits of a non-negative f32 in an atomic (non-negative finite floats order as their bits
// do). A NaN difference counts as no change, as a CPU fold with `f32::max` would have it.

@group(0) @binding(0) var a: texture_2d<f32>;
@group(0) @binding(1) var b: texture_2d<f32>;
@group(0) @binding(2) var<storage, read_write> peak: atomic<u32>;

var<workgroup> local: atomic<u32>;

// |v| as the bits of a float, by the integer rules: the sign is the top bit, and a NaN lies above infinity.
fn magnitude(v: f32) -> u32 {
    let bits = bitcast<u32>(v) & 0x7fffffffu;
    return select(bits, 0u, bits > 0x7f800000u);
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>, @builtin(local_invocation_index) index: u32) {
    if (index == 0u) {
        atomicStore(&local, 0u);
    }
    workgroupBarrier();
    let size = textureDimensions(a);
    var bits = 0u;
    if (id.x < size.x && id.y < size.y) {
        let p = vec2<i32>(id.xy);
        let diff = textureLoad(a, p, 0) - textureLoad(b, p, 0);
        bits = max(max(magnitude(diff.x), magnitude(diff.y)), max(magnitude(diff.z), magnitude(diff.w)));
    }
    atomicMax(&local, bits);
    workgroupBarrier();
    if (index == 0u) {
        atomicMax(&peak, atomicLoad(&local));
    }
}
