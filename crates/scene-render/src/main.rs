//! `scene-render` — validate and inspect scene-render 1.1 documents.

#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod arena;
mod changes;
mod frames;
mod mcp;
mod output;
mod schedule;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anstream::{AutoStream, ColorChoice};
use anstyle::{AnsiColor, Style};
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use output::{style, ErrorKind, Out};
use serde_json::{json, Value};
use sr_media::encode::InputFormat;
use sr_model::diag::LineIndex;
use sr_model::{codes, Diagnostic, LoadError, LoadOptions, Report, Severity};

const EXAMPLES: &str = "\
Examples:
  scene-render validate promo.scene.xml
  scene-render validate scenes/*.xml --format json > report.json
  scene-render inspect promo.scene.xml --json
  scene-render validate promo.scene.xml --no-assets --deny-warnings
  scene-render inspect promo.scene.xml
  scene-render params promo.scene.xml
  scene-render eval promo.scene.xml --time 2.5 --variant dark --param headline=Hi
  scene-render eval promo.scene.xml --bench
  scene-render captions promo.scene.xml --at 1.5,3
  scene-render scopes final.mp4 --step 5 --format json
  scene-render explain C21

Output: text by default; with --format json (or --json) every command prints one JSON document,\nthe envelope scene-render-cli/1 (format, schema_version, command, ok, exit_code, result,\ndiagnostics, error), and messages go to standard error.

Exit status: 0 valid, 1 invalid (or warnings with --deny-warnings; render/encode: anything\nnot rendered as authored with --strict), 2 usage, I/O or internal error.";

/// Validate and inspect scene-render 1.1 documents.
///
/// Validation checks the XSD structure, the Schematron co-occurrence and
/// reference rules, and every referenced file with its SHA-256 digest.
#[derive(Parser, Debug)]
#[command(name = "scene-render", version, propagate_version = true, after_help = EXAMPLES, arg_required_else_help = true)]
struct Cli {
    /// When to use colour: auto detects a terminal and honours NO_COLOR.
    #[arg(long, value_enum, default_value_t = Color::Auto, global = true, env = "SCENE_RENDER_COLOR")]
    color: Color,

    /// Worker threads for the CPU stages (vector tiling, audio analysis); default: every core.
    #[arg(long, global = true, env = "SR_THREADS", value_name = "N")]
    threads: Option<usize>,

    /// Turn on the GPU debug and validation layers (they name every object through the Vulkan loader, and are off by
    /// default: a delivery does not need them).
    /// The environment variable SR_GPU_DEBUG (any value but empty or 0) does the same; the GPU layer reads it itself.
    #[arg(long, global = true)]
    debug_gpu: bool,

    /// Output format: text for a person, or json: one JSON document on standard output (the envelope
    /// scene-render-cli/1 with the command's result, its diagnostics and its error), messages on standard error.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text, global = true)]
    format: OutputFormat,

    /// The same as --format json.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

/// Bake a native pyro object. Example: scene-render bake-volume impact.scene.xml --object plume -o plume-cache
#[derive(clap::Args, Debug)]
struct BakeArgs {
    /// Scene document.
    file: PathBuf,
    /// Instantiated object ID (including its namespace, when applicable).
    #[arg(long)]
    object: String,
    /// New cache directory; existing files or directories are never replaced.
    #[arg(short, long)]
    output: PathBuf,
    /// First project frame, inclusive.
    #[arg(long, default_value_t = 0)]
    first: u64,
    /// Last project frame, exclusive (default: project duration).
    #[arg(long)]
    end: Option<u64>,
    /// Maximum cache size in MiB, including all unique frames and the manifest.
    #[arg(long, default_value_t=65536, value_parser=clap::value_parser!(u64).range(1..=(u64::MAX >> 20)))]
    max_mib: u64,
    /// Parameter value, repeatable: --param id=value.
    #[arg(long="param", value_name="ID=VALUE", value_parser=parse_param)]
    params: Vec<(String, String)>,
}

/// Quality tier (overrides the document's `project@quality`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum QualityArg {
    /// Half-size render in the document's coordinates, at most 2 motion-blur samples, no grain.
    Draft,
    /// Full size, at most 4 motion-blur samples.
    Preview,
    /// As authored.
    Final,
}

impl QualityArg {
    fn model(self) -> sr_model::model::ProjectQuality {
        match self {
            QualityArg::Draft => sr_model::model::ProjectQuality::Draft,
            QualityArg::Preview => sr_model::model::ProjectQuality::Preview,
            QualityArg::Final => sr_model::model::ProjectQuality::Final,
        }
    }
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum Color {
    Auto,
    Always,
    Never,
}

impl Color {
    fn choice(self) -> ColorChoice {
        match self {
            Color::Auto => ColorChoice::Auto,
            Color::Always => ColorChoice::Always,
            Color::Never => ColorChoice::Never,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum OutputFormat {
    /// Text for a person: diagnostics with source excerpts, summaries (formerly `human` and `summary`).
    #[value(alias = "human", alias = "summary")]
    Text,
    /// One JSON document on standard output: the envelope scene-render-cli/1.
    Json,
}

fn parse_parallel(s: &str) -> Result<sr_deliver::Parallel, String> {
    sr_deliver::Parallel::parse(s).ok_or_else(|| format!("expected auto or a positive count, got {s:?}"))
}

fn parse_time(s: &str) -> Result<f64, String> {
    s.trim()
        .parse::<f64>()
        .ok()
        .filter(|t| t.is_finite())
        .ok_or_else(|| format!("expected a time in seconds, got {s:?}"))
}

fn parse_chroma_limit(s: &str) -> Result<f32, String> {
    s.trim()
        .parse::<f32>()
        .ok()
        .filter(|c| c.is_finite() && *c > 0.0)
        .ok_or_else(|| format!("expected a positive chroma such as 0.45, got {s:?}"))
}

fn parse_path_noise(s: &str) -> Result<f32, String> {
    s.trim()
        .parse::<f32>()
        .ok()
        .filter(|n| n.is_finite() && *n > 0.0 && *n <= 1.0)
        .ok_or_else(|| format!("expected a relative error above 0 and at most 1, such as 0.02, got {s:?}"))
}

fn parse_param(s: &str) -> Result<(String, String), String> {
    s.split_once('=')
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .ok_or_else(|| format!("expected ID=VALUE, got {s:?}"))
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Validate documents and report every problem found.
    Validate {
        /// Scene documents to validate.
        #[arg(required = true, value_name = "FILE")]
        files: Vec<PathBuf>,
        /// Skip file existence and SHA-256 checks.
        #[arg(long)]
        no_assets: bool,
        /// Directory relative URIs resolve against (default: each document's directory).
        #[arg(long, value_name = "DIR")]
        base_dir: Option<PathBuf>,
        /// Treat warnings as errors for the exit status.
        #[arg(long)]
        deny_warnings: bool,
        /// Print only the per-file summary lines.
        #[arg(short, long)]
        quiet: bool,
    },
    /// Load a document and summarise the typed model.
    ///
    /// With --format json the result carries the summary and the whole typed model under `model`.
    Inspect {
        /// Scene document.
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// Skip file existence and SHA-256 checks.
        #[arg(long)]
        no_assets: bool,
    },
    /// Print parameter definitions and variant labels as JSON, without evaluating the scene.
    Params {
        /// Scene document.
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// Skip file existence and SHA-256 checks.
        #[arg(long)]
        no_assets: bool,
    },
    /// Evaluate one frame and print its FrameGraph.
    Eval {
        /// Scene document.
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// Composition time in seconds.
        #[arg(long, short, conflicts_with = "frame", allow_negative_numbers = true)]
        time: Option<f64>,
        /// Frame index at the project frame rate.
        #[arg(long, short)]
        frame: Option<u64>,
        /// Variant to apply.
        #[arg(long)]
        variant: Option<String>,
        /// Layout to render.
        #[arg(long)]
        layout: Option<String>,
        /// Parameter value, repeatable: --param id=value.
        #[arg(long = "param", value_name = "ID=VALUE", value_parser = parse_param)]
        params: Vec<(String, String)>,
        /// Data row for batch rendering.
        #[arg(long)]
        row: Option<usize>,
        /// Data source of --row (default: the first one).
        #[arg(long, requires = "row")]
        data: Option<String>,
        /// Evaluate every frame and report timing instead of printing a frame.
        #[arg(long)]
        bench: bool,
        /// Skip file existence and SHA-256 checks.
        #[arg(long)]
        no_assets: bool,
    },
    /// Print each caption track after paging as JSON: the words of each cue with their times, the pages (lines,
    /// start and end), and the current word over each page; with --at, the page and current word at those times.
    ///
    /// The pages are those the burn-in layout draws (lineBreaks, maxCharsPerLine, maxWordsPerLine, maxLines and the
    /// preset applied); word indices count the words of a cue. Exit status 1 when a track cannot be read (its entry
    /// carries the error).
    Captions {
        /// Scene document.
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// Composition times (seconds) at which to report the page and current word: repeatable, or comma-separated.
        #[arg(long, value_name = "SECONDS", value_delimiter = ',', allow_negative_numbers = true, value_parser = parse_time)]
        at: Vec<f64>,
        /// Variant to apply.
        #[arg(long)]
        variant: Option<String>,
        /// Layout to render.
        #[arg(long)]
        layout: Option<String>,
        /// Parameter value, repeatable: --param id=value.
        #[arg(long = "param", value_name = "ID=VALUE", value_parser = parse_param)]
        params: Vec<(String, String)>,
        /// Skip file existence and SHA-256 checks.
        #[arg(long)]
        no_assets: bool,
    },
    /// Measure the levels of a video or still image: clipping at black and white, codes outside the legal range,
    /// chroma above a limit and colours outside the gamut, with waveform, RGB parade, histogram and vectorscope
    /// summaries. Exit status 2 when the file cannot be read.
    Scopes {
        /// Video or still image.
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// Measure every Nth frame of a video, from the first.
        #[arg(long, value_name = "N", default_value_t = 1, value_parser = clap::value_parser!(u64).range(1..))]
        step: u64,
        /// Chroma √(Cb² + Cr²) above which a sample is oversaturated (default: the largest 75 % colour-bar chroma
        /// of the stream's matrix).
        #[arg(long, value_name = "CHROMA", value_parser = parse_chroma_limit)]
        chroma_limit: Option<f32>,
    },
    /// Render frames on the GPU and write PNG files.
    Render {
        /// Scene document.
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// Composition time in seconds.
        #[arg(long, short, conflicts_with_all = ["frame", "frames"], allow_negative_numbers = true)]
        time: Option<f64>,
        /// Frame index at the project frame rate.
        #[arg(long, short, conflicts_with = "frames")]
        frame: Option<u64>,
        /// Frame range START..END (end exclusive) or START..=END.
        #[arg(long, value_name = "RANGE", value_parser = parse_range)]
        frames: Option<(u64, u64)>,
        /// Output PNG; for several frames include %04d or {frame} in the name.
        #[arg(long, short, value_name = "PATH", required_unless_present = "bench")]
        output: Option<PathBuf>,
        /// Bits per channel of the PNG.
        #[arg(long, value_parser = ["8", "16"], default_value = "8")]
        bit_depth: String,
        /// Variant to apply.
        #[arg(long)]
        variant: Option<String>,
        /// Layout to render.
        #[arg(long)]
        layout: Option<String>,
        /// Parameter value, repeatable: --param id=value.
        #[arg(long = "param", value_name = "ID=VALUE", value_parser = parse_param)]
        params: Vec<(String, String)>,
        /// Data row for batch rendering.
        #[arg(long)]
        row: Option<usize>,
        /// Data source of --row (default: the first one).
        #[arg(long, requires = "row")]
        data: Option<String>,
        /// Render the frames into GPU memory and report timing instead of writing files.
        #[arg(long)]
        bench: bool,
        /// Print per-frame renderer statistics as JSON lines on stderr.
        #[arg(long)]
        stats: bool,
        /// With --bench, keep one frame in flight: wait for the previous frame's submission
        /// instead of draining the GPU after every frame, so the figure is throughput.
        #[arg(long, requires = "bench")]
        pipelined: bool,
        /// Exit 1 if anything was not rendered as authored: unsupported content, shader fallbacks
        /// (pass-through effects, crossfaded transitions), evaluator warnings or, when encoding,
        /// accessibility findings. Output files are still written.
        #[arg(long)]
        strict: bool,
        /// Quality tier overriding the document's project@quality (draft renders at half size).
        #[arg(long, value_enum)]
        quality: Option<QualityArg>,
        /// Adaptive sampling for path-traced cameras: a pixel stops once the standard error of its mean is this
        /// fraction of it (0.01 to 0.05 is typical). Each camera's pathSamples caps its pixels.
        #[arg(long, value_name = "REL", value_parser = parse_path_noise)]
        path_noise: Option<f32>,
        /// Render only the frames whose file is missing or whose content changed since the last
        /// --changed-only render into the same files (fingerprints in .scene-render-frames.json
        /// beside them).
        #[arg(long, conflicts_with = "bench")]
        changed_only: bool,
    },
    /// Render frames to PNG files, then again whenever the document or a file it names changes,
    /// rendering only the frames the edit changed (see render --changed-only).
    Watch {
        /// Scene document.
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// Frame range START..END (end exclusive) or START..=END (default: every frame).
        #[arg(long, value_name = "RANGE", value_parser = parse_range)]
        frames: Option<(u64, u64)>,
        /// Output PNG path with %04d or {frame} for the frame number.
        #[arg(long, short)]
        output: PathBuf,
        /// Quality tier overriding the document's project@quality (draft renders at half size).
        #[arg(long, value_enum)]
        quality: Option<QualityArg>,
        /// Adaptive sampling for path-traced cameras, as render --path-noise: the preview samples as the render will.
        #[arg(long, value_name = "REL", value_parser = parse_path_noise)]
        path_noise: Option<f32>,
        /// Milliseconds between checks for changes.
        #[arg(long, default_value_t = 250)]
        interval: u64,
        /// Stop after this many renders (for scripts and tests).
        #[arg(long, hide = true)]
        max_runs: Option<u32>,
    },
    /// Render the document's outputs (or one ad-hoc output) to finished files.
    Encode {
        /// Scene document.
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// Render only these outputs (by id); all outputs by default.
        #[arg(long = "output", value_name = "ID")]
        outputs: Vec<String>,
        /// Output path. With one --output, that output is written here with all its settings; on its own, an
        /// ad-hoc output with default settings replaces the document's outputs.
        #[arg(short = 'o', long = "path", value_name = "PATH")]
        path: Option<String>,
        /// Codec of the ad-hoc output (default from the extension); a named output keeps its own.
        #[arg(long, requires = "path", conflicts_with = "outputs")]
        codec: Option<String>,
        /// Directory relative output paths resolve against (default: the document's).
        #[arg(long, value_name = "DIR")]
        out_dir: Option<PathBuf>,
        /// Preferred asset representation name (including proxy), or language tag.
        #[arg(long)]
        representation: Option<String>,
        /// Encoder family for H.264, H.265 and AV1.
        #[arg(long = "hw", value_parser = ["auto", "software", "nvenc", "videotoolbox", "vaapi", "qsv", "amf"], default_value = "auto")]
        hardware: String,
        /// Start time override, seconds.
        #[arg(long)]
        start: Option<f64>,
        /// End time override, seconds.
        #[arg(long)]
        end: Option<f64>,
        /// Skip destinations.
        #[arg(long)]
        no_upload: bool,
        /// Time segments of a video output rendered and encoded at once, then joined without
        /// re-encoding. `auto`, the default, splits by the programme alone (one segment per 10 s,
        /// at most 12, at most 128 frames each with flash or contrast checks, one stream under
        /// 20 s), so the file is the same on any machine; the machine only sets how many render at
        /// once (3 with a hardware encoder, else half the cores up to 4, one on a software adapter,
        /// at most one per 20 s). N renders N at once over three segments each; 1 renders serially.
        #[arg(long, value_name = "auto|N", value_parser = parse_parallel, default_value = "auto")]
        parallel: sr_deliver::Parallel,
        /// Parameter value, repeatable: --param id=value.
        #[arg(long = "param", value_name = "ID=VALUE", value_parser = parse_param)]
        params: Vec<(String, String)>,
        /// Data row for batch rendering.
        #[arg(long)]
        row: Option<usize>,
        /// Data source of --row (default: the first one).
        #[arg(long, requires = "row")]
        data: Option<String>,
        /// Exit 1 if anything was not rendered as authored: unsupported content, shader fallbacks
        /// (pass-through effects, crossfaded transitions), evaluator warnings or, when encoding,
        /// accessibility findings. Output files are still written.
        #[arg(long)]
        strict: bool,
        /// Quality tier overriding the document's project@quality. draft renders internally at half size (with
        /// fewer motion-blur samples and no grain) and scales the frames up to the output's size, so the file keeps
        /// its size and is softer.
        #[arg(long, value_enum)]
        quality: Option<QualityArg>,
        /// Adaptive sampling for path-traced cameras: a pixel stops once the standard error of its mean is this
        /// fraction of it (0.01 to 0.05 is typical). Each camera's pathSamples caps its pixels.
        #[arg(long, value_name = "REL", value_parser = parse_path_noise)]
        path_noise: Option<f32>,
        /// Also write each output's render report (scene-render-report/1, SREP 18) here, as output/@report does;
        /// with several outputs, include {id} in the path for the output's id.
        #[arg(long, value_name = "PATH")]
        report: Option<PathBuf>,
    },
    /// Simulate the document's physics and write its cache file (physics@cache).
    Simulate {
        /// Scene document.
        file: PathBuf,
        /// Cache file to write (default: the document's physics@cache).
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Bake one native pyro object to a verified composition-time volume sequence.
    BakeVolume(BakeArgs),
    /// Fill the caches of generated media and transcribed captions through their providers,
    /// and pin their SHA-256 in the document.
    ///
    /// Providers: whisper (whisper.cpp), piper, audioforge, the cloud services openai and
    /// elevenlabs (with --allow-cloud), or any `scene-render-provider-NAME` program speaking
    /// the JSON protocol. Unchanged targets are never made again.
    Resolve {
        /// Scene document (its cacheSha256 attributes are rewritten in place).
        file: PathBuf,
        /// Report what is stale and change nothing; exit 1 if anything is.
        #[arg(long)]
        check: bool,
        /// Make every target again.
        #[arg(long)]
        force: bool,
        /// Only these ids (repeatable).
        #[arg(long, value_name = "ID")]
        only: Vec<String>,
        /// Allow providers that send prompts to a cloud service.
        #[arg(long)]
        allow_cloud: bool,
        /// Do not read or write the shared result store.
        #[arg(long)]
        no_store: bool,
    },
    /// List the GPU adapters found and mark the one renders use (see SR_GPU_ADAPTER, SR_GPU_BACKEND).
    Gpus,
    /// Assign a path-traced document's frames to the GPU adapters, by each frame's path cost and each adapter's rate.
    ///
    /// Without --rate every frame goes to the best adapter: hardware with ray queries, then other hardware, then
    /// software. With rates, each frame goes to the adapter that would finish it first, as contiguous ranges, so the
    /// render ends as early as it can; an adapter without a rate then gets none. Render each range with SR_GPU_BACKEND and SR_GPU_ADAPTER set to its adapter.
    Schedule {
        /// Scene document.
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// An adapter's measured rate in path segments per second (repeatable): its name or part of it, ignoring case,
        /// then `=` and the rate, such as `rtx=2.6e9`; a whole name names its adapter even where another adapter's
        /// name contains it. An encode report's `paths.traced` over its render time measures it.
        #[arg(long, value_name = "NAME=RATE")]
        rate: Vec<String>,
    },
    /// Print this engine's capability manifest (scene-render-capabilities/1, SREP 22): the constructs of the scene
    /// format it does not draw exactly.
    Capabilities,
    /// Describe a diagnostic code, or list every code.
    Explain {
        /// Code such as S06, C21 or R24-fill; omit to list all codes.
        code: Option<String>,
    },
    /// Serve the read-only commands to a Model Context Protocol client over standard input and output.
    ///
    /// JSON-RPC 2.0, one message per line. The tools are validate, inspect, eval, captions, capabilities and explain;
    /// each returns the command's `--format json` envelope. Nothing is written and the program cache is off. The
    /// session ends when the input closes; --format does not apply.
    Mcp,
    /// Print a shell completion script.
    Completions {
        /// Target shell.
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

impl Command {
    /// The subcommand as it is typed, the envelope's `command`.
    fn name(&self) -> &'static str {
        match self {
            Command::Validate { .. } => "validate",
            Command::Inspect { .. } => "inspect",
            Command::Params { .. } => "params",
            Command::Eval { .. } => "eval",
            Command::Captions { .. } => "captions",
            Command::Render { .. } => "render",
            Command::Watch { .. } => "watch",
            Command::Encode { .. } => "encode",
            Command::Simulate { .. } => "simulate",
            Command::BakeVolume(_) => "bake-volume",
            Command::Resolve { .. } => "resolve",
            Command::Scopes { .. } => "scopes",
            Command::Gpus => "gpus",
            Command::Schedule { .. } => "schedule",
            Command::Capabilities => "capabilities",
            Command::Explain { .. } => "explain",
            Command::Mcp => "mcp",
            Command::Completions { .. } => "completions",
        }
    }
}

#[derive(serde::Serialize)]
struct FileReport<'a> {
    file: String,
    valid: bool,
    errors: usize,
    warnings: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    io_error: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    info: Vec<String>,
    diagnostics: &'a [Diagnostic],
}

/// What validation adds once a document is structurally valid: the errors compiling it for rendering would
/// raise (a key that does not parse as its property's type, an expression with an unknown function or a
/// property the element lacks), so that a document `validate` accepts is one `render` can compile; then the
/// `SA01` findings of `safeArea@enforce` over the frames that matter. Also the information lines: one `SA02`
/// per node carrying `safeAreaForce`, and how the frames were sampled.
/// SREP 67's build-stage checks of every compute node: its source can be read and matches @sha256 (CMP10), and it
/// compiles against the engine's prelude, defines sr_point and binds nothing in group 0 (CMP11). No device is needed.
fn compute_findings(doc: &sr_model::Document) -> Vec<Diagnostic> {
    use sr_model::element::Element;
    let mut out = Vec::new();
    let mut visit = |e: &dyn Element| {
        let Some(c) = e.as_any().downcast_ref::<sr_model::model::Compute>() else { return };
        let err = |code: &str, msg: String| Diagnostic::error(code, msg, c.loc, c.id.as_str());
        let bytes = match sr_model::assets::resolve(&c.src, doc.base_dir()) {
            sr_model::assets::Resolved::Local(path) => match std::fs::read(&path) {
                Ok(b) => b,
                Err(e) => {
                    return out.push(err("CMP10", format!("compute {:?}: cannot read {}: {e}", c.id, path.display())))
                }
            },
            _ => return out.push(err("CMP10", format!("compute {:?}: src {:?} is not a local file", c.id, c.src))),
        };
        if let Some(want) = &c.sha256 {
            let got: [u8; 32] = sha2::Digest::finalize(<sha2::Sha256 as sha2::Digest>::new_with_prefix(&bytes)).into();
            if got != want.0 {
                return out
                    .push(err("CMP10", format!("compute {:?}: the SHA-256 of {} differs from @sha256", c.id, c.src)));
            }
        }
        match String::from_utf8(bytes) {
            Ok(user) => {
                if let Err(e) = sr_gpu::compute::check_source(&user) {
                    out.push(err("CMP11", format!("compute {:?}: {e}", c.id)));
                }
            }
            Err(_) => out.push(err("CMP11", format!("compute {:?}: {} is not UTF-8", c.id, c.src))),
        }
    };
    sr_model::element::walk(&doc.scene as &dyn Element, &mut visit);
    out
}

fn compile_findings(text: &str, opts: &LoadOptions) -> (Vec<Diagnostic>, Vec<String>) {
    // loading runs what validation alone does not (SREP 66 build-mode programs, the model build) and render loads
    // the same way, so its errors are the document's: an output colliding with another id is PRG14 here as at
    // render. Its warnings are the validation pass's, already reported, except W13 (SREP 87).
    let doc = match sr_model::load_str(text, opts) {
        Ok(doc) => doc,
        Err(e) => {
            let found = e
                .report()
                .map(|r| r.diagnostics.iter().filter(|d| d.severity == Severity::Error).cloned().collect())
                .unwrap_or_default();
            return (found, Vec::new());
        }
    };
    // the bare references to program-emitted ids that only loading resolves (SREP 87: W13)
    let (mut found, mut info): (Vec<Diagnostic>, Vec<String>) =
        (doc.warnings().iter().filter(|d| d.code == "W13").cloned().collect(), Vec::new());
    // the document as authored, then each layout it declares: a layout has its own frame size, safe area and
    // overrides, and render and encode check the one they are asked for
    let layouts: Vec<Option<String>> = std::iter::once(None)
        .chain(doc.scene.layouts.iter().flat_map(|l| l.layouts.iter()).map(|l| Some(l.id.clone())))
        .collect();
    found.extend(compute_findings(&doc));
    for layout in layouts {
        let eo = sr_eval::EvalOptions { layout: layout.clone(), ..Default::default() };
        let tag = |mut d: Diagnostic| {
            if let Some(l) = &layout {
                d.message = format!("{} (layout {l})", d.message);
            }
            d
        };
        let ev = match sr_eval::Evaluator::new(&doc, &eo) {
            Ok(ev) => ev,
            Err(report) => {
                found.extend(report.diagnostics.into_iter().map(tag));
                continue;
            }
        };
        // what compiling warns about (an attribute this build does not read, a placeholder naming no parameter)
        // a warning every layout shares is listed once
        for d in ev.warnings().iter().cloned().map(tag) {
            if !found.iter().any(|f| f.code == d.code && f.message == d.message) {
                found.push(d);
            }
        }
        if ev.program().safe_enforce == sr_eval::SafeEnforce::Off && doc.scene.captions.is_none() {
            continue;
        }
        let out = sr_gpu::safe_audit::check(&ev, 0.0, ev.program().duration);
        for id in &out.forced {
            let line = format!("info[SA02]: {id} forced outside the safe area");
            if !info.contains(&line) {
                info.push(line);
            }
        }
        if let Some(n) = &out.note {
            let line = format!("info: safe area audit {n}");
            if !info.contains(&line) {
                info.push(line);
            }
        }
        found.extend(out.diagnostics(ev.program()).into_iter().map(tag));
    }
    (found, info)
}

fn validate(
    files: &[PathBuf],
    no_assets: bool,
    base_dir: Option<PathBuf>,
    deny_warnings: bool,
    quiet: bool,
    out: &mut Out,
) -> std::io::Result<u8> {
    let opts = LoadOptions { verify_assets: !no_assets, base_dir };
    let mut worst = 0u8;
    let mut json_files = Vec::new();
    let mut reports = Vec::new();
    let mut infos: std::collections::HashMap<PathBuf, Vec<String>> = Default::default();
    for file in files {
        let text = match sr_model::read_document(file) {
            Ok(t) => t,
            // past the document size limit: an XML error, the file left unread
            Err(LoadError::Invalid(report)) => {
                reports.push((file, Some((String::new(), report)), None));
                continue;
            }
            Err(LoadError::Io { source, .. }) => {
                reports.push((file, None, Some(source.to_string())));
                continue;
            }
        };
        let mut o = opts.clone();
        if o.base_dir.is_none() {
            o.base_dir = Some(match file.parent() {
                Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
                _ => PathBuf::from("."),
            });
        }
        let mut report = sr_model::validate_str(&text, &o);
        // SREP 87: an xs:IDREF may name an id a build-mode program emits, which loading (running the programs)
        // resolves. When every error is on an element holding an unresolved reference, loading decides: it reports
        // again what it cannot resolve
        let holders: std::collections::HashSet<String> =
            report.diagnostics.iter().filter(|d| d.code == "S10").map(|d| d.path.clone()).collect();
        let only_refs = report.has_errors()
            && report.diagnostics.iter().filter(|d| d.severity == Severity::Error).all(|d| holders.contains(&d.path));
        if !report.has_errors() || only_refs {
            let (found, info) = compile_findings(&text, &o);
            if only_refs {
                report.diagnostics.retain(|d| d.severity != Severity::Error);
            }
            report.diagnostics.extend(found);
            infos.insert(file.clone(), info);
        }
        reports.push((file, Some((text, report)), None));
    }
    let (mut invalid, mut codes) = (0usize, Vec::new());
    for (file, result, io) in &reports {
        match (result, io) {
            (_, Some(err)) => {
                worst = 2;
                let message = format!("cannot read {}: {err}", file.display());
                if out.json() {
                    json_files.push(FileReport {
                        file: file.display().to_string(),
                        valid: false,
                        errors: 0,
                        warnings: 0,
                        io_error: Some(err.clone()),
                        info: Vec::new(),
                        diagnostics: &[],
                    });
                    out.fail(ErrorKind::Io, message);
                } else {
                    let st = style(AnsiColor::Red, true);
                    let mut e = AutoStream::auto(std::io::stderr());
                    writeln!(e, "{st}error{st:#}: {message}")?;
                    out.record(ErrorKind::Io, message, Vec::new());
                }
            }
            (Some((text, report)), None) => {
                let ok = !(report.has_errors() || deny_warnings && report.warning_count() > 0);
                if !ok {
                    worst = worst.max(1);
                    invalid += 1;
                    let counted = |d: &&Diagnostic| {
                        d.severity == Severity::Error || deny_warnings && d.severity == Severity::Warning
                    };
                    codes.extend(report.diagnostics.iter().filter(counted).map(|d| d.code.clone()));
                }
                if out.json() {
                    json_files.push(FileReport {
                        file: file.display().to_string(),
                        valid: ok,
                        errors: report.error_count(),
                        warnings: report.warning_count(),
                        io_error: None,
                        info: infos.get(*file).cloned().unwrap_or_default(),
                        diagnostics: &report.diagnostics,
                    });
                } else {
                    if !quiet {
                        let lines = LineIndex::new(text);
                        for d in &report.diagnostics {
                            out.diagnostic(file, &lines, d)?;
                        }
                    }
                    for line in infos.get(*file).into_iter().flatten() {
                        writeln!(out.w, "{line}")?;
                    }
                    out.summary(file, report, deny_warnings)?;
                }
            }
            (None, None) => unreachable!(),
        }
    }
    if invalid > 0 {
        out.record(ErrorKind::Invalid, format!("{invalid} of {} file(s) invalid", files.len()), codes);
    }
    out.set_result(json!({ "valid": worst == 0, "files": json_files }));
    Ok(worst)
}

fn simulate(file: &Path, output: Option<PathBuf>, out: &mut Out) -> std::io::Result<u8> {
    use sha2::Digest;
    // the cache being regenerated may differ from its recorded digest, so assets are not verified
    let doc = match sr_model::load_file(file, &LoadOptions::without_assets()) {
        Ok(d) => d,
        Err(e) => {
            let codes = match &e {
                LoadError::Invalid(report) => {
                    if out.json() {
                        for d in &report.diagnostics {
                            out.note(file, d);
                        }
                    }
                    report.diagnostics.iter().filter(|d| d.is_error()).map(|d| d.code.clone()).collect()
                }
                LoadError::Io { .. } => Vec::new(),
            };
            let message = format!("{}: {e}", file.display());
            if out.echo() {
                eprintln!("error: {message}");
            }
            // an unreadable document has always exited 1 here, like an invalid one
            let kind = if codes.is_empty() { ErrorKind::Failed } else { ErrorKind::Invalid };
            out.record(kind, message, codes);
            return Ok(1);
        }
    };
    let dir = file.parent().map(Path::to_path_buf).unwrap_or_default();
    let target = match output.or_else(|| doc.scene.physics.as_ref().and_then(|p| p.cache.as_ref()).map(|c| dir.join(c)))
    {
        Some(t) => t,
        None => {
            return Ok(out.fail(
                ErrorKind::Usage,
                format!("{} has no physics@cache; name the cache file with -o", file.display()),
            ))
        }
    };
    let ev = match sr_eval::Evaluator::new(&doc, &sr_eval::EvalOptions::default()) {
        Ok(ev) => ev,
        Err(report) => return evaluator_errors(file, &report, out),
    };
    let bytes = match ev.physics_cache() {
        Ok(b) => b,
        Err(e) => return Ok(out.fail(ErrorKind::Failed, format!("{}: {e}", file.display()))),
    };
    if let Err(e) = std::fs::write(&target, &bytes) {
        return Ok(out.fail(ErrorKind::Io, format!("cannot write {}: {e}", target.display())));
    }
    let sha: String = sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    writeln!(out.w, "wrote {} ({} bytes)", target.display(), bytes.len())?;
    writeln!(out.w, "cacheSha256=\"{sha}\"")?;
    out.set_result(json!({ "cache": target.display().to_string(), "bytes": bytes.len(), "sha256": sha }));
    Ok(0)
}

/// Errors compiling a document for evaluation: one `error[CODE]: message` line each on standard error (and the
/// envelope's diagnostics), and the failure.
fn evaluator_errors(file: &Path, report: &Report, out: &mut Out) -> std::io::Result<u8> {
    for d in &report.diagnostics {
        if out.json() {
            out.note(file, d);
        } else {
            eprintln!("error[{}]: {}", d.code, d.message);
        }
    }
    let codes = report.diagnostics.iter().filter(|d| d.is_error()).map(|d| d.code.clone()).collect();
    out.record(ErrorKind::Invalid, format!("{}: {} error(s)", file.display(), report.error_count()), codes);
    Ok(1)
}

fn bake_volume(args: BakeArgs, out: &mut Out) -> std::io::Result<u8> {
    let BakeArgs { file, object, output, first, end, max_mib, params } = args;
    if std::fs::symlink_metadata(&output).is_ok() {
        return Ok(
            out.fail(ErrorKind::Usage, format!("{} already exists; choose a new cache directory", output.display()))
        );
    }
    let result = (|| -> Result<sr_volume::bake::BakeReceipt, String> {
        let bytes = max_mib
            .checked_mul(1 << 20)
            .filter(|n| *n > 0)
            .ok_or("--max-mib must be positive and fit a u64 byte count")?;
        let doc = sr_model::load_file(&file, &LoadOptions::default()).map_err(|e| e.to_string())?;
        let ev = sr_eval::Evaluator::new(&doc, &sr_eval::EvalOptions { params, ..Default::default() })
            .map_err(|r| r.diagnostics.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("; "))?;
        let warned: Vec<_> = ev.warnings().iter().filter(|d| !d.is_info()).collect();
        if !warned.is_empty() {
            return Err(warned.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("; "));
        }
        let end = end.unwrap_or(ev.frame_count());
        eprintln!("Baking {object}: project frames {first}..{end} into {}", output.display());
        let mut last = std::time::Instant::now();
        ev.bake_pyro_with_progress(
            &object,
            &output,
            first,
            end,
            sr_volume::bake::BakeLimits { max_total_bytes: bytes, ..Default::default() },
            |done, total| {
                if done == total || last.elapsed().as_secs() >= 1 {
                    eprintln!("Baked {done}/{total} frames");
                    last = std::time::Instant::now();
                }
            },
        )
    })();
    match result {
        Ok(receipt) => {
            writeln!(
                out.w,
                "wrote {} ({} frames, {} bytes)",
                receipt.manifest.display(),
                receipt.frames,
                receipt.bytes
            )?;
            writeln!(out.w, "sha256=\"{}\"", receipt.sha256)?;
            writeln!(out.w,"Use a volume asset with format=\"srvseq\" and this manifest/digest; replace {object}'s pyro child with the asset reference. Playback uses composition time.")?;
            out.set_result(json!({
                "manifest": receipt.manifest,
                "sha256": receipt.sha256,
                "frames": receipt.frames,
                "bytes": receipt.bytes,
                "clock": "composition",
                "object": object,
            }));
            Ok(0)
        }
        Err(e) => Ok(out.fail(ErrorKind::Failed, e)),
    }
}

fn resolve(file: &Path, o: &sr_resolve::Options, out: &mut Out) -> std::io::Result<u8> {
    use sr_resolve::Status;
    let rows = match sr_resolve::resolve(file, o) {
        Ok(r) => r,
        Err(e) => return Ok(out.fail(ErrorKind::Io, e.to_string())),
    };
    for r in &rows {
        let (label, color) = match r.status {
            Status::UpToDate => ("up to date", AnsiColor::Green),
            Status::Pinned => ("pinned", AnsiColor::Green),
            Status::Restored => ("restored", AnsiColor::Green),
            Status::Made => ("made", AnsiColor::Green),
            Status::Stale => ("stale", AnsiColor::Yellow),
            Status::Error => ("error", AnsiColor::Red),
        };
        let st = style(color, true);
        let sha = r.sha256.as_deref().map(|s| &s[..12.min(s.len())]).unwrap_or("");
        write!(out.w, "{st}{label:>10}{st:#}  {} ({}, {})  {}", r.id, r.element, r.provider, r.cache)?;
        if !sha.is_empty() {
            write!(out.w, "  {sha}…")?;
        }
        writeln!(out.w)?;
        if !r.message.is_empty() {
            writeln!(out.w, "            {}", r.message)?;
        }
        for n in &r.notes {
            writeln!(out.w, "            note: {n}")?;
        }
    }
    if rows.is_empty() {
        writeln!(out.w, "nothing to resolve: no <generated> or <pdf> assets or transcribed caption tracks")?;
    }
    let errors = rows.iter().filter(|r| r.status == Status::Error).count();
    let stale = rows.iter().filter(|r| o.check && r.status == Status::Stale).count();
    if errors > 0 {
        out.record(ErrorKind::Failed, format!("{errors} target(s) could not be made"), Vec::new());
    }
    if stale > 0 {
        out.record(ErrorKind::Failed, format!("{stale} target(s) stale"), Vec::new());
    }
    out.set_result(json!({ "targets": rows }));
    Ok(u8::from(errors + stale > 0))
}

fn params(file: &Path, no_assets: bool, out: &mut Out) -> std::io::Result<u8> {
    let opts = if no_assets { LoadOptions::without_assets() } else { LoadOptions::default() };
    let doc = match sr_model::load_file(file, &opts) {
        Ok(d) => d,
        Err(LoadError::Io { path, source }) => {
            return Ok(out.fail(ErrorKind::Io, format!("cannot read {}: {source}", path.display())))
        }
        Err(LoadError::Invalid(report)) => {
            let text = sr_model::read_document(file).unwrap_or_default();
            return out.report_invalid(file, &LineIndex::new(&text), &report);
        }
    };
    let mut params = Vec::new();
    let mut variants = Vec::new();
    for child in doc.scene.parameters.iter().flat_map(|p| &p.children) {
        match child {
            sr_model::model::ParametersChild::Param(p) => {
                let options: Vec<_> =
                    p.options.as_deref().unwrap_or("").split(',').map(str::trim).filter(|o| !o.is_empty()).collect();
                params.push(json!({
                    "id": p.id, "type": p.r#type, "default": p.default,
                    "label": p.label, "description": p.description, "required": p.required,
                    "min": p.min, "max": p.max, "maxLength": p.max_length,
                    "pattern": p.pattern, "options": options,
                }));
            }
            sr_model::model::ParametersChild::Variant(v) => {
                variants.push(json!({"id": v.id, "label": v.label}));
            }
            _ => {}
        }
    }
    let result = json!({ "params": params, "variants": variants });
    if !out.json() {
        let text = serde_json::to_string_pretty(&result).map_err(std::io::Error::other)?;
        writeln!(out.w, "{text}")?;
    }
    out.set_result(result);
    Ok(0)
}

fn inspect(file: &Path, no_assets: bool, out: &mut Out) -> std::io::Result<u8> {
    let opts = if no_assets { LoadOptions::without_assets() } else { LoadOptions::default() };
    let doc = match sr_model::load_file(file, &opts) {
        Ok(d) => d,
        Err(LoadError::Io { path, source }) => {
            return Ok(out.fail(ErrorKind::Io, format!("cannot read {}: {source}", path.display())))
        }
        Err(LoadError::Invalid(report)) => {
            let text = sr_model::read_document(file).unwrap_or_default();
            return out.report_invalid(file, &LineIndex::new(&text), &report);
        }
    };
    let s = &doc.scene;
    let (w, h) = doc.frame_size();
    let fps = doc.fps();
    let mut kinds: std::collections::BTreeMap<&str, usize> = Default::default();
    if let Some(a) = &s.assets {
        for c in &a.children {
            *kinds.entry(c.element_name()).or_default() += 1;
        }
    }
    let nodes = doc.composition_nodes();
    let depth = nodes.iter().map(|(d, _)| d + 1).max().unwrap_or(0);
    let symbols = s.symbols.as_ref().map_or(0, |x| x.symbols.len());
    let effects = s.effects.as_ref().map_or(0, |x| x.effects.len());
    // the document loaded, so it parses
    let path_costs =
        sr_model::read_document(file).ok().and_then(|text| sr_model::rules::path_costs(&text).ok()).unwrap_or_default();
    if out.json() {
        for d in doc.warnings() {
            out.note(file, d);
        }
        out.set_result(json!({
            "file": file.display().to_string(),
            "version": doc.version().to_string(),
            "width": w,
            "height": h,
            "fps": fps.to_string(),
            "fps_ratio": { "num": fps.num, "den": fps.den },
            "duration": doc.duration(),
            "frames": doc.frame_count(),
            "background": s.project.background.to_string(),
            "working_color_space": s.project.working_color_space.to_string(),
            "linear_light": s.project.linear_light,
            "assets": kinds,
            "nodes": nodes.len(),
            "depth": depth,
            "symbols": symbols,
            "effects": effects,
            "outputs": s.outputs.iter().map(|o| json!({ "id": o.id, "path": o.path, "codec": o.codec.to_string() })).collect::<Vec<_>>(),
            "ids": doc.index().len(),
            "warnings": doc.warnings().len(),
            "path_costs": path_costs.iter().map(|c| json!({
                "camera": c.camera,
                "width": c.width,
                "height": c.height,
                "samples": c.samples,
                "bounces": c.bounces,
                "segments": c.segments,
                "over_limit": c.over_limit(),
            })).collect::<Vec<_>>(),
            "model": &doc.scene,
        }));
        return Ok(0);
    }
    let bold = Style::new().bold();
    writeln!(out.w, "{bold}{}{bold:#}", file.display())?;
    writeln!(out.w, "  version      {}", doc.version())?;
    writeln!(out.w, "  frame        {w}x{h} @ {fps} fps")?;
    writeln!(out.w, "  duration     {} s = {} frames", doc.duration(), doc.frame_count())?;
    writeln!(out.w, "  background   {}", s.project.background)?;
    writeln!(
        out.w,
        "  color space  {}{}",
        s.project.working_color_space,
        if s.project.linear_light { " (linear light)" } else { "" }
    )?;
    let kinds: Vec<String> = kinds.iter().map(|(k, v)| format!("{v} {k}")).collect();
    writeln!(out.w, "  assets       {}", if kinds.is_empty() { "none".into() } else { kinds.join(", ") })?;
    writeln!(out.w, "  nodes        {} in the composition, depth {depth}", nodes.len())?;
    writeln!(out.w, "  symbols      {symbols}")?;
    if let Some(symbols) = &s.symbols {
        for (i, symbol) in symbols.symbols.iter().enumerate() {
            writeln!(out.w, "    {} [{}]", sr_model::diag::element_label(symbol, i), symbol.id)?;
        }
    }
    writeln!(out.w, "  effects      {effects}")?;
    writeln!(out.w, "  outputs      {}", s.outputs.len())?;
    for o in &s.outputs {
        writeln!(out.w, "    {} ({})", o.path, o.codec)?;
    }
    for c in &path_costs {
        writeln!(
            out.w,
            "  path cost    {}: {}x{}, {} samples, {} bounces, {:.2} billion segments per frame{}",
            if c.camera.is_empty() { "(camera)" } else { &c.camera },
            c.width,
            c.height,
            c.samples,
            c.bounces,
            c.segments as f64 / 1e9,
            if c.over_limit() { " (W15)" } else { "" }
        )?;
    }
    writeln!(out.w, "  ids          {}", doc.index().len())?;
    if !doc.warnings().is_empty() {
        writeln!(out.w, "  warnings     {}", doc.warnings().len())?;
    }
    Ok(0)
}

/// The adapters of the enabled backends, the ranked choice among them, and whether a device opened on each traverses
/// with ray queries (SR_GPU_RAY_QUERY=0 turns them off).
type Adapters = (Vec<sr_gpu::AdapterInfo>, Result<usize, sr_gpu::GpuError>, Vec<bool>);

fn list_adapters() -> Adapters {
    let opts = sr_gpu::gpu::GpuOptions::from_env();
    let (found, chosen) = sr_gpu::gpu::adapters(&opts);
    let setting = std::env::var("SR_GPU_RAY_QUERY").ok();
    let features = sr_gpu::gpu::adapter_features(&opts);
    let ray_query: Vec<bool> = found
        .iter()
        .map(|a| {
            features
                .iter()
                .any(|(info, f)| info == a && !sr_gpu::gpu::ray_query_features(*f, setting.as_deref()).is_empty())
        })
        .collect();
    (found, chosen, ray_query)
}

fn schedule(file: &Path, rates: &[String], out: &mut Out) -> std::io::Result<u8> {
    let doc = match sr_model::load_file(file, &LoadOptions::default()) {
        Ok(d) => d,
        Err(LoadError::Io { path, source }) => {
            return Ok(out.fail(ErrorKind::Io, format!("cannot read {}: {source}", path.display())))
        }
        Err(LoadError::Invalid(report)) => {
            let text = sr_model::read_document(file).unwrap_or_default();
            return out.report_invalid(file, &LineIndex::new(&text), &report);
        }
    };
    // the document loaded, so it parses
    let costs =
        sr_model::read_document(file).ok().and_then(|text| sr_model::rules::path_costs(&text).ok()).unwrap_or_default();
    let segments = costs.iter().fold(0u64, |sum, c| sum.saturating_add(c.segments));
    let frames = doc.frame_count();
    let (found, _, ray_query) = list_adapters();
    if found.is_empty() {
        let e = sr_gpu::GpuError::NoAdapter("no adapter was found".into());
        return Ok(out.fail(ErrorKind::Unavailable, e.to_string()));
    }
    let mut list: Vec<schedule::Adapter> = found
        .iter()
        .zip(&ray_query)
        .map(|(a, &rq)| schedule::Adapter {
            name: a.name.clone(),
            backend: format!("{:?}", a.backend),
            software: sr_gpu::gpu::is_software(a),
            ray_query: rq,
            rate: None,
        })
        .collect();
    let names = list.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ");
    for given in rates {
        let Some((part, value)) = given.split_once('=') else {
            return Ok(out.fail(ErrorKind::Usage, format!("--rate {given}: expected NAME=RATE")));
        };
        let rate = value.trim().parse::<f64>().ok().filter(|r| r.is_finite() && *r > 0.0);
        let Some(rate) = rate else {
            return Ok(out.fail(
                ErrorKind::Usage,
                format!("--rate {given}: the rate must be a positive number of path segments per second"),
            ));
        };
        let hit = match schedule::named(&list, part) {
            Ok(hit) => hit,
            Err(why) => {
                return Ok(out.fail(ErrorKind::Usage, format!("--rate {given} {why}; the adapters are: {names}")))
            }
        };
        for i in hit {
            list[i].rate = Some(rate);
        }
    }
    let shares = match schedule::schedule(&list, frames, segments) {
        Ok(s) => s,
        Err(e) => return Ok(out.fail(ErrorKind::Usage, e.to_string())),
    };
    out.set_result(json!({
        "file": file.display().to_string(),
        "frames": frames,
        "segments_per_frame": segments,
        "adapters": list.iter().map(|a| json!({
            "name": a.name,
            "backend": a.backend,
            "software": a.software,
            "ray_query": a.ray_query,
            "rate": a.rate,
        })).collect::<Vec<_>>(),
        "shares": shares.iter().map(|s| json!({
            "adapter": list[s.adapter].name,
            "backend": list[s.adapter].backend,
            "start": s.frames.start,
            "end": s.frames.end,
            "seconds": s.seconds,
        })).collect::<Vec<_>>(),
    }));
    for s in &shares {
        let a = &list[s.adapter];
        let n = s.frames.end - s.frames.start;
        let predicted = s.seconds.map(|t| format!(", predicted {t:.1} s")).unwrap_or_default();
        writeln!(
            out.w,
            "{} ({}): frames {}..{} ({n} frame{}){predicted}",
            a.name,
            a.backend,
            s.frames.start,
            s.frames.end,
            if n == 1 { "" } else { "s" }
        )?;
    }
    Ok(0)
}

fn gpus(out: &mut Out) -> std::io::Result<u8> {
    let (found, chosen, ray_query) = list_adapters();
    if found.is_empty() {
        let e = sr_gpu::GpuError::NoAdapter("no adapter was found".into());
        out.set_result(json!({ "adapters": [] }));
        return Ok(out.fail(ErrorKind::Unavailable, e.to_string()));
    }
    let chosen = match chosen {
        Ok(i) => Some(i),
        Err(e) => {
            out.fail(ErrorKind::Unavailable, e.to_string());
            None
        }
    };
    let list: Vec<_> = found
        .iter()
        .enumerate()
        .map(|(i, a)| {
            json!({
                "name": a.name,
                "backend": format!("{:?}", a.backend),
                "device_type": format!("{:?}", a.device_type),
                "driver": a.driver,
                "software": sr_gpu::gpu::is_software(a),
                "ray_query": ray_query[i],
                "chosen": chosen == Some(i),
            })
        })
        .collect();
    out.set_result(json!({ "adapters": list }));
    for (i, a) in found.iter().enumerate() {
        let mark = if chosen == Some(i) { '*' } else { ' ' };
        let soft = if sr_gpu::gpu::is_software(a) { ", software" } else { "" };
        let rays = if ray_query[i] { ", ray queries" } else { "" };
        writeln!(out.w, "{mark} {} ({:?}, {:?}{soft}{rays})", a.name, a.backend, a.device_type)?;
    }
    Ok(if chosen.is_some() { 0 } else { ErrorKind::Unavailable.exit() })
}

fn explain(code: Option<&str>, out: &mut Out) -> std::io::Result<u8> {
    let bold = Style::new().bold();
    let entry = |code: &str, stage: &str, summary: &str| json!({ "code": code, "stage": stage, "summary": summary });
    match code {
        None => {
            let mut list = Vec::new();
            for c in codes::all() {
                writeln!(out.w, "{bold}{:<16}{bold:#} {:<9} {}", c.code, c.stage, c.summary)?;
                list.push(entry(c.code, c.stage, c.summary));
            }
            for (c, summary) in sr_eval::codes::CODES {
                writeln!(out.w, "{bold}{c:<16}{bold:#} {:<9} {summary}", "eval")?;
                list.push(entry(c, "eval", summary));
            }
            out.set_result(json!({ "codes": list }));
            Ok(0)
        }
        Some(code) if sr_eval::codes::CODES.iter().any(|(c, _)| c.eq_ignore_ascii_case(code)) => {
            let (c, summary) = sr_eval::codes::CODES.iter().find(|(c, _)| c.eq_ignore_ascii_case(code)).unwrap();
            writeln!(out.w, "{bold}{c}{bold:#} (eval)\n{summary}")?;
            out.set_result(json!({ "codes": [entry(c, "eval", summary)] }));
            Ok(0)
        }
        Some(code) => match codes::lookup(code) {
            Some(c) => {
                writeln!(out.w, "{bold}{}{bold:#} ({})", c.code, c.stage)?;
                writeln!(out.w, "{}", c.summary)?;
                let mut e = entry(c.code, c.stage, c.summary);
                if let Some((ctx, test)) = c.rule {
                    writeln!(out.w, "\n  context: {ctx}\n  test:    {test}")?;
                    e["context"] = ctx.into();
                    e["test"] = test.into();
                }
                out.set_result(json!({ "codes": [e] }));
                Ok(0)
            }
            None => {
                let all = codes::all().len();
                Ok(out.fail(
                    ErrorKind::Usage,
                    format!("unknown code {code:?}; `scene-render explain` lists all {all} codes"),
                ))
            }
        },
    }
}

/// `scene-render captions`: the caption dump of `sr_gpu::caption_dump`, one JSON document on standard output (the
/// envelope's result with --format json).
fn captions(
    file: &Path,
    at: &[f64],
    opts: sr_eval::EvalOptions,
    no_assets: bool,
    out: &mut Out,
) -> std::io::Result<u8> {
    let lopts = if no_assets { LoadOptions::without_assets() } else { LoadOptions::default() };
    let report = |out: &mut Out, r: &Report| {
        for d in &r.diagnostics {
            out.note(file, d);
        }
        let message = format!("invalid {}: {} error(s)", file.display(), r.error_count());
        if out.echo() {
            eprintln!("{message}");
        }
        let codes = r.diagnostics.iter().filter(|d| d.is_error()).map(|d| d.code.clone()).collect();
        out.record(ErrorKind::Invalid, message, codes);
        1
    };
    let doc = match sr_model::load_file(file, &lopts) {
        Ok(d) => d,
        Err(LoadError::Io { path, source }) => {
            return Ok(out.fail(ErrorKind::Io, format!("cannot read {}: {source}", path.display())))
        }
        Err(LoadError::Invalid(r)) => return Ok(report(out, &r)),
    };
    let ev = match sr_eval::Evaluator::new(&doc, &opts) {
        Ok(e) => e,
        Err(r) => return Ok(report(out, &r)),
    };
    // the dump owns standard output: warnings and track errors go to standard error, one line each
    for w in ev.warnings() {
        out.note(file, w);
    }
    let dump = sr_gpu::caption_dump::dump(ev.program(), at);
    for t in &dump.tracks {
        if let Some(e) = &t.error {
            out.fail(ErrorKind::Failed, format!("caption track {}: {e}", t.id));
        }
    }
    if out.json() {
        out.set_result(serde_json::to_value(&dump).expect("serialisable"));
    } else {
        writeln!(out.w, "{}", serde_json::to_string_pretty(&dump).expect("serialisable"))?;
    }
    Ok(if dump.is_complete() { 0 } else { 1 })
}

#[allow(clippy::too_many_arguments)]
fn eval(
    file: &Path,
    time: Option<f64>,
    frame: Option<u64>,
    opts: sr_eval::EvalOptions,
    bench: bool,
    no_assets: bool,
    out: &mut Out,
) -> std::io::Result<u8> {
    let lopts = if no_assets { LoadOptions::without_assets() } else { LoadOptions::default() };
    let text = match sr_model::read_document(file) {
        Ok(t) => t,
        // past the document size limit: loading reports it
        Err(LoadError::Invalid(_)) => String::new(),
        Err(e) => return Ok(out.fail(ErrorKind::Io, e.to_string())),
    };
    let lines = LineIndex::new(&text);
    let doc = match sr_model::load_file(file, &lopts) {
        Ok(d) => d,
        Err(LoadError::Io { path, source }) => {
            return Ok(out.fail(ErrorKind::Io, format!("cannot read {}: {source}", path.display())))
        }
        Err(LoadError::Invalid(r)) => return out.report_invalid(file, &lines, &r),
    };
    let ev = match sr_eval::Evaluator::new(&doc, &opts) {
        Ok(e) => e,
        Err(r) => return out.report_invalid(file, &lines, &r),
    };
    // in JSON mode warnings go to the envelope and to standard error, one line each
    for w in ev.warnings() {
        out.diagnostic(file, &lines, w)?;
    }
    if bench {
        let n = ev.frame_count().max(1);
        let mut times: Vec<f64> = (0..n)
            .map(|i| {
                let t = std::time::Instant::now();
                std::hint::black_box(ev.evaluate_frame(i));
                t.elapsed().as_secs_f64() * 1e3
            })
            .collect();
        times.sort_by(f64::total_cmp);
        let (nodes, slots, channels, exprs) = ev.stats();
        let (median, p95, max) =
            (times[times.len() / 2], times[(times.len() * 95 / 100).min(times.len() - 1)], times[times.len() - 1]);
        writeln!(
            out.w,
            "{}: {nodes} nodes, {slots} properties, {channels} keyframe channels, {exprs} expressions",
            file.display()
        )?;
        writeln!(out.w, "{n} frames: median {median:.3} ms, p95 {p95:.3} ms, max {max:.3} ms per frame on one core")?;
        out.set_result(json!({
            "file": file.display().to_string(),
            "nodes": nodes,
            "properties": slots,
            "channels": channels,
            "expressions": exprs,
            "frames": n,
            "median_ms": median,
            "p95_ms": p95,
            "max_ms": max,
        }));
        return Ok(0);
    }
    let g = match (time, frame) {
        (Some(t), _) => ev.evaluate(t),
        (None, Some(f)) => ev.evaluate_frame(f),
        (None, None) => ev.evaluate(0.0),
    };
    if out.json() {
        out.set_result(serde_json::to_value(&g).expect("serialisable"));
        return Ok(0);
    }
    let bold = Style::new().bold();
    let dim = Style::new().dimmed();
    writeln!(
        out.w,
        "{bold}t = {:.4} s{bold:#}  frame {}  {}x{}  {} nodes",
        g.time,
        g.frame,
        g.size[0],
        g.size[1],
        g.nodes.len()
    )?;
    for n in &g.nodes {
        let o = n.world.apply([0.0, 0.0]);
        let rot = n.world.0[1].atan2(n.world.0[0]).to_degrees();
        let indent = "  ".repeat(n.depth as usize);
        let mut extra = String::new();
        if let Some(s) = n.source_time {
            extra.push_str(&format!(" src={s:.3}"));
        }
        if let Some(t) = &n.text {
            extra.push_str(&format!(" text={t:?}"));
        }
        if n.is_matte {
            extra.push_str(" matte-source");
        }
        for (k, v) in &n.props.0 {
            extra.push_str(&format!(" {k}={}", serde_json::to_string(v).unwrap_or_default()));
        }
        let hidden = if n.draw { "" } else { " (hidden)" };
        writeln!(
            out.w,
            "{indent}{bold}{}{bold:#} {dim}{}{hidden}{dim:#}  at ({:.1}, {:.1}) rot {:.1}° opacity {:.3} local {:.3}{extra}",
            n.id, n.kind, o[0], o[1], rot, n.world_opacity, n.local_time
        )?;
    }
    for t in &g.transitions {
        let name = |i: Option<u32>| i.map(|i| g.nodes[i as usize].id.to_string()).unwrap_or_else(|| "-".into());
        writeln!(out.w, "transition {} {} → {} progress {:.3}", t.kind, name(t.from), name(t.to), t.progress)?;
    }
    if let Some(c) = g.camera {
        writeln!(out.w, "camera {}", g.nodes[c as usize].id)?;
    }
    for e in &g.elements {
        writeln!(out.w, "{} <{}> {}", e.key, e.element, serde_json::to_string(&e.props).unwrap_or_default())?;
    }
    Ok(0)
}

fn parse_range(s: &str) -> Result<(u64, u64), String> {
    let (a, b, inclusive) = if let Some((a, b)) = s.split_once("..=") {
        (a, b, true)
    } else if let Some((a, b)) = s.split_once("..") {
        (a, b, false)
    } else {
        return Err("expected START..END or START..=END".into());
    };
    let a: u64 = a.trim().parse().map_err(|_| format!("bad start {a:?}"))?;
    let b: u64 = b.trim().parse().map_err(|_| format!("bad end {b:?}"))?;
    let end = if inclusive { b + 1 } else { b };
    if end <= a {
        return Err("empty frame range".into());
    }
    Ok((a, end))
}

fn frame_path(pattern: &Path, frame: u64, several: bool) -> Result<PathBuf, String> {
    let s = pattern.to_string_lossy();
    if s.contains("{frame}") {
        return Ok(PathBuf::from(s.replace("{frame}", &frame.to_string())));
    }
    // a printf pattern is in the file name: a `%` in a directory name is just that
    let name = pattern.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
    if let Some(i) = name.find('%') {
        let rest = &name[i + 1..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if rest[digits.len()..].starts_with('d') {
            let width: usize = digits.trim_start_matches('0').parse().unwrap_or(0);
            let num = format!("{frame:0width$}");
            return Ok(pattern.with_file_name(format!("{}{num}{}", &name[..i], &rest[digits.len() + 1..])));
        }
    }
    if several {
        Err(format!("{} names one file; use %04d or {{frame}} to write several frames", pattern.display()))
    } else {
        Ok(pattern.to_path_buf())
    }
}

/// What a `render` has written so far, and the seconds its stages took: evaluation, render, the wait for a frame's
/// readback, and writing the files (on the writer thread), as an encode reports them.
struct Progress {
    written: Vec<Value>,
    rendered: usize,
    /// When the fingerprint sidecar was last saved.
    recorded: std::time::Instant,
    stage_seconds: [f64; 4],
}

/// Saves the fingerprint sidecar of the frames written into `dir`, or warns.
fn record(side: &mut changes::Sidecar, dir: &Path) {
    if let Err(e) = side.save() {
        eprintln!("warning: cannot record frame fingerprints in {}: {e}", dir.display());
    }
}

/// Reports the frames the writer finished, in order: the `wrote` line and the report entry of each, and its
/// fingerprint recorded now that the file is there. The first failure's message, if a frame could not be written.
fn report_written(
    done: Vec<frames::Written>,
    out: &mut Out,
    progress: &mut Progress,
    sidecar: &mut changes::Sidecar,
    dir: &Path,
) -> std::io::Result<Option<String>> {
    for w in done {
        progress.stage_seconds[3] += w.seconds;
        if let Some(e) = w.error {
            return Ok(Some(e));
        }
        let f = w.frame;
        writeln!(out.w, "wrote {} (t = {:.4} s, {} draws)", f.path.display(), f.time, f.draws)?;
        progress.written.push(json!({ "path": f.path.display().to_string(), "time": f.time, "draws": f.draws }));
        progress.rendered += 1;
        if let Some(print) = f.print {
            sidecar.frames.insert(f.name, print);
            // now and then, so a run that is killed keeps most of what it rendered
            if progress.recorded.elapsed().as_secs() >= 2 {
                record(sidecar, dir);
                progress.recorded = std::time::Instant::now();
            }
        }
    }
    Ok(None)
}

/// Writes out the frame in flight and every frame queued at the writer, and reports them: what a run leaves
/// complete, whether it ends or stops. The first failure's message, if a frame could not be written.
fn drain(
    stage: &mut sr_gpu::output::OutputStage,
    in_flight: Option<(sr_gpu::output::Pending, frames::Frame)>,
    writer: frames::Writer,
    out: &mut Out,
    progress: &mut Progress,
    sidecar: &mut changes::Sidecar,
    dir: &Path,
) -> std::io::Result<Option<String>> {
    if let Some((pending, frame)) = in_flight {
        let waited = std::time::Instant::now();
        let bytes = stage.wait(pending);
        progress.stage_seconds[2] += waited.elapsed().as_secs_f64();
        writer.send(frame, bytes);
    }
    report_written(writer.finish(), out, progress, sidecar, dir)
}

#[allow(clippy::too_many_arguments)]
fn render(
    file: &Path,
    frames: Vec<u64>,
    time: Option<f64>,
    opts: sr_eval::EvalOptions,
    output: Option<PathBuf>,
    bit_depth: u8,
    bench: bool,
    stats: bool,
    pipelined: bool,
    strict: bool,
    quality: Option<sr_model::model::ProjectQuality>,
    path_noise: Option<f32>,
    inc: Incremental,
    out: &mut Out,
) -> std::io::Result<u8> {
    let text = match sr_model::read_document(file) {
        Ok(t) => t,
        // past the document size limit: loading reports it
        Err(LoadError::Invalid(_)) => String::new(),
        Err(e) => return Ok(out.fail(ErrorKind::Io, e.to_string())),
    };
    let lines = LineIndex::new(&text);
    let doc = match sr_model::load_file(file, &LoadOptions::default()) {
        Ok(d) => d,
        Err(LoadError::Io { path, source }) => {
            return Ok(out.fail(ErrorKind::Io, format!("cannot read {}: {source}", path.display())))
        }
        Err(LoadError::Invalid(r)) => return out.report_invalid(file, &lines, &r),
    };
    let ev = match sr_eval::Evaluator::new(&doc, &opts) {
        Ok(e) => e,
        Err(r) => return out.report_invalid(file, &lines, &r),
    };
    for w in doc.warnings().iter().chain(ev.warnings()) {
        out.diagnostic(file, &lines, w)?;
    }
    // information (an inert attribute) is shown but never counts toward --strict
    let eval_warnings = ev.warnings().iter().filter(|d| !d.is_info()).count();
    // content a safe area holds to its region, over the frames being rendered
    let safe = match (time, frames.is_empty() && bench) {
        (None, true) => sr_gpu::safe_audit::check(&ev, 0.0, ev.program().duration).diagnostics(ev.program()),
        _ => {
            let audited: Vec<f64> = match (time, frames.is_empty()) {
                (Some(t), _) => vec![t],
                (None, true) => vec![0.0],
                (None, false) => frames.iter().map(|&f| ev.program().fps.frame_time(f)).collect(),
            };
            sr_gpu::safe_audit::diagnostics(&ev, &audited)
        }
    };
    if safe.iter().any(|d| d.is_error()) {
        return out.report_invalid(file, &lines, &Report { diagnostics: safe });
    }
    for d in &safe {
        out.diagnostic(file, &lines, d)?;
    }
    // a renderer kept from an earlier render of this document (watch), while it still fits
    let inputs = changes::effective_files(file, &doc, ev.program())?;
    let persistent = sr_gpu::shader::has_persistent(ev.program(), doc.base_dir());
    let setup = changes::setup_key(&text, &inputs);
    let mut keep = inc.keep;
    let mut notes: Vec<String> = Vec::new();
    // Feedback checkpoints contain pixels from the previous document, even when renderer
    // setup is unchanged. Rebuild them on a watch run before replaying edited history.
    let kept = keep.as_mut().and_then(|k| k.take()).filter(|(key, _)| *key == setup && !persistent).map(|(_, r)| r);
    let mut r = match kept {
        Some(r) => r,
        None => {
            let gpu = match sr_gpu::Gpu::new_for(ev.program().uses_3d()) {
                Ok(g) => g,
                Err(e) => return Ok(out.fail(ErrorKind::Unavailable, e.to_string())),
            };
            if let Some(w) = sr_gpu::gpu::software_warning(&gpu.info) {
                writeln!(out.w, "warning: {w}")?;
                notes.push(format!("warning: {w}"));
            }
            sr_gpu::Renderer::new(gpu, ev.program())
        }
    };
    let adapter = sr_gpu::gpu::describe(&r.gpu().info);
    r.quality = quality;
    r.path_noise = path_noise;
    // GPU time, where the device has timestamp queries, with --stats: timing every pass costs
    // time itself (much of it on a software rasteriser), so a plain --bench stays untimed
    r.time_gpu = stats;
    let graphs = |f: u64| ev.evaluate_frame(f);
    let mut unsupported = std::collections::BTreeSet::new();
    let mut result = serde_json::Map::new();
    if bench {
        let frames = if frames.is_empty() { (0..ev.frame_count().max(1)).collect() } else { frames };
        let warm = frames.len().min(2);
        for &f in &frames[..warm] {
            let mut sub = |st: f64| ev.evaluate(st);
            let frame = r.render_with(&graphs(f), ev.program(), Some(&mut sub));
            if !frame.stats.errors.is_empty() {
                return Ok(report_frame_errors(out, &format!("frame {f}"), &frame.stats));
            }
            unsupported.extend(frame.stats.unsupported.iter().cloned());
            r.gpu().wait();
        }
        let mut times = Vec::new();
        let mut eval_ms = Vec::new();
        let mut submit_ms = Vec::new();
        let mut last = sr_gpu::RenderStats::default();
        let mut gpu_ms = Vec::new();
        let mut prev: Option<sr_gpu::SubmissionIndex> = None;
        for &f in &frames {
            let t0 = std::time::Instant::now();
            let g = graphs(f);
            let t1 = std::time::Instant::now();
            let mut sub = |st: f64| ev.evaluate(st);
            last = r.render_with(&g, ev.program(), Some(&mut sub)).stats;
            if !last.errors.is_empty() {
                return Ok(report_frame_errors(out, &format!("frame {f}"), &last));
            }
            submit_ms.push(t1.elapsed().as_secs_f64() * 1e3);
            if pipelined {
                if let Some(p) = prev.take() {
                    r.gpu().wait_for(p);
                }
                prev = r.last_submission();
            } else {
                r.gpu().wait();
            }
            eval_ms.push((t1 - t0).as_secs_f64() * 1e3);
            times.push(t1.elapsed().as_secs_f64() * 1e3);
            unsupported.extend(last.unsupported.iter().cloned());
            // pipelined, the frame's work may still run: read its times only when drained
            let gpu = if pipelined { None } else { r.gpu_times() };
            if let Some(ms) = gpu.as_ref().and_then(|g| g.frame_ms) {
                gpu_ms.push(ms);
            }
            if stats {
                let paths = if pipelined { None } else { r.path_samples() };
                eprintln!("{}", serde_json::json!({ "frame": f, "stats": &last, "gpu": gpu, "paths": paths }));
            }
        }
        if let Some(p) = prev.take() {
            r.gpu().wait_for(p);
        }
        let sorted = |mut v: Vec<f64>| {
            v.sort_by(f64::total_cmp);
            v
        };
        let (times, eval_ms, submit_ms, gpu_ms) = (sorted(times), sorted(eval_ms), sorted(submit_ms), sorted(gpu_ms));
        let med = times[times.len() / 2];
        let p95 = |v: &[f64]| v[(v.len() * 95 / 100).min(v.len() - 1)];
        result.insert("file".into(), json!(file.display().to_string()));
        result.insert("frames".into(), json!(frames.len()));
        result.insert("adapter".into(), json!(adapter));
        result.insert("pipelined".into(), json!(pipelined));
        result.insert("median_ms".into(), json!(med));
        result.insert("fps".into(), json!(1e3 / med));
        result.insert("p95_ms".into(), json!(p95(&times)));
        result.insert("submit_ms".into(), json!(submit_ms[submit_ms.len() / 2]));
        result.insert("eval_ms".into(), json!(eval_ms[eval_ms.len() / 2]));
        if !gpu_ms.is_empty() {
            result.insert("gpu_median_ms".into(), json!(gpu_ms[gpu_ms.len() / 2]));
            result.insert("gpu_p95_ms".into(), json!(p95(&gpu_ms)));
        }
        result.insert(
            "last".into(),
            json!({
                "draws": last.draws,
                "targets": last.targets,
                "cache_hits": last.cache_hits,
                "prefix_restored": last.prefix_restored,
                "backdrop_copies": last.backdrop_copies,
                "compositor_passes": last.compositor_passes,
                "fx_passes": last.fx_passes,
                "subframes": last.subframes,
                "objects3d": last.objects3d,
                "triangles": last.triangles,
                "splats": last.splats,
            }),
        );
        writeln!(
            out.w,
            "{}: {} frames on {adapter}{}",
            file.display(),
            frames.len(),
            if pipelined { ", pipelined" } else { "" }
        )?;
        writeln!(
            out.w,
            "render: median {med:.3} ms ({:.1} fps), p95 {:.3} ms; of which CPU planning and submission {:.3} ms; evaluation {:.3} ms",
            1e3 / med,
            times[(times.len() * 95 / 100).min(times.len() - 1)],
            submit_ms[submit_ms.len() / 2],
            eval_ms[eval_ms.len() / 2]
        )?;
        if !gpu_ms.is_empty() {
            writeln!(
                out.w,
                "gpu: median {:.3} ms, p95 {:.3} ms of GPU work per frame (timestamp queries)",
                gpu_ms[gpu_ms.len() / 2],
                gpu_ms[(gpu_ms.len() * 95 / 100).min(gpu_ms.len() - 1)]
            )?;
        }
        writeln!(
            out.w,
            "last frame: {} draws, {} targets, {} cache hits, {} restored, {} backdrop copies, {} compositor passes, {} effect passes, {} sub-frames",
            last.draws, last.targets, last.cache_hits, last.prefix_restored, last.backdrop_copies, last.compositor_passes, last.fx_passes, last.subframes
        )?;
        if last.objects3d > 0 || last.splats > 0 {
            writeln!(out.w, "3d: {} mesh draws, {} triangles, {} splats", last.objects3d, last.triangles, last.splats)?;
        }
    } else {
        let output = output.expect("clap requires --output");
        let list: Vec<(Option<u64>, f64)> = match (time, frames.is_empty()) {
            (Some(t), _) => vec![(None, t)],
            (None, true) => vec![(Some(0), 0.0)],
            (None, false) => frames.iter().map(|&f| (Some(f), ev.program().fps.frame_time(f))).collect(),
        };
        let several = list.len() > 1;
        let total = list.len();
        // --changed-only: fingerprints of what decides each frame's pixels, and the last ones
        // written beside the outputs
        let prints = inc.changed_only.then(|| {
            let effective = quality.unwrap_or(doc.scene.project.quality).as_str();
            let settings = format!("{effective} {bit_depth} {:?} {path_noise:?}", opts);
            changes::Fingerprints::new(&text, &doc, &inputs, &settings, ev.has_simulation(), persistent)
        });
        let dir = output.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")).to_path_buf();
        // every render keeps them true: a frame written anew loses the fingerprint recorded for the old one
        // before it is written, so a run that stops half way leaves none that its files do not match
        let mut sidecar = changes::Sidecar::load(&dir);
        let mut progress =
            Progress { written: Vec::new(), rendered: 0, recorded: std::time::Instant::now(), stage_seconds: [0.0; 4] };
        // frames are converted to 16-bit display sRGB with their alpha on the output stage, one read back while the
        // next renders, and written on a writer thread
        let gpu = r.gpu();
        let mut stage = sr_gpu::output::OutputStage::new(gpu.device.clone(), gpu.queue.clone());
        let srgb =
            sr_gpu::output::OutputColor::new(sr_model::model::ColorSpace::Srgb, sr_model::model::Transfer::Srgb, true);
        let working = r.working();
        let writer = frames::Writer::start(bit_depth);
        let mut in_flight: Option<(sr_gpu::output::Pending, frames::Frame)> = None;
        for (f, t) in list {
            let t0 = std::time::Instant::now();
            let path = match frame_path(&output, f.unwrap_or(0), several) {
                Ok(p) => p,
                Err(e) => return Ok(out.fail(ErrorKind::Usage, e)),
            };
            let g = ev.evaluate(t);
            let print = prints.as_ref().map(|p| format!("{:016x}", p.frame(&text, &g)));
            let name = path.strip_prefix(&dir).unwrap_or(&path).display().to_string();
            if print.is_some() && path.exists() && sidecar.frames.get(&name) == print.as_ref() {
                continue;
            }
            let mut sub = |st: f64| ev.evaluate(st);
            let t1 = std::time::Instant::now();
            let frame = r.render_with(&g, ev.program(), Some(&mut sub));
            unsupported.extend(frame.stats.unsupported.iter().cloned());
            if !frame.stats.errors.is_empty() {
                // the frames before it are written out: a run that stops leaves what it rendered
                if let Some(e) = drain(&mut stage, in_flight.take(), writer, out, &mut progress, &mut sidecar, &dir)? {
                    return Ok(out.fail(ErrorKind::Io, e));
                }
                return Ok(report_frame_errors(out, &format!("frame at {:.3} s", g.time), &frame.stats));
            }
            let size = frame.texture.size;
            let pending = stage.submit(&frame.texture, &working, &srgb, InputFormat::Rgba16, size, true);
            if let Err(e) = r.gpu().take_error() {
                if let Some(e) = drain(&mut stage, in_flight.take(), writer, out, &mut progress, &mut sidecar, &dir)? {
                    return Ok(out.fail(ErrorKind::Io, e));
                }
                return Ok(out.fail(ErrorKind::Failed, format!("frame at {:.3} s: {e}", g.time)));
            }
            progress.stage_seconds[0] += (t1 - t0).as_secs_f64();
            progress.stage_seconds[1] += t1.elapsed().as_secs_f64();
            if stats {
                let gpu = r.gpu_times();
                let paths = r.path_samples();
                eprintln!(
                    "{}",
                    serde_json::json!({ "frame": g.frame, "stats": &frame.stats, "gpu": gpu, "paths": paths })
                );
            }
            if sidecar.frames.remove(&name).is_some() {
                record(&mut sidecar, &dir);
            }
            // Unsupported output must be rendered again so --strict cannot lose diagnostics,
            // including when a prior invocation did not request strict validation.
            let print = print.filter(|_| frame.stats.unsupported.is_empty());
            let next = frames::Frame { path, name, size, time: g.time, draws: frame.stats.draws, print };
            if let Some((pending, frame)) = in_flight.replace((pending, next)) {
                let waited = std::time::Instant::now();
                let bytes = stage.wait(pending);
                progress.stage_seconds[2] += waited.elapsed().as_secs_f64();
                writer.send(frame, bytes);
            }
            if let Some(e) = report_written(writer.ready(), out, &mut progress, &mut sidecar, &dir)? {
                drain(&mut stage, in_flight.take(), writer, out, &mut progress, &mut sidecar, &dir)?;
                return Ok(out.fail(ErrorKind::Io, e));
            }
        }
        if let Some(e) = drain(&mut stage, in_flight.take(), writer, out, &mut progress, &mut sidecar, &dir)? {
            return Ok(out.fail(ErrorKind::Io, e));
        }
        record(&mut sidecar, &dir);
        let Progress { written, rendered, stage_seconds, .. } = progress;
        if let Some(prints) = &prints {
            if let Some(why) = &prints.whole {
                writeln!(out.w, "note: {why}: an edit to the composition renders every frame")?;
                notes.push(format!("{why}: an edit to the composition renders every frame"));
            }
            writeln!(out.w, "rendered {rendered} of {total} frames ({} unchanged)", total - rendered)?;
        }
        if several {
            writeln!(
                out.w,
                "stages: evaluation {:.2} s, render {:.2} s, readback wait {:.2} s, PNG writing {:.2} s",
                stage_seconds[0], stage_seconds[1], stage_seconds[2], stage_seconds[3]
            )?;
        }
        result.insert("written".into(), json!(written));
        result.insert("rendered".into(), json!(rendered));
        result.insert("total".into(), json!(total));
        result.insert("stage_seconds".into(), json!(stage_seconds));
    }
    if let Some(k) = keep {
        *k = Some((setup, r));
    }
    let problems = unsupported.len() + eval_warnings;
    for u in &unsupported {
        writeln!(out.w, "note: not rendered yet: {u}")?;
    }
    result.insert("unsupported".into(), json!(unsupported));
    result.insert("notes".into(), json!(notes));
    result.insert("warnings".into(), json!(eval_warnings));
    out.set_result(Value::Object(result));
    if strict && problems > 0 {
        return Ok(out.fail(
            ErrorKind::Failed,
            format!("--strict: {problems} item(s) not rendered as authored (see the notes and warnings above)"),
        ));
    }
    Ok(0)
}

/// Every error of a frame that cannot be written, and the notes about what else was not rendered as
/// authored, so that the cause of a failure is never left out; the exit status of the failure.
fn report_frame_errors(out: &mut Out, label: &str, stats: &sr_gpu::RenderStats) -> u8 {
    for error in &stats.errors {
        out.fail(ErrorKind::Failed, format!("{label}: {error}"));
    }
    for note in &stats.unsupported {
        if out.echo() {
            eprintln!("note: not rendered yet: {note}");
        }
    }
    ErrorKind::Failed.exit()
}

/// Incremental rendering: only changed frames, and a renderer kept between renders (watch).
struct Incremental<'a> {
    changed_only: bool,
    keep: Option<&'a mut Option<(u64, sr_gpu::Renderer)>>,
}

/// The document and the files it names, with their sizes and modification times.
fn watched_stamps(file: &Path) -> std::io::Result<Vec<(PathBuf, Option<changes::Stamp>)>> {
    let mut files = vec![file.to_path_buf()];
    if let Ok(doc) = sr_model::load_file(file, &LoadOptions::without_assets()) {
        files.extend(match sr_eval::Evaluator::new(&doc, &Default::default()) {
            Ok(ev) => changes::effective_files(file, &doc, ev.program()),
            Err(_) => changes::files(file, &doc),
        }?);
    }
    Ok(files.into_iter().map(|f| (f.clone(), changes::stamp(&f))).collect())
}

#[allow(clippy::too_many_arguments)]
fn watch(
    file: &Path,
    frames: Option<(u64, u64)>,
    output: PathBuf,
    quality: Option<sr_model::model::ProjectQuality>,
    path_noise: Option<f32>,
    interval: u64,
    max_runs: Option<u32>,
    out: &mut Out,
) -> std::io::Result<u8> {
    let mut keep: Option<(u64, sr_gpu::Renderer)> = None;
    let mut runs = 0u32;
    loop {
        // taken before rendering, so an edit made while it renders starts another render
        let before = watched_stamps(file)?;
        let list: Vec<u64> = match frames {
            Some((a, b)) => (a..b).collect(),
            None => {
                let n = sr_model::load_file(file, &LoadOptions::without_assets()).map(|d| d.frame_count()).unwrap_or(1);
                (0..n.max(1)).collect()
            }
        };
        let code = render(
            file,
            list,
            None,
            sr_eval::EvalOptions::default(),
            Some(output.clone()),
            8,
            false,
            false,
            false,
            false,
            quality,
            path_noise,
            Incremental { changed_only: true, keep: Some(&mut keep) },
            out,
        )?;
        runs += 1;
        if max_runs.is_some_and(|m| runs >= m) {
            return Ok(code);
        }
        // in JSON mode each render is one envelope, on its own line, as it finishes
        if out.json() {
            out.emit(Some("watch"), code);
        }
        writeln!(out.w, "watching {} for changes (Ctrl-C stops)", file.display())?;
        out.w.flush()?;
        // Resolve bindings once per render, then poll those files. A changed source or data
        // file starts another iteration, which discovers any newly selected dependencies.
        let stamps: Vec<_> = before.iter().map(|(_, s)| *s).collect();
        settled_change(
            &stamps,
            || before.iter().map(|(f, _)| changes::stamp(f)).collect(),
            || std::thread::sleep(std::time::Duration::from_millis(interval.max(20))),
        );
    }
}

/// Polls until the stamps differ from `before`, then until two polls one `pause` apart agree. A writer that
/// truncates a file and then writes it changes the stamp twice; a render between the two would read a partial file.
fn settled_change<S: PartialEq>(before: &[S], mut poll: impl FnMut() -> Vec<S>, mut pause: impl FnMut()) {
    let mut seen = poll();
    while seen == before {
        pause();
        seen = poll();
    }
    loop {
        pause();
        let next = poll();
        if next == seen {
            return;
        }
        seen = next;
    }
}

/// How `to` would change the file type `from` writes, if it would: the extension, or a frame pattern (`%`) in
/// the file name. The container and the sequence writer follow both, so a redirect may change neither.
fn file_type_change(from: &str, to: &str) -> Option<String> {
    let kind = |p: &str| {
        let name =
            Path::new(p.strip_prefix("file://").unwrap_or(p)).file_name().map(|n| n.to_string_lossy().to_lowercase());
        let name = name.unwrap_or_default();
        let ext = Path::new(&name).extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
        (ext, name.contains('%'))
    };
    let ((ext, seq), (to_ext, to_seq)) = (kind(from), kind(to));
    let what = |e: &str, s: bool| format!("{}.{e} files", if s { "numbered " } else { "" });
    (ext != to_ext || seq != to_seq).then(|| format!("writes {}, not {}", what(&ext, seq), what(&to_ext, to_seq)))
}

fn codec_for_path(p: &str) -> &'static str {
    let lower = p.to_ascii_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    let seq = Path::new(&lower).file_name().is_some_and(|n| n.to_string_lossy().contains('%'));
    match ext {
        "mov" => "prores",
        "mxf" => "dnxhr",
        "mkv" => "ffv1",
        "webm" => "vp9",
        "gif" => "gif",
        "webp" => "webp",
        "png" if seq => "png-sequence",
        "png" => "apng",
        "jpg" | "jpeg" => "jpeg-sequence",
        "exr" => "exr-sequence",
        "tif" | "tiff" => "tiff-sequence",
        "wav" | "m4a" | "mp3" => "audio-only",
        _ => "h264",
    }
}

/// Where the `--report` path puts the report of an output: `{id}` is the output's id (`output` without one).
fn cli_report_path(pattern: &Path, id: Option<&str>) -> PathBuf {
    PathBuf::from(pattern.to_string_lossy().replace("{id}", id.unwrap_or("output")))
}

/// Writes a render report, saying so on stderr; false when it could not be written.
fn write_render_report(report: &sr_deliver::render_report::RenderReport, path: &Path) -> bool {
    match sr_deliver::render_report::ReportWriter::write(report, path) {
        Ok(()) => {
            eprintln!("report: {}", path.display());
            true
        }
        Err(e) => {
            eprintln!("error: cannot write the render report {}: {e}", path.display());
            false
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn encode(
    file: &Path,
    ids: &[String],
    path: Option<String>,
    codec: Option<String>,
    opts: sr_deliver::Options,
    strict: bool,
    report_to: Option<&Path>,
    out: &mut Out,
) -> std::io::Result<u8> {
    use sr_deliver::render_report::{self, Finding, OutputInfo, ReportWriter};
    let bytes = match sr_model::limits::read_capped(file, sr_model::limits::MAX_DOCUMENT_BYTES) {
        Ok(b) => b,
        Err(e) => return Ok(out.fail(ErrorKind::Io, format!("cannot read {}: {e}", file.display()))),
    };
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let folder = match file.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let writer = ReportWriter::new(&bytes, &folder);
    let several = path.is_none() && ids.len() != 1;
    if let Some(r) = report_to.filter(|r| several && !r.to_string_lossy().contains("{id}")) {
        // one path for several outputs would keep only the last report
        let n = sr_model::load_file(file, &LoadOptions::without_assets()).map(|d| d.scene.outputs.len()).unwrap_or(2);
        if n > 1 {
            return Ok(out.fail(
                ErrorKind::Usage,
                format!("--report {}: the document has {n} outputs; include {{id}} in the path", r.display()),
            ));
        }
    }
    let lines = LineIndex::new(&text);
    let doc = match sr_model::load_file(file, &LoadOptions::default()) {
        Ok(d) => d,
        Err(LoadError::Io { path, source }) => {
            return Ok(out.fail(ErrorKind::Io, format!("cannot read {}: {source}", path.display())))
        }
        Err(LoadError::Invalid(r)) => {
            let exit = out.report_invalid(file, &lines, &r)?;
            // a render that stops on an error still writes its report, with the error among the findings (SREP 18)
            let findings: Vec<Finding> = r.diagnostics.iter().map(Finding::of_diagnostic).collect();
            let mut targets: Vec<(PathBuf, OutputInfo)> = Vec::new();
            match &path {
                Some(_) if ids.is_empty() => {
                    if let Some(rp) = report_to {
                        let info = OutputInfo { id: None, width: 0, height: 0, start: 0.0, end: 0.0 };
                        targets.push((cli_report_path(rp, None), info));
                    }
                }
                // the document's outputs, or the one -o redirects
                _ => {
                    for o in writer.raw_outputs() {
                        if !ids.is_empty() && !o.info.id.as_ref().is_some_and(|i| ids.contains(i)) {
                            continue;
                        }
                        if let Some(rp) = &o.report {
                            targets.push((folder.join(rp.strip_prefix("file://").unwrap_or(rp)), o.info.clone()));
                        }
                        if let Some(rp) = report_to {
                            targets.push((cli_report_path(rp, o.info.id.as_deref()), o.info.clone()));
                        }
                    }
                }
            }
            for (rp, info) in targets {
                write_render_report(&writer.report(info, findings.clone()), &rp);
            }
            return Ok(exit);
        }
    };
    let outputs: Vec<sr_model::model::Output> = match (path, ids) {
        (Some(p), []) => {
            let c = codec.unwrap_or_else(|| codec_for_path(&p).to_string());
            match sr_deliver::adhoc_output(&p, &c) {
                Ok(o) => vec![o],
                Err(e) => return Ok(out.fail(ErrorKind::Usage, e.to_string())),
            }
        }
        // a named output written elsewhere keeps every setting but its path, so the path keeps its file type
        (Some(p), [id]) => match doc.scene.outputs.iter().find(|o| o.id.as_deref() == Some(id.as_str())) {
            Some(o) => match file_type_change(&o.path, &p) {
                None => vec![sr_model::model::Output { path: p, ..o.clone() }],
                Some(e) => return Ok(out.fail(ErrorKind::Usage, format!("-o {p}: output {id:?} {e}"))),
            },
            None => {
                let message = format!("{} has no <output id=\"{id}\"> for -o to redirect", file.display());
                return Ok(out.fail(ErrorKind::Usage, message));
            }
        },
        (Some(_), several) => {
            let message = format!("-o redirects one --output; {} were given", several.len());
            return Ok(out.fail(ErrorKind::Usage, message));
        }
        (None, _) => doc
            .scene
            .outputs
            .iter()
            .filter(|o| ids.is_empty() || o.id.as_ref().is_some_and(|i| ids.contains(i)))
            .cloned()
            .collect(),
    };
    // where each output's report goes, and the output as the document gives it, for a run that stops before rendering
    let report_targets = |outputs: &[sr_model::model::Output]| -> Vec<(PathBuf, OutputInfo)> {
        let (pw, ph) = doc.frame_size();
        let mut t = Vec::new();
        for o in outputs {
            let info = OutputInfo {
                id: o.id.clone(),
                width: o.width.map_or(pw as u32, |w| w as u32),
                height: o.height.map_or(ph as u32, |h| h as u32),
                start: opts.start.unwrap_or(o.start),
                end: opts.end.or(o.end).unwrap_or(doc.scene.project.duration.get()),
            };
            t.extend(render_report::report_path(&folder, o).map(|p| (p, info.clone())));
            if let Some(rp) = report_to {
                t.push((cli_report_path(rp, o.id.as_deref()), info));
            }
        }
        t
    };
    // a render that stops on an error still writes its reports, with the error among the findings (SREP 18, 1.3)
    let stop_early = |targets: Vec<(PathBuf, OutputInfo)>, message: String| {
        let mut findings: Vec<Finding> = doc.warnings().iter().map(Finding::of_diagnostic).collect();
        findings.push(Finding::scene(render_report::code::engine("DELIVERY"), Severity::Error, message));
        for (rp, info) in targets {
            write_render_report(&writer.report(info, findings.clone()), &rp);
        }
    };
    if outputs.is_empty() {
        let message = format!("{} defines no matching <output>; pass -o PATH for an ad-hoc output", file.display());
        let exit = out.fail(ErrorKind::Usage, message.clone());
        if let Some(rp) = report_to {
            let info = OutputInfo { id: None, width: 0, height: 0, start: 0.0, end: 0.0 };
            stop_early(vec![(cli_report_path(rp, None), info)], message);
        }
        return Ok(exit);
    }
    // the adapter is chosen from the document: one that cannot run the 3D pass is skipped when an output draws in 3D
    let needs_3d = outputs
        .iter()
        .filter(|o| o.codec.as_str() != "audio-only")
        .map(|o| sr_deliver::output_uses_3d(&doc, o, &opts))
        .collect::<Result<Vec<_>, _>>()
        .map(|v| v.contains(&true));
    let gpu = match needs_3d {
        Ok(needs_3d) => {
            outputs.iter().any(|o| o.codec.as_str() != "audio-only").then(|| sr_gpu::Gpu::new_for(needs_3d)).transpose()
        }
        Err(e) => {
            let exit = out.fail(ErrorKind::Usage, e.to_string());
            stop_early(report_targets(&outputs), e.to_string());
            return Ok(exit);
        }
    };
    let gpu = match gpu {
        Ok(g) => g,
        Err(e) => {
            let exit = out.fail(ErrorKind::Unavailable, e.to_string());
            stop_early(report_targets(&outputs), e.to_string());
            return Ok(exit);
        }
    };
    let tty = std::io::IsTerminal::is_terminal(&std::io::stderr());
    if let Some(g) = gpu.as_ref() {
        // say which adapter renders: a software or GL fallback would otherwise only show up as a very slow encode
        eprintln!("rendering on {} ({:?}, {:?})", g.info.name, g.info.backend, g.info.device_type);
        if g.is_software() {
            eprintln!(
                "warning: {} is a software or GL adapter; expect encodes far slower than on the GPU",
                g.info.name
            );
        }
    }
    let mut failed = false;
    let mut delivered = Vec::new();
    for o in &outputs {
        // On a terminal: a live counter. Otherwise (a log file, a CI job) a line about every 10 s with the
        // rate and the time left, so a long encode never runs blind.
        let started = std::time::Instant::now();
        let mut last_line = started;
        // per interval: frames, stage times, process CPU time; and the best rate seen, to flag a collapse
        let mut last = (0u64, sr_deliver::pipeline::meter::snapshot(), process_cpu_seconds());
        let mut best_rate = 0.0f64;
        let mut progress = |done: u64, total: u64| {
            if tty {
                if done.is_multiple_of(10) || done == total {
                    eprint!("\r{}: frame {done}/{total}", o.path);
                    if done == total {
                        eprintln!();
                    }
                }
            } else if done > 0 && (last_line.elapsed().as_secs_f64() >= 10.0 || done == total) {
                last_line = std::time::Instant::now();
                let fps = done as f64 / started.elapsed().as_secs_f64().max(1e-9);
                let left = total.saturating_sub(done) as f64 / fps.max(1e-9);
                let dt = last_line.duration_since(started).as_secs_f64();
                let (d0, st0, cpu0) = last;
                let (st1, cpu1) = (sr_deliver::pipeline::meter::snapshot(), process_cpu_seconds());
                let span = (dt - LAST_SPAN.with(|c| c.replace(dt))).max(1e-9);
                let n = (done - d0).max(1) as f64;
                let rate = (done - d0) as f64 / span;
                let ms = |i: usize| 1e3 * (st1[i] - st0[i]) / n;
                let cpu = match (cpu0, cpu1) {
                    (Some(a), Some(b)) => format!(", CPU {:.0} %", 100.0 * (b - a) / span),
                    _ => String::new(),
                };
                eprintln!(
                    "{}: frame {done}/{total} ({:.1} %), {fps:.1} frames/s, about {left:.0} s left; last {span:.0} s: {rate:.1} frames/s, per frame evaluate {:.0} ms, render {:.0} ms, GPU wait {:.0} ms, encoder {:.0} ms{cpu}",
                    o.path,
                    100.0 * done as f64 / total.max(1) as f64,
                    ms(0), ms(1), ms(2), ms(3)
                );
                if dt > 60.0 && best_rate > 0.0 && rate < best_rate / 3.0 {
                    eprintln!(
                        "warning: {}: the rate fell to {rate:.1} frames/s from {best_rate:.1}; see the per-frame stage times above (CPU near 100 % with little GPU wait means the CPU work itself got slower or starved; a long GPU wait means the GPU is shared or stalled)",
                        o.path
                    );
                }
                best_rate = best_rate.max(rate);
                last = (done, st1, cpu1);
            }
        };
        let mut report_paths: Vec<PathBuf> = render_report::report_path(&folder, o).into_iter().collect();
        if let Some(rp) = report_to {
            report_paths.push(cli_report_path(rp, o.id.as_deref()));
        }
        // --strict counts every finding at warning or error (SREP 18: only information is left out), so it needs
        // the measures a report has
        let o_opts = sr_deliver::Options { report: !report_paths.is_empty() || strict, ..opts.clone() };
        let (r, outcome) = sr_deliver::deliver_reporting(&doc, o, gpu.as_ref(), &o_opts, &mut progress);
        let mut counted = 0;
        if !report_paths.is_empty() || strict {
            let mut findings: Vec<Finding> = doc.warnings().iter().map(Finding::of_diagnostic).collect();
            findings.extend(r.evaluation_warnings.iter().map(Finding::of_diagnostic));
            findings.extend(
                r.unsupported
                    .iter()
                    .map(|u| Finding::scene(render_report::code::engine("UNSUPPORTED"), Severity::Warning, u.clone())),
            );
            findings.extend(r.findings.iter().cloned());
            // SREP 22: what this engine does not draw exactly, as the document uses it
            findings.extend(sr_deliver::capabilities::findings(&sr_deliver::capabilities::manifest(), &text));
            if let Err(e) = &outcome {
                findings.extend(render_report::error_findings(e));
            }
            let (pw, ph) = doc.frame_size();
            let [width, height] = if r.size != [0, 0] {
                r.size
            } else {
                [o.width.map_or(pw as u32, |w| w as u32), o.height.map_or(ph as u32, |h| h as u32)]
            };
            let [start, end] = if r.range[1] > r.range[0] {
                r.range
            } else {
                [opts.start.unwrap_or(o.start), opts.end.or(o.end).unwrap_or(doc.scene.project.duration.get())]
            };
            let info = OutputInfo { id: o.id.clone(), width, height, start, end };
            let report = writer.report(info, findings);
            counted = report.findings.iter().filter(|f| f.severity != Severity::Info).count();
            for rp in &report_paths {
                if !write_render_report(&report, rp) {
                    out.record(
                        ErrorKind::Failed,
                        format!("cannot write the render report {}", rp.display()),
                        Vec::new(),
                    );
                    failed = true;
                }
            }
        }
        match outcome.map(|()| r) {
            Ok(r) => {
                // every finding of the output's report at warning or error severity: validation and evaluator
                // warnings, unsupported content, accessibility and legibility, safe area, text fit, fonts, engine
                // support; information (inert attributes) is not counted (SREP 18)
                if strict && counted > 0 {
                    out.fail(
                        ErrorKind::Failed,
                        format!(
                            "--strict: {}: {counted} finding(s) at warning or error severity (the render report's findings; information is not counted)",
                            o.path
                        ),
                    );
                    failed = true;
                }
                if out.json() {
                    delivered.push(serde_json::to_value(&r).expect("serialisable"));
                    continue;
                }
                let bold = Style::new().bold();
                writeln!(out.w, "{bold}wrote{bold:#} {} ({} file(s), {})", r.path.display(), r.files.len(), r.encoder)?;
                if let Some(a) = &r.render_adapter {
                    writeln!(out.w, "  render adapter: {} ({}, {})", a.name, a.backend, a.device_type)?;
                    if r.quality != "final" {
                        writeln!(out.w, "  quality: {} (see project@quality)", r.quality)?;
                    }
                    if a.software {
                        writeln!(
                            out.w,
                            "  warning: rendering on a software adapter ({}): expect renders many times slower than on a GPU; `scene-render gpus` lists the adapters found and SR_GPU_ADAPTER picks one",
                            a.name
                        )?;
                    }
                }
                if let Some(why) = &r.serial_because {
                    writeln!(out.w, "  note: {why}")?;
                }
                if r.frames > 0 {
                    let [e, s, w, b] = r.stage_seconds;
                    writeln!(
                        out.w,
                        "  {} frames {}x{} at {} fps in {:.2} s: {:.1} frames/s (evaluate {:.2} s, render {:.2} s of which decode wait {:.2} s, vector geometry {:.2} s and shutter sub-frames {:.2} s, GPU/readback blocked {:.2} s, encoder blocked {:.2} s), {} pass(es){}",
                        r.frames, r.size[0], r.size[1], r.fps, r.seconds, r.render_fps, e, s, r.decode_wait_seconds, r.vector_seconds, r.subframe_seconds, w, b, r.passes,
                        if r.segments > 1 { format!(", {} segments at once (stage times summed over them)", r.segments) } else { String::new() }
                    )?;
                }
                if let Some(p) = r.paths {
                    let share = if p.budget > 0 { 100.0 * p.traced as f64 / p.budget as f64 } else { 100.0 };
                    writeln!(
                        out.w,
                        "  paths: {} of {} samples traced ({share:.1}%){}",
                        p.traced,
                        p.budget,
                        if p.hardware_rays { ", hits found with the adapter's ray queries" } else { "" }
                    )?;
                }
                if let Some(tp) = r.true_peak {
                    let l = r
                        .loudness
                        .map(|l| format!("{l:.1} LUFS integrated"))
                        .unwrap_or_else(|| "too short for integrated loudness".into());
                    let mut parts = vec![l];
                    parts.extend(r.loudness_range.map(|v| format!("{v:.1} LU loudness range")));
                    parts.extend(r.momentary_max.map(|v| format!("{v:.1} LUFS momentary max")));
                    parts.extend(r.short_term_max.map(|v| format!("{v:.1} LUFS short-term max")));
                    parts.push(format!("{tp:.1} dBTP true peak"));
                    writeln!(out.w, "  audio: {}, mixed in {:.2} s", parts.join(", "), r.audio_seconds)?;
                    if let Some(h) = &r.audio_ceiling {
                        if h.passes.is_empty() {
                            writeln!(
                                out.w,
                                "  audio ceiling: the AAC decodes to {:.2} dBTP, under the ceiling of {:.2}",
                                h.first, h.ceiling
                            )?;
                        } else {
                            let passes: Vec<String> = h
                                .passes
                                .iter()
                                .enumerate()
                                .map(|(i, p)| {
                                    format!(
                                        "pass {}: mixed down {:.2} dB, now {:.2} dBTP",
                                        i + 1,
                                        -p.gain_db,
                                        p.decoded_true_peak
                                    )
                                })
                                .collect();
                            writeln!(
                                out.w,
                                "  audio ceiling: the AAC decodes to {:.2} dBTP over a ceiling of {:.2}; {}",
                                h.first,
                                h.ceiling,
                                passes.join("; ")
                            )?;
                        }
                    }
                }
                for u in &r.uploads {
                    writeln!(out.w, "  delivered to {u}")?;
                }
                for u in &r.unsupported {
                    writeln!(out.w, "  note: not rendered yet: {u}")?;
                }
                for w in &r.warnings {
                    writeln!(out.w, "  warning: {w}")?;
                }
                for w in &r.evaluation_warnings {
                    out.diagnostic(file, &lines, w)?;
                }
                for a in &r.accessibility {
                    writeln!(out.w, "  accessibility: {a}")?;
                }
            }
            Err(e) => {
                out.fail(ErrorKind::Failed, format!("{}: {e}", o.path));
                failed = true;
            }
        }
    }
    out.set_result(json!({ "outputs": delivered }));
    Ok(u8::from(failed))
}

thread_local! {
    /// Seconds from the encode's start to the previous progress line.
    static LAST_SPAN: std::cell::Cell<f64> = const { std::cell::Cell::new(0.0) };
}

/// User plus system CPU time of this process (Linux /proc), for the progress lines.
fn process_cpu_seconds() -> Option<f64> {
    let s = std::fs::read_to_string("/proc/self/stat").ok()?;
    let f: Vec<&str> = s.rsplit_once(')')?.1.split_whitespace().collect();
    let ticks = f.get(11)?.parse::<f64>().ok()? + f.get(12)?.parse::<f64>().ok()?;
    Some(ticks / 100.0)
}

/// Exit status of a run that could not complete: a usage or I/O error, or an internal error
/// (a panic) reported by [`install_panic_hook`].
const COULD_NOT_RUN: u8 = 2;

/// Report a panic as one `internal error: <message> at <file>:<line>` line; with `RUST_BACKTRACE`
/// set the previous hook still runs after it, so the standard report and its backtrace follow.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        eprintln!("{}", internal_error(panic_message(info.payload()), info.location()));
        if std::env::var_os("RUST_BACKTRACE").is_some_and(|v| v != "0") {
            previous(info);
        }
    }));
}

fn internal_error(message: &str, at: Option<&std::panic::Location<'_>>) -> String {
    match at {
        Some(at) => format!("internal error: {message} at {}:{}", at.file(), at.line()),
        None => format!("internal error: {message}"),
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    match (payload.downcast_ref::<&str>(), payload.downcast_ref::<String>()) {
        (Some(s), _) => s,
        (_, Some(s)) => s,
        _ => "no message",
    }
}

/// Runs a command: its exit status, with an I/O error or a panic recorded as its failure.
fn run(command: Command, out: &mut Out) -> u8 {
    // nothing the closure touches is used after a panic but the record of the failure
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| dispatch(command, out)));
    let code = settle(res, out);
    out.exit_status(code)
}

/// The exit status of a command's outcome: a closed pipe ends the run quietly, an I/O error is the failure of a
/// run that could not complete, and a panic (already reported by the hook) is an internal error.
fn settle(res: std::thread::Result<std::io::Result<u8>>, out: &mut Out) -> u8 {
    match res {
        Ok(Ok(code)) => code,
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::BrokenPipe => 0,
        Ok(Err(e)) => out.fail(ErrorKind::Io, e.to_string()),
        Err(payload) => {
            out.record(ErrorKind::Internal, internal_error(panic_message(&*payload), None), Vec::new());
            COULD_NOT_RUN
        }
    }
}

/// `scopes`: the levels report of a video or still, `scene-render-scopes/1`, as text or the envelope's result.
fn scopes(file: &Path, step: u64, chroma_limit: Option<f32>, out: &mut Out) -> std::io::Result<u8> {
    let report = match sr_media::scopes::analyse(file, step, chroma_limit) {
        Ok(r) => r,
        Err(e) => {
            let kind =
                if matches!(e, sr_media::scopes::ScopeError::Invalid(_)) { ErrorKind::Usage } else { ErrorKind::Io };
            return Ok(out.fail(kind, e.to_string()));
        }
    };
    if out.json() {
        match serde_json::to_value(&report) {
            Ok(v) => out.set_result(v),
            Err(e) => return Ok(out.fail(ErrorKind::Internal, format!("the scopes report: {e}"))),
        }
    }
    write!(out.w, "{}", report.human())?;
    Ok(0)
}

fn dispatch(command: Command, out: &mut Out) -> std::io::Result<u8> {
    match command {
        Command::Validate { files, no_assets, base_dir, deny_warnings, quiet } => {
            validate(&files, no_assets, base_dir, deny_warnings, quiet, out)
        }
        Command::Inspect { file, no_assets } => inspect(&file, no_assets, out),
        Command::Params { file, no_assets } => params(&file, no_assets, out),
        Command::Eval { file, time, frame, variant, layout, params, row, data, bench, no_assets } => {
            let opts =
                sr_eval::EvalOptions { variant, layout, params, row: row.map(|r| (data, r)), ..Default::default() };
            eval(&file, time, frame, opts, bench, no_assets, out)
        }
        Command::Captions { file, at, variant, layout, params, no_assets } => {
            let opts = sr_eval::EvalOptions { variant, layout, params, ..Default::default() };
            captions(&file, &at, opts, no_assets, out)
        }
        Command::Render {
            file,
            time,
            frame,
            frames,
            output,
            bit_depth,
            variant,
            layout,
            params,
            row,
            data,
            bench,
            stats,
            pipelined,
            strict,
            quality,
            path_noise,
            changed_only,
        } => {
            let opts =
                sr_eval::EvalOptions { variant, layout, params, row: row.map(|r| (data, r)), ..Default::default() };
            let list: Vec<u64> = match (frame, frames) {
                (Some(f), _) => vec![f],
                (None, Some((a, b))) => (a..b).collect(),
                (None, None) => Vec::new(),
            };
            render(
                &file,
                list,
                time,
                opts,
                output,
                bit_depth.parse().unwrap_or(8),
                bench,
                stats,
                pipelined,
                strict,
                quality.map(QualityArg::model),
                path_noise,
                Incremental { changed_only, keep: None },
                out,
            )
        }
        Command::Watch { file, frames, output, quality, path_noise, interval, max_runs } => {
            watch(&file, frames, output, quality.map(QualityArg::model), path_noise, interval, max_runs, out)
        }
        Command::Encode {
            file,
            outputs,
            path,
            codec,
            out_dir,
            representation,
            hardware,
            start,
            end,
            no_upload,
            parallel,
            params,
            row,
            data,
            strict,
            quality,
            path_noise,
            report,
        } => {
            let opts = sr_deliver::Options {
                out_dir,
                representation,
                hardware: sr_media::encode::Hardware::parse(&hardware).unwrap_or(sr_media::encode::Hardware::Auto),
                start,
                end,
                upload: !no_upload,
                params,
                row: row.map(|r| (data, r)),
                parallel,
                quality: quality.map(QualityArg::model),
                path_noise,
                report: false,
            };
            encode(&file, &outputs, path, codec, opts, strict, report.as_deref(), out)
        }
        Command::Simulate { file, output } => simulate(&file, output, out),
        Command::BakeVolume(args) => bake_volume(args, out),
        Command::Resolve { file, check, force, only, allow_cloud, no_store } => {
            let o = sr_resolve::Options { check, force, only, allow_cloud, store: no_store.then(PathBuf::new) };
            resolve(&file, &o, out)
        }
        Command::Scopes { file, step, chroma_limit } => scopes(&file, step, chroma_limit, out),
        Command::Gpus => gpus(out),
        Command::Schedule { file, rate } => schedule(&file, &rate, out),
        Command::Capabilities => {
            let manifest = sr_deliver::capabilities::MANIFEST;
            if out.json() {
                match serde_json::from_str::<Value>(manifest) {
                    Ok(v) => out.set_result(v),
                    Err(e) => return Ok(out.fail(ErrorKind::Internal, format!("the capabilities manifest: {e}"))),
                }
            }
            write!(out.w, "{manifest}").map(|()| 0)
        }
        Command::Explain { code } => explain(code.as_deref(), out),
        Command::Mcp => Ok(mcp::run_stdio()),
        Command::Completions { shell } => {
            if out.json() {
                let mut script = Vec::new();
                clap_complete::generate(shell, &mut Cli::command(), "scene-render", &mut script);
                let script = String::from_utf8_lossy(&script).into_owned();
                out.set_result(json!({ "shell": shell.to_string(), "script": script }));
            } else {
                clap_complete::generate(shell, &mut Cli::command(), "scene-render", &mut std::io::stdout());
            }
            Ok(0)
        }
    }
}

/// Whether a command line asks for JSON output, read before it could be parsed.
fn wants_json(args: &[String]) -> bool {
    args.iter().take_while(|a| *a != "--").enumerate().any(|(i, a)| {
        a == "--json" || a == "--format=json" || (a == "--format" && args.get(i + 1).is_some_and(|v| v == "json"))
    })
}

/// The subcommand a command line names, if any: the first argument that is one.
fn named_command(args: &[String]) -> Option<String> {
    let cli = Cli::command();
    args.iter().take_while(|a| *a != "--").find_map(|a| {
        cli.get_subcommands()
            .find(|c| c.get_name() == a || c.get_all_aliases().any(|x| x == a))
            .map(|c| c.get_name().to_string())
    })
}

/// A command line that cannot be parsed: clap's message, and with JSON output asked for, a usage envelope.
fn parse_failure(e: clap::Error, args: &[String]) -> u8 {
    if !e.use_stderr() || !wants_json(args) {
        e.exit()
    }
    let _ = e.print();
    let text = e.to_string();
    let message = text.lines().next().unwrap_or_default().trim_start_matches("error: ").trim().to_string();
    let mut out = Out::new(ColorChoice::Never, true);
    out.record(ErrorKind::Usage, message, Vec::new());
    out.emit(named_command(args).as_deref(), ErrorKind::Usage.exit())
}

fn main() -> ExitCode {
    install_panic_hook();
    let args: Vec<String> = std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => return ExitCode::from(parse_failure(e, &args)),
    };
    if cli.debug_gpu {
        std::env::set_var("SR_GPU_DEBUG", "1");
    }
    // on WSL2 the GPU is reachable only through Mesa's D3D12 driver; Mesa reads these
    // variables when a GL display opens, so set them before any thread starts
    sr_gpu::gpu::prepare_environment();
    if let Some(n) = cli.threads.filter(|&n| n > 0) {
        sr_vector::set_threads(n);
        if std::env::var_os("RAYON_NUM_THREADS").is_none() {
            std::env::set_var("RAYON_NUM_THREADS", n.to_string());
        }
    }
    // the server speaks JSON-RPC on standard output, so no envelope follows it
    let json = (cli.json || cli.format == OutputFormat::Json) && !matches!(cli.command, Command::Mcp);
    let mut out = Out::new(cli.color.choice(), json);
    let name = cli.command.name();
    let code = run(cli.command, &mut out);
    ExitCode::from(if json { out.emit(Some(name), code) } else { code })
}

#[cfg(test)]
mod exit_tests {
    use super::*;

    #[test]
    fn a_panic_exits_like_a_run_that_could_not_complete() {
        let mut out = Out::capture();
        let res = std::panic::catch_unwind(|| -> std::io::Result<u8> { panic!("cannot continue") });
        assert_eq!(settle(res, &mut out), COULD_NOT_RUN);
        assert_eq!(COULD_NOT_RUN, 2, "the documented exit status");
        let (doc, exit) = out.finish(Some("render"), COULD_NOT_RUN);
        assert_eq!(exit, 2);
        assert_eq!(doc["error"]["kind"], "internal");
        assert_eq!(doc["error"]["message"], "internal error: cannot continue");
    }

    #[test]
    fn outcomes_keep_their_exit_status() {
        let mut out = Out::capture();
        assert_eq!(settle(Ok(Ok(1)), &mut out), 1);
        let pipe = std::io::Error::from(std::io::ErrorKind::BrokenPipe);
        assert_eq!(settle(Ok(Err(pipe)), &mut out), 0);
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert_eq!(settle(Ok(Err(missing)), &mut out), COULD_NOT_RUN);
        let (doc, _) = out.finish(Some("inspect"), COULD_NOT_RUN);
        assert_eq!(doc["error"]["kind"], "io");
    }

    #[test]
    fn json_is_asked_for_by_either_spelling_but_not_after_the_end_of_options() {
        let args = |s: &str| s.split_whitespace().map(String::from).collect::<Vec<_>>();
        assert!(wants_json(&args("inspect --json a.xml")));
        assert!(wants_json(&args("--format json inspect a.xml")));
        assert!(wants_json(&args("inspect --format=json a.xml")));
        assert!(!wants_json(&args("inspect --format text a.xml")));
        assert!(!wants_json(&args("validate -- --json")));
        assert_eq!(named_command(&args("--color never validate --bogus")).as_deref(), Some("validate"));
        assert_eq!(named_command(&args("--format json --bogus")), None);
    }

    #[test]
    fn internal_errors_name_the_message_and_its_location() {
        let payload = std::panic::catch_unwind(|| panic!("index {} out of range", 3)).unwrap_err();
        let at = std::panic::Location::caller();
        let line = internal_error(panic_message(&*payload), Some(at));
        assert_eq!(line, format!("internal error: index 3 out of range at {}:{}", at.file(), at.line()));
        let payload = std::panic::catch_unwind(|| panic!("fixed")).unwrap_err();
        assert_eq!(internal_error(panic_message(&*payload), None), "internal error: fixed");
        let payload = std::panic::catch_unwind(|| std::panic::panic_any(7_u8)).unwrap_err();
        assert_eq!(panic_message(&*payload), "no message");
    }
}

#[cfg(test)]
mod watch_tests {
    #[test]
    fn watches_bound_asset_files() {
        let dir = std::env::temp_dir().join(format!("sr-watch-bound-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("main.xml");
        std::fs::write(&file, r#"<scene version="1.2"><project width="16" height="16" fps="1" duration="1"/>
          <parameters><param id="picture" type="string" default="actual.png"/><bind param="picture" target="img" property="src"/></parameters>
          <assets><image id="img" src="base.png" width="16" height="16"/></assets><composition><layer id="l" asset="img"/></composition></scene>"#).unwrap();
        let watched = super::watched_stamps(&file).unwrap();
        assert!(watched.iter().any(|(f, _)| f == &dir.join("actual.png")), "bound file missing: {watched:?}");
    }

    /// The polls `settled_change` makes over `seen`, the stamps of successive polls.
    fn polls(before: &[u8], seen: &[u8]) -> usize {
        let mut next = seen.iter();
        let mut n = 0;
        super::settled_change(
            before,
            || {
                n += 1;
                vec![*next.next().expect("returned after the stamps settled")]
            },
            || {},
        );
        n
    }

    #[test]
    fn a_change_is_rendered_once_its_files_settle() {
        // unchanged, unchanged, truncated, written, written: the truncated file is never the one rendered
        assert_eq!(polls(&[1], &[1, 1, 2, 3, 3, 9]), 5);
        // a change that is complete when first seen waits one more poll
        assert_eq!(polls(&[1], &[3, 3, 9]), 2);
        // a file written back to its old stamp is still a change once seen different
        assert_eq!(polls(&[1], &[2, 1, 1, 9]), 3);
    }
}
