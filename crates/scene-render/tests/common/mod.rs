//! What the binary's integration tests skip without: FFmpeg for an encode, a GPU adapter for a render.

#![allow(dead_code)]

use std::process::Output;

/// Whether `ffmpeg` and `ffprobe` run (`SR_FFMPEG` and `SR_FFPROBE` name them), which every encode needs, a PNG
/// sequence included; if not, skips `context`, or fails under `SR_REQUIRE_FFMPEG=1`.
pub fn ffmpeg(context: &str) -> bool {
    sr_testkit::ffmpeg(context, &[sr_media::ffmpeg(), sr_media::ffprobe()])
}

/// Whether the binary stopped (exit 2) because it found no GPU adapter; if so, skips the running test, or fails under
/// `SR_REQUIRE_GPU=1`. An adapter name that matches nothing ("no GPU adapter matches …") is an error, not a skip.
pub fn no_gpu(o: &Output) -> bool {
    let e = String::from_utf8_lossy(&o.stderr);
    let none = o.status.code() == Some(2) && e.contains("no GPU adapter is available");
    if none {
        sr_testkit::GPU.skip(std::thread::current().name().unwrap_or("scene-render"), e.trim());
    }
    none
}
