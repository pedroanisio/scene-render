//! Build-mode programs (SREP 66): each `<program>` in build mode runs once when the document is loaded, and its
//! output takes its place.
//!
//! * In the composition (or a symbol, group, …), the program becomes a `<group>` that carries its node
//!   attributes and behaviour children, with the emitted nodes as its last children.
//! * In `<parameters>`, the program becomes `<data format="json">` holding the emitted rows.
//!
//! The ids of each node program's output are scoped by the program before the expanded text is validated (SREP 87,
//! see [`crate::program_ids`]). The expanded text is then validated and built like any document. Its errors are
//! reported as PRG14 at the program that emitted the offending output.
//!
//! Outputs are cached on disk by the key of SREP 66, Determinism 6, under `$SR_PROGRAM_CACHE`, else
//! `$XDG_CACHE_HOME/scene-render/program`, else `~/.cache/scene-render/program`. `SR_PROGRAM_CACHE=off` turns the
//! cache off. A cache file holds the output's SHA-256 before the output. A file whose digest does not match is
//! ignored and recomputed.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::ops::Range;
use std::path::{Path, PathBuf};

use roxmltree::Node;
use sha2::{Digest, Sha256};

use crate::assets::{resolve, Resolved};
use crate::diag::{element_path, Diagnostic, Loc};

/// Elements a program's output may hold at most.
pub const MAX_ELEMENTS: usize = 1_000_000;

/// Attributes of `<program>` that are not node attributes of the group it becomes.
const PROGRAM_ONLY: [&str; 10] =
    ["src", "sha256", "seed", "fuel", "memoryLimit", "outputSha256", "mode", "width", "height", "stepsPerFrame"];

/// A document whose build-mode programs were replaced by their output.
pub(crate) struct Expanded {
    /// The new document text.
    pub xml: String,
    /// Where each node program's output lies in `xml`, with the program's id, location and path in the original.
    pub outputs: Vec<(Range<usize>, String, Loc, String)>,
    /// W13: bare references resolved to the one program that emitted their id (SREP 87).
    pub warnings: Vec<Diagnostic>,
    /// `xs:IDREF` tokens outside every output that name an id once the outputs are in place: (offset of their
    /// attribute in the original document, token).
    pub resolved: HashSet<(usize, String)>,
    /// Namespaced id → local id of every element an output holds (SREP 87, Semantics 7).
    pub local_ids: HashMap<String, String>,
    /// Where the text of `xml` outside the outputs came from in the original document.
    pub origin: crate::program_ids::Origin,
}

/// Whether `n` is a program that runs when the document is built.
pub fn is_build_program(n: Node<'_, '_>) -> bool {
    n.is_element()
        && n.tag_name().name() == "program"
        && (n.parent_element().is_some_and(|p| p.tag_name().name() == "parameters")
            || n.attribute("mode").is_none_or(|m| m == "build"))
}

/// Whether `doc` has a build-mode program in its composition (one whose output holds nodes).
pub fn has_node_programs(doc: &roxmltree::Document<'_>) -> bool {
    doc.root_element()
        .descendants()
        .any(|n| is_build_program(n) && n.parent_element().is_some_and(|p| p.tag_name().name() != "parameters"))
}

/// The inputs and limits of a program element, as SREP 66 reads them.
pub fn program_call(n: Node<'_, '_>) -> (sr_wasm::Inputs, sr_wasm::Limits) {
    let num = |k: &str| n.attribute(k).and_then(|v| v.trim().parse::<u64>().ok());
    let project_seed = n
        .document()
        .root_element()
        .children()
        .find(|c| c.tag_name().name() == "project")
        .and_then(|p| p.attribute("seed"))
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let params = n
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "param")
        .map(|c| (c.attribute("name").unwrap_or("").to_string(), c.attribute("value").unwrap_or("").to_string()))
        .collect();
    let limits = sr_wasm::Limits {
        fuel: num("fuel").unwrap_or(1_000_000_000),
        memory_mib: num("memoryLimit").map(|m| m as u32).unwrap_or(64),
    };
    (sr_wasm::Inputs { project_seed, seed: num("seed").unwrap_or(0), params }, limits)
}

/// Reads a program's module and checks it against `@sha256` (PRG10).
pub fn read_module(n: Node<'_, '_>, base: &Path) -> Result<(Vec<u8>, String), String> {
    let src = n.attribute("src").unwrap_or("");
    let sha = n.attribute("sha256").unwrap_or("");
    read_module_file(src, sha, base).map(|bytes| (bytes, sha.trim().to_ascii_lowercase()))
}

/// Reads the module at `src` (resolved against `base`) and checks it against the hex digest `sha256` (PRG10).
pub fn read_module_file(src: &str, sha256: &str, base: &Path) -> Result<Vec<u8>, String> {
    let path = match resolve(src, base) {
        Resolved::Local(p) => p,
        Resolved::Remote(scheme) => {
            return Err(format!("{src:?} is a {scheme} URI; a program's module is a local file"))
        }
    };
    let bytes = std::fs::read(&path).map_err(|e| format!("cannot read {src:?} ({}): {e}", path.display()))?;
    let actual = hex(&Sha256::digest(&bytes));
    let expected = sha256.trim().to_ascii_lowercase();
    if actual != expected {
        return Err(format!("{src:?} does not match @sha256: expected {expected}, found {actual}"));
    }
    Ok(bytes)
}

fn hex(b: &[u8]) -> String {
    b.iter().fold(String::with_capacity(b.len() * 2), |mut s, x| {
        let _ = write!(s, "{x:02x}");
        s
    })
}

fn cache_dir() -> Option<PathBuf> {
    match std::env::var_os("SR_PROGRAM_CACHE") {
        Some(v) if v == "off" => None,
        Some(v) if !v.is_empty() => Some(PathBuf::from(v)),
        _ => std::env::var_os("XDG_CACHE_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
            .map(|d| d.join("scene-render").join("program")),
    }
}

fn cached(key: &str) -> Option<Vec<u8>> {
    let data = std::fs::read(cache_dir()?.join(key)).ok()?;
    let (digest, output) = (data.get(..64)?, data.get(65..)?);
    (data.get(64) == Some(&b'\n') && digest == hex(&Sha256::digest(output)).as_bytes()).then(|| output.to_vec())
}

fn store(key: &str, output: &[u8]) {
    let Some(dir) = cache_dir() else { return };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let tmp = dir.join(format!(".{key}.{}", std::process::id()));
    let mut data = hex(&Sha256::digest(output)).into_bytes();
    data.push(b'\n');
    data.extend_from_slice(output);
    if std::fs::write(&tmp, &data).is_ok() && std::fs::rename(&tmp, dir.join(key)).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Runs a build-mode program, or takes its output from the cache, and checks `@outputSha256` (PRG15).
fn output_of(n: Node<'_, '_>, base: &Path) -> Result<Vec<u8>, (&'static str, String)> {
    let (module, sha) = read_module(n, base).map_err(|m| ("PRG10", m))?;
    let (inputs, limits) = program_call(n);
    let key = sr_wasm::cache_key(&sha, &inputs, limits);
    let output = match cached(&key) {
        Some(o) => o,
        None => {
            let o = sr_wasm::generate(&module, &inputs, limits).map_err(|e| (e.code.as_str(), e.message))?;
            store(&key, &o);
            o
        }
    };
    if let Some(want) = n.attribute("outputSha256") {
        let got = hex(&Sha256::digest(&output));
        if got != want.trim().to_ascii_lowercase() {
            return Err(("PRG15", format!("the output's SHA-256 is {got}, not the outputSha256 {want}")));
        }
    }
    Ok(output)
}

fn escape(s: &str, attr: bool) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' if attr => o.push_str("&quot;"),
            '\n' if attr => o.push_str("&#10;"),
            '\t' if attr => o.push_str("&#9;"),
            '\r' => o.push_str("&#13;"),
            c => o.push(c),
        }
    }
    o
}

/// Checks a node program's output as SREP 66, Semantics 5 asks, before it goes in place.
fn check_fragment(text: &str) -> Result<(), String> {
    if text.contains("<!DOCTYPE") || text.contains("<!ENTITY") {
        return Err("the output has a DOCTYPE or an entity declaration".into());
    }
    let wrapped = format!("<sr-fragment>{text}</sr-fragment>");
    if crate::limits::too_deep(&wrapped, crate::limits::MAX_DEPTH).is_some() {
        return Err(format!("the output nests elements more than {} levels deep", crate::limits::MAX_DEPTH));
    }
    let doc = roxmltree::Document::parse_with_options(&wrapped, crate::limits::parsing_options())
        .map_err(|e| format!("the output is not a well-formed XML fragment: {e}"))?;
    let root = doc.root_element();
    for c in root.children() {
        if c.is_pi() {
            return Err("the output has a processing instruction".into());
        }
        if c.is_text() && !c.text().unwrap_or("").trim().is_empty() {
            return Err("the output has text outside its elements".into());
        }
    }
    let mut count = 0usize;
    for d in root.descendants().filter(|d| d.is_element() && *d != root) {
        count += 1;
        if count > MAX_ELEMENTS {
            return Err(format!("the output has more than {MAX_ELEMENTS} elements"));
        }
        if d.tag_name().name() == "program" {
            return Err("the output contains a program; a program's output is final".into());
        }
        if d.tag_name().namespace().is_some() {
            return Err("the output has an element in a namespace".into());
        }
    }
    if root.descendants().any(|d| d.is_pi()) {
        return Err("the output has a processing instruction".into());
    }
    Ok(())
}

/// Checks a data program's output: JSON that a data source accepts (an array of rows, or an object with a `rows`
/// array).
fn check_rows(text: &str) -> Result<(), String> {
    let j: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("the output is not JSON: {e}"))?;
    match &j {
        serde_json::Value::Array(_) => Ok(()),
        serde_json::Value::Object(o) if matches!(o.get("rows"), Some(serde_json::Value::Array(_))) => Ok(()),
        _ => Err("the output is not an array of rows or an object with a \"rows\" array".into()),
    }
}

/// Replaces every build-mode program of `doc` by its output, with the ids of each output scoped by its program
/// (SREP 87). `None` when the document has none.
pub(crate) fn expand(
    doc: &roxmltree::Document<'_>,
    xml: &str,
    base: &Path,
) -> Option<Result<Expanded, Vec<Diagnostic>>> {
    let programs: Vec<Node> = doc.root_element().descendants().filter(|n| is_build_program(*n)).collect();
    if programs.is_empty() {
        return None;
    }
    let mut errors = Vec::new();
    // per program: its range in `xml`, and what replaces it (pieces of new text, or copies of the original)
    let mut replacements: Vec<(Range<usize>, Vec<Piece>, Node)> = Vec::new();
    for n in programs {
        let id = n.attribute("id").unwrap_or("").to_string();
        let fail = |code: &str, message: String| {
            Diagnostic::error(code, format!("program {id:?}: {message}"), Loc::of(n), element_path(n))
        };
        let output = match output_of(n, base) {
            Ok(o) => o,
            Err((code, message)) => {
                errors.push(fail(code, message));
                continue;
            }
        };
        let text = match String::from_utf8(output) {
            Ok(t) => t,
            Err(_) => {
                errors.push(fail("PRG14", "the output is not UTF-8".into()));
                continue;
            }
        };
        let data = n.parent_element().is_some_and(|p| p.tag_name().name() == "parameters");
        let checked = if data { check_rows(&text) } else { check_fragment(&text) };
        if let Err(m) = checked {
            errors.push(fail("PRG14", m));
            continue;
        }
        let mut pieces = Vec::new();
        if data {
            pieces.push(Piece::New(
                format!("<data id=\"{}\" format=\"json\">", escape(&id, true)) + &escape(&text, false) + "</data>",
                None,
            ));
        } else {
            pieces.push(Piece::New("<group".into(), Some(n.range().start)));
            for a in n.attributes().filter(|a| !PROGRAM_ONLY.contains(&a.name()) && a.name() != "prewarm") {
                pieces.push(Piece::New(" ".into(), None));
                pieces.push(Piece::New(format!("{}=\"{}\"", a.name(), escape(a.value(), true)), Some(a.range().start)));
            }
            pieces.push(Piece::New(">".into(), None));
            for c in n.children() {
                if c.is_element() && c.tag_name().name() == "param" {
                    continue;
                }
                pieces.push(Piece::Copy(c.range()));
            }
            pieces.push(Piece::Output(text));
            pieces.push(Piece::New("</group>".into(), None));
        }
        replacements.push((n.range(), pieces, n));
    }
    if !errors.is_empty() {
        return Some(Err(errors));
    }
    replacements.sort_by_key(|r| r.0.start);

    // the document with every output in place, and where each part of it came from
    let mut text = String::with_capacity(xml.len());
    let mut map = crate::program_ids::OffsetMap::default();
    let mut emitters = Vec::new();
    let mut spans: Vec<(Range<usize>, usize)> = Vec::new();
    let mut at = 0;
    let copy = |text: &mut String, map: &mut crate::program_ids::OffsetMap, r: Range<usize>| {
        map.copied(text.len()..text.len() + r.len(), r.start);
        text.push_str(&xml[r]);
    };
    for (range, pieces, n) in replacements {
        copy(&mut text, &mut map, at..range.start);
        for piece in pieces {
            match piece {
                Piece::New(s, anchor) => {
                    if let Some(o) = anchor {
                        map.anchor(text.len(), o);
                    }
                    text.push_str(&s);
                }
                Piece::Copy(r) => copy(&mut text, &mut map, r),
                Piece::Output(s) => {
                    spans.push((text.len()..text.len() + s.len(), emitters.len()));
                    text.push_str(&s);
                }
            }
        }
        if matches!(n.parent_element(), Some(p) if p.tag_name().name() != "parameters") {
            emitters.push(crate::program_ids::Emitter { id: n.attribute("id").unwrap_or("").to_string(), node: n });
        }
        at = range.end;
    }
    copy(&mut text, &mut map, at..xml.len());

    // SREP 87: each output's ids become the program's, and the references to them follow
    let scoped = match crate::program_ids::scope(&text, &spans, &emitters, &map, doc) {
        Ok(s) => s,
        Err(errors) => return Some(Err(errors)),
    };
    let shift = |pos: usize| -> usize {
        let mut p = pos as isize;
        for (r, s) in &scoped.edits {
            if r.end <= pos {
                p += s.len() as isize - r.len() as isize;
            }
        }
        p as usize
    };
    let outputs = spans
        .iter()
        .map(|(r, k)| {
            let n = emitters[*k].node;
            (shift(r.start)..shift(r.end), emitters[*k].id.clone(), Loc::of(n), element_path(n))
        })
        .collect();
    let mut out = String::with_capacity(text.len() + 64 * scoped.edits.len());
    let mut at = 0;
    for (r, s) in &scoped.edits {
        out.push_str(&text[at..r.start]);
        out.push_str(s);
        at = r.end;
    }
    out.push_str(&text[at..]);
    let origin = crate::program_ids::Origin::new(map, &scoped.edits);
    Some(Ok(Expanded {
        xml: out,
        outputs,
        warnings: scoped.warnings,
        resolved: scoped.resolved,
        local_ids: scoped.local_ids,
        origin,
    }))
}

/// A part of what replaces a program.
enum Piece {
    /// New text, and the offset in the original document it stands for.
    New(String, Option<usize>),
    /// A copy of the original text.
    Copy(Range<usize>),
    /// The program's output.
    Output(String),
}
