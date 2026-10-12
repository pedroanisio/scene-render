//! An included document's own definitions (an include merges its document's assets, effects and materials are merged under the
//! namespace `includeId/` and every reference to them is renamed): content inside an include finds the materials,
//! effects, font assets, generators and geo assets its own document declares, never the including document's, and
//! the including document never finds the included one's.
//!
//! Two measures. Drawn through an include, a document draws exactly as it draws by itself (the include composites
//! its own target 1:1 onto the frame). And renaming the including document's own definition (so that no id is shared
//! any more) changes no pixel: neither side reads the other's definition of a shared id.

mod common;
use common::*;

use std::path::{Path, PathBuf};

#[path = "../../sr-text/tests/support/test_font.rs"]
mod test_font;

/// A fresh directory for one test's documents.
fn dir(test: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr-gpu-include-scope-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// The document at `path`, loaded from its own directory (includes and relative paths resolve from there).
fn load(path: &Path) -> sr_model::Document {
    let text = std::fs::read_to_string(path).unwrap();
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(path.parent().unwrap().to_path_buf()) };
    sr_model::load_str(&text, &opts).unwrap_or_else(|e| panic!("{e:?}\n{text}"))
}

/// Renders `path` at time `t`; None without a GPU. The frame must have no error and nothing unsupported.
fn frame(path: &Path, t: f64) -> Option<Rendered> {
    let r = render_times(&load(path), &[t])?;
    assert!(r.stats.errors.is_empty(), "{}: {:?}", path.display(), r.stats.errors);
    assert!(r.stats.unsupported.is_empty(), "{}: {:?}", path.display(), r.stats.unsupported);
    Some(r)
}

/// The number of pixels of `a`'s columns `[x0, x0 + b.width)` that differ from `b`.
fn differ_at(a: &Rendered, x0: u32, b: &Rendered) -> usize {
    assert_eq!(a.size[1], b.size[1]);
    let mut n = 0;
    for y in 0..b.size[1] {
        for x in 0..b.size[0] {
            if a.at(x0 + x, y) != b.at(x, y) {
                if n < 5 {
                    eprintln!("differs at ({x}, {y}): {:?} and {:?}", a.at(x0 + x, y), b.at(x, y));
                }
                n += 1;
            }
        }
    }
    n
}

fn differ(a: &Rendered, b: &Rendered) -> usize {
    assert_eq!(a.size, b.size);
    a.px.iter().zip(&b.px).filter(|(p, q)| p != q).count()
}

/// Pixels of `r` that are mostly red.
fn red(r: &Rendered) -> usize {
    r.px.iter().filter(|p| p[0] > 0.5 && p[1] < 0.2 && p[2] < 0.2).count()
}

/// The checks shared by every case: `inc` (a document of `w` × `h` with only the include, of `inner`, at its
/// origin) draws `inner` as `inner` draws itself; `both` (that include at the origin, beside content of the including
/// document that uses the same ids) draws the include's part as `inner` does, and as `renamed` (`both` with the
/// including document's ids renamed) draws, pixel for pixel.
fn check(inner: &Path, inc: &Path, both: &Path, renamed: &Path, t: f64) -> Option<(Rendered, Rendered)> {
    let direct = frame(inner, t)?;
    let through = frame(inc, t)?;
    let n = differ(&direct, &through);
    assert_eq!(n, 0, "drawn through an include, the document differs from itself in {n} pixels");
    let a = frame(both, t)?;
    let b = frame(renamed, t)?;
    let n = differ_at(&a, 0, &direct);
    assert_eq!(n, 0, "beside a definition of the same id, the include differs from its document in {n} pixels");
    let n = differ(&a, &b);
    assert_eq!(n, 0, "renaming the including document's definition changed {n} pixels");
    if t > 0.0 {
        // the frame at `t` after the frame at 0, from the same renderer: what changed in the included document's
        // definitions between the two frames reaches the cached content
        let later = render_times(&load(both), &[0.0, t])?;
        let n = differ(&a, &later);
        assert_eq!(n, 0, "after an earlier frame, the frame differs from itself rendered alone in {n} pixels");
    }
    Some((direct, a))
}

/// The including documents of `check`: the include alone (`w` × `h`), and the include at the origin of a frame
/// twice as wide whose right half holds `parent` (the including document's own content, with `defs` as its
/// top-level sections before the composition and `post` after it), once as written and once with each `(from, to)`
/// of `renames` applied to `defs`, `parent` and `post`.
#[allow(clippy::too_many_arguments)]
fn including(
    d: &Path,
    src: &str,
    version: &str,
    [w, h]: [u32; 2],
    defs: &str,
    parent: &str,
    post: &str,
    renames: &[(&str, &str)],
) -> (PathBuf, PathBuf, PathBuf) {
    let doc = |w: u32, defs: &str, body: &str, post: &str| {
        format!(
            r##"<scene version="{version}"><project width="{w}" height="{h}" fps="24" duration="2" background="#000000"/>{defs}<composition><include id="i" src="{src}"/>{body}</composition>{post}</scene>"##
        )
    };
    let rename = |s: &str| renames.iter().fold(s.to_string(), |s, (from, to)| s.replace(from, to));
    let (inc, both, renamed) = (d.join("inc.scene.xml"), d.join("both.scene.xml"), d.join("renamed.scene.xml"));
    write(&inc, &doc(w, "", "", ""));
    write(&both, &doc(2 * w, defs, parent, post));
    write(&renamed, &doc(2 * w, &rename(defs), &rename(parent), &rename(post)));
    (inc, both, renamed)
}

/// A viewport3D of 200 × 100 at x = `x` whose camera looks at a sphere of radius 30 at its centre, of material `mat`.
fn sphere_viewport(id: &str, x: u32, mat: &str) -> String {
    format!(
        r#"<viewport3D id="{id}" x="{x}" y="0" width="200" height="100" camera="{id}-c"><camera id="{id}-c" x="100" y="50" z="-173.205"/><object3D id="{id}-o" primitive="sphere" radius="30" x="100" y="50" material="{mat}"/></viewport3D>"#
    )
}

/// The issue's reproduction: an object3D inside an included document names a material its document declares.
#[test]
fn an_included_object_finds_its_documents_material() {
    let d = dir("material");
    let inner = d.join("B.scene.xml");
    write(
        &inner,
        &format!(
            r##"<scene version="1.6"><project width="200" height="100" fps="24" duration="2" background="#000000"/><materials><material id="m" baseColor="#F4BB69" emissive="#F4BB69"/></materials><composition>{}</composition></scene>"##,
            sphere_viewport("v", 0, "m")
        ),
    );
    let inc = d.join("A.scene.xml");
    write(
        &inc,
        r##"<scene version="1.6"><project width="200" height="100" fps="24" duration="2" background="#000000"/><composition><include id="i" src="B.scene.xml"/></composition></scene>"##,
    );
    let (Some(direct), Some(through)) = (frame(&inner, 0.5), frame(&inc, 0.5)) else { return };
    let centre = direct.at(100, 50);
    assert!(centre[0] > 0.3 && centre[0] > centre[2], "the sphere is drawn in its material's colour: {centre:?}");
    let n = differ(&direct, &through);
    assert_eq!(n, 0, "drawn through an include, the sphere differs from the document itself in {n} pixels");
}

/// Including and included documents both declare (and animate) a material `m`: each object takes its own
/// document's.
#[test]
fn a_material_id_in_both_documents_is_each_documents_own() {
    let d = dir("material-same-id");
    let inner = d.join("B.scene.xml");
    write(
        &inner,
        &format!(
            r##"<scene version="1.6"><project width="200" height="100" fps="24" duration="2" background="#000000"/><materials><material id="m" unlit="true" baseColor="#FFC000"><animate property="baseColor"><key time="0" value="#FFC000"/><key time="1" value="#00FF40"/></animate></material></materials><composition>{}</composition></scene>"##,
            sphere_viewport("v", 0, "m")
        ),
    );
    let (inc, both, renamed) = including(
        &d,
        "B.scene.xml",
        "1.6",
        [200, 100],
        r##"<materials><material id="m" unlit="true" baseColor="#2040FF"><animate property="baseColor"><key time="0" value="#2040FF"/><key time="1" value="#FF00FF"/></animate></material></materials>"##,
        &sphere_viewport("p", 200, "m"),
        "",
        &[(r#"id="m""#, r#"id="pm""#), (r#"material="m""#, r#"material="pm""#)],
    );
    let Some((direct, a)) = check(&inner, &inc, &both, &renamed, 0.5) else { return };
    // and each sphere is drawn: the include's halfway from orange to green, the parent's from blue to magenta
    let left = direct.at(100, 50);
    assert!(left[1] > 0.2 && left[2] < 0.1, "the include's sphere: {left:?}");
    let right = a.at(300, 50);
    assert!(right[0] > 0.1 && right[2] > 0.5 && right[1] < 0.1, "the parent's sphere: {right:?}");
}

/// A material map that names an asset (`#gen`) finds it in the material's document, and a map that is a path
/// resolves from that document's directory.
#[test]
fn an_included_materials_maps_are_its_documents() {
    let d = dir("material-maps");
    let sub = d.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    image::RgbaImage::from_fn(4, 4, |_, _| image::Rgba([0, 0, 255, 255])).save(sub.join("blue.png")).unwrap();
    let inner = sub.join("B.scene.xml");
    write(
        &inner,
        r##"<scene version="1.6"><project width="200" height="100" fps="24" duration="2" background="#000000"/>
<assets><generator id="gen" kind="solid" width="4" height="4" paint="#00FF00"/></assets>
<materials><material id="mg" unlit="true" doubleSided="true" baseColorMap="#gen"/><material id="mf" unlit="true" doubleSided="true" baseColorMap="blue.png"/></materials>
<composition><viewport3D id="v" x="0" y="0" width="200" height="100" camera="c"><camera id="c" x="100" y="50" z="-173.205"/>
<object3D id="g" primitive="plane" width="60" height="60" x="60" y="50" material="mg"/>
<object3D id="f" primitive="plane" width="60" height="60" x="140" y="50" material="mf"/>
</viewport3D></composition></scene>"##,
    );
    let (inc, both, renamed) = including(
        &d,
        "sub/B.scene.xml",
        "1.6",
        [200, 100],
        r##"<assets><generator id="gen" kind="solid" width="4" height="4" paint="#FF0000"/></assets>"##,
        r##"<layer id="pl" asset="gen" x="300" y="0" scaleX="10" scaleY="10"/>"##,
        "",
        &[(r#"id="gen""#, r#"id="pgen""#), (r#"asset="gen""#, r#"asset="pgen""#)],
    );
    let Some((direct, _)) = check(&inner, &inc, &both, &renamed, 0.0) else { return };
    let (g, f) = (direct.at(60, 50), direct.at(140, 50));
    assert!(g[1] > 0.5 && g[0] < 0.1 && g[2] < 0.1, "the generator map is the include's green: {g:?}");
    assert!(f[2] > 0.5 && f[0] < 0.1 && f[1] < 0.1, "the file map is the include's blue.png: {f:?}");
}

/// Including and included documents both declare (and animate) an effect `e`: each node takes its own document's.
#[test]
fn an_effect_id_in_both_documents_is_each_documents_own() {
    let d = dir("effect-same-id");
    let inner = d.join("B.scene.xml");
    write(
        &inner,
        r##"<scene version="1.1"><project width="100" height="60" fps="24" duration="2" background="#000000"/><composition>
<shape id="r" shape="rect" x="20" y="10" width="60" height="40" fill="#FFFFFF" effects="e"/></composition>
<effects><effect id="e" type="fill" color="#00FF00"><animate property="color"><key time="0" value="#00FF00"/><key time="1" value="#FFFF00"/></animate></effect></effects></scene>"##,
    );
    let (inc, both, renamed) = including(
        &d,
        "B.scene.xml",
        "1.1",
        [100, 60],
        "",
        r##"<shape id="pr" shape="rect" x="120" y="10" width="60" height="40" fill="#FFFFFF" effects="e"/>"##,
        r##"<effects><effect id="e" type="fill" color="#FF00FF"><animate property="color"><key time="0" value="#FF00FF"/><key time="1" value="#0000FF"/></animate></effect></effects>"##,
        &[(r#"id="e""#, r#"id="pe""#), (r#"effects="e""#, r#"effects="pe""#)],
    );
    let Some((direct, a)) = check(&inner, &inc, &both, &renamed, 0.5) else { return };
    let left = direct.at(50, 30);
    assert!(left[0] > 0.1 && left[1] > 0.5 && left[2] < 0.1, "the include's fill, green to yellow: {left:?}");
    let right = a.at(150, 30);
    assert!(right[0] > 0.1 && right[2] > 0.5 && right[1] < 0.1, "the parent's fill, magenta to blue: {right:?}");
}

/// An included document's shader effect reads its shader file from that document's directory, not the including
/// document's (which here holds a different file of the same name, used by an effect of the same id).
#[test]
fn an_included_shader_effect_reads_its_documents_file() {
    let d = dir("shader-file");
    let sub = d.join("sub");
    write(&sub.join("tint.glsl"), "vec4 effect(vec2 uv) { vec4 c = getColor(uv); return vec4(c.b, c.g, c.r, c.a); }\n");
    write(&d.join("tint.glsl"), "vec4 effect(vec2 uv) { vec4 c = getColor(uv); return vec4(0.0, c.a, 0.0, c.a); }\n");
    let inner = sub.join("B.scene.xml");
    write(
        &inner,
        r##"<scene version="1.1"><project width="100" height="60" fps="24" duration="2" background="#000000"/><composition>
<shape id="r" shape="rect" x="20" y="10" width="60" height="40" fill="#FF8000" effects="sh"/></composition>
<effects><effect id="sh" type="shader" src="tint.glsl"/></effects></scene>"##,
    );
    let (inc, both, renamed) = including(
        &d,
        "sub/B.scene.xml",
        "1.1",
        [100, 60],
        "",
        r##"<shape id="pr" shape="rect" x="120" y="10" width="60" height="40" fill="#FF8000" effects="sh"/>"##,
        r##"<effects><effect id="sh" type="shader" src="tint.glsl"/></effects>"##,
        &[(r#"id="sh""#, r#"id="psh""#), (r#"effects="sh""#, r#"effects="psh""#)],
    );
    let Some((direct, a)) = check(&inner, &inc, &both, &renamed, 0.0) else { return };
    let left = direct.at(50, 30);
    assert!(left[2] > 0.9 && left[0] < 0.1, "the include's shader swaps orange to azure: {left:?}");
    let right = a.at(150, 30);
    assert!(right[1] > 0.9 && right[0] < 0.1 && right[2] < 0.1, "the parent's shader draws green: {right:?}");
}

/// Including and included documents both declare a font asset `f` (two fonts whose glyphs sit at different
/// heights): each text takes its own document's.
#[test]
fn a_font_asset_id_in_both_documents_is_each_documents_own() {
    let d = dir("font-same-id");
    let font = |name: &str, family: &str, ascent: i16| {
        let m = test_font::Metrics {
            hhea_ascender: ascent,
            typo_ascender: ascent,
            win_ascent: ascent as u16,
            ..Default::default()
        };
        std::fs::write(d.join(name), test_font::build(family, m)).unwrap();
    };
    font("low.ttf", "Include Low", 1400);
    font("high.ttf", "Include High", 800);
    let text = |id: &str, font: &str| {
        format!(r##"<text id="{id}" text="H" width="80" height="80" size="40" color="#FFFFFF" fontAsset="{font}"/>"##)
    };
    let inner = d.join("B.scene.xml");
    write(
        &inner,
        &format!(
            r##"<scene version="1.1"><project width="100" height="80" fps="24" duration="2" background="#000000"/><assets><font id="f" family="Include Low" src="low.ttf"/>{}</assets><composition><layer id="l" asset="t" x="10" y="0"/></composition></scene>"##,
            text("t", "f")
        ),
    );
    let defs = format!(r##"<assets><font id="f" family="Include High" src="high.ttf"/>{}</assets>"##, text("pt", "f"));
    let (inc, both, renamed) = including(
        &d,
        "B.scene.xml",
        "1.1",
        [100, 80],
        &defs,
        r#"<layer id="pl" asset="pt" x="110" y="0"/>"#,
        "",
        &[(r#"id="f""#, r#"id="pf""#), (r#"fontAsset="f""#, r#"fontAsset="pf""#)],
    );
    let Some((direct, a)) = check(&inner, &inc, &both, &renamed, 0.0) else { return };
    // the two fonts place the glyph at different heights: the include's text and the parent's are not alike
    let rows = |r: &Rendered, x0: u32| {
        (0..80).filter(|&y| (x0..x0 + 100).any(|x| r.at(x, y)[0] > 0.5)).min().unwrap_or_else(|| panic!("no glyph"))
    };
    assert_ne!(rows(&direct, 0), rows(&a, 100), "the parent's text takes the parent's font, the include's its own");
}

/// Including and included documents both declare an animated generator `gen` drawn by a layer: each layer takes
/// its own document's animation.
#[test]
fn an_animated_asset_id_in_both_documents_is_each_documents_own() {
    let d = dir("asset-same-id");
    let inner = d.join("B.scene.xml");
    write(
        &inner,
        r##"<scene version="1.1"><project width="100" height="60" fps="24" duration="2" background="#000000"/><assets><generator id="gen" kind="solid" width="4" height="4" paint="#00FF00"><animate property="paint"><key time="0" value="#00FF00"/><key time="1" value="#FFFF00"/></animate></generator></assets><composition><layer id="l" asset="gen" x="20" y="10" scaleX="15" scaleY="10"/></composition></scene>"##,
    );
    let (inc, both, renamed) = including(
        &d,
        "B.scene.xml",
        "1.1",
        [100, 60],
        r##"<assets><generator id="gen" kind="solid" width="4" height="4" paint="#FF00FF"><animate property="paint"><key time="0" value="#FF00FF"/><key time="1" value="#0000FF"/></animate></generator></assets>"##,
        r#"<layer id="pl" asset="gen" x="120" y="10" scaleX="15" scaleY="10"/>"#,
        "",
        &[(r#"id="gen""#, r#"id="pgen""#), (r#"asset="gen""#, r#"asset="pgen""#)],
    );
    let Some((direct, a)) = check(&inner, &inc, &both, &renamed, 0.5) else { return };
    let left = direct.at(50, 30);
    assert!(left[0] > 0.1 && left[1] > 0.5 && left[2] < 0.1, "the include's generator, green to yellow: {left:?}");
    let right = a.at(150, 30);
    assert!(right[0] > 0.1 && right[2] > 0.5 && right[1] < 0.1, "the parent's generator, magenta to blue: {right:?}");
}

/// A map inside an included document draws the geo assets its document declares, and the including document's
/// map draws its own of the same id.
#[test]
fn a_geo_asset_id_in_both_documents_is_each_documents_own() {
    let d = dir("geo-same-id");
    let geo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/geo");
    std::fs::copy(geo.join("lambert-box.geojson"), d.join("box.geojson")).unwrap();
    std::fs::copy(geo.join("squares.geojson"), d.join("squares.geojson")).unwrap();
    let map = |id: &str, geo: &str| {
        format!(
            r##"<geo id="{geo}" src="SRC"/><map id="{id}" width="480" height="360" projection="lambert-conformal" centerLon="0" centerLat="50" background="#000000"><geoLayer geo="{geo}" fill="#FF0000" stroke="#FF0000" strokeWidth="0"/></map>"##
        )
    };
    let inner = d.join("B.scene.xml");
    write(
        &inner,
        &format!(
            r##"<scene version="1.2"><project width="480" height="360" fps="10" duration="1" background="#000000"/><assets>{}</assets><composition><layer id="l" asset="m"/></composition></scene>"##,
            map("m", "g").replace("SRC", "box.geojson")
        ),
    );
    let (inc, both, renamed) = including(
        &d,
        "B.scene.xml",
        "1.2",
        [480, 360],
        &format!("<assets>{}</assets>", map("pm", "g").replace("SRC", "squares.geojson")),
        r#"<layer id="pl" asset="pm" x="480"/>"#,
        "",
        &[(r#"id="g""#, r#"id="pg""#), (r#"geo="g""#, r#"geo="pg""#)],
    );
    let Some((direct, _)) = check(&inner, &inc, &both, &renamed, 0.0) else { return };
    let n = red(&direct);
    assert!(n > 100, "the include's map draws its box: {n} red pixels");
}

/// An included document's shader effect reads the node its `source` names and the image asset a `<param>` names
/// (an asset a layer of the document draws) from its own document (as `includeId/source` and `includeId/img`), even when the including document has a node
/// and an image of the same ids; the node it reads moves, and the effect follows it.
#[test]
fn an_included_effects_source_and_params_are_its_documents() {
    let d = dir("effect-refs");
    let png = |name: &str, c: [u8; 3]| {
        image::RgbaImage::from_fn(4, 4, |_, _| image::Rgba([c[0], c[1], c[2], 255])).save(d.join(name)).unwrap()
    };
    png("green.png", [0, 255, 0]);
    png("dark.png", [0, 40, 0]);
    write(
        &d.join("two.glsl"),
        "uniform sampler2D extra;\nvoid main() { vec4 a = texture(inputTexture, uv); vec4 b = texture(sourceTexture, uv); vec4 e = texture(extra, uv); fragColor = vec4(b.r, e.g, a.b, a.a); }\n",
    );
    let inner = d.join("B.scene.xml");
    write(
        &inner,
        r##"<scene version="1.1"><project width="100" height="60" fps="24" duration="2" background="#000000"/>
<assets><image id="img" src="green.png" width="4" height="4"/><generator id="red" kind="solid" width="4" height="4" paint="#FF0000"/></assets><composition>
<layer id="w" asset="red" x="0" y="10" scaleX="10" scaleY="10" visible="false"><animate property="x"><key time="0" value="0"/><key time="1" value="40"/></animate></layer>
<layer id="iv" asset="img" visible="false"/>
<shape id="r" shape="rect" x="20" y="10" width="60" height="40" fill="#0000FF" effects="s"/></composition>
<effects><effect id="s" type="shader" src="two.glsl" source="w"><param name="extra" value="img"/></effect></effects></scene>"##,
    );
    let (inc, both, renamed) = including(
        &d,
        "B.scene.xml",
        "1.1",
        [100, 60],
        r##"<assets><image id="img" src="dark.png" width="4" height="4"/><generator id="pred" kind="solid" width="4" height="4" paint="#400000"/></assets>"##,
        r#"<layer id="w" asset="pred" x="100" y="0" scaleX="25" scaleY="15" visible="false"/><layer id="pl" asset="img" x="120" y="10" scaleX="5" scaleY="5"/>"#,
        "",
        &[(r#"id="w""#, r#"id="pw""#), (r#"id="img""#, r#"id="pimg""#), (r#"asset="img""#, r#"asset="pimg""#)],
    );
    let Some((direct, _)) = check(&inner, &inc, &both, &renamed, 0.5) else { return };
    // at 0.5 s the include's red source covers x 20..60: there the shader draws red + green + blue
    let lit = direct.at(30, 30);
    assert!(lit[0] > 0.9 && lit[1] > 0.9 && lit[2] > 0.9, "the include's source and image: {lit:?}");
    let unlit = direct.at(70, 30);
    assert!(unlit[0] < 0.1 && unlit[1] > 0.9 && unlit[2] > 0.9, "outside the include's source: {unlit:?}");
}

/// A compute node of an included document reads its `src` from that document's directory, not the including
/// document's (which holds another shader of the same name).
#[test]
fn an_included_compute_reads_its_documents_shader() {
    let d = dir("compute-src");
    let shaders = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/shaders");
    std::fs::create_dir_all(d.join("sub")).unwrap();
    std::fs::copy(shaders.join("srep67-colour.wgsl"), d.join("sub/k.wgsl")).unwrap();
    // the including document's directory holds a shader of the same name that draws no point
    write(&d.join("k.wgsl"), "fn sr_point(i: u32) {\n}\n");
    let doc = |body: &str| {
        format!(
            r##"<scene version="1.6"><project width="640" height="360" fps="24" duration="1" background="#000000FF" seed="1"/><composition>{body}</composition></scene>"##
        )
    };
    let inner = d.join("sub/B.scene.xml");
    write(
        &inner,
        &doc(
            r##"<compute id="c" src="k.wgsl" width="400" height="200" invocations="100" channels="4" fractionBits="8"><tonemap ramp="#000000FF #FFFFFFFF"/></compute>"##,
        ),
    );
    let inc = d.join("A.scene.xml");
    write(&inc, &doc(r#"<include id="i" src="sub/B.scene.xml"/>"#));
    let (Some(direct), Some(through)) = (frame(&inner, 0.0), frame(&inc, 0.0)) else { return };
    let (r, b) = (direct.at(100, 100), direct.at(200, 100));
    assert!(r[0] > 0.9 && r[2] < 0.1 && b[2] > 0.9 && b[0] < 0.1, "the colour kit's red and blue: {r:?} {b:?}");
    let n = differ(&direct, &through);
    assert_eq!(n, 0, "drawn through an include, the compute node differs from the document itself in {n} pixels");
}

/// An included document's transitions read their `shader` from that document's directory and their `matte` from its
/// own nodes, beside an including document with a shader file of the same name and a node of the same id.
#[test]
fn an_included_transitions_shader_and_matte_are_its_documents() {
    let d = dir("transition");
    let sub = d.join("sub");
    write(
        &sub.join("t.glsl"),
        "vec4 transition(vec2 uv) { return mix(getFromColor(uv), getToColor(uv), progress); }\n",
    );
    write(&d.join("t.glsl"), "vec4 transition(vec2 uv) { return vec4(0.0, 1.0, 0.0, 1.0); }\n");
    let gens = r##"<generator id="red" kind="solid" width="4" height="4" paint="#FF0000"/><generator id="blue" kind="solid" width="4" height="4" paint="#0000FF"/><generator id="grey" kind="solid" width="4" height="4" paint="#808080"/>"##;
    for (what, transition) in [
        ("shader", r#"<transition type="shader" shader="t.glsl" from="a" to="b" duration="1" curve="linear"/>"#),
        ("luma", r#"<transition type="luma" matte="lm" from="a" to="b" duration="1" curve="linear"/>"#),
    ] {
        let inner = sub.join(format!("{what}.scene.xml"));
        write(
            &inner,
            &format!(
                r##"<scene version="1.1"><project width="100" height="60" fps="24" duration="4" background="#000000"/><assets>{gens}</assets><composition>
<layer id="a" asset="red" x="0" y="0" scaleX="25" scaleY="15" end="2"/>
<layer id="b" asset="blue" x="0" y="0" scaleX="25" scaleY="15" start="2"/>
<layer id="lm" asset="grey" x="0" y="0" scaleX="25" scaleY="15" visible="false"/>
{transition}</composition></scene>"##
            ),
        );
        let (inc, both, renamed) = including(
            &d,
            &format!("sub/{what}.scene.xml"),
            "1.1",
            [100, 60],
            r##"<assets><generator id="white" kind="solid" width="4" height="4" paint="#FFFFFF"/></assets>"##,
            r#"<layer id="lm" asset="white" x="100" y="0" scaleX="25" scaleY="15" visible="false"/>"#,
            "",
            &[(r#"id="lm""#, r#"id="plm""#)],
        );
        let Some((direct, _)) = check(&inner, &inc, &both, &renamed, 2.0) else { return };
        // half way through, the fade is half red, half blue (the luma wipe has passed the uniform grey or not)
        let mid = direct.at(50, 30);
        if what == "shader" {
            assert!(mid[0] > 0.2 && mid[2] > 0.2 && mid[1] < 0.1, "half way from red to blue: {mid:?}");
        }
    }
}

/// A skin deformer of an included document follows the skeleton its document declares, beside an including
/// document with a skeleton of the same id that does not move.
#[test]
fn an_included_skin_follows_its_documents_skeleton() {
    let d = dir("skin");
    let inner = d.join("B.scene.xml");
    write(
        &inner,
        r##"<scene version="1.1"><project width="64" height="64" fps="24" duration="2" background="#000000"/><assets><generator id="white" kind="solid" width="4" height="4" paint="#FFFFFF"/></assets><composition>
<skeleton id="rig"><bone id="b" x="0" y="8" length="32"><animate property="rotation"><key time="0" value="0"/><key time="1" value="90"/></animate></bone></skeleton>
<layer id="w" asset="white" x="0" y="0" scaleX="8" scaleY="4"><deform><modifier type="skin" skeleton="rig"/></deform></layer></composition></scene>"##,
    );
    let (inc, both, renamed) = including(
        &d,
        "B.scene.xml",
        "1.1",
        [64, 64],
        "",
        r##"<skeleton id="rig"><bone id="pb" x="0" y="8" length="32"/></skeleton><shape id="pl" shape="rect" x="64" y="0" width="32" height="16" fill="#FFFFFF"><deform><modifier type="skin" skeleton="rig"/></deform></shape>"##,
        "",
        &[(r#"id="rig""#, r#"id="prig""#), (r#"skeleton="rig""#, r#"skeleton="prig""#)],
    );
    let Some((direct, _)) = check(&inner, &inc, &both, &renamed, 1.0) else { return };
    // the include's bone has turned 90° about (0, 8): the layer hangs down from the joint
    assert!(direct.at(24, 8)[0] < 0.1 && direct.at(3, 24)[0] > 0.9, "{:?} {:?}", direct.at(24, 8), direct.at(3, 24));
}

/// An included document's IK constraint bends toward the pole node its document declares, and a shape that copies the
/// position of a bone follows its document's bone, beside an including document that declares a pole, a goal and a
/// skeleton with bones of the same ids (before the include, so that a lookup in document order would find them first).
#[test]
fn an_included_constraints_pole_and_bone_are_its_documents() {
    let d = dir("constraint-refs");
    let doc = |w: u32, body: &str| {
        format!(
            r##"<scene version="1.2"><project width="{w}" height="100" fps="24" duration="2" background="#000000"/><composition>{body}</composition></scene>"##
        )
    };
    let inner = d.join("B.scene.xml");
    write(
        &inner,
        &doc(
            100,
            r##"<shape id="goal" shape="ellipse" x="70" y="40" width="4" height="4" fill="#00FF00"/>
<shape id="knee" shape="ellipse" x="50" y="0" width="4" height="4" fill="#0000FF"/>
<group id="arm" x="40" y="60"><shape id="fore" shape="rect" x="20" width="20" height="4" fill="#FF0000"><transformConstraint type="ik" target="goal" pole="knee" offsetX="20"/></shape></group>
<group id="rg" x="10" y="80"><skeleton id="rig"><bone id="b" length="20" rotation="-45"/><bone id="tip" parent="b" x="20" length="5"/></skeleton></group>
<shape id="dot" shape="rect" width="6" height="6" fill="#FFFFFF"><transformConstraint type="copy-position" target="tip"/></shape>"##,
        ),
    );
    let parent = r##"<group id="rg" x="110" y="20"><skeleton id="rig"><bone id="b" length="20" rotation="45"/><bone id="tip" parent="b" x="20" length="5"/></skeleton></group>
<shape id="knee" shape="ellipse" x="150" y="96" width="4" height="4" fill="#0000FF"/>
<shape id="goal" shape="ellipse" x="180" y="10" width="4" height="4" fill="#00FF00"/>"##;
    let renames = [
        (r#"id="rig""#, r#"id="prig""#),
        (r#"id="rg""#, r#"id="prg""#),
        (r#"id="b""#, r#"id="pb""#),
        (r#"parent="b""#, r#"parent="pb""#),
        (r#"id="tip""#, r#"id="ptip""#),
        (r#"id="knee""#, r#"id="pknee""#),
        (r#"id="goal""#, r#"id="pgoal""#),
    ];
    let renamed_parent = renames.iter().fold(parent.to_string(), |s, (from, to)| s.replace(from, to));
    let include = r#"<include id="i" src="B.scene.xml"/>"#;
    let (inc, both, renamed) = (d.join("inc.scene.xml"), d.join("both.scene.xml"), d.join("renamed.scene.xml"));
    write(&inc, &doc(100, include));
    write(&both, &doc(200, &format!("{parent}{include}")));
    write(&renamed, &doc(200, &format!("{renamed_parent}{include}")));
    let Some((direct, _)) = check(&inner, &inc, &both, &renamed, 0.5) else { return };
    assert!(red(&direct) > 20, "the include's forearm is drawn: {} red pixels", red(&direct));
}

/// A camera of an included document whose `target` names a node of its document that lies inside a repeat (so that
/// no node of that id is in the camera's scope) has no target, as in the document by itself: the including document's
/// node of that id is not looked at instead.
#[test]
fn an_included_cameras_target_does_not_reach_the_including_document() {
    let d = dir("camera-target");
    let inner = d.join("B.scene.xml");
    write(
        &inner,
        &format!(
            r##"<scene version="1.6"><project width="200" height="100" fps="24" duration="2" background="#000000"/><materials><material id="m" unlit="true" baseColor="#FF0000"/></materials><composition><repeat id="r" count="1"><shape id="x" shape="rect" width="2" height="2" fill="#000000"/></repeat>{}</composition></scene>"##,
            sphere_viewport("v", 0, "m").replace(r#"z="-173.205"/>"#, r#"z="-173.205" target="x"/>"#)
        ),
    );
    let doc = |body: &str| {
        format!(
            r##"<scene version="1.6"><project width="200" height="100" fps="24" duration="2" background="#000000"/><composition>{body}</composition></scene>"##
        )
    };
    let include = r#"<include id="i" src="B.scene.xml"/>"#;
    let (alone, beside) = (d.join("A.scene.xml"), d.join("P.scene.xml"));
    write(&alone, &doc(include));
    write(
        &beside,
        &doc(&format!(r##"<shape id="x" shape="rect" x="190" y="5" width="2" height="2" fill="#000000"/>{include}"##)),
    );
    let (Some(direct), Some(a), Some(p)) = (frame(&inner, 0.5), frame(&alone, 0.5), frame(&beside, 0.5)) else {
        return;
    };
    assert!(red(&direct) > 100, "the camera looks ahead at the sphere: {} red pixels", red(&direct));
    let n = differ(&direct, &a);
    assert_eq!(n, 0, "drawn through an include, the view differs from the document itself in {n} pixels");
    let n = differ(&direct, &p);
    assert_eq!(n, 0, "beside a node of the target's id, the include's camera turned toward it ({n} pixels differ)");
}
