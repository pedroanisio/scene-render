// Resolves the nearest reverse-Z depth into a float texture. The depth is bound as a float texture, multisampled or
// not as the pass is, not a depth one, so that OpenGL, whose shaders cannot read multisampled depth, runs it too.
@group(0) @binding(1) var<uniform> pfr: Frame;
@group(0) @binding(0) var ms_depth: DEPTH_TEXTURE;

struct PostOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_post(@builtin(vertex_index) vi: u32) -> PostOut {
    var o: PostOut;
    let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    o.pos = vec4(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2(p.x, 1.0 - p.y);
    return o;
}


fn resolve_depth(i: PostOut) -> f32 {
    let dims = vec2<f32>(textureDimensions(ms_depth));
    let q = vec2<i32>(clamp(i.uv * dims, vec2(0.0), dims - 1.0));
    DEPTH_RESOLVE
}

@fragment
fn fs_depth_resolve(i: PostOut) -> @location(0) vec4<f32> {
    return vec4(resolve_depth(i), 0.0, 0.0, 1.0);
}

@fragment
fn fs_depth_coc(i: PostOut) -> @location(0) vec4<f32> {
    let d = resolve_depth(i);
    return vec4(d, coc(d), 0.0, 1.0);
}
