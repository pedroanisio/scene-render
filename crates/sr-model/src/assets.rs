//! Verification of the files a document references.
//!
//! Every attribute of type `xs:anyURI` that names an input (all of them
//! except `output/@path`, `still/@path` and `destination/@uri`, and `output/@report`) must resolve
//! to an existing file. Where the document declares a digest — `@sha256` for
//! `@src`, `@cacheSha256` for `@cache` — the file's SHA-256 must match it,
//! which is what makes renders of generated media and transcriptions
//! reproducible. Image, mesh and volume sequences expand and check every declared frame.
//!
//! Codes: `A01` missing file, `A02` digest mismatch, `A03` remote input not
//! verified offline, `A04` missing sequence frames, `A05` unreadable file,
//! `A06` physics cache absent (recomputed at render time), `A07` declared
//! image dimensions differ from the local source file, `A08` an image/video representation declares other
//! dimensions than its file or another aspect ratio than the asset, `A09` an audio asset declares a sample rate,
//! channel count or duration that its file contradicts.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rayon::prelude::*;
use roxmltree::{Document, Node};
use sha2::{Digest, Sha256};

use crate::diag::{element_path, Diagnostic, Loc, Severity};
use crate::model::{ColorSpace, Representation, Transfer};
use crate::xsd::{root_builtin, Builtin, COMPLEX_TYPES};

/// Frames beyond this count are not checked individually.
const MAX_SEQUENCE_FRAMES: i128 = 1_000_000;

/// Widest frame-number placeholder accepted (`%0Nd` or a run of `#`).
pub const MAX_FRAME_DIGITS: usize = 64;

/// How far, in seconds, an audio asset's declared `duration` may differ from its probed container duration before
/// A09 warns: container durations are rounded to the packet or frame.
pub const AUDIO_DURATION_TOLERANCE: f64 = 0.05;

/// The name that the asset attribute `proxy` answers to when a representation is preferred.
pub const PROXY_REPRESENTATION: &str = "proxy";

/// The source an asset is read from when a representation is preferred: see [`select_representation`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Selected<'a> {
    /// The URI to read instead of the asset's `src`.
    pub src: &'a str,
    /// The representation's `colorSpace`, when it declares one.
    pub color_space: Option<ColorSpace>,
    /// The representation's `transfer`, when it declares one.
    pub transfer: Option<Transfer>,
    /// The representation's `language`, when it declares one.
    pub language: Option<&'a str>,
}

fn primary_subtag(tag: &str) -> String {
    tag.split('-').next().unwrap_or("").to_ascii_lowercase()
}

/// Which source of an asset the preference `wanted` (`--representation`, else `output/@representation`) selects, or
/// `None` for the asset's own `src`. One mechanism for every way an asset can have more than one file:
///
/// 1. a `<representation>` whose `name` is `wanted`;
/// 2. the asset's `proxy` attribute, when `wanted` is `proxy` (an implicit representation of that name, so an
///    explicit `<representation name="proxy">` wins over it);
/// 3. a `<representation>` whose `language` is `wanted` (BCP 47, compared case-insensitively), else the first
///    whose primary language subtag is `wanted`'s.
pub fn select_representation<'a>(
    reps: &'a [Representation],
    proxy: Option<&'a str>,
    wanted: Option<&str>,
) -> Option<Selected<'a>> {
    let wanted = wanted?;
    let of = |r: &'a Representation| Selected {
        src: r.src.as_str(),
        color_space: r.color_space,
        transfer: r.transfer,
        language: r.language.as_ref().map(|l| l.0.as_str()),
    };
    if let Some(r) = reps.iter().find(|r| r.name == wanted) {
        return Some(of(r));
    }
    if wanted == PROXY_REPRESENTATION {
        if let Some(src) = proxy {
            return Some(Selected { src, color_space: None, transfer: None, language: None });
        }
    }
    // A requested name that is not a schema language tag must not accidentally select a language.
    let mut subtags = wanted.split('-');
    let primary = subtags.next().unwrap_or_default();
    if wanted.len() > 35
        || !(2..=8).contains(&primary.len())
        || !primary.bytes().all(|b| b.is_ascii_alphabetic())
        || subtags.any(|s| !(1..=8).contains(&s.len()) || !s.bytes().all(|b| b.is_ascii_alphanumeric()))
    {
        return None;
    }
    let language = |r: &&'a Representation| r.language.as_ref().map(|l| l.0.as_str());
    let exact = reps.iter().find(|r| language(r).is_some_and(|l| l.eq_ignore_ascii_case(wanted)));
    let wanted_primary = primary_subtag(wanted);
    exact.or_else(|| reps.iter().find(|r| language(r).is_some_and(|l| primary_subtag(l) == wanted_primary))).map(of)
}

/// Whether a `rw`x`rh` representation keeps the aspect ratio of a `aw`x`ah` asset, up to the rounding of a scaled
/// size to whole pixels (one pixel in either dimension).
pub fn keeps_aspect(rw: u64, rh: u64, aw: u64, ah: u64) -> bool {
    let cross = (rh as u128 * aw as u128).abs_diff(ah as u128 * rw as u128);
    cross <= aw.max(ah) as u128
}

/// Names of input-file attributes, derived from the schema: every `xs:anyURI`
/// attribute except the output-side `path` and `uri`. (`output/@report` is an output too: see [`is_output_side`].)
pub fn input_uri_attributes() -> &'static BTreeSet<&'static str> {
    static S: OnceLock<BTreeSet<&'static str>> = OnceLock::new();
    S.get_or_init(|| {
        COMPLEX_TYPES
            .iter()
            .flat_map(|c| c.attrs.iter())
            .filter(|a| root_builtin(a.ty) == Some(Builtin::AnyUri))
            .map(|a| a.name)
            .filter(|n| !matches!(*n, "path" | "uri"))
            .collect()
    })
}

/// Where a URI points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// A local file.
    Local(PathBuf),
    /// A remote resource with the given scheme.
    Remote(String),
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (
                b.get(i + 1).and_then(|c| (*c as char).to_digit(16)),
                b.get(i + 2).and_then(|c| (*c as char).to_digit(16)),
            ) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Resolves a URI reference against the document directory.
pub fn resolve(uri: &str, base_dir: &Path) -> Resolved {
    let uri = uri.trim();
    if let Some(rest) = uri.strip_prefix("file://") {
        let p = rest.strip_prefix("localhost").unwrap_or(rest);
        return Resolved::Local(PathBuf::from(percent_decode(p)));
    }
    if let Some((scheme, _)) = uri.split_once(':') {
        let is_scheme = scheme.len() >= 2
            && scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
        if is_scheme {
            if scheme.eq_ignore_ascii_case("file") {
                return Resolved::Local(PathBuf::from(percent_decode(&uri[5..])));
            }
            return Resolved::Remote(scheme.to_ascii_lowercase());
        }
    }
    let path = PathBuf::from(percent_decode(uri));
    if path.is_absolute() {
        Resolved::Local(path)
    } else {
        Resolved::Local(base_dir.join(path))
    }
}

/// SHA-256 of a file, streamed.
pub fn sha256_file(path: &Path) -> std::io::Result<[u8; 32]> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().into())
}

fn hex(d: &[u8; 32]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// Expands an image-sequence pattern (`name_%04d.png`, `name_%d.png` or
/// `name_####.png`) for one frame number. `None` when the pattern has no
/// placeholder or one wider than [`MAX_FRAME_DIGITS`].
pub fn sequence_frame(pattern: &str, frame: i64) -> Option<String> {
    if let Some(start) = pattern.find('%') {
        let rest = &pattern[start + 1..];
        let spec_len = rest.find('d')?;
        let spec = &rest[..spec_len];
        if !spec.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let digits = spec.trim_start_matches('0');
        let width: usize = if digits.is_empty() { 0 } else { digits.parse().ok()? };
        if width > MAX_FRAME_DIGITS {
            return None;
        }
        let num = format_frame(frame, width);
        return Some(format!("{}{}{}", &pattern[..start], num, &rest[spec_len + 1..]));
    }
    let start = pattern.find('#')?;
    let width = pattern[start..].bytes().take_while(|b| *b == b'#').count();
    if width > MAX_FRAME_DIGITS {
        return None;
    }
    Some(format!("{}{}{}", &pattern[..start], format_frame(frame, width), &pattern[start + width..]))
}

fn format_frame(frame: i64, width: usize) -> String {
    if frame < 0 {
        format!("-{:0width$}", frame.unsigned_abs(), width = width.saturating_sub(1))
    } else {
        format!("{frame:0width$}")
    }
}

struct Check<'a, 'i> {
    node: Node<'a, 'i>,
    attr: &'static str,
    value: &'a str,
    path: PathBuf,
    expected: Option<[u8; 32]>,
    hash_attr: &'static str,
    missing: Severity,
    missing_code: &'static str,
}

fn attr_loc(n: Node, attr: &str) -> Loc {
    n.attributes()
        .find(|a| a.name() == attr && a.namespace().is_none())
        .map(|a| Loc::at(a.range().start))
        .unwrap_or_else(|| Loc::of(n))
}

fn parse_digest(s: &str) -> Option<[u8; 32]> {
    <crate::values::Sha256 as crate::parse::ParseValue>::parse_value(s).ok().map(|d| d.0)
}

/// Whether `attr` of an element named `element` names a file the render writes rather than one it reads:
/// `output/@report`, where a render report goes (SREP 18). Any other element's `report` is an input as usual.
pub fn is_output_side(element: &str, attr: &str) -> bool {
    (element, attr) == ("output", "report")
}

/// Whether `value` of `element/@attr` names something of the document rather than a file: a material map that begins
/// with `#` reads the generator or image asset with that id (SREP 71, rule MTX1).
pub fn is_document_reference(element: &str, attr: &str, value: &str) -> bool {
    // generated elements of some types report their XSD type's name
    matches!(element, "material" | "materialType")
        && crate::rules::MATERIAL_MAP_RULES.iter().any(|(a, _)| *a == attr)
        && value.trim_start().starts_with('#')
}

/// Verifies every input file of `doc`, resolving relative references
/// against `base_dir`, and appends diagnostics.
pub fn verify(doc: &Document<'_>, base_dir: &Path, out: &mut Vec<Diagnostic>) {
    let uri_attrs = input_uri_attributes();
    let mut checks: Vec<Check> = Vec::new();
    let mut remote: Vec<(Node, &'static str, String)> = Vec::new();
    for n in doc.root_element().descendants().filter(|n| n.is_element()) {
        let name = n.tag_name().name();
        let numbered = matches!(name, "imageSequence" | "meshSequence")
            || (name == "volume" && (n.has_attribute("first") || n.has_attribute("last")));
        if numbered {
            sequence(n, base_dir, out);
        }
        for &attr in uri_attrs {
            if (numbered && attr == "src") || is_output_side(name, attr) {
                continue;
            }
            // a renderer never reads a pdf asset's source, only its pinned cache (SREP 17 §1.2); the resolve step
            // checks the source against @sha256
            if name == "pdf" && attr == "src" {
                continue;
            }
            let Some(value) = n.attribute(attr) else { continue };
            if is_document_reference(name, attr, value) {
                continue;
            }
            if name == "volume" && n.attribute("format") == Some("srvseq") && attr == "src" {
                match resolve(value, base_dir) {
                    Resolved::Local(path) => {
                        let result = (|| -> Result<(), sr_volume::Error> {
                            let limits = sr_volume::bake::BakeLimits::default();
                            let bake = sr_volume::bake::BakedSequence::open(&path, n.attribute("sha256"), limits)?;
                            let mut seen = BTreeSet::new();
                            for frame in bake.frames() {
                                if seen.insert(frame.sha256()) {
                                    frame.read(bake.directory(), limits.frame)?;
                                }
                            }
                            Ok(())
                        })();
                        if let Err(e) = result {
                            out.push(Diagnostic::error(
                                "A05",
                                format!("volume bake {}: {e}", path.display()),
                                attr_loc(n, attr),
                                element_path(n),
                            ));
                        }
                    }
                    Resolved::Remote(scheme) => remote.push((n, attr, scheme)),
                }
                continue;
            }
            let (expected, hash_attr) = match attr {
                "src" => (n.attribute("sha256").and_then(parse_digest), "sha256"),
                "cache" => (n.attribute("cacheSha256").and_then(parse_digest), "cacheSha256"),
                _ => (None, ""),
            };
            let (missing, missing_code) = match (name, attr) {
                ("physics", "cache") => (Severity::Warning, "A06"),
                // a program's module is checked under its own code (SREP 66, PRG10)
                ("program", "src") => (Severity::Error, "PRG10"),
                (_, "proxy") => (Severity::Warning, "A01"),
                _ => (Severity::Error, "A01"),
            };
            match resolve(value, base_dir) {
                Resolved::Local(path) => {
                    checks.push(Check { node: n, attr, value, path, expected, hash_attr, missing, missing_code })
                }
                Resolved::Remote(scheme) => remote.push((n, attr, scheme)),
            }
        }
    }

    for (n, attr, scheme) in remote {
        if n.has_tag_name("representation") && attr == "src" {
            representation_dimensions(n, None, n.attribute("src").unwrap_or_default(), out);
        }
        out.push(Diagnostic::warning(
            "A03",
            format!("@{attr} of <{}> is a {scheme} URI and was not verified offline", n.tag_name().name()),
            attr_loc(n, attr),
            element_path(n),
        ));
    }

    // Hash each distinct file once, in parallel.
    let to_hash: BTreeSet<&PathBuf> = checks.iter().filter(|c| c.expected.is_some()).map(|c| &c.path).collect();
    let digests: HashMap<&PathBuf, std::io::Result<[u8; 32]>> =
        to_hash.into_par_iter().map(|p| (p, sha256_file(p))).collect::<Vec<_>>().into_iter().collect();

    // Probe each original image once, without decoding its pixels. Proxies intentionally
    // have different dimensions; formats delegated to FFmpeg at render time may not have
    // a native reader, so an unavailable probe does not invalidate an otherwise usable asset.
    let images: BTreeSet<&PathBuf> = checks.iter().filter(|c| is_image_source(c)).map(|c| &c.path).collect();
    let sizes: HashMap<&PathBuf, Option<(u32, u32)>> = images
        .into_par_iter()
        .map(|p| {
            let size = image::ImageReader::open(p)
                .and_then(|r| r.with_guessed_format())
                .ok()
                .and_then(|r| r.into_decoder().ok())
                .map(|mut decoder| {
                    use image::metadata::Orientation;
                    use image::ImageDecoder;
                    let (w, h) = decoder.dimensions();
                    match decoder.orientation().unwrap_or(Orientation::NoTransforms) {
                        Orientation::Rotate90
                        | Orientation::Rotate270
                        | Orientation::Rotate90FlipH
                        | Orientation::Rotate270FlipH => (h, w),
                        _ => (w, h),
                    }
                });
            (p, size)
        })
        .collect();

    // Probe audio declarations and video representations once per file. An unavailable probe leaves declarations
    // unchecked; missing/unreadable files still receive the ordinary asset diagnostics.
    let media: BTreeSet<&PathBuf> = checks
        .iter()
        .filter(|c| {
            c.attr == "src"
                && ((c.node.has_tag_name("audio")
                    && ["sampleRate", "channels", "duration"].iter().any(|a| c.node.has_attribute(*a)))
                    || (c.node.has_tag_name("representation")
                        && c.node.parent_element().is_some_and(|p| p.has_tag_name("video"))))
        })
        .map(|c| &c.path)
        .collect();
    let probes: HashMap<&PathBuf, Option<sr_media::MediaInfo>> =
        media.into_par_iter().map(|p| (p, sr_media::probe(p).ok())).collect::<Vec<_>>().into_iter().collect();

    for c in &checks {
        let ename = c.node.tag_name().name();
        // a program's module that cannot be read or does not match its digest is PRG10 (SREP 66)
        let module = ename == "program" && c.attr == "src";
        let unreadable = if module { "PRG10" } else { "A05" };
        let loc = attr_loc(c.node, c.attr);
        let path = element_path(c.node);
        let shown = format!("{:?}", c.value);
        let meta = std::fs::metadata(&c.path);
        match meta {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let msg = if c.missing_code == "A06" {
                    format!("physics cache {shown} does not exist; the simulation will be recomputed")
                } else {
                    format!("@{} of <{ename}>: file {shown} does not exist (resolved to {})", c.attr, c.path.display())
                };
                let resolvable = matches!(ename, "generated" | "captionTrack" | "tiles") && c.attr == "cache";
                out.push(Diagnostic {
                    severity: c.missing,
                    code: c.missing_code.into(),
                    message: msg,
                    loc,
                    path,
                    help: resolvable.then(|| "run `scene-render resolve` to make the cache and pin its digest".into()),
                    label: None,
                    measured: None,
                });
                continue;
            }
            Err(e) => {
                out.push(Diagnostic::error(
                    unreadable,
                    format!("@{} of <{ename}>: cannot read {shown}: {e}", c.attr),
                    loc,
                    path,
                ));
                continue;
            }
            Ok(m) if m.is_dir() => {
                out.push(Diagnostic::error(
                    unreadable,
                    format!("@{} of <{ename}>: {shown} is a directory, not a file", c.attr),
                    loc,
                    path,
                ));
                continue;
            }
            Ok(_) => {}
        }
        if ename == "image" && c.attr == "src" {
            if let Some(Some((w, h))) = sizes.get(&c.path) {
                let declared = |name| c.node.attribute(name).and_then(|s| s.trim().parse::<u64>().ok());
                if let (Some(dw), Some(dh)) = (declared("width"), declared("height")) {
                    if (dw, dh) != (*w as u64, *h as u64) {
                        out.push(Diagnostic::warning(
                            "A07",
                            format!("image {shown} declares {dw}x{dh}, but the file is {w}x{h} (resolved to {})", c.path.display()),
                            attr_loc(c.node, "width"),
                            path.clone(),
                        ).with_help("set width and height to the source image dimensions; use the layer's boxWidth, boxHeight and fit to size it in the scene"));
                    }
                }
            }
        }
        if ename == "representation" && c.attr == "src" {
            let file = sizes.get(&c.path).copied().flatten().or_else(|| {
                let video = probes.get(&c.path)?.as_ref()?.video.as_ref()?;
                let declared_rotation =
                    c.node.parent_element()?.attribute("rotation").and_then(|s| s.parse::<i32>().ok()).unwrap_or(0);
                let rotation = if declared_rotation == 0 { video.rotation } else { declared_rotation };
                Some(if rotation.rem_euclid(180) == 90 {
                    (video.height, video.width)
                } else {
                    (video.width, video.height)
                })
            });
            representation_dimensions(c.node, file, &shown, out);
        }
        if ename == "audio" && c.attr == "src" {
            if let Some(Some(info)) = probes.get(&c.path) {
                audio_declarations(c.node, info, &shown, out);
            }
        }
        let Some(expected) = c.expected else { continue };
        match &digests[&c.path] {
            Ok(actual) if *actual == expected => {}
            Ok(actual) => out.push(
                Diagnostic::error(
                    if module { "PRG10" } else { "A02" },
                    format!(
                        "{shown} does not match @{} of <{ename}>: expected {}, found {}",
                        c.hash_attr,
                        hex(&expected),
                        hex(actual)
                    ),
                    attr_loc(c.node, c.hash_attr),
                    path,
                )
                .with_help(if matches!(ename, "generated" | "captionTrack" | "tiles") {
                    "run `scene-render resolve` to make the cache and pin its digest"
                } else {
                    "the file changed since the digest was recorded; regenerate it or update the digest"
                }),
            ),
            Err(e) => out.push(Diagnostic::error(unreadable, format!("cannot read {shown}: {e}"), loc, path)),
        }
    }
}

/// Whether `c` is the `src` of an image, or of a representation of one: the files whose dimensions are probed.
fn is_image_source(c: &Check) -> bool {
    c.attr == "src"
        && match c.node.tag_name().name() {
            "image" => true,
            "representation" => c.node.parent_element().is_some_and(|p| p.tag_name().name() == "image"),
            _ => false,
        }
}

fn declared_u64(n: Node, attr: &str) -> Option<u64> {
    n.attribute(attr).and_then(|s| s.trim().parse::<u64>().ok())
}

/// A08: representation dimensions describe its own file; layout still uses the asset's size.
fn representation_dimensions(n: Node, file: Option<(u32, u32)>, shown: &str, out: &mut Vec<Diagnostic>) {
    let Some(asset) = n.parent_element().filter(|p| matches!(p.tag_name().name(), "image" | "video")) else { return };
    let (dw, dh) = (declared_u64(n, "width"), declared_u64(n, "height"));
    let name = n.attribute("name").unwrap_or_default();
    let path = element_path(n);
    let attr = if dw.is_some() {
        "width"
    } else if dh.is_some() {
        "height"
    } else {
        "src"
    };
    if let Some((w, h)) = file {
        if dw.is_some_and(|d| d != w as u64) || dh.is_some_and(|d| d != h as u64) {
            let width = dw.map_or_else(|| "?".into(), |d| d.to_string());
            let height = dh.map_or_else(|| "?".into(), |d| d.to_string());
            out.push(
                Diagnostic::warning(
                    "A08",
                    format!(
                        "representation {name:?} of {} {} declares {width}x{height}, but the file {shown} is {w}x{h}",
                        asset.tag_name().name(),
                        asset.attribute("id").unwrap_or_default()
                    ),
                    attr_loc(n, attr),
                    path.clone(),
                )
                .with_help("set each declared dimension to the representation file's dimension"),
            );
        }
    }
    let dimensions = dw.or(file.map(|f| f.0 as u64)).zip(dh.or(file.map(|f| f.1 as u64)));
    if let (Some((rw, rh)), Some(aw), Some(ah)) =
        (dimensions, declared_u64(asset, "width"), declared_u64(asset, "height"))
    {
        if !keeps_aspect(rw, rh, aw, ah) {
            out.push(Diagnostic::warning("A08",
                format!("representation {name:?} is {rw}x{rh}, a different aspect ratio from its asset's {aw}x{ah}; layout uses the asset's size"),
                attr_loc(n, attr), path)
                .with_help("keep the asset's aspect ratio when making a representation"));
        }
    }
}

/// A09: the sample rate, channel count and duration an audio asset declares are those the probe reads from its file
/// (the probe wins; a declaration the file contradicts is a warning).
fn audio_declarations(n: Node, info: &sr_media::MediaInfo, shown: &str, out: &mut Vec<Diagnostic>) {
    let Some(stream) = info.audio.first() else { return };
    let path = element_path(n);
    let mut warn = |attr: &str, declared: String, actual: String| {
        out.push(
            Diagnostic::warning(
                "A09",
                format!("@{attr} of <audio> {shown} declares {declared}, but the file has {actual}"),
                attr_loc(n, attr),
                path.clone(),
            )
            .with_help("the file wins when they differ: correct the declaration"),
        );
    };
    if let Some(d) = declared_u64(n, "sampleRate").filter(|d| *d != stream.sample_rate as u64) {
        warn("sampleRate", d.to_string(), stream.sample_rate.to_string());
    }
    if let Some(d) = declared_u64(n, "channels").filter(|d| *d != stream.channels as u64) {
        warn("channels", d.to_string(), stream.channels.to_string());
    }
    let duration = n.attribute("duration").and_then(|s| s.trim().parse::<f64>().ok());
    if let Some(d) = duration.filter(|d| info.duration > 0.0 && (d - info.duration).abs() > AUDIO_DURATION_TOLERANCE) {
        warn("duration", format!("{d} s"), format!("{:.3} s", info.duration));
    }
}

fn sequence(n: Node, base_dir: &Path, out: &mut Vec<Diagnostic>) {
    let kind = if n.has_tag_name("volume") {
        "volume"
    } else if n.has_tag_name("meshSequence") {
        "mesh"
    } else {
        "image"
    };
    let Some(src) = n.attribute("src") else { return };
    let int = |k: &str| {
        n.attribute(k).and_then(|v| crate::xsd::parse_xsd_integer(v.trim())).and_then(|v| i64::try_from(v).ok())
    };
    let (Some(first), Some(last)) = (int("first"), int("last")) else { return };
    let step = int("step").unwrap_or(1).max(1);
    let loc = attr_loc(n, "src");
    let path = element_path(n);
    if last < first {
        out.push(Diagnostic::error(
            "A04",
            format!("{kind} sequence runs from frame {first} to {last}; last precedes first"),
            loc,
            path,
        ));
        return;
    }
    if sequence_frame(src, first).is_none() {
        out.push(Diagnostic::error(
            "A04",
            format!(
                "{kind} sequence src {src:?} has no frame placeholder (%0Nd, %d or ####) of at most \
                 {MAX_FRAME_DIGITS} digits"
            ),
            loc,
            path,
        ));
        return;
    }
    let (first_w, step_w) = (first as i128, step as i128);
    let count = (last as i128 - first_w) / step_w + 1;
    if count > MAX_SEQUENCE_FRAMES {
        out.push(Diagnostic::warning(
            "A04",
            format!("{kind} sequence has {count} frames; frames were not checked individually"),
            loc,
            path,
        ));
        return;
    }
    let frames: Vec<i64> = (0..count).map(|i| (first_w + i * step_w) as i64).collect();
    let missing: Vec<i64> = frames
        .par_iter()
        .filter(|f| match resolve(&sequence_frame(src, **f).unwrap_or_default(), base_dir) {
            Resolved::Local(p) => !p.is_file(),
            Resolved::Remote(_) => false,
        })
        .copied()
        .collect();
    if let Some(Resolved::Remote(scheme)) = sequence_frame(src, first).map(|f| resolve(&f, base_dir)) {
        out.push(Diagnostic::warning(
            "A03",
            format!("{kind} sequence is a {scheme} URI and was not verified offline"),
            loc,
            path,
        ));
        return;
    }
    if missing.is_empty() {
        return;
    }
    let mut runs: BTreeMap<i64, i64> = BTreeMap::new();
    let mut prev: Option<i64> = None;
    let mut start = 0;
    for &f in &missing {
        match prev {
            Some(p) if p.checked_add(step) == Some(f) => {}
            _ => start = f,
        }
        runs.insert(start, f);
        prev = Some(f);
    }
    let shown: Vec<String> =
        runs.iter().take(5).map(|(a, b)| if a == b { a.to_string() } else { format!("{a}–{b}") }).collect();
    let more = if runs.len() > 5 { ", …" } else { "" };
    let policy = n.attribute("missingFrame").unwrap_or("error");
    let msg = format!("{kind} sequence is missing {} of {count} frames ({}{more})", missing.len(), shown.join(", "));
    if policy == "error" || (kind != "image" && policy == "hold" && missing.first() == Some(&first)) {
        out.push(Diagnostic::error("A04", msg, loc, path).with_help(if kind != "image" {
            "add the frames, use transparent, or use hold with an available first frame"
        } else {
            "add the frames or set missingFrame to hold, black or transparent"
        }));
    } else {
        out.push(Diagnostic::warning("A04", format!("{msg}; missingFrame=\"{policy}\" applies"), loc, path));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_patterns() {
        assert_eq!(sequence_frame("f_%04d.png", 7).unwrap(), "f_0007.png");
        assert_eq!(sequence_frame("f_%d.png", 12).unwrap(), "f_12.png");
        assert_eq!(sequence_frame("f_####.exr", 42).unwrap(), "f_0042.exr");
        assert_eq!(sequence_frame("f_%04d.png", -3).unwrap(), "f_-003.png");
        assert!(sequence_frame("plain.png", 1).is_none());
    }

    #[test]
    fn uri_resolution() {
        let base = Path::new("/proj");
        assert_eq!(resolve("media/a%20b.png", base), Resolved::Local(PathBuf::from("/proj/media/a b.png")));
        assert_eq!(resolve("/abs/x.png", base), Resolved::Local(PathBuf::from("/abs/x.png")));
        assert_eq!(resolve("file:///abs/x.png", base), Resolved::Local(PathBuf::from("/abs/x.png")));
        assert_eq!(resolve("https://cdn/x.png", base), Resolved::Remote("https".into()));
        assert!(input_uri_attributes().contains("src"));
        assert!(input_uri_attributes().contains("cache"));
        assert!(!input_uri_attributes().contains("path"));
    }

    #[test]
    fn only_output_report_is_an_output_side_report() {
        assert!(is_output_side("output", "report"), "output/@report names a file the render writes");
        assert!(!is_output_side("captionTrack", "report"));
        assert!(!is_output_side("output", "src"));
    }
}
