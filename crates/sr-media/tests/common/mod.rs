//! What the integration tests of this crate share: whether FFmpeg is here.
#![allow(dead_code)]

use std::process::Command;

/// Whether a missing `ffmpeg`, `ffprobe` or encoder fails the tests that need it instead of skipping them:
/// `SR_REQUIRE_FFMPEG=1`, as in CI.
pub fn ffmpeg_required() -> bool {
    std::env::var("SR_REQUIRE_FFMPEG").as_deref() == Ok("1")
}

/// Skips what `context` needs FFmpeg for, saying why, or fails under `SR_REQUIRE_FFMPEG=1`.
pub fn skip_ffmpeg(context: &str, reason: &str) {
    assert!(!ffmpeg_required(), "{context}: {reason} (SR_REQUIRE_FFMPEG=1 requires FFmpeg)");
    eprintln!("SKIPPED ({context}): {reason}");
}

/// Whether `ffmpeg` and `ffprobe` run (`SR_FFMPEG` and `SR_FFPROBE` name them); if not, skips with
/// [`skip_ffmpeg`].
pub fn ffmpeg_available(context: &str) -> bool {
    for tool in [sr_media::ffmpeg(), sr_media::ffprobe()] {
        if !Command::new(&tool).arg("-version").output().is_ok_and(|o| o.status.success()) {
            skip_ffmpeg(context, &format!("cannot run {tool}"));
            return false;
        }
    }
    true
}

/// Whether FFmpeg runs and its `name` encoder encodes; if not, skips with [`skip_ffmpeg`].
pub fn encoder(context: &str, name: &str) -> bool {
    if !ffmpeg_available(context) {
        return false;
    }
    if !sr_media::encode::working_encoder(name) {
        skip_ffmpeg(context, &format!("FFmpeg has no working {name} encoder"));
        return false;
    }
    true
}
