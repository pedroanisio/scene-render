//! GPU and FFmpeg availability is mandatory in renderer CI and optional on developer machines.
#![allow(dead_code)]
use std::sync::OnceLock;
/// The adapter, created once; without one, each caller skips (or fails under `SR_REQUIRE_GPU=1`, as in CI).
pub fn gpu() -> Option<sr_gpu::Gpu> {
    static GPU: OnceLock<Result<sr_gpu::Gpu, String>> = OnceLock::new();
    sr_testkit::gpu(GPU.get_or_init(|| sr_gpu::Gpu::new().map_err(|e| e.to_string())).clone(), "GPU tests")
}

/// Skips what `context` needs FFmpeg for, saying why, or fails under `SR_REQUIRE_FFMPEG=1`.
pub fn skip_ffmpeg(context: &str, reason: &str) {
    sr_testkit::FFMPEG.skip(context, reason);
}

/// Whether `ffmpeg` and `ffprobe` run (`SR_FFMPEG` and `SR_FFPROBE` name them); if not, skips with [`skip_ffmpeg`].
pub fn ffmpeg_available(context: &str) -> bool {
    sr_testkit::ffmpeg(context, &[sr_media::ffmpeg(), sr_media::ffprobe()])
}

/// Whether FFmpeg runs and its `name` encoder encodes; if not, skips with [`skip_ffmpeg`].
pub fn encoder(context: &str, name: &str) -> bool {
    sr_testkit::ffmpeg_encoder(context, &[sr_media::ffmpeg(), sr_media::ffprobe()], name, || {
        sr_media::encode::working_encoder(name)
    })
}
