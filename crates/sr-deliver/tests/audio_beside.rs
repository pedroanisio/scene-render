//! A picture that reads nothing of the mix renders while the audio chain runs; the file is the one the chain would
//! have given first, frame for frame and packet for packet.

mod common;
use std::path::Path;
use std::process::Command;

fn ffmpeg(args: &[&str]) -> bool {
    Command::new(sr_media::ffmpeg())
        .args(["-v", "error", "-nostdin", "-y"])
        .args(args)
        .status()
        .is_ok_and(|s| s.success())
}

/// What FFmpeg makes of one kind of stream of `path`, copied out unchanged.
fn md5(path: &Path, map: &str, extra: &[&str]) -> String {
    let out = Command::new(sr_media::ffmpeg())
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-map", map])
        .args(extra)
        .args(["-f", if map == "0:v" { "framemd5" } else { "md5" }, "-"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().lines().filter(|l| !l.starts_with('#')).collect::<Vec<_>>().join("\n")
}

/// A 20 s moving picture over a tone, master normalised (so the AAC ceiling applies); `reads` makes an expression read
/// the track's analysis without changing a pixel.
fn scene(dir: &Path, name: &str, attrs: &str, reads: bool) -> sr_model::Document {
    let reading =
        if reads { r#"<expression property="opacity">1 + 0 * audioAmplitude("music")</expression>"# } else { "" };
    let xml = format!(
        r##"<scene version="1.1"><project width="64" height="36" fps="10" duration="20" background="#101820"/>
        <output id="main" path="out/{name}.mp4" codec="h264" preset="ultrafast" {attrs}/>
        <assets><audio id="musicA" src="tone.wav"/></assets>
        <composition><shape id="s" shape="rect" width="20" height="12" fill="#FF8800"><expression property="x">10 + time * 2</expression>{reading}</shape></composition>
        <audioMix sampleRate="48000"><audioTrack id="music" asset="musicA"/><master normalize="integrated" loudness="-16" truePeak="-1"/></audioMix></scene>"##
    );
    let path = dir.join(format!("{name}.xml"));
    std::fs::write(&path, xml).unwrap();
    sr_model::load_file(&path, &sr_model::LoadOptions::default()).unwrap_or_else(|e| panic!("{e:?}"))
}

fn deliver(gpu: &sr_gpu::Gpu, d: &sr_model::Document, parallel: sr_deliver::Parallel) -> sr_deliver::pipeline::Report {
    let opts = sr_deliver::Options { hardware: sr_media::encode::Hardware::Software, parallel, ..Default::default() };
    sr_deliver::deliver(d, &d.scene.outputs[0], Some(gpu), &opts, &mut |_, _| {}).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn a_picture_that_ignores_the_mix_renders_beside_it_and_delivers_the_same_file() {
    let ctx = "a_picture_that_ignores_the_mix_renders_beside_it_and_delivers_the_same_file";
    let Some(gpu) = common::delivery(ctx) else { return };
    if !common::encoder(ctx, "libx264") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("sr-audio-beside-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    assert!(ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=330:sample_rate=48000:duration=20:beep_factor=3",
        "-ac",
        "2",
        &dir.join("tone.wav").display().to_string()
    ]));
    // one stream, two chunks joined, and a lossless intermediate encoded again: the three places the audio is needed
    let modes = [
        ("single", sr_deliver::Parallel::Count(1), ""),
        ("chunked", sr_deliver::Parallel::Count(2), ""),
        ("twopass", sr_deliver::Parallel::Count(1), r#"twoPass="true" bitrate="200000""#),
    ];
    for (mode, parallel, attrs) in modes {
        let free = deliver(&gpu, &scene(&dir, &format!("free-{mode}"), attrs, false), parallel);
        let read = deliver(&gpu, &scene(&dir, &format!("read-{mode}"), attrs, true), parallel);
        assert!(free.audio_beside_picture, "{mode}: nothing the picture draws reads the mix");
        assert!(!read.audio_beside_picture, "{mode}: an expression reads the track's analysis, so the mix comes first");
        assert_eq!(free.segments > 1, mode == "chunked", "{mode}: {} segments", free.segments);
        assert!(free.audio_ceiling.is_some() && read.audio_ceiling.is_some(), "{mode}: the ceiling was held");
        assert_eq!(free.audio_languages, read.audio_languages, "{mode}");
        assert_eq!(
            (free.loudness, free.true_peak),
            (read.loudness, read.true_peak),
            "{mode}: the mix is measured alike"
        );
        let (a, b) = (dir.join(format!("out/free-{mode}.mp4")), dir.join(format!("out/read-{mode}.mp4")));
        assert_eq!(md5(&a, "0:v", &[]), md5(&b, "0:v", &[]), "{mode}: frames");
        assert_eq!(md5(&a, "0:a", &["-c", "copy"]), md5(&b, "0:a", &["-c", "copy"]), "{mode}: audio packets");
    }
}

#[test]
fn an_audio_only_output_does_not_overlap_anything() {
    let ctx = "an_audio_only_output_does_not_overlap_anything";
    let Some(gpu) = common::delivery(ctx) else { return };
    let dir = std::env::temp_dir().join(format!("sr-audio-beside-only-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    assert!(ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=330:sample_rate=48000:duration=2",
        &dir.join("tone.wav").display().to_string()
    ]));
    let d = scene(&dir, "only", "", false);
    let o = sr_deliver::adhoc_output(&dir.join("out/only.m4a").display().to_string(), "audio-only").unwrap();
    let opts = sr_deliver::Options { hardware: sr_media::encode::Hardware::Software, ..Default::default() };
    let report = sr_deliver::deliver(&d, &o, Some(&gpu), &opts, &mut |_, _| {}).unwrap_or_else(|e| panic!("{e}"));
    assert!(!report.audio_beside_picture && report.audio_ceiling.is_some());
}
