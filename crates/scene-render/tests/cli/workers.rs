//! `encode --workers` with a `serve` on this machine: the chunks a worker renders over the link join into the
//! file, and a worker with another token is left out.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};

use crate::common::{ffmpeg, no_gpu};

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr-cli-workers-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Six seconds at 10 fps with an image beside the document, so the scene has a file to send.
fn scene(d: &Path) -> PathBuf {
    let img = image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 128, 0, 255]));
    std::fs::create_dir_all(d.join("assets")).unwrap();
    img.save(d.join("assets/tile.png")).unwrap();
    let xml = r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="6" background="#101010"/>
<assets><image id="tile" src="assets/tile.png" width="8" height="8"/></assets>
<composition>
  <shape id="w" shape="rect" width="8" height="8" y="4" fill="#FFFFFF">
    <animate property="x"><key time="0" value="0"/><key time="6" value="56"/></animate>
  </shape>
  <layer id="t" asset="tile"/>
</composition></scene>"##;
    let path = d.join("s.scene.xml");
    std::fs::write(&path, xml).unwrap();
    path
}

/// A worker serving one coordinator, and the address it listens on.
fn worker(cache: &Path, token: &str) -> (Child, String) {
    worker_at_once(cache, token, 1)
}

/// A worker that renders `at_once` chunks at once.
fn worker_at_once(cache: &Path, token: &str, at_once: usize) -> (Child, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_scene-render"))
        .args(["serve", "--listen", "127.0.0.1:0", "--once", "--parallel", &at_once.to_string(), "--cache"])
        .arg(cache)
        .env("SR_WORKER_TOKEN", token)
        .env("NO_COLOR", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the worker starts");
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    let address = line
        .split("listening on ")
        .nth(1)
        .and_then(|r| r.split(',').next())
        .unwrap_or_else(|| panic!("{line:?}"))
        .to_string();
    (child, address)
}

fn encode(scene: &Path, out: &Path, address: &str, token: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_scene-render"))
        .args(["encode", scene.to_str().unwrap(), "-o", out.to_str().unwrap(), "--codec", "ffv1", "--parallel", "1"])
        .args(["--workers", address, "--json"])
        .env("SR_WORKER_TOKEN", token)
        .env("NO_COLOR", "1")
        .output()
        .expect("binary runs")
}

fn decode(path: &Path) -> Vec<u8> {
    Command::new(sr_media::ffmpeg())
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .unwrap()
        .stdout
}

#[test]
fn a_worker_on_the_network_renders_chunks_that_join_into_the_serial_frames() {
    if !ffmpeg("encode --workers") {
        return;
    }
    let d = dir("joined");
    let scene = scene(&d);
    // the frames a render without workers gives
    let alone = Command::new(env!("CARGO_BIN_EXE_scene-render"))
        .args(["encode", scene.to_str().unwrap(), "-o", d.join("alone.mkv").to_str().unwrap(), "--codec", "ffv1"])
        .args(["--parallel", "1"])
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    if no_gpu(&alone) {
        return;
    }
    assert_eq!(alone.status.code(), Some(0), "{}", String::from_utf8_lossy(&alone.stderr));
    let reference = decode(&d.join("alone.mkv"));
    assert_eq!(reference.len(), 60 * 64 * 36 * 3);

    let (mut child, address) = worker(&d.join("cache"), "one token");
    let o = encode(&scene, &d.join("shared.mkv"), &address, "one token");
    let err = String::from_utf8_lossy(&o.stderr).into_owned();
    assert_eq!(o.status.code(), Some(0), "{err}");
    assert!(err.contains("ready"), "{err}");
    assert!(decode(&d.join("shared.mkv")) == reference, "the frames joined from the worker's chunks differ");
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    let remote = v["result"]["outputs"][0]["remote"].as_array().unwrap_or_else(|| panic!("{v:#}"));
    assert_eq!(remote.len(), 1, "{v:#}");
    assert!(remote[0]["chunks"].as_u64().unwrap() >= 1 && remote[0]["failed"].is_null(), "{v:#}");
    assert_eq!(remote[0]["frames"].as_u64().unwrap(), 20 * remote[0]["chunks"].as_u64().unwrap(), "{v:#}");
    // the worker served its one coordinator and ended; what it was sent is kept by digest, and laid out as a scene
    assert!(child.wait().unwrap().success());
    let kept = std::fs::read_dir(d.join("cache/files")).unwrap().count();
    assert_eq!(kept, 2, "the document and its image");
    let scenes: Vec<_> = std::fs::read_dir(d.join("cache/scenes")).unwrap().flatten().collect();
    assert_eq!(scenes.len(), 1);
    assert!(scenes[0].path().join("s.scene.xml").is_file() && scenes[0].path().join("assets/tile.png").is_file());
    assert_eq!(std::fs::read_dir(d.join("cache/chunks")).unwrap().count(), 0, "a sent chunk is not kept");
}

#[test]
fn a_worker_that_renders_two_chunks_at_once_is_given_two_links() {
    if !ffmpeg("encode --workers") {
        return;
    }
    let d = dir("two");
    let scene = scene(&d);
    let (mut child, address) = worker_at_once(&d.join("cache"), "one token", 2);
    let o = encode(&scene, &d.join("shared.mkv"), &address, "one token");
    if no_gpu(&o) {
        let _ = child.kill();
        let _ = child.wait();
        return;
    }
    let err = String::from_utf8_lossy(&o.stderr).into_owned();
    assert_eq!(o.status.code(), Some(0), "{err}");
    assert!(err.contains("ready, 2 chunk(s) at once"), "{err}");
    // one worker here and two links: three chunks of two seconds, and the links take at least the last two
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    let remote = v["result"]["outputs"][0]["remote"].as_array().unwrap_or_else(|| panic!("{v:#}"));
    assert_eq!(remote.len(), 2, "{v:#}");
    assert!(remote[0]["worker"].as_str().unwrap().ends_with("#1"), "{v:#}");
    assert!(remote[1]["worker"].as_str().unwrap().ends_with("#2"), "{v:#}");
    assert!(remote.iter().all(|r| r["failed"].is_null()), "{v:#}");
    assert!(remote.iter().map(|r| r["chunks"].as_u64().unwrap()).sum::<u64>() >= 2, "{v:#}");
    assert_eq!(decode(&d.join("shared.mkv")).len(), 60 * 64 * 36 * 3);
    assert!(child.wait().unwrap().success());
}

#[test]
fn a_worker_with_another_token_is_left_out_and_the_encode_renders_alone() {
    if !ffmpeg("encode --workers") {
        return;
    }
    let d = dir("token");
    let scene = scene(&d);
    let (mut child, address) = worker(&d.join("cache"), "the worker's token");
    let o = encode(&scene, &d.join("alone.mkv"), &address, "another token");
    let _ = child.kill();
    let _ = child.wait();
    if no_gpu(&o) {
        return;
    }
    let err = String::from_utf8_lossy(&o.stderr).into_owned();
    assert_eq!(o.status.code(), Some(0), "{err}");
    assert!(err.contains("is left out") && err.contains("do not hold the same token"), "{err}");
    assert!(err.contains("rendering here alone"), "{err}");
    assert_eq!(decode(&d.join("alone.mkv")).len(), 60 * 64 * 36 * 3);
    // nothing reached the worker's cache
    assert_eq!(std::fs::read_dir(d.join("cache/files")).unwrap().count(), 0);
}

#[test]
fn workers_need_a_token_and_serve_says_so_too() {
    let d = dir("no-token");
    let scene = scene(&d);
    let o = Command::new(env!("CARGO_BIN_EXE_scene-render"))
        .args(["encode", scene.to_str().unwrap(), "-o", d.join("x.mkv").to_str().unwrap(), "--workers", "127.0.0.1:1"])
        .env_remove("SR_WORKER_TOKEN")
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    // without FFmpeg or a GPU the encode stops earlier, for that reason
    let err = String::from_utf8_lossy(&o.stderr).into_owned();
    if !err.contains("no GPU adapter") {
        assert_eq!(o.status.code(), Some(2), "{err}");
        assert!(err.contains("SR_WORKER_TOKEN") && err.contains("--token-file"), "{err}");
    }
    let o = Command::new(env!("CARGO_BIN_EXE_scene-render"))
        .args(["serve", "--listen", "127.0.0.1:0"])
        .env_remove("SR_WORKER_TOKEN")
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(String::from_utf8_lossy(&o.stderr).contains("SR_WORKER_TOKEN"));
}
