// Rule 5 of the star-fields draft: a coverage mask of constellation labels or boundaries over the pass, added in one
// grey at background depth (reverse Z: 0), so every surface drawn after it covers it.

@group(0) @binding(0) var mask: texture_2d<f32>;
@group(0) @binding(1) var<uniform> grey: vec4<f32>;

// one triangle over the frame
@vertex
fn vs_overlay(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2(f32((vi << 1u) & 2u), f32(vi & 2u));
    return vec4(uv * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_overlay(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let k = textureLoad(mask, vec2<i32>(pos.xy), 0).r;
    return vec4(grey.rgb * k, 0.0);
}
