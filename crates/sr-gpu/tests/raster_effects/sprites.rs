//! R32 admits image sequences, videos and generators for particles and flocks.
use super::common::*;
use std::sync::Arc;

fn sprite_doc(kind: &str, asset: &str, extra: &str) -> sr_model::Document {
    let node = if kind == "flock" {
        String::from(r#"<flock id="p" width="32" height="32" count="3" start="0.5" shape="sprite" sprite="s"/>"#)
    } else {
        String::from(
            r#"<particleEmitter id="p" start="0.5" shape="sprite" sprite="s" rate="0" lifetime="3" spriteFps="99"><burst time="0.5" count="3"/></particleEmitter>"#,
        )
    };
    let xml = format!(
        r##"<scene version="1.2"><project width="32" height="32" fps="10" duration="4" background="#000000"/>
      <assets>{asset}</assets><composition>{node}</composition>{extra}</scene>"##
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: false, base_dir: Some(fixtures()) }).unwrap()
}

/// Hold positions fixed so texture sampling and blend order are observed independently of dynamics.
fn sample(
    d: &sr_model::Document,
    time: f64,
    ages: &[f64],
    overlap: bool,
) -> Option<(Vec<[f32; 4]>, sr_gpu::RenderStats)> {
    let gpu = gpu()?;
    let ev = sr_eval::Evaluator::new(d, &Default::default()).unwrap();
    let mut frame = ev.evaluate(time);
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    let n = frame.nodes.iter_mut().find(|n| &*n.id == "p").unwrap();
    let pf = Arc::make_mut(n.particles.as_mut().unwrap());
    assert_eq!(pf.pos.len(), ages.len());
    pf.age.resize(ages.len(), 0.0);
    for (i, &age) in ages.iter().enumerate() {
        pf.pos[i] = [if overlap { 16.0 } else { 5.0 + 10.0 * i as f32 }, 16.0];
        pf.vel[i] = [0.0; 2];
        pf.size[i] = 8.0;
        pf.rot[i] = 0.0;
        pf.alpha[i] = if overlap { 0.5 } else { 1.0 };
        pf.frame[i] = 0;
        pf.age[i] = age;
    }
    pf.trail = 0.0;
    let mut renderer = sr_gpu::Renderer::new(gpu, ev.program());
    let got = renderer.render(&frame, ev.program());
    Some((renderer.read(&got.texture), got.stats))
}

#[test]
fn sequence_sprites_loop_by_individual_age_at_asset_fps_and_preserve_paint_order() {
    for kind in ["particleEmitter", "flock"] {
        let d = sprite_doc(
            kind,
            r#"<imageSequence id="s" src="seq_%04d.png" first="1" last="3" fps="10" width="2" height="2"/>"#,
            "",
        );
        let Some((px, stats)) = sample(&d, 1.0, &[0.21, 0.01, 0.41], false) else { return };
        assert!(stats.errors.is_empty(), "{:?}", stats.errors);
        for (i, v) in [240, 80, 160].into_iter().enumerate() {
            assert!(
                close(px[16 * 32 + 5 + 10 * i], [lin8(v), 0.0, 0.0, 1.0], 0.01),
                "{kind}: {:?}",
                px[16 * 32 + 5 + 10 * i]
            );
        }
        let (px, stats) = sample(&d, 1.0, &[0.21, 0.01, 0.11], true).unwrap();
        assert!(stats.errors.is_empty(), "{:?}", stats.errors);
        let expected = lin8(240) * 0.125 + lin8(80) * 0.25 + lin8(160) * 0.5;
        assert!((px[16 * 32 + 16][0] - expected).abs() < 0.01, "{kind}: {:?}, expected {expected}", px[16 * 32 + 16]);
    }
}

#[test]
fn generator_sprites_sample_the_layers_local_time() {
    for kind in ["particleEmitter", "flock"] {
        let d = sprite_doc(
            kind,
            r##"<generator id="s" kind="solid" width="8" height="8" paint="#000000">
          <animate property="paint"><key time="0" value="#000000"/><key time="1" value="#FFFFFF"/></animate>
        </generator>"##,
            "",
        );
        let Some((px, stats)) = sample(&d, 1.0, &[0.1, 0.2, 0.3], false) else { return };
        assert!(stats.errors.is_empty(), "{:?}", stats.errors);
        // start=0.5: at composition 1.0 the layer samples the generator at 0.5.
        let c = px[16 * 32 + 5][0];
        assert!(c > 0.15 && c < 0.65, "{kind}: {c}");
        assert_eq!(px[16 * 32 + 5], px[16 * 32 + 15]);
    }
}

#[test]
fn video_sprites_sample_layer_time_and_share_one_frame_across_ages() {
    let file = fixtures().join("sprite-counter.mkv");
    let status = std::process::Command::new(sr_media::ffmpeg()).args([
        "-v", "error", "-y", "-f", "lavfi", "-i",
        "color=c=black:s=16x16:r=10:d=3,format=yuv444p,geq=lum='16+N*8':cb=128:cr=128,setparams=range=tv:color_primaries=bt709:color_trc=bt709:colorspace=bt709",
        "-c:v", "ffv1", file.to_str().unwrap(),
    ]).status();
    if status.is_err() {
        eprintln!("FFmpeg unavailable");
        return;
    }
    assert!(status.unwrap().success());
    for kind in ["particleEmitter", "flock"] {
        let d = sprite_doc(
            kind,
            r#"<video id="s" src="sprite-counter.mkv" width="16" height="16" fps="10" duration="3" colorSpace="rec709"/>"#,
            "",
        );
        let Some((px, stats)) = sample(&d, 1.0, &[0.0, 0.1, 0.2], false) else { return };
        assert!(stats.errors.is_empty(), "{:?}", stats.errors);
        assert_eq!(stats.video_frames, 1);
        let expected = (40.0f32 / 219.0).powf(2.4);
        assert!((px[16 * 32 + 5][0] - expected).abs() < 0.004, "{kind}: {:?}", px[16 * 32 + 5]);
        assert_eq!(px[16 * 32 + 5], px[16 * 32 + 25]);
    }
}

#[test]
fn sequence_sprites_honour_missing_frame_policies() {
    for kind in ["particleEmitter", "flock"] {
        for policy in ["error", "transparent", "black", "hold"] {
            let d = sprite_doc(
                kind,
                &format!(
                    r#"<imageSequence id="s" src="seq_%04d.png" first="1" last="4" fps="10" width="2" height="2" missingFrame="{policy}"/>"#
                ),
                "",
            );
            let Some((px, stats)) = sample(&d, 1.0, &[0.31; 3], false) else { return };
            assert_eq!(!stats.errors.is_empty(), policy == "error", "{kind} {policy}: {:?}", stats.errors);
            let expected = if policy == "hold" { lin8(240) } else { 0.0 };
            assert!((px[16 * 32 + 5][0] - expected).abs() < 0.01, "{kind} {policy}: {:?}", px[16 * 32 + 5]);
        }
    }
}

#[test]
fn particle_and_flock_frames_carry_birth_age_through_seeks() {
    for kind in ["particleEmitter", "flock"] {
        let d = sprite_doc(
            kind,
            r#"<imageSequence id="s" src="seq_%04d.png" first="1" last="3" fps="10" width="2" height="2"/>"#,
            "",
        );
        let ev = sr_eval::Evaluator::new(&d, &Default::default()).unwrap();
        for t in [1.0, 0.75, 1.0] {
            let g = ev.evaluate(t);
            let pf = g.nodes.iter().find(|n| &*n.id == "p").unwrap().particles.as_ref().unwrap();
            assert_eq!(pf.age.len(), pf.pos.len());
            assert!(pf.age.iter().all(|a| (a - (t - 0.5)).abs() < 0.02), "{kind} {t}: {:?}", pf.age);
        }
    }
}
