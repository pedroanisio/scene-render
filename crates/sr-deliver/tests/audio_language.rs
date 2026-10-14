//! `audio/@language` is written as the language tag of the encoded audio stream.

use std::path::Path;

fn write_tone(path: &Path) {
    let tone: Vec<f32> =
        (0..48_000).map(|k| (0.25 * (std::f64::consts::TAU * 440.0 * k as f64 / 48_000.0).sin()) as f32).collect();
    sr_audio::wav::write(path, &vec![tone.clone(), tone], 48_000, 16, sr_audio::Layout::Stereo, false).unwrap();
}

/// The `language` tag ffprobe reads from the first audio stream of `path`.
fn language_of(path: &Path) -> String {
    let out = std::process::Command::new(sr_media::ffprobe())
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream_tags=language",
            "-of",
            "default=nw=1:nk=1",
        ])
        .arg(path)
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn deliver(name: &str, asset_attrs: &str, second: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("sr-audio-language-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    write_tone(&dir.join("a.wav"));
    write_tone(&dir.join("b.wav"));
    let xml = format!(
        r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="1" background="#000000"/>
        <output id="m" path="out.m4a" codec="audio-only"/>
        <assets><audio id="a" src="a.wav" {asset_attrs}/><audio id="b" src="b.wav" {second}/></assets><composition/>
        <audioMix sampleRate="48000"><audioTrack id="ta" asset="a"/><audioTrack id="tb" asset="b"/></audioMix></scene>"##
    );
    let path = dir.join("scene.xml");
    std::fs::write(&path, xml).unwrap();
    let d = sr_model::load_file(&path, &sr_model::LoadOptions::default()).unwrap_or_else(|e| panic!("{e:?}"));
    let opts = sr_deliver::Options { hardware: sr_media::encode::Hardware::Software, ..Default::default() };
    sr_deliver::deliver(&d, &d.scene.outputs[0], None, &opts, &mut |_, _| {}).unwrap_or_else(|e| panic!("{e}")).path
}

fn ffmpeg_available() -> bool {
    std::process::Command::new(sr_media::ffmpeg()).arg("-version").output().is_ok()
}

#[test]
fn the_language_of_the_assets_in_the_mix_tags_the_audio_stream() {
    if !ffmpeg_available() {
        return;
    }
    // one language across the mix (a regional tag is written as its ISO 639-2 language)
    assert_eq!(language_of(&deliver("one", r#"language="pt-BR""#, r#"language="pt""#)), "por");
    // an asset without a language does not change it
    assert_eq!(language_of(&deliver("unset", r#"language="en""#, "")), "eng");
}

#[test]
fn a_mix_of_languages_is_mul_and_none_is_und() {
    if !ffmpeg_available() {
        return;
    }
    // several languages are ISO 639-2 `mul`, none is `und`
    assert_eq!(language_of(&deliver("mixed", r#"language="en""#, r#"language="de""#)), "mul");
    let none = language_of(&deliver("none", "", ""));
    assert!(none.is_empty() || none == "und", "{none}");
}

#[test]
fn the_selected_dub_language_tags_a_segmented_output() {
    if !ffmpeg_available() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("sr-audio-language-selected-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    write_tone(&dir.join("a.wav"));
    write_tone(&dir.join("b.wav"));
    let xml = r#"<scene version="1.2"><project width="64" height="36" fps="10" duration="1"/>
<output path="selected.m4a" codec="audio-only" representation="de-AT" audioTracks="ta"><segment from="0" to="1"/></output>
<assets><audio id="a" src="a.wav" language="en"><representation name="dub" src="b.wav" language="de"/></audio>
<audio id="b" src="b.wav" language="fr"/></assets><composition/>
<audioMix><audioTrack id="ta" asset="a"/><audioTrack id="tb" asset="b"/></audioMix></scene>"#;
    let path = dir.join("scene.xml");
    std::fs::write(&path, xml).unwrap();
    let doc = sr_model::load_file(path, &Default::default()).unwrap();
    let result = sr_deliver::deliver(&doc, &doc.scene.outputs[0], None, &Default::default(), &mut |_, _| {}).unwrap();
    assert_eq!(language_of(&result.path), "deu");
}
