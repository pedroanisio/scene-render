//! One `<output>` from document to delivered files.
//!
//! Stages overlap: while the encoder thread writes frame N, the GPU renders
//! and converts frame N+1 and the readback of frame N maps; a helper thread
//! evaluates frame N+2 when the document has no simulation; decoders run in
//! their own FFmpeg processes ahead of the render position. Audio mixes
//! first, because loudness normalisation needs the whole programme and the
//! evaluator reads the analysis table built from it; when nothing the picture
//! draws reads the mix, the audio chain runs on a thread of its own beside the
//! render and is joined where the mux needs its files.

use std::path::{Path, PathBuf};
use std::sync::mpsc::sync_channel;
use std::time::Instant;

use sr_eval::{EvalOptions, Evaluator};
use sr_gpu::output::{OutputColor, OutputStage};
use sr_gpu::{Gpu, Renderer};
use sr_media::encode::{Codec, ColorTags, Container, EncodeSpec, Encoder, Hardware, Hdr, InputFormat, StillFormat};
use sr_model::model as m;

use crate::access::{Judge, Observe, Seen, Streaming, OBSERVATION_FRAMES};
use crate::audio::{self, SceneAudio};
use crate::DeliverError;

/// Delivery options that are not part of the document.
#[derive(Debug, Clone)]
pub struct Options {
    /// Directory relative output paths resolve against (default: the document's).
    pub out_dir: Option<PathBuf>,
    /// Preferred asset representation, over the output's own.
    pub representation: Option<String>,
    /// Hardware encoder preference.
    pub hardware: Hardware,
    /// Override the output's start time.
    pub start: Option<f64>,
    /// Override the output's end time.
    pub end: Option<f64>,
    /// Upload to destinations after rendering.
    pub upload: bool,
    /// Parameters, as on the command line.
    pub params: Vec<(String, String)>,
    /// Data row.
    pub row: Option<(Option<String>, usize)>,
    /// Segments rendered at once for a single-pass video output.
    pub parallel: Parallel,
    /// Quality tier overriding the document's `project@quality`.
    pub quality: Option<m::ProjectQuality>,
    /// The noise target of adaptive sampling for path-traced cameras (`sr_gpu::Renderer::path_noise`).
    pub path_noise: Option<f32>,
    /// Measure what only a render report reads (SREP 18): text that does not fit its box. The checks the document
    /// asks for (accessibility, safe area) run either way.
    pub report: bool,
}

/// How many time segments of a video output render and encode at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Parallel {
    /// The output is split by its duration alone (one chunk per 10 s, at most 12; one stream under 20 s), so the
    /// file is the same on any machine; 3 chunks render at once with a hardware encoder, otherwise half the cores
    /// (at most 4), one on a software adapter, and at most one per 20 s.
    #[default]
    Auto,
    /// Exactly this many (1 renders serially).
    Count(u32),
}

impl Parallel {
    /// `auto` or a positive count.
    pub fn parse(s: &str) -> Option<Parallel> {
        match s {
            "auto" => Some(Parallel::Auto),
            n => n.parse().ok().filter(|&n| n > 0).map(Parallel::Count),
        }
    }
}

impl Default for Options {
    fn default() -> Options {
        Options {
            out_dir: None,
            representation: None,
            hardware: Hardware::Auto,
            start: None,
            end: None,
            upload: true,
            params: Vec::new(),
            row: None,
            parallel: Parallel::default(),
            quality: None,
            path_noise: None,
            report: false,
        }
    }
}

/// Device chosen for scene rendering, independently of the video encoder.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RenderAdapter {
    pub name: String,
    pub backend: String,
    pub device_type: String,
    /// True for a software rasteriser such as llvmpipe.
    pub software: bool,
}

/// What a delivery produced and how long each stage took.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Report {
    /// Main output path (pattern for sequences).
    pub path: PathBuf,
    /// Every file written.
    pub files: Vec<PathBuf>,
    /// Video frames encoded.
    pub frames: u64,
    /// Output size.
    pub size: [u32; 2],
    /// Output frame rate.
    pub fps: f64,
    /// Encoder in use.
    pub encoder: String,
    /// Rendering device, independently selected from the video encoder. Absent for audio-only output.
    pub render_adapter: Option<RenderAdapter>,
    /// Quality tier the frames were rendered at (`draft`, `preview` or `final`).
    pub quality: String,
    /// Wall-clock seconds for the whole output.
    pub seconds: f64,
    /// Frames per second achieved for the video pass.
    pub render_fps: f64,
    /// Seconds spent evaluating (waiting for the next graph, when a helper thread evaluates
    /// ahead), rendering (CPU planning and submission), waiting for converted frames, and
    /// blocked on the encoder. Waiting for converted frames is residual blocking after
    /// overlapping work, not total GPU execution time; render may include internal waits.
    pub stage_seconds: [f64; 4],
    /// Seconds of the render stage spent waiting for video decoders.
    pub decode_wait_seconds: f64,
    /// Seconds of the render stage spent building and tile-encoding vector geometry.
    pub vector_seconds: f64,
    /// Seconds spent mixing audio.
    pub audio_seconds: f64,
    /// Whether the audio chain (mix, loudness, ceiling) ran on a thread of its own while the picture rendered, because
    /// nothing the picture draws reads the mix; otherwise the mix came first.
    pub audio_beside_picture: bool,
    /// ISO 639-2 languages of delivered audio streams, in stream order.
    pub audio_languages: Vec<String>,
    /// Caption identities, including labels and kind, in delivery order.
    pub caption_tracks: Vec<crate::metadata::CaptionInfo>,
    /// Integrated loudness of the delivered mix, LUFS.
    pub loudness: Option<f64>,
    /// True peak of the delivered mix, dBTP.
    pub true_peak: Option<f64>,
    /// Loudness range of the delivered mix (EBU Tech 3342), LU; absent when no 3-second window reaches −70 LUFS.
    pub loudness_range: Option<f64>,
    /// Loudest 400 ms window of the delivered mix, LUFS; absent for a programme shorter than one window or silent.
    pub momentary_max: Option<f64>,
    /// Loudest 3-second window of the delivered mix, LUFS; absent for a programme shorter than one window or silent.
    pub short_term_max: Option<f64>,
    /// The decoded-AAC check of the master's true-peak ceiling: what it measured and any gain it applied.
    pub audio_ceiling: Option<crate::ceiling::Held>,
    /// Content not rendered by this batch, per node.
    pub unsupported: Vec<String>,
    /// Stateful effects (SREPs 67 and 68): checkpoints loaded from disk for a seek and seeks that found none, and the
    /// steps each looped effect inside an `iterate` took (effect and node), the fewest and the most over the frames.
    pub checkpoint_hits: usize,
    pub checkpoint_misses: usize,
    pub iterate_steps: std::collections::BTreeMap<String, [u64; 2]>,
    /// Each `object3D` drawn as a mesh: the first and last times it was drawn, and whether it was inside the camera's
    /// view at any of them (the render report's 3D-OFF-CAMERA).
    #[serde(skip)]
    pub objects3d_seen: std::collections::BTreeMap<String, ([f64; 2], bool)>,
    /// Each `object3D` drawn with a metallic material in a 3D pass with no environment image: its highest metallic and
    /// the first and last times (the render report's 3D-METAL-NO-ENVIRONMENT).
    #[serde(skip)]
    pub metal_without_environment: std::collections::BTreeMap<String, (f32, [f64; 2])>,
    /// Accessibility findings (flash analysis, text contrast, required captions).
    pub accessibility: Vec<String>,
    /// The finding that fails delivery (a check set to `error`).
    #[serde(skip)]
    pub accessibility_error: Option<String>,
    /// Delivery warnings: content that renders but is likely a mistake (a segment starting on an
    /// empty frame, a caption shown too briefly to read).
    pub warnings: Vec<String>,
    /// Warnings raised by composition or overlay templating; strict delivery rejects these.
    pub evaluation_warnings: Vec<sr_model::Diagnostic>,
    /// Remote locations the files were delivered to.
    pub uploads: Vec<String>,
    /// Encoded passes (2 for two-pass, more when fitting a file size).
    pub passes: u32,
    /// Time segments rendered and encoded at once (1 for a serial render).
    pub segments: u32,
    /// Why the render ran with one worker although more were asked for or chosen (GPU debug layers on).
    pub serial_because: Option<String>,
    /// The output time range rendered, seconds (after command-line overrides).
    pub range: [f64; 2],
    /// What the delivery found, for the render report (SREP 18): accessibility, safe area, caption and segment
    /// warnings and, with [`Options::report`], text fit. Validation and evaluation diagnostics are in the
    /// document and in `evaluation_warnings`.
    pub findings: Vec<crate::render_report::Finding>,
}

/// An `<output>` element built from command-line settings.
pub fn adhoc_output(path: &str, codec: &str) -> Result<m::Output, DeliverError> {
    // a command-line path is a file path, not a URI: the rules judge its file name (extension, frame
    // pattern), and a `%` in a directory is no escape
    let name = Path::new(path).file_name().map_or(path.into(), |n| n.to_string_lossy());
    let xml = format!(r#"<output path="{}" codec="{}"/>"#, xml_escape(&name), xml_escape(codec));
    let doc = format!(
        r##"<scene version="1.1"><project width="16" height="16" fps="1" duration="1"/>{xml}<composition/></scene>"##
    );
    let d = sr_model::load_str(&doc, &sr_model::LoadOptions::without_assets()).map_err(|e| match e {
        sr_model::LoadError::Invalid(r) => DeliverError::Document(r),
        e => DeliverError::Invalid(e.to_string()),
    })?;
    Ok(m::Output { path: path.to_string(), ..d.scene.outputs[0].clone() })
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;")
}

fn resolve(dir: &Path, p: &str) -> PathBuf {
    let p = p.strip_prefix("file://").unwrap_or(p);
    let pb = PathBuf::from(p);
    if pb.is_absolute() {
        pb
    } else {
        dir.join(pb)
    }
}

/// The raw frame format the GPU writes for an output.
fn input_format(o: &m::Output, codec: Codec) -> InputFormat {
    let deep = o.pixel_format.contains("10") || o.pixel_format.contains("12") || o.pixel_format.contains("16");
    match codec {
        Codec::ExrSequence => InputFormat::Gbrapf32,
        Codec::TiffSequence | Codec::Prores | Codec::Dnxhr => InputFormat::Rgba16,
        Codec::PngSequence | Codec::Apng => {
            if deep {
                InputFormat::Rgba16
            } else {
                InputFormat::Rgba8
            }
        }
        Codec::Gif | Codec::Webp | Codec::JpegSequence => InputFormat::Rgba8,
        _ if o.alpha => InputFormat::Rgba16,
        _ => match o.pixel_format.as_str() {
            "yuv420p" | "nv12" => InputFormat::Nv12,
            "yuv420p10le" | "p010le" | "p010" => InputFormat::P010,
            _ => InputFormat::Rgba16,
        },
    }
}

fn metadata(scene: &m::Scene) -> Vec<(String, String)> {
    // Case-insensitive collisions follow the muxers' tag semantics. Explicit attributes win;
    // repeated custom keys use the last authored value, in a stable sorted order.
    let mut tags = std::collections::BTreeMap::new();
    if let Some(md) = &scene.metadata {
        for c in &md.children {
            if let m::MetadataChild::Meta(meta) = c {
                tags.insert(meta.name.to_ascii_lowercase(), meta.value.clone());
            }
        }
        for (key, value) in [
            ("title", &md.title),
            ("artist", &md.author),
            ("comment", &md.description),
            ("copyright", &md.copyright),
            ("keywords", &md.keywords),
            ("version", &md.revision),
            ("creation_time", &md.created),
            ("date", &md.modified),
            ("generator", &md.generator),
        ] {
            if let Some(value) = value {
                tags.insert(key.into(), value.clone());
            }
        }
        if let Some(language) = &md.language {
            tags.insert("language".into(), language.to_string());
        }
        for c in &md.children {
            if let m::MetadataChild::Accessibility(a) = c {
                if let Some(value) = &a.description {
                    tags.insert("synopsis".into(), value.clone());
                }
            }
        }
    }
    // FFmpeg owns `encoder` and overwrites it on some muxers. Preserve the authoring tool in
    // `generator`, independently of this engine and FFmpeg's actual encoder identification.
    tags.insert("encoder".into(), format!("scene-render {}", env!("CARGO_PKG_VERSION")));
    tags.into_iter().collect()
}

struct Video<'a> {
    ev: &'a Evaluator,
    renderer: Renderer,
    stage: OutputStage,
    color: OutputColor,
    format: InputFormat,
    size: [u32; 2],
    keep_alpha: bool,
    /// Output frame rate.
    fps: f64,
    /// How frames are placed in the output (a layout's reframing).
    reframe: Option<sr_eval::program::Reframe>,
    /// The output's segments, and the frame time of each entry of `times`.
    segments: Option<(&'a crate::segments::TimeMap, Vec<crate::segments::FrameTime>)>,
    /// The symbol drawn over every frame in output time.
    overlay: Option<crate::overlay::Overlay>,
    /// Without segments, the composition time of output time 0 (`output/@start`).
    origin: f64,
    /// Output-sized working textures: the two placed sides of a join, and their combination.
    join_tex: Option<[std::sync::Arc<sr_gpu::resources::Tex>; 3]>,
    contrasts: ContrastPrefetch,
}

impl Video<'_> {
    /// The placement in the output of a frame of segment time `f` (the output's own frame without segments).
    fn placement(&self, f: Option<&crate::segments::FrameTime>) -> sr_gpu::output::Placement {
        use sr_gpu::output::Placement;
        let Some(r) = self.reframe else { return Placement::default() };
        let focus = match (&self.segments, f) {
            (Some((tm, _)), Some(f)) => tm.focus(f, r.focus),
            _ => r.focus,
        };
        match r.mode {
            m::Reframe::Crop => Placement::Crop { focus },
            m::Reframe::FitBlur => Placement::FitBlur { focus },
            // a plain fit shares the free space by the focus too (0.5, 0.5 centres it)
            _ => Placement::Fit { focus },
        }
    }
}

impl Video<'_> {
    /// Renders the graph `g` of composition time `t`, the frame of segment time `f` when the output
    /// has segments. A sub-frame sample dt seconds from the frame (motion blur, video lookups) is
    /// taken dt output seconds away and mapped through the frame's segment, so blur covers the
    /// composition time the shutter spans at that speed.
    fn render_side(
        &mut self,
        g: &sr_eval::FrameGraph,
        t: f64,
        f: Option<&crate::segments::FrameTime>,
    ) -> Result<sr_gpu::Frame, DeliverError> {
        let (ev, p, fps) = (self.ev, self.ev.program(), self.fps);
        let seg = self.segments.as_ref().zip(f).map(|((tm, _), f)| (*tm, *f));
        let mut sub = |st: f64| match seg {
            Some((tm, f)) if (st - t).abs() < 1.0 / fps => ev.evaluate(tm.sample(&f, st - t)),
            _ => ev.evaluate(st),
        };
        let frame = match self.contrasts.planned.remove(&t.to_bits()) {
            Some(targets) if seg.is_none() => {
                let (frame, ratios) = self.renderer.render_with_deferred_contrasts(g, p, &targets, &mut sub);
                self.contrasts.measured.extend(ratios.into_iter().map(|(id, ratio)| ((t.to_bits(), id), ratio)));
                frame
            }
            _ => self.renderer.render_with(g, p, Some(&mut sub)),
        };
        if !frame.stats.errors.is_empty() {
            return Err(DeliverError::Render { time: t, message: crate::overlay::failure_message(&frame.stats) });
        }
        Ok(frame)
    }

    /// The picture of a frame and how to place it: the render `frame` of segment time `f`, or inside a
    /// join both sides, each through its own map and placed with its own focus, combined in the
    /// output's frame.
    fn picture(
        &mut self,
        frame: &sr_gpu::Frame,
        f: Option<&crate::segments::FrameTime>,
        unsupported: &mut std::collections::BTreeSet<String>,
    ) -> Result<(std::sync::Arc<sr_gpu::resources::Tex>, sr_gpu::output::Placement), DeliverError> {
        let tm = self.segments.as_ref().map(|(tm, _)| *tm);
        let Some(j) = tm.zip(f).and_then(|(tm, f)| tm.join_at(f.output)) else {
            return Ok((frame.texture.clone(), self.placement(f)));
        };
        let (ev, working) = (self.ev, self.renderer.working());
        let tex = self.join_tex.get_or_insert_with(|| [(); 3].map(|_| self.renderer.texture(self.size))).clone();
        let own = if f.is_some_and(|f| f.segment == j.from.segment) { 0 } else { 1 };
        let sides = [j.from, j.to];
        self.stage.place(&frame.texture, &working, &tex[own], self.placement(Some(&sides[own])));
        let other = sides[1 - own];
        let go = ev.evaluate(other.composition);
        let f2 = self.render_side(&go, other.composition, Some(&other))?;
        unsupported.extend(f2.stats.unsupported.iter().cloned());
        self.stage.place(&f2.texture, &working, &tex[1 - own], self.placement(Some(&other)));
        let problems = self.renderer.join(
            ev.program(),
            &j.join.elem,
            &tex[0],
            &tex[1],
            j.progress,
            j.velocity,
            j.from.output,
            &tex[2],
        );
        unsupported.extend(problems);
        Ok((tex[2].clone(), sr_gpu::output::Placement::default()))
    }
}

impl Video<'_> {
    /// Predict likely deferred checks without advancing simulation. Rendered observations
    /// remain authoritative; predictions can only avoid a later original-frame render.
    fn prepare_contrasts(&mut self, times: &[f64]) {
        self.contrasts = ContrastPrefetch::default();
        if self.segments.is_some() || !Checks::of(self.ev.program()).contrast_on() {
            return;
        }
        let mut predicted = std::collections::BTreeMap::<_, Vec<_>>::new();
        for (k, &t) in times.iter().enumerate() {
            let g = self.ev.evaluate_layout(t);
            for (id, opacity) in self.renderer.contrast_candidates(&g, self.ev.program()) {
                predicted.entry(id).or_default().push((k, opacity));
            }
        }
        self.contrasts.prepare(times, predicted);
    }

    /// Renders `times` and hands each converted frame to `sink` and what the accessibility checks
    /// take from it to `seen`; adds stage timings to `report`. The checks' verdicts come from
    /// [`Video::judge`], once every frame of the output has been observed.
    fn run(
        &mut self,
        times: &[f64],
        report: &mut Report,
        seen: &mut dyn Observe,
        mut sink: impl FnMut(sr_gpu::output::FrameBytes) -> Result<f64, DeliverError>,
    ) -> Result<(), DeliverError> {
        let p = self.ev.program();
        let working = self.renderer.working();
        let mut pending: Option<(f64, sr_gpu::output::Pending)> = None;
        // A worker reuses its report for several chunks; retain earlier findings.
        let mut unsupported = report.unsupported.iter().cloned().collect::<std::collections::BTreeSet<_>>();
        let checks = Checks::of(p);
        let flash_picture = checks.flash_on().then(|| self.renderer.texture(self.size));
        self.renderer.contrast_probe = checks.contrast_on();
        let opacity_of = |g: &sr_eval::FrameGraph, id: &sr_gpu::ContrastTarget| match id {
            sr_gpu::ContrastTarget::Node(id) => {
                g.nodes.iter().find(|n| &*n.id == id).map(|n| n.world_opacity).unwrap_or(1.0)
            }
            sr_gpu::ContrastTarget::Captions => 1.0,
        };
        let ev = self.ev;
        // A helper thread evaluates the next frame while this one renders: the channel holds one graph and the
        // helper works on the one after it, two frames ahead at most. A simulated document gives each frame the
        // same graph whether or not the frame after it, or a sub-frame sample around it, was evaluated first
        // (sr-eval's tests/sim_order.rs), so simulated documents evaluate ahead as well. The helper also extracts the
        // frame's clay surfaces, so that work overlaps the render of the frame before.
        let clay = std::sync::Mutex::new(sr_eval::solid::ClayCache::default());
        evaluated(
            times,
            |t| {
                let mut g = ev.evaluate(t);
                sr_eval::solid::attach_clay(&mut g, &mut clay.lock().unwrap_or_else(|e| e.into_inner()));
                g
            },
            |next| {
                for (k, &t) in times.iter().enumerate() {
                    let t0 = Instant::now();
                    let g = next(t)?;
                    let t1 = Instant::now();
                    let ft = self.segments.as_ref().map(|(_, frames)| frames[k]);
                    let out_t = ft.map_or(t - self.origin, |f| f.output);
                    let frame = self.render_side(&g, t, ft.as_ref())?;
                    unsupported.extend(frame.stats.unsupported.iter().cloned());
                    for id in &frame.stats.contrast_unprobed {
                        seen.unprobed(id, k, opacity_of(&g, id));
                    }
                    for (id, ratio) in &frame.stats.contrast {
                        seen.contrast(id, opacity_of(&g, id), *ratio, t);
                    }
                    report.decode_wait_seconds += frame.stats.decode_wait;
                    report.vector_seconds += frame.stats.vector_seconds;
                    stateful(report, &frame.stats);
                    seen_in_3d(report, &frame.stats, t);
                    let (picture, placement) = self.picture(&frame, ft.as_ref(), &mut unsupported)?;
                    if checks.contrast_on() {
                        if let Some(o) = self.overlay.as_mut() {
                            for (id, opacity, ratio) in o.contrast(&mut self.stage, out_t, &picture, placement)? {
                                let id = sr_gpu::ContrastTarget::Node(format!("output layer: {id}"));
                                seen.contrast(&id, opacity, ratio, out_t);
                            }
                        }
                    }
                    let over = match self.overlay.as_mut() {
                        Some(o) => {
                            let (tex, problems) = o.draw(&mut self.stage, out_t)?;
                            unsupported.extend(problems);
                            Some(tex)
                        }
                        None => None,
                    };
                    if let Some(tex) = flash_picture.as_ref() {
                        self.stage.place_with_overlay(&picture, &working, tex, placement, over);
                        seen.flash(out_t, self.renderer.flash_grid(tex));
                    }
                    seen.finish_frame().map_err(DeliverError::Invalid)?;
                    let next = self.stage.submit_placed(
                        &picture,
                        &working,
                        &self.color,
                        self.format,
                        self.size,
                        self.keep_alpha,
                        placement,
                        over,
                    );
                    let t2 = Instant::now();
                    meter::add(&mut report.stage_seconds, 0, (t1 - t0).as_secs_f64());
                    meter::add(&mut report.stage_seconds, 1, (t2 - t1).as_secs_f64());
                    if let Some((_, prev)) = pending.replace((t, next)) {
                        let w = Instant::now();
                        let bytes = self.stage.wait(prev);
                        meter::add(&mut report.stage_seconds, 2, w.elapsed().as_secs_f64());
                        meter::add(&mut report.stage_seconds, 3, sink(bytes)?);
                    }
                    if k + 1 == times.len() {
                        if let Some((_, last)) = pending.take() {
                            let w = Instant::now();
                            let bytes = self.stage.wait(last);
                            meter::add(&mut report.stage_seconds, 2, w.elapsed().as_secs_f64());
                            meter::add(&mut report.stage_seconds, 3, sink(bytes)?);
                        }
                    }
                    // the output stage and the overlay draw on the same device after the frame's own check
                    self.renderer.gpu().take_error()?;
                }
                Ok(())
            },
        )?;
        report.unsupported = unsupported.into_iter().collect();
        Ok(())
    }

    /// Completes the accessibility checks over an output and records their findings in `report`.
    /// `judge` has observed every frame of `times`, in order; text the inline probe could not reach is
    /// measured here, then the verdicts are given.
    fn judge(&mut self, mut judge: Judge, times: &[f64], report: &mut Report) -> Result<(), DeliverError> {
        three_d_findings(report);
        let p = self.ev.program();
        let checks = Checks::of(p);
        // Probe over the whole span of maximum visibility. A middle-frame sample can miss low
        // contrast at either end, or a change in the text's backdrop.
        // Keep time moving forward and share each original frame among the texts
        // measured there, instead of replaying the timeline separately for every label.
        let thinned = thinned_contrasts(&judge.unprobed);
        for (k, targets) in contrast_schedule(std::mem::take(&mut judge.unprobed)) {
            let t = times[k];
            let ev = self.ev;
            let renderer = &mut self.renderer;
            let ids: Vec<_> = targets.iter().map(|(id, _)| id.clone()).collect();
            let ratios = self.contrasts.resolve(t, &ids, |missing| {
                let g = ev.evaluate(t);
                let mut sub = |st: f64| ev.evaluate(st);
                renderer.contrasts_with_without(&g, p, missing, &mut sub)
            })?;
            for ((id, opacity), ratio) in targets.into_iter().zip(ratios) {
                if let Some(ratio) = ratio {
                    judge.contrast(&id, opacity, ratio, t);
                }
            }
        }
        self.contrasts = ContrastPrefetch::default();
        // accessibility verdicts, also as render-report findings at the severity the check is set to
        use crate::render_report::{code, Finding};
        let fail = |mode: &str, msg: String, finding: Finding, report: &mut Report| {
            if mode == "error" && report.accessibility_error.is_none() {
                report.accessibility_error = Some(msg.clone());
            }
            let severity = if mode == "error" { sr_model::Severity::Error } else { sr_model::Severity::Warning };
            report.findings.push(Finding { severity, ..finding });
            report.accessibility.push(msg);
        };
        if let Some((msg, first)) = judge.flash.as_ref().and_then(|d| Some((d.verdict()?, d.first.unwrap_or(0.0)))) {
            let finding =
                Finding::scene(code::ACC_FLASH, sr_model::Severity::Warning, msg.clone()).at_time(first, first);
            fail(&checks.flash, msg, finding, report);
        }
        if let Some((id, got, of)) = thinned.first() {
            let msg = format!(
                "contrastCheck: {} texts share the check's budget of {CONTRAST_BUDGET} probes and are measured at fewer of \
                 their frames (e.g. {id} at {got} of {of}); a backdrop that changes between two probes is not seen",
                thinned.len()
            );
            report.findings.push(Finding::scene(code::engine("CONTRAST-SAMPLED"), sr_model::Severity::Info, msg));
        }
        let min_contrast = checks.min_contrast;
        for (id, (_, ratio, at)) in &judge.lowest {
            if *ratio < min_contrast {
                let msg = format!("contrastCheck: {id} reaches only {ratio:.2}:1 against its background at {at:.3} s (minimum {min_contrast}:1)");
                let finding = match id {
                    sr_gpu::ContrastTarget::Node(n) => {
                        Finding::node(code::LEG_CONTRAST, sr_model::Severity::Warning, n, msg.clone())
                    }
                    sr_gpu::ContrastTarget::Captions => {
                        Finding::scene(code::LEG_CONTRAST, sr_model::Severity::Warning, msg.clone())
                    }
                }
                .at_time(*at, *at)
                .measuring(*ratio, Some(min_contrast), "ratio");
                fail(&checks.contrast, msg, finding, report);
            }
        }
        Ok(())
    }
}

/// The accessibility checks a document asks for on its rendered frames.
struct Checks {
    /// `flashCheck`: `off`, `warn` or `error`.
    flash: String,
    /// `contrastCheck`: `off`, `warn` or `error`.
    contrast: String,
    /// `minContrast`.
    min_contrast: f64,
}

impl Checks {
    fn of(p: &sr_eval::Program) -> Checks {
        let acc = p.scene.metadata.as_ref().and_then(|m| {
            m.children.iter().find_map(|c| match c {
                m::MetadataChild::Accessibility(a) => Some(a),
                _ => None,
            })
        });
        // Without an <accessibility> element nothing is checked: the XSD defaults (flashCheck
        // "warn", contrastCheck "off") apply to the element's attributes, and the checks
        // are skipped when it is absent. Declare <accessibility/> to opt in.
        Checks {
            flash: acc.map(|a| a.flash_check.to_string()).unwrap_or_else(|| "off".into()),
            contrast: acc.map(|a| a.contrast_check.to_string()).unwrap_or_else(|| "off".into()),
            min_contrast: acc.map(|a| a.min_contrast.get()).unwrap_or(4.5),
        }
    }

    fn flash_on(&self) -> bool {
        self.flash != "off"
    }

    fn contrast_on(&self) -> bool {
        self.contrast != "off"
    }
}

/// Runs `body` with a source of the graphs of `times`, which `body` asks for in order: a helper thread evaluates
/// them through `eval` one frame ahead of `body`. A panic of the helper comes back as an error that carries its
/// message, rather than unwinding the delivery.
fn evaluated<G: Send, R>(
    times: &[f64],
    eval: impl Fn(f64) -> G + Sync,
    body: impl FnOnce(&mut dyn FnMut(f64) -> Result<G, DeliverError>) -> Result<R, DeliverError>,
) -> Result<R, DeliverError> {
    let eval = &eval;
    std::thread::scope(|s| {
        let (tx, rx) = sync_channel::<G>(1);
        let helper = std::thread::Builder::new().name("sr-eval".into()).spawn_scoped(s, move || {
            for &t in times {
                // the render loop stopped early: nobody wants the rest
                if tx.send(eval(t)).is_err() {
                    break;
                }
            }
        })?;
        let result = body(&mut |_| rx.recv().map_err(|_| DeliverError::Invalid("evaluator thread stopped".into())));
        // unblock the helper when the body stopped early, then join it here so its panic is not re-raised
        drop(rx);
        match helper.join() {
            Ok(()) => result,
            Err(payload) => Err(DeliverError::Invalid(format!("evaluation panicked: {}", panic_message(&*payload)))),
        }
    })
}

/// The message a panic was raised with, when it is a string.
pub(crate) fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("no message")
}

/// Writes frames through an encoder on its own thread; returns blocked seconds per frame.
struct Feeder {
    tx: Option<std::sync::mpsc::SyncSender<sr_gpu::output::FrameBytes>>,
    handle: Option<std::thread::JoinHandle<Result<String, sr_media::MediaError>>>,
    /// Set once every frame was sent: an encode whose frames stop short is abandoned, not finished.
    complete: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// Bytes of frames the queue in front of an encoder may hold.
const FRAME_QUEUE_BYTES: usize = 96_000_000;
/// The fewest and the most frames that queue holds, whatever their size.
const FRAME_QUEUE_MIN: usize = 3;
const FRAME_QUEUE_MAX: usize = 48;

/// How many frames of `frame_bytes` wait in front of an encoder. A hardware encoder opens its session on the first
/// frame (FFmpeg takes about twenty frames and then stops reading for the 0.2 s that takes): the queue lets the render
/// go on meanwhile, in a bounded amount of memory, which the encoders of `workers` parallel workers share.
fn frame_queue(frame_bytes: usize, workers: usize) -> usize {
    (FRAME_QUEUE_BYTES / workers.max(1) / frame_bytes.max(1)).clamp(FRAME_QUEUE_MIN, FRAME_QUEUE_MAX)
}

impl Feeder {
    /// Starts the encoder of `spec` for one of `workers` parallel workers.
    fn start(spec: &EncodeSpec, workers: usize) -> Result<Feeder, DeliverError> {
        use std::sync::atomic::Ordering::Relaxed;
        let mut enc = Encoder::start(spec)?;
        let (tx, rx) = sync_channel::<sr_gpu::output::FrameBytes>(frame_queue(
            spec.input.frame_bytes(spec.width, spec.height),
            workers,
        ));
        let complete = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let all_sent = complete.clone();
        let handle = std::thread::Builder::new().name("sr-encode".into()).spawn(move || {
            for f in rx {
                enc.write(&f)?;
            }
            if !all_sent.load(Relaxed) {
                // the render failed: dropping the encoder stops FFmpeg and removes the partial file
                return Err(sr_media::MediaError::Invalid("the encode was abandoned".into()));
            }
            let name = enc.encoder.clone();
            enc.finish()?;
            Ok(name)
        })?;
        Ok(Feeder { tx: Some(tx), handle: Some(handle), complete })
    }

    /// Queues `frame` for the encoder thread, which drops it, returning its buffer to its stage, once written.
    fn send(&mut self, frame: sr_gpu::output::FrameBytes) -> Result<f64, DeliverError> {
        let t = Instant::now();
        let Some(tx) = self.tx.as_ref() else {
            return Err(DeliverError::Invalid("a frame was sent after the encode was finished".into()));
        };
        if tx.send(frame).is_err() {
            // the encoder thread stopped: surface its error
            return Err(self.finish().err().unwrap_or_else(|| DeliverError::Invalid("encoder stopped".into())));
        }
        Ok(t.elapsed().as_secs_f64())
    }

    fn finish(&mut self) -> Result<String, DeliverError> {
        self.complete.store(true, std::sync::atomic::Ordering::Relaxed);
        drop(self.tx.take());
        match self.handle.take() {
            Some(h) => Ok(h
                .join()
                .map_err(|p| DeliverError::Invalid(format!("encoder thread panicked: {}", panic_message(&*p))))??),
            None => Ok(String::new()),
        }
    }
}

impl Drop for Feeder {
    fn drop(&mut self) {
        // not finished: wait for the encoder to be stopped, so nothing of it outlives the delivery
        drop(self.tx.take());
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Finishes encoders, one at a time, on a thread of its own: a chunk's last frames drain and its file closes while the
/// next chunk renders. At most one drain runs, so a hardware encoder never holds more sessions than the one finishing
/// and the one starting.
#[derive(Default)]
struct Closing(std::collections::VecDeque<std::thread::JoinHandle<Result<String, DeliverError>>>);

impl Closing {
    /// The encoder name of the oldest drain, once it is done.
    fn wait_one(&mut self) -> Result<Option<String>, DeliverError> {
        match self.0.pop_front() {
            Some(h) => h
                .join()
                .map_err(|p| DeliverError::Invalid(format!("encoder thread panicked: {}", panic_message(&*p))))?
                .map(Some),
            None => Ok(None),
        }
    }

    /// Waits for every drain; the name of the last one's encoder.
    fn wait(&mut self) -> Result<Option<String>, DeliverError> {
        let mut name = None;
        let mut first_error = None;
        while !self.0.is_empty() {
            match self.wait_one() {
                Ok(n) => name = n.or(name),
                Err(e) => first_error = first_error.or(Some(e)),
            }
        }
        first_error.map_or(Ok(name), Err)
    }

    /// Finishes `feeder` in the background. With [`DRAINS`] already running, waits for the oldest first and returns
    /// its encoder name.
    fn start(&mut self, mut feeder: Feeder) -> Result<Option<String>, DeliverError> {
        let previous = if self.0.len() >= DRAINS { self.wait_one()? } else { None };
        self.0.push_back(std::thread::Builder::new().name("sr-finish".into()).spawn(move || feeder.finish())?);
        Ok(previous)
    }
}

/// Encoders that drain at once behind the chunk that renders.
const DRAINS: usize = 1;

impl Drop for Closing {
    fn drop(&mut self) {
        // the delivery must not end with an encoder still running
        let _ = self.wait();
    }
}

/// The delivery's temporary directory, removed when the delivery ends, however it ends.
struct Scratch(PathBuf);

impl std::ops::Deref for Scratch {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The frame an output delivers, and where its size comes from (for messages): the output's width and height, else
/// scene360's (360 video renders at the scene360 size unless the output asks for another), else the layout's (a layout
/// that crops or fits delivers at its own frame size, one that reflows renders at it), else the project's.
fn output_frame(p: &sr_eval::Program, o: &m::Output) -> ([f64; 2], String) {
    let frame = p
        .scene
        .scene360
        .as_ref()
        .map(|s| [s.width as f64, s.height as f64])
        .or(p.reframe.map(|r| r.size))
        .unwrap_or(p.size);
    let size = [o.width.map(|w| w as f64).unwrap_or(frame[0]), o.height.map(|h| h as f64).unwrap_or(frame[1])];
    let source = if o.width.is_some() && o.height.is_some() {
        "the output".to_string()
    } else if p.scene.scene360.is_some() {
        "scene360".into()
    } else if let Some(l) = &o.layout {
        format!("layout \"{l}\"")
    } else {
        "the project".into()
    };
    (size, source)
}

/// The evaluation options an output is rendered with.
fn eval_options(output: &m::Output, opts: &Options) -> EvalOptions {
    EvalOptions {
        variant: output.variant.clone(),
        layout: output.layout.clone(),
        params: opts.params.clone(),
        row: opts.row.clone(),
        ..Default::default()
    }
}

/// Whether rendering `output` draws anything in 3D, so a caller that makes the GPU can ask for one
/// that can run the 3D pass (`deliver` makes its own for a caller that passes none).
pub fn output_uses_3d(doc: &sr_model::Document, output: &m::Output, opts: &Options) -> Result<bool, DeliverError> {
    let ev = Evaluator::new(doc, &eval_options(output, opts)).map_err(DeliverError::Document)?;
    Ok(ev.program().uses_3d())
}

/// Renders one output of `doc` and delivers it.
pub fn deliver(
    doc: &sr_model::Document,
    output: &m::Output,
    gpu: Option<&Gpu>,
    opts: &Options,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<Report, DeliverError> {
    let (report, outcome) = deliver_reporting(doc, output, gpu, opts, progress);
    outcome.map(|()| report)
}

/// [`deliver`], keeping the report of a delivery that fails: what was measured before the error, for the render
/// report (SREP 18), which is written whether or not the render succeeded.
pub fn deliver_reporting(
    doc: &sr_model::Document,
    output: &m::Output,
    gpu: Option<&Gpu>,
    opts: &Options,
    progress: &mut dyn FnMut(u64, u64),
) -> (Report, Result<(), DeliverError>) {
    let mut report = Report::default();
    let outcome = run_delivery(doc, output, gpu, opts, progress, &mut report);
    (report, outcome)
}

/// Records what the meter read of the delivered mix. A value at the meter's floor (−200) means the programme had
/// no window of that length above silence and is left out: integrated loudness needs one 400 ms gating block, and
/// the loudness range one 3-second window at or above −70 LUFS.
fn record_loudness(report: &mut Report, m: &sr_audio::loudness::Measurement) {
    let read = |v: f64| (v > -150.0).then_some(v);
    report.loudness = read(m.integrated);
    report.true_peak = Some(m.true_peak);
    report.momentary_max = read(m.momentary_max);
    report.short_term_max = read(m.short_term_max);
    report.loudness_range = (m.short_term_max >= -70.0).then_some(m.range);
}

/// The audio chain of one delivery, from the mix to the files the mux reads, with everything it needs owned so that it
/// can run on a thread of its own ([`PendingAudio`]) beside the picture's render, or in line before it.
struct AudioJob {
    /// The evaluator the delivery renders with.
    ev: std::sync::Arc<Evaluator>,
    /// The scene's mix when it was made already, for the evaluator's analysis table; `None` mixes it here.
    mixed: Option<Option<SceneAudio>>,
    output: m::Output,
    codec: Codec,
    segments: Option<crate::segments::TimeMap>,
    fps: f64,
    representation: Option<String>,
    takes_audio: bool,
    overlay_audio_needed: bool,
    own_tracks: bool,
    separate_description: bool,
    description_id: Option<String>,
    container: Option<Container>,
    start: f64,
    end: f64,
    /// The delivery's scratch directory, which outlives the job.
    tmp: PathBuf,
    /// The main output's path.
    path: PathBuf,
    /// Hand the mix and the output's programme back, for the renderer's shaders and the overlay; otherwise they are
    /// dropped as soon as the files are written.
    keep: bool,
}

/// What an [`AudioJob`] made, for the report and the mux.
#[derive(Default)]
struct AudioDone {
    /// Seconds spent mixing (when the job mixed) and rendering the output's programme.
    seconds: f64,
    /// What the meter read of the delivered mix, when the output carries audio.
    loudness: Option<sr_audio::loudness::Measurement>,
    ceiling: Option<crate::ceiling::Held>,
    /// The bit depth of the final PCM written to the output's path (an audio-only WAV).
    pcm_bits: Option<u16>,
    /// The audio file the mux reads, and whether it is already coded for the output.
    file: Option<(PathBuf, bool)>,
    /// The audio description stream the mux carries.
    description: Option<PathBuf>,
    scene_audio: Option<SceneAudio>,
    programme: Option<crate::segment_audio::OutputAudio>,
}

impl AudioDone {
    /// Records the job's measurements in the report.
    fn record(&self, report: &mut Report) {
        report.audio_seconds += self.seconds;
        if let Some(m) = &self.loudness {
            record_loudness(report, m);
        }
        if let Some(held) = &self.ceiling {
            report.audio_ceiling = Some(held.clone());
        }
        if let Some(bits) = self.pcm_bits {
            report.files.push(report.path.clone());
            report.passes = 1;
            report.encoder = format!("pcm_s{bits}le");
        }
    }

    /// Gives the encode its audio: the file the ceiling's probe coded is muxed as it is, any other is encoded with the
    /// output's codec and bitrate.
    fn mux_into(&self, spec: &mut EncodeSpec, output: &m::Output) {
        spec.audio = self.file.clone().map(|(file, coded)| {
            (file, if coded { "copy".to_string() } else { output.audio_codec.clone() }, output.audio_bitrate)
        });
        spec.tags.description = self.description.clone().map(|path| sr_media::encode::DescriptionTrack { path });
    }
}

impl AudioJob {
    fn run(self) -> Result<AudioDone, DeliverError> {
        let AudioJob {
            ev,
            mixed,
            output,
            codec,
            segments,
            fps,
            representation,
            takes_audio,
            overlay_audio_needed,
            own_tracks,
            separate_description,
            description_id,
            container,
            start,
            end,
            tmp,
            path,
            keep,
        } = self;
        let p = ev.program();
        let output = &output;
        let description_id = description_id.as_deref();
        let mut done = AudioDone::default();
        let mut scene_audio = match mixed {
            Some(mixed) => mixed,
            None => {
                let t = Instant::now();
                let mixed = audio::mix_scene(&ev, fps, representation.as_deref())?;
                done.seconds += t.elapsed().as_secs_f64();
                mixed
            }
        };
        if (takes_audio || overlay_audio_needed) && scene_audio.is_none() && own_tracks {
            scene_audio = Some(audio::silent(p, fps));
        }
        // Without segments, selection attributes have no effect. Only output-owned
        // tracks need a programme of their own, starting at output/@start.
        let plain = match (&segments, own_tracks || separate_description) {
            (None, true) => Some(crate::segment_audio::output_map(p, output)?),
            _ => None,
        };
        let mut programme = match (segments.as_ref().or(plain.as_ref()), &scene_audio) {
            (Some(tm), Some(sa)) if takes_audio || overlay_audio_needed => {
                let t = Instant::now();
                let mut a = crate::segment_audio::render_version(
                    p,
                    sa,
                    output,
                    tm,
                    fps,
                    representation.as_deref(),
                    segments.is_some(),
                    description_id.filter(|_| takes_audio).map(|id| (id, !separate_description)),
                )?;
                if !overlay_audio_needed {
                    a.nodes.clear();
                }
                done.seconds += t.elapsed().as_secs_f64();
                Some(a)
            }
            _ => None,
        };
        let wants_audio = takes_audio && scene_audio.is_some() && (segments.is_none() || programme.is_some());
        if separate_description {
            if let (Some(sa), Some(tm)) = (&scene_audio, segments.as_ref().or(plain.as_ref())) {
                let described = crate::segment_audio::render_version(
                    p,
                    sa,
                    output,
                    tm,
                    fps,
                    representation.as_deref(),
                    segments.is_some(),
                    description_id.map(|id| (id, true)),
                )?;
                let origin = plain.as_ref().map_or(0.0, |tm| tm.composition(0, 0.0));
                let mut part = audio::slice(&described.master, sa.mix.rate, start - origin, end - origin);
                let master = &sa.mix.master;
                if (master.limiter || master.normalize != sr_audio::mix::Normalize::None)
                    && crate::ceiling::applies(&output.audio_codec, container)
                {
                    crate::ceiling::hold(
                        &mut part,
                        sa.mix.rate,
                        sa.mix.layout,
                        master.true_peak,
                        output.audio_bitrate,
                        &tmp,
                    )?;
                }
                let file = tmp.join("description.wav");
                sr_audio::wav::write(&file, &part, sa.mix.rate, 32, sa.mix.layout, false)?;
                done.description = Some(file);
            }
        }
        if let (true, Some(sa)) = (wants_audio, &scene_audio) {
            // Plain programmes start at their mapped composition time; the scene and
            // explicitly segmented programmes start at zero on their respective clocks.
            let origin = plain.as_ref().map_or(0.0, |tm| tm.composition(0, 0.0));
            let master = programme.as_ref().map(|a| &a.master).unwrap_or(&sa.mixed.master);
            let mut part = audio::slice(master, sa.mix.rate, start - origin, end - origin);
            let weights = sa.mix.layout.loudness_weights();
            done.loudness = Some(sr_audio::loudness::measure(&part, sa.mix.rate as f64, &weights));
            let container = output
                .container
                .map(|c| c.as_str().to_string())
                .and_then(|c| Container::parse(&c))
                .or_else(|| Container::from_path(&path));
            // a lossy encode can leave the master's ceiling: hold it for the stream that is delivered
            let master_settings = &sa.mix.master;
            let limited = master_settings.limiter || master_settings.normalize != sr_audio::mix::Normalize::None;
            // the ceiling's last probe is the AAC encode of the mix as it is delivered: a picture's container muxes it
            // as it is
            let mut probed = None;
            if limited && crate::ceiling::applies(&output.audio_codec, container) {
                let (held, coded) = crate::ceiling::hold_encoded(
                    &mut part,
                    sa.mix.rate,
                    sa.mix.layout,
                    master_settings.true_peak,
                    output.audio_bitrate,
                    &tmp,
                )?;
                if !held.passes.is_empty() {
                    done.loudness = Some(sr_audio::loudness::measure(&part, sa.mix.rate as f64, &weights));
                }
                done.ceiling = Some(held);
                probed = Some(coded);
            }
            let stream_copy =
                !codec.is_audio_only() && matches!(container, Some(Container::Mp4 | Container::Mov | Container::Mkv));
            if let Some(coded) = probed.filter(|_| stream_copy) {
                done.file = Some((coded, true));
            } else if codec.is_audio_only() && container.is_none_or(|c| c == Container::Wav) {
                // final PCM with TPDF dither at the mix bit depth
                sr_audio::wav::write(&path, &part, sa.mix.rate, sa.bits, sa.mix.layout, sa.dither)?;
                done.pcm_bits = Some(sa.bits);
            } else {
                let file = tmp.join("mix.wav");
                sr_audio::wav::write(&file, &part, sa.mix.rate, 32, sa.mix.layout, false)?;
                done.file = Some((file, false));
            }
        }
        if keep {
            done.scene_audio = scene_audio;
            done.programme = programme.take();
        }
        Ok(done)
    }
}

/// An [`AudioJob`] running on a thread of its own, joined before the mux needs its files; dropping it joins it, so
/// the scratch directory is never removed under it.
struct PendingAudio(Option<std::thread::JoinHandle<Result<AudioDone, DeliverError>>>);

impl PendingAudio {
    fn spawn(job: AudioJob) -> PendingAudio {
        PendingAudio(Some(std::thread::spawn(move || job.run())))
    }

    fn join(mut self) -> Result<AudioDone, DeliverError> {
        // `join` takes `self` and only `Drop` empties the slot otherwise, so the handle is always there
        let Some(handle) = self.0.take() else {
            return Err(DeliverError::Invalid("the audio chain was joined twice".into()));
        };
        handle
            .join()
            .unwrap_or_else(|p| Err(DeliverError::Invalid(format!("the audio chain panicked: {}", panic_message(&*p)))))
    }
}

impl Drop for PendingAudio {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            let _ = handle.join();
        }
    }
}

/// Joins the audio chain, when it runs beside the picture and has not been joined yet: records what it measured and
/// gives the encode its audio.
fn join_audio(
    pending: &mut Option<PendingAudio>,
    report: &mut Report,
    spec: &mut EncodeSpec,
    output: &m::Output,
) -> Result<(), DeliverError> {
    if let Some(job) = pending.take() {
        let done = job.join()?;
        done.record(report);
        done.mux_into(spec, output);
    }
    Ok(())
}

/// Waits until every worker holding a sender of `finished` has returned, reporting progress every `tick` meanwhile.
/// A worker's sender drops as its thread returns or unwinds, so the wait ends as the last one does, not at a tick.
fn wait_for_workers(finished: &std::sync::mpsc::Receiver<()>, tick: std::time::Duration, mut progress: impl FnMut()) {
    loop {
        match finished.recv_timeout(tick) {
            Ok(()) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => progress(),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Starts the encoder of a single stream. The audio chain, when it runs beside the picture, is joined first: the
/// encoder opens the audio file as it starts. A delivery that carries no audio does not wait for it.
fn start_encoder(
    spec: &mut EncodeSpec,
    output: &m::Output,
    pending: &mut Option<PendingAudio>,
    late: &mut Option<AudioDone>,
    carries_audio: bool,
) -> Result<Feeder, DeliverError> {
    if carries_audio {
        if let Some(job) = pending.take() {
            let done = job.join()?;
            done.mux_into(spec, output);
            *late = Some(done);
        }
    }
    Feeder::start(spec, 1)
}

fn run_delivery(
    doc: &sr_model::Document,
    output: &m::Output,
    gpu: Option<&Gpu>,
    opts: &Options,
    progress: &mut dyn FnMut(u64, u64),
    report: &mut Report,
) -> Result<(), DeliverError> {
    let started = Instant::now();
    let codec = Codec::parse(output.codec.as_str())
        .ok_or_else(|| DeliverError::Invalid(format!("unknown codec {}", output.codec.as_str())))?;
    // Choosing the encoder probes FFmpeg with a test encode for each hardware encoder in turn, which takes a good part of
    // a second; it starts now, beside the audio mix and the renderer's setup, and the encode later finds the answer
    // waiting (sr_media keeps it, and a caller that asks while the probe runs waits for it rather than probing again).
    let hardware = opts.hardware;
    std::thread::spawn(move || sr_media::encode::choose_encoder(codec, hardware));
    let base =
        opts.out_dir.clone().or_else(|| Some(doc.base_dir().to_path_buf())).unwrap_or_else(|| PathBuf::from("."));
    let representation = opts.representation.clone().or_else(|| output.representation.clone());
    let mut eo = eval_options(output, opts);
    let ev0 = std::sync::Arc::new(Evaluator::new(doc, &eo).map_err(DeliverError::Document)?);
    let p0 = ev0.program();
    let frame_rate = output.fps.unwrap_or(p0.fps);
    let fps = frame_rate.as_f64();
    // with segments, the output has its own timeline: start and end (from the command line) are output times
    let segments = crate::segments::TimeMap::of(p0, output).map_err(DeliverError::Invalid)?;
    let timeline = segments.as_ref().map(|tm| tm.duration).unwrap_or(p0.duration);
    let start = opts.start.unwrap_or(output.start).max(0.0);
    let end = opts.end.or(output.end).unwrap_or(timeline).min(timeline);
    if end <= start {
        return Err(DeliverError::Invalid(format!("empty range {start}..{end}")));
    }
    // CLI overrides define the effective output, including the origin/duration of its own
    // audio, overlay and captions. Explicit segments retain their independent output clock.
    let effective_output = m::Output { start, end: Some(end), ..output.clone() };
    let output = &effective_output;
    // OUT1 before anything is rendered: validate checks the document's outputs, not an ad-hoc one (encode -o)
    let (frame, source) = output_frame(p0, output);
    if let Some(message) = sr_model::rules::odd_frame_message(
        output.codec.as_str(),
        &output.pixel_format,
        frame.map(|v| v.round().max(1.0) as u64),
        &source,
    ) {
        let d = sr_model::Diagnostic::error("OUT1", message, sr_model::Loc::default(), "/scene/output");
        return Err(DeliverError::Document(sr_model::Report { diagnostics: vec![d] }));
    }
    *report = Report {
        path: resolve(&base, &output.path),
        range: [start, end],
        fps,
        quality: opts.quality.unwrap_or(doc.scene.project.quality).as_str().to_string(),
        ..Default::default()
    };
    // information (inert attributes) is for validate and the render report, not a delivery warning
    report
        .warnings
        .extend(doc.warnings().iter().filter(|d| !d.is_info()).map(|d| format!("{}: {}", d.code, d.message)));
    // with segments the programme is the output's own mix in output time (its own tracks may be its
    // only audio)
    let takes_audio = output.audio && (codec.takes_audio() || codec.is_audio_only());
    // Shader inputs are still needed for silent video and image-sequence exports.
    let overlay_audio_needed = output.overlay.is_some();
    // The mix feeds the analysis table the evaluator reads, and the shaders' and the overlay's audio, so it comes first.
    // When the picture reads none of that (and the output is a picture, which can wait for the audio files where the
    // mux needs them), the audio chain runs on a thread of its own beside the render instead.
    let beside = !codec.is_audio_only() && !overlay_audio_needed && audio::picture_ignores_mix(p0);
    report.audio_beside_picture = beside;
    let mut mixed: Option<Option<SceneAudio>> = None;
    let ev = if beside {
        ev0.clone()
    } else {
        let t_audio = Instant::now();
        let scene_audio = audio::mix_scene(&ev0, fps, representation.as_deref())?;
        report.audio_seconds = t_audio.elapsed().as_secs_f64();
        let ev = match &scene_audio {
            Some(sa) => {
                eo.analysis = sa.analysis.clone();
                std::sync::Arc::new(Evaluator::new(doc, &eo).map_err(DeliverError::Document)?)
            }
            None => ev0.clone(),
        };
        mixed = Some(scene_audio);
        ev
    };
    let p = ev.program();
    report.evaluation_warnings.extend_from_slice(ev.warnings());
    if let Some(parent) = report.path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // captions in output time: with segments the composition's are mapped; the output's own tracks
    let captions = crate::captions::output_captions(p, output, segments.as_ref())?;
    report.warnings.extend(captions.warnings.iter().cloned());
    if segments.is_none() {
        report.caption_tracks.extend(p.scene.captions.iter().flat_map(|c| &c.caption_tracks).map(Into::into));
    }
    report.caption_tracks.extend(captions.tracks.iter().map(Into::into));
    for track in &report.caption_tracks {
        report.findings.push(crate::render_report::Finding::node(
            crate::render_report::code::engine("CAPTION-TRACK"),
            sr_model::Severity::Info,
            &track.id,
            format!(
                "caption track {}: kind={}, language={}, label={}",
                track.id,
                track.kind,
                track.language,
                track.label.as_deref().unwrap_or("")
            ),
        ));
    }

    report.findings.extend(captions.warnings.iter().map(|w| {
        crate::render_report::Finding::scene(
            crate::render_report::code::engine("CAPTION"),
            sr_model::Severity::Warning,
            w.clone(),
        )
    }));
    // Check once for the delivery, before workers start. Composition tracks and
    // output-owned tracks both satisfy the document's caption requirement.
    let requires_captions = p.scene.metadata.as_ref().is_some_and(|md| {
        md.children.iter().any(|c| matches!(c, m::MetadataChild::Accessibility(a) if a.require_captions))
    });
    if requires_captions
        && captions.tracks.is_empty()
        && p.scene.captions.as_ref().is_none_or(|c| c.caption_tracks.is_empty())
    {
        return Err(DeliverError::Accessibility("requireCaptions: the output has no caption track".into()));
    }
    let tmp = Scratch(std::env::temp_dir().join(format!(
        "scene-render-{}-{}",
        std::process::id(),
        started.elapsed().as_nanos()
    )));
    std::fs::create_dir_all(&tmp.0)?;
    let mut chapters = None;
    if let Some(tm) = &segments {
        let empty = empty_ends(&ev, tm);
        report.findings.extend(empty.iter().map(|w| {
            crate::render_report::Finding::scene(
                crate::render_report::code::engine("SEGMENT-EMPTY"),
                sr_model::Severity::Warning,
                w.clone(),
            )
        }));
        report.warnings.extend(empty);
        chapters = write_chapters(p, tm, start, end, &tmp)?;
    }
    if segments.is_none() {
        let tm = crate::segments::TimeMap::span_of(p, 0.0, p.duration).map_err(DeliverError::Invalid)?;
        chapters = write_chapters(p, &tm, start, end, &tmp)?;
    }
    let own_tracks = output.children.iter().any(|c| matches!(c, m::OutputChild::AudioTrack(_)));
    let container = output
        .container
        .and_then(|c| Container::parse(c.as_str()))
        .or_else(|| Container::from_path(&report.path))
        .or_else(|| Container::default_for(codec));
    let description_id = crate::metadata::description(&p.scene).filter(|id| {
        p.scene
            .audio_mix
            .iter()
            .flat_map(|am| &am.children)
            .any(|c| matches!(c, m::AudioMixChild::AudioTrack(t) if t.id == *id))
            || output.children.iter().any(|c| matches!(c, m::OutputChild::AudioTrack(t) if t.id == *id))
    });
    let separate_description = takes_audio
        && !codec.is_audio_only()
        && container.is_some_and(|c| c.carries_audio_description())
        && description_id.is_some();
    if separate_description && matches!(container, Some(Container::Mov | Container::Webm)) {
        let name = if container == Some(Container::Mov) { "MOV" } else { "WebM" };
        report.warnings.push(format!("{name} does not retain the audio-description disposition in FFmpeg; the alternate stream is named Audio description"));
    }
    if takes_audio && description_id.is_some() && !separate_description {
        report.warnings.push(
            "this output cannot identify a separate audio description stream; narration remains in the programme mix"
                .into(),
        );
    }

    // What the audio chain will deliver, known before it runs: the scene mixes something (or the output's own tracks
    // are all its audio), an output with segments or tracks of its own has a programme, and the programme is what
    // the audio streams are made of.
    let mix_exists = audio::has_audio(&ev) || ((takes_audio || overlay_audio_needed) && own_tracks);
    let programme_exists = (segments.is_some() || own_tracks || separate_description)
        && mix_exists
        && (takes_audio || overlay_audio_needed);
    let wants_audio = takes_audio && mix_exists && (segments.is_none() || programme_exists);
    let job = AudioJob {
        ev: ev.clone(),
        mixed,
        output: output.clone(),
        codec,
        segments: segments.clone(),
        fps,
        representation: representation.clone(),
        takes_audio,
        overlay_audio_needed,
        own_tracks,
        separate_description,
        description_id: description_id.map(str::to_string),
        container,
        start,
        end,
        tmp: tmp.0.clone(),
        path: report.path.clone(),
        keep: !beside,
    };
    // Beside the picture, the chain's files are needed where the mux starts; run in line, it is done now.
    let mut pending: Option<PendingAudio> = None;
    let mut audio_done = AudioDone::default();
    if beside {
        pending = Some(PendingAudio::spawn(job));
    } else {
        audio_done = job.run()?;
        audio_done.record(report);
        debug_assert_eq!(audio_done.file.is_some() || audio_done.pcm_bits.is_some(), wants_audio);
    }
    // the audio streams, when the output carries any: joined before the encoder opens their files
    let carries_audio = takes_audio && mix_exists;
    let main_language = crate::metadata::audio_language(
        p,
        output,
        description_id.filter(|_| takes_audio).map(|id| (id, !separate_description)),
        representation.as_deref(),
        &mut report.warnings,
    );
    let description_language = description_id.filter(|_| separate_description).map(|id| {
        crate::metadata::audio_language(p, output, Some((id, true)), representation.as_deref(), &mut report.warnings)
    });
    if wants_audio {
        if main_language != "und" && matches!(container, Some(Container::Wav | Container::Mp3 | Container::Mxf)) {
            report.warnings.push(
                "this container does not retain audio stream language tags; language remains in the delivery manifest"
                    .into(),
            );
        }
        report.audio_languages.push(main_language.clone());
        report.audio_languages.extend(description_language.iter().cloned());
    }
    if codec.is_audio_only() {
        if report.files.is_empty() {
            let Some((f, _)) = audio_done.file.clone() else {
                return Err(DeliverError::Invalid("audio-only output, but the scene has no audio".into()));
            };
            let mut spec = base_spec(
                output,
                &report.path,
                codec,
                [16, 16],
                fps,
                InputFormat::Nv12,
                &OutputColor::new(m::ColorSpace::Srgb, m::Transfer::Auto, false),
                Some((f, output.audio_codec.clone(), output.audio_bitrate)),
                p,
                opts,
            );
            spec.chapters = chapters.clone();
            spec.tags.audio_language = Some(main_language.clone());
            let enc = Encoder::start(&spec)?;
            report.encoder = enc.encoder.clone();
            enc.finish()?;
            report.files.push(report.path.clone());
            report.passes = 1;
        }
    } else {
        // a document with 3D objects needs an adapter that can run the 3D pass; the one asked for or handed in
        // that cannot is an error, not frames without the objects
        let gpu = match gpu {
            Some(g) => {
                if p.uses_3d() {
                    sr_gpu::gpu::require_3d(&g.info)?;
                }
                g.clone()
            }
            None => Gpu::new_for(p.uses_3d())?,
        };
        report.render_adapter = Some(RenderAdapter {
            name: gpu.info.name.clone(),
            backend: format!("{:?}", gpu.info.backend),
            device_type: format!("{:?}", gpu.info.device_type),
            software: gpu.is_software(),
        });
        let size = sr_gpu::output::frame_size(output_frame(p, output).0, &gpu.device.limits())
            .map_err(DeliverError::Invalid)?;
        report.size = size;
        let format = input_format(output, codec);
        let exr = codec == Codec::ExrSequence;
        let rgb_image = codec.is_sequence() || matches!(codec, Codec::Gif | Codec::Apng | Codec::Webp);
        let color = if exr {
            OutputColor::new(
                if output.color_space == m::ColorSpace::Srgb { m::ColorSpace::LinearSrgb } else { output.color_space },
                m::Transfer::Linear,
                true,
            )
        } else {
            OutputColor::new(output.color_space, output.transfer, output.color_range.as_str() == "full" || rgb_image)
        };
        let mut renderer = Renderer::new(gpu.clone(), p);
        renderer.quality = opts.quality;
        renderer.path_noise = opts.path_noise;
        renderer.representation = representation.clone();
        renderer.burn_captions = output.burn_captions.clone();
        // with segments, captions burn in output time over the placed picture
        renderer.captions_off = segments.is_some();
        if let Some(sa) = audio_done.scene_audio.as_mut() {
            // the mix is done with these buffers (the master was sliced to disk above), so the shaders take over the
            // channels they read rather than doubling a programme's worth of audio; the output's overlay is a document
            // of its own whose shaders may read any of them
            let read = if overlay_audio_needed {
                None
            } else {
                sr_gpu::shader::audio_sources(p, &p.base_dirs.first().cloned().unwrap_or_default())
            };
            renderer.audio =
                shader_signals(&mut sa.mixed.master, &mut sa.mixed.nodes, sa.mix.rate as f64, read.as_ref());
        }
        let overlay_audio = renderer.audio.as_ref().filter(|_| overlay_audio_needed).map(|audio| {
            if let Some(mixed) = audio_done.programme.take() {
                let mut channels: std::collections::HashMap<_, _> =
                    mixed.nodes.into_iter().map(|(id, samples)| (id, std::sync::Arc::new(samples))).collect();
                channels.insert("master".into(), std::sync::Arc::new(mixed.master));
                std::sync::Arc::new(sr_gpu::shader::AudioSignals { rate: audio.rate, channels })
            } else if start > 0.0 {
                let channels = audio
                    .channels
                    .iter()
                    .map(|(id, samples)| {
                        (id.clone(), std::sync::Arc::new(audio::slice(samples, audio.rate as u32, start, end)))
                    })
                    .collect();
                std::sync::Arc::new(sr_gpu::shader::AudioSignals { rate: audio.rate, channels })
            } else {
                audio.clone()
            }
        });
        let frame_times: Option<Vec<crate::segments::FrameTime>> = segments.as_ref().map(|tm| {
            tm.frames(fps).into_iter().filter(|f| f.output >= start - 1e-9 && f.output < end - 1e-9).collect()
        });
        let times: Vec<f64> = match &frame_times {
            Some(f) => f.iter().map(|f| f.composition).collect(),
            None => {
                let n = frame_rate.frame_count(end - start).max(1);
                (0..n).map(|k| start + frame_rate.frame_time(k)).collect()
            }
        };
        // content a safe area holds to its region: findings are warnings, or fail before anything is rendered
        let safe = if p.safe_enforce == sr_eval::safe_area::SafeEnforce::Off && p.scene.captions.is_none() {
            Vec::new()
        } else if let Some((tm, frames)) = segments.as_ref().zip(frame_times.as_ref()) {
            // Cuts, remaps and holds need not visit composition times in increasing order. A bounding
            // interval would also audit footage that was cut. Include both pictures used by a join.
            let mut audited = times.clone();
            for f in frames {
                if let Some(join) = tm.join_at(f.output) {
                    audited.extend([join.from.composition, join.to.composition]);
                }
            }
            // Simulations advance chronologically, and a frozen/repeated frame only needs one check.
            audited.sort_by(f64::total_cmp);
            audited.dedup();
            sr_gpu::safe_audit::audit(&ev, &audited)
        } else {
            // Output fps and a fractional trim origin can both differ from the project's
            // frame grid. Only the timestamps actually exported belong in this audit.
            sr_gpu::safe_audit::audit(&ev, &times)
        };
        let mut diagnostics: Vec<_> = safe.iter().map(|t| t.diagnostic(p)).collect();
        report.findings.extend(safe.iter().zip(&diagnostics).map(|(t, d)| {
            crate::render_report::Finding::node(
                crate::render_report::code::SAFE_AREA,
                d.severity,
                &t.finding.id,
                d.message.clone(),
            )
            .at_time(t.t, t.t)
            .measuring(t.finding.overshoot, Some(0.0), "px")
        }));
        let mut overlay = crate::overlay::Overlay::new(
            doc,
            output,
            timeline,
            size,
            &captions,
            &gpu,
            &eo,
            opts.quality,
            representation.as_deref(),
        )?;
        if let Some(layer) = &mut overlay {
            layer.set_audio(overlay_audio.clone());
            let output_times: Vec<f64> = match &frame_times {
                Some(frames) => frames.iter().map(|f| f.output).collect(),
                None => times.iter().map(|t| t - output.start).collect(),
            };
            let overlay_diagnostics = layer.safe_diagnostics(&output_times);
            report.findings.extend(overlay_diagnostics.iter().map(crate::render_report::Finding::of_diagnostic));
            diagnostics.extend(overlay_diagnostics);
        }
        if diagnostics.iter().any(|d| d.is_error()) {
            // the findings above carry them, with their nodes and times, into the render report
            return Err(DeliverError::Document(sr_model::Report { diagnostics }));
        }
        report.warnings.extend(diagnostics.iter().map(|d| format!("{}: {}", d.code, d.message)));
        if opts.report {
            report.findings.extend(text_fit_findings(&ev, &times, p.fonts_pinned()));
        }
        // SREP 19: reading speed, display time and text size, when accessibility/@legibilityCheck asks
        if let Some(s) = crate::legibility::Settings::of(p) {
            let mut found =
                crate::legibility::text_findings(&ev, &s, &times, fps, frame, [size[0] as f64, size[1] as f64]);
            let base_dir = p.base_dirs.first().cloned().unwrap_or_default();
            let own: Vec<&m::CaptionTrack> = captions.tracks.iter().collect();
            let tracks: Vec<&m::CaptionTrack> = match &segments {
                // mapped composition tracks and the output's own, in output time
                Some(_) => own,
                None => p.scene.captions.iter().flat_map(|c| c.caption_tracks.iter()).chain(own).collect(),
            };
            let mut shown = Vec::new();
            for tr in tracks {
                let t = sr_gpu::text::load_track(tr, &base_dir).map_err(DeliverError::Invalid)?;
                let items = if t.burn {
                    t.pages
                        .iter()
                        .map(|pg| crate::legibility::Shown { start: pg.start, end: pg.end, text: pg.text() })
                        .collect()
                } else {
                    t.cues
                        .iter()
                        .map(|c| crate::legibility::Shown { start: c.start, end: c.end, text: c.text.clone() })
                        .collect()
                };
                shown.push(crate::legibility::ShownTrack {
                    id: tr.id.clone(),
                    reading_speed: crate::legibility::ShownTrack::reading_speed_of(tr),
                    shown: items,
                });
            }
            found.extend(crate::legibility::caption_findings(&s, &shown, [start, end]));
            for f in found {
                if f.severity == sr_model::Severity::Error && report.accessibility_error.is_none() {
                    report.accessibility_error = Some(format!("legibilityCheck: {}", f.message));
                }
                report.accessibility.push(format!("legibilityCheck: {}", f.message));
                report.findings.push(f);
            }
        }
        let n = times.len() as u64;
        let mut video = Video {
            ev: &ev,
            renderer,
            stage: OutputStage::new(gpu.device.clone(), gpu.queue.clone()),
            color,
            format,
            size,
            keep_alpha: output.alpha || rgb_image,
            fps,
            reframe: p.reframe,
            segments: segments.as_ref().zip(frame_times),
            overlay,
            origin: if segments.is_some() { 0.0 } else { output.start },
            join_tex: None,
            contrasts: ContrastPrefetch::default(),
        };
        if let Some(overlay) = &video.overlay {
            for warning in overlay.warnings() {
                if !report.evaluation_warnings.contains(warning) {
                    report.evaluation_warnings.push(warning.clone());
                }
            }
        }
        let mut spec = base_spec(output, &report.path, codec, size, fps, format, &color, None, p, opts);
        // run in line, the chain's files are known now; beside the picture they are joined where the mux needs them
        audio_done.mux_into(&mut spec, output);
        let pixel_aspect = p.scene.project.pixel_aspect.get();
        if spec.tags.sar.is_none() && (pixel_aspect - 1.0).abs() > pixel_aspect * 1e-6 {
            return Err(DeliverError::Invalid(format!(
                "pixelAspect {pixel_aspect} cannot be represented within the encoder's rational precision"
            )));
        }
        spec.tags.audio_language = Some(main_language.clone());
        spec.tags.description_language = description_language;
        if spec.container() == Some(Container::Webm) {
            if let Some((n, d)) = spec.tags.sar {
                let numerator = u64::from(size[0]) * u64::from(n);
                if !numerator.is_multiple_of(u64::from(d)) {
                    let display = (numerator as f64 / f64::from(d)).round() as u64;
                    report.warnings.push(format!("WebM stores integral display dimensions: pixelAspect {n}/{d} becomes {display}:{} at this frame width", size[0]));
                }
            }
        }
        if let Some(tc) = p.scene.project.timecode_start {
            // Timecode labels use the output rate; the base is authored at the project rate.
            // Segmented deliveries have their own output clock beginning at the project base.
            let first_frame =
                video.segments.as_ref().and_then(|(_, frames)| frames.first()).map_or(start, |f| f.output);
            let seconds =
                tc.to_seconds(p.scene.project.fps).map_err(|e| DeliverError::Invalid(e.to_string()))? + first_frame;
            let output_fps = output.fps.unwrap_or(p.scene.project.fps);
            spec.tags.timecode = Some(
                sr_model::values::Timecode::from_frame(
                    (seconds * fps).round() as u64,
                    output_fps,
                    tc.drop_frame && output_fps.nominal().is_multiple_of(30),
                )
                .map_err(|e| DeliverError::Invalid(e.to_string()))?
                .to_string(),
            );
        }

        spec.start_number = (start * fps).round() as u64;
        spec.chapters = chapters.clone();
        let fit = output.max_file_size;
        let checks = Checks::of(p);
        let t_video = Instant::now();
        report.segments = 1;
        // llvmpipe already spreads one device over every core, so automatic parallelism would only add
        // devices competing for them; an explicit worker count is still honoured
        let can_split = splittable(output, codec, &ev);
        let auto = matches!(opts.parallel, Parallel::Auto);
        let host = if can_split && auto && !gpu.is_software() { host_workers(codec, opts) } else { 1 };
        let observe = checks.flash_on() || checks.contrast_on();
        let mut plan = segment_plan(opts.parallel, can_split, host, end - start, n, observe);
        // the debug layers name every object through the Vulkan loader, which is not safe from several devices at
        // once: one worker renders the same chunks on the caller's device
        if sr_gpu::gpu::debug_layers() && plan.workers > 1 {
            report.serial_because = Some(format!(
                "GPU debug layers are on (SR_GPU_DEBUG / --debug-gpu): rendering with 1 worker instead of {}, as their object naming is not safe from several devices at once",
                plan.workers
            ));
            plan.workers = 1;
        }
        let workers = plan.workers;
        if output.two_pass || fit.is_some() {
            // render once into a lossless intermediate, then encode it as often as needed
            let inter = tmp.join("intermediate.mkv");
            let ipix = match format {
                InputFormat::Nv12 => "yuv420p",
                InputFormat::P010 => "yuv420p10le",
                InputFormat::Rgba8 => "bgra",
                _ => "gbrap16le",
            };
            let ispec = EncodeSpec {
                path: inter.clone(),
                codec: Codec::Ffv1,
                container: Some(Container::Mkv),
                pixel_format: ipix.into(),
                audio: None,
                pass: None,
                alpha: false,
                hdr: Hdr::default(),
                metadata: Vec::new(),
                chapters: None,
                bitrate: None,
                ..spec.clone()
            };
            let mut feeder = Feeder::start(&ispec, 1)?;
            let mut done = 0;
            let mut judge = Judge::new(checks.flash_on());
            video.stage.seek(spec.start_number as u32);
            video.prepare_contrasts(&times);
            video.run(&times, report, &mut judge, |b| {
                done += 1;
                progress(done, n);
                feeder.send(b)
            })?;
            feeder.finish()?;
            report.frames = n;
            // the intermediate has no audio: the passes that mux it need the chain's files
            join_audio(&mut pending, report, &mut spec, output)?;
            let duration = end - start;
            let audio_bps = if spec.audio.is_some() {
                output.audio_bitrate * (1 + u64::from(spec.tags.description.is_some()))
            } else {
                0
            };
            let mut bitrate = fit.map(|max| sr_media::encode::fit_bitrate(max, duration, audio_bps)).or(output.bitrate);
            for attempt in 0..4 {
                let log = tmp.join(format!("pass{attempt}"));
                let passes: &[u8] = if output.two_pass || fit.is_some() { &[1, 2] } else { &[0] };
                for &pass in passes {
                    let s = EncodeSpec { bitrate, pass: (pass > 0).then(|| (pass, log.clone())), ..spec.clone() };
                    report.encoder = replay(&inter, &s, format, size, n)?;
                    report.passes += 1;
                }
                match fit {
                    Some(max) => {
                        let got = std::fs::metadata(&report.path)?.len();
                        if got <= max {
                            break;
                        }
                        if attempt == 3 {
                            return Err(DeliverError::Invalid(format!(
                                "{} is {got} bytes after four attempts; maxFileSize is {max}",
                                report.path.display()
                            )));
                        }
                        bitrate = bitrate.map(|b| ((b as f64) * (max as f64 / got as f64) * 0.95) as u64);
                    }
                    None => break,
                }
            }
            // the checks' post pass runs once the file is written: a check that fails, or takes long, leaves it whole
            video.judge(judge, &times, report)?;
        } else if plan.chunks > 1 {
            // Each worker renders contiguous chunks with its own encoder. Accessibility-enabled
            // chunks fit one bounded observation batch, so ordering does not serialize the workers.
            // Every chunk starts on a keyframe; stream copy joins them without re-encoding.
            use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
            let chunks = plan.chunks;
            let bounds = plan.bounds(n);
            let ext = report.path.extension().and_then(|e| e.to_str()).unwrap_or("mp4").to_string();
            let parts: Vec<PathBuf> = (0..chunks).map(|i| tmp.join(format!("segment{i:04}.{ext}"))).collect();
            let part_spec = |i: usize| EncodeSpec {
                path: parts[i].clone(),
                audio: None,
                faststart: false,
                metadata: Vec::new(),
                chapters: None,
                ..spec.clone()
            };
            let (audio, keep_alpha) = (video.renderer.audio.clone(), video.keep_alpha);
            // with output segments, the frame time of each entry of `times`
            let all_frames = video.segments.as_ref().map(|(_, f)| f.clone());
            let map = segments.as_ref();
            let cancelled = AtomicBool::new(false);
            let done = AtomicU64::new(0);
            // Create worker devices sequentially and keep them alive until every
            // worker has joined. Concurrent device creation crashed the Vulkan loader;
            // independent devices avoid allocation contention between renderers.
            // A single worker (a software adapter, the debug layers, a short programme) shares the caller's device.
            let worker_gpus: Vec<_> = if workers == 1 {
                vec![gpu.clone()]
            } else {
                (0..workers).map(|_| gpu.open_like()).collect::<Result<_, _>>()?
            };
            // Fixed round-robin chunk ownership lets the coordinator drain each worker in time
            // order. Two queued batches per worker plus one being rendered: at most 384 flash
            // grids (about 11.4 MiB) per worker, independent of output duration.
            let mut judge = Judge::new(checks.flash_on());
            let (senders, receivers): (Vec<_>, Vec<_>) = (0..workers).map(|_| sync_channel::<Seen>(2)).unzip();
            // each worker holds a sender until its thread returns or unwinds: the channel closes as the last one ends
            let (finished_tx, finished) = std::sync::mpsc::channel::<()>();
            let mut lend = (workers == 1).then_some(&mut video);
            let results: Vec<Result<(Report, String), DeliverError>> = std::thread::scope(|sc| {
                let handles: Vec<_> = worker_gpus
                    .iter()
                    .zip(senders)
                    .enumerate()
                    .map(|(worker_index, (gpu, tx))| {
                        let lent = lend.take();
                        let finishing = finished_tx.clone();
                        let (
                            cancelled,
                            done,
                            bounds,
                            times,
                            part_spec,
                            audio,
                            all_frames,
                            representation,
                            ev,
                            eo,
                            captions,
                            overlay_audio,
                        ) = (
                            &cancelled,
                            &done,
                            &bounds,
                            &times,
                            &part_spec,
                            &audio,
                            &all_frames,
                            &representation,
                            &ev,
                            &eo,
                            &captions,
                            &overlay_audio,
                        );
                        sc.spawn(move || -> Result<(Report, String), DeliverError> {
                            let _finishing = finishing;
                            // a single worker renders on the caller's own renderer, stage and overlay
                            let mut built;
                            let worker: &mut Video = match lent {
                                Some(video) => video,
                                None => {
                                    let gpu = gpu.clone();
                                    let mut renderer = Renderer::new(gpu.clone(), p);
                                    renderer.quality = opts.quality;
                                    renderer.path_noise = opts.path_noise;
                                    renderer.representation = representation.clone();
                                    renderer.burn_captions = output.burn_captions.clone();
                                    renderer.audio = audio.clone();
                                    renderer.captions_off = map.is_some();
                                    renderer.share_budgets(workers);
                                    let stage = OutputStage::new(gpu.device.clone(), gpu.queue.clone());
                                    let mut overlay = crate::overlay::Overlay::new(
                                        doc,
                                        output,
                                        timeline,
                                        size,
                                        captions,
                                        &gpu,
                                        eo,
                                        opts.quality,
                                        representation.as_deref(),
                                    )?;
                                    if let Some(layer) = &mut overlay {
                                        layer.set_audio(overlay_audio.clone());
                                    }
                                    built = Video {
                                        ev,
                                        renderer,
                                        stage,
                                        color,
                                        format,
                                        size,
                                        keep_alpha,
                                        fps,
                                        reframe: p.reframe,
                                        segments: None,
                                        overlay,
                                        origin: if map.is_some() { 0.0 } else { output.start },
                                        join_tex: None,
                                        contrasts: ContrastPrefetch::default(),
                                    };
                                    &mut built
                                }
                            };
                            let (mut part, mut encoder) = (Report::default(), String::new());
                            let mut seen = Streaming::new(tx, observe);
                            let mine: Vec<usize> = (worker_index..chunks).step_by(workers).collect();
                            // The next chunk's encoder session starts while this chunk renders, and a finished
                            // chunk's encoder drains on a thread of its own: frames never wait for a session to open
                            // or to flush.
                            let mut closing = Closing::default();
                            let mut next = None;
                            let started = |i: usize| Feeder::start(&part_spec(i), workers);
                            if let Some(&first) = mine.first() {
                                match started(first) {
                                    Ok(f) => next = Some(f),
                                    Err(e) => {
                                        cancelled.store(true, Relaxed);
                                        return Err(e);
                                    }
                                }
                            }
                            for (at, &i) in mine.iter().enumerate() {
                                if cancelled.load(Relaxed) {
                                    return Ok((part, encoder));
                                }
                                let chunk = (|| {
                                    let mut feeder = next.take().ok_or_else(|| {
                                        DeliverError::Invalid("a chunk's encoder was not started".into())
                                    })?;
                                    next = mine.get(at + 1).map(|&j| started(j)).transpose()?;
                                    worker.stage.seek((spec.start_number + bounds[i] as u64) as u32);
                                    worker.segments = map
                                        .zip(all_frames.as_ref())
                                        .map(|(tm, f)| (tm, f[bounds[i]..bounds[i + 1]].to_vec()));
                                    worker.run(&times[bounds[i]..bounds[i + 1]], &mut part, &mut seen, |b| {
                                        done.fetch_add(1, Relaxed);
                                        feeder.send(b)
                                    })?;
                                    seen.flush().map_err(DeliverError::Invalid)?;
                                    closing.start(feeder)
                                })();
                                match chunk {
                                    Ok(Some(e)) => encoder = e,
                                    Ok(None) => {}
                                    Err(e) => {
                                        // stop the other workers at their next chunk
                                        cancelled.store(true, Relaxed);
                                        return Err(e);
                                    }
                                }
                            }
                            match closing.wait() {
                                Ok(Some(e)) => encoder = e,
                                Ok(None) => {}
                                Err(e) => {
                                    cancelled.store(true, Relaxed);
                                    return Err(e);
                                }
                            }
                            Ok((part, encoder))
                        })
                    })
                    .collect();
                'chunks: for i in 0..chunks {
                    for _ in 0..if observe { (bounds[i + 1] - bounds[i]).div_ceil(OBSERVATION_FRAMES) } else { 1 } {
                        loop {
                            match receivers[i % workers].recv_timeout(std::time::Duration::from_millis(200)) {
                                Ok(seen) => {
                                    judge.replay(seen, bounds[i]);
                                    break;
                                }
                                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => progress(done.load(Relaxed), n),
                                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                                    cancelled.store(true, Relaxed);
                                    break 'chunks;
                                }
                            }
                        }
                    }
                }
                // On an error or panic, unblock every producer before joining the scoped threads.
                drop(receivers);
                drop(finished_tx);
                wait_for_workers(&finished, std::time::Duration::from_millis(200), || progress(done.load(Relaxed), n));
                handles
                    .into_iter()
                    .map(|h| {
                        h.join().unwrap_or_else(|p| {
                            Err(DeliverError::Invalid(format!("a segment render panicked: {}", panic_message(&*p))))
                        })
                    })
                    .collect()
            });
            let mut unsupported = std::collections::BTreeSet::new();
            for r in results {
                let (part, encoder) = r?;
                for (a, b) in report.stage_seconds.iter_mut().zip(part.stage_seconds) {
                    *a += b;
                }
                report.decode_wait_seconds += part.decode_wait_seconds;
                report.vector_seconds += part.vector_seconds;
                report.checkpoint_hits += part.checkpoint_hits;
                report.checkpoint_misses += part.checkpoint_misses;
                for (who, [lo, hi]) in part.iterate_steps {
                    let e = report.iterate_steps.entry(who).or_insert([lo, hi]);
                    *e = [e[0].min(lo), e[1].max(hi)];
                }
                for (id, seen) in part.objects3d_seen {
                    merge_seen(&mut report.objects3d_seen, id, seen);
                }
                for (id, metal) in part.metal_without_environment {
                    merge_metal(&mut report.metal_without_environment, id, metal);
                }
                unsupported.extend(part.unsupported);
                if report.encoder.is_empty() {
                    report.encoder = encoder;
                }
            }
            report.unsupported = unsupported.into_iter().collect();
            // every chunk was rendered, so every chunk has been judged; text the workers could not
            // measure in place is measured by this thread's renderer
            progress(n, n);
            // the chunks carry no audio: the join muxes the chain's files
            join_audio(&mut pending, report, &mut spec, output)?;
            spec.join(&parts, &tmp.join("segments.txt"))?;
            video.judge(judge, &times, report)?;
            report.frames = n;
            report.passes = 1;
            report.segments = workers as u32;
        } else {
            // The encoder opens the audio files as it starts. Run beside the picture, the chain has until the first
            // frame is rendered to finish them, so the encoder starts then.
            let mut feeder: Option<Feeder> = None;
            let mut late: Option<AudioDone> = None;
            let mut done = 0;
            let mut judge = Judge::new(checks.flash_on());
            video.stage.seek(spec.start_number as u32);
            video.prepare_contrasts(&times);
            video.run(&times, report, &mut judge, |b| {
                done += 1;
                progress(done, n);
                let started = match feeder.take() {
                    Some(f) => f,
                    None => start_encoder(&mut spec, output, &mut pending, &mut late, carries_audio)?,
                };
                feeder.insert(started).send(b)
            })?;
            let mut feeder = match feeder {
                Some(f) => f,
                None => start_encoder(&mut spec, output, &mut pending, &mut late, carries_audio)?,
            };
            // the file is closed before the checks' post pass: a check that fails, or takes long, leaves it whole
            report.encoder = feeder.finish()?;
            if let Some(chain) = late.take() {
                chain.record(report);
            }
            video.judge(judge, &times, report)?;
            report.frames = n;
            report.passes = 1;
        }
        // an output without audio never waited for the chain; its errors and measurements come in here
        join_audio(&mut pending, report, &mut spec, output)?;
        report.render_fps = n as f64 / t_video.elapsed().as_secs_f64();
        if let Some(msg) = report.accessibility_error.clone() {
            return Err(DeliverError::Accessibility(msg));
        }
        if let Some(s360) = p.scene.scene360.as_ref().filter(|_| output.spherical_metadata && !codec.is_sequence()) {
            use sr_media::spherical::{inject, Projection, Stereo};
            let projection = match s360.layout {
                m::Scene360Layout::Equirectangular => Some(Projection::Equirectangular),
                m::Scene360Layout::Cubemap => Some(Projection::Cubemap),
                _ => None,
            };
            let stereo = match s360.stereo {
                m::Stereo::Mono => Stereo::Mono,
                m::Stereo::TopBottom => Stereo::TopBottom,
                m::Stereo::LeftRight => Stereo::LeftRight,
            };
            let ext = report.path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            match projection {
                Some(pr) if matches!(ext.as_str(), "mp4" | "mov" | "m4v") => {
                    if let Err(e) = inject(&report.path, pr, stereo) {
                        report.unsupported.push(format!("spherical metadata: {e}"));
                    }
                }
                Some(_) => {
                    report.unsupported.push(format!("spherical metadata is written for MP4 and MOV only, not .{ext}"))
                }
                None => report.unsupported.push(format!(
                    "spherical metadata for {} needs a mesh projection, which is not written",
                    s360.layout
                )),
            }
        }
        if codec.is_sequence() {
            for k in 0..n {
                report.files.push(PathBuf::from(sequence_name(&report.path, spec.start_number + k)));
            }
        } else {
            report.files.push(report.path.clone());
        }
        // posters and thumbnails
        for c in &output.children {
            let (m::OutputChild::Poster(st) | m::OutputChild::Thumbnail(st)) = c else { continue };
            let srgb = OutputColor::new(m::ColorSpace::Srgb, m::Transfer::Srgb, true);
            // A plain still's time is composition time; a segmented still's time is
            // output time. Both use the delivered picture's placement and overlay clock.
            let (t, f, output_time) = match &segments {
                Some(tm) => {
                    let t = match &st.marker {
                        Some(mk) => {
                            let c = sr_eval::eval::marker(p, mk)
                                .ok_or_else(|| DeliverError::Invalid(format!("{}: no marker {mk:?}", st.path)))?;
                            tm.output_of(c).ok_or_else(|| {
                                DeliverError::Invalid(format!("{}: marker {mk:?} is in no segment", st.path))
                            })?
                        }
                        None => st.time.clamp(0.0, tm.duration),
                    };
                    let f = tm.at(t);
                    (f.composition, Some(f), t)
                }
                None => {
                    let t = match &st.marker {
                        Some(mk) => sr_eval::eval::marker(p, mk).unwrap_or(st.time),
                        None => st.time,
                    }
                    .clamp(0.0, p.duration);
                    (t, None, t - video.origin)
                }
            };
            let g = ev.evaluate(t);
            let frame = video.render_side(&g, t, f.as_ref())?;
            let mut unsupported = report.unsupported.iter().cloned().collect::<std::collections::BTreeSet<_>>();
            unsupported.extend(frame.stats.unsupported.iter().cloned());
            let (picture, placement) = video.picture(&frame, f.as_ref(), &mut unsupported)?;
            let over = match video.overlay.as_mut() {
                Some(o) => {
                    let (tex, problems) = o.draw(&mut video.stage, output_time)?;
                    unsupported.extend(problems);
                    Some(tex)
                }
                None => None,
            };
            let working = video.renderer.working();
            let pending =
                video.stage.submit_placed(&picture, &working, &srgb, InputFormat::Rgba8, size, true, placement, over);
            let (rgba, psize) = (video.stage.wait(pending), size);
            report.unsupported = unsupported.into_iter().collect();
            let path = resolve(&base, &st.path);
            let fmt = StillFormat::parse(st.format.as_str()).unwrap_or(StillFormat::Jpeg);
            sr_media::encode::write_still(
                &path,
                &rgba,
                psize[0],
                psize[1],
                fmt,
                st.width.map(|w| w as u32),
                st.quality.get(),
            )?;
            report.files.push(path);
        }
    }
    drop(tmp);
    // caption files: the output-time tracks (with segments, the composition's mapped), and without
    // segments the composition's own, cut to the rendered range
    let base_dir = p.base_dirs.first().cloned().unwrap_or_default();
    if segments.is_none() {
        let tracks = p.scene.captions.as_ref().map(|c| c.caption_tracks.as_slice()).unwrap_or_default();
        report.files.extend(write_sidecars(tracks, &base_dir, output, &report.path, start, end)?);
    }
    let origin = if segments.is_some() { 0.0 } else { output.start };
    report.files.extend(write_sidecars(
        &captions.tracks,
        &base_dir,
        output,
        &report.path,
        start - origin,
        end - origin,
    )?);
    if opts.upload {
        let dests: Vec<&m::Destination> = output
            .children
            .iter()
            .filter_map(|c| if let m::OutputChild::Destination(d) = c { Some(d) } else { None })
            .collect();
        report.uploads = crate::destinations::deliver_all(&dests, &report.files, output, &base)?;
    }
    report.seconds = started.elapsed().as_secs_f64();
    Ok(())
}

/// `TXT-FIT` and `TXT-CUT` for the text layers drawn at `times` (SREP 18; the measures of SREP 20 rule 6), and
/// `FONT-GLYPH` and `FONT-SUB` (SREP 21; a missing glyph is an error under `fontPolicy="pinned"`, a warning under
/// `system`). `TXT-FIT` is information when the text is drawn whole inside the frame, and a warning when it is cut
/// (clipped to its box, or out of the frame).
fn text_fit_findings(ev: &Evaluator, times: &[f64], pinned: bool) -> Vec<crate::render_report::Finding> {
    use crate::render_report::{code, Finding};
    let audit = sr_gpu::text_audit::check(ev, times);
    let sampled = audit.note.map(|n| format!(" ({n})")).unwrap_or_default();
    let mut out = Vec::new();
    for f in audit.found {
        let at = |mut x: Finding| {
            x.at = Some(crate::render_report::At { offset: Some(f.loc.offset), id: Some(f.id.clone()) });
            x
        };
        // text drawn whole inside the frame loses nothing: information, which --strict does not count; text clipped
        // to its box or reaching out of the frame is cut: a warning
        match (&f.lost, &f.overflow) {
            (Some(l), _) => out.push(at(Finding::node(
                code::TXT_FIT,
                sr_model::Severity::Warning,
                &f.id,
                format!(
                    "text {:?} is cut: it reaches {:.1} px past its box (clipped) or out of the frame{sampled}",
                    f.id, l.worst
                ),
            )
            .at_time(l.time[0], l.time[1])
            .measuring(l.worst, Some(0.0), "px"))),
            (None, Some(s)) => out.push(at(Finding::node(
                code::TXT_FIT,
                sr_model::Severity::Info,
                &f.id,
                format!(
                    "text {:?} reaches {:.1} px past its box at the size drawn, drawn whole inside the frame{sampled}",
                    f.id, s.worst
                ),
            )
            .at_time(s.time[0], s.time[1])
            .measuring(s.worst, Some(0.0), "px"))),
            (None, None) => {}
        }
        if let Some(s) = &f.dropped {
            out.push(at(Finding::node(
                code::TXT_CUT,
                sr_model::Severity::Warning,
                &f.id,
                format!("text {:?}: {} characters are not drawn (maxLines or overflow){sampled}", f.id, s.worst),
            )
            .at_time(s.time[0], s.time[1])
            .measuring(s.worst, Some(0.0), "characters")));
        }
        let glyph_severity = if pinned { sr_model::Severity::Error } else { sr_model::Severity::Warning };
        for (c, t) in &f.missing {
            let policy = if pinned { "permitted by fontPolicy=\"pinned\"" } else { "available" };
            out.push(at(Finding::node(
                code::FONT_GLYPH,
                glyph_severity,
                &f.id,
                format!(
                    "text {:?}: no face {policy} has U+{:04X} {c:?}; it is drawn as .notdef{sampled}",
                    f.id, *c as u32
                ),
            )
            .at_time(*t, *t)
            .measuring(*c as u32 as f64, None, "code point")));
        }
        for (s, t) in &f.substituted {
            out.push(at(Finding::node(
                code::FONT_SUB,
                sr_model::Severity::Warning,
                &f.id,
                format!("text {:?}: {} is drawn with {}{sampled}", f.id, s.requested, s.drawn),
            )
            .at_time(*t, *t)));
        }
    }
    out
}

/// Whether `output` can render as time chunks joined without re-encoding: a single-pass video file whose frames
/// render the same from a cold start (no simulation). The accessibility checks compare frames across chunks and do
/// not prevent it: the chunks' observations are judged together, in frame order.
fn splittable(output: &m::Output, codec: Codec, ev: &Evaluator) -> bool {
    !(output.two_pass
        || output.max_file_size.is_some()
        || codec.is_sequence()
        || codec.is_audio_only()
        || matches!(codec, Codec::Gif | Codec::Apng | Codec::Webp)
        || ev.has_simulation())
}

/// How many chunks this machine renders at once under `--parallel auto`: 3 with a hardware encoder, otherwise half
/// the cores, at most 4. It sets concurrency only, never the split ([`segment_plan`]).
fn host_workers(codec: Codec, opts: &Options) -> usize {
    // measured on a 229 s programme: NVENC levels off at 3, libx264 (CPU-bound) at 4 on 8 cores
    let hardware = sr_media::encode::choose_encoder(codec, opts.hardware)
        .is_ok_and(|e| ["_nvenc", "_videotoolbox", "_qsv", "_amf", "_vaapi"].iter().any(|s| e.ends_with(s)));
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    if hardware {
        3
    } else {
        (cores / 2).clamp(1, 4)
    }
}

/// The time split of a video output and how many of its chunks render at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SegmentPlan {
    /// Contiguous chunks, each encoded from a keyframe and joined without re-encoding (1: one stream, serially).
    chunks: usize,
    /// Chunks rendered at once.
    workers: usize,
}

impl SegmentPlan {
    /// The first frame of each chunk, then the frame count.
    fn bounds(&self, frames: u64) -> Vec<usize> {
        (0..=self.chunks).map(|i| (i as u64 * frames / self.chunks as u64) as usize).collect()
    }
}

/// Splits `frames` frames lasting `duration` seconds. Under `auto` the split depends on the programme alone, so an
/// output is byte-for-byte the same file on any machine: one chunk per 10 s, at most 12 (with the flash or contrast
/// checks, also at most [`OBSERVATION_FRAMES`] frames each), and one stream under 20 s. `host` chunks render at
/// once there, at most one per 20 s. An explicit count renders that many at once, each over three chunks of at least
/// 10 s (`1` renders serially). `can_split` is false for outputs that render as one stream ([`splittable`]).
fn segment_plan(
    parallel: Parallel,
    can_split: bool,
    host: usize,
    duration: f64,
    frames: u64,
    observe: bool,
) -> SegmentPlan {
    let serial = SegmentPlan { chunks: 1, workers: 1 };
    if !can_split || frames < 2 {
        return serial;
    }
    let frames_cap = frames.min(usize::MAX as u64) as usize;
    let tens = (duration / 10.0).floor().max(0.0) as usize;
    let observed = |chunks: usize| if observe { chunks.max(frames_cap.div_ceil(OBSERVATION_FRAMES)) } else { chunks };
    match parallel {
        Parallel::Auto => {
            let chunks = tens.clamp(1, 12).min(frames_cap);
            if chunks < 2 {
                return serial;
            }
            let chunks = observed(chunks).min(frames_cap);
            // every worker pays the renderer's start-up: keep each one busy for at least 20 s
            let workers = host.min((duration / 20.0).floor() as usize).min(chunks).max(1);
            SegmentPlan { chunks, workers }
        }
        Parallel::Count(n) => {
            let workers = (n as usize).min(frames_cap).max(1);
            if workers < 2 {
                return serial;
            }
            let chunks = observed((workers * 3).min(tens).max(workers)).min(frames_cap);
            SegmentPlan { chunks, workers }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn base_spec(
    o: &m::Output,
    path: &Path,
    codec: Codec,
    size: [u32; 2],
    fps: f64,
    input: InputFormat,
    color: &OutputColor,
    audio: Option<(PathBuf, String, u64)>,
    p: &sr_eval::Program,
    opts: &Options,
) -> EncodeSpec {
    let (primaries, transfer, matrix) = color.ffmpeg_tags();
    EncodeSpec {
        path: path.to_path_buf(),
        codec,
        container: o.container.and_then(|c| Container::parse(c.as_str())),
        width: size[0],
        height: size[1],
        fps,
        input,
        pixel_format: o.pixel_format.clone(),
        preset: o.preset.clone(),
        profile: o.profile.clone(),
        level: o.level.clone(),
        prores_profile: o.prores_profile.map(|p| p.as_str().to_string()),
        crf: o.crf as u32,
        bitrate: o.bitrate,
        max_bitrate: o.max_bitrate,
        buffer_size: o.buffer_size,
        pass: None,
        keyframe_interval: o.keyframe_interval.get(),
        b_frames: o.b_frames.map(|b| b as u32),
        faststart: o.faststart,
        alpha: o.alpha,
        audio,
        loop_count: o.loop_count as u32,
        start_number: 0,
        color: ColorTags { primaries, transfer, matrix, full_range: color.full_range },
        hdr: Hdr {
            max_cll: o.max_cll.map(|v| v as u32),
            max_fall: o.max_fall.map(|v| v as u32),
            mastering_display: o.mastering_display.clone(),
        },
        metadata: if o.embed_metadata { metadata(&p.scene) } else { Vec::new() },
        chapters: None,
        hardware: opts.hardware,
        tags: sr_media::encode::StreamTags {
            sar: sr_media::encode::StreamTags::sar_of(p.scene.project.pixel_aspect.get()),
            timecode: p.scene.project.timecode_start.map(|tc| tc.to_string()),
            ..Default::default()
        },
        audio_bits: p.scene.audio_mix.as_ref().map(|a| a.bit_depth.as_str().parse().unwrap_or(24)).unwrap_or(24),
    }
}

/// File name of frame `k` of a printf-patterned sequence path.
pub fn sequence_name(pattern: &Path, k: u64) -> String {
    // the pattern is in the file name: a `%` in a directory name is just that
    let s = pattern.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
    if let Some(i) = s.find('%') {
        let rest = &s[i + 1..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if rest[digits.len()..].starts_with('d') {
            let width: usize = digits.trim_start_matches('0').parse().unwrap_or(0);
            let name = format!("{}{k:0width$}{}", &s[..i], &rest[digits.len() + 1..]);
            return pattern.with_file_name(name).to_string_lossy().into_owned();
        }
    }
    pattern.to_string_lossy().into_owned()
}

/// Decodes the intermediate and feeds it to an encoder for one pass.
fn replay(
    inter: &Path,
    spec: &EncodeSpec,
    format: InputFormat,
    size: [u32; 2],
    frames: u64,
) -> Result<String, DeliverError> {
    use std::io::Read;
    let pix = match format {
        InputFormat::Nv12 => "nv12",
        InputFormat::P010 => "p010le",
        InputFormat::Rgba8 => "rgba",
        InputFormat::Rgba16 => "rgba64le",
        InputFormat::Gbrapf32 => "gbrapf32le",
    };
    let mut dec = std::process::Command::new(sr_media::ffmpeg())
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(inter)
        .args(["-f", "rawvideo", "-pix_fmt", pix, "pipe:1"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| sr_media::MediaError::Spawn { tool: sr_media::ffmpeg(), source: e })?;
    let Some((mut out, mut err)) = crate::child::take_or_reap(&mut dec, |c| c.stdout.take().zip(c.stderr.take()))
    else {
        return Err(DeliverError::Invalid("ffmpeg was started without its output pipes".into()));
    };
    let said = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = err.read_to_end(&mut v);
        v
    });
    let mut buf = vec![0u8; format.frame_bytes(size[0], size[1])];
    let fed = Encoder::start(spec).map_err(DeliverError::from).and_then(|mut enc| {
        let mut n = 0u64;
        while out.read_exact(&mut buf).is_ok() {
            enc.write(&buf)?;
            n += 1;
        }
        Ok((enc, n))
    });
    // the decoder is reaped whatever became of the encoder
    if fed.is_err() {
        let _ = dec.kill();
    }
    let status = dec.wait()?;
    let said = sr_media::reason(&said.join().unwrap_or_default());
    let (enc, n) = fed?;
    if !status.success() || n != frames {
        // the encoder is dropped unfinished: a pass fed part of the frames is no output
        return Err(sr_media::MediaError::Failed {
            tool: "ffmpeg".into(),
            path: inter.display().to_string(),
            message: format!(
                "the intermediate decoded to {n} of {frames} frames{}{said}",
                if said.is_empty() { "" } else { ": " }
            ),
        }
        .into());
    }
    let name = enc.encoder.clone();
    enc.finish()?;
    Ok(name)
}

/// Caption sidecars: tracks listed in the output's `captions`, or with mode
/// sidecar or both, written next to the output as `<stem>.<track>.<language>.vtt`
/// (`.srt` when the track's source is SRT), with times relative to the range start.
/// Warnings for segments whose span begins or ends where nothing is drawn above the background.
fn empty_ends(ev: &Evaluator, tm: &crate::segments::TimeMap) -> Vec<String> {
    let fps = ev.program().fps.as_f64();
    let drawn = |c: f64| {
        ev.evaluate(c).nodes.iter().any(|n| {
            n.draw
                && !n.is_matte
                && n.world_opacity > 1e-3
                && !matches!(n.kind, "group" | "camera" | "instance" | "repeat" | "transition" | "null")
        })
    };
    let mut out = Vec::new();
    for (i, seg) in tm.segments.iter().enumerate() {
        let name = seg.elem.id.clone().unwrap_or_else(|| format!("segment {}", i + 1));
        let last = (seg.duration - 1.0 / fps).max(0.0);
        for (edge, u) in [("starts", 0.0), ("ends", last)] {
            let c = tm.composition(i, u);
            if !drawn(c) {
                out.push(format!("{name} {edge} at {c:.3} s, where nothing is drawn above the background"));
            }
        }
    }
    out
}

/// The output's chapters (markers of kind `chapter` at the first output time each maps to; those in
/// skipped spans are dropped), cut to `start`..`end`, as an FFmetadata file in `dir`.
fn write_chapters(
    p: &sr_eval::Program,
    tm: &crate::segments::TimeMap,
    start: f64,
    end: f64,
    dir: &Path,
) -> Result<Option<PathBuf>, DeliverError> {
    let mut marks: Vec<(f64, Option<f64>, String)> = p
        .scene
        .markers
        .iter()
        .flat_map(|mk| &mk.children)
        .filter_map(|c| match c {
            m::MarkersChild::Marker(mk) if mk.kind.as_str() == "chapter" => {
                let t = tm.output_of(mk.time)?;
                let stop = (mk.duration.get() > 0.0).then(|| {
                    // Duration is in composition seconds. Reuse the caption interval mapper so
                    // speed changes, skipped spans, holds and reverse remaps have one definition.
                    let mut intervals: Vec<_> = (0..tm.segments.len())
                        .flat_map(|i| {
                            crate::captions::CueMap::new(tm, i)
                                .intervals(mk.time, mk.time + mk.duration.get())
                                .into_iter()
                                .map(move |(a, b)| (tm.segments[i].start + a, tm.segments[i].start + b))
                        })
                        .collect();
                    intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
                    let mut stop = t;
                    for (a, b) in intervals {
                        if a > stop + 1e-7 {
                            break;
                        }
                        if b > stop {
                            stop = b;
                        }
                    }
                    stop
                });
                Some((t, stop, mk.label.clone().or_else(|| mk.id.clone()).unwrap_or_default()))
            }
            _ => None,
        })
        .filter(|(t, _, _)| *t >= start - 1e-9 && *t < end)
        .collect();
    if marks.is_empty() {
        return Ok(None);
    }
    marks.sort_by(|a, b| a.0.total_cmp(&b.0));
    let ms = |t: f64| ((t - start) * 1000.0).round().max(0.0) as u64;
    let esc = |s: &str| {
        s.chars().fold(String::new(), |mut o, c| {
            if matches!(c, '=' | ';' | '#' | '\\' | '\n') {
                o.push('\\');
            }
            o.push(c);
            o
        })
    };
    let mut text = String::from(";FFMETADATA1\n");
    for (k, (t, stop, title)) in marks.iter().enumerate() {
        let next = marks.get(k + 1).map_or(end, |m| m.0).min(end);
        let next = stop.map_or(next, |stop| stop.min(next));
        text.push_str(&format!(
            "[CHAPTER]\nTIMEBASE=1/1000\nSTART={}\nEND={}\ntitle={}\n",
            ms(*t),
            ms(next),
            esc(title)
        ));
    }
    let path = dir.join("chapters.txt");
    std::fs::write(&path, text)?;
    Ok(Some(path))
}

/// Writes the caption files of `tracks` next to `out`: tracks the output lists in `captions`, and
/// every track that is not burn-only, cut to `start`..`end` and starting at `start`.
fn write_sidecars(
    tracks: &[m::CaptionTrack],
    base: &Path,
    output: &m::Output,
    out: &std::path::Path,
    start: f64,
    end: f64,
) -> Result<Vec<PathBuf>, DeliverError> {
    let mut files = Vec::new();
    for tr in tracks {
        let listed = output.captions.as_ref().is_some_and(|ids| ids.contains(&tr.id));
        let mode = tr.mode.to_string();
        if !listed && mode == "burn" {
            continue;
        }
        let cues = sr_gpu::text::track_cues(tr, base).map_err(DeliverError::Invalid)?;
        let cues: Vec<sr_text::captions::Cue> = cues
            .into_iter()
            .filter(|c| c.end > start && c.start < end)
            .map(|mut c| {
                c.start = (c.start - start).max(0.0);
                c.end = (c.end.min(end) - start).max(c.start);
                for w in &mut c.words {
                    w.start = (w.start - start).max(0.0);
                    w.end = (w.end - start).max(w.start);
                }
                c
            })
            .collect();
        let srt = tr.format.as_ref().map(|f| f.to_string()) == Some("srt".into());
        let stem = out.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
        let kind = tr.kind.as_str();
        let suffix = if srt && matches!(kind, "sdh" | "forced") { format!(".{kind}") } else { String::new() };
        let name = format!("{stem}.{}.{}{suffix}.{}", tr.id, tr.language, if srt { "srt" } else { "vtt" });
        let path = out.with_file_name(name);
        let text = if srt {
            sr_text::captions::to_srt(&cues)
        } else {
            // NOTE is standard WebVTT syntax. These are informational fields, not cue settings
            // or nonstandard WebVTT header directives. Keep default sidecars byte-identical.
            let mut text = sr_text::captions::to_vtt(&cues);
            if tr.label.is_some() || kind != "captions" {
                let label = tr.label.as_deref().unwrap_or("").replace(['\r', '\n'], " ").replace("-->", "→");
                text.insert_str(
                    "WEBVTT\n\n".len(),
                    &format!("NOTE\nKind: {kind}\nLanguage: {}\nLabel: {label}\n\n", tr.language),
                );
            }
            text
        };
        std::fs::write(&path, text)?;
        files.push(path);
    }
    Ok(files)
}

/// What a frame's 3D pass measured for the render report (`t` in composition seconds): which objects it drew and whether
/// they were in the camera's view, and its metals without an environment.
fn seen_in_3d(report: &mut Report, stats: &sr_gpu::RenderStats, t: f64) {
    for (id, inside) in &stats.objects3d_in_view {
        merge_seen(&mut report.objects3d_seen, id.clone(), ([t, t], *inside));
    }
    for (id, metallic) in &stats.metal_without_environment {
        merge_metal(&mut report.metal_without_environment, id.clone(), (*metallic, [t, t]));
    }
}

fn merge_seen(
    into: &mut std::collections::BTreeMap<String, ([f64; 2], bool)>,
    id: String,
    (at, inside): ([f64; 2], bool),
) {
    let e = into.entry(id).or_insert((at, inside));
    *e = ([e.0[0].min(at[0]), e.0[1].max(at[1])], e.1 || inside);
}

fn merge_metal(into: &mut std::collections::BTreeMap<String, (f32, [f64; 2])>, id: String, (m, at): (f32, [f64; 2])) {
    let e = into.entry(id).or_insert((m, at));
    *e = (e.0.max(m), [e.1[0].min(at[0]), e.1[1].max(at[1])]);
}

/// The render report's 3D findings, from what the frames' 3D passes measured: an `object3D` never inside the camera's
/// view in any frame it was drawn in, and one drawn with a metallic material where the pass had no environment image.
/// Warnings, with the same text among the delivery's warnings.
fn three_d_findings(report: &mut Report) {
    use crate::render_report::{code, Finding};
    let mut found = Vec::new();
    for (id, ([from, to], inside)) in &report.objects3d_seen {
        if !inside {
            let msg = format!(
                "object3D {id} is drawn from {from:.3} s to {to:.3} s and is never inside the camera's view: its bounds \
                 lie wholly behind, beside or beyond the camera's frustum in every frame rendered, so only its shadows \
                 or reflections can show"
            );
            found.push(
                Finding::node(code::engine("3D-OFF-CAMERA"), sr_model::Severity::Warning, id, msg).at_time(*from, *to),
            );
        }
    }
    for (id, (metallic, [from, to])) in &report.metal_without_environment {
        let limit = sr_gpu::three::METALLIC_NEEDS_ENVIRONMENT;
        let msg = format!(
            "object3D {id} has a metallic material ({metallic:.2}, at least {limit}) and its 3D pass has no environment \
             image: a metal shows what it reflects, and with nothing to reflect it shades as a uniform colour (flat \
             grey under the default lights). Add a dome light with an environment image, or lower metallic below {limit}"
        );
        found.push(
            Finding::node(code::engine("3D-METAL-NO-ENVIRONMENT"), sr_model::Severity::Warning, id, msg)
                .at_time(*from, *to)
                .measuring(*metallic as f64, Some(limit as f64), "metallic"),
        );
    }
    report.warnings.extend(found.iter().map(|f| format!("{}: {}", f.code, f.message)));
    report.findings.extend(found);
}

/// What a frame's stateful effects add to the report (SREPs 67 and 68).
fn stateful(report: &mut Report, stats: &sr_gpu::RenderStats) {
    report.checkpoint_hits += stats.checkpoint_hits;
    report.checkpoint_misses += stats.checkpoint_misses;
    for (who, n) in &stats.iterate_steps {
        let e = report.iterate_steps.entry(who.clone()).or_insert([*n, *n]);
        *e = [e[0].min(*n), e[1].max(*n)];
    }
}

/// Most with/without renders spent measuring one text's contrast.
const CONTRAST_PROBES: usize = 32;

/// Predictions only save work: the final schedule still comes from rendered observations.
#[derive(Default)]
struct ContrastPrefetch {
    planned: std::collections::BTreeMap<u64, Vec<sr_gpu::ContrastTarget>>,
    measured: std::collections::BTreeMap<(u64, sr_gpu::ContrastTarget), Option<f64>>,
}

impl ContrastPrefetch {
    fn prepare(
        &mut self,
        times: &[f64],
        predicted: std::collections::BTreeMap<sr_gpu::ContrastTarget, Vec<(usize, f64)>>,
    ) {
        self.planned.clear();
        self.measured.clear();
        for (k, targets) in contrast_schedule(predicted) {
            self.planned.entry(times[k].to_bits()).or_default().extend(targets.into_iter().map(|(id, _)| id));
        }
    }

    fn resolve(
        &mut self,
        time: f64,
        targets: &[sr_gpu::ContrastTarget],
        mut fallback: impl FnMut(&[sr_gpu::ContrastTarget]) -> Vec<Option<f64>>,
    ) -> Result<Vec<Option<f64>>, DeliverError> {
        let missing: Vec<_> = targets
            .iter()
            .filter(|id| !self.measured.contains_key(&(time.to_bits(), (*id).clone())))
            .cloned()
            .collect();
        let found = if missing.is_empty() { Vec::new() } else { fallback(&missing) };
        if found.len() != missing.len() {
            return Err(DeliverError::Invalid(format!(
                "contrastCheck at {time:.3} s: {} measurements for {} texts",
                found.len(),
                missing.len()
            )));
        }
        let mut found = found.into_iter();
        Ok(targets
            .iter()
            .map(|id| match self.measured.get(&(time.to_bits(), id.clone())) {
                Some(&ratio) => ratio,
                None => found.next().flatten(),
            })
            .collect())
    }
}

/// Most probes of one output's contrast check, over all its texts. Each probe renders its frame once more without
/// the text (and twice more when the text and its backdrop have the same colour), so the check's cost is bounded by
/// this, or by the number of texts when there are more: each text is measured at least once.
const CONTRAST_BUDGET: usize = 256;

/// How many probes each of `texts` texts gets: [`CONTRAST_PROBES`] while the budget allows, else an equal share of
/// [`CONTRAST_BUDGET`], and at least one.
fn probe_allowance(texts: usize) -> usize {
    (CONTRAST_BUDGET / texts.max(1)).clamp(1, CONTRAST_PROBES)
}

/// The (frame, opacity) observations of a text at its most visible.
fn at_top(seen: &[(usize, f64)]) -> Vec<(usize, f64)> {
    let top = seen.iter().map(|s| s.1).fold(f64::MIN, f64::max);
    seen.iter().filter(|s| s.1 >= top - 1e-3).copied().collect()
}

/// The frames to measure a text's contrast at, of the (frame, opacity) it was seen in: those where it is at its most
/// visible, all of them up to [`CONTRAST_PROBES`], else that many spread evenly from the first to the last, so a title
/// held for minutes does not render every one of its frames twice more. Under a smaller `limit`, the first `limit` of
/// those in coarse-to-fine order (both ends, then the middle, then the quarters, ...): the frames for a smaller limit
/// are always among those for a larger one, so a prediction made with another limit still saves its renders.
fn probe_frames(seen: &[(usize, f64)], limit: usize) -> Vec<(usize, f64)> {
    let at_top = at_top(seen);
    let full: Vec<(usize, f64)> = if at_top.len() <= CONTRAST_PROBES {
        at_top
    } else {
        (0..CONTRAST_PROBES).map(|i| at_top[i * (at_top.len() - 1) / (CONTRAST_PROBES - 1)]).collect()
    };
    if full.len() <= limit {
        return full;
    }
    let mut keep = coarse_to_fine(full.len());
    keep.truncate(limit);
    keep.sort_unstable();
    keep.into_iter().map(|i| full[i]).collect()
}

/// The indices `0..n` in coarse-to-fine order: `0` and `n - 1`, then the middle of each gap, breadth first.
fn coarse_to_fine(n: usize) -> Vec<usize> {
    let mut order: Vec<usize> = match n {
        0 => return Vec::new(),
        1 => return vec![0],
        _ => vec![0, n - 1],
    };
    let mut gaps = std::collections::VecDeque::from([(0, n - 1)]);
    while let Some((a, b)) = gaps.pop_front() {
        if b - a < 2 {
            continue;
        }
        let m = (a + b) / 2;
        order.push(m);
        gaps.push_back((a, m));
        gaps.push_back((m, b));
    }
    order
}

/// The same selected observations, grouped in time order without dropping any target; every target shares the
/// output's [`CONTRAST_BUDGET`].
fn contrast_schedule(
    unprobed: std::collections::BTreeMap<sr_gpu::ContrastTarget, Vec<(usize, f64)>>,
) -> std::collections::BTreeMap<usize, Vec<(sr_gpu::ContrastTarget, f64)>> {
    let limit = probe_allowance(unprobed.len());
    let mut schedule = std::collections::BTreeMap::<_, Vec<_>>::new();
    for (id, seen) in unprobed {
        for (frame, opacity) in probe_frames(&seen, limit) {
            schedule.entry(frame).or_default().push((id.clone(), opacity));
        }
    }
    schedule
}

/// The texts the budget measures at fewer frames than [`CONTRAST_PROBES`] would have: (text, probes, frames at its
/// most visible). The report says so, since a backdrop that changes between two probes is then not seen.
fn thinned_contrasts(
    unprobed: &std::collections::BTreeMap<sr_gpu::ContrastTarget, Vec<(usize, f64)>>,
) -> Vec<(sr_gpu::ContrastTarget, usize, usize)> {
    let limit = probe_allowance(unprobed.len());
    unprobed
        .iter()
        .filter_map(|(id, seen)| {
            let of = at_top(seen).len();
            let got = of.min(limit);
            (got < of.min(CONTRAST_PROBES)).then(|| (id.clone(), got, of))
        })
        .collect()
}

/// Takes the mix's `master` and `nodes` over as the shaders' audio, keeping only the channels in `read` (every one when
/// `None`); nothing when no shader reads any. What is not kept is released here rather than held for the render.
fn shader_signals(
    master: &mut sr_audio::loudness::Planar,
    nodes: &mut std::collections::HashMap<String, sr_audio::loudness::Planar>,
    rate: f64,
    read: Option<&std::collections::BTreeSet<String>>,
) -> Option<std::sync::Arc<sr_gpu::shader::AudioSignals>> {
    let master = std::mem::take(master);
    let mut channels: std::collections::HashMap<String, std::sync::Arc<sr_audio::loudness::Planar>> =
        std::mem::take(nodes)
            .into_iter()
            .chain([("master".to_string(), master)])
            .filter(|(k, _)| read.is_none_or(|r| r.contains(k)))
            .map(|(k, v)| (k, std::sync::Arc::new(v)))
            .collect();
    channels.shrink_to_fit();
    // a shader reading only a channel the mix lacks still finds the mix, and reports the missing channel
    read.is_none_or(|r| !r.is_empty()).then(|| std::sync::Arc::new(sr_gpu::shader::AudioSignals { rate, channels }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workers_divide_the_frame_queue_bytes_and_keep_a_few_frames() {
        assert_eq!(frame_queue(1_000_000, 1), 48);
        assert_eq!(frame_queue(1_000_000, 4), 24);
        assert_eq!(frame_queue(8_294_400, 1), 11);
        assert_eq!(frame_queue(8_294_400, 2), 5);
        assert_eq!(frame_queue(8_294_400, 8), FRAME_QUEUE_MIN);
        assert_eq!(frame_queue(8_294_400, 0), 11);
    }

    #[test]
    fn shaders_take_over_only_the_mixed_channels_they_read() {
        let take = |read: Option<&[&str]>| {
            let mut master = vec![vec![1.0f32; 4]];
            let mut nodes: std::collections::HashMap<String, sr_audio::loudness::Planar> =
                [("music".to_string(), vec![vec![2.0; 4]]), ("voice".to_string(), vec![vec![3.0; 4]])].into();
            let read: Option<std::collections::BTreeSet<String>> =
                read.map(|r| r.iter().map(|n| n.to_string()).collect());
            let signals = shader_signals(&mut master, &mut nodes, 48000.0, read.as_ref());
            // what the shaders do not take is released with the mix, not held for the render
            assert!(master.is_empty() && nodes.is_empty());
            signals.map(|s| {
                assert_eq!(s.rate, 48000.0);
                let mut kept: Vec<(String, f32)> = s.channels.iter().map(|(k, v)| (k.clone(), v[0][0])).collect();
                kept.sort_by(|a, b| a.0.cmp(&b.0));
                kept
            })
        };
        let all = vec![("master".to_string(), 1.0), ("music".to_string(), 2.0), ("voice".to_string(), 3.0)];
        assert_eq!(take(None), Some(all));
        assert_eq!(take(Some(&["music"])), Some(vec![("music".to_string(), 2.0)]));
        assert_eq!(take(Some(&["master", "ghost"])), Some(vec![("master".to_string(), 1.0)]));
        assert_eq!(take(Some(&["ghost"])), Some(vec![]));
        assert_eq!(take(Some(&[])), None);
    }

    #[test]
    fn the_wait_for_workers_ends_as_the_last_one_returns() {
        // a tick far longer than any test: the wait must end on the workers, not on the clock
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let workers: Vec<_> = (0..3).map(|_| tx.clone()).map(|t| std::thread::spawn(move || drop(t))).collect();
        drop(tx);
        let mut ticks = 0;
        wait_for_workers(&rx, std::time::Duration::from_secs(3600), || ticks += 1);
        assert_eq!(ticks, 0);
        workers.into_iter().for_each(|w| w.join().unwrap());
    }

    #[test]
    fn the_wait_for_workers_reports_progress_while_one_still_runs() {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let (go, gone) = std::sync::mpsc::channel::<()>();
        let worker = std::thread::spawn(move || {
            gone.recv().unwrap();
            drop(tx);
        });
        let mut ticks = 0;
        wait_for_workers(&rx, std::time::Duration::from_millis(1), || {
            ticks += 1;
            let _ = go.send(());
        });
        assert!(ticks >= 1, "progress is reported while a worker runs");
        worker.join().unwrap();
    }

    #[test]
    fn a_worker_that_panics_still_ends_the_wait() {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let worker = std::thread::spawn(move || {
            let _held = tx;
            panic!("a segment render panicked");
        });
        wait_for_workers(&rx, std::time::Duration::from_secs(3600), || {});
        assert!(worker.join().is_err());
    }

    fn a1_document(extra: &str) -> sr_model::Document {
        let (before, after) = extra.find("<captions>").map_or((extra, ""), |i| extra.split_at(i));
        sr_model::load_str(&format!(r#"<scene version="1.2"><project width="64" height="36" fps="25" duration="10" pixelAspect="1.2" timecodeStart="01:00:00:00"/>{before}<composition/>{after}</scene>"#), &sr_model::LoadOptions::without_assets()).unwrap()
    }

    #[test]
    fn a1_metadata_explicit_fields_win_and_dates_are_authored() {
        let d = a1_document(
            r#"<metadata title="Title" revision="r7" created="2020-01-02T03:04:05Z" modified="2021-02-03T04:05:06Z" generator="Authoring tool" language="pt-BR" description="Editorial"><meta name="TITLE" value="wrong"/><meta name="custom" value="kept"/><accessibility description="Text alternative"/></metadata>"#,
        );
        let tags: std::collections::BTreeMap<_, _> = metadata(&d.scene).into_iter().collect();
        for (key, val) in [
            ("title", "Title"),
            ("version", "r7"),
            ("creation_time", "2020-01-02T03:04:05Z"),
            ("date", "2021-02-03T04:05:06Z"),
            ("language", "pt-BR"),
            ("comment", "Editorial"),
            ("synopsis", "Text alternative"),
            ("custom", "kept"),
            ("generator", "Authoring tool"),
        ] {
            assert_eq!(tags.get(key).map(String::as_str), Some(val), "{key}");
        }
        assert!(!tags.contains_key("TITLE"));
        assert!(!metadata(&a1_document("").scene).iter().any(|(k, _)| k == "creation_time" || k == "date"));
    }

    #[test]
    fn a1_project_stream_tags_are_independent_of_embed_metadata() {
        let d = a1_document("");
        let ev = Evaluator::new(&d, &EvalOptions::default()).unwrap();
        let mut o = adhoc_output("x.mov", "h264").unwrap();
        o.embed_metadata = false;
        let color = OutputColor::new(m::ColorSpace::Srgb, m::Transfer::Auto, false);
        let spec = base_spec(
            &o,
            Path::new("x.mov"),
            Codec::H264,
            [64, 36],
            25.0,
            InputFormat::Rgba8,
            &color,
            None,
            ev.program(),
            &Options::default(),
        );
        assert_eq!(spec.tags.sar, Some((6, 5)));
        assert_eq!(spec.tags.timecode.as_deref(), Some("01:00:00:00"));
        assert!(spec.metadata.is_empty());
    }

    #[test]
    fn a1_chapter_duration_caps_end_after_a_skipped_span() {
        let d = a1_document(
            r#"<markers><marker time="0" kind="chapter" label="Skipped"/><marker time="3" duration="0.5" kind="chapter" label="Short"/><marker time="5" kind="chapter" label="Next"/></markers>"#,
        );
        let ev = Evaluator::new(&d, &EvalOptions::default()).unwrap();
        let tm = crate::segments::TimeMap::span_of(ev.program(), 2.0, 8.0).unwrap();
        let dir = std::env::temp_dir().join(format!("a1-chapters-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = write_chapters(ev.program(), &tm, 0.0, 6.0, &dir).unwrap().unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.contains("START=1000\nEND=1500\ntitle=Short"), "{text}");
        assert!(!text.contains("Skipped"));
        assert!(text.contains("START=3000\nEND=6000\ntitle=Next"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a1_caption_sidecars_preserve_track_identity() {
        let d = a1_document(
            r#"<captions><captionTrack id="cc" language="pt-BR" label="Português" kind="sdh" mode="sidecar"><cue start="0" end="1" text="Hello"/></captionTrack></captions>"#,
        );
        let dir = std::env::temp_dir().join(format!("a1-sidecars-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut tracks = d.scene.captions.unwrap().caption_tracks;
        let o = adhoc_output("x.mp4", "h264").unwrap();
        let paths = write_sidecars(&tracks, &dir, &o, &dir.join("x.mp4"), 0.0, 1.0).unwrap();
        let vtt = std::fs::read_to_string(&paths[0]).unwrap();
        assert!(vtt.contains("NOTE\nKind: sdh\nLanguage: pt-BR\nLabel: Português\n\n"), "{vtt}");
        tracks[0].format = Some(m::CaptionTrackFormat::Srt);
        let paths = write_sidecars(&tracks, &dir, &o, &dir.join("x.mp4"), 0.0, 1.0).unwrap();
        assert!(paths[0].ends_with("x.cc.pt-BR.sdh.srt"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a1_chapter_duration_follows_speed_and_contiguous_mapped_spans() {
        for (segments, expected) in [
            (r#"<segment from="0" to="6" speed="2"/>"#, "START=500\nEND=1500"),
            (r#"<segment from="0" to="2"/><segment from="2.5" to="6"/>"#, "START=1000\nEND=2500"),
        ] {
            let d = a1_document(&format!(
                r#"<output path="a.mkv" codec="h264">{segments}</output><markers><marker time="1" duration="2" kind="chapter" label="Span"/></markers>"#
            ));
            let ev = Evaluator::new(&d, &EvalOptions::default()).unwrap();
            let tm = crate::segments::TimeMap::of(ev.program(), &d.scene.outputs[0]).unwrap().unwrap();
            let dir = std::env::temp_dir().join(format!("a1-chapter-speed-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = write_chapters(ev.program(), &tm, 0.0, tm.duration, &dir).unwrap().unwrap();
            let text = std::fs::read_to_string(path).unwrap();
            assert!(text.contains(expected), "{text}");
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    /// Graphs evaluated ahead arrive in order, and a body that stops early releases the helper.
    #[test]
    fn graphs_evaluated_ahead_arrive_in_order() {
        let times = [0.0, 0.5, 1.0, 1.5];
        let got = evaluated(&times, |t| t * 2.0, |next| times.iter().map(|&t| next(t)).collect::<Result<Vec<_>, _>>());
        assert_eq!(got.unwrap(), vec![0.0, 1.0, 2.0, 3.0]);
        let first = evaluated(&times, |t| t, |next| next(0.0));
        assert_eq!(first.unwrap(), 0.0);
    }

    /// A panic while evaluating ahead (a bug in sr-eval) is an error of the delivery that names it, not a panic
    /// re-raised when the thread scope ends.
    #[test]
    fn a_panic_while_evaluating_ahead_is_an_error() {
        let times = [0.0, 0.5, 1.0];
        let result = evaluated(
            &times,
            |t| if t > 0.25 { panic!("cannot evaluate {t}") } else { t },
            |next| times.iter().map(|&t| next(t)).collect::<Result<Vec<_>, _>>(),
        );
        match result {
            Err(DeliverError::Invalid(m)) => assert_eq!(m, "evaluation panicked: cannot evaluate 0.5"),
            other => panic!("expected an evaluation error, got {other:?}"),
        }
    }

    #[test]
    fn panic_messages_are_read_from_either_payload() {
        assert_eq!(panic_message(&"literal"), "literal");
        assert_eq!(panic_message(&String::from("formatted")), "formatted");
        assert_eq!(panic_message(&7_u8), "no message");
    }

    /// `--parallel auto` splits an output by the scene alone, so the file is the same on every machine (v0.2.1
    /// pinned the encoders' threads for the same reason): the host only decides how many chunks render at once.
    #[test]
    fn the_automatic_split_depends_on_the_scene_not_on_the_host() {
        for (duration, fps, observe) in
            [(5.0, 30.0, false), (19.9, 30.0, false), (20.0, 30.0, false), (45.0, 24.0, false), (229.0, 60.0, false)]
                .into_iter()
                .chain([(45.0, 24.0, true), (229.0, 60.0, true), (600.0, 30.0, false)])
        {
            let frames = (duration * fps) as u64;
            let plans: Vec<SegmentPlan> =
                (1..=16).map(|host| segment_plan(Parallel::Auto, true, host, duration, frames, observe)).collect();
            for p in &plans {
                assert_eq!(p.chunks, plans[0].chunks, "{duration} s: the split must not follow the host: {plans:?}");
                assert_eq!(p.bounds(frames), plans[0].bounds(frames), "{duration} s");
                assert!(p.workers >= 1 && p.workers <= p.chunks.max(1), "{p:?}");
            }
            // chunks of at least 10 s, at most 12 of them (fewer frames than that: one per frame at most)
            let c = plans[0].chunks;
            if !observe {
                assert!(c == 1 || duration / c as f64 >= 10.0, "{duration} s in {c} chunks");
                assert!(c <= 12);
            }
            // the host decides concurrency, at most one worker per 20 s and per chunk: a 1-worker host renders the
            // same chunks one after another
            for (host, p) in (1..=16).zip(&plans) {
                assert_eq!(p.workers, host.min((duration / 20.0) as usize).min(c).max(1), "host {host}: {p:?}");
            }
        }
        // short programmes stay one stream; 20 s already splits in two
        assert_eq!(segment_plan(Parallel::Auto, true, 4, 19.9, 597, false).chunks, 1);
        assert_eq!(segment_plan(Parallel::Auto, true, 4, 20.0, 600, false).chunks, 2);
        // an output that cannot be split stays one stream whatever is asked
        let serial = SegmentPlan { chunks: 1, workers: 1 };
        assert_eq!(segment_plan(Parallel::Auto, false, 8, 229.0, 13740, false), serial);
        assert_eq!(segment_plan(Parallel::Count(4), false, 8, 229.0, 13740, false), serial);
        // an explicit count keeps its own split
        assert_eq!(segment_plan(Parallel::Count(1), true, 8, 229.0, 13740, false), serial);
        let p = segment_plan(Parallel::Count(2), true, 1, 40.0, 40, false);
        assert_eq!((p.chunks, p.workers), (4, 2));
    }

    #[test]
    fn prefetched_contrast_only_reuses_exact_selected_samples() {
        use sr_gpu::ContrastTarget::Node;
        let (a, b, hidden, extra) = (Node("a".into()), Node("b".into()), Node("hidden".into()), Node("extra".into()));
        let time = 0.5_f64;
        let mut cache = ContrastPrefetch::default();
        cache.measured.insert((time.to_bits(), a.clone()), Some(3.0));
        cache.measured.insert((time.to_bits(), hidden.clone()), None);
        cache.measured.insert((time.to_bits(), extra), Some(1.0));
        cache.measured.insert(((time + 0.01).to_bits(), b.clone()), Some(9.0));
        let mut requested = Vec::new();
        let ratios = cache
            .resolve(time, &[b.clone(), hidden, a], |missing| {
                requested = missing.to_vec();
                vec![Some(5.0); missing.len()]
            })
            .unwrap();
        assert_eq!(requested, vec![b], "only an absent exact sample needs a fallback");
        assert_eq!(ratios, vec![Some(5.0), None, Some(3.0)]);
        assert!(cache.resolve(time, &[], |_| panic!("no selected targets")).unwrap().is_empty());
    }

    /// A hardware encoder opens its session when the first frame arrives, and takes no frame meanwhile: the queue in
    /// front of it keeps the render going for that long, within a fixed amount of memory.
    #[test]
    fn the_encoder_queue_holds_a_fixed_amount_of_frames_in_bytes() {
        let frame = |w, h| InputFormat::Nv12.frame_bytes(w, h);
        assert_eq!(frame_queue(frame(1920, 1080), 1), 30);
        // never more than the cap, never fewer than three
        assert_eq!(frame_queue(frame(64, 36), 1), FRAME_QUEUE_MAX);
        assert_eq!(frame_queue(frame(3840, 2160), 1), 7);
        assert_eq!(frame_queue(InputFormat::Gbrapf32.frame_bytes(7680, 4320), 1), 3);
        assert_eq!(frame_queue(0, 1), FRAME_QUEUE_MAX);
    }

    fn feeder_finishing(result: Result<String, sr_media::MediaError>) -> Feeder {
        let complete = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        Feeder { tx: None, handle: Some(std::thread::spawn(move || result)), complete }
    }

    #[test]
    fn encoders_finish_one_behind_the_chunk_that_renders() {
        let mut closing = Closing::default();
        assert_eq!(closing.wait().unwrap(), None);
        assert_eq!(closing.start(feeder_finishing(Ok("first".into()))).unwrap(), None);
        // starting the next drain waits for the one before, and names its encoder
        assert_eq!(closing.start(feeder_finishing(Ok("second".into()))).unwrap().as_deref(), Some("first"));
        assert_eq!(closing.wait().unwrap().as_deref(), Some("second"));
        assert_eq!(closing.wait().unwrap(), None);
    }

    #[test]
    fn an_encoder_that_failed_while_finishing_is_reported_by_the_next_drain() {
        let mut closing = Closing::default();
        let failed = sr_media::MediaError::Invalid("ffmpeg stopped".into());
        assert!(closing.start(feeder_finishing(Err(failed))).unwrap().is_none());
        let error = closing.start(feeder_finishing(Ok("next".into()))).unwrap_err().to_string();
        assert!(error.contains("ffmpeg stopped"), "{error}");
        // the drain that was refused did not start, and nothing is left running
        assert_eq!(closing.wait().unwrap(), None);
    }

    #[test]
    fn a_panicking_encoder_thread_is_an_error_not_an_unwind() {
        let mut closing = Closing::default();
        let complete = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let feeder = Feeder {
            tx: None,
            handle: Some(std::thread::spawn(|| -> Result<String, sr_media::MediaError> { panic!("boom") })),
            complete,
        };
        assert!(closing.start(feeder).unwrap().is_none());
        let error = closing.wait().unwrap_err().to_string();
        assert!(error.contains("boom"), "{error}");
    }

    #[test]
    fn a_frame_sent_to_a_finished_encode_is_an_error() {
        let mut feeder =
            Feeder { tx: None, handle: None, complete: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)) };
        let error = feeder.send(vec![0; 4].into()).unwrap_err().to_string();
        assert!(error.contains("after the encode was finished"), "{error}");
    }

    #[test]
    fn a_contrast_fallback_with_the_wrong_number_of_results_is_an_error() {
        use sr_gpu::ContrastTarget::Node;
        let (a, b) = (Node("a".into()), Node("b".into()));
        for results in [vec![Some(2.0)], vec![Some(2.0), None, Some(4.0)]] {
            let mut cache = ContrastPrefetch::default();
            let error = cache.resolve(1.0, &[a.clone(), b.clone()], |_| results.clone()).unwrap_err().to_string();
            assert!(error.contains(&format!("{} measurements for 2 texts", results.len())), "{error}");
        }
    }

    #[test]
    fn contrast_predictions_reset_between_outputs_and_never_replace_actual_selection() {
        use sr_gpu::ContrastTarget::Node;
        let id = Node("title".into());
        let times: Vec<_> = (0..100).map(|k| k as f64 / 30.0).collect();
        let predicted: Vec<_> = (0..100).map(|k| (k, if k < 10 { 0.5 } else { 1.0 })).collect();
        let mut cache = ContrastPrefetch::default();
        cache.prepare(&times, std::collections::BTreeMap::from([(id.clone(), predicted.clone())]));
        let planned: Vec<_> = times
            .iter()
            .enumerate()
            .filter(|(_, t)| cache.planned.contains_key(&t.to_bits()))
            .map(|(k, _)| k)
            .collect();
        assert_eq!(planned, probe_frames(&predicted, CONTRAST_PROBES).iter().map(|&(k, _)| k).collect::<Vec<_>>());
        for &k in &planned {
            cache.measured.insert((times[k].to_bits(), id.clone()), Some(2.0));
        }
        // Actual visibility peaks on a different frame: predictions cannot suppress it.
        let actual = contrast_schedule(std::collections::BTreeMap::from([(id.clone(), vec![(0, 1.0), (10, 0.5)])]));
        let mut calls = 0;
        for (k, targets) in actual {
            let ids: Vec<_> = targets.into_iter().map(|(id, _)| id).collect();
            assert_eq!(
                cache
                    .resolve(times[k], &ids, |_| {
                        calls += 1;
                        vec![Some(1.0)]
                    })
                    .unwrap(),
                vec![Some(1.0)]
            );
        }
        assert_eq!(calls, 1);
        cache.prepare(&times, Default::default());
        assert!(cache.planned.is_empty() && cache.measured.is_empty());
        assert_eq!(cache.resolve(times[10], &[id], |_| vec![Some(7.0)]).unwrap(), vec![Some(7.0)]);
    }

    #[test]
    fn contrast_schedule_preserves_every_selected_observation_in_time_order() {
        use sr_gpu::ContrastTarget;
        let observations = std::collections::BTreeMap::from([
            (ContrastTarget::Node("late".into()), (100..300).map(|k| (k, 1.0)).collect()),
            (ContrastTarget::Node("early".into()), (0..160).map(|k| (k, (k as f64 / 20.0).min(1.0))).collect()),
            (ContrastTarget::Captions, vec![(0, 1.0), (100, 1.0), (299, 1.0)]),
        ]);
        let schedule = contrast_schedule(observations.clone());
        let frames: Vec<_> = schedule.keys().copied().collect();
        assert!(frames.windows(2).all(|w| w[0] < w[1]));
        assert!(schedule[&100].contains(&(ContrastTarget::Node("late".into()), 1.0)));
        assert!(schedule[&100].contains(&(ContrastTarget::Captions, 1.0)));
        for (id, seen) in observations {
            let actual: Vec<_> = schedule
                .iter()
                .flat_map(|(&k, targets)| {
                    targets.iter().filter(|(target, _)| *target == id).map(move |(_, opacity)| (k, *opacity))
                })
                .collect();
            assert_eq!(actual, probe_frames(&seen, CONTRAST_PROBES));
        }
    }

    /// The post pass of the contrast check renders each probe's frame without its text: its cost grew with the number of
    /// texts times 32 probes each, which kept v0.1.4 at full CPU for about 15 minutes after the last frame of the Inova film
    /// (`encode --strict`, PALS-Notes/projects/inova-institucional/film/GATES.md). The whole output has one budget.
    #[test]
    fn the_contrast_post_pass_is_bounded_whatever_the_number_of_texts() {
        use sr_gpu::ContrastTarget;
        let held = |texts: usize| -> std::collections::BTreeMap<ContrastTarget, Vec<(usize, f64)>> {
            (0..texts).map(|i| (ContrastTarget::Node(format!("t{i}")), (0..1500).map(|k| (k, 1.0)).collect())).collect()
        };
        for texts in [1, 8, 40, 400, 1000] {
            let unprobed = held(texts);
            let schedule = contrast_schedule(unprobed.clone());
            let probes: usize = schedule.values().map(Vec::len).sum();
            assert!(probes <= CONTRAST_BUDGET.max(texts), "{texts} texts: {probes} probes");
            // every text is still measured, at both ends of its hold when it has two probes
            for id in unprobed.keys() {
                let at: Vec<usize> =
                    schedule.iter().filter(|(_, t)| t.iter().any(|(i, _)| i == id)).map(|(k, _)| *k).collect();
                assert!(!at.is_empty(), "{texts} texts: {id} is not measured");
                if at.len() > 1 {
                    assert_eq!((at[0], at[at.len() - 1]), (0, 1499), "{texts} texts: {id}");
                }
            }
            // a few texts keep every probe they had; more share the budget, and say so
            let thinned = thinned_contrasts(&unprobed);
            if texts * CONTRAST_PROBES <= CONTRAST_BUDGET {
                assert_eq!(probes, texts * CONTRAST_PROBES);
                assert!(thinned.is_empty());
            } else {
                assert_eq!(thinned.len(), texts, "{texts} texts");
                assert!(thinned.iter().all(|(_, got, of)| *got < CONTRAST_PROBES && *of == 1500));
            }
        }
    }

    /// The prefetch during the main pass predicts the texts, and the post pass measures those actually seen: their
    /// numbers can differ, and so can each text's share of the budget. A smaller share's frames are among a larger
    /// one's, so what the prediction measured is reused.
    #[test]
    fn a_smaller_share_of_probes_is_a_subset_of_a_larger_one() {
        let seen: Vec<(usize, f64)> = (0..1500).map(|k| (k, 1.0)).collect();
        for n in [0, 1, 2, 3, 7, 32, 33, 100] {
            let mut order = coarse_to_fine(n);
            order.sort_unstable();
            assert_eq!(order, (0..n).collect::<Vec<_>>(), "every index once for {n}");
        }
        for small in 1..=CONTRAST_PROBES {
            let a = probe_frames(&seen, small);
            assert_eq!(a.len(), small);
            for large in small..=CONTRAST_PROBES {
                let b = probe_frames(&seen, large);
                assert!(a.iter().all(|f| b.contains(f)), "{small} probes are not among {large}");
            }
        }
    }

    #[test]
    fn a_held_title_is_probed_a_bounded_number_of_times() {
        // a title fading in over a second, then held for five minutes at 30 fps
        let seen: Vec<(usize, f64)> = (0..9030).map(|k| (k, (k as f64 / 30.0).min(1.0))).collect();
        let frames = probe_frames(&seen, CONTRAST_PROBES);
        assert!(frames.len() <= CONTRAST_PROBES, "{} probe renders", frames.len());
        // both ends of the hold, and spread over it
        assert_eq!((frames[0], frames[frames.len() - 1]), ((30, 1.0), (9029, 1.0)));
        assert!(frames.windows(2).all(|w| w[1].0 > w[0].0 && w[1].0 - w[0].0 <= 9000 / (CONTRAST_PROBES - 1) + 1));
        // a short one is probed at every frame
        assert_eq!(probe_frames(&seen[..40], CONTRAST_PROBES), seen[30..40].to_vec());
        assert!(probe_frames(&[], CONTRAST_PROBES).is_empty());
    }

    #[test]
    fn only_the_file_name_of_a_sequence_is_a_pattern() {
        assert_eq!(sequence_name(Path::new("out/f_%04d.png"), 7), "out/f_0007.png");
        assert_eq!(sequence_name(Path::new("renders/50%/f_%04d.png"), 7), "renders/50%/f_0007.png");
        assert_eq!(sequence_name(Path::new("renders/%d/f_%d.png"), 12), "renders/%d/f_12.png");
        assert_eq!(sequence_name(Path::new("out/plain.png"), 7), "out/plain.png");
    }

    /// A 5-frame FFV1 intermediate, as the two-pass path writes it.
    fn intermediate(dir: &Path) -> Option<(EncodeSpec, PathBuf)> {
        std::process::Command::new(sr_media::ffmpeg()).arg("-version").output().ok()?;
        std::fs::create_dir_all(dir).ok()?;
        let adhoc = adhoc_output("x.mkv", "ffv1").ok()?;
        let doc = sr_model::load_str(
            r#"<scene version="1.1"><project width="64" height="36" fps="25" duration="1"/><composition/></scene>"#,
            &sr_model::LoadOptions::without_assets(),
        )
        .ok()?;
        let ev = Evaluator::new(&doc, &EvalOptions::default()).ok()?;
        let color = OutputColor::new(m::ColorSpace::Srgb, m::Transfer::Auto, false);
        let opts = Options { hardware: Hardware::Software, ..Default::default() };
        let out = dir.join("out.mkv");
        let spec =
            base_spec(&adhoc, &out, Codec::Ffv1, [64, 36], 25.0, InputFormat::Rgba8, &color, None, ev.program(), &opts);
        let inter = dir.join("intermediate.mkv");
        let mut enc =
            Encoder::start(&EncodeSpec { path: inter.clone(), pixel_format: "bgra".into(), ..spec.clone() }).ok()?;
        for k in 0..5u8 {
            enc.write(&vec![k * 40; InputFormat::Rgba8.frame_bytes(64, 36)]).ok()?;
        }
        enc.finish().ok()?;
        Some((spec, inter))
    }

    #[test]
    fn a_partial_or_failed_intermediate_decode_fails_the_pass() {
        let dir = std::env::temp_dir().join(format!("sr-deliver-replay-{}", std::process::id()));
        let Some((spec, inter)) = intermediate(&dir) else { return };
        assert!(replay(&inter, &spec, InputFormat::Rgba8, [64, 36], 5).is_ok());
        let successful_output = std::fs::read(&spec.path).unwrap();
        // cut in the middle of its frames
        let bytes = std::fs::read(&inter).unwrap();
        std::fs::write(&inter, &bytes[..bytes.len() * 6 / 10]).unwrap();
        let cut = replay(&inter, &spec, InputFormat::Rgba8, [64, 36], 5);
        assert!(cut.is_err(), "a truncated intermediate was encoded as if whole");
        assert_eq!(std::fs::read(&spec.path).unwrap(), successful_output, "a failed pass replaced the previous output");
        std::fs::write(&inter, b"not a video").unwrap();
        let e = replay(&inter, &spec, InputFormat::Rgba8, [64, 36], 5).unwrap_err().to_string();
        assert!(e.contains("intermediate.mkv") && e.contains("Invalid data"), "{e}");
        assert_eq!(std::fs::read(&spec.path).unwrap(), successful_output);
    }
}

/// Live stage times of the frames rendered so far in this process, readable while an encode runs, so a progress
/// line can say where the time goes (a slow encode must not be a mystery until it ends).
pub mod meter {
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

    static STAGE_NS: [AtomicU64; 4] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];

    /// Adds `secs` to stage `i` of `report` and of the live meter (0 evaluate, 1 render, 2 GPU and readback wait,
    /// 3 encoder blocked).
    pub(crate) fn add(report: &mut [f64; 4], i: usize, secs: f64) {
        report[i] += secs;
        STAGE_NS[i].fetch_add((secs.max(0.0) * 1e9) as u64, Relaxed);
    }

    /// Seconds spent in each stage so far, summed over every render worker of this process.
    pub fn snapshot() -> [f64; 4] {
        std::array::from_fn(|i| STAGE_NS[i].load(Relaxed) as f64 * 1e-9)
    }
}
