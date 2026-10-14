use std::path::PathBuf;

use sr_model::{load_str, validate_str, LoadOptions, Severity};

fn options() -> LoadOptions {
    LoadOptions {
        verify_assets: true,
        base_dir: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/media")),
    }
}

fn scene(assets: &str) -> String {
    format!(
        r#"<scene version="1.1"><project width="64" height="32" fps="30" duration="1"/>
<assets>{assets}</assets><composition/></scene>"#
    )
}

#[test]
fn image_dimensions_follow_exif_orientation() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sr-media/tests/fixtures/still/rotated.jpg");
    for (width, height, warns) in [(8, 12, false), (12, 8, true)] {
        let xml = scene(&format!(r#"<image id="photo" src="{}" width="{width}" height="{height}"/>"#, path.display()));
        let report = validate_str(&xml, &options());
        assert!(!report.has_errors(), "{report}");
        assert_eq!(report.diagnostics.iter().any(|d| d.code == "A07"), warns, "{width}x{height}: {report}");
    }
}

#[test]
fn image_dimension_mismatch_is_a_located_warning_without_a_digest() {
    let xml = scene(r#"<image id="im" src="logo.png" width="1536" height="1024"/>"#);
    let doc = load_str(&xml, &options()).expect("a mismatch does not prevent rendering");
    let warning = doc.warnings().iter().find(|d| d.code == "A07").expect("dimension warning");
    assert_eq!(warning.severity, Severity::Warning);
    assert!(warning.message.contains("1536x1024") && warning.message.contains("16x16"), "{warning:?}");
    assert!(warning.message.contains("logo.png"));
    assert!(warning.loc.line > 0 && warning.loc.column > 0);
    assert!(warning.path.contains("image"));
    assert!(warning.help.is_some());
    let skipped = validate_str(&xml, &LoadOptions::without_assets());
    assert!(!skipped.diagnostics.iter().any(|d| d.code == "A07"));
}

/// A directory with `logo.png` (16x16) and a `proxy.png` of `w`x`h`.
fn proxy_dir(name: &str, w: u32, h: u32) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sr-model-proxy-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(options().base_dir.unwrap().join("logo.png"), dir.join("logo.png")).unwrap();
    image::RgbaImage::new(w, h).save(dir.join("proxy.png")).unwrap();
    dir
}

#[test]
fn matching_images_and_smaller_proxy_representations_do_not_warn() {
    let dir = proxy_dir("match", 8, 8);
    let xml = scene(
        r#"<image id="im" src="logo.png" width="16" height="16">
<representation name="proxy" src="proxy.png" width="8" height="8"/>
</image>"#,
    );
    let report = validate_str(&xml, &LoadOptions { verify_assets: true, base_dir: Some(dir.clone()) });
    assert!(!report.has_errors(), "{report}");
    assert!(!report.diagnostics.iter().any(|d| d.code == "A07" || d.code == "A08"), "{report}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_representation_that_declares_other_dimensions_than_its_file_warns() {
    let dir = proxy_dir("declared", 8, 8);
    let xml = scene(
        r#"<image id="im" src="logo.png" width="16" height="16">
<representation name="proxy" src="proxy.png" width="4" height="4"/>
</image>"#,
    );
    let doc = load_str(&xml, &LoadOptions { verify_assets: true, base_dir: Some(dir.clone()) })
        .expect("a mismatch does not prevent rendering");
    let warning = doc.warnings().iter().find(|d| d.code == "A08").expect("representation dimension warning");
    assert_eq!(warning.severity, Severity::Warning);
    assert!(warning.message.contains("4x4") && warning.message.contains("8x8"), "{warning:?}");
    assert!(warning.message.contains("proxy"), "{warning:?}");
    assert!(warning.loc.line > 0 && warning.help.is_some());
    // the asset's own dimensions are right: no A07
    assert!(!doc.warnings().iter().any(|d| d.code == "A07"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_representation_must_keep_the_aspect_ratio_of_its_asset() {
    // the file is 8x4 and declared so, but the asset is square: laid out at the asset's size it would distort
    let dir = proxy_dir("aspect", 8, 4);
    let xml = scene(
        r#"<image id="im" src="logo.png" width="16" height="16">
<representation name="proxy" src="proxy.png" width="8" height="4"/>
</image>"#,
    );
    let doc = load_str(&xml, &LoadOptions { verify_assets: true, base_dir: Some(dir.clone()) }).unwrap();
    let warning = doc.warnings().iter().find(|d| d.code == "A08").expect("aspect warning");
    assert!(warning.message.contains("aspect") && warning.message.contains("8x4"), "{warning:?}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_representation_rounded_to_whole_pixels_keeps_the_aspect_ratio() {
    // 16x16 scaled to 7x7 is square; 16x10 scaled to 7x4 is 1.75 against 1.6 but within one pixel of 7x4.375
    let dir = proxy_dir("rounded", 7, 4);
    let xml = scene(
        r#"<image id="im" src="logo.png" width="16" height="10">
<representation name="proxy" src="proxy.png" width="7" height="4"/>
</image>"#,
    );
    let report = validate_str(&xml, &LoadOptions { verify_assets: true, base_dir: Some(dir.clone()) });
    assert!(!report.diagnostics.iter().any(|d| d.code == "A08"), "{report}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn missing_images_keep_the_existing_missing_file_diagnostic() {
    let report = validate_str(&scene(r#"<image id="im" src="absent.png" width="16" height="16"/>"#), &options());
    assert!(report.diagnostics.iter().any(|d| d.code == "A01"));
    assert!(!report.diagnostics.iter().any(|d| d.code == "A07"));
}

#[test]
fn image_headers_are_identified_by_content_and_each_declaration_is_checked() {
    let dir = std::env::temp_dir().join(format!("sr-model-image-headers-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(options().base_dir.unwrap().join("logo.png"), dir.join("image.data")).unwrap();
    let xml = scene(
        r#"<image id="correct" src="image.data" width="16" height="16"/>
<image id="wide" src="image.data" width="32" height="16"/>
<image id="tall" src="image.data" width="16" height="32"/>"#,
    );
    let report = validate_str(&xml, &LoadOptions { verify_assets: true, base_dir: Some(dir.clone()) });
    assert!(!report.has_errors(), "{report}");
    assert_eq!(report.diagnostics.iter().filter(|d| d.code == "A07").count(), 2, "{report}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn each_optional_representation_dimension_is_checked_independently() {
    let dir = proxy_dir("one-dimension", 8, 8);
    for attrs in [r#"width="4""#, r#"height="4""#] {
        let xml = scene(&format!(
            r#"<image id="im" src="logo.png" width="16" height="16">
<representation name="proxy" src="proxy.png" {attrs}/></image>"#
        ));
        let report = validate_str(&xml, &LoadOptions { verify_assets: true, base_dir: Some(dir.clone()) });
        assert!(report.diagnostics.iter().any(|d| d.code == "A08"), "{report}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn video_representation_dimensions_are_checked_against_the_file() {
    if std::process::Command::new(sr_media::ffmpeg()).arg("-version").output().is_err() {
        return;
    }
    let dir = proxy_dir("video", 8, 8);
    assert!(std::process::Command::new(sr_media::ffmpeg())
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "color=s=16x8:r=10:d=0.1", "-c:v", "ffv1"])
        .arg(dir.join("v.mkv"))
        .status()
        .unwrap()
        .success());
    let xml = scene(
        r#"<video id="v" src="v.mkv" width="16" height="8" fps="10" duration="1">
<representation name="small" src="v.mkv" width="8" height="4"/></video>"#,
    );
    let report = validate_str(&xml, &LoadOptions { verify_assets: true, base_dir: Some(dir.clone()) });
    assert!(report.diagnostics.iter().any(|d| d.code == "A08"), "{report}");
    std::fs::remove_dir_all(dir).unwrap();
}
