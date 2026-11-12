//! `encode --workers`: the coordinator's side of rendering on other machines.
//!
//! [`Workers::connect`] reaches each `scene-render serve` named, proves the token both ways, sends the files of the
//! scene the worker does not hold yet and opens the scene there. The delivery then asks for chunks through
//! [`sr_deliver::remote::RemoteWorkers`]. A worker that cannot be reached, speaks another version or cannot load
//! the scene is left out with a note, and the encode goes on with the others or alone; one that fails later loses
//! its chunk to another worker (see `sr_deliver::remote`).

use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};

use crate::net::{self, Link, LinkError, Side};
use crate::serve::{hardware_name, quality_name, RenderRequest, SceneFile, SceneManifest};

/// The port a worker is reached on when an address names none.
pub const DEFAULT_PORT: u16 = 7878;
/// The environment variable naming the workers when `--workers` does not.
pub const WORKERS_ENV: &str = "SR_WORKERS";

/// The files of a scene as they are sent: where each lies on this machine, and as the worker names it.
pub struct Bundle {
    /// What a worker is told.
    pub manifest: SceneManifest,
    /// The file of each entry of `manifest.files`, in the same order.
    pub local: Vec<PathBuf>,
}

impl Bundle {
    /// The document `file` and the files it names (`named`, from `changes::effective_files`), laid out under their
    /// common directory. A file that does not exist is left out: the worker's load says what it misses.
    pub fn of(file: &Path, named: &[PathBuf]) -> Result<Bundle, String> {
        let real = |p: &Path| p.canonicalize().map_err(|e| format!("cannot read {}: {e}", p.display()));
        let document = real(file)?;
        let mut files = vec![document.clone()];
        for f in named {
            if let Ok(f) = f.canonicalize() {
                if f.is_file() && !files.contains(&f) {
                    files.push(f);
                }
            }
        }
        // the deepest directory that holds them all: a file beside or below the document keeps its place relative
        // to it, and so does one a few directories up
        let mut root = document.parent().map(Path::to_path_buf).unwrap_or_default();
        for f in &files {
            while !f.starts_with(&root) {
                if !root.pop() {
                    return Err(format!("{} and the scene's document share no directory", f.display()));
                }
            }
        }
        let relative = |p: &Path| -> Result<String, String> {
            let r = p.strip_prefix(&root).map_err(|_| format!("{} is outside the scene", p.display()))?;
            let names: Option<Vec<&str>> = r.iter().map(|n| n.to_str()).collect();
            names.map(|n| n.join("/")).ok_or_else(|| format!("{} is not named in UTF-8", p.display()))
        };
        let mut entries = Vec::with_capacity(files.len());
        for f in &files {
            let mut open = std::fs::File::open(f).map_err(|e| format!("cannot read {}: {e}", f.display()))?;
            let (_, sha256) = net::digest_of(&mut open).map_err(|e| format!("cannot read {}: {e}", f.display()))?;
            entries.push(SceneFile { path: relative(f)?, sha256 });
        }
        Ok(Bundle { manifest: SceneManifest { document: relative(&document)?, files: entries }, local: files })
    }
}

/// What every chunk of this encode is asked with, whatever the output.
#[derive(Debug, Clone)]
pub struct Job {
    /// Parameter values.
    pub params: Vec<(String, String)>,
    /// Data row and its source.
    pub row: Option<(Option<String>, usize)>,
    /// Quality override.
    pub quality: Option<sr_model::model::ProjectQuality>,
    /// Preferred asset representation.
    pub representation: Option<String>,
    /// Noise target of adaptive path sampling.
    pub path_noise: Option<f32>,
    /// Encoder family: never `auto`, so every chunk is encoded the same way.
    pub hardware: sr_media::encode::Hardware,
    /// Output ids the document defines: an output with another id, or none, is ad-hoc.
    pub outputs: Vec<String>,
}

struct Worker {
    name: String,
    scene: String,
    /// None once the link failed.
    link: Mutex<Option<Link>>,
}

/// The workers of one encode.
pub struct Workers {
    workers: Vec<Worker>,
    job: Job,
}

impl std::fmt::Debug for Workers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.workers.iter().map(|w| &w.name)).finish()
    }
}

/// `host`, `host:port` or `[v6]:port` as an address to connect to.
pub fn address(spec: &str) -> String {
    let has_port = match spec.rfind(':') {
        Some(at) => spec[at + 1..].parse::<u16>().is_ok() && (!spec[..at].contains(':') || spec[..at].ends_with(']')),
        None => false,
    };
    if has_port {
        spec.to_string()
    } else {
        format!("{spec}:{DEFAULT_PORT}")
    }
}

fn link_error(e: LinkError) -> String {
    e.to_string()
}

/// A link to a worker with the scene ready there: the link, the worker's name, the scene as it names it, and how
/// many chunks it renders at once.
type Ready = (Link, String, String, usize);

/// Opens a link to one worker and readies the scene there.
fn ready(spec: &str, token: &str, bundle: &Bundle) -> Result<Ready, String> {
    let at = address(spec);
    let addresses: Vec<_> = at.to_socket_addrs().map_err(|e| format!("cannot find {at}: {e}"))?.collect();
    let mut stream = Err(format!("cannot find {at}"));
    for a in addresses {
        match TcpStream::connect_timeout(&a, Duration::from_secs(5)) {
            Ok(s) => {
                stream = Ok(s);
                break;
            }
            Err(e) => stream = Err(format!("cannot connect to {a}: {e}")),
        }
    }
    let mut link = Link::new(stream?, Side::Coordinator, token).map_err(|e| e.to_string())?;
    let about = link.greet(json!({ "version": env!("CARGO_PKG_VERSION") })).map_err(link_error)?;
    let theirs = about.get("version").and_then(Value::as_str).unwrap_or("");
    if theirs != env!("CARGO_PKG_VERSION") {
        return Err(format!("it is scene-render {theirs} and this is {}", env!("CARGO_PKG_VERSION")));
    }
    let most = crate::serve::MAX_SLOTS as u64;
    let slots = about.get("slots").and_then(Value::as_u64).unwrap_or(1).clamp(1, most) as usize;
    let host = about.get("host").and_then(Value::as_str).filter(|h| !h.is_empty()).map(str::to_string);
    let name = host.map_or_else(|| at.clone(), |h| format!("{h} ({at})"));
    let mut quiet = |_: &str, _: &Value| Ok(());
    let digests: Vec<&str> = bundle.manifest.files.iter().map(|f| f.sha256.as_str()).collect();
    let (missing, _) = link.call("have", json!({ "sha256": digests }), None, &mut quiet).map_err(link_error)?;
    let missing: Vec<String> = missing
        .get("missing")
        .and_then(Value::as_array)
        .map(|l| l.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    for digest in &missing {
        let Some(at) = bundle.manifest.files.iter().position(|f| &f.sha256 == digest) else {
            return Err("it asked for a file the scene does not have".into());
        };
        link.call("put", json!({ "sha256": digest }), Some(&bundle.local[at]), &mut quiet).map_err(link_error)?;
    }
    let manifest = serde_json::to_value(&bundle.manifest).map_err(|e| e.to_string())?;
    let (opened, _) = link.call("open", manifest, None, &mut quiet).map_err(link_error)?;
    let scene = opened.get("scene").and_then(Value::as_str).ok_or("it did not name the scene it opened")?.to_string();
    Ok((link, name, scene, slots))
}

impl Workers {
    /// Readies every worker of `specs`, all at once. Each that cannot be readied is reported through `note` and
    /// left out.
    pub fn connect(specs: &[String], token: &str, bundle: &Bundle, job: Job, note: &mut dyn FnMut(String)) -> Workers {
        // The first link to each worker sends the files; then one more link for each further chunk it renders at
        // once, which finds the files there and the scene loaded.
        let readied: Vec<Result<Vec<Ready>, String>> = std::thread::scope(|sc| {
            let handles: Vec<_> = specs
                .iter()
                .map(|spec| {
                    sc.spawn(move || {
                        let first = ready(spec, token, bundle)?;
                        let mut links = Vec::new();
                        for _ in 1..first.3 {
                            // a link that does not open costs a slot, not the worker
                            if let Ok(more) = ready(spec, token, bundle) {
                                links.push(more);
                            }
                        }
                        links.insert(0, first);
                        Ok(links)
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap_or_else(|_| Err("its set-up panicked".into()))).collect()
        });
        let mut workers = Vec::new();
        for (spec, r) in specs.iter().zip(readied) {
            match r {
                Ok(links) => {
                    let several = links.len() > 1;
                    if let Some(first) = links.first() {
                        note(format!("worker {}: ready, {} chunk(s) at once", first.1, links.len()));
                    }
                    for (k, (link, name, scene, _)) in links.into_iter().enumerate() {
                        let name = if several { format!("{name} #{}", k + 1) } else { name };
                        workers.push(Worker { name, scene, link: Mutex::new(Some(link)) });
                    }
                }
                Err(e) => note(format!("worker {spec} is left out: {e}")),
            }
        }
        Workers { workers, job }
    }

    /// True when no worker is ready.
    pub fn is_empty(&self) -> bool {
        self.workers.is_empty()
    }
}

impl sr_deliver::remote::RemoteWorkers for Workers {
    fn names(&self) -> Vec<String> {
        self.workers.iter().map(|w| w.name.clone()).collect()
    }

    fn render(
        &self,
        worker: usize,
        output: &sr_model::model::Output,
        part: sr_deliver::remote::Part,
        to: &Path,
        cancelled: &AtomicBool,
        frame: &dyn Fn(),
    ) -> Result<sr_deliver::remote::PartDone, String> {
        let w = self.workers.get(worker).ok_or("no such worker")?;
        let mut guard = w.link.lock().unwrap_or_else(|e| e.into_inner());
        let Some(link) = guard.as_mut() else { return Err("its link has ended".into()) };
        let extension = Path::new(&output.path).extension().and_then(|e| e.to_str()).unwrap_or("mp4").to_string();
        let request = RenderRequest {
            scene: w.scene.clone(),
            output: output.id.clone().filter(|id| self.job.outputs.contains(id)),
            extension,
            codec: output.codec.as_str().to_string(),
            start: output.start,
            end: output.end.unwrap_or(output.start),
            params: self.job.params.clone(),
            row: self.job.row.clone(),
            quality: self.job.quality.map(|q| quality_name(q).to_string()),
            representation: self.job.representation.clone(),
            path_noise: self.job.path_noise,
            hardware: hardware_name(self.job.hardware).to_string(),
            part,
        };
        let params = serde_json::to_value(&request).map_err(|e| e.to_string())?;
        let mut seen = 0u64;
        let mut notified = |name: &str, params: &Value| {
            if cancelled.load(Relaxed) {
                return Err("the render was cancelled".to_string());
            }
            if name == "progress" {
                let done = params.get("done").and_then(Value::as_u64).unwrap_or(seen);
                let most = (part.last - part.first) as u64;
                while seen < done.min(most) {
                    seen += 1;
                    frame();
                }
            }
            Ok(())
        };
        let outcome = link.call("render", params, None, &mut notified).and_then(|(result, file)| match file {
            Some(file) => link.take(&file, to).map(|()| result),
            None => Err(LinkError::Protocol("the worker answered without the chunk".into())),
        });
        match outcome {
            Ok(result) => serde_json::from_value(result.get("done").cloned().unwrap_or(Value::Null))
                .map_err(|e| format!("the worker's answer does not read: {e}")),
            // a refusal leaves the link in step; anything else ends it
            Err(LinkError::Refused(message)) if !cancelled.load(Relaxed) => Err(message),
            Err(e) => {
                link.close();
                *guard = None;
                Err(e.to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_without_a_port_gets_the_workers_port() {
        assert_eq!(address("render-2"), "render-2:7878");
        assert_eq!(address("render-2:9000"), "render-2:9000");
        assert_eq!(address("10.0.0.7"), "10.0.0.7:7878");
        assert_eq!(address("[::1]:9000"), "[::1]:9000");
    }

    #[test]
    fn a_bundle_keeps_each_file_where_it_lies_from_the_common_directory() {
        let dir = std::env::temp_dir().join(format!("sr-bundle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("scenes")).unwrap();
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("scenes/s.scene.xml"), "<scene/>").unwrap();
        std::fs::write(dir.join("assets/a.png"), "a").unwrap();
        let named = vec![dir.join("scenes/../assets/a.png"), dir.join("assets/missing.png")];
        let b = Bundle::of(&dir.join("scenes/s.scene.xml"), &named).unwrap();
        assert_eq!(b.manifest.document, "scenes/s.scene.xml");
        let paths: Vec<&str> = b.manifest.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["scenes/s.scene.xml", "assets/a.png"]);
        assert!(b.manifest.files.iter().all(|f| net::is_digest(&f.sha256)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
