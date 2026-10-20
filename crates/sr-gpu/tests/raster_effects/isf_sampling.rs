//! ISF pass buffers read at exact texel centres return that texel (atlas ENGINE_FINDINGS s1 #5, cell 65): an integer
//! cellular automaton read through `texture()` / `IMG_PIXEL` / `IMG_NORM_PIXEL` at `(cell + 0.5) / size` must evolve
//! exactly as the same automaton read through `texelFetch`.

use super::common;
use common::*;

fn shader_doc(w: u32, h: u32, src: &str, body: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.6"><project width="{w}" height="{h}" fps="24" duration="1" background="#000000"/><composition>{body}</composition><effects><effect id="fx" type="shader" src="{src}" space="raw"/></effects></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

fn rect(w: u32, h: u32) -> String {
    format!(r##"<shape id="s" shape="rect" width="{w}" height="{h}" fill="#000000" effects="fx"/>"##)
}

/// Reads a non-power-of-two FLOAT pass buffer holding each texel's own coordinates at every texel centre, through
/// every ISF read the header language offers, and draws the largest error per read in the output.
const CENTRES: &str = r#"/*{ "PASSES": [ {"TARGET": "st", "FLOAT": true, "WIDTH": 410, "HEIGHT": 221}, {"WIDTH": 410, "HEIGHT": 221} ] }*/
void main(){
  vec2 G = vec2(410.0, 221.0); vec2 cell = floor(isf_FragNormCoord * G);
  if(PASSINDEX == 0){ gl_FragColor = vec4(cell, 0.0, 1.0); return; }
  vec2 want = cell;
  float a = length(texture(st, (cell + 0.5) / G).xy - want);
  float b = length(IMG_NORM_PIXEL(st, (cell + 0.5) / G).xy - want);
  float c = length(IMG_PIXEL(st, cell + 0.5).xy - want);
  float d = length(IMG_THIS_PIXEL(st).xy - want);
  gl_FragColor = vec4(a, b, c + d, 1.0);
}
"#;

#[test]
fn texel_centre_reads_of_a_pass_buffer_are_exact() {
    write_fixture("isf-centres.fs", CENTRES);
    let d = shader_doc(410, 221, "isf-centres.fs", &rect(410, 221));
    let Some(r) = render(&d) else { return };
    assert!(
        r.stats.errors.is_empty() && r.stats.unsupported.is_empty(),
        "{:?} {:?}",
        r.stats.errors,
        r.stats.unsupported
    );
    assert_eq!(r.size, [410, 221]);
    let mut worst = [0.0f32; 3];
    let mut bad = [0usize; 3];
    for p in &r.px {
        for k in 0..3 {
            worst[k] = worst[k].max(p[k]);
            if p[k] != 0.0 {
                bad[k] += 1;
            }
        }
    }
    assert_eq!(
        bad, [0; 3],
        "texel-centre reads off (texture, IMG_NORM_PIXEL, IMG_PIXEL + IMG_THIS_PIXEL): worst {worst:?}"
    );
}

fn atlas_ca(read: &str) -> String {
    format!(
        r#"/*{{ "DESCRIPTION": "cyclic CA (8 states, range 3, threshold 5) in one persistent pass under <iterate>",
    "PASSES": [ {{"TARGET": "st", "PERSISTENT": true, "FLOAT": true, "WIDTH": 410, "HEIGHT": 221}}, {{}} ] }}*/
uint lowbias32(uint x){{ x ^= x >> 16u; x *= 0x7feb352du; x ^= x >> 15u; x *= 0x846ca68bu; x ^= x >> 16u; return x; }}
#define READ(p) {read}
void main(){{
  vec2 uv = isf_FragNormCoord; vec2 G = vec2(410.0, 221.0); vec2 cell = floor(uv*G);
  if(PASSINDEX == 0){{
    if(FRAMEINDEX == 0){{ uint i = uint(int(cell.x) + 4096*int(cell.y)); gl_FragColor = vec4(float(lowbias32(i) % 8u), 0.0, 0.0, 1.0); return; }}
    float c = READ(cell), nxt = mod(c + 1.0, 8.0); int cnt = 0;
    for(int j=-3;j<=3;j++) for(int i=-3;i<=3;i++) if(abs(READ(mod(cell + vec2(i,j), G)) - nxt) < 0.5) cnt++;
    gl_FragColor = vec4(cnt >= 5 ? nxt : c, 0.0, 0.0, 1.0); return;
  }}
  gl_FragColor = vec4(vec3(texelFetch(st, ivec2(cell), 0).x/8.0), 1.0);
}}
"#
    )
}

#[test]
fn persistent_cellular_automaton_reads_like_texel_fetch() {
    // atlas cells/s1/_repro/iterate-ca.scene.xml (texture) against iterate-ca-fetch.scene.xml (texelFetch)
    let body = r#"<iterate id="it" steps="60">RECT</iterate>"#.replace("RECT", &rect(821, 442));
    let mut out = Vec::new();
    for (name, read) in [
        ("isf-ca-fetch.fs", "texelFetch(st, ivec2(p), 0).x"),
        ("isf-ca-texture.fs", "texture(st, ((p) + 0.5) / G).x"),
        ("isf-ca-norm.fs", "IMG_NORM_PIXEL(st, ((p) + 0.5) / G).x"),
        ("isf-ca-pixel.fs", "IMG_PIXEL(st, (p) + 0.5).x"),
    ] {
        write_fixture(name, atlas_ca(read));
        let d = shader_doc(821, 442, name, &body);
        let Some(r) = render(&d) else { return };
        assert!(
            r.stats.errors.is_empty() && r.stats.unsupported.is_empty(),
            "{name}: {:?} {:?}",
            r.stats.errors,
            r.stats.unsupported
        );
        out.push((name, r.px));
    }
    let (_, want) = &out[0];
    // the automaton has moved: the field is not the seed's eight flat levels everywhere frozen
    assert!(want.iter().any(|p| p[0] > 0.0) && want.iter().any(|p| p[0] == 0.0));
    for (name, px) in &out[1..] {
        let differ = px.iter().zip(want).filter(|(a, b)| a != b).count();
        assert_eq!(differ, 0, "{name}: {differ} pixels differ from the texelFetch automaton");
    }
}

#[test]
fn pass_names_may_also_name_locals() {
    // atlas ENGINE_FINDINGS (cells 64, 01): a TARGET `q` broke the hash helper's `ivec2 q` and `uv.st` swizzles.
    // The values are the same at every pixel: a custom shader's tile has a margin around the node, so a pixel of the
    // frame is not the same texel of an 8 × 4 pass.
    let code = r#"/*{ "PASSES": [ {"TARGET": "q", "WIDTH": 8, "HEIGHT": 4}, {"TARGET": "st", "WIDTH": 8, "HEIGHT": 4}, {"WIDTH": 8, "HEIGHT": 4} ] }*/
float hash(vec2 p) { ivec2 q = ivec2(p); return float(q.x + 8 * q.y) / 32.0; }
void main(){
  vec2 uv = vec2(0.25, 0.75);
  if(PASSINDEX == 0){ gl_FragColor = vec4(hash(vec2(5.0, 2.0)), 0.0, 0.0, 1.0); return; }
  if(PASSINDEX == 1){ float st = IMG_THIS_PIXEL(q).x; gl_FragColor = vec4(st, uv.st, 1.0); return; }
  gl_FragColor = vec4(IMG_THIS_PIXEL(st).xyz, 1.0);
}
"#;
    write_fixture("isf-names.fs", code);
    let d = shader_doc(8, 4, "isf-names.fs", &rect(8, 4));
    let Some(r) = render(&d) else { return };
    assert!(
        r.stats.errors.is_empty() && r.stats.unsupported.is_empty(),
        "{:?} {:?}",
        r.stats.errors,
        r.stats.unsupported
    );
    // hash(5, 2) = (5 + 8·2) / 32 through the helper's local `q`; then uv.st = (0.25, 0.75) through the swizzle
    let want = [21.0 / 32.0, 0.25, 0.75];
    for (i, p) in r.px.iter().enumerate() {
        assert!((0..3).all(|k| (p[k] - want[k]).abs() < 1e-3), "pixel {i}: {p:?} want {want:?}");
    }
}

#[test]
fn unsupported_glsl_gets_a_named_diagnostic() {
    for (name, code, want) in [
        (
            "isf-f64.fs",
            "/*{ \"PASSES\": [{}] }*/\nvoid main(){\n  double d = 0.5;\n  gl_FragColor = vec4(float(d));\n}\n",
            "shader:3: [GLSL-F64]",
        ),
        (
            "isf-sampler-arg.fs",
            "/*{ \"INPUTS\": [{\"NAME\": \"img\", \"TYPE\": \"image\"}] }*/\nvec4 tap(sampler2D s, vec2 p){ return texture(s, p); }\nvoid main(){ gl_FragColor = tap(img, isf_FragNormCoord); }\n",
            "shader:2: [GLSL-SAMPLER-ARG]",
        ),
    ] {
        write_fixture(name, code);
        let d = shader_doc(8, 4, name, &rect(8, 4));
        let Some(r) = render(&d) else { return };
        let all = r.stats.errors.iter().chain(&r.stats.unsupported).cloned().collect::<Vec<_>>().join("\n");
        assert!(all.contains(want), "{name}: {all}");
    }
}
