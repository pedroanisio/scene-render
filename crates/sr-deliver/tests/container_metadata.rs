//! A1 delivery contract, exercised through real FFmpeg/FFprobe on the test VM.
use std::path::{Path, PathBuf};

mod common;

// Parallel delivery opens extra devices; keep device creation out of concurrent tests.
static GPU_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sr-a1-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, frequency) in [("main", 440.0), ("description", 1000.0)] {
        let wave: Vec<f32> =
            (0..48000).map(|i| (0.1 * (std::f64::consts::TAU * frequency * i as f64 / 48000.0).sin()) as f32).collect();
        sr_audio::wav::write(
            &dir.join(format!("{name}.wav")),
            &vec![wave.clone(), wave],
            48000,
            32,
            sr_audio::Layout::Stereo,
            false,
        )
        .unwrap();
    }
    dir
}

fn document(dir: &Path, output: &str, reference: &str, language: &str) -> sr_model::Document {
    let xml = format!(
        r#"<scene version="1.2"><project width="64" height="36" fps="25" duration="1" pixelAspect="1.2" timecodeStart="01:00:00:00"/>
    <metadata title="A1" created="2020-01-02T03:04:05Z"><accessibility audioDescription="{reference}" flashCheck="off"/></metadata>
    {output}<assets><audio id="a" src="main.wav"/><audio id="b" src="description.wav"/></assets><composition/>
    <audioMix><audioTrack id="main" asset="a" language="{language}"/><audioTrack id="ad" asset="b" language="pt-BR"/></audioMix>
    <captions><captionTrack id="cc" language="pt-BR" kind="sdh" label="Português" mode="sidecar"><cue start="0" end="1" text="Hello"/></captionTrack></captions></scene>"#
    );
    let path = dir.join("scene.xml");
    std::fs::write(&path, xml).unwrap();
    sr_model::load_file(&path, &Default::default()).unwrap()
}

fn energy(path: &Path, stream: usize, frequency: f64) -> f64 {
    let a = sr_media::decode_audio(path, stream, 48000).unwrap();
    let samples: Vec<f64> =
        a.samples.iter().step_by(a.channels as usize).skip(4800).take(28800).map(|s| *s as f64).collect();
    let (re, im) = samples.iter().enumerate().fold((0.0, 0.0), |(re, im), (i, x)| {
        let phase = std::f64::consts::TAU * frequency * i as f64 / 48000.0;
        (re + x * phase.cos(), im + x * phase.sin())
    });
    (re * re + im * im).sqrt() / samples.len() as f64
}

#[test]
fn a1_audio_description_is_selectable_and_main_excludes_narration() {
    let _guard = GPU_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let Some(gpu) = common::delivery("a1_audio_description_is_selectable_and_main_excludes_narration") else { return };
    let dir = fixture("audio");
    for (ext, codec) in [("mp4", "h264"), ("mov", "h264"), ("mkv", "h264"), ("webm", "vp9")] {
        let d =
            document(&dir, &format!(r#"<output path="out.{ext}" codec="{codec}" preset="ultrafast"/>"#), "ad", "pt-BR");
        let opts =
            sr_deliver::Options { hardware: sr_media::encode::Hardware::Software, upload: false, ..Default::default() };
        let report = sr_deliver::deliver(&d, &d.scene.outputs[0], Some(&gpu), &opts, &mut |_, _| {}).unwrap();
        let info = sr_media::probe(&report.path).unwrap();
        assert_eq!(info.audio.len(), 2, "{ext}");
        assert_eq!(info.audio[0].language, "por", "{ext}");
        assert_eq!(info.audio[1].language, "por", "{ext}");
        if matches!(ext, "mov" | "webm") {
            assert!(!info.audio[1].description, "FFmpeg 7.1 MOV/WebM do not store this disposition");
            assert_eq!(info.audio[1].title, "Audio description");
            let container = if ext == "mov" { "MOV" } else { "WebM" };
            assert!(report.warnings.iter().any(|s| s.contains(container) && s.contains("disposition")));
        } else {
            assert!(info.audio[1].description, "{ext}: {:?}", info.audio);
        }
        assert!(energy(&report.path, 0, 440.0) > 0.03, "{ext}: programme audible");
        assert!(energy(&report.path, 0, 1000.0) < 0.002, "{ext}: description excluded from main");
        assert!(energy(&report.path, 1, 440.0) > 0.03, "{ext}: described version retains programme");
        assert!(energy(&report.path, 1, 1000.0) > 0.03, "{ext}: narration audible");
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["audio_languages"][0], "por");
        assert_eq!(json["caption_tracks"][0]["label"], "Português");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a1_audio_description_falls_back_in_single_stream_outputs() {
    let dir = fixture("fallback");
    let d = document(&dir, r#"<output path="out.m4a" codec="audio-only"/>"#, "ad", "pt-BR");
    let report = sr_deliver::deliver(&d, &d.scene.outputs[0], None, &Default::default(), &mut |_, _| {}).unwrap();
    assert_eq!(sr_media::probe(&report.path).unwrap().audio[0].language, "por");
    assert!(report.warnings.iter().any(|s| s.contains("audio description")));
    assert!(energy(&report.path, 0, 440.0) > 0.03);
    assert!(energy(&report.path, 0, 1000.0) > 0.03);
    let d = document(
        &dir,
        r#"<output path="selected.m4a" codec="audio-only" audioTracks="main"><segment from="0" to="1"/></output>"#,
        "ad",
        "pt-BR",
    );
    let report = sr_deliver::deliver(&d, &d.scene.outputs[0], None, &Default::default(), &mut |_, _| {}).unwrap();
    assert!(energy(&report.path, 0, 1000.0) > 0.03, "fallback includes narration despite ordinary source selection");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a1_audio_description_reference_must_name_an_audio_track() {
    let xml = r#"<scene version="1.2"><project width="64" height="36" fps="25" duration="1"/><metadata><accessibility audioDescription="wrong"/></metadata><assets><audio id="wrong" src="a.wav"/></assets><composition/></scene>"#;
    let report = sr_model::validate_str(xml, &sr_model::LoadOptions::without_assets());
    assert!(report.diagnostics.iter().any(|d| d.code == "AD01"), "{:?}", report.diagnostics);
}

#[test]
fn a1_language_mapping_reports_unknown_and_multilingual_mixes() {
    let dir = fixture("languages");
    for (language, expected, warning) in [("en-US", "mul", false), ("zz-ZZ", "mul", true), ("por", "por", false)] {
        let d = document(&dir, r#"<output path="out.m4a" codec="audio-only"/>"#, "ad", language);
        let report = sr_deliver::deliver(&d, &d.scene.outputs[0], None, &Default::default(), &mut |_, _| {}).unwrap();
        assert_eq!(sr_media::probe(&report.path).unwrap().audio[0].language, expected);
        assert_eq!(report.warnings.iter().any(|s| s.contains("no ISO 639-2 mapping")), warning);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a1_description_survives_segments_ranges_and_parallel_muxing() {
    let _guard = GPU_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let Some(gpu) = common::delivery("a1_description_survives_segments_ranges_and_parallel_muxing") else { return };
    let dir = fixture("segments");
    for (name, extra, parallel) in [
        ("segment", r#"<segment from="0.2" to="0.6"/>"#, 1),
        ("parallel", "", 2),
        ("two-pass", "", 1),
        ("range", "", 1),
        ("drop-rate", "", 1),
        ("segment-range", r#"<segment from="0" to="1"/>"#, 1),
    ] {
        let two_pass = if name == "two-pass" {
            "twoPass=\"true\" bitrate=\"100000\""
        } else if name == "range" {
            "embedMetadata=\"false\""
        } else if name == "drop-rate" {
            "fps=\"25\""
        } else {
            ""
        };
        let d = document(
            &dir,
            &format!(r#"<output path="{name}.mp4" codec="h264" preset="ultrafast" {two_pass}>{extra}</output>"#),
            "ad",
            "pt-BR",
        );
        let d = if name == "drop-rate" {
            let path = dir.join("scene.xml");
            let xml = std::fs::read_to_string(&path)
                .unwrap()
                .replacen("fps=\"25\"", "fps=\"30000/1001\"", 1)
                .replace("01:00:00:00", "01:00:00;00");
            std::fs::write(&path, xml).unwrap();
            sr_model::load_file(&path, &Default::default()).unwrap()
        } else {
            d
        };
        let opts = sr_deliver::Options {
            hardware: sr_media::encode::Hardware::Software,
            parallel: sr_deliver::Parallel::Count(parallel),
            start: match name {
                "range" => Some(0.2),
                "segment-range" => Some(0.21),
                _ => None,
            },
            end: matches!(name, "range" | "segment-range").then_some(0.8),
            upload: false,
            ..Default::default()
        };
        let report = sr_deliver::deliver(&d, &d.scene.outputs[0], Some(&gpu), &opts, &mut |_, _| {}).unwrap();
        let info = sr_media::probe(&report.path).unwrap();
        assert_eq!(report.segments, parallel, "exercise the requested muxing path");
        assert_eq!(info.audio.len(), 2, "{name}");
        assert!(info.audio[1].description, "{name}");
        assert_eq!(info.video.unwrap().sample_aspect_ratio, "6:5", "{name}");
        assert_eq!(
            info.timecode.as_deref(),
            Some(match name {
                "range" => "01:00:00:05",
                "segment-range" => "01:00:00:06",
                _ => "01:00:00:00",
            }),
            "{name}"
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}
