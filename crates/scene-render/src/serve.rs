//! `scene-render serve`: a render worker that takes chunks of video outputs from `encode --workers` over the
//! network.
//!
//! The worker listens on a TCP address and speaks the protocol of [`crate::net`]. A coordinator that holds the
//! same token sends the files of a scene (each once: they are kept by their SHA-256), opens the scene, and asks
//! for chunks of its outputs; the worker renders each with [`sr_deliver::remote::Segment`] and sends the file back.
//! It renders one chunk at a time, on one GPU device it keeps between chunks, and keeps the scenes it has loaded
//! in memory, so a second chunk of the same scene, from the same encode or a later one, does not load it again.
//!
//! What a coordinator sends is untrusted until its messages verify, and bounded after: file names are relative,
//! without `..`, and land under the scene's own directory in the cache; a scene holds at most [`MAX_FILES`] files;
//! nothing a coordinator names is run, only read as a scene document and its assets. The worker never writes outside
//! its cache directory.
//!
//! Requests: `have` (which of these digests are missing), `put` (a file, by digest), `open` (a scene: its document
//! and files by relative path and digest), `render` (a chunk of an output of an open scene; `progress` notifications
//! follow, then the answer with the file). The link closing ends the session, and stops a render at its next frame.

use std::collections::BTreeMap;
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};

use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::net::{self, answer, is_digest, notice, refusal, Link, LinkError, Side};

/// The most files one scene names.
pub const MAX_FILES: usize = 100_000;
/// The longest relative path of a scene's file, in bytes.
pub const MAX_PATH: usize = 1_024;
/// Scenes kept loaded in memory.
const LOADED: usize = 2;
/// Bytes of files a worker keeps when no limit is given: past it, the files used longest ago go first.
pub const DEFAULT_CACHE_BYTES: u64 = 32 << 30;
/// Days a scene's directory is kept after it was last opened.
const SCENE_DAYS: u64 = 7;
/// The address a worker listens on when none is given: this machine only.
pub const DEFAULT_LISTEN: &str = "127.0.0.1:7878";

/// One file of a scene.
#[derive(Debug, Clone, Deserialize, serde::Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct SceneFile {
    /// Where it lies, relative to the scene's root, with `/` between names.
    pub path: String,
    /// Its SHA-256, hexadecimal.
    pub sha256: String,
}

/// A scene as a coordinator describes it: the document and every file it names.
#[derive(Debug, Clone, Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SceneManifest {
    /// The document's path among `files`.
    pub document: String,
    /// The document and the files it names.
    pub files: Vec<SceneFile>,
}

impl SceneManifest {
    /// The scene's name on a worker: the SHA-256 of its document path and its files, sorted.
    pub fn id(&self) -> String {
        let mut files = self.files.clone();
        files.sort();
        let mut h = Sha256::new();
        h.update(self.document.as_bytes());
        for f in &files {
            h.update([0]);
            h.update(f.path.as_bytes());
            h.update([0]);
            h.update(f.sha256.as_bytes());
        }
        net::hex(&h.finalize())
    }
}

/// A chunk of an output, as a coordinator asks for it.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct RenderRequest {
    /// The scene, as `open` named it.
    pub scene: String,
    /// The id of the document's output, or none for an ad-hoc one.
    pub output: Option<String>,
    /// The file type of the chunk (the extension of the output's path).
    pub extension: String,
    /// The codec of an ad-hoc output.
    pub codec: String,
    /// The output's range, seconds.
    pub start: f64,
    /// The end of the range.
    pub end: f64,
    /// Parameter values.
    pub params: Vec<(String, String)>,
    /// Data row and its source.
    pub row: Option<(Option<String>, usize)>,
    /// `draft`, `preview` or `final`; none for the document's own.
    pub quality: Option<String>,
    /// Preferred asset representation.
    pub representation: Option<String>,
    /// Noise target of adaptive path sampling.
    pub path_noise: Option<f32>,
    /// Encoder family (`software`, `nvenc`, …).
    pub hardware: String,
    /// The chunk.
    pub part: sr_deliver::remote::Part,
}

/// The name of a quality tier.
pub fn quality_name(q: sr_model::model::ProjectQuality) -> &'static str {
    use sr_model::model::ProjectQuality as Q;
    match q {
        Q::Draft => "draft",
        Q::Preview => "preview",
        Q::Final => "final",
    }
}

fn quality_of(name: &str) -> Option<sr_model::model::ProjectQuality> {
    use sr_model::model::ProjectQuality as Q;
    Some(match name {
        "draft" => Q::Draft,
        "preview" => Q::Preview,
        "final" => Q::Final,
        _ => return None,
    })
}

/// The name of an encoder family, as `--hw` spells it.
pub fn hardware_name(h: sr_media::encode::Hardware) -> &'static str {
    use sr_media::encode::Hardware as H;
    match h {
        H::Auto => "auto",
        H::Software => "software",
        H::Nvenc => "nvenc",
        H::VideoToolbox => "videotoolbox",
        H::Vaapi => "vaapi",
        H::Qsv => "qsv",
        H::Amf => "amf",
    }
}

/// `path` as a relative path of plain names, or why it is not one.
pub fn safe_relative(path: &str) -> Result<PathBuf, String> {
    if path.is_empty() || path.len() > MAX_PATH || path.contains('\0') || path.contains('\\') {
        return Err(format!("{path:?} is not a file name a scene may have"));
    }
    let p = Path::new(path);
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::Normal(name) => out.push(name),
            _ => return Err(format!("{path:?} leaves the scene's directory")),
        }
    }
    if out.as_os_str().is_empty() {
        return Err(format!("{path:?} is not a file name a scene may have"));
    }
    Ok(out)
}

/// What the workers of this process share.
struct Shared {
    token: String,
    cache: PathBuf,
    /// Bytes of files kept.
    cache_bytes: u64,
    /// One chunk renders at a time; the device is made for the first and kept.
    gpu: Mutex<Option<sr_gpu::Gpu>>,
    /// Scenes loaded, most recently used last.
    loaded: Mutex<Vec<(String, Arc<sr_model::Document>)>>,
    /// Counts the chunk files, so two never share a name.
    chunk: AtomicU64,
    quiet: bool,
}

impl Shared {
    fn say(&self, line: impl AsRef<str>) {
        if !self.quiet {
            eprintln!("{}", line.as_ref());
        }
    }

    fn file(&self, digest: &str) -> PathBuf {
        self.cache.join("files").join(digest)
    }

    fn scene_dir(&self, id: &str) -> PathBuf {
        self.cache.join("scenes").join(id)
    }
}

/// What a worker says of itself in answer to `hello`.
fn about() -> Value {
    let ffmpeg = std::process::Command::new(sr_media::ffmpeg())
        .arg("-version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).lines().next().map(str::to_string))
        .unwrap_or_default();
    json!({
        "protocol": net::PROTOCOL,
        "version": env!("CARGO_PKG_VERSION"),
        "host": std::env::var("HOSTNAME").ok().or_else(|| std::fs::read_to_string("/etc/hostname").ok().map(|s| s.trim().to_string())).unwrap_or_default(),
        "ffmpeg": ffmpeg,
        "document_bytes": sr_model::limits::MAX_DOCUMENT_BYTES,
    })
}

/// Runs the worker on `listen` until the process ends. `once` serves one coordinator and returns.
pub fn run(
    listen: &str,
    cache: Option<PathBuf>,
    cache_bytes: u64,
    token_file: Option<&Path>,
    once: bool,
    quiet: bool,
) -> Result<(), String> {
    let token = net::token(token_file)?;
    let cache = match cache {
        Some(c) => c,
        None => default_cache(),
    };
    for sub in ["files", "scenes", "chunks"] {
        std::fs::create_dir_all(cache.join(sub))
            .map_err(|e| format!("cannot make {}: {e}", cache.join(sub).display()))?;
    }
    // chunks of a run that ended abruptly
    if let Ok(entries) = std::fs::read_dir(cache.join("chunks")) {
        for e in entries.flatten() {
            let _ = std::fs::remove_file(e.path());
        }
    }
    forget_scenes(&cache.join("scenes"), std::time::Duration::from_secs(SCENE_DAYS * 86_400));
    let listener = TcpListener::bind(listen).map_err(|e| format!("cannot listen on {listen}: {e}"))?;
    let shared = Arc::new(Shared {
        token,
        cache,
        cache_bytes,
        gpu: Mutex::new(None),
        loaded: Mutex::new(Vec::new()),
        chunk: AtomicU64::new(0),
        quiet,
    });
    let address = listener.local_addr().map_or_else(|_| listen.to_string(), |a| a.to_string());
    // one line a caller can wait for and read the port from (`--listen 127.0.0.1:0` takes a free one)
    println!(
        "scene-render {} worker listening on {address}, cache {}",
        env!("CARGO_PKG_VERSION"),
        shared.cache.display()
    );
    let loopback = listener.local_addr().is_ok_and(|a| a.ip().is_loopback());
    if !loopback {
        shared.say(
            "note: the link is authenticated, not encrypted: on a network others can read, run it through a tunnel",
        );
    }
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                shared.say(format!("a connection failed: {e}"));
                continue;
            }
        };
        let shared_for = shared.clone();
        let session = std::thread::spawn(move || {
            let peer = stream.peer_addr().map_or_else(|_| "an unknown address".into(), |a| a.to_string());
            match serve(stream, &shared_for) {
                Ok(()) => shared_for.say(format!("{peer}: done")),
                Err(e) => shared_for.say(format!("{peer}: {e}")),
            }
        });
        if once {
            let _ = session.join();
            return Ok(());
        }
    }
    Ok(())
}

/// Where a worker keeps what it is sent when no directory is given.
fn default_cache() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("scene-render").join("worker")
}

/// Removes the scene directories under `scenes` last opened more than `age` ago. Their files stay, by digest.
fn forget_scenes(scenes: &Path, age: std::time::Duration) {
    let Ok(entries) = std::fs::read_dir(scenes) else { return };
    for e in entries.flatten() {
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|d| d > age);
        if old {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// Marks a path as used now.
fn touch(path: &Path) {
    if let Ok(f) = std::fs::File::options().write(true).open(path) {
        let _ = f.set_modified(std::time::SystemTime::now());
    } else if let Ok(f) = std::fs::File::open(path) {
        let _ = f.set_modified(std::time::SystemTime::now());
    }
}

/// Keeps the files under `files` within `limit` bytes: those used longest ago are removed first, never one of
/// `keep` (the scene being opened). A removed file is sent again by the next coordinator that needs it; a scene
/// directory that still links it keeps its own copy of the bytes until it is forgotten.
fn evict(files: &Path, limit: u64, keep: &std::collections::BTreeSet<&str>) -> u64 {
    let Ok(entries) = std::fs::read_dir(files) else { return 0 };
    let mut held: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file().then(|| (m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = held.iter().map(|h| h.1).sum();
    held.sort();
    let mut removed = 0;
    for (_, bytes, path) in held {
        if total <= limit {
            break;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if keep.contains(name) {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= bytes;
            removed += bytes;
        }
    }
    removed
}

/// One coordinator's session.
fn serve(stream: TcpStream, shared: &Shared) -> Result<(), LinkError> {
    let mut link = Link::new(stream, Side::Worker, &shared.token)?;
    let peer = link.peer();
    let (hello, id) = link.accept()?;
    let theirs = hello.get("version").and_then(Value::as_str).unwrap_or("");
    if theirs != env!("CARGO_PKG_VERSION") {
        let message = format!(
            "this worker is scene-render {} and the coordinator {theirs}: both must be the same version",
            env!("CARGO_PKG_VERSION")
        );
        link.send(refusal(&id, message.clone()), None)?;
        return Err(LinkError::Refused(message));
    }
    link.send(answer(&id, about()), None)?;
    shared.say(format!("{peer}: connected"));
    // the scenes this session opened: their documents stay loaded while it lasts
    let mut open: BTreeMap<String, Arc<sr_model::Document>> = BTreeMap::new();
    loop {
        let (request, payload) = match link.recv() {
            Ok(m) => m,
            Err(LinkError::Io(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        };
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request.get("method").and_then(Value::as_str).unwrap_or("").to_string();
        let params = request.get("params").cloned().unwrap_or(Value::Null);
        if method != "put" {
            if let Some(p) = &payload {
                link.skip(p)?;
            }
        }
        let outcome: Result<(Value, Option<PathBuf>), String> = match method.as_str() {
            "have" => have(shared, &params).map(|v| (v, None)),
            "put" => match &payload {
                Some(p) => put(shared, &mut link, &params, p)?.map(|v| (v, None)),
                None => Err("put carries no file".into()),
            },
            "open" => opened(shared, &params).map(|(scene, doc)| {
                open.insert(scene.clone(), doc);
                (json!({ "scene": scene }), None)
            }),
            "render" => render(shared, &mut link, &open, params, &peer).map(|(v, file)| (v, Some(file))),
            other => Err(format!("{other:?} is not a request a worker takes")),
        };
        match outcome {
            Ok((result, file)) => {
                let sent = link.send(answer(&id, result), file.as_deref());
                if let Some(f) = file {
                    let _ = std::fs::remove_file(f);
                }
                sent?;
            }
            Err(message) => {
                shared.say(format!("{peer}: {method} refused: {message}"));
                link.send(refusal(&id, message), None)?;
            }
        }
    }
}

fn digests(params: &Value) -> Result<Vec<String>, String> {
    let list = params.get("sha256").and_then(Value::as_array).ok_or("the request names no digests")?;
    if list.len() > MAX_FILES {
        return Err(format!("more than {MAX_FILES} digests"));
    }
    list.iter()
        .map(|v| {
            v.as_str()
                .filter(|s| is_digest(s))
                .map(str::to_string)
                .ok_or_else(|| "a digest is not a SHA-256".to_string())
        })
        .collect()
}

/// Which of the named files the worker does not hold.
fn have(shared: &Shared, params: &Value) -> Result<Value, String> {
    let missing: Vec<String> = digests(params)?.into_iter().filter(|d| !shared.file(d).is_file()).collect();
    Ok(json!({ "missing": missing }))
}

/// Keeps a file under its digest. The link's errors end the session; a refusal answers the request.
fn put(
    shared: &Shared,
    link: &mut Link,
    params: &Value,
    payload: &net::Payload,
) -> Result<Result<Value, String>, LinkError> {
    let named = params.get("sha256").and_then(Value::as_str).unwrap_or("");
    if named != payload.sha256 {
        link.skip(payload)?;
        return Ok(Err("the file's digest is not the one the request names".into()));
    }
    let to = shared.file(&payload.sha256);
    if to.is_file() {
        link.skip(payload)?;
        return Ok(Ok(json!({})));
    }
    // written beside its place and moved there whole: a file under a digest always is that file
    let part =
        shared.cache.join("files").join(format!("{}.part{}", payload.sha256, shared.chunk.fetch_add(1, Relaxed)));
    link.take(payload, &part)?;
    if let Err(e) = std::fs::rename(&part, &to) {
        let _ = std::fs::remove_file(&part);
        return Ok(Err(format!("cannot keep the file: {e}")));
    }
    Ok(Ok(json!({})))
}

/// Lays a scene's files out under its own directory and loads its document; a scene already loaded is reused.
fn opened(shared: &Shared, params: &Value) -> Result<(String, Arc<sr_model::Document>), String> {
    let manifest: SceneManifest = serde_json::from_value(params.clone())
        .map_err(|e| format!("the scene is not described as open expects: {e}"))?;
    if manifest.files.is_empty() || manifest.files.len() > MAX_FILES {
        return Err(format!("a scene holds between 1 and {MAX_FILES} files"));
    }
    let document = safe_relative(&manifest.document)?;
    let mut places = Vec::with_capacity(manifest.files.len());
    for f in &manifest.files {
        if !is_digest(&f.sha256) {
            return Err(format!("{}: its digest is not a SHA-256", f.path));
        }
        if !shared.file(&f.sha256).is_file() {
            return Err(format!("{}: the worker does not hold this file (send it with put)", f.path));
        }
        touch(&shared.file(&f.sha256));
        places.push((safe_relative(&f.path)?, shared.file(&f.sha256)));
    }
    if !manifest.files.iter().any(|f| f.path == manifest.document) {
        return Err("the scene's document is not among its files".into());
    }
    let id = manifest.id();
    {
        let mut loaded = shared.loaded.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(at) = loaded.iter().position(|(have, _)| *have == id) {
            let entry = loaded.remove(at);
            let doc = entry.1.clone();
            loaded.push(entry);
            touch(&shared.scene_dir(&id));
            return Ok((id, doc));
        }
    }
    let dir = shared.scene_dir(&id);
    let keep: std::collections::BTreeSet<&str> = manifest.files.iter().map(|f| f.sha256.as_str()).collect();
    let removed = evict(&shared.cache.join("files"), shared.cache_bytes, &keep);
    if removed > 0 {
        shared.say(format!("cache: removed {removed} bytes of files used longest ago"));
    }
    for (relative, kept) in &places {
        let to = dir.join(relative);
        if to.is_file() {
            continue;
        }
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("cannot make {}: {e}", parent.display()))?;
        }
        // the same bytes under the scene's name: a link where the file system has them, else a copy
        if std::fs::hard_link(kept, &to).is_err() {
            std::fs::copy(kept, &to).map_err(|e| format!("cannot place {}: {e}", relative.display()))?;
        }
    }
    touch(&dir);
    let path = dir.join(document);
    let doc = match sr_model::load_file(&path, &sr_model::LoadOptions::default()) {
        Ok(d) => Arc::new(d),
        Err(sr_model::LoadError::Io { path, source }) => {
            return Err(format!("cannot read {}: {source}", path.display()))
        }
        Err(sr_model::LoadError::Invalid(report)) => {
            let first: Vec<String> = report
                .diagnostics
                .iter()
                .filter(|d| d.is_error())
                .take(5)
                .map(|d| format!("{}: {}", d.code, d.message))
                .collect();
            return Err(format!(
                "the scene does not load on the worker ({}). A file the document names by an absolute path, or one outside its directory tree, is not sent",
                first.join("; ")
            ));
        }
    };
    let mut loaded = shared.loaded.lock().unwrap_or_else(|e| e.into_inner());
    loaded.push((id.clone(), doc.clone()));
    while loaded.len() > LOADED {
        loaded.remove(0);
    }
    Ok((id, doc))
}

/// Renders one chunk and returns what to answer and the file to send.
fn render(
    shared: &Shared,
    link: &mut Link,
    open: &BTreeMap<String, Arc<sr_model::Document>>,
    params: Value,
    peer: &str,
) -> Result<(Value, PathBuf), String> {
    let request: RenderRequest =
        serde_json::from_value(params).map_err(|e| format!("the chunk is not described as render expects: {e}"))?;
    let doc = open.get(&request.scene).ok_or("the scene was not opened in this session")?;
    if request.extension.is_empty()
        || request.extension.len() > 16
        || !request.extension.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return Err(format!("{:?} is not a file type", request.extension));
    }
    if !(request.start.is_finite() && request.end.is_finite()) || request.end <= request.start {
        return Err("the output's range is empty".into());
    }
    let quality = match &request.quality {
        Some(name) => Some(quality_of(name).ok_or_else(|| format!("{name:?} is not a quality"))?),
        None => None,
    };
    let hardware = sr_media::encode::Hardware::parse(&request.hardware)
        .ok_or_else(|| format!("{:?} is not an encoder family", request.hardware))?;
    let file = shared.cache.join("chunks").join(format!(
        "chunk{:06}.{}",
        shared.chunk.fetch_add(1, Relaxed),
        request.extension
    ));
    let path = file.to_string_lossy().into_owned();
    let output = match &request.output {
        Some(id) => match doc.scene.outputs.iter().find(|o| o.id.as_deref() == Some(id.as_str())) {
            Some(o) => sr_model::model::Output { path, ..o.clone() },
            None => return Err(format!("the scene has no output {id:?}")),
        },
        None => sr_deliver::adhoc_output(&path, &request.codec).map_err(|e| e.to_string())?,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let opts = sr_deliver::Options {
        out_dir: None,
        representation: request.representation.clone(),
        hardware,
        start: Some(request.start),
        end: Some(request.end),
        upload: false,
        params: request.params.clone(),
        row: request.row.clone(),
        parallel: sr_deliver::Parallel::Count(1),
        quality,
        path_noise: request.path_noise,
        report: false,
        segment: Some(sr_deliver::remote::Segment { part: request.part, cancel: Some(cancel.clone()) }),
        remote: None,
    };
    // one chunk at a time: a second coordinator's waits here
    let mut gpu = shared.gpu.lock().unwrap_or_else(|e| e.into_inner());
    if gpu.is_none() {
        *gpu = Some(sr_gpu::Gpu::new_for(true).map_err(|e| e.to_string())?);
    }
    let Some(device) = gpu.as_ref() else { return Err("no GPU device".into()) };
    let began = std::time::Instant::now();
    shared.say(format!("{peer}: frames {}..{} of {}", request.part.first, request.part.last, request.part.frames));
    // every frame is announced, which is also how a coordinator that went away is noticed: the render stops
    let mut announced = 0u64;
    let mut progress = |done: u64, _total: u64| {
        if done > announced && !cancel.load(Relaxed) {
            announced = done;
            if link.send(notice("progress", json!({ "done": done })), None).is_err() {
                cancel.store(true, Relaxed);
            }
        }
    };
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        sr_deliver::deliver(doc, &output, Some(device), &opts, &mut progress)
    }));
    let report = match outcome {
        Ok(Ok(report)) => report,
        Ok(Err(e)) => {
            let _ = std::fs::remove_file(&file);
            return Err(e.to_string());
        }
        Err(_) => {
            let _ = std::fs::remove_file(&file);
            // a device a panic left half used is not kept
            *gpu = None;
            return Err("the render failed inside the engine (a panic; see the worker's output)".into());
        }
    };
    let frames = (request.part.last - request.part.first) as f64;
    shared.say(format!("{peer}: {frames:.0} frames in {:.1} s", began.elapsed().as_secs_f64()));
    let done = sr_deliver::remote::PartDone { encoder: report.encoder, unsupported: report.unsupported };
    Ok((json!({ "done": done, "seconds": began.elapsed().as_secs_f64() }), file))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_relative_names_are_a_scenes_files() {
        assert_eq!(safe_relative("a/b.png").unwrap(), PathBuf::from("a").join("b.png"));
        for bad in ["", "/etc/passwd", "../x", "a/../../x", "a\\b", "./a", "x\0y"] {
            assert!(safe_relative(bad).is_err(), "{bad:?}");
        }
        assert!(safe_relative(&"a".repeat(MAX_PATH + 1)).is_err());
    }

    #[test]
    fn the_cache_drops_the_files_used_longest_ago_and_never_the_scene_being_opened() {
        let dir = std::env::temp_dir().join(format!("sr-evict-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let now = std::time::SystemTime::now();
        for (name, age) in [("old", 300), ("kept", 200), ("new", 100)] {
            std::fs::write(dir.join(name), vec![0u8; 1000]).unwrap();
            let f = std::fs::File::options().write(true).open(dir.join(name)).unwrap();
            f.set_modified(now - std::time::Duration::from_secs(age)).unwrap();
        }
        let keep = std::collections::BTreeSet::from(["kept"]);
        assert_eq!(evict(&dir, 2500, &keep), 1000); // the oldest goes
        assert!(!dir.join("old").exists() && dir.join("kept").exists() && dir.join("new").exists());
        assert_eq!(evict(&dir, 500, &keep), 1000); // then the next, but not the one in use
        assert!(dir.join("kept").exists() && !dir.join("new").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_scenes_name_does_not_depend_on_the_order_of_its_files() {
        let f = |p: &str, d: char| SceneFile { path: p.into(), sha256: d.to_string().repeat(64) };
        let a = SceneManifest { document: "s.xml".into(), files: vec![f("s.xml", 'a'), f("t.png", 'b')] };
        let b = SceneManifest { document: "s.xml".into(), files: vec![f("t.png", 'b'), f("s.xml", 'a')] };
        let c = SceneManifest { document: "s.xml".into(), files: vec![f("s.xml", 'a'), f("t.png", 'c')] };
        assert_eq!(a.id(), b.id());
        assert_ne!(a.id(), c.id());
    }
}
