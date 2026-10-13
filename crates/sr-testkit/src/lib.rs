//! Test helpers shared by the scene-render crates, as a dev-dependency only.
//!
//! A test that needs a GPU adapter, FFmpeg or a HEIC decoder skips itself with a `SKIPPED (<test>): <reason>` line
//! when the requirement is missing, unless the environment requires it: `SR_REQUIRE_GPU=1`, `SR_REQUIRE_FFMPEG=1` or
//! `SR_REQUIRE_HEIC=1` (as in renderer CI) turn the skip into a failure that names the reason.
//!
//! A test that stands in a fake tool (`curl`, `ffmpeg`) writes its script with [`executable`], never `std::fs::write`.
//!
//! A property test runs through [`property::check`], which draws the same cases on every run.

#![forbid(unsafe_code)]

pub mod property;

use std::fmt::Display;
#[cfg(unix)]
use std::path::Path;
use std::process::Command;

/// A requirement a test can skip without: the variable that requires it and what it needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Requirement {
    /// The environment variable that, set to `1`, turns a skip into a failure.
    pub var: &'static str,
    /// What the requirement needs, as the failure says it: "a GPU adapter".
    pub needs: &'static str,
}

/// A GPU adapter: `SR_REQUIRE_GPU=1`.
pub const GPU: Requirement = Requirement { var: "SR_REQUIRE_GPU", needs: "a GPU adapter" };
/// FFmpeg, ffprobe and the encoders a test uses: `SR_REQUIRE_FFMPEG=1`.
pub const FFMPEG: Requirement = Requirement { var: "SR_REQUIRE_FFMPEG", needs: "FFmpeg" };
/// A decoder for HEIC stills: `SR_REQUIRE_HEIC=1`.
pub const HEIC: Requirement = Requirement { var: "SR_REQUIRE_HEIC", needs: "a HEIC decoder" };

impl Requirement {
    /// Whether the environment requires it: its variable is exactly `1`.
    pub fn required(self) -> bool {
        std::env::var(self.var).as_deref() == Ok("1")
    }

    /// Skips what `context` needs this for, saying why, or fails when the environment requires it.
    ///
    /// # Panics
    /// When the variable is `1`, with `<context>: <reason> (<VAR>=1 requires <needs>)`.
    pub fn skip(self, context: &str, reason: impl Display) {
        assert!(!self.required(), "{context}: {reason} ({}=1 requires {})", self.var, self.needs);
        eprintln!("SKIPPED ({context}): {reason}");
    }

    /// The value of a probe that found what this needs, or nothing after [`Requirement::skip`] with its error.
    pub fn require<T, E: Display>(self, context: &str, probe: Result<T, E>) -> Option<T> {
        match probe {
            Ok(value) => Some(value),
            Err(reason) => {
                self.skip(context, reason);
                None
            }
        }
    }
}

/// The adapter a GPU probe found, or nothing after skipping `context` (see [`GPU`]).
pub fn gpu<T, E: Display>(probe: Result<T, E>, context: &str) -> Option<T> {
    GPU.require(context, probe)
}

/// The decoder a HEIC probe found, or nothing after skipping `context` (see [`HEIC`]).
pub fn heic<T, E: Display>(probe: Result<T, E>, context: &str) -> Option<T> {
    HEIC.require(context, probe)
}

/// Whether every one of `tools` runs with `-version`; if one does not, skips `context` (see [`FFMPEG`]) with
/// `cannot run <tool>`.
pub fn ffmpeg<S: AsRef<str>>(context: &str, tools: &[S]) -> bool {
    tools.iter().map(AsRef::as_ref).all(|tool| {
        let runs = Command::new(tool).arg("-version").output().is_ok_and(|out| out.status.success());
        if !runs {
            FFMPEG.skip(context, format_args!("cannot run {tool}"));
        }
        runs
    })
}

/// Whether `tools` run (see [`ffmpeg`]) and `working` finds that FFmpeg's `name` encoder encodes; if it does not,
/// skips `context` with `FFmpeg has no working <name> encoder`.
pub fn ffmpeg_encoder<S: AsRef<str>>(context: &str, tools: &[S], name: &str, working: impl FnOnce() -> bool) -> bool {
    if !ffmpeg(context, tools) {
        return false;
    }
    let works = working();
    if !works {
        FFMPEG.skip(context, format_args!("FFmpeg has no working {name} encoder"));
    }
    works
}

/// The adapter a GPU probe (`Result<T, impl Display>`) found; returns from the test after [`gpu`] skipped it.
/// Writes `script` to `path` as an executable (mode 0700) that a test can run at once, from any thread.
///
/// Linux refuses to run a file that any process holds open for writing (ETXTBSY). A test that writes the file itself
/// holds that descriptor, and a process another test thread forks meanwhile inherits it until it execs, so the run
/// fails now and then (rust-lang/rust#114554). Here the test process writes only `<path>.src`, which nothing runs;
/// `cp` creates `path`, and only `cp` ever holds it open.
///
/// # Panics
///
/// When the script cannot be written, copied or made executable: a fixture the test cannot build.
#[cfg(unix)]
pub fn executable(path: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt as _;
    let mut src = path.as_os_str().to_owned();
    src.push(".src");
    let src = Path::new(&src);
    std::fs::write(src, script).unwrap_or_else(|e| panic!("cannot write {}: {e}", src.display()));
    // `cp` would truncate an existing file in place, which fails while that file runs; a new inode never does.
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => panic!("cannot replace {}: {e}", path.display()),
        _ => {}
    }
    let status = Command::new("cp").arg(src).arg(path).status();
    assert!(status.as_ref().is_ok_and(|s| s.success()), "cp {} {}: {status:?}", src.display(), path.display());
    std::fs::remove_file(src).unwrap_or_else(|e| panic!("cannot remove {}: {e}", src.display()));
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .unwrap_or_else(|e| panic!("cannot make {} executable: {e}", path.display()));
}

#[macro_export]
macro_rules! require_gpu {
    ($probe:expr, $context:expr $(,)?) => {
        match $crate::gpu($probe, $context) {
            ::core::option::Option::Some(value) => value,
            ::core::option::Option::None => return,
        }
    };
}

/// Returns from the test after [`ffmpeg`] skipped it because one of the tools does not run.
#[macro_export]
macro_rules! require_ffmpeg {
    ($context:expr, $tools:expr $(,)?) => {
        if !$crate::ffmpeg($context, $tools) {
            return;
        }
    };
}

/// The decoder a HEIC probe (`Result<T, impl Display>`) found; returns from the test after [`heic`] skipped it.
#[macro_export]
macro_rules! require_heic {
    ($probe:expr, $context:expr $(,)?) => {
        match $crate::heic($probe, $context) {
            ::core::option::Option::Some(value) => value,
            ::core::option::Option::None => return,
        }
    };
}
