//! The time-frames draft's file checks (assets.rs): TF1 (an ephemeris asset's file is an SPK file of type 2 segments
//! in the ICRF whose sha256 equals its digest) and the clauses of TF4 and TF5 that need the file's bodies.

use std::path::PathBuf;

const EXCERPT_DIGEST: &str = "d113bb5117946244cf63de9d06287d5aac008acd1a561d28b760bcc7d4c3d1fd";

fn media() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/corpus/media"))
}

fn codes(ephemeris: &str, composition: &str, space: &str) -> Vec<String> {
    let xml = format!(
        r#"<scene version="1.8"><project width="64" height="64" fps="24" duration="1"/><assets>{ephemeris}</assets><composition>{composition}</composition>{space}</scene>"#
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(media()) };
    sr_model::validate_str(&xml, &opts).diagnostics.into_iter().map(|d| d.code).collect()
}

fn asset(src: &str, digest: &str) -> String {
    format!(r#"<ephemeris id="de" src="{src}" digest="sha256:{digest}"/>"#)
}

fn excerpt() -> String {
    asset("de440s-excerpt.bsp", EXCERPT_DIGEST)
}

fn space(rest: &str) -> String {
    format!(r#"<space epoch="2451545.0" timeScale="tdb" ephemeris="de"{rest}/>"#)
}

fn body(b: &str) -> String {
    format!(r#"<object3D id="b" primitive="sphere" radius="4" body="{b}" positionFrom="ephemeris"/>"#)
}

#[test]
fn the_excerpt_with_its_digest_and_bodies_in_it_is_valid() {
    for (b, origin) in [("301", "399"), ("399", "0"), ("10", "3"), ("5", "10")] {
        let got = codes(&excerpt(), &body(b), &space(&format!(r#" origin="{origin}""#)));
        assert!(got.is_empty(), "{b} about {origin}: {got:?}");
    }
    let got = codes(&excerpt(), "", &space(r#" frame="bodyFixed" frameBody="399""#));
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn a_file_of_another_digest_or_kind_is_refused_tf1() {
    let mut wrong = EXCERPT_DIGEST.to_string();
    wrong.replace_range(0..1, "0");
    let got = codes(&asset("de440s-excerpt.bsp", &wrong), "", &space(""));
    assert!(got.iter().any(|c| c == "TF1"), "{got:?}");
    // a WAV file pinned by its own digest is not an SPK file
    let wav = std::fs::read(media().join("music.wav")).unwrap();
    let digest: String = {
        use sha2::Digest;
        sha2::Sha256::digest(&wav).iter().map(|b| format!("{b:02x}")).collect()
    };
    let got = codes(&asset("music.wav", &digest), "", &space(""));
    assert!(got.iter().any(|c| c == "TF1"), "{got:?}");
}

#[test]
fn a_missing_file_is_a01() {
    let got = codes(&asset("absent.bsp", EXCERPT_DIGEST), "", &space(""));
    assert!(got.iter().any(|c| c == "A01"), "{got:?}");
    assert!(!got.iter().any(|c| c == "TF1"), "{got:?}");
}

#[test]
fn a_placed_body_is_in_the_file_tf4() {
    let got = codes(&excerpt(), &body("499"), &space(""));
    assert!(got.iter().any(|c| c == "TF4"), "{got:?}");
    // orientation alone reads no position from the file
    let got = codes(
        &excerpt(),
        r#"<object3D id="b" primitive="sphere" radius="4" body="499" orientation="bodyFixed"/>"#,
        &space(""),
    );
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn the_origin_and_frame_body_are_in_the_file_or_the_barycentre_tf5() {
    let got = codes(&excerpt(), "", &space(r#" origin="499""#));
    assert!(got.iter().any(|c| c == "TF5"), "{got:?}");
    let got = codes(&excerpt(), "", &space(r#" frame="bodyFixed" frameBody="499""#));
    assert!(got.iter().any(|c| c == "TF5"), "{got:?}");
    let got = codes(&excerpt(), "", &space(r#" origin="+0399""#));
    assert!(got.is_empty(), "{got:?}");
}
