//! `encode` reports the loudness range and the loudest momentary and short-term windows beside integrated
//! loudness and true peak.

use std::path::Path;
use std::process::{Command, Output};

/// A stereo 16-bit WAV at 48 kHz: a 1 kHz tone through `(seconds, dBFS)` steps.
fn write_steps(path: &Path, steps: &[(usize, f64)]) {
    let rate = 48_000usize;
    let mut pcm = Vec::new();
    let mut i = 0usize;
    for &(seconds, level) in steps {
        let a = 10f64.powf(level / 20.0);
        for _ in 0..rate * seconds {
            let v = (a * (std::f64::consts::TAU * 1000.0 * i as f64 / rate as f64).sin() * 32767.0).round() as i16;
            pcm.extend(v.to_le_bytes());
            pcm.extend(v.to_le_bytes());
            i += 1;
        }
    }
    let mut wav = Vec::new();
    wav.extend(b"RIFF");
    wav.extend((36 + pcm.len() as u32).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend((rate as u32).to_le_bytes());
    wav.extend((rate as u32 * 4).to_le_bytes());
    wav.extend(4u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend((pcm.len() as u32).to_le_bytes());
    wav.extend(pcm);
    std::fs::write(path, wav).unwrap();
}

/// Encodes `steps` as an audio-only WAV programme, with `--json` when asked.
fn encode(dir: &Path, steps: &[(usize, f64)], json: bool) -> Output {
    std::fs::create_dir_all(dir).unwrap();
    write_steps(&dir.join("tone.wav"), steps);
    let duration: usize = steps.iter().map(|s| s.0).sum();
    std::fs::write(
        dir.join("scene.xml"),
        format!(
            r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="{duration}" background="#000000"/>
            <assets><audio id="a" src="tone.wav"/></assets><composition/>
            <audioMix sampleRate="48000"><audioTrack id="t" asset="a"/><master normalize="none" limiter="false"/></audioMix></scene>"##
        ),
    )
    .unwrap();
    let mut args = vec!["encode", "scene.xml", "-o", "out.wav", "--codec", "audio-only", "--no-upload"];
    if json {
        args.push("--json");
    }
    let o = Command::new(env!("CARGO_BIN_EXE_scene-render"))
        .args(args)
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    o
}

fn report(o: &Output) -> serde_json::Value {
    let text = String::from_utf8_lossy(&o.stdout);
    let line = text.lines().find(|l| l.starts_with('{')).unwrap_or_else(|| panic!("no JSON report in {text}"));
    serde_json::from_str(line).unwrap()
}

fn near(r: &serde_json::Value, key: &str, want: f64, tol: f64) {
    let got = r[key].as_f64().unwrap_or_else(|| panic!("{key} is {}", r[key]));
    assert!((got - want).abs() <= tol, "{key}: {got}, want {want}");
}

#[test]
fn encode_reports_the_loudness_range_and_the_loudest_windows() {
    if !crate::common::ffmpeg("encode_reports_the_loudness_range_and_the_loudest_windows") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("sr-cli-loudness-{}", std::process::id()));
    // 5 s at −20 then 5 s at −30 dBFS: the range spans the two levels, the loudest windows read the louder
    let steps = [(5, -20.0), (5, -30.0)];
    let r = report(&encode(&dir.join("json"), &steps, true));
    near(&r, "loudness_range", 10.0, 0.1);
    near(&r, "momentary_max", -20.0, 0.1);
    near(&r, "short_term_max", -20.0, 0.1);
    near(&r, "true_peak", -20.0, 0.1);
    let text = String::from_utf8_lossy(&encode(&dir.join("text"), &steps, false).stdout).into_owned();
    let audio = text.lines().find(|l| l.trim_start().starts_with("audio:")).unwrap_or_else(|| panic!("{text}"));
    for part in ["LUFS integrated", "LU loudness range", "LUFS momentary max", "LUFS short-term max", "dBTP true peak"]
    {
        assert!(audio.contains(part), "{part:?} missing from {audio:?}");
    }
    // 2 s: no 3 s window, so no range and no short-term maximum
    let r = report(&encode(&dir.join("short"), &[(2, -20.0)], true));
    assert!(r["loudness_range"].is_null() && r["short_term_max"].is_null(), "{r}");
    near(&r, "momentary_max", -20.0, 0.1);
    let text = String::from_utf8_lossy(&encode(&dir.join("short-text"), &[(2, -20.0)], false).stdout).into_owned();
    assert!(
        !text.contains("loudness range") && !text.contains("short-term max") && text.contains("momentary max"),
        "{text}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
