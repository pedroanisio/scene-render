//! Loading documents: validation stages, the typed [`Document`] and its ID [`Index`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::diag::{element_path, with_line_index, Diagnostic, LineIndex, Loc, Report};
use crate::model::{
    AssetsChild, AudioMixChild, AudioTrack, Bus, CaptionTrack, DataSource, Effect, Layout, Light, Marker, MarkersChild,
    Material, Node, Output, PaintsChild, Param, ParametersChild, SafeArea, Scene, StylesChild, Symbol, TextStyle,
    Token, TrackData, Variant,
};
use crate::values::{Color, Fps, Paint, Rgba};

/// Document version (`scene/@version`).
pub use crate::model::SceneVersion as Version;

/// Options for loading and validating.
#[derive(Debug, Clone)]
pub struct LoadOptions {
    /// Check that referenced files exist and match their declared SHA-256.
    pub verify_assets: bool,
    /// Directory against which relative URIs resolve. Defaults to the
    /// directory of the loaded file, or the working directory for strings.
    pub base_dir: Option<PathBuf>,
}

impl Default for LoadOptions {
    fn default() -> Self {
        LoadOptions { verify_assets: true, base_dir: None }
    }
}

impl LoadOptions {
    /// Options that skip file verification.
    pub fn without_assets() -> Self {
        LoadOptions { verify_assets: false, base_dir: None }
    }
}

/// Why a document could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// The file could not be read.
    #[error("cannot read {path}: {source}")]
    Io {
        /// File path.
        path: PathBuf,
        /// Cause.
        source: std::io::Error,
    },
    /// The document is invalid; the report lists every problem.
    #[error("{0}")]
    Invalid(Report),
}

impl LoadError {
    /// The validation report, when the document was read but is invalid.
    pub fn report(&self) -> Option<&Report> {
        match self {
            LoadError::Invalid(r) => Some(r),
            LoadError::Io { .. } => None,
        }
    }
}

// ------------------------------------------------------------------ index

/// Root of a node tree: the composition or a symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
pub enum NodeRoot {
    /// `/scene/composition`.
    Composition,
    /// `/scene/symbols/symbol[i]`.
    Symbol(u32),
}

/// Location of a node: its root and the child-node indices leading to it.
/// The first step indexes the root's node list; each further step indexes
/// [`Node::child_nodes`] of the previous node.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
pub struct NodePath {
    /// Tree root.
    pub root: NodeRoot,
    /// Child-node indices from the root.
    pub steps: Vec<u32>,
}

/// What an `xs:ID` names. Indices address the vector that holds the element
/// in [`Scene`] (for sections with mixed children, the section's `children`).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[allow(missing_docs)]
pub enum Target {
    Asset(usize),
    Paint(usize),
    TextStyle(usize),
    Material(usize),
    Symbol(usize),
    Marker(usize),
    Effect(usize),
    Light(usize),
    TrackData(usize),
    Param(usize),
    DataSource(usize),
    Variant(usize),
    Layout(usize),
    SafeArea(usize),
    Output(usize),
    AudioTrack(usize),
    Bus(usize),
    CaptionTrack(usize),
    Node(NodePath),
    /// Any other element carrying an id (masks, bones, cues, …).
    Element {
        /// Element name.
        name: String,
        /// Source position.
        loc: Loc,
    },
}

/// Maps every id of the document, and every style token name, to its element.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Index {
    ids: HashMap<String, Target>,
    tokens: HashMap<String, usize>,
}

impl Index {
    fn build(scene: &Scene, ids: crate::xsd::structure::IdMap) -> Index {
        let mut ix = Index::default();
        let mut put = |id: Option<&str>, t: Target| {
            if let Some(id) = id {
                ix.ids.insert(id.to_string(), t);
            }
        };
        if let Some(a) = &scene.assets {
            a.children.iter().enumerate().for_each(|(i, c)| put(c.id(), Target::Asset(i)));
        }
        if let Some(p) = &scene.paints {
            p.children.iter().enumerate().for_each(|(i, c)| put(c.id(), Target::Paint(i)));
        }
        if let Some(s) = &scene.styles {
            for (i, c) in s.children.iter().enumerate() {
                match c {
                    StylesChild::TextStyle(t) => put(Some(&t.id), Target::TextStyle(i)),
                    StylesChild::Token(t) => {
                        ix.tokens.entry(t.name.clone()).or_insert(i);
                    }
                }
            }
        }
        if let Some(m) = &scene.materials {
            m.materials.iter().enumerate().for_each(|(i, c)| put(c.id(), Target::Material(i)));
        }
        if let Some(m) = &scene.markers {
            m.children.iter().enumerate().for_each(|(i, c)| put(c.id(), Target::Marker(i)));
        }
        if let Some(e) = &scene.effects {
            e.effects.iter().enumerate().for_each(|(i, c)| put(c.id(), Target::Effect(i)));
        }
        if let Some(l) = &scene.lights {
            l.lights.iter().enumerate().for_each(|(i, c)| put(c.id(), Target::Light(i)));
        }
        if let Some(t) = &scene.tracking {
            t.track_data.iter().enumerate().for_each(|(i, c)| put(c.id(), Target::TrackData(i)));
        }
        if let Some(p) = &scene.parameters {
            for (i, c) in p.children.iter().enumerate() {
                match c {
                    ParametersChild::Param(x) => put(x.id(), Target::Param(i)),
                    ParametersChild::Data(x) => put(x.id(), Target::DataSource(i)),
                    ParametersChild::Variant(x) => put(x.id(), Target::Variant(i)),
                    ParametersChild::Bind(_) => {}
                    // data programs are replaced by their rows before the model is built (SREP 66)
                    ParametersChild::Program(_) => {}
                }
            }
        }
        if let Some(l) = &scene.layouts {
            l.layouts.iter().enumerate().for_each(|(i, c)| put(c.id(), Target::Layout(i)));
        }
        if let Some(s) = &scene.safe_areas {
            s.safe_areas.iter().enumerate().for_each(|(i, c)| put(c.id(), Target::SafeArea(i)));
        }
        scene.outputs.iter().enumerate().for_each(|(i, c)| put(c.id(), Target::Output(i)));
        if let Some(m) = &scene.audio_mix {
            for (i, c) in m.children.iter().enumerate() {
                match c {
                    AudioMixChild::AudioTrack(x) => put(x.id(), Target::AudioTrack(i)),
                    AudioMixChild::Bus(x) => put(x.id(), Target::Bus(i)),
                    AudioMixChild::Master(_) => {}
                }
            }
        }
        if let Some(c) = &scene.captions {
            c.caption_tracks.iter().enumerate().for_each(|(i, t)| put(t.id(), Target::CaptionTrack(i)));
        }
        if let Some(s) = &scene.symbols {
            s.symbols.iter().enumerate().for_each(|(i, c)| put(c.id(), Target::Symbol(i)));
        }
        fn walk(
            nodes: &mut dyn Iterator<Item = &Node>,
            root: NodeRoot,
            prefix: &mut Vec<u32>,
            put: &mut dyn FnMut(Option<&str>, Target),
        ) {
            for (i, n) in nodes.enumerate() {
                prefix.push(i as u32);
                put(n.id(), Target::Node(NodePath { root, steps: prefix.clone() }));
                walk(&mut n.child_nodes(), root, prefix, put);
                prefix.pop();
            }
        }
        walk(&mut scene.composition.children.iter(), NodeRoot::Composition, &mut Vec::new(), &mut put);
        if let Some(s) = &scene.symbols {
            for (si, sym) in s.symbols.iter().enumerate() {
                walk(&mut sym.children.iter(), NodeRoot::Symbol(si as u32), &mut Vec::new(), &mut put);
            }
        }
        for (id, (name, loc)) in ids {
            ix.ids.entry(id).or_insert(Target::Element { name, loc });
        }
        ix
    }

    /// What `id` names.
    pub fn get(&self, id: &str) -> Option<&Target> {
        self.ids.get(id)
    }

    /// Number of indexed ids.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// True when the document has no ids.
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Every id, unordered.
    pub fn ids(&self) -> impl Iterator<Item = (&str, &Target)> {
        self.ids.iter().map(|(k, v)| (k.as_str(), v))
    }
}

// ------------------------------------------------------------------ document

/// A validated, typed and indexed scene-render document.
#[derive(Debug, Clone)]
pub struct Document {
    /// The typed model.
    pub scene: Scene,
    index: Index,
    base_dir: PathBuf,
    warnings: Vec<Diagnostic>,
    /// SHA-256 of the document's bytes as loaded.
    source_sha256: [u8; 32],
    /// Namespaced id → local id of every element a build-mode program emitted (SREP 87).
    program_ids: HashMap<String, String>,
}

/// A paint resolved against the document.
#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedPaint<'d> {
    /// A solid colour.
    Solid(Rgba),
    /// A gradient, pattern or mesh gradient from `<paints>`.
    Paint(&'d PaintsChild),
}

macro_rules! lookup {
    ($(#[$doc:meta] $fn:ident -> $ty:ty, $variant:ident, |$s:ident, $i:ident| $get:expr;)*) => {$(
        #[$doc]
        pub fn $fn(&self, id: &str) -> Option<&$ty> {
            match self.index.get(id)? {
                Target::$variant($i) => {
                    let $i = *$i;
                    let $s = &self.scene;
                    $get
                }
                _ => None,
            }
        }
    )*};
}

impl Document {
    /// SHA-256 of the document's bytes as loaded (the identity of its source, for caches keyed by content).
    pub fn source_sha256(&self) -> [u8; 32] {
        self.source_sha256
    }

    /// The id `P.E` of every element that the build-mode program `P` emitted as `E`, mapped to `E` (SREP 87).
    /// Values the format derives from an element's id use `E` (Semantics 7).
    pub fn program_ids(&self) -> &HashMap<String, String> {
        &self.program_ids
    }

    /// Document version.
    pub fn version(&self) -> Version {
        self.scene.version
    }

    /// Directory against which relative URIs resolve.
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    /// Warnings found while loading.
    pub fn warnings(&self) -> &[Diagnostic] {
        &self.warnings
    }

    /// The ID index.
    pub fn index(&self) -> &Index {
        &self.index
    }

    /// What `id` names.
    pub fn target(&self, id: &str) -> Option<&Target> {
        self.index.get(id)
    }

    /// Project frame rate.
    pub fn fps(&self) -> Fps {
        self.scene.project.fps
    }

    /// Project duration in seconds.
    pub fn duration(&self) -> f64 {
        self.scene.project.duration.get()
    }

    /// Frames in the project timeline.
    pub fn frame_count(&self) -> u64 {
        self.fps().frame_count(self.duration())
    }

    /// Project frame size in pixels.
    pub fn frame_size(&self) -> (u64, u64) {
        (self.scene.project.width, self.scene.project.height)
    }

    lookup! {
        /// Asset by id.
        asset -> AssetsChild, Asset, |s, i| s.assets.as_ref()?.children.get(i);
        /// Paint (gradient, pattern, mesh gradient) by id.
        paint -> PaintsChild, Paint, |s, i| s.paints.as_ref()?.children.get(i);
        /// Material by id.
        material -> Material, Material, |s, i| s.materials.as_ref()?.materials.get(i);
        /// Symbol by id.
        symbol -> Symbol, Symbol, |s, i| s.symbols.as_ref()?.symbols.get(i);
        /// Effect by id.
        effect -> Effect, Effect, |s, i| s.effects.as_ref()?.effects.get(i);
        /// Light by id.
        light -> Light, Light, |s, i| s.lights.as_ref()?.lights.get(i);
        /// Tracking data by id.
        track_data -> TrackData, TrackData, |s, i| s.tracking.as_ref()?.track_data.get(i);
        /// Layout by id.
        layout -> Layout, Layout, |s, i| s.layouts.as_ref()?.layouts.get(i);
        /// Safe area by id.
        safe_area -> SafeArea, SafeArea, |s, i| s.safe_areas.as_ref()?.safe_areas.get(i);
        /// Output by id.
        output -> Output, Output, |s, i| s.outputs.get(i);
        /// Caption track by id.
        caption_track -> CaptionTrack, CaptionTrack, |s, i| s.captions.as_ref()?.caption_tracks.get(i);
        /// Text style by id.
        text_style -> TextStyle, TextStyle, |s, i| match s.styles.as_ref()?.children.get(i)? { StylesChild::TextStyle(t) => Some(t), _ => None };
        /// Marker by id.
        marker -> Marker, Marker, |s, i| match s.markers.as_ref()?.children.get(i)? { MarkersChild::Marker(m) => Some(m), _ => None };
        /// Template parameter by id.
        param -> Param, Param, |s, i| match s.parameters.as_ref()?.children.get(i)? { ParametersChild::Param(p) => Some(p), _ => None };
        /// Data source by id.
        data_source -> DataSource, DataSource, |s, i| match s.parameters.as_ref()?.children.get(i)? { ParametersChild::Data(p) => Some(p), _ => None };
        /// Variant by id.
        variant -> Variant, Variant, |s, i| match s.parameters.as_ref()?.children.get(i)? { ParametersChild::Variant(p) => Some(p), _ => None };
        /// Audio track by id.
        audio_track -> AudioTrack, AudioTrack, |s, i| match s.audio_mix.as_ref()?.children.get(i)? { AudioMixChild::AudioTrack(t) => Some(t), _ => None };
        /// Audio bus by id.
        bus -> Bus, Bus, |s, i| match s.audio_mix.as_ref()?.children.get(i)? { AudioMixChild::Bus(b) => Some(b), _ => None };
    }

    /// Composition or symbol node by id.
    pub fn node(&self, id: &str) -> Option<&Node> {
        match self.index.get(id)? {
            Target::Node(p) => self.node_at(p),
            _ => None,
        }
    }

    /// Node at a path.
    pub fn node_at(&self, path: &NodePath) -> Option<&Node> {
        let roots: &[Node] = match path.root {
            NodeRoot::Composition => &self.scene.composition.children,
            NodeRoot::Symbol(i) => &self.scene.symbols.as_ref()?.symbols.get(i as usize)?.children,
        };
        let (first, rest) = path.steps.split_first()?;
        roots.get(*first as usize)?.descend(rest)
    }

    /// Every composition node in depth-first document order, with its depth.
    pub fn composition_nodes(&self) -> Vec<(usize, &Node)> {
        fn walk<'a>(n: &'a Node, d: usize, out: &mut Vec<(usize, &'a Node)>) {
            out.push((d, n));
            n.child_nodes().for_each(|c| walk(c, d + 1, out));
        }
        let mut out = Vec::new();
        self.scene.composition.children.iter().for_each(|n| walk(n, 0, &mut out));
        out
    }

    /// Style token by name.
    pub fn token(&self, name: &str) -> Option<&Token> {
        let i = *self.index.tokens.get(name)?;
        match self.scene.styles.as_ref()?.children.get(i)? {
            StylesChild::Token(t) => Some(t),
            _ => None,
        }
    }

    /// Resolves a colour, following `var(--name)` token references (up to
    /// eight levels). `None` when a token is missing, cyclic or not a colour.
    pub fn resolve_color(&self, c: &Color) -> Option<Rgba> {
        let mut cur = c.clone();
        for _ in 0..8 {
            match cur {
                Color::Rgba(rgba) => return Some(rgba),
                Color::Token(name) => {
                    let t = self.token(&name)?;
                    cur = <Color as crate::parse::ParseValue>::parse_value(t.value.trim()).ok()?;
                }
            }
        }
        None
    }

    /// Resolves a paint to a solid colour or a `<paints>` element.
    pub fn resolve_paint(&self, p: &Paint) -> Option<ResolvedPaint<'_>> {
        match p {
            Paint::Color(c) => self.resolve_color(c).map(ResolvedPaint::Solid),
            Paint::Ref(r) => self.paint(&r.0).map(ResolvedPaint::Paint),
        }
    }
}

// ------------------------------------------------------------------ pipeline

struct Outcome {
    report: Report,
    document: Option<Document>,
}

fn run(xml: &str, base_dir: PathBuf, opts: &LoadOptions, build: bool) -> Outcome {
    let lines = LineIndex::new(xml);
    with_line_index(&lines, || {
        let mut report = Report::default();
        // the parser descends recursively: a document nested past the limit is refused before it runs
        if let Some(at) = crate::limits::too_deep(xml, crate::limits::MAX_DEPTH) {
            let message = format!("elements are nested more than {} levels deep", crate::limits::MAX_DEPTH);
            report.diagnostics.push(Diagnostic::error("S12", message, Loc::at(at), "/"));
            return Outcome { report, document: None };
        }
        let doc = match roxmltree::Document::parse_with_options(xml, crate::limits::parsing_options()) {
            Ok(d) => d,
            Err(roxmltree::Error::NodesLimitReached) => {
                let message = format!("the document has more than {} XML nodes", crate::limits::MAX_XML_NODES);
                report.diagnostics.push(Diagnostic::error("XML", message, Loc { line: 1, column: 1, offset: 0 }, "/"));
                return Outcome { report, document: None };
            }
            Err(e) => {
                let p = e.pos();
                let line_start: usize =
                    xml.split_inclusive('\n').take(p.row.saturating_sub(1) as usize).map(str::len).sum();
                let col_bytes: usize =
                    lines.line_text(p.row).chars().take(p.col.saturating_sub(1) as usize).map(char::len_utf8).sum();
                let loc = Loc { line: p.row, column: p.col, offset: (line_start + col_bytes) as u32 };
                report.diagnostics.push(Diagnostic::error("XML", format!("not well-formed XML: {e}"), loc, "/"));
                return Outcome { report, document: None };
            }
        };
        let (ids, types, unresolved) = crate::xsd::structure::validate_deferring_refs(&doc, &mut report.diagnostics);
        // SREP 87, Semantics 4: an xs:IDREF may name an id that a build-mode program emits, which only running the
        // programs tells; its S10 waits for them
        let deferred = if build && crate::program::has_node_programs(&doc) {
            unresolved
        } else {
            report.diagnostics.extend(unresolved.into_iter().map(|u| u.diagnostic));
            Vec::new()
        };
        crate::rules::validate(&doc, &mut report.diagnostics);
        crate::inert::validate(&doc, &types, &mut report.diagnostics);
        if opts.verify_assets {
            crate::assets::verify(&doc, &base_dir, &mut report.diagnostics);
        }
        // the other errors of an element that holds such a reference wait with it (R8: a matte names a node); the
        // document with the outputs in place is validated again in full
        let holders: std::collections::HashSet<String> = deferred.iter().map(|u| u.diagnostic.path.clone()).collect();
        let (waiting, rest): (Vec<Diagnostic>, Vec<Diagnostic>) =
            report.diagnostics.drain(..).partition(|d| d.is_error() && holders.contains(&d.path));
        report.diagnostics = rest;
        report.sort();
        if !build || report.has_errors() {
            report.diagnostics.extend(deferred.into_iter().map(|u| u.diagnostic));
            report.diagnostics.extend(waiting);
            report.sort();
            return Outcome { report, document: None };
        }
        // SREP 66: build-mode programs run now, and the document is loaded again with their output in place
        match crate::program::expand(&doc, xml, &base_dir) {
            None => {}
            Some(Err(errors)) => {
                report.diagnostics.extend(errors);
                report.sort();
                return Outcome { report, document: None };
            }
            Some(Ok(expanded)) => {
                let still: Vec<Diagnostic> = deferred
                    .into_iter()
                    .filter(|u| !expanded.resolved.contains(&(u.attr_offset, u.value.clone())))
                    .map(|u| u.diagnostic)
                    .collect();
                if !still.is_empty() {
                    report.diagnostics.extend(still);
                    report.diagnostics.extend(waiting);
                    report.diagnostics.extend(expanded.warnings);
                    report.sort();
                    return Outcome { report, document: None };
                }
                let mut inner = run(&expanded.xml, base_dir.clone(), opts, true);
                if inner.document.is_none() {
                    // the output is invalid in place: every error is the output's (the document was valid without
                    // it), except an error of an element whose errors waited for its reference (SREP 87): that one
                    // is the element's, reported where the element is written
                    let mut mapped = Report::default();
                    for d in inner.report.diagnostics.iter().filter(|d| d.severity == crate::diag::Severity::Error) {
                        let at = d.loc.offset as usize;
                        let inside = expanded.outputs.iter().any(|o| o.0.contains(&at));
                        let holder = if holders.is_empty() || inside { None } else { expanded.origin.orig(at) }
                            .and_then(|o| {
                                let e = doc
                                    .descendants()
                                    .filter(|n| n.is_element() && n.range().contains(&o))
                                    .max_by_key(|n| n.range().start)?;
                                let path = element_path(e);
                                holders.contains(&path).then_some((o, path))
                            });
                        if let Some((o, path)) = holder {
                            mapped.diagnostics.push(Diagnostic { loc: Loc::at(o), path, ..d.clone() });
                            continue;
                        }
                        let owner = expanded.outputs.iter().find(|o| o.0.contains(&at)).or(expanded.outputs.first());
                        let (id, loc, path) =
                            owner.map(|o| (o.1.as_str(), o.2, o.3.clone())).unwrap_or(("", d.loc, d.path.clone()));
                        mapped.diagnostics.push(Diagnostic::error(
                            "PRG14",
                            format!("program {id:?}: its output is not valid in place: {} {}", d.code, d.message),
                            loc,
                            path,
                        ));
                    }
                    mapped.sort();
                    inner.report = mapped;
                }
                inner.report.diagnostics.extend(expanded.warnings.iter().cloned());
                inner.report.sort();
                if let Some(d) = inner.document.as_mut() {
                    d.warnings.extend(expanded.warnings);
                    d.program_ids = expanded.local_ids;
                }
                return inner;
            }
        }
        match Scene::build(&doc) {
            Ok(scene) => {
                let index = Index::build(&scene, ids);
                let warnings = report.diagnostics.clone();
                let source_sha256 =
                    sha2::Digest::finalize(<sha2::Sha256 as sha2::Digest>::new_with_prefix(xml.as_bytes())).into();
                Outcome {
                    report,
                    document: Some(Document {
                        scene,
                        index,
                        base_dir,
                        warnings,
                        source_sha256,
                        program_ids: HashMap::new(),
                    }),
                }
            }
            Err(e) => {
                let path = element_path(doc.root_element());
                report.diagnostics.push(Diagnostic::error("M01", e.message, e.loc, path));
                Outcome { report, document: None }
            }
        }
    })
}

fn base_for(opts: &LoadOptions, file: Option<&Path>) -> PathBuf {
    if let Some(b) = &opts.base_dir {
        return b.clone();
    }
    match file.and_then(Path::parent) {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    }
}

/// The report of a document past [`crate::limits::MAX_DOCUMENT_BYTES`], which is refused unparsed.
fn oversize() -> Report {
    let message = format!("the document is larger than {} bytes", crate::limits::MAX_DOCUMENT_BYTES);
    Report { diagnostics: vec![Diagnostic::error("XML", message, Loc { line: 1, column: 1, offset: 0 }, "/")] }
}

/// Runs the pipeline over a document a caller handed in, refusing one past the size limit first.
fn run_checked(xml: &str, base_dir: PathBuf, opts: &LoadOptions, build: bool) -> Outcome {
    if xml.len() as u64 > crate::limits::MAX_DOCUMENT_BYTES {
        return Outcome { report: oversize(), document: None };
    }
    run(xml, base_dir, opts, build)
}

/// Reads a scene document's text. A file past [`crate::limits::MAX_DOCUMENT_BYTES`] is refused before its content
/// is read, as an invalid document whose report holds one `XML` error; a file that cannot be read, or is not UTF-8,
/// is an I/O error.
pub fn read_document(path: impl AsRef<Path>) -> Result<String, LoadError> {
    let path = path.as_ref();
    let bytes = match crate::limits::read_capped(path, crate::limits::MAX_DOCUMENT_BYTES) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::FileTooLarge => return Err(LoadError::Invalid(oversize())),
        Err(source) => return Err(LoadError::Io { path: path.to_path_buf(), source }),
    };
    String::from_utf8(bytes).map_err(|e| LoadError::Io {
        path: path.to_path_buf(),
        source: std::io::Error::new(std::io::ErrorKind::InvalidData, format!("not UTF-8: {e}")),
    })
}

/// Validates a document held in memory. Every stage runs; the report lists
/// errors and warnings in document order.
pub fn validate_str(xml: &str, opts: &LoadOptions) -> Report {
    run_checked(xml, base_for(opts, None), opts, false).report
}

/// Validates a file.
pub fn validate_file(path: impl AsRef<Path>, opts: &LoadOptions) -> Result<Report, LoadError> {
    let path = path.as_ref();
    let xml = match read_document(path) {
        Ok(x) => x,
        Err(LoadError::Invalid(report)) => return Ok(report),
        Err(e) => return Err(e),
    };
    Ok(run(&xml, base_for(opts, Some(path)), opts, false).report)
}

/// Loads a document held in memory into the typed model.
pub fn load_str(xml: &str, opts: &LoadOptions) -> Result<Document, LoadError> {
    let o = run_checked(xml, base_for(opts, None), opts, true);
    o.document.ok_or(LoadError::Invalid(o.report))
}

/// Loads a file into the typed model.
pub fn load_file(path: impl AsRef<Path>, opts: &LoadOptions) -> Result<Document, LoadError> {
    let path = path.as_ref();
    let xml = read_document(path)?;
    let o = run(&xml, base_for(opts, Some(path)), opts, true);
    o.document.ok_or(LoadError::Invalid(o.report))
}
