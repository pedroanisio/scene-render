//! `scopes` measures a still: clipping per channel, chroma above the limit, the JSON report and the exit status.
//! `--format json` wraps the report in the envelope scene-render-cli/1 as its `result`.

use std::path::PathBuf;
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_scene-render")).args(args).env("NO_COLOR", "1").output().expect("binary runs")
}

/// The scopes report of a successful `--format json` run: the `result` of its one-line envelope.
fn report(o: &Output) -> serde_json::Value {
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let text = String::from_utf8_lossy(&o.stdout);
    assert_eq!(text.lines().count(), 1, "one envelope line: {text}");
    let mut v: serde_json::Value = serde_json::from_str(&text).expect("json");
    assert_eq!(v["format"], "scene-render-cli/1", "{v}");
    assert_eq!((&v["command"], &v["ok"], &v["exit_code"]), (&"scopes".into(), &true.into(), &0.into()), "{v}");
    v["result"].take()
}

/// A 4×2 PNG whose columns are white, black, 100 % red and mid grey.
fn still(tag: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("sr-cli-scopes-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("bars.png");
    let columns = [[255u8, 255, 255], [0, 0, 0], [255, 0, 0], [128, 128, 128]];
    image::RgbImage::from_fn(4, 2, |x, _| image::Rgb(columns[x as usize])).save(&file).unwrap();
    (dir, file)
}

#[test]
fn json_reports_clipping_and_chroma_of_a_still() {
    let (dir, file) = still("json");
    let v = report(&run(&["scopes", file.to_str().unwrap(), "--format", "json"]));
    assert_eq!(v["schema"], "scene-render-scopes/1");
    assert_eq!(v["source"]["kind"], "still");
    assert_eq!((v["source"]["width"].as_u64(), v["source"]["height"].as_u64()), (Some(4), Some(2)));
    assert_eq!(v["frames"]["analysed"], 1);
    assert_eq!(v["levels"]["samples"], 8);
    let clip = &v["levels"]["clipping"];
    // white and red reach 1 in R'; black and red reach 0 in G' and B'
    let rgb = |c: &serde_json::Value| ["r", "g", "b"].map(|k| c[k].as_f64());
    assert_eq!(rgb(&clip["white"]), [Some(0.5), Some(0.25), Some(0.25)], "{clip}");
    assert_eq!(rgb(&clip["black"]), [Some(0.25), Some(0.5), Some(0.5)], "{clip}");
    // 8-bit RGB has no legal range
    assert!(v["levels"]["illegal"].is_null(), "{v}");
    assert!(v["levels"]["legal"].is_null(), "{v}");
    // 100 % red (chroma 0.513 in BT.709) is above the 75 % bar limit; nothing leaves the gamut
    let sat = &v["levels"]["saturation"];
    assert_eq!((sat["above_limit"].as_f64(), sat["out_of_gamut"].as_f64()), (Some(0.25), Some(0.0)));
    assert_eq!(v["worst"]["above_chroma_limit"], serde_json::json!({"frame": 0, "value": 0.25}));
    assert_eq!(v["worst"]["white_clipped"], serde_json::json!({"frame": 0, "value": 0.5}));

    // a looser limit passes the red
    let v = report(&run(&["scopes", file.to_str().unwrap(), "--format", "json", "--chroma-limit", "0.6"]));
    assert_eq!(v["levels"]["saturation"]["above_limit"].as_f64(), Some(0.0));
    assert_eq!(v["levels"]["saturation"]["chroma_limit"].as_f64().map(|c| (c * 1e6).round() / 1e6), Some(0.6));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn human_output_lists_the_clipping_per_channel() {
    let (dir, file) = still("human");
    let o = run(&["scopes", file.to_str().unwrap()]);
    let text = String::from_utf8_lossy(&o.stdout);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(text.contains("still 4×2"), "{text}");
    assert!(text.contains("white clipped  R 50.00% G 25.00% B 25.00%"), "{text}");
    assert!(text.contains("black clipped  R 25.00% G 50.00% B 50.00%"), "{text}");
    assert!(text.contains("illegal        n/a"), "{text}");
    assert!(text.contains("worst above chroma: frame 0 (25.00%)"), "{text}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn unreadable_files_and_bad_settings_exit_two() {
    let missing = std::env::temp_dir().join(format!("sr-cli-scopes-missing-{}.png", std::process::id()));
    let o = run(&["scopes", missing.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("error: cannot read"), "{o:?}");
    // in JSON mode the failure is the envelope's error, of kind io
    let o = run(&["scopes", missing.to_str().unwrap(), "--format", "json"]);
    assert_eq!(o.status.code(), Some(2));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).expect("json");
    assert_eq!((&v["command"], &v["ok"], &v["error"]["kind"]), (&"scopes".into(), &false.into(), &"io".into()), "{v}");
    assert!(v["result"].is_null(), "{v}");

    let (dir, file) = still("settings");
    for bad in [&["--step", "0"][..], &["--chroma-limit", "0"], &["--chroma-limit", "NaN"], &["--format", "xml"]] {
        let mut args = vec!["scopes", file.to_str().unwrap()];
        args.extend_from_slice(bad);
        let o = run(&args);
        assert_eq!(o.status.code(), Some(2), "{bad:?}: {o:?}");
        assert!(o.stdout.is_empty(), "{bad:?}");
    }
    std::fs::remove_dir_all(&dir).ok();
}
