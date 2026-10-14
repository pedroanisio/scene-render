//! `audio/@duration`, `@sampleRate` and `@channels` are claims about the file: the probe wins, and a claim the file
//! contradicts is a warning (A09).

use std::path::{Path, PathBuf};

use sr_model::{validate_str, LoadOptions};

/// A silent 16-bit PCM WAV of `seconds` at `rate` Hz with `channels` channels.
fn write_wav(path: &Path, rate: u32, channels: u16, seconds: u32) {
    let pcm = vec![0u8; (rate * seconds) as usize * channels as usize * 2];
    let mut wav = Vec::new();
    wav.extend(b"RIFF");
    wav.extend((36 + pcm.len() as u32).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(channels.to_le_bytes());
    wav.extend(rate.to_le_bytes());
    wav.extend((rate * channels as u32 * 2).to_le_bytes());
    wav.extend((channels * 2).to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend((pcm.len() as u32).to_le_bytes());
    wav.extend(pcm);
    std::fs::write(path, wav).unwrap();
}

fn probe_available() -> bool {
    std::process::Command::new(sr_media::ffprobe()).arg("-version").output().is_ok()
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr-model-audio-declared-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    write_wav(&d.join("m.wav"), 48_000, 2, 1);
    d
}

fn codes(attrs: &str, d: &Path) -> Vec<String> {
    let xml = format!(
        r#"<scene version="1.1"><project width="64" height="32" fps="30" duration="1"/>
<assets><audio id="m" src="m.wav" {attrs}/></assets><composition/></scene>"#
    );
    let report = validate_str(&xml, &LoadOptions { verify_assets: true, base_dir: Some(d.to_path_buf()) });
    assert!(!report.has_errors(), "{report}");
    report.diagnostics.iter().filter(|x| x.code == "A09").map(|x| x.message.clone()).collect()
}

#[test]
fn declarations_the_file_contradicts_warn_once_each() {
    if !probe_available() {
        return;
    }
    let d = dir("mismatch");
    let found = codes(r#"sampleRate="44100" channels="1" duration="3""#, &d);
    assert_eq!(found.len(), 3, "{found:?}");
    for (attr, declared, actual) in [("sampleRate", "44100", "48000"), ("channels", "1", "2"), ("duration", "3", "1")] {
        assert!(
            found.iter().any(|m| m.contains(attr) && m.contains(declared) && m.contains(actual)),
            "{attr}: {found:?}"
        );
    }
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn declarations_the_file_confirms_are_silent() {
    if !probe_available() {
        return;
    }
    let d = dir("match");
    // the container duration of a 1 s file is within the tolerance of a declared 1.02 s
    assert!(codes(r#"sampleRate="48000" channels="2" duration="1.02""#, &d).is_empty());
    assert!(codes("", &d).is_empty());
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn a_remote_file_is_not_probed() {
    let xml = r#"<scene version="1.1"><project width="64" height="32" fps="30" duration="1"/>
<assets><audio id="m" src="https://example.invalid/m.wav" sampleRate="44100" duration="3"/></assets><composition/></scene>"#;
    let report = validate_str(xml, &LoadOptions { verify_assets: true, base_dir: Some(std::env::temp_dir()) });
    assert!(!report.diagnostics.iter().any(|d| d.code == "A09"), "{report}");
    assert!(report.diagnostics.iter().any(|d| d.code == "A03"), "{report}");
}
