//! Scopes of a decoded video: limited-range code values outside 16–235 are illegal in every frame.

mod common;

use std::process::Command;

use sr_media::scopes::{analyse, Matrix};

#[test]
fn super_white_video_is_illegal_in_every_frame() {
    if !common::ffmpeg_available("super_white_video_is_illegal_in_every_frame") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("sr-media-scopes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let clip = dir.join("super-white.mkv");
    // Y' code 250 (above the 235 white) with neutral chroma, 25 frames
    let made = Command::new(sr_media::ffmpeg())
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=64x36:r=25:d=1,format=yuv420p,geq=lum=250:cb=128:cr=128",
            "-c:v",
            "ffv1",
        ])
        .arg(&clip)
        .status()
        .is_ok_and(|s| s.success());
    if !made {
        common::skip_ffmpeg("super_white_video_is_illegal_in_every_frame", "ffmpeg could not create the fixture");
        return;
    }

    let r = analyse(&clip, 1, None).unwrap();
    assert_eq!((r.source.kind, r.source.width, r.source.height, r.source.depth), ("video", 64, 36, 8));
    assert_eq!((r.source.range, r.source.matrix, r.frames.analysed), (Some("limited"), Matrix::Bt601, 25));
    let legal = r.levels.legal.unwrap();
    assert_eq!((legal.y, legal.above.y, legal.below.y, legal.above.cb), ([16, 235], 1.0, 0.0, 0.0));
    assert_eq!(r.levels.illegal, Some(1.0));
    assert_eq!(r.levels.clipping.white.y, 1.0);
    assert_eq!(r.worst.illegal.unwrap(), sr_media::scopes::Worst { frame: 0, value: 1.0 });
    assert_eq!(r.levels.samples, 64 * 36 * 25);

    // every tenth frame: 0, 10 and 20
    let r = analyse(&clip, 10, None).unwrap();
    assert_eq!((r.frames.analysed, r.frames.step, r.levels.samples), (3, 10, 64 * 36 * 3));
    std::fs::remove_dir_all(&dir).ok();
}
