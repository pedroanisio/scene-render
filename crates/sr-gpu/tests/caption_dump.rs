//! The caption dump (SREP 52 conformance): each track's pages after paging, the words of each cue with their times,
//! and the word the presets treat as current, as the burn-in layout computes them. No GPU is needed.

use sr_gpu::caption_dump::{dump, CaptionDump};

fn doc(captions: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.2"><project width="640" height="360" fps="24" duration="8" background="#000000FF"/><composition/><captions>{captions}</captions></scene>"##
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}"))
}

fn dumped(captions: &str, at: &[f64]) -> CaptionDump {
    let d = doc(captions);
    let ev = sr_eval::Evaluator::new(&d, &sr_eval::EvalOptions::default()).unwrap_or_else(|r| panic!("{r:?}"));
    dump(ev.program(), at)
}

fn texts(d: &CaptionDump, track: usize) -> Vec<String> {
    d.tracks[track].pages.iter().map(|p| p.text.clone()).collect()
}

const THREE_LINES: &str = "aa bb&#10;cc dd ee ff&#10;gg hh";

fn track(attrs: &str, text: &str) -> String {
    format!(
        r#"<captionTrack id="cap" language="en" preset="classic" {attrs}><cue start="0" end="8" text="{text}"/></captionTrack>"#
    )
}

#[test]
fn pages_follow_the_line_breaks_mode_and_the_limits() {
    let pages = |attrs: &str| texts(&dumped(&track(attrs, THREE_LINES), &[]), 0);
    // SREP 52 conformance: source, default, greedy, limits
    assert_eq!(pages(r#"lineBreaks="source" maxCharsPerLine="80" maxLines="2""#), ["aa bb\ncc dd ee ff", "gg hh"]);
    assert_eq!(pages(r#"maxCharsPerLine="80" maxLines="2""#), ["aa bb cc dd ee ff gg hh"]);
    assert_eq!(pages(r#"lineBreaks="greedy" maxCharsPerLine="80" maxLines="2""#), ["aa bb cc dd ee ff gg hh"]);
    assert_eq!(pages(r#"lineBreaks="source" maxCharsPerLine="5" maxLines="2""#), ["aa bb\ncc dd", "ee ff\ngg hh"]);
    assert_eq!(
        pages(r#"lineBreaks="source" maxWordsPerLine="2" maxCharsPerLine="80" maxLines="2""#),
        ["aa bb\ncc dd", "ee ff\ngg hh"]
    );
    assert_eq!(pages(r#"lineBreaks="source" maxCharsPerLine="80" maxLines="1""#), ["aa bb", "cc dd ee ff", "gg hh"]);
}

#[test]
fn a_page_lists_its_lines_cue_times_and_first_word() {
    // eight words of two letters over 8 s: each word lasts exactly 1 s
    let d = dumped(
        &track(r#"lineBreaks="source" maxWordsPerLine="2" maxCharsPerLine="80" maxLines="2""#, THREE_LINES),
        &[],
    );
    let t = &d.tracks[0];
    assert_eq!((t.id.as_str(), t.preset, t.line_breaks, t.burn), ("cap", "classic", "source", true));
    assert_eq!((t.max_chars_per_line, t.max_words_per_line, t.max_lines), (80, Some(2), 2));
    assert_eq!(t.error, None);
    assert_eq!(t.word_count, 8);
    assert_eq!(t.pages.len(), 2);
    let (p0, p1) = (&t.pages[0], &t.pages[1]);
    assert_eq!(p0.lines, ["aa bb", "cc dd"]);
    assert_eq!(p1.lines, ["ee ff", "gg hh"]);
    assert_eq!((p0.cue, p0.start, p0.end, p0.first_word), (0, 0.0, 4.0, 0));
    assert_eq!((p1.cue, p1.start, p1.end, p1.first_word), (0, 4.0, 8.0, 4));
    // the cue lists every word once, in order, with its time
    assert_eq!(t.cues.len(), 1);
    let words: Vec<_> = t.cues[0].words.iter().map(|w| (w.index, w.text.as_str(), w.start, w.end)).collect();
    assert_eq!(words[0], (0, "aa", 0.0, 1.0));
    assert_eq!(words[5], (5, "ff", 5.0, 6.0));
    assert_eq!(words.len(), 8);
    // the current word of the second page is addressed by its index in the cue
    assert_eq!(
        p1.active.iter().map(|s| (s.start, s.end, s.word)).collect::<Vec<_>>(),
        [(4.0, 5.0, Some(4)), (5.0, 6.0, Some(5)), (6.0, 7.0, Some(6)), (7.0, 8.0, Some(7))]
    );
}

#[test]
fn presets_address_the_same_word_indices_in_both_modes() {
    for preset in ["karaoke", "highlight", "boxed-word"] {
        let doc = format!(
            r#"<captionTrack id="src" language="en" preset="{preset}" lineBreaks="source" maxCharsPerLine="80" maxLines="2"><cue start="0" end="4" text="aa bb&#10;cc dd"/></captionTrack><captionTrack id="grd" language="en" preset="{preset}" lineBreaks="greedy" maxCharsPerLine="80" maxLines="2"><cue start="0" end="4" text="aa bb&#10;cc dd"/></captionTrack>"#
        );
        let d = dumped(&doc, &[0.5, 1.5, 2.5, 3.5, 4.0, 9.0]);
        assert_eq!(texts(&d, 0), ["aa bb\ncc dd"]);
        assert_eq!(texts(&d, 1), ["aa bb cc dd"]);
        for t in &d.tracks {
            assert_eq!(t.preset, preset);
            let at: Vec<_> = t.at.iter().map(|s| (s.time, s.page, s.cue, s.word)).collect();
            assert_eq!(
                at,
                [
                    (0.5, Some(0), Some(0), Some(0)),
                    (1.5, Some(0), Some(0), Some(1)),
                    (2.5, Some(0), Some(0), Some(2)),
                    (3.5, Some(0), Some(0), Some(3)),
                    // a page ends at its end time: nothing is on screen after the cue
                    (4.0, None, None, None),
                    (9.0, None, None, None)
                ]
            );
        }
    }
}

#[test]
fn blank_lines_and_crlf_change_neither_words_nor_times() {
    let text = "&#10; aa bb&#13;&#10;&#10; cc dd&#10;";
    let src = dumped(&track(r#"lineBreaks="source" maxCharsPerLine="80" maxLines="2""#, text), &[]);
    let grd = dumped(&track(r#"lineBreaks="greedy" maxCharsPerLine="80" maxLines="2""#, text), &[]);
    assert_eq!(texts(&src, 0), ["aa bb\ncc dd"]);
    assert_eq!(texts(&grd, 0), ["aa bb cc dd"]);
    assert_eq!((src.tracks[0].word_count, grd.tracks[0].word_count), (4, 4));
    assert_eq!(src.tracks[0].cues, grd.tracks[0].cues);
}

#[test]
fn one_word_pages_each_word_and_sidecar_tracks_are_listed_unburned() {
    let d = dumped(
        r#"<captionTrack id="ow" language="en" preset="one-word" lineBreaks="source"><cue start="0" end="4" text="aa bb&#10;cc dd"/></captionTrack><captionTrack id="sc" language="en" mode="sidecar"><cue start="0" end="2" text="hello"/></captionTrack>"#,
        &[2.5],
    );
    assert_eq!(texts(&d, 0), ["aa", "bb", "cc", "dd"]);
    assert_eq!(d.tracks[0].pages.iter().map(|p| p.first_word).collect::<Vec<_>>(), [0, 1, 2, 3]);
    assert_eq!((d.tracks[0].at[0].page, d.tracks[0].at[0].word), (Some(2), Some(2)));
    assert!(!d.tracks[1].burn);
    assert_eq!(texts(&d, 1), ["hello"]);
}

#[test]
fn a_track_that_cannot_be_read_is_listed_with_its_error() {
    let d = dumped(
        r#"<captionTrack id="gone" language="en" src="no-such-file.srt"/><captionTrack id="ok" language="en"><cue start="0" end="1" text="hi"/></captionTrack>"#,
        &[0.5],
    );
    let gone = &d.tracks[0];
    assert!(gone.error.as_deref().is_some_and(|e| e.contains("no-such-file.srt")), "{:?}", gone.error);
    assert!(gone.pages.is_empty() && gone.cues.is_empty());
    assert_eq!(gone.at[0].page, None);
    assert_eq!(texts(&d, 1), ["hi"]);
    assert!(!d.is_complete());
    assert!(dumped(r#"<captionTrack id="ok" language="en"><cue start="0" end="1" text="hi"/></captionTrack>"#, &[])
        .is_complete());
}

#[test]
fn the_json_form_is_stable_and_camel_cased() {
    let caps = track(r#"lineBreaks="source" maxCharsPerLine="80" maxLines="2""#, THREE_LINES);
    let a = serde_json::to_string(&dumped(&caps, &[0.5])).unwrap();
    let b = serde_json::to_string(&dumped(&caps, &[0.5])).unwrap();
    assert_eq!(a, b, "the dump is deterministic");
    let v: serde_json::Value = serde_json::from_str(&a).unwrap();
    let t = &v["tracks"][0];
    assert_eq!(t["lineBreaks"], "source");
    assert_eq!(t["maxCharsPerLine"], 80);
    assert_eq!(t["maxWordsPerLine"], serde_json::Value::Null);
    assert_eq!(t["wordCount"], 8);
    assert_eq!(t["pages"][0]["text"], "aa bb\ncc dd ee ff");
    assert_eq!(t["pages"][0]["lines"], serde_json::json!(["aa bb", "cc dd ee ff"]));
    assert_eq!(t["pages"][1]["firstWord"], 6);
    assert_eq!(t["at"][0], serde_json::json!({"time": 0.5, "page": 0, "cue": 0, "word": 0}));
    assert!(t.get("error").is_none(), "no error key when the track loaded");
}

#[test]
fn a_document_without_captions_dumps_no_tracks() {
    let d = sr_model::load_str(
        r#"<scene version="1.2"><project width="64" height="64" fps="24" duration="1"/><composition/></scene>"#,
        &sr_model::LoadOptions::without_assets(),
    )
    .unwrap();
    let ev = sr_eval::Evaluator::new(&d, &sr_eval::EvalOptions::default()).unwrap();
    let out = dump(ev.program(), &[0.0]);
    assert!(out.tracks.is_empty() && out.is_complete());
}

mod common;

fn a4_pixels(caps: &str, body: &str) -> common::Rendered {
    let xml = format!(
        r##"<scene version="1.2"><project width="320" height="180" fps="24" duration="4" background="#000000"/><composition>{body}</composition><captions>{caps}</captions></scene>"##
    );
    let d = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    common::render_times(&d, &[0.1]).expect("A4 pixel evidence requires a software Vulkan adapter")
}

#[test]
fn a4_captions_stack_by_z_and_equal_z_document_order() {
    let caps = r#"<captionTrack id="cc" language="en" preset="none" y="10%"><cue start="0" end="2" text="CAPTION"/></captionTrack>"#;
    let cover = |z| format!(r##"<shape id="cover" z="{z}" shape="rect" width="320" height="180" fill="#FF0000"/>"##);
    let high = a4_pixels(caps, &cover(2000000));
    assert!(high.px.iter().all(|p| p[1] < 0.001 && p[2] < 0.001), "higher layer must cover captions");
    let low = a4_pixels(caps, &cover(0));
    assert!(low.px.iter().any(|p| p[1] > 0.1), "default captions remain visible");
    let tracks = r##"<captionTrack id="first" language="en" z="20" preset="highlight" activeColor="#FF0000" y="10%"><cue start="0" end="2" text="CAPTION"/></captionTrack><captionTrack id="last" language="en" z="10" preset="highlight" activeColor="#00FF00" y="10%"><cue start="0" end="2" text="CAPTION"/></captionTrack>"##;
    let sorted = a4_pixels(tracks, "");
    assert!(sorted.px.iter().map(|p| p[0] - p[1]).sum::<f32>() > 1.0);
    let tied = a4_pixels(&tracks.replace("z=\"10\"", "z=\"20\""), "");
    assert!(tied.px.iter().map(|p| p[1] - p[0]).sum::<f32>() > 1.0);
}

#[test]
fn a4_cue_position_overrides_track_and_reports_ignored_settings() {
    let caps = |position: &str| {
        format!(
            r#"<captionTrack id="cc" language="en" preset="none" x="80%" y="70%"><cue start="0" end="2" position="{position}" text="CAPTION"/></captionTrack>"#
        )
    };
    let moved = a4_pixels(&caps("line:10% position:30% align:start"), "");
    let reference = a4_pixels(&caps("").replace("x=\"80%\" y=\"70%\"", "x=\"30%\" y=\"10%\""), "");
    assert!(moved.px == reference.px, "cue percentages must match track percentages");
    assert!(moved.stats.errors.is_empty(), "{:?}", moved.stats.errors);
    assert!(moved.stats.unsupported.iter().any(|s| s.contains("align:start")));
}

#[test]
fn a4_caption_emphasis_uses_bold_without_active_colour() {
    let caps = |emphasis| {
        format!(
            r#"<captionTrack id="cc" language="en" preset="none" y="10%"><cue start="0" end="2"><word start="0" end="1" text="MMMM" emphasis="{emphasis}"/></cue></captionTrack>"#
        )
    };
    let plain = a4_pixels(&caps(false), "");
    let bold = a4_pixels(&caps(true), "");
    assert!(bold.px.iter().map(|p| p[0]).sum::<f32>() > plain.px.iter().map(|p| p[0]).sum::<f32>() * 1.05);
}

#[test]
fn a4_glyph_depth_uses_the_layer_camera_pose_and_projection() {
    let image = |camera: &str, glyph_depth: bool| {
        let (depth, anim) = if glyph_depth { ("0", r#"<textAnimator zDepth="60"/>"#) } else { ("60", "") };
        let xml = format!(
            r##"<scene version="1.2"><project width="320" height="180" fps="24" duration="1" background="#000000"/><assets><text id="t" width="200" height="100" text="M" size="48" font="DejaVu Sans" color="#FFFFFF"/></assets><composition>{camera}<layer id="l" asset="t" threeD="true" x="50" y="30" zDepth="{depth}">{anim}</layer></composition></scene>"##
        );
        let d = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
        common::render_times(&d, &[0.0]).expect("A4 requires software Vulkan")
    };
    let centroid = |r: &common::Rendered| {
        let mut s = [0.0f64; 3];
        for (i, p) in r.px.iter().enumerate() {
            let v = p[0] as f64;
            s[0] += v * (i % 320) as f64;
            s[1] += v * (i / 320) as f64;
            s[2] += v;
        }
        assert!(s[2] > 10.0, "visible glyph");
        [s[0] / s[2], s[1] / s[2]]
    };
    for camera in [
        "",
        r#"<camera id="cam" x="130" y="80" z="-300" fov="45" roll="12"/>"#,
        r#"<camera id="cam" projection="orthographic" x="130" y="80" z="-300" yaw="8"/>"#,
    ] {
        let (a, b) = (centroid(&image(camera, true)), centroid(&image(camera, false)));
        assert!((a[0] - b[0]).abs() < 0.7 && (a[1] - b[1]).abs() < 0.7, "{camera}: {a:?} vs {b:?}");
    }
}

#[test]
fn a4_checkpoint_labels_share_name_id_index_without_key_collisions() {
    let xml = r#"<scene version="1.2"><project width="64" height="64" fps="24" duration="1"/><assets><text id="t" width="64" height="64" text="M" size="12"/></assets><symbols><symbol id="sym" name="Lower third"/></symbols><composition><layer id="l" name="Title" asset="t"><textAnimator name="Turn"/><textAnimator name="Turn"/><textAnimator/></layer><shape id="shape" shape="rect" width="10" height="10"><deform><modifier type="puppet"><pin name="Elbow" restX="0" restY="0"/><pin restX="10" restY="10"/></modifier></deform></shape></composition></scene>"#;
    let d = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = sr_eval::Evaluator::new(&d, &Default::default()).unwrap();
    let labels = sr_gpu::checkpoints::labels(ev.program());
    assert_eq!(labels.values().filter(|v| v.as_str() == "Turn").count(), 2);
    for label in ["Lower third", "Title", "textAnimator[3]", "Elbow", "pin[2]", "shape"] {
        assert!(labels.values().any(|v| v == label), "missing {label}: {labels:?}");
    }
    let dir = std::env::temp_dir().join(format!("sr-a4-checkpoint-labels-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    sr_gpu::checkpoints::Store::new(&dir, ev.program()).unwrap();
    let manifest = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.to_string_lossy().ends_with(".labels.json"))
        .unwrap();
    let saved: std::collections::BTreeMap<String, String> =
        serde_json::from_slice(&std::fs::read(manifest).unwrap()).unwrap();
    assert_eq!(saved, labels);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a4_rotation_sign_changes_outline_and_bitmap_perspective() {
    for text in ["M", "😀"] {
        let image = |angle| {
            let xml = format!(
                r##"<scene version="1.2"><project width="160" height="160" fps="24" duration="1" background="#000000"/><assets><text id="t" width="160" height="160" text="{text}" size="80" font="DejaVu Sans" color="#FFFFFF"/></assets><composition><layer id="l" asset="t"><textAnimator rotationX="{angle}" rotationY="30"/></layer></composition></scene>"##
            );
            let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
            common::render_times(&doc, &[0.0]).expect("A4 requires software Vulkan")
        };
        let positive = image(45);
        let negative = image(-45);
        assert!(positive.stats.errors.is_empty(), "{:?}", positive.stats.errors);
        assert!(
            positive.px.iter().zip(&negative.px).map(|(a, b)| (a[0] - b[0]).abs()).sum::<f32>() > 5.0,
            "{text}: signed turns need true perspective"
        );
    }
}

#[test]
fn a4_glyph_camera_and_parent_motion_invalidate_cached_picture() {
    for motion in ["camera", "group"] {
        let animation =
            r#"<animate property="rotationY"><key time="0" value="0"/><key time="1" value="35"/></animate>"#;
        let camera_animation =
            r#"<animate property="x"><key time="0" value="160"/><key time="1" value="200"/></animate>"#;
        let xml = format!(
            r##"<scene version="1.2"><project width="320" height="180" fps="24" duration="2" background="#000000"/><assets><text id="t" width="200" height="100" text="M" size="48" font="DejaVu Sans" color="#FFFFFF"/></assets><composition><camera id="cam" x="160" y="90" z="-300">{}</camera><group id="g" threeD="true" x="30" y="20">{}<layer id="l" asset="t"><textAnimator rotationX="35" rotationY="30"/></layer></group></composition></scene>"##,
            if motion == "camera" { camera_animation } else { "" },
            if motion == "group" { animation } else { "" }
        );
        let d = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
        let sequential = common::render_times(&d, &[0.0, 1.0]).unwrap();
        let fresh = common::render_times(&d, &[1.0]).unwrap();
        assert!(sequential.px == fresh.px, "{motion}: cached glyph geometry must follow the camera/parent");
    }
}
