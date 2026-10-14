//! Byte-level targets: the parsers that read files a document points at.
//!
//! Each [`Target`] takes arbitrary bytes and must never panic: captions
//! (SRT, WebVTT, ASS, TTML, SCC, transcripts), Lottie JSON and dotLottie
//! archives, SVG and path data, formulas, text layout, PMTiles, vector
//! tiles, GeoJSON/TopoJSON/KML/GPX, PLY and splats, USD, glTF, the document
//! pipeline with asset verification on, LUTs (.cube, .3dl, .clf), ICC
//! profiles and the colour boxes of HEIF/AVIF files, OpenVDB volumes, SRVOL
//! simulation caches, MagicaVoxel VOX models, font files, Hershey stroke
//! fonts and IES photometric profiles. Inputs are built-in seeds
//! and the repository's fixtures, mutated by bit flips, boundary bytes and
//! integers, span deletion and duplication, splices between seeds, inserted
//! number tokens and truncation; text formats also get the document
//! mutations of [`crate::mutate_doc`].

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use sr_text::layout::{self, Align, AutoFit, Dir, Opts, Overflow, Para, Run, Wrap, Writing};
use sr_vector::Paint;

use crate::Rng;

/// Inputs are cut to this length, so a campaign's speed does not drift with duplicated spans.
pub const MAX_INPUT: usize = 64 << 10;

/// A parser under test.
pub struct Target {
    pub name: &'static str,
    /// Text formats also take the document mutations.
    pub text: bool,
    pub seeds: fn() -> Vec<Vec<u8>>,
    pub run: fn(&[u8]),
}

/// Every byte-level target.
pub const TARGETS: &[Target] = &[
    Target { name: "captions", text: true, seeds: seeds_captions, run: run_captions },
    Target { name: "lottie", text: true, seeds: seeds_lottie, run: run_lottie },
    Target { name: "dotlottie", text: false, seeds: seeds_dotlottie, run: run_lottie },
    Target { name: "svg", text: true, seeds: seeds_svg, run: run_svg },
    Target { name: "path", text: true, seeds: seeds_path, run: run_path },
    Target { name: "formula", text: true, seeds: seeds_formula, run: run_formula },
    Target { name: "text", text: true, seeds: seeds_text, run: run_text },
    Target { name: "pmtiles", text: false, seeds: seeds_pmtiles, run: run_pmtiles },
    Target { name: "mvt", text: false, seeds: seeds_mvt, run: run_mvt },
    Target { name: "geodata", text: true, seeds: seeds_geodata, run: run_geodata },
    Target { name: "ply", text: false, seeds: seeds_ply, run: run_ply },
    Target { name: "usd", text: false, seeds: seeds_usd, run: run_usd },
    Target { name: "gltf", text: false, seeds: seeds_gltf, run: run_gltf },
    Target { name: "document-assets", text: true, seeds: seeds_documents, run: run_doc_assets },
    Target { name: "lut", text: true, seeds: seeds_lut, run: run_lut },
    Target { name: "icc", text: false, seeds: seeds_icc, run: run_icc },
    Target { name: "openvdb", text: false, seeds: seeds_openvdb, run: run_openvdb },
    Target { name: "srvol", text: false, seeds: seeds_srvol, run: run_srvol },
    Target { name: "vox", text: false, seeds: seeds_vox, run: run_vox },
    Target { name: "font", text: false, seeds: seeds_font, run: run_font },
    Target { name: "stroke-font", text: true, seeds: seeds_stroke_font, run: run_stroke_font },
    Target { name: "ies", text: true, seeds: seeds_ies, run: run_ies },
];

/// The target called `name`.
pub fn target(name: &str) -> Option<&'static Target> {
    TARGETS.iter().find(|t| t.name == name)
}

fn repo(rel: &str) -> PathBuf {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).join(rel)
}

/// Built-in seeds plus the repository files that exist and fit [`MAX_INPUT`].
fn with_files(built_in: &[&[u8]], files: &[&str]) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = built_in.iter().map(|s| s.to_vec()).collect();
    out.extend(files.iter().filter_map(|f| std::fs::read(repo(f)).ok()).filter(|b| b.len() <= MAX_INPUT));
    out
}

// ------------------------------------------------------------------ captions

const SRT: &str = "1\n00:00:01,000 --> 00:00:02,500\n<i>Hello</i> &amp; world\nsecond line\n\n2\n00:00:03,000 --> 00:00:04,000 X1:10\nBye\n";
const VTT: &str = "WEBVTT\n\nintro\n00:00.000 --> 00:02.000 line:90% align:center\n<v Ana>Hello <00:00:00.500>there <00:00:01.000><b>you</b>\n\n00:00:02.000 --> 00:00:03.000\nplain &lt;text&gt;\n";
const ASS: &str = "[Script Info]\nTitle: t\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:03.00,Default,Ana,0,0,0,,{\\k50}Hel{\\kf30}lo {\\ko20\\b1}wor{\\i1}ld\\Nnext, line\nDialogue: 0,0:00:03.00,0:00:04.00,Top,,0,0,0,,{\\an8}plain\n";
const TTML: &str = r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttp="http://www.w3.org/ns/ttml#parameter" ttp:frameRate="24" ttp:frameRateMultiplier="1000 1001" ttp:tickRate="10000000"><body><div><p begin="00:00:01:12" end="2s" style="s1" agent="ana">Hello<br/><span begin="0.5s" end="1s">timed</span> text</p><p begin="3000ms" dur="48f">Later</p><p begin="50000000t">Ticks</p></div></body></tt>"#;
const SCC: &str = "Scenarist_SCC V1.0\n\n00:00:01:00\t9420 9420 94ae 94ae 9452 9452 c8e5 ecec ef20 f7ef f2ec 6480 942c 942c 942f 942f\n\n00:00:03;15\t9425 9425 94ad 94ad c1b0 1130 9137 942c\n";
const TRANSCRIPT: &str = r#"{"segments":[{"start":0,"end":1.5,"text":" Hello there","speaker":"a","words":[{"start":0,"end":0.5,"word":" Hello"},{"start":0.5,"end":1.5,"text":"there","emphasis":true}]}],"words":[{"start":0,"end":1,"text":"x"}]}"#;

fn seeds_captions() -> Vec<Vec<u8>> {
    with_files(
        &[SRT.as_bytes(), VTT.as_bytes(), ASS.as_bytes(), TTML.as_bytes(), SCC.as_bytes(), TRANSCRIPT.as_bytes()],
        &["tests/corpus/media/transcript.vtt"],
    )
}

/// Every caption parser on `data`, then the cue consumers.
pub fn run_captions(data: &[u8]) {
    use sr_text::captions;
    let text = String::from_utf8_lossy(data);
    let mut all = Vec::new();
    for format in ["srt", "vtt", "ass", "ttml", "itt", "scc"] {
        if let Ok(cues) = captions::parse(&text, format) {
            all.extend(cues);
        }
    }
    if let Ok(cues) = captions::from_transcript(&text) {
        all.extend(cues);
    }
    all.truncate(64);
    for c in &all {
        let _ = captions::timed_words(c);
        let _ = captions::profanity(&c.text);
    }
    let _ = captions::paginate(&all, Some(3), 16, 2, false);
    let _ = captions::paginate(&all, None, 1, 1, true);
    let _ = captions::to_srt(&all);
    let _ = captions::to_vtt(&all);
}

// ------------------------------------------------------------------ Lottie

const LOTTIE: &str = r##"{"v":"5.7.0","fr":30,"ip":0,"op":60,"w":100,"h":100,
"markers":[{"cm":"intro","tm":10,"dr":20}],
"slots":{"c":{"p":{"a":0,"k":[0,0,1,1]}}},
"assets":[{"id":"pre","layers":[{"ty":1,"ind":1,"ip":0,"op":60,"sw":20,"sh":20,"sc":"#ff8800","ks":{}},{"ty":0,"refId":"pre2","ip":0,"op":60,"w":20,"h":20,"ks":{}}]},{"id":"pre2","layers":[{"ty":0,"refId":"pre","ip":0,"op":60,"ks":{}}]}],
"layers":[
{"ty":4,"ind":2,"parent":1,"ip":0,"op":60,"st":0,"ks":{"p":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]},"r":{"a":0,"k":0},"o":{"a":0,"k":100}},
 "masksProperties":[{"mode":"a","inv":false,"o":{"a":0,"k":100},"pt":{"a":0,"k":{"c":true,"v":[[0,0],[50,0],[50,50]],"i":[[0,0],[0,0],[0,0]],"o":[[0,0],[0,0],[0,0]]}}}],
 "shapes":[{"ty":"gr","it":[
  {"ty":"rc","p":{"a":0,"k":[10,10]},"s":{"a":0,"k":[20,20]},"r":{"a":0,"k":2}},
  {"ty":"el","p":{"a":0,"k":[40,40]},"s":{"a":0,"k":[20,10]}},
  {"ty":"sr","sy":1,"p":{"a":0,"k":[60,60]},"pt":{"a":0,"k":5},"r":{"a":0,"k":0},"or":{"a":0,"k":10},"ir":{"a":0,"k":4},"os":{"a":0,"k":0},"is":{"a":0,"k":0}},
  {"ty":"sh","ks":{"a":1,"k":[{"t":0,"s":[{"c":false,"v":[[0,0],[30,0]],"i":[[0,0],[0,0]],"o":[[0,0],[0,0]]}],"o":{"x":0.3,"y":0},"i":{"x":0.7,"y":1}},{"t":30,"s":[{"c":false,"v":[[0,0],[30,30]],"i":[[0,0],[0,0]],"o":[[0,0],[0,0]]}]}]}},
  {"ty":"tm","s":{"a":0,"k":10},"e":{"a":0,"k":90},"o":{"a":0,"k":45},"m":2},
  {"ty":"rd","r":{"a":0,"k":3}},
  {"ty":"zz","s":{"a":0,"k":2},"r":{"a":0,"k":3},"pt":{"a":0,"k":2}},
  {"ty":"pb","a":{"a":0,"k":10}},
  {"ty":"tw","a":{"a":0,"k":30}},
  {"ty":"op","a":{"a":0,"k":2},"lj":2,"ml":{"a":0,"k":4}},
  {"ty":"rp","c":{"a":0,"k":3},"o":{"a":0,"k":0},"m":1,"tr":{"p":{"a":0,"k":[5,0]},"s":{"a":0,"k":[90,90]},"r":{"a":0,"k":10},"so":{"a":0,"k":100},"eo":{"a":0,"k":50}}},
  {"ty":"mm","mm":3},
  {"ty":"fl","c":{"sid":"c","a":0,"k":[1,0,0,1]},"o":{"a":0,"k":100},"r":2},
  {"ty":"st","c":{"a":0,"k":[0,0,0,1]},"o":{"a":0,"k":100},"w":{"a":0,"k":2},"lc":2,"lj":2,"ml":4,"d":[{"n":"d","v":{"a":0,"k":4}},{"n":"g","v":{"a":0,"k":2}},{"n":"o","v":{"a":0,"k":1}}]},
  {"ty":"gf","o":{"a":0,"k":100},"s":{"a":0,"k":[0,0]},"e":{"a":0,"k":[100,0]},"t":2,"h":{"a":0,"k":50},"a":{"a":0,"k":0},"g":{"p":2,"k":{"a":0,"k":[0,1,0,0,1,0,0,1,0,1,1,0.5]}}},
  {"ty":"gs","o":{"a":0,"k":100},"w":{"a":0,"k":1},"s":{"a":0,"k":[0,0]},"e":{"a":0,"k":[100,0]},"t":1,"g":{"p":2,"k":{"a":0,"k":[0,1,0,0,1,0,0,1]}}},
  {"ty":"tr","p":{"a":0,"k":[0,0]},"a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]},"r":{"a":0,"k":0},"o":{"a":0,"k":100},"sk":{"a":0,"k":5},"sa":{"a":0,"k":10}}]}]},
{"ty":3,"ind":1,"ip":0,"op":60,"ks":{"p":{"s":true,"x":{"a":0,"k":5},"y":{"a":1,"k":[{"t":0,"s":[0],"h":1},{"t":30,"s":[60],"to":[1,1],"ti":[2,2]},{"t":50}]}}}},
{"ty":0,"ind":4,"refId":"pre","ip":0,"op":60,"st":5,"sr":2,"w":50,"h":50,"tt":1,"tp":2,"tm":{"a":0,"k":0.5},"ks":{"o":{"a":0,"k":50}}},
{"ty":1,"ind":5,"td":1,"ip":0,"op":60,"sw":100,"sh":100,"sc":"#00ff00","ks":{}}]}"##;

fn seeds_lottie() -> Vec<Vec<u8>> {
    with_files(&[LOTTIE.as_bytes()], &["tests/corpus/media/intro.json"])
}

/// A zip of stored entries.
fn zip_stored(files: &[(&str, &[u8])]) -> Vec<u8> {
    let (mut out, mut central) = (Vec::new(), Vec::new());
    for (name, data) in files {
        let off = out.len() as u32;
        let sizes = |v: &mut Vec<u8>| {
            v.extend([0u8; 4]);
            v.extend((data.len() as u32).to_le_bytes());
            v.extend((data.len() as u32).to_le_bytes());
            v.extend((name.len() as u16).to_le_bytes());
        };
        out.extend(0x0403_4b50u32.to_le_bytes());
        out.extend([20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        sizes(&mut out);
        out.extend([0, 0]);
        out.extend(name.as_bytes());
        out.extend(*data);
        central.extend(0x0201_4b50u32.to_le_bytes());
        central.extend([20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        sizes(&mut central);
        central.extend([0u8; 12]);
        central.extend(off.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let cd = out.len() as u32;
    out.extend(&central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0, 0, 0, 0]);
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((central.len() as u32).to_le_bytes());
    out.extend(cd.to_le_bytes());
    out.extend([0, 0]);
    out
}

fn seeds_dotlottie() -> Vec<Vec<u8>> {
    let small = br##"{"fr":30,"ip":0,"op":10,"w":10,"h":10,"layers":[{"ty":1,"ip":0,"op":10,"sw":5,"sh":5,"sc":"#102030","ks":{}}]}"##;
    vec![
        zip_stored(&[("manifest.json", b"{}"), ("animations/a.json", LOTTIE.as_bytes())]),
        zip_stored(&[("a/one.json", small), ("a/two.json", small), ("images/x.png", b"\x89PNG")]),
    ]
}

/// Parses Lottie JSON or a dotLottie archive and renders three frames.
pub fn run_lottie(data: &[u8]) {
    use sr_vector::lottie::Lottie;
    let _ = Lottie::parse(data, Some("a"), &[]);
    let overrides = [("c".to_string(), "#ff000080".to_string()), ("d".to_string(), "1, 2".to_string())];
    if let Ok(l) = Lottie::parse(data, None, &overrides) {
        let _ = l.segment("intro");
        let _ = l.segment("1,2");
        for f in [l.ip, l.frame_at(0.5, None), l.op - 1.0] {
            let _ = l.render(f, 0.5);
        }
    }
}

// ------------------------------------------------------------------ SVG and paths

const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="100" height="50" viewBox="0 0 200 100">
<defs><linearGradient id="g" gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="200" y2="0"><stop offset="0" stop-color="#f00"/><stop offset="1" stop-color="#00f" stop-opacity="0.5"/></linearGradient>
<radialGradient id="r" cx="50%" cy="50%" r="50%" fx="30%"><stop offset="0" stop-color="white"/><stop offset="1" stop-color="black"/></radialGradient>
<clipPath id="c"><rect x="0" y="0" width="100" height="100" rx="5"/></clipPath><mask id="m"><circle cx="50" cy="50" r="40" fill="#fff"/></mask>
<symbol id="s" viewBox="0 0 10 10"><path d="M0 0h10v10z"/></symbol></defs>
<rect x="0" y="0" width="200" height="100" fill="url(#r)"/>
<g clip-path="url(#c)" opacity="0.5" transform="translate(10 5) rotate(15) scale(1.2, 0.8) skewX(5)"><ellipse cx="50" cy="50" rx="30" ry="20" fill="#ff0000" mask="url(#m)"/></g>
<path d="M10 10 L190 10 C150 40 100 40 60 90 Q30 60 10 90 A20 30 45 1 0 80 80 Z" fill="none" stroke="url(#g)" stroke-width="4" stroke-dasharray="10 5" stroke-dashoffset="3" stroke-linecap="round" stroke-linejoin="bevel"/>
<polygon points="10,10 20,30 30,10" fill-rule="evenodd"/><polyline points="1 2 3 4 5 6"/><line x1="0" y1="0" x2="5" y2="5" stroke="blue"/>
<use xlink:href="#s" x="20" y="20" width="30" height="30"/><text x="10" y="20" font-size="12">Hi</text></svg>"##;

fn seeds_svg() -> Vec<Vec<u8>> {
    vec![SVG.as_bytes().to_vec(), br#"<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0L1 1"/></svg>"#.to_vec()]
}

/// Loads an SVG document.
pub fn run_svg(data: &[u8]) {
    let _ = sr_vector::svg::load(data, 0.5);
}

fn seeds_path() -> Vec<Vec<u8>> {
    [
        "M10 10 L190 10 C150 40 100 40 60 90 S20 20 5 5 Q30 60 10 90 T50 50 A20 30 45 1 0 80 80 Z",
        "m1,2 l3-4 h5 v-6 c1 2 3 4 5 6 s1 2 3 4 q1 2 3 4 t5 6 a1 2 3 0 1 4 5 z m0 0",
        "M.5.5 1e2-3E-1 L 1e308 -1e308 A0 0 0 0 0 1 1 a1,1,0,11,2,2",
        "M0 0 H10 V10 H0 Z M2 2 L8 2 8 8 2 8 z",
    ]
    .iter()
    .map(|s| s.as_bytes().to_vec())
    .collect()
}

/// Parses path data and runs the path operations on it.
pub fn run_path(data: &[u8]) {
    use sr_vector::geom::Xf;
    if let Ok(p) = sr_vector::Path::parse(&String::from_utf8_lossy(data)) {
        let _ = p.bounds();
        let cs = p.contours();
        let _ = sr_vector::Path::from_contours(&cs);
        let polys = p.transform(&Xf::rotate(30.0)).flatten(0.5);
        let _ = sr_vector::measure::dash(&polys, &[3.0, 1.0], 0.5);
        let _ = sr_vector::measure::trim(&polys, 0.1, 0.7, 0.3, sr_vector::measure::TrimMode::Sequential);
    }
}

// ------------------------------------------------------------------ text

fn fonts() -> MutexGuard<'static, sr_text::FontLib> {
    static LIB: OnceLock<Mutex<sr_text::FontLib>> = OnceLock::new();
    LIB.get_or_init(|| Mutex::new(sr_text::FontLib::new(true))).lock().unwrap_or_else(|e| e.into_inner())
}

fn seeds_formula() -> Vec<Vec<u8>> {
    [
        r"x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}",
        r"\sum_{i=1}^{n} i^2 = \left( \frac{n(n+1)(2n+1)}{6} \right) \quad \int_0^\infty e^{-x}\,dx",
        r"\sqrt[3]{x_1^2 + \alpha'} \leq \lim_{n \to \infty} \text{rate} \cdot \mathrm{d}t \left\{ a \right.",
        r"a^{b^{c_d}} \{ x \} \langle y \rangle \; \! \sin\theta",
    ]
    .iter()
    .map(|s| s.as_bytes().to_vec())
    .collect()
}

/// Typesets a formula (parsing only when no MATH font is installed).
pub fn run_formula(data: &[u8]) {
    let tex = String::from_utf8_lossy(data);
    let paint = Paint::Solid { rgba: [0.0, 0.0, 0.0, 1.0], srgb: true };
    let _ = sr_text::formula::draw(&mut fonts(), &tex, 24.0, [200.0, 100.0], paint, 0.5);
}

fn seeds_text() -> Vec<Vec<u8>> {
    [
        "\u{1}\u{2}\u{3}\u{4}Hello world, extraordinarily hyphenatable incomprehensibilities again and again",
        "\u{13}\u{21}\u{32}\u{40}مرحبا بالعالم שלום עולם mixed ‏RTL‎ and LTR 123",
        "\u{25}\u{16}\u{07}\u{38}日本語のテキスト、縦書き。😀👨‍👩‍👧 e\u{301} İstanbul\u{ad}soft\u{200b}zero\ttab\nline\r\nbreak",
        "\u{ff}\u{80}\u{7f}\u{0}ﬁ ﬂ office AVATAR To. 1/2 --- x²",
    ]
    .iter()
    .map(|s| s.as_bytes().to_vec())
    .collect()
}

/// Lays out and draws text; the first four bytes pick the box and flow options.
pub fn run_text(data: &[u8]) {
    let (head, body) = data.split_at(data.len().min(4));
    let b = |k: usize| head.get(k).copied().unwrap_or(0) as usize;
    let text: String = String::from_utf8_lossy(body).chars().take(400).collect();
    let opts = Opts {
        width: [f64::INFINITY, 200.0, 40.0, 1.0, 0.0, 1e6][b(0) % 6],
        height: [f64::INFINITY, 100.0, 10.0, 0.0][b(0) / 6 % 4],
        align: [Align::Start, Align::Center, Align::End, Align::Justify][b(1) % 4],
        direction: [Dir::Auto, Dir::Ltr, Dir::Rtl][b(1) / 4 % 3],
        writing: [Writing::Horizontal, Writing::VerticalRl, Writing::VerticalLr][b(1) / 12 % 3],
        wrap: [Wrap::Word, Wrap::Character, Wrap::None, Wrap::Balance][b(2) % 4],
        hyphenate: b(2) / 4 % 2 == 1,
        auto_fit: [AutoFit::None, AutoFit::Shrink, AutoFit::Grow, AutoFit::Fit][b(2) / 8 % 4],
        max_lines: [None, Some(1), Some(3)][b(2) / 32 % 3],
        overflow: [Overflow::Visible, Overflow::Ellipsis, Overflow::Clip][b(3) % 3],
        letter_spacing: [0.0, 2.0, -1.0][b(3) / 3 % 3],
        line_height: [1.2, 0.5, 3.0][b(3) / 9 % 3],
        ..Default::default()
    };
    let mid = text.char_indices().nth(text.chars().count() / 2).map_or(0, |c| c.0);
    let style = |size: f64, italic: bool| sr_text::Style { size, italic, ..Default::default() };
    let para = Para {
        runs: vec![
            Run { text: text[..mid].to_string(), style: 0, role: None },
            Run { text: text[mid..].to_string(), style: 1, role: Some("em".into()) },
        ],
        styles: vec![style(24.0, false), style([8.0, 64.0, 0.5][b(3) / 27 % 3], true)],
        opts,
    };
    let mut lib = fonts();
    let lay = layout::layout(&mut lib, &para);
    let _ = sr_text::glyph::draw(&lib, &lay, None, &Default::default(), 0.5);
}

// ------------------------------------------------------------------ maps

fn varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn pb_bytes(out: &mut Vec<u8>, field: u64, b: &[u8]) {
    varint(out, field << 3 | 2);
    varint(out, b.len() as u64);
    out.extend(b);
}

fn pb_varint(out: &mut Vec<u8>, field: u64, v: u64) {
    varint(out, field << 3);
    varint(out, v);
}

/// A vector tile: one layer with a line and a polygon, two keys and typed values.
fn mvt_tile() -> Vec<u8> {
    let feature = |id: u64, kind: u64, tags: &[u8], geom: &[u8]| {
        let mut f = Vec::new();
        pb_varint(&mut f, 1, id);
        pb_bytes(&mut f, 2, tags);
        pb_varint(&mut f, 3, kind);
        pb_bytes(&mut f, 4, geom);
        f
    };
    let mut layer = Vec::new();
    pb_varint(&mut layer, 15, 2);
    pb_bytes(&mut layer, 1, b"roads");
    pb_bytes(&mut layer, 2, &feature(1, 2, &[0, 0, 1, 1], &[9, 4, 4, 18, 10, 0, 0, 10]));
    pb_bytes(&mut layer, 2, &feature(2, 3, &[0, 2], &[9, 0, 0, 26, 20, 0, 0, 20, 19, 0, 15]));
    pb_bytes(&mut layer, 2, &feature(3, 1, &[1, 3], &[17, 2, 2, 6, 6]));
    pb_bytes(&mut layer, 3, b"kind");
    pb_bytes(&mut layer, 3, b"lanes");
    pb_bytes(&mut layer, 4, &[10, 4, b'r', b'o', b'a', b'd']);
    pb_bytes(&mut layer, 4, &[40, 2]);
    pb_bytes(&mut layer, 4, &[25, 0, 0, 0, 0, 0, 0, 0xf0, 0x3f]);
    pb_bytes(&mut layer, 4, &[56, 1]);
    pb_varint(&mut layer, 5, 4096);
    let mut tile = Vec::new();
    pb_bytes(&mut tile, 3, &layer);
    tile
}

fn seeds_mvt() -> Vec<Vec<u8>> {
    vec![mvt_tile()]
}

/// Decodes a vector tile and groups its polygons.
pub fn run_mvt(data: &[u8]) {
    if let Ok(layers) = sr_geo::mvt::decode(data) {
        for f in layers.iter().flat_map(|l| &l.features) {
            let _ = f.polygons();
        }
    }
}

fn seeds_pmtiles() -> Vec<Vec<u8>> {
    use sr_geo::pmtiles::{write, TileType};
    let tile = mvt_tile();
    let tiles = [((0, 0, 0), tile.clone()), ((1, 0, 0), tile.clone()), ((1, 1, 0), tile.clone()), ((1, 1, 1), vec![1])];
    let mut out = vec![write(&tiles, TileType::Mvt, 1, &serde_json::json!({"vector_layers": [{"id": "roads"}]}))];
    out.extend(with_files(&[], &["tests/corpus/media/streets.pmtiles", "crates/sr-geo/tests/fixtures/raster.pmtiles"]));
    out
}

/// A file holding one input, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    /// Writes `data` under `$SR_FUZZ_SCRATCH`, or the workspace's `target/sr-fuzz`.
    fn new(name: &str, data: &[u8]) -> Option<Scratch> {
        let dir = std::env::var_os("SR_FUZZ_SCRATCH").map(PathBuf::from).unwrap_or_else(|| repo("target/sr-fuzz"));
        std::fs::create_dir_all(&dir).ok()?;
        let path = dir.join(format!("{}-{name}", std::process::id()));
        std::fs::write(&path, data).ok()?;
        Some(Scratch(path))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Reads a PMTiles header and directory from memory, then opens the bytes as an archive.
pub fn run_pmtiles(data: &[u8]) {
    use sr_geo::pmtiles;
    let _ = pmtiles::parse_directory(data);
    let _ = pmtiles::decompress(2, data.to_vec());
    let Ok(h) = pmtiles::Header::parse(data) else { return };
    let Some(file) = Scratch::new("in.pmtiles", data) else { return };
    if let Ok(a) = pmtiles::Archive::open(&file.0) {
        for z in [0, h.min_zoom.min(20), h.max_zoom.min(20)] {
            let n = 1u32 << z;
            for (x, y) in [(0, 0), (n / 2, n / 2), (n - 1, n - 1)] {
                if let Ok(Some(t)) = a.tile(z, x, y) {
                    run_mvt(&t);
                }
            }
        }
    }
}

const GEOJSON: &str = r#"{"type":"FeatureCollection","features":[
{"type":"Feature","id":7,"properties":{"name":"a","n":1.5,"b":true,"z":null},"geometry":{"type":"Polygon","coordinates":[[[0,0],[10,0],[10,10],[0,10],[0,0]],[[2,2],[2,4],[4,4],[2,2]]]}},
{"type":"Feature","properties":{},"geometry":{"type":"MultiLineString","coordinates":[[[170,0],[-170,5]],[[0,89],[0,-89]]]}},
{"type":"Feature","properties":null,"geometry":{"type":"GeometryCollection","geometries":[{"type":"Point","coordinates":[1,2,3]},{"type":"MultiPoint","coordinates":[[1,2],[3,4]]},{"type":"MultiPolygon","coordinates":[[[[0,0],[1,0],[1,1],[0,0]]]]},{"type":"LineString","coordinates":[[0,0],[1,1]]}]}},
{"type":"Feature","geometry":null}]}"#;
const TOPOJSON: &str = r#"{"type":"Topology","transform":{"scale":[0.01,0.01],"translate":[-10,-10]},
"objects":{"land":{"type":"GeometryCollection","geometries":[{"type":"Polygon","id":"p","properties":{"name":"x"},"arcs":[[0,-2]]},{"type":"MultiPolygon","arcs":[[[0],[1]]]},{"type":"LineString","arcs":[0,1]},{"type":"MultiLineString","arcs":[[-1]]},{"type":"Point","coordinates":[100,200]},{"type":"MultiPoint","coordinates":[[1,2]]}]},"one":{"type":"Polygon","arcs":[[1]]}},
"arcs":[[[0,0],[1000,0],[0,1000],[-1000,0],[0,-1000]],[[200,200],[100,0],[0,100],[-100,-100]]]}"#;
const KML: &str = r#"<kml xmlns="http://www.opengis.net/kml/2.2"><Document><Folder><Placemark id="a"><name>Spot</name><description>d</description><ExtendedData><Data name="k"><value>v</value></Data></ExtendedData><Point><coordinates>-9.1,38.7,0</coordinates></Point></Placemark>
<Placemark><name>Area</name><MultiGeometry><Polygon><outerBoundaryIs><LinearRing><coordinates>0,0 10,0 10,10 0,0</coordinates></LinearRing></outerBoundaryIs><innerBoundaryIs><LinearRing><coordinates>2,1 3,1 3,2 2,1</coordinates></LinearRing></innerBoundaryIs></Polygon><LineString><coordinates>0,0,5
1,1,5</coordinates></LineString></MultiGeometry></Placemark></Folder></Document></kml>"#;
const GPX: &str = r#"<gpx version="1.1" xmlns="http://www.topografix.com/GPX/1/1"><wpt lat="38.7" lon="-9.1"><ele>10</ele><name>W</name></wpt><rte><name>R</name><rtept lat="1" lon="2"/><rtept lat="3" lon="4"/></rte><trk><name>T</name><trkseg><trkpt lat="38.7" lon="-9.1"><ele>5</ele><time>2020-01-01T00:00:00Z</time></trkpt><trkpt lat="38.8" lon="-9.2"/></trkseg><trkseg><trkpt lat="0" lon="179.9"/><trkpt lat="0" lon="-179.9"/></trkseg></trk></gpx>"#;

fn seeds_geodata() -> Vec<Vec<u8>> {
    with_files(
        &[GEOJSON.as_bytes(), TOPOJSON.as_bytes(), KML.as_bytes(), GPX.as_bytes()],
        &["tests/corpus/media/places.geojson"],
    )
}

/// Reads `data` as each of GeoJSON, TopoJSON, KML and GPX.
pub fn run_geodata(data: &[u8]) {
    use sr_geo::data::{parse, Format};
    let text = String::from_utf8_lossy(data);
    for format in [Format::GeoJson, Format::TopoJson, Format::Kml, Format::Gpx] {
        let _ = parse(&text, format, None);
        let _ = parse(&text, format, Some("land"));
    }
}

// ------------------------------------------------------------------ 3D

fn seeds_ply() -> Vec<Vec<u8>> {
    let ascii = b"ply\nformat ascii 1.0\ncomment made by hand\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nproperty uchar red\nproperty uchar green\nproperty uchar blue\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0 255 0 0\n1 0 0 0 255 0\n0 1 0 0 0 255\n3 0 1 2\n".to_vec();
    let floats = |v: &[f32]| v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>();
    let mut binary = b"ply\nformat binary_little_endian 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nproperty float nx\nproperty float ny\nproperty float nz\nproperty float s\nproperty float t\nelement face 1\nproperty list uchar uint vertex_indices\nend_header\n".to_vec();
    binary.extend(floats(&[0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0]));
    binary.extend(floats(&[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0]));
    binary.extend(floats(&[0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0]));
    binary.push(3);
    binary.extend([0u32, 1, 2].iter().flat_map(|i| i.to_le_bytes()));
    let mut big = b"ply\nformat binary_big_endian 1.0\nelement vertex 1\nproperty double x\nproperty double y\nproperty double z\nproperty short q\nend_header\n".to_vec();
    big.extend([1.0f64, 2.0, 3.0].iter().flat_map(|f| f.to_be_bytes()));
    big.extend([0, 7]);
    let names = [
        "x", "y", "z", "f_dc_0", "f_dc_1", "f_dc_2", "opacity", "scale_0", "scale_1", "scale_2", "rot_0", "rot_1",
        "rot_2", "rot_3",
    ];
    let mut gauss = b"ply\nformat binary_little_endian 1.0\nelement vertex 2\n".to_vec();
    for n in names {
        gauss.extend(format!("property float {n}\n").bytes());
    }
    gauss.extend(b"end_header\n");
    for k in 0..2 {
        gauss.extend(floats(&[k as f32, 0.5, -0.5, 0.1, 0.2, 0.3, 2.0, -1.0, -1.0, -1.0, 1.0, 0.0, 0.0, 0.0]));
    }
    let mut splat = Vec::new();
    for k in 0..3u8 {
        splat.extend(floats(&[k as f32, 1.0, 2.0, 0.1, 0.1, 0.1]));
        splat.extend([255, 128, k, 200, 255, 128, 128, 128]);
    }
    vec![ascii, binary, big, gauss, splat]
}

/// Imports PLY meshes and Gaussian splats, and `.splat` records.
pub fn run_ply(data: &[u8]) {
    let _ = sr_3d::import::ply(data);
    let _ = sr_3d::import::splat(data);
}

fn seeds_usd() -> Vec<Vec<u8>> {
    let usda = b"#usda 1.0\n(\n    upAxis = \"Z\"\n    metersPerUnit = 0.01\n)\ndef Xform \"root\"\n{\n    double3 xformOp:translate = (1, 2, 3)\n    uniform token[] xformOpOrder = [\"xformOp:translate\"]\n    def Mesh \"tri\"\n    {\n        int[] faceVertexCounts = [3]\n        int[] faceVertexIndices = [0, 1, 2]\n        point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]\n        normal3f[] normals = [(0, 0, 1), (0, 0, 1), (0, 0, 1)]\n        texCoord2f[] primvars:st = [(0, 0), (1, 0), (0, 1)]\n        rel material:binding = </root/mat>\n    }\n    def Material \"mat\"\n    {\n        def Shader \"s\"\n        {\n            uniform token info:id = \"UsdPreviewSurface\"\n            color3f inputs:diffuseColor = (1, 0.5, 0)\n            float inputs:roughness = 0.4\n        }\n    }\n}\n";
    with_files(
        &[usda],
        &[
            "crates/sr-3d/tests/fixtures/values.usdc",
            "crates/sr-3d/tests/fixtures/stage.usdc",
            "crates/sr-3d/tests/fixtures/stage.usdz",
            "crates/sr-3d/tests/fixtures/stage.usda",
        ],
    )
}

/// Reads a USDC crate from memory, then imports the bytes as a USD file (USDA, USDC or USDZ).
pub fn run_usd(data: &[u8]) {
    let _ = sr_3d::usdc::read(data);
    if let Some(file) = Scratch::new("in.usd", data) {
        let _ = sr_3d::import::usd(&file.0);
    }
}

const GLTF: &str = r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
"nodes":[{"name":"tri","mesh":0,"translation":[1,2,3],"rotation":[0,0,0,1],"scale":[1,1,1],"children":[1]},{"matrix":[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1]}],
"meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1,"material":0,"mode":4}]}],
"materials":[{"pbrMetallicRoughness":{"baseColorFactor":[1,0.5,0,1],"metallicFactor":0,"roughnessFactor":0.5},"alphaMode":"BLEND","doubleSided":true,"extensions":{"KHR_materials_emissive_strength":{"emissiveStrength":2}}}],
"animations":[{"channels":[{"sampler":0,"target":{"node":0,"path":"translation"}}],"samplers":[{"input":2,"output":0,"interpolation":"LINEAR"}]}],
"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]},{"bufferView":1,"componentType":5123,"count":3,"type":"SCALAR"},{"bufferView":2,"componentType":5126,"count":3,"type":"SCALAR","min":[0],"max":[1]}],
"bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36},{"buffer":0,"byteOffset":36,"byteLength":6},{"buffer":0,"byteOffset":44,"byteLength":12}],
"buffers":[{"byteLength":56,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAAAAABAAIAAAAAAAAAAAAAPwAAgD8="}]}"#;

fn seeds_gltf() -> Vec<Vec<u8>> {
    with_files(&[GLTF.as_bytes()], &["tests/corpus/media/robot.glb"])
}

/// Imports the bytes as a `.gltf` or `.glb` file.
pub fn run_gltf(data: &[u8]) {
    if let Some(file) = Scratch::new("in.gltf", data) {
        let _ = sr_3d::import::gltf(&file.0);
    }
}

// ------------------------------------------------------------------ colour

const CUBE_1D: &str = "TITLE \"shaper\"\n# a comment\nDOMAIN_MIN -0.25 0 0\nDOMAIN_MAX 1.5 1 2\nLUT_1D_SIZE 3\n0 0 0\n0.5 0.4 0.3\n1 1 1\n";
const LUT_3DL: &str =
    "0 341 682 1023\n0 0 0\n0 0 1023\n0 1023 0\n0 1023 1023\n1023 0 0\n1023 0 1023\n1023 1023 0\n1023 1023 1023\n";
const CLF_CHAIN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<ProcessList id="chain" compCLFversion="3">
  <Description>every node the reader takes</Description>
  <Matrix inBitDepth="32f" outBitDepth="32f"><Array dim="3 4">1.1 0 0 0.01 0 0.9 0 0 0 0 1 -0.02</Array></Matrix>
  <LUT1D inBitDepth="32f" outBitDepth="10i"><Array dim="3 1">0 512 1023</Array></LUT1D>
  <Range inBitDepth="10i" outBitDepth="32f"><minInValue>64</minInValue><maxInValue>940</maxInValue><minOutValue>0</minOutValue><maxOutValue>1</maxOutValue></Range>
  <ASC_CDL><SOPNode><Slope>1.1 1 0.9</Slope><Offset>0.01 0 -0.01</Offset><Power>1 1.2 1</Power></SOPNode><SatNode><Saturation>0.8</Saturation></SatNode></ASC_CDL>
  <LUT3D inBitDepth="32f" outBitDepth="32f"><Array dim="2 2 2 3">0 0 0 0 0 1 0 1 0 0 1 1 1 0 0 1 0 1 1 1 0 1 1 1</Array></LUT3D>
</ProcessList>"#;
const CLF_LUT3D: &str = r#"<ProcessList id="one"><LUT3D outBitDepth="12i"><Array dim="2 2 2 3">0 0 0 0 0 4095 0 4095 0 0 4095 4095 4095 0 0 4095 0 4095 4095 4095 0 4095 4095 4095</Array></LUT3D></ProcessList>"#;

fn seeds_lut() -> Vec<Vec<u8>> {
    with_files(
        &[CUBE_1D.as_bytes(), LUT_3DL.as_bytes(), CLF_CHAIN.as_bytes(), CLF_LUT3D.as_bytes()],
        &["tests/corpus/media/grade.cube"],
    )
}

/// Reads the text as a .cube, .3dl and .clf LUT: a table read is n³ entries.
pub fn run_lut(data: &[u8]) {
    let text = String::from_utf8_lossy(data);
    for ext in ["cube", "3dl", "clf"] {
        if let Ok((n, t)) = sr_gpu::fx::parse_lut(&text, ext, "fuzz") {
            assert_eq!(Some(t.len()), (n as usize).checked_pow(3), "a {ext} LUT of side {n} has {} entries", t.len());
        }
    }
}

/// A matrix/TRC profile of the ICC layout: a 128-byte header, the tag count and table, then the tags.
fn icc_profile(gray: bool, curve: &[u8]) -> Vec<u8> {
    let fixed = |v: f64| ((v * 65536.0).round() as i32).to_be_bytes();
    let mut tags: Vec<([u8; 4], Vec<u8>)> = Vec::new();
    if gray {
        tags.push((*b"kTRC", curve.to_vec()));
    } else {
        let colorants = [[0.5151, 0.2412, -0.0011], [0.2920, 0.6922, 0.0419], [0.1571, 0.0666, 0.7841]];
        for (sig, c) in [(*b"rXYZ", colorants[0]), (*b"gXYZ", colorants[1]), (*b"bXYZ", colorants[2])] {
            let mut t = b"XYZ \0\0\0\0".to_vec();
            c.iter().for_each(|v| t.extend(fixed(*v)));
            tags.push((sig, t));
        }
        for sig in [*b"rTRC", *b"gTRC", *b"bTRC"] {
            tags.push((sig, curve.to_vec()));
        }
        tags.push((*b"cicp", b"cicp\0\0\0\0\x0c\x0d\0\x01".to_vec()));
    }
    let mut desc = b"mluc\0\0\0\0\0\0\0\x01\0\0\0\x0cenUS\0\0\0\x08\0\0\0\x1c".to_vec();
    desc.extend("fuzz".encode_utf16().flat_map(u16::to_be_bytes));
    tags.push((*b"desc", desc));
    let base = 132 + 12 * tags.len();
    let (mut table, mut body) = (Vec::new(), Vec::new());
    for (sig, t) in &tags {
        table.extend(sig);
        table.extend(((base + body.len()) as u32).to_be_bytes());
        table.extend((t.len() as u32).to_be_bytes());
        body.extend(t);
        body.resize(body.len().next_multiple_of(4), 0);
    }
    let mut out = vec![0u8; 128];
    out[8] = 4;
    out[12..16].copy_from_slice(b"mntr");
    out[16..20].copy_from_slice(if gray { b"GRAY" } else { b"RGB " });
    out[20..24].copy_from_slice(b"XYZ ");
    out[36..40].copy_from_slice(b"acsp");
    out.extend((tags.len() as u32).to_be_bytes());
    out.extend(table);
    out.extend(body);
    let n = out.len() as u32;
    out[0..4].copy_from_slice(&n.to_be_bytes());
    out
}

fn seeds_icc() -> Vec<Vec<u8>> {
    let mut para = b"para\0\0\0\0\0\x03\0\0".to_vec();
    for v in [2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045] {
        para.extend(((v * 65536.0f64).round() as i32).to_be_bytes());
    }
    let curv = b"curv\0\0\0\0\0\0\0\x05\0\0\x20\0\x50\0\xa0\0\xff\xff".to_vec();
    let mut out = vec![icc_profile(false, &para), icc_profile(true, &curv)];
    out.extend(with_files(
        &[],
        &[
            "crates/sr-media/tests/fixtures/still/p3.icc",
            "crates/sr-media/tests/fixtures/still/p3-icc.heic",
            "crates/sr-media/tests/fixtures/still/p3-nclx.heic",
            "crates/sr-media/tests/fixtures/still/pattern.avif",
        ],
    ));
    out
}

/// Reads the bytes as an ICC profile and as a HEIF/AVIF file's colour boxes, with every embedded profile read
/// and its curves evaluated.
pub fn run_icc(data: &[u8]) {
    use sr_media::{heif, icc};
    let mut profiles = vec![data.to_vec()];
    if heif::is_heif(data) {
        profiles.extend(heif::colour(data).into_iter().filter_map(|c| match c {
            heif::Colr::Icc(p) => Some(p),
            heif::Colr::Nclx(..) => None,
        }));
    }
    for p in profiles {
        if let Ok(p) = icc::parse(&p) {
            for c in &p.curves {
                for x in [-1.0, 0.0, 0.04, 0.25, 0.5, 1.0, 2.0, f64::NAN, f64::INFINITY] {
                    let _ = c.eval(x);
                }
            }
        }
    }
}

// ------------------------------------------------------------------ volumes

/// The decoder budgets of the volume targets: room for every fixture and the OpenVDB reader's 4 MiB workspace.
const VOLUME_LIMITS: sr_volume::CacheLimits =
    sr_volume::CacheLimits { max_bytes: 16 << 20, max_bricks: 4096, max_grids: 8 };

fn seeds_openvdb() -> Vec<Vec<u8>> {
    let fixtures = [
        "large-tile",
        "unsupported-bool",
        "render-0",
        "double",
        "affine-background",
        "vector-double",
        "dense-zip",
        "dense-blosc",
        "density-zip",
        "density-half",
        "density-blosc",
        "tiles",
        "instanced",
        "vector",
    ]
    .map(|f| format!("crates/sr-volume/tests/data/openvdb/{f}.vdb"));
    let mut files: Vec<&str> = fixtures.iter().map(String::as_str).collect();
    files.push("tests/corpus/media/impact-0.vdb");
    with_files(&[], &files)
}

/// Samples every grid of a volume read within `limits`, and checks it holds to them.
fn probe_volume(v: &sr_volume::Volume, limits: sr_volume::CacheLimits) {
    let grids: Vec<_> = v.grids().collect();
    assert!(grids.len() <= limits.max_grids, "{} grids read, over {}", grids.len(), limits.max_grids);
    let bricks: usize = grids.iter().map(|(_, g)| g.brick_count()).sum();
    assert!(bricks <= limits.max_bricks, "{bricks} bricks read, over {}", limits.max_bricks);
    for (name, g) in grids {
        assert!(v.grid(name).is_some(), "the grid {name:?} is listed but not found");
        for p in [[0.0; 3], [0.5, -1.25, 3.75], [1e9, -1e9, 1e30], [f64::NAN, 0.0, f64::INFINITY]] {
            let _ = g.sample_world(p);
            let _ = g.sample_index(p);
        }
        let _ = g.background();
        if let Some((key, _)) = g.bricks().next() {
            let at = key.map(|k| k.saturating_mul(sr_volume::BRICK_SIDE));
            let _ = g.value(at);
            let _ = g.sample_index(at.map(|k| f64::from(k) + 3.5));
        }
    }
}

/// Reads the bytes as an OpenVDB file within [`VOLUME_LIMITS`] and samples what it holds.
pub fn run_openvdb(data: &[u8]) {
    if let Ok(v) = sr_volume::openvdb::read(std::io::Cursor::new(data), VOLUME_LIMITS) {
        probe_volume(&v, VOLUME_LIMITS);
    }
}

fn seeds_srvol() -> Vec<Vec<u8>> {
    with_files(
        &[],
        &[
            "tests/corpus/media/advected-0.srvol",
            "tests/corpus/media/advected-1.srvol",
            "tests/corpus/media/uniform.srvol",
            "tests/corpus/media/uniform-00.srvol",
        ],
    )
}

/// The cell budget of the voxel targets.
const VOXEL_LIMITS: sr_3d::occupancy::Limits =
    sr_3d::occupancy::Limits { max_bricks: 1024, max_cells: 1 << 20, max_bytes: 16 << 20 };

/// Checks an occupancy read within [`VOXEL_LIMITS`] holds to them.
fn within_voxel_limits(o: &sr_3d::occupancy::Occupancy) {
    assert!(o.count() <= VOXEL_LIMITS.max_cells, "{} cells read, over {}", o.count(), VOXEL_LIMITS.max_cells);
    assert!(o.bytes() <= VOXEL_LIMITS.max_bytes, "{} bytes built, over {}", o.bytes(), VOXEL_LIMITS.max_bytes);
}

/// Reads the bytes as a simulation cache (SRVOL) and as a model in one, writes what it read and reads that back.
pub fn run_srvol(data: &[u8]) {
    use sr_3d::voxel::srvol;
    if let Ok(v) = sr_volume::Volume::read(data, VOLUME_LIMITS) {
        probe_volume(&v, VOLUME_LIMITS);
        let mut out = Vec::new();
        let written = v.write(&mut out);
        assert!(written.is_ok(), "a volume read does not write to memory: {written:?}");
        let back = sr_volume::Volume::read(out.as_slice(), VOLUME_LIMITS);
        assert!(back.is_ok(), "a volume written does not read back: {:?}", back.as_ref().err());
        if let Ok(back) = back {
            assert_eq!(back.grids().count(), v.grids().count(), "a volume read back has other grids");
        }
    }
    if let Ok(model) = srvol::import(data, srvol::GRID, VOXEL_LIMITS, VOLUME_LIMITS) {
        within_voxel_limits(&model.occupancy);
    }
}

fn seeds_vox() -> Vec<Vec<u8>> {
    with_files(&[], &["tests/corpus/media/voxels.vox", "tests/corpus/media/block.vox"])
}

/// Reads the bytes as a MagicaVoxel file and builds the cells of its first model, of an absent model and of its
/// scene; a model that builds within the limits round-trips through a cache.
pub fn run_vox(data: &[u8]) {
    use sr_3d::voxel::{srvol, vox};
    let bounds = vox::Bounds {
        max_file_bytes: MAX_INPUT,
        max_models: 64,
        max_nodes: 4096,
        max_depth: 64,
        max_placements: 4096,
        max_placed_cells: 1 << 20,
        max_built_bytes: 16 << 20,
    };
    let Ok(file) = vox::parse(data, &bounds) else { return };
    for model in [Some(0), Some(usize::MAX), None] {
        if let Ok(imported) = file.occupancy(model, VOXEL_LIMITS, &bounds) {
            within_voxel_limits(&imported.occupancy);
            let cache = srvol::write(&imported.occupancy, 0.5);
            assert!(cache.is_ok(), "cells built do not write as a cache: {:?}", cache.as_ref().err());
            let Ok(cache) = cache else { continue };
            let back = srvol::import(&cache, srvol::GRID, VOXEL_LIMITS, VOLUME_LIMITS);
            assert!(back.is_ok(), "a cache written does not read back: {:?}", back.as_ref().err());
            if let Ok(back) = back {
                assert_eq!(back.occupancy.count(), imported.occupancy.count(), "a model read back has other cells");
            }
        }
    }
}

// ------------------------------------------------------------------ fonts

fn seeds_font() -> Vec<Vec<u8>> {
    with_files(&[], &["crates/sr-eval/tests/fixtures/solid.ttf", "crates/sr-gpu/tests/fixtures/fonts/wght-test.ttf"])
}

/// Loads the bytes as a font file, pins it and as the style's own file, then lays out and draws text in it at a
/// variation of its weight axis.
pub fn run_font(data: &[u8]) {
    let Some(file) = Scratch::new("in.ttf", data) else { return };
    // declared after the file, so the library lets go of it first
    let mut lib = sr_text::FontLib::pinned();
    let _ = lib.pin(&file.0, 0, "fuzz", 400, false);
    let style = sr_text::Style {
        families: vec!["fuzz".into()],
        file: Some((file.0.clone(), 0)),
        size: 24.0,
        variations: vec![(*b"wght", 700.0)],
        ..Default::default()
    };
    let para = Para {
        runs: vec![Run { text: "Hamburgefonstiv 0123 ffi \u{e9}\u{301} \u{5d0}\u{5d1}".into(), style: 0, role: None }],
        styles: vec![style],
        opts: Opts { width: 200.0, ..Default::default() },
    };
    let lay = layout::layout(&mut lib, &para);
    let _ = sr_text::glyph::draw(&lib, &lay, None, &Default::default(), 0.5);
}

const JHF_GLYPHS: &str = "    1  1JZ\n    2  9MWRFRT RRYQZR[SZRY\n    3  9I[NFN[ RVFV[ RNPVP\n";

fn seeds_stroke_font() -> Vec<Vec<u8>> {
    with_files(&[JHF_GLYPHS.as_bytes()], &["tests/corpus/media/test.jhf"])
}

/// Reads the text as a Hershey stroke font and sets a line in it: what it sets is path data.
pub fn run_stroke_font(data: &[u8]) {
    use sr_eval::stroke_font;
    let Ok(font) = stroke_font::parse_jhf(&String::from_utf8_lossy(data)) else { return };
    let (d, _missing) = stroke_font::layout(&font, "Hello, World!\n0123 ~\u{7f}\u{1f}\u{e9}", 24.0);
    if !d.is_empty() {
        let parsed = sr_vector::Path::parse(&d);
        assert!(parsed.is_ok(), "a stroke font set path data that does not parse: {:?}", parsed.err());
    }
}

// ------------------------------------------------------------------ lights

const IES_AXIAL: &str =
    "IESNA:LM-63-2002\n[TEST] axial\nTILT=NONE\n1 1000 1 3 1 1 2 0 0 0\n1 1 100\n0 45 90\n0\n100 50 0\n";
const IES_QUADRANT: &str = "IESNA:LM-63-2019\n[MANUFAC] fuzz\nTILT=NONE\n1,-1,1.5,5,3,1,1,0.1,0.1,0\n1,1,40\n\
0 22.5 45 67.5 90\n0 45 90\n900 800 500 200 0\n850 700 400 150 0\n800 600 300 100 0\n";
const IES_FULL: &str = "IESNA91\nTILT=NONE\n1 2000 0.5 4 5 1 2 0 0 0\n1 1 60\n0 60 120 180\n0 90 180 270 360\n\
400 300 100 0\n380 280 90 0\n360 260 80 0\n380 280 90 0\n400 300 100 0\n";

fn seeds_ies() -> Vec<Vec<u8>> {
    with_files(&[IES_AXIAL.as_bytes(), IES_QUADRANT.as_bytes(), IES_FULL.as_bytes()], &[])
}

/// Reads the text as an IES profile: a profile read has finite ascending angles of each kind and a candela per
/// pair, every direction samples to a finite intensity, and its table bakes to values in [0, 1].
pub fn run_ies(data: &[u8]) {
    let Ok(ies) = sr_3d::light::Ies::parse(&String::from_utf8_lossy(data)) else { return };
    for axis in [&ies.vertical, &ies.horizontal] {
        assert!(!axis.is_empty(), "a profile read has no angles of one kind");
        assert!(axis.iter().all(|a| a.is_finite()), "a profile read has an angle that is not finite: {axis:?}");
        assert!(axis.windows(2).all(|w| w[0] <= w[1]), "a profile read has angles out of order: {axis:?}");
    }
    assert_eq!(ies.candela.len(), ies.horizontal.len(), "a profile read has a row per horizontal angle");
    assert!(ies.candela.iter().all(|row| row.len() == ies.vertical.len()), "a row has a candela per vertical angle");
    for theta in [-10.0, 0.0, 12.5, 90.0, 179.0, 180.0, 500.0, f32::NAN, f32::INFINITY] {
        for phi in [-90.0, 0.0, 45.0, 135.0, 270.0, 359.0, f32::NAN] {
            let c = ies.sample(theta, phi);
            assert!(c.is_finite() && c >= 0.0, "a profile read samples to {c} at ({theta}, {phi})");
        }
    }
    let table = ies.bake(16, 8);
    assert_eq!(table.len(), 16 * 8, "a table baked has its size");
    assert!(table.iter().all(|v| (0.0..=1.0).contains(v)), "a table baked holds a value outside [0, 1]");
}

// ------------------------------------------------------------------ documents

fn seeds_documents() -> Vec<Vec<u8>> {
    crate::corpus(&repo("tests/corpus")).into_iter().map(String::into_bytes).filter(|b| b.len() <= MAX_INPUT).collect()
}

/// [`crate::run_doc`] with asset verification on: referenced files resolve against the
/// corpus (`tests/corpus/valid`), are read and hashed.
pub fn run_doc_assets(data: &[u8]) {
    let input = String::from_utf8_lossy(data);
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(repo("tests/corpus/valid")) };
    let _ = sr_model::validate_str(&input, &opts);
    if let Ok(doc) = sr_model::load_str(&input, &opts) {
        if let Ok(ev) = sr_eval::Evaluator::new(&doc, &Default::default()) {
            let d = ev.program().duration;
            for t in [0.0, d * 0.5, d] {
                let _ = ev.evaluate(t);
            }
        }
    }
}

// ------------------------------------------------------------------ campaign

const NUMBERS: &[&str] = &[
    "0",
    "-1",
    "1e308",
    "-1e308",
    "1e-320",
    "NaN",
    "Infinity",
    "99999999999999999999",
    "4294967296",
    "65536",
    "-0.5",
    "18446744073709551615",
    "0x7fffffff",
    "1e12",
    "100000000000",
];

/// One mutation of an input.
pub fn mutate_bytes(r: &mut Rng, seeds: &[Vec<u8>], src: &[u8]) -> Vec<u8> {
    let mut s = src.to_vec();
    for _ in 0..1 + r.below(4) {
        let a = r.below(s.len().max(1)).min(s.len());
        let b = (a + 1 + r.below(64)).min(s.len());
        match r.below(9) {
            0 if a < s.len() => s[a] ^= 1 << r.below(8),
            1 if a < s.len() => s[a] = *r.pick(&[0x00, 0xff, 0x7f, 0x80, 0x01, b' ', b'\n']),
            2 => {
                s.drain(a..b);
            }
            3 => {
                let span = s[a..b].to_vec();
                s.splice(a..a, span);
            }
            4 => {
                // a boundary integer over the bytes here, little- or big-endian
                let v: u64 =
                    *r.pick(&[0, 1, 0x7f, 0x80, 0xff, 0x7fff, 0xffff, 0x7fff_ffff, 0xffff_ffff, u64::MAX, 1 << 62]);
                let w = *r.pick(&[1usize, 2, 4, 8]);
                let mut bytes = v.to_le_bytes()[..w].to_vec();
                if r.below(2) == 0 {
                    bytes.reverse();
                }
                let end = (a + w).min(s.len());
                s.splice(a..end, bytes);
            }
            5 => {
                let o = r.pick(seeds);
                let oa = r.below(o.len().max(1)).min(o.len());
                let ob = (oa + r.below(200)).min(o.len());
                s.splice(a..a, o[oa..ob].iter().copied());
            }
            6 => {
                // replace a run of digits with a boundary number
                let from = s[a..].iter().position(u8::is_ascii_digit).map(|k| a + k).unwrap_or(a);
                let to = s[from..].iter().position(|c| !c.is_ascii_digit() && *c != b'.').map_or(s.len(), |k| from + k);
                s.splice(from..to, r.pick(NUMBERS).bytes());
            }
            7 => s.truncate(a),
            _ => {
                let c = b' ' + r.below(95) as u8;
                s.insert(a, c);
            }
        }
    }
    s.truncate(MAX_INPUT);
    s
}

/// A panic found by the fuzzer.
#[derive(Debug, Clone)]
pub struct Crash {
    pub target: &'static str,
    pub input: Vec<u8>,
    pub message: String,
}

/// Runs `f(input)`, turning a panic into a [`Crash`].
pub fn guard(target: &'static str, input: &[u8], f: fn(&[u8])) -> Option<Crash> {
    catch_unwind(AssertUnwindSafe(|| f(input))).err().map(|e| Crash {
        target,
        input: input.to_vec(),
        message: e
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default(),
    })
}

/// Statistics of a campaign on one target.
#[derive(Debug, Default, Clone)]
pub struct Campaign {
    pub inputs: u64,
    pub crashes: Vec<Crash>,
}

/// Fuzzes `t` until `stop()` returns true: every seed as it is, then mutations.
pub fn fuzz(seed: u64, t: &Target, mut stop: impl FnMut(&Campaign) -> bool) -> Campaign {
    let mut r = Rng(seed | 1);
    let mut c = Campaign::default();
    let seeds = (t.seeds)();
    let texts: Vec<String> = seeds.iter().map(|s| String::from_utf8_lossy(s).into_owned()).collect();
    let mut next = 0;
    while !stop(&c) && !seeds.is_empty() {
        let input = if next < seeds.len() {
            next += 1;
            seeds[next - 1].clone()
        } else if t.text && r.below(2) == 0 {
            let base = r.pick(&texts).clone();
            let mut s = crate::mutate_doc(&mut r, &texts, &base).into_bytes();
            s.truncate(MAX_INPUT);
            s
        } else {
            let base = r.pick(&seeds).clone();
            mutate_bytes(&mut r, &seeds, &base)
        };
        if let Some(k) = guard(t.name, &input, t.run) {
            c.crashes.push(k);
        }
        c.inputs += 1;
    }
    c
}
