//! Outputs whose encoder reads planar YUV get those planes from the GPU, and the pictures come out right.

mod common;
use std::path::{Path, PathBuf};

fn directory(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sr-planar-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The first frame of `path` as 8-bit RGBA, straight alpha.
fn first_frame(path: &Path, w: u32, h: u32) -> Vec<u8> {
    let out = std::process::Command::new(sr_media::ffmpeg())
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(out.stdout.len(), (w * h * 4) as usize);
    out.stdout
}

#[test]
fn planar_outputs_carry_the_picture_and_its_alpha() {
    let Some(gpu) = common::delivery("planar outputs") else { return };
    let (w, h) = (256u32, 144u32);
    // a red left half and a blue right half, on a transparent background where the output keeps alpha
    let cases: [(&str, &str, bool); 6] = [
        ("prores", r#"path="p.mov" codec="prores""#, false),
        ("prores4444", r#"path="p4.mov" codec="prores" proresProfile="4444""#, false),
        ("prores-alpha", r#"path="pa.mov" codec="prores" proresProfile="4444" alpha="true""#, true),
        ("dnxhr", r#"path="d.mxf" codec="dnxhr""#, false),
        ("ffv1-alpha", r#"path="f.mkv" codec="ffv1" alpha="true""#, true),
        ("h265", r#"path="h.mp4" codec="h265" preset="ultrafast""#, false),
    ];
    for (name, attrs, alpha) in cases {
        let encoder = match name {
            n if n.starts_with("prores") => "prores_ks",
            "dnxhr" => "dnxhd",
            "h265" => "libx265",
            _ => "ffv1",
        };
        if !common::encoder(name, encoder) {
            continue;
        }
        let dir = directory(name);
        let background = if alpha { "#00000000" } else { "#000000" };
        let xml = format!(
            r##"<scene version="1.2"><project width="{w}" height="{h}" fps="10" duration="0.2" background="{background}"/>
              <output {attrs} audio="false"/>
              <composition><shape id="left" shape="rect" width="{half}" height="{third}" fill="#FF0000"/>
                <shape id="right" shape="rect" width="{half}" height="{third}" x="{half}" fill="#0000FF"/></composition></scene>"##,
            half = w / 2,
            third = h / 3
        );
        let path = dir.join("scene.xml");
        std::fs::write(&path, xml).unwrap();
        let doc = sr_model::load_file(path, &Default::default()).unwrap();
        let opts = sr_deliver::Options { hardware: sr_media::encode::Hardware::Software, ..Default::default() };
        let out = &doc.scene.outputs[0];
        sr_deliver::deliver(&doc, out, Some(&gpu), &opts, &mut |_, _| {}).unwrap_or_else(|e| panic!("{name}: {e}"));
        let px = first_frame(&dir.join(&out.path), w, h);
        let at = |x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            [px[i], px[i + 1], px[i + 2], px[i + 3]]
        };
        let near = |got: [u8; 4], want: [u8; 4]| got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= 6);
        // the rectangles cover the top third; the sampled texels sit away from the edges and the chroma blocks
        let red = at(w / 4, h / 6);
        let blue = at(3 * w / 4, h / 6);
        assert!(near(red, [255, 0, 0, 255]), "{name}: red rectangle {red:?}");
        assert!(near(blue, [0, 0, 255, 255]), "{name}: blue rectangle {blue:?}");
        let below = at(w / 2, 2 * h / 3);
        if alpha {
            assert_eq!(below[3], 0, "{name}: transparent below the rectangles, got {below:?}");
        } else {
            assert!(near(below, [0, 0, 0, 255]), "{name}: black below the rectangles, got {below:?}");
        }
    }
}
