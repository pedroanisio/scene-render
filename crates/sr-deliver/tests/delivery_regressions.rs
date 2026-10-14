mod common;
use std::path::PathBuf;

/// The adapter for a delivery: without it or FFmpeg, each test skips.
fn gpu() -> Option<sr_gpu::Gpu> {
    common::delivery("delivery regression tests")
}

fn directory(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sr-delivery-regression-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn plain_output_posters_include_overlay_crop_and_output_clock() {
    let Some(gpu) = gpu() else { return };
    let dir = directory("poster");
    let xml = r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="2" background="#000000"/>
      <output path="frame_%03d.png" codec="png-sequence" layout="square" overlay="tag" start="1" end="1.1" audio="false">
        <poster path="poster.png" format="png" time="1"/>
        <thumbnail path="thumbnail.png" format="png" time="1" width="18"/>
      </output>
      <layouts><layout id="square" width="36" height="36" reframe="crop" focusX="1"/></layouts>
      <symbols><symbol id="tag"><shape id="stamp" shape="rect" width="8" height="8" fill="#FFFFFF">
        <animate property="x"><key time="0" value="8"/><key time="1" value="24"/></animate>
      </shape></symbol></symbols>
      <composition><shape id="left" shape="rect" width="32" height="36" fill="#FF0000"/>
        <shape id="right" shape="rect" width="32" height="36" x="32" fill="#0000FF"/></composition></scene>"##;
    let path = dir.join("scene.xml");
    std::fs::write(&path, xml).unwrap();
    let doc = sr_model::load_file(path, &Default::default()).unwrap();
    sr_deliver::deliver(&doc, &doc.scene.outputs[0], Some(&gpu), &Default::default(), &mut |_, _| {}).unwrap();
    let frame = image::open(dir.join("frame_010.png")).unwrap().to_rgba8();
    let poster = image::open(dir.join("poster.png")).unwrap().to_rgba8();
    assert_eq!(poster.dimensions(), frame.dimensions());
    assert_eq!(frame.get_pixel(10, 2).0, [255; 4], "overlay starts at output time zero");
    // Each readback dithers independently; placement and colours agree within one level.
    assert!(
        poster.as_raw().iter().zip(frame.as_raw()).all(|(a, b)| a.abs_diff(*b) <= 1),
        "poster is the same delivered picture"
    );
    assert_eq!(image::open(dir.join("thumbnail.png")).unwrap().to_rgba8().dimensions(), (18, 18));
}

#[test]
fn overlay_uses_output_representation_and_cli_override_in_every_worker() {
    let Some(gpu) = gpu() else { return };
    for workers in [1, 2] {
        for cli in [false, true] {
            let dir = directory(&format!("proxy-{workers}-{cli}"));
            for (name, color) in
                [("original", [255, 0, 0, 255]), ("proxy", [0, 0, 255, 255]), ("preview", [0, 255, 0, 255])]
            {
                image::RgbaImage::from_pixel(4, 4, image::Rgba(color)).save(dir.join(format!("{name}.png"))).unwrap();
            }
            let xml = r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="0.2" background="#000000"/>
              <output path="clip.mkv" codec="ffv1" overlay="tag" representation="proxy" audio="false"/>
              <assets><image id="im" src="original.png" width="4" height="4">
                <representation name="proxy" src="proxy.png" width="4" height="4"/>
                <representation name="preview" src="preview.png" width="4" height="4"/>
              </image></assets>
              <symbols><symbol id="tag"><layer id="over" asset="im"/></symbol></symbols>
              <composition><layer id="main" asset="im" x="32"/></composition></scene>"##;
            let path = dir.join("scene.xml");
            std::fs::write(&path, xml).unwrap();
            let doc = sr_model::load_file(path, &Default::default()).unwrap();
            let opts = sr_deliver::Options {
                representation: cli.then(|| "preview".into()),
                parallel: sr_deliver::Parallel::Count(workers),
                hardware: sr_media::encode::Hardware::Software,
                ..Default::default()
            };
            let report = sr_deliver::deliver(&doc, &doc.scene.outputs[0], Some(&gpu), &opts, &mut |_, _| {}).unwrap();
            assert_eq!(report.segments, workers);
            let decoded = std::process::Command::new(sr_media::ffmpeg())
                .args(["-v", "error", "-i"])
                .arg(report.path)
                .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
                .output()
                .unwrap();
            assert!(decoded.status.success());
            assert_eq!(decoded.stdout.len(), 2 * 64 * 36 * 3);
            for frame in decoded.stdout.as_chunks::<{ 64 * 36 * 3 }>().0 {
                let want = if cli { [0, 255, 0] } else { [0, 0, 255] };
                for x in [2, 34] {
                    assert!(
                        frame[(2 * 64 + x) * 3..(2 * 64 + x) * 3 + 3]
                            .iter()
                            .zip(want)
                            .all(|(a, b)| (*a as i32 - b).abs() <= 4),
                        "representation at {x}, workers={workers}, cli={cli}"
                    );
                }
            }
        }
    }
}

#[test]
fn the_proxy_attribute_is_the_implicit_proxy_representation() {
    let Some(gpu) = gpu() else { return };
    // (case, extra representations, colour that must be drawn): the asset's @proxy answers to the name `proxy`,
    // and an explicit representation of that name wins over it
    for workers in [1, 2] {
        for (case, reps, want) in [
            ("implicit", "", [0, 0, 255]),
            ("explicit", r#"<representation name="proxy" src="explicit.png" width="4" height="4"/>"#, [0, 255, 0]),
        ] {
            let dir = directory(&format!("proxy-attr-{case}-{workers}"));
            for (name, color) in
                [("original", [255, 0, 0, 255]), ("proxy", [0, 0, 255, 255]), ("explicit", [0, 255, 0, 255])]
            {
                image::RgbaImage::from_pixel(4, 4, image::Rgba(color)).save(dir.join(format!("{name}.png"))).unwrap();
            }
            let xml = format!(
                r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="0.2" background="#000000"/>
              <output path="clip.mkv" codec="ffv1" representation="proxy" audio="false"/>
              <assets><image id="im" src="original.png" width="4" height="4" proxy="proxy.png">{reps}</image></assets>
              <composition><layer id="main" asset="im" x="32"/></composition></scene>"##
            );
            let path = dir.join("scene.xml");
            std::fs::write(&path, xml).unwrap();
            let doc = sr_model::load_file(path, &Default::default()).unwrap();
            let opts = sr_deliver::Options {
                parallel: sr_deliver::Parallel::Count(workers),
                hardware: sr_media::encode::Hardware::Software,
                ..Default::default()
            };
            let report = sr_deliver::deliver(&doc, &doc.scene.outputs[0], Some(&gpu), &opts, &mut |_, _| {}).unwrap();
            let decoded = std::process::Command::new(sr_media::ffmpeg())
                .args(["-v", "error", "-i"])
                .arg(report.path)
                .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
                .output()
                .unwrap();
            assert!(decoded.status.success());
            for frame in decoded.stdout.as_chunks::<{ 64 * 36 * 3 }>().0 {
                assert!(
                    frame[(2 * 64 + 34) * 3..(2 * 64 + 34) * 3 + 3]
                        .iter()
                        .zip(want)
                        .all(|(a, b)| (*a as i32 - b).abs() <= 4),
                    "{case}: the representation drawn"
                );
            }
        }
    }
}

#[test]
fn anamorphic_video_pixels_fill_the_display_box() {
    let Some(gpu) = gpu() else { return };
    let dir = directory("anamorphic");
    let status = std::process::Command::new(sr_media::ffmpeg())
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "color=c=red:s=32x32:r=10:d=0.2", "-c:v", "ffv1"])
        .arg(dir.join("red.mkv"))
        .status()
        .unwrap();
    assert!(status.success());
    let xml = r##"<scene version="1.2"><project width="64" height="32" fps="10" duration="0.1" background="#000000"/>
<output path="frame_%03d.png" codec="png-sequence" audio="false"/>
<assets><video id="v" src="red.mkv" width="32" height="32" fps="10" duration="0.2" pixelAspect="2"/></assets>
<composition><layer id="l" asset="v" fit="contain" boxWidth="64" boxHeight="32"/></composition></scene>"##;
    let path = dir.join("scene.xml");
    std::fs::write(&path, xml).unwrap();
    let doc = sr_model::load_file(path, &Default::default()).unwrap();
    sr_deliver::deliver(&doc, &doc.scene.outputs[0], Some(&gpu), &Default::default(), &mut |_, _| {}).unwrap();
    let frame = image::open(dir.join("frame_000.png")).unwrap().to_rgb8();
    for x in [2, 32, 61] {
        let pixel = frame.get_pixel(x, 16).0;
        assert!(pixel[0] > 200 && pixel[1] < 20 && pixel[2] < 20, "x={x}: {pixel:?}");
    }
}

#[test]
fn mesh_proxy_uses_the_same_representation_preference() {
    let Some(gpu) = gpu() else { return };
    let dir = directory("mesh-proxy");
    std::fs::write(dir.join("proxy.obj"), "v -0.3 -0.3 0\nv 0.3 -0.3 0\nv 0 0.3 0\nf 1 2 3\n").unwrap();
    let xml = r##"<scene version="1.3"><project width="64" height="64" fps="10" duration="1"/>
<assets><mesh id="m" src="missing.glb" format="glb" proxy="proxy.obj"/></assets>
<materials><material id="white" baseColor="#FFFFFF" unlit="true" doubleSided="true"/></materials>
<composition><camera id="cam" x="32" y="32" z="-100" projection="orthographic" orthoHeight="64"/>
<object3D id="obj" primitive="mesh" mesh="m" material="white" x="32" y="32"/></composition></scene>"##;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions { verify_assets: false, base_dir: Some(dir) }).unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let mut renderer = sr_gpu::Renderer::new(gpu.clone(), ev.program());
    renderer.representation = Some("proxy".into());
    let out = renderer.render(&ev.evaluate(0.0), ev.program());
    assert!(out.stats.errors.is_empty(), "{:?}", out.stats.errors);
    assert!(renderer.read(&out.texture).iter().any(|p| p[0] > 0.5));
    let mut renderer = sr_gpu::Renderer::new(gpu, ev.program());
    let out = renderer.render(&ev.evaluate(0.0), ev.program());
    assert!(!out.stats.errors.is_empty(), "the original is missing");
}
