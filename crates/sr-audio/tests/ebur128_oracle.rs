// Adapted from FilmCraft, crates/audio-dsp/tests/loudness_oracle.rs (the ffmpeg runner and parser and the
// pink-noise, speech-like and gated-stereo signals), Copyright (c) 2026 ArtCraft Team and the FilmCraft
// contributors, used under the MIT License (FilmCraft is MIT OR Apache-2.0); see THIRD-PARTY-NOTICES.md.

//! The loudness meter against FFmpeg's `ebur128` filter on generated signals.
//!
//! Each signal is written as a 32-bit float WAV and measured twice: by [`sr_audio::loudness`] and by
//! `ffmpeg -af ebur128=peak=true:metadata=1,ametadata=mode=print` (one frame of values every 100 ms). FFmpeg is
//! an outside reference only, run as a program. Without it the test is skipped with `SKIPPED`, or fails where
//! `SR_REQUIRE_FFMPEG=1`.
//!
//! Tolerances (EBU Tech 3341 allows ±0.1 LU for M, S and I and +0.2/−0.4 dB for true peak; Tech 3342 allows
//! ±1 LU for LRA):
//! - momentary (from the first full 400 ms) and short-term (from the first full 3 s): ±0.1 LU wherever FFmpeg
//!   reads above −70 LUFS, and their maxima the same;
//! - integrated: ±0.1 LU;
//! - loudness range: ±0.5 LU;
//! - true peak: ±0.2 dB, against the waveform's own peak where the signal has a known one, else against FFmpeg
//!   (both interpolate, with different filters).

use std::f64::consts::PI;
use std::path::Path;
use std::process::Command;

use sr_audio::layout::Layout;
use sr_audio::loudness::{self, Planar};

const TOL_MS: f64 = 0.1;
const TOL_I: f64 = 0.1;
const TOL_LRA: f64 = 0.5;
const TOL_TP_DB: f64 = 0.2;

/// FFmpeg's values for one 100 ms frame.
#[derive(Clone, Copy, Debug, Default)]
struct Frame {
    m: f64,
    s: f64,
    i: f64,
    lra: f64,
    true_peak: f64,
}

fn ffmpeg() -> String {
    std::env::var("SR_FFMPEG").unwrap_or_else(|_| "ffmpeg".into())
}

fn ebur128(ff: &str, wav: &Path) -> Vec<Frame> {
    let out = Command::new(ff)
        .args(["-hide_banner", "-nostats", "-nostdin", "-v", "error", "-i"])
        .arg(wav)
        .args(["-af", "ebur128=peak=true:metadata=1,ametadata=mode=print:file=-", "-f", "null", "-"])
        .output()
        .unwrap_or_else(|e| panic!("cannot run {ff}: {e}"));
    assert!(out.status.success(), "ffmpeg ebur128 failed: {}", String::from_utf8_lossy(&out.stderr));
    let mut frames = Vec::new();
    let mut cur: Option<Frame> = None;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if line.starts_with("frame:") {
            frames.extend(cur.take());
            cur = Some(Frame::default());
            continue;
        }
        let (Some(f), Some((k, v))) = (cur.as_mut(), line.strip_prefix("lavfi.r128.").and_then(|l| l.split_once('=')))
        else {
            continue;
        };
        let v: f64 = v.trim().parse().unwrap_or_else(|e| panic!("ffmpeg printed {line:?}: {e}"));
        match k {
            "M" => f.m = v,
            "S" => f.s = v,
            "I" => f.i = v,
            "LRA" => f.lra = v,
            "true_peak" => f.true_peak = v,
            _ => {}
        }
    }
    frames.extend(cur);
    frames
}

/// Compares the meter with FFmpeg on `buf` at `rate` Hz; `peak` is the waveform's own true peak (dBTP) when the
/// signal has a known one.
fn compare(ff: &str, name: &str, buf: Planar, rate: u32, layout: Layout, peak: Option<f64>) {
    let dir = std::env::temp_dir().join(format!("sr-audio-ebur128-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let wav = dir.join(format!("{name}-{rate}.wav"));
    sr_audio::wav::write(&wav, &buf, rate, 32, layout, false).unwrap();
    let frames = ebur128(ff, &wav);
    std::fs::remove_file(&wav).unwrap();
    let r = rate as f64;
    let weights = layout.loudness_weights();
    let ours = loudness::measure(&buf, r, &weights);
    let m = loudness::momentary(&buf, r, &weights, 0.1);
    let s = loudness::short_term(&buf, r, &weights, 0.1);
    assert!(!frames.is_empty() && frames.len() + 1 >= m.len() + 3, "{name}: {} frames from ffmpeg", frames.len());
    // FFmpeg's frame k ends at (k + 1) · 100 ms: momentary j (400 ms from j · 100 ms) is frame j + 3 and
    // short-term j (3 s) frame j + 29
    let (mut max_m, mut max_s, mut compared) = (-200f64, -200f64, 0);
    for (j, ours) in m.iter().enumerate() {
        let Some(f) = frames.get(j + 3) else { break };
        max_m = max_m.max(f.m);
        if f.m > -70.0 {
            assert!(
                (f.m - ours).abs() <= TOL_MS,
                "{name}: momentary at {:.1} s: ffmpeg {:.3}, ours {ours:.3}",
                (j + 4) as f64 / 10.0,
                f.m
            );
            compared += 1;
        }
    }
    for (j, ours) in s.iter().enumerate() {
        let Some(f) = frames.get(j + 29) else { break };
        max_s = max_s.max(f.s);
        if f.s > -70.0 {
            assert!(
                (f.s - ours).abs() <= TOL_MS,
                "{name}: short-term at {:.1} s: ffmpeg {:.3}, ours {ours:.3}",
                (j + 30) as f64 / 10.0,
                f.s
            );
        }
    }
    assert!(compared > 0, "{name}: no momentary value above −70 LUFS");
    let last = frames.last().unwrap();
    let ff_tp = 20.0 * last.true_peak.log10();
    eprintln!(
        "{name} at {rate} Hz: I {:.3} / ffmpeg {:.3} LUFS, LRA {:.2} / {:.2} LU, max M {:.3} / {:.3}, max S {:.3} / {:.3}, TP {:.3} / {:.3} dBTP{}",
        ours.integrated,
        last.i,
        ours.range,
        last.lra,
        ours.momentary_max,
        max_m,
        ours.short_term_max,
        max_s,
        ours.true_peak,
        ff_tp,
        peak.map(|p| format!(" (waveform {p:.3})")).unwrap_or_default()
    );
    assert!(
        (ours.integrated - last.i).abs() <= TOL_I,
        "{name}: integrated {:.3}, ffmpeg {:.3}",
        ours.integrated,
        last.i
    );
    assert!((ours.range - last.lra).abs() <= TOL_LRA, "{name}: LRA {:.3}, ffmpeg {:.3}", ours.range, last.lra);
    assert!((ours.momentary_max - max_m).abs() <= TOL_MS, "{name}: max M {:.3}, ffmpeg {max_m:.3}", ours.momentary_max);
    if max_s > -70.0 {
        assert!(
            (ours.short_term_max - max_s).abs() <= TOL_MS,
            "{name}: max S {:.3}, ffmpeg {max_s:.3}",
            ours.short_term_max
        );
    }
    let reference = peak.unwrap_or(ff_tp);
    assert!(
        (ours.true_peak - reference).abs() <= TOL_TP_DB,
        "{name}: true peak {:.3}, reference {reference:.3} dBTP",
        ours.true_peak
    );
}

struct Rng(u64);

impl Rng {
    /// Uniform in [−1, 1).
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    }
}

fn db(x: f64) -> f64 {
    10f64.powf(x / 20.0)
}

fn sine(freq: f64, amp: f64, rate: u32, secs: f64, phase: f64) -> Vec<f32> {
    (0..(secs * rate as f64) as usize)
        .map(|i| (amp * (2.0 * PI * freq * i as f64 / rate as f64 + phase).sin()) as f32)
        .collect()
}

/// A sine with raised-cosine fades of 50 ms at each end, so its true peak is the crest of the steady part.
fn faded(freq: f64, amp: f64, rate: u32, secs: f64, phase: f64) -> Vec<f32> {
    let n = (secs * rate as f64) as usize;
    let ramp = rate as usize / 20;
    sine(freq, amp, rate, secs, phase)
        .into_iter()
        .enumerate()
        .map(|(i, v)| {
            let edge = i.min(n - 1 - i);
            if edge < ramp {
                (v as f64 * (0.5 - 0.5 * (PI * edge as f64 / ramp as f64).cos())) as f32
            } else {
                v
            }
        })
        .collect()
}

/// Pink noise (Paul Kellet's refined filter on white noise), scaled by `amp`.
fn pink(rate: u32, secs: f64, amp: f64, seed: u64) -> Vec<f32> {
    let mut r = Rng(seed);
    let (mut b0, mut b1, mut b2, mut b3, mut b4, mut b5, mut b6) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    (0..(secs * rate as f64) as usize)
        .map(|_| {
            let w = r.next();
            b0 = 0.99886 * b0 + w * 0.0555179;
            b1 = 0.99332 * b1 + w * 0.0750759;
            b2 = 0.96900 * b2 + w * 0.1538520;
            b3 = 0.86650 * b3 + w * 0.3104856;
            b4 = 0.55000 * b4 + w * 0.5329522;
            b5 = -0.7616 * b5 - w * 0.0168980;
            let y = b0 + b1 + b2 + b3 + b4 + b5 + b6 + w * 0.5362;
            b6 = w * 0.115926;
            (y * 0.11 * amp) as f32
        })
        .collect()
}

/// Speech-like: syllables (80–300 ms) of a pitched, formant-shaped tone with pauses (50–600 ms) and changes of
/// level between phrases, so the gates and the M and S windows all have work.
fn speech(rate: u32, secs: f64, seed: u64) -> Vec<f32> {
    let mut r = Rng(seed);
    let sr = rate as f64;
    let n = (secs * sr) as usize;
    let mut out = vec![0f32; n];
    let mut pos = 0usize;
    let mut phrase = 1.0;
    while pos < n {
        if r.next() > 0.7 {
            phrase = db(-12.0 * (r.next() + 1.0) / 2.0);
        }
        let syl = ((0.08 + 0.22 * (r.next() + 1.0) / 2.0) * sr) as usize;
        let f0 = 110.0 + 60.0 * (r.next() + 1.0) / 2.0;
        let (f1, f2) = (500.0 + 400.0 * r.next(), 1500.0 + 700.0 * r.next());
        for i in 0..syl.min(n - pos) {
            let t = i as f64 / sr;
            let env = (PI * i as f64 / syl as f64).sin().powi(2);
            let mut v = 0.0;
            for h in 1..=20 {
                let f = f0 * h as f64;
                if f > sr / 2.2 {
                    break;
                }
                let formant = (-((f - f1) / 200.0).powi(2)).exp() + 0.5 * (-((f - f2) / 300.0).powi(2)).exp() + 0.05;
                v += formant * (2.0 * PI * f * t).sin() / h as f64;
            }
            out[pos + i] = (0.3 * phrase * env * v + 0.003 * r.next()) as f32;
        }
        pos += syl + ((0.05 + 0.55 * (r.next() + 1.0) / 2.0) * sr) as usize;
    }
    out
}

#[test]
fn loudness_matches_ffmpeg_ebur128() {
    let ff = ffmpeg();
    sr_testkit::require_ffmpeg!("loudness against ffmpeg ebur128", &[ff.as_str()]);
    let sr = 48_000;

    // a steady 997 Hz sine at −20 dBFS in both channels
    let s = sine(997.0, db(-20.0), sr, 10.0, 0.0);
    compare(&ff, "sine-997", vec![s.clone(), s], sr, Layout::Stereo, None);

    // independent pink noise per channel
    compare(&ff, "pink", vec![pink(sr, 15.0, 0.5, 1), pink(sr, 15.0, 0.5, 2)], sr, Layout::Stereo, None);

    // syllables and pauses: the relative gate and the loudness range
    let sp = speech(sr, 25.0, 7);
    let sp2: Vec<f32> = sp.iter().map(|v| v * 0.7).collect();
    compare(&ff, "speech", vec![sp, sp2], sr, Layout::Stereo, None);

    // a loud tone, digital silence, a −30 dBFS tone on the left only, a passage under the relative gate and
    // one under the absolute gate
    let l = [
        sine(440.0, db(-18.0), sr, 5.0, 0.0),
        vec![0.0; 3 * sr as usize],
        sine(1000.0, db(-30.0), sr, 5.0, 0.0),
        pink(sr, 4.0, db(-50.0), 3),
        pink(sr, 4.0, db(-80.0), 4),
    ]
    .concat();
    let r = [
        sine(440.0, db(-18.0), sr, 5.0, 0.3),
        vec![0.0; 8 * sr as usize],
        pink(sr, 4.0, db(-50.0), 5),
        pink(sr, 4.0, db(-80.0), 6),
    ]
    .concat();
    compare(&ff, "gated", vec![l, r], sr, Layout::Stereo, None);

    // the Tech 3342 programme of four levels (−50, −35, −20, −35, −50 dBFS, 20 s each): LRA 15 LU
    let steps = [-50.0, -35.0, -20.0, -35.0, -50.0];
    let mut x = Vec::new();
    for level in steps {
        let n = x.len();
        x.extend(sine(1000.0, db(level), sr, 20.0, 2.0 * PI * 1000.0 * n as f64 / sr as f64));
    }
    compare(&ff, "tech3342-4", vec![x.clone(), x], sr, Layout::Stereo, None);

    // inter-sample peaks: fs/4 at 45°, a sample peak 3 dB under the waveform's
    let x = faded(sr as f64 / 4.0, 0.9, sr, 5.0, PI / 4.0);
    compare(&ff, "fs4-48k", vec![x.clone(), x], sr, Layout::Stereo, Some(20.0 * 0.9f64.log10()));

    // 44.1 kHz: fs/4 with every crest an eighth of a sample off a 4× grid (true peak at 8×)
    let x = faded(44_100.0 / 4.0, 0.9, 44_100, 5.0, 0.4375 * PI);
    compare(&ff, "fs4-44k1", vec![x.clone(), x], 44_100, Layout::Stereo, Some(20.0 * 0.9f64.log10()));

    // mono speech at 44.1 kHz
    compare(&ff, "speech-mono", vec![speech(44_100, 20.0, 11)], 44_100, Layout::Mono, None);

    // speech at 96 kHz: FFmpeg's filter resamples to 48 kHz, so the signal stays under 24 kHz (pink noise would
    // lose its top octave on FFmpeg's side and not on ours)
    let sp = speech(96_000, 12.0, 21);
    let sp2: Vec<f32> = sp.iter().map(|v| v * 0.8).collect();
    compare(&ff, "speech-96k", vec![sp, sp2], 96_000, Layout::Stereo, None);
}
