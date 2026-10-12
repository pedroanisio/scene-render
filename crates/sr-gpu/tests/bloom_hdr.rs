//! Bloom's bright pass and supersampling keep scene-linear values honest:
//! - a dark pixel contributes nothing to `bloom` at any threshold (the soft knee never reaches below luminance 0);
//! - with `project/@supersample`, colour finishing (exposure, looks, tone mapping) runs on the filtered frame;
//!   keeping values above 1 through the filter needs an SREP 72 amendment (its clamp to [0, 1] is specified).

mod common;
use common::*;

fn load(xml: &str) -> sr_model::Document {
    sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

/// A 200 × 100 frame on `ground`, with a white dot at its centre, all under a bloom of `threshold`.
fn bloom_scene(ground: &str, threshold: f64, dot: bool) -> String {
    let dot = if dot {
        r##"<shape id="dot" shape="ellipse" x="90" y="40" width="20" height="20" fill="#FFFFFF"/>"##
    } else {
        ""
    };
    format!(
        r##"<scene version="1.6"><project width="200" height="100" fps="24" duration="1" background="{ground}"/>
  <composition><group id="g" width="200" height="100" effects="b">
    <shape id="bg" shape="rect" width="200" height="100" fill="{ground}"/>{dot}
  </group></composition>
  <effects><effect id="b" type="bloom" threshold="{threshold}" radius="8" intensity="0.5"/></effects></scene>"##
    )
}

#[test]
fn bloom_keeps_an_all_black_frame_black_at_every_threshold() {
    for threshold in [0.0, 0.05, 0.1, 0.2, 0.3, 0.45, 0.5, 0.7] {
        let Some(r) = render(&load(&bloom_scene("#000000", threshold, false))) else { return };
        for (k, p) in r.px.iter().enumerate() {
            assert!(p[0] == 0.0 && p[1] == 0.0 && p[2] == 0.0, "threshold {threshold}: pixel {k} is {p:?}");
        }
    }
}

#[test]
fn bloom_does_not_lift_dark_ground_at_low_thresholds() {
    // atlas s2 _repro/bloom-knee.scene.xml: #050505 ground rendered #1C1C1C under threshold 0.3
    let ground = lin8(5);
    for threshold in [0.05, 0.1, 0.2, 0.3, 0.45] {
        let Some(r) = render(&load(&bloom_scene("#050505", threshold, true))) else { return };
        // a corner, far beyond the dot's glow (radius 8 around a dot 90 px away)
        for (x, y) in [(2, 2), (197, 2), (2, 97), (197, 97)] {
            let p = r.at(x, y);
            for c in &p[..3] {
                // the knee may pass a sliver of a pixel this dark, never a visible lift: under one 8-bit code value
                assert!((c - ground).abs() < 2e-4, "threshold {threshold}: ({x}, {y}) is {p:?}, ground {ground}");
            }
        }
        // the highlight still blooms
        let near = r.at(100, 30);
        assert!(near[0] > ground + 0.01, "threshold {threshold}: no glow above the dot: {near:?}");
    }
}

/// A 64 × 32 frame filled with white pushed to 4.0 by an exposure of 2 stops, tone mapped by `tm` at the end.
fn hdr_scene(project: &str, tm: &str) -> String {
    format!(
        r##"<scene version="1.6"><project width="64" height="32" fps="24" duration="1" background="#000000" {project}/>
  <colorManagement toneMapping="{tm}"/>
  <composition><shape id="s" shape="rect" width="64" height="32" fill="#FFFFFF" effects="e"/></composition>
  <effects><effect id="e" type="exposure" exposure="2"/></effects></scene>"##
    )
}

/// The same frame with SDR content only: values stay in [0, 1], so SREP 72's clamp does not bite.
fn sdr_scene(project: &str, tm: &str) -> String {
    format!(
        r##"<scene version="1.6"><project width="64" height="32" fps="24" duration="1" background="#000000" {project}/>
  <colorManagement toneMapping="{tm}"/>
  <composition><shape id="s" shape="rect" width="64" height="32" fill="#B4B4B4"/></composition></scene>"##
    )
}

#[test]
fn supersampled_frames_are_colour_finished() {
    // colour finishing runs on the filtered frame, so a supersampled frame is tone mapped like any other: with SDR
    // content the result at n = 2, 3 equals n = 1
    for tm in ["reinhard", "aces"] {
        let Some(one) = render(&load(&sdr_scene("", tm))) else { return };
        let plain = render(&load(&sdr_scene("", "none"))).unwrap();
        assert!(!close(one.at(32, 16), plain.at(32, 16), 1e-3), "{tm} must change the frame at n = 1");
        for filter in ["box", "lanczos3"] {
            for n in [2, 3] {
                let project = format!(r#"supersample="{n}" supersampleFilter="{filter}""#);
                let r = render(&load(&sdr_scene(&project, tm))).unwrap();
                assert!(
                    close(r.at(32, 16), one.at(32, 16), 2e-3),
                    "{tm}, {filter} n = {n}: {:?}, at n = 1 {:?}",
                    r.at(32, 16),
                    one.at(32, 16)
                );
            }
        }
    }
}

#[test]
fn supersampled_hdr_reaches_the_tone_mapper() {
    for tm in ["reinhard", "aces"] {
        let Some(one) = render(&load(&hdr_scene("", tm))) else { return };
        let reference = one.at(32, 16);
        // the tone mapper saw 4.0, not 1.0: Reinhard gives 0.8 for 4.0 and 0.5 for 1.0, the ACES fit about 0.973 and 0.804
        let (at_four, at_one) = if tm == "reinhard" { (0.8, 0.5) } else { (0.9734, 0.8038) };
        assert!((reference[0] - at_four).abs() < 0.01 && reference[0] > at_one + 0.1, "{tm} at n = 1: {reference:?}");
        for filter in ["box", "lanczos3"] {
            for n in [2, 3] {
                let project = format!(r#"supersample="{n}" supersampleFilter="{filter}""#);
                let r = render(&load(&hdr_scene(&project, tm))).unwrap();
                assert_eq!(r.size, one.size, "the frame keeps its size");
                let p = r.at(32, 16);
                assert!(close(p, reference, 2e-3), "{tm}, {filter} n = {n}: {p:?}, at n = 1 {reference:?}");
            }
        }
    }
}

#[test]
fn supersampled_hdr_survives_without_tone_mapping() {
    // no tone mapper: the frame holds the scene-linear 4.0 as n = 1 does
    let Some(one) = render(&load(&hdr_scene("", "none"))) else { return };
    assert!(one.at(32, 16)[0] > 3.9, "{:?}", one.at(32, 16));
    let r = render(&load(&hdr_scene(r#"supersample="2""#, "none"))).unwrap();
    assert!(close(r.at(32, 16), one.at(32, 16), 2e-3), "{:?} vs {:?}", r.at(32, 16), one.at(32, 16));
}
