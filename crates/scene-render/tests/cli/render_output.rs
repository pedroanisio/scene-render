//! `render` writes its PNG frames from the output stage's conversion and reports the seconds its stages took.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::common::no_gpu;

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_scene-render")).args(args).env("NO_COLOR", "1").output().expect("binary runs")
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr-cli-render-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A transparent 48x32 programme: a half-transparent grey on the left, an opaque blue on the right, nothing between.
const SCENE: &str = r##"<scene version="1.2"><project width="48" height="32" fps="10" duration="0.3" background="#00000000"/>
<composition>
  <shape id="grey" shape="rect" x="0" y="0" width="16" height="32" fill="#808080" opacity="0.5"/>
  <shape id="blue" shape="rect" x="32" y="0" width="16" height="32" fill="#3366CC"/>
</composition></scene>"##;

fn png8(path: &Path) -> image::RgbaImage {
    image::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())).into_rgba8()
}

fn png16(path: &Path) -> image::ImageBuffer<image::Rgba<u16>, Vec<u16>> {
    image::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())).into_rgba16()
}

#[test]
fn render_writes_the_output_stages_frames_and_reports_its_stage_seconds() {
    let d = dir("stages");
    let scene = d.join("s.scene.xml");
    std::fs::write(&scene, SCENE).unwrap();
    let scene = scene.to_str().unwrap();
    let eight = d.join("f_%02d.png");
    let o = run(&["render", scene, "--frames", "0..3", "-o", eight.to_str().unwrap(), "--json"]);
    if no_gpu(&o) {
        return;
    }
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    let r = &v["result"];
    assert_eq!(r["written"].as_array().unwrap().len(), 3, "{r:#}");
    assert_eq!(r["rendered"], 3, "{r:#}");
    // the seconds the frames' stages took, as an encode reports them: evaluation, render, the wait for the
    // readback of a converted frame, and writing the files
    let stages = r["stage_seconds"].as_array().unwrap_or_else(|| panic!("stage_seconds: {r:#}"));
    assert_eq!(stages.len(), 4, "{r:#}");
    for s in stages {
        assert!(s.as_f64().is_some_and(|s| s >= 0.0), "{r:#}");
    }
    assert!(stages[3].as_f64().unwrap() > 0.0, "writing three files takes time: {r:#}");

    let sixteen = d.join("g_%02d.png");
    let o = run(&["render", scene, "--frames", "0..3", "-o", sixteen.to_str().unwrap(), "--bit-depth", "16", "--json"]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));

    for k in 0..3 {
        let a = png8(&d.join(format!("f_{k:02}.png")));
        let b = png16(&d.join(format!("g_{k:02}.png")));
        assert_eq!((a.width(), a.height()), (48, 32));
        assert_eq!((b.width(), b.height()), (48, 32));
        // straight alpha, display sRGB: the grey keeps its colour at half opacity, the blue is the literal,
        // the background is clear
        let near = |got: [u8; 4], want: [u8; 4]| got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= 1);
        let grey = a.get_pixel(8, 16).0;
        assert!(near(grey, [128, 128, 128, 128]), "frame {k}: grey {grey:?}");
        let blue = a.get_pixel(40, 16).0;
        assert!(near(blue, [0x33, 0x66, 0xCC, 255]), "frame {k}: blue {blue:?}");
        assert_eq!(a.get_pixel(24, 16).0, [0, 0, 0, 0], "frame {k}: clear between the shapes");
        let grey16 = b.get_pixel(8, 16).0;
        let want16 = [32896u16, 32896, 32896, 32768];
        assert!(grey16.iter().zip(want16).all(|(&v, w)| v.abs_diff(w) <= 40), "frame {k}: 16-bit grey {grey16:?}");
        assert!(b.get_pixel(40, 16).0[2].abs_diff(0xCCCC) <= 40, "frame {k}: 16-bit blue {:?}", b.get_pixel(40, 16).0);
        // the 8-bit frame is the 16-bit frame rounded, pixel for pixel
        for (p8, p16) in a.pixels().zip(b.pixels()) {
            for c in 0..4 {
                let rounded = ((p16.0[c] as u32 + 128) / 257) as u8;
                assert!(p8.0[c].abs_diff(rounded) <= 1, "frame {k}: 8-bit {:?} against 16-bit {:?}", p8.0, p16.0);
            }
        }
    }
}

#[test]
fn render_fails_cleanly_when_a_frame_cannot_be_written() {
    let d = dir("unwritable");
    let scene = d.join("s.scene.xml");
    std::fs::write(&scene, SCENE).unwrap();
    // the second frame's file name is a directory
    let pattern = d.join("f_{frame}.png");
    std::fs::create_dir_all(d.join("f_1.png")).unwrap();
    let o = run(&["render", scene.to_str().unwrap(), "--frames", "0..3", "-o", pattern.to_str().unwrap(), "--json"]);
    if no_gpu(&o) {
        return;
    }
    assert_eq!(o.status.code(), Some(2), "{}", String::from_utf8_lossy(&o.stderr));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["error"]["kind"], "io", "{v:#}");
    assert!(v["error"]["message"].as_str().unwrap().contains("f_1.png"), "{v:#}");
    assert!(d.join("f_0.png").is_file(), "the frame before the failure is written");
    assert!(!d.join("f_2.png").exists(), "no frame after the failure is written");
}
