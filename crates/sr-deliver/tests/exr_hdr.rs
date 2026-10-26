//! An OpenEXR output holds the scene-linear frame: values above 1, here a star field's brightest pixels, reach the
//! file whole, with or without alpha and with `twoPass`; `maxFileSize` asks for a bitrate a sequence has none of.

mod common;
use std::path::{Path, PathBuf};

/// The digest of the Bright Star Catalogue fixture (`sr_space::stars::CATALOGUE_DIGEST`).
const CATALOGUE_DIGEST: &str = "cd18b1c9b8c009d922edfed020ba718aa14a577416fb625bf01ddd9cbdaab617";

fn media() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/corpus/media"))
}

fn directory(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sr-exr-hdr-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Sirius at the centre of a 10° view, bright enough that its disc's pixels exceed 1, into `output`.
fn document(dir: &Path, output: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.8"><project width="64" height="48" fps="1" duration="1" background="#000000FF"/><output path="{}/f_%04d.exr" codec="exr-sequence" audio="false" {output}/><assets><catalogue id="cat" src="bsc5-compact.csv.gz" digest="sha256:{CATALOGUE_DIGEST}"/></assets><composition><camera id="eye" x="0" y="0" z="0" fov="10" yaw="-11.28708" pitch="-16.71611"/></composition><lights><light id="dome" type="dome"><starField catalogue="cat" limitMagnitude="-1" magnitudeZero="5" pointSize="3"/></light></lights></scene>"##,
        dir.display()
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(media()) };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}"))
}

/// The float planes of the first frame of `dir`, as `pix`.
fn planes(dir: &Path, pix: &str) -> Vec<f32> {
    let out = std::process::Command::new(sr_media::ffmpeg())
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(dir.join("f_0000.exr"))
        .args(["-f", "rawvideo", "-pix_fmt", pix, "-"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    out.stdout.chunks(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect()
}

#[test]
fn exr_outputs_keep_values_above_one() {
    let Some(gpu) = common::delivery("exr outputs keep values above one") else { return };
    let opts = sr_deliver::Options { hardware: sr_media::encode::Hardware::Software, ..Default::default() };
    let mut peaks = Vec::new();
    for (name, attrs, pix) in [
        ("plain", "", "gbrpf32le"),
        ("alpha", r#"alpha="true""#, "gbrapf32le"),
        ("two-pass", r#"twoPass="true""#, "gbrpf32le"),
    ] {
        let dir = directory(name);
        let doc = document(&dir, attrs);
        sr_deliver::deliver(&doc, &doc.scene.outputs[0], Some(&gpu), &opts, &mut |_, _| {})
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let v = planes(&dir, pix);
        let n = 64 * 48;
        // G, B and R; Sirius is blue-white, so its blue exceeds 1 most
        let peak = (0..3).map(|p| v[p * n..(p + 1) * n].iter().fold(0f32, |m, &x| m.max(x))).collect::<Vec<_>>();
        assert!(peak[1] > 2.0, "{name}: the brightest blue is {peak:?}");
        peaks.push(peak);
    }
    // the three files hold one picture
    assert!(peaks.windows(2).all(|w| w[0] == w[1]), "{peaks:?}");
}

#[test]
fn a_sequence_has_no_bitrate_to_fit_a_file_size() {
    let Some(gpu) = common::delivery("a sequence has no bitrate to fit a file size") else { return };
    let opts = sr_deliver::Options { hardware: sr_media::encode::Hardware::Software, ..Default::default() };
    let dir = directory("fit");
    let doc = document(&dir, r#"maxFileSize="1000000""#);
    let err = sr_deliver::deliver(&doc, &doc.scene.outputs[0], Some(&gpu), &opts, &mut |_, _| {}).unwrap_err();
    assert!(err.to_string().contains("maxFileSize"), "{err}");
    assert!(!dir.join("f_0000.exr").exists(), "nothing is rendered");
}
