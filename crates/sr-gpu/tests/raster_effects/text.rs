//! Text layers with animators, charts, codes,
//! formulas, colour emoji and burned-in captions.

use super::common;
use common::*;

fn doc_text(assets: &str, extra: &str, body: &str, w: u32, h: u32) -> sr_model::Document {
    let a = ASSETS.replace("</assets>", &format!("{assets}</assets>"));
    let xml = format!(
        r##"<scene version="1.1"><project width="{w}" height="{h}" fps="10" duration="4" background="#000000"/>{a}<composition>{body}</composition>{extra}</scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}"))
}

/// Sum of a channel over a rectangle.
fn sum(r: &Rendered, x0: u32, y0: u32, x1: u32, y1: u32, c: usize) -> f32 {
    let mut s = 0.0;
    for y in y0..y1 {
        for x in x0..x1 {
            s += r.at(x, y)[c];
        }
    }
    s
}

#[test]
fn text_charts_codes_formulas_and_captions() {
    let d = doc_text(
        r##"<text id="t" text="Hello" width="200" height="60" size="40" color="#FF0000" font="DejaVu Sans"/>
            <text id="e" text="😀" width="60" height="60" size="40"/>
            <chart id="c" kind="column" width="200" height="100" labels="a,b" showAxes="false"><series name="s" values="1 2" color="#00FF00"/></chart>
            <code id="q" kind="qr" data="HELLO" width="100" height="100"/>
            <formula id="f" tex="\frac{a}{b}" width="100" height="100" color="#0000FF"/>"##,
        r##"<captions><captionTrack id="cc" language="en" preset="karaoke" activeColor="#FFFF00" y="95%"><cue start="0" end="4" text="sing along now"/></captionTrack></captions>"##,
        r##"<layer id="lt" asset="t" x="0" y="0"/>
            <layer id="le" asset="e" x="220" y="0"/>
            <layer id="lc" asset="c" x="0" y="100"/>
            <layer id="lq" asset="q" x="220" y="100"/>
            <layer id="lf" asset="f" x="320" y="100"/>"##,
        480,
        300,
    );
    let Some(r) = render_times(&d, &[0.0, 2.0]) else { return };
    assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
    assert!(r.stats.unsupported.is_empty(), "{:?}", r.stats.unsupported);
    // red text in its box, nothing red elsewhere
    assert!(sum(&r, 0, 0, 200, 60, 0) > 300.0);
    assert!(sum(&r, 0, 0, 200, 60, 2) < 1.0);
    // the emoji is a colour bitmap: coloured pixels in its box
    let emoji: f32 = sum(&r, 220, 0, 280, 60, 0) + sum(&r, 220, 0, 280, 60, 1);
    assert!(emoji > 100.0, "{emoji}");
    // green columns
    assert!(sum(&r, 0, 100, 200, 200, 1) > 1000.0);
    // QR on white: a mix of dark and light modules
    let qr_light = sum(&r, 220, 100, 320, 200, 1);
    assert!(qr_light > 3000.0 && qr_light < 9500.0, "{qr_light}");
    // blue formula
    assert!(sum(&r, 320, 100, 420, 200, 2) > 100.0);
    // captions at the bottom: yellow (red + green) karaoke on the sung words
    let cap_r = sum(&r, 0, 220, 480, 300, 0);
    let cap_b = sum(&r, 0, 220, 480, 300, 2);
    assert!(cap_r > 200.0 && cap_r > cap_b * 1.5, "{cap_r} {cap_b}");
}

#[test]
fn text_animators_move_characters() {
    let body = |anim: &str| format!(r##"<layer id="lt" asset="t" x="0" y="0">{anim}</layer>"##);
    let asset = r##"<text id="t" text="ABCD" width="200" height="100" size="40" color="#FFFFFF" font="DejaVu Sans"/>"##;
    // before a fade-in starts the layer is empty; after it ends the text is fully drawn
    let d =
        doc_text(asset, "", &body(r#"<textAnimator preset="fade-in" presetStart="1" presetDuration="1"/>"#), 200, 100);
    let Some(r0) = render_times(&d, &[0.5]) else { return };
    assert!(sum(&r0, 0, 0, 200, 100, 0) < 1.0);
    let r1 = render_times(&d, &[2.5]).unwrap();
    let full = sum(&r1, 0, 0, 200, 100, 0);
    assert!(full > 300.0);
    // presetStart is on the layer's clock (composition time), not measured from the layer's start
    let late = r#"<layer id="lt" asset="t" x="0" y="0" start="2"><textAnimator preset="fade-in" presetStart="2.5" presetDuration="1"/></layer>"#;
    let d = doc_text(asset, "", late, 200, 100);
    assert!(sum(&render_times(&d, &[2.2]).unwrap(), 0, 0, 200, 100, 0) < 1.0);
    assert!(sum(&render_times(&d, &[4.0]).unwrap(), 0, 0, 200, 100, 0) > full * 0.95);
    // without presetStart the preset starts with the layer
    let own =
        r#"<layer id="lt" asset="t" x="0" y="0" start="2"><textAnimator preset="fade-in" presetDuration="1"/></layer>"#;
    let d = doc_text(asset, "", own, 200, 100);
    assert!(sum(&render_times(&d, &[2.1]).unwrap(), 0, 0, 200, 100, 0) < full * 0.5);
    assert!(sum(&render_times(&d, &[3.5]).unwrap(), 0, 0, 200, 100, 0) > full * 0.95);
    // a range selector moving y by +50 pushes the glyphs down
    let d = doc_text(asset, "", &body(r#"<textAnimator y="50"/>"#), 200, 100);
    let r = render_times(&d, &[0.0]).unwrap();
    assert!(sum(&r, 0, 0, 200, 45, 0) < full * 0.1);
    assert!((sum(&r, 0, 0, 200, 100, 0) - full).abs() < full * 0.05);
}

#[test]
fn per_character_blur_spreads_glyphs() {
    let body = |anim: &str| format!(r##"<layer id="lt" asset="t" x="0" y="0">{anim}</layer>"##);
    let asset = r##"<text id="t" text="I I" width="200" height="100" size="60" color="#FFFFFF" font="DejaVu Sans"/>"##;
    let d = doc_text(asset, "", &body(""), 200, 100);
    let Some(sharp) = render_times(&d, &[0.0]) else { return };
    let d = doc_text(asset, "", &body(r#"<textAnimator blur="8"/>"#), 200, 100);
    let soft = render_times(&d, &[0.0]).unwrap();
    assert!(!soft.stats.unsupported.iter().any(|m| m.contains("blur")), "{:?}", soft.stats.unsupported);
    // white text over the opaque background: measure the red channel
    let covered = |r: &Rendered| r.px.iter().filter(|p| p[0] > 0.02).count();
    let total = |r: &Rendered| sum(r, 0, 0, 200, 100, 0);
    assert!(covered(&soft) > covered(&sharp) * 2, "blur spreads: {} vs {}", covered(&soft), covered(&sharp));
    assert!(
        (total(&soft) - total(&sharp)).abs() < total(&sharp) * 0.1,
        "and keeps coverage: {} vs {}",
        total(&soft),
        total(&sharp)
    );
}

#[test]
fn text_animators_move_variable_font_axes() {
    let body = |anim: &str| format!(r##"<layer id="lt" asset="t" x="0" y="0">{anim}</layer>"##);
    // a variable font made by tools/fixtures/make_variable_font.py: the stem of "I" widens along wght
    let font = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fonts/wght-test.ttf");
    let asset = format!(
        r##"<font id="vf" src="{}" family="SR Wght Test"/><text id="t" text="IIII" width="200" height="100" size="48" color="#FFFFFF" font="SR Wght Test"/>"##,
        font.display()
    );
    let asset = asset.as_str();
    let Some(regular) = render_times(&doc_text(asset, "", &body(""), 200, 100), &[0.0]) else { return };
    let bold =
        render_times(&doc_text(asset, "", &body(r#"<textAnimator variation="wght 700"/>"#), 200, 100), &[0.0]).unwrap();
    assert!(bold.stats.unsupported.iter().all(|m| !m.contains("variation")), "{:?}", bold.stats.unsupported);
    let (a, b) = (sum(&regular, 0, 0, 200, 100, 0), sum(&bold, 0, 0, 200, 100, 0));
    assert!(b > a * 1.2, "wght 700 inks more than the default 400: {b} vs {a}");
}

#[test]
fn unreadable_font_assets_are_reported() {
    // a font file the renderer cannot read falls back to another family, and says so
    let bad = std::env::temp_dir().join(format!("sr-bad-font-{}.ttf", std::process::id()));
    std::fs::write(&bad, b"not a font").unwrap();
    let asset = format!(
        r##"<font id="broken" src="{}" family="Broken"/><text id="t" text="I" width="50" height="50" size="20" color="#FFFFFF" font="Broken"/>"##,
        bad.display()
    );
    let body = r##"<layer id="lt" asset="t"/>"##;
    let r = render_times(&doc_text(&asset, "", body, 50, 50), &[0.0]);
    std::fs::remove_file(&bad).ok();
    let Some(r) = r else { return };
    assert!(r.stats.unsupported.iter().any(|m| m.contains("font asset broken")), "{:?}", r.stats.unsupported);
}

#[test]
fn font_assets_are_loaded_even_though_no_layer_references_them() {
    // <font> assets are referenced from text (fontAsset), never by layers; they must still be loaded, not
    // silently replaced by the default family. A monospaced "i" is several times wider than a proportional one.
    let mono = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf";
    if !std::path::Path::new(mono).exists() {
        return;
    }
    let ink_right = |attrs: &str| {
        let d = doc_text(
            &format!(
                r##"<font id="fm" src="{mono}" family="DejaVu Sans Mono"/><text id="t" text="iiiiiiiiii" width="460" height="60" size="40" color="#FFFFFF" {attrs}/>"##
            ),
            "",
            r#"<layer id="lt" asset="t" x="0" y="0"/>"#,
            480,
            64,
        );
        let r = render(&d)?;
        assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
        (0..480u32).rev().find(|&x| (0..64).any(|y| r.at(x, y)[0] > 0.5))
    };
    let (Some(prop), Some(mono_w)) = (ink_right(r#"font="DejaVu Sans""#), ink_right(r#"fontAsset="fm""#)) else {
        return;
    };
    assert!(
        mono_w as f32 > prop as f32 * 1.8,
        "fontAsset must select the monospaced file: {mono_w} vs proportional {prop}"
    );
}

#[test]
fn text_takes_its_colour_from_its_style_when_it_sets_none() {
    // The model fills the text asset's schema default colour (#FFFFFFFF) in; it must not override the colour of
    // the text's style, inherited through basedOn and given as a token. An explicit colour still wins.
    let run = |attrs: &str| {
        let a = ASSETS.replace(
            "</assets>",
            &format!(r##"<text id="t" text="HHHH" width="200" height="60" size="44" font="DejaVu Sans" style="t-red" {attrs}/></assets>"##),
        );
        let xml = format!(
            r##"<scene version="1.1"><project width="200" height="60" fps="10" duration="1" background="#000000"/><styles><token name="brand-red" value="#FF0000"/><textStyle id="t-base" color="var(--brand-red)"/><textStyle id="t-red" basedOn="t-base" size="44"/></styles>{a}<composition><layer id="lt" asset="t" x="0" y="0"/></composition></scene>"##
        );
        let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
        let d = sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}"));
        let r = render(&d)?;
        assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
        Some((sum(&r, 0, 0, 200, 60, 0), sum(&r, 0, 0, 200, 60, 1)))
    };
    let Some((red, green)) = run("") else { return };
    assert!(red > 50.0 && green < red * 0.05, "styled text is red, not white: r {red} g {green}");
    let Some((red, green)) = run(r##"color="#00FF00""##) else { return };
    assert!(green > 50.0 && red < green * 0.05, "an explicit colour wins over the style: r {red} g {green}");
}

#[test]
fn caption_source_newlines_are_opt_in_and_default_pixels_stay_identical() {
    let make = |attrs: &str, text: &str, preset: &str| {
        doc_text(
            "",
            &format!(
                r##"<captions><captionTrack id="cc" language="en" preset="{preset}" {attrs} maxCharsPerLine="80" y="50%"><cue start="0" end="4" text="{text}"/></captionTrack></captions>"##
            ),
            "",
            320,
            240,
        )
    };
    for preset in ["classic", "highlight", "karaoke", "one-word"] {
        for time in [0.25, 1.75, 3.25] {
            let Some(legacy) = render_times(&make("", "aa bb&#10;cc dd", preset), &[time]) else {
                panic!("a software Vulkan adapter is required for caption compatibility evidence")
            };
            let greedy = render_times(&make(r#"lineBreaks="greedy""#, "aa bb&#10;cc dd", preset), &[time]).unwrap();
            let flattened = render_times(&make("", "aa bb cc dd", preset), &[time]).unwrap();
            assert!(legacy.stats.errors.is_empty() && legacy.stats.unsupported.is_empty());
            assert_eq!(legacy.px, greedy.px, "explicit greedy changes {preset} at {time}");
            assert_eq!(legacy.px, flattened.px, "default flattens newlines: {preset} at {time}");
            {
                let source = render_times(&make(r#"lineBreaks="source""#, "aa bb&#10;cc dd", preset), &[time]).unwrap();
                if preset == "one-word" {
                    assert_eq!(legacy.px, source.px);
                } else {
                    assert_ne!(legacy.px, source.px, "source did not preserve the newline");
                }
                assert!(source.stats.errors.is_empty() && source.stats.unsupported.is_empty());
            }
        }
    }
}

#[test]
fn caption_source_words_keep_breaks_when_profanity_is_filtered() {
    let d = doc_text(
        "",
        r#"<captions><captionTrack id="cc" language="en" lineBreaks="source" profanityFilter="true"><cue start="0" end="4" text="fuck&#10;shit"/></captionTrack></captions>"#,
        "",
        320,
        240,
    );
    let track = &d.scene.captions.as_ref().unwrap().caption_tracks[0];
    let cues = sr_gpu::text::track_cues(track, std::path::Path::new("")).unwrap();
    assert_eq!(cues[0].text, "f***\ns***");
    let pages =
        sr_text::captions::paginate_with_line_breaks(&cues, None, 80, 2, false, sr_text::captions::LineBreaks::Source);
    assert_eq!(pages[0].text(), "f***\ns***");
    assert_eq!(pages[0].words().len(), 2);
}

#[test]
fn a_burned_caption_uses_its_font_asset_whether_or_not_a_text_asset_is_drawn() {
    // font assets load with the first text drawn; a caption on its own must load them too, or it falls back to the
    // default family. A monospaced "i" is several times wider than a proportional one.
    let mono = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf";
    if !std::path::Path::new(mono).exists() {
        return;
    }
    let ink_width = |with_text: bool| {
        let layer = if with_text { r#"<layer id="lt" asset="t" x="0" y="0"/>"# } else { "" };
        let xml = format!(
            r##"<scene version="1.1"><project width="480" height="120" fps="10" duration="1" background="#000000"/>
<styles><textStyle id="cap" fontAsset="fm" size="30" color="#FFFFFF"/></styles>
<assets><font id="fm" src="{mono}" family="DejaVu Sans Mono"/><text id="t" text="." width="20" height="20" size="10" color="#000000"/></assets>
<composition>{layer}</composition>
<captions><captionTrack id="cc" language="en" mode="burn" preset="classic" style="cap" maxCharsPerLine="40" x="50%" y="55%" width="90%"><cue start="0" end="1" text="iiiiiiiiii"/></captionTrack></captions></scene>"##
        );
        let d = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}"));
        let r = render(&d)?;
        assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
        let lit = |x: u32| (60..120).any(|y| r.at(x, y)[0] > 0.5);
        let (a, b) = ((0..480u32).find(|&x| lit(x))?, (0..480u32).rev().find(|&x| lit(x))?);
        Some(b - a)
    };
    let (Some(with), Some(without)) = (ink_width(true), ink_width(false)) else { return };
    assert!(
        without as f32 >= with as f32 * 0.9,
        "the caption lost its font asset when no text layer was drawn: ink {without} px against {with} px with one"
    );
}

#[test]
fn mask_reveal_clips_each_line_to_its_own_box() {
    // a mask-reveal unit waits 1.1 em below its place, which is inside the next line's box: with one clip over
    // the union of the line boxes, line 1 showed through line 2's box before its window; each line has its own clip
    let asset = r##"<text id="t" text="AAAA&#10;BBBB" width="200" height="120" size="40" color="#FFFFFF" font="DejaVu Sans"/>"##;
    let layer = r#"<layer id="lt" asset="t" x="0" y="0"><textAnimator preset="mask-reveal" presetStart="1" presetDuration="1"/></layer>"#;
    let d = doc_text(asset, "", layer, 200, 120);
    let Some(waiting) = render_times(&d, &[0.5]) else { return };
    let ink = sum(&waiting, 0, 0, 200, 120, 0);
    assert!(ink < 1.0, "nothing shows before the preset starts: {ink}");
    let done = render_times(&d, &[2.5]).unwrap();
    assert!(sum(&done, 0, 0, 200, 120, 0) > 300.0, "both lines are fully drawn after it ends");
}

#[test]
fn counter_starts_on_the_same_clock_as_the_other_presets() {
    // presetStart is composition time for every preset (the layer's parent timeline), not an offset from the
    // layer's start; absent, a preset starts with its layer
    let asset = r##"<text id="t" text="8888" width="200" height="100" size="40" color="#FFFFFF" font="DejaVu Sans"/>"##;
    let layer = |attrs: &str| {
        format!(
            r#"<layer id="lt" asset="t" x="0" y="0" start="2"><textAnimator preset="counter" presetDuration="1" {attrs}/></layer>"#
        )
    };
    let ink = |d: &sr_model::Document, t: f64| sum(&render_times(d, &[t]).unwrap(), 0, 0, 200, 100, 0);
    let d = doc_text(asset, "", &layer(r#"presetStart="2.5""#), 200, 100);
    let Some(first) = render_times(&d, &[2.2]) else { return };
    let before = sum(&first, 0, 0, 200, 100, 0);
    let after = ink(&d, 4.0);
    // before the start the count shows 0000 -> "0" digits; after it ends the authored 8888 is shown (more ink)
    assert!(
        after > before * 1.2,
        "counter with presetStart=2.5 on a layer starting at 2: before {before}, after {after}"
    );
    // without presetStart the count starts with the layer
    let own = doc_text(asset, "", &layer(""), 200, 100);
    assert!(ink(&own, 2.05) < ink(&own, 3.5) * 0.95, "the count runs from the layer's start");
}

/// A 200 x 100 chart whose kind is the string parameter `k`, bound to it after validation.
fn chart_with_kind_param(k: &str) -> sr_model::Document {
    let a = ASSETS.replace(
        "</assets>",
        r##"<chart id="c" kind="column" width="200" height="100" labels="a,b" showAxes="false"><series name="s" values="1 2" color="#00FF00"/></chart></assets>"##,
    );
    let xml = format!(
        r##"<scene version="1.1"><project width="200" height="100" fps="10" duration="1" background="#000000"/><parameters><param id="k" type="string" default="{k}"/><bind param="k" target="c" property="kind"/></parameters>{a}<composition><layer id="lc" asset="c" x="0" y="0"/></composition></scene>"##
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) })
        .unwrap_or_else(|e| panic!("{e:?}"))
}

#[test]
fn an_unknown_chart_kind_is_a_render_error_and_draws_no_column_chart() {
    let Some(ok) = render_times(&chart_with_kind_param("bar"), &[0.0]) else { return };
    assert!(ok.stats.errors.is_empty(), "{:?}", ok.stats.errors);
    assert!(sum(&ok, 0, 0, 200, 100, 1) > 1000.0, "a known kind draws its green series");
    // Typed parameter bindings reject unknown enum literals during evaluation.
    // Exercise the renderer boundary with an overridden evaluated property.
    let d = chart_with_kind_param("column");
    let ev = sr_eval::Evaluator::new(&d, &Default::default()).unwrap();
    let mut frame = ev.evaluate(0.0);
    frame.elements.push(sr_eval::eval::ElementState {
        key: "c".into(),
        element: "chart",
        props: sr_eval::eval::Props(vec![("kind".into(), sr_eval::Value::Str("pyramid".into()))]),
    });
    let mut renderer = sr_gpu::Renderer::new(gpu().unwrap(), ev.program());
    let rendered = renderer.render(&frame, ev.program());
    let bad =
        Rendered { px: renderer.read(&rendered.texture), size: rendered.texture.size, stats: rendered.stats, renderer };
    assert!(bad.stats.errors.iter().any(|e| e.contains("pyramid") && e.contains("chart")), "{:?}", bad.stats.errors);
    assert!(sum(&bad, 0, 0, 200, 100, 1) < 1.0, "nothing is drawn for an unknown kind");
}

/// A 640×320 version-1.8 document of `assets` and `body` (the semantic-tex draft).
fn doc_1_8(assets: &str, body: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.8"><project width="640" height="320" fps="10" duration="2" background="#000000"/><assets>{assets}</assets><composition>{body}</composition></scene>"##
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}"))
}

/// The red-weighted centroid of a rectangle, and the red it weighs.
fn centroid(r: &Rendered, x0: u32, y0: u32, x1: u32, y1: u32) -> ([f64; 2], f64) {
    let (mut s, mut sx, mut sy) = (0.0, 0.0, 0.0);
    for y in y0..y1 {
        for x in x0..x1 {
            let v = r.at(x, y)[0] as f64;
            s += v;
            sx += v * (x as f64 + 0.5);
            sy += v * (y as f64 + 0.5);
        }
    }
    ([sx / s.max(1e-9), sy / s.max(1e-9)], s)
}

#[test]
fn formula_parts_animate_and_formulas_morph() {
    // the kit's formula-part case: E = mc² in white at size 48, the rhs part moved up 40 px by a formulaAnimator
    let eq = r"\part{lhs}{E} = \part{rhs}{m c^2}";
    let f = format!(r##"<formula id="f" tex="{eq}" width="640" height="320" size="48" color="#FFFFFF"/>"##);
    let plain = doc_1_8(&f, r#"<layer id="l" asset="f" x="0" y="0"/>"#);
    let Some(a) = render_times(&plain, &[0.0]) else { return };
    assert!(a.stats.errors.is_empty(), "{:?}", a.stats.errors);
    let moved = doc_1_8(&f, r#"<layer id="l" asset="f" x="0" y="0"><formulaAnimator part="rhs" y="-40"/></layer>"#);
    let b = render_times(&moved, &[0.0]).unwrap();
    assert!(b.stats.errors.is_empty(), "{:?}", b.stats.errors);
    let t = sr_text::formula::typeset_parts(&mut sr_text::FontLib::new(true), eq, 48.0, [640.0, 320.0], 0.1).unwrap();
    let rhs = t.boxes[1].unwrap();
    let split = rhs.x.floor() as u32 - 1;
    // the lhs glyphs are pixel-identical
    for y in 0..320 {
        for x in 0..split {
            assert_eq!(a.at(x, y), b.at(x, y), "({x}, {y})");
        }
    }
    // the rhs ink is the plain rhs moved up 40 px, within 0.5 px (rule 11)
    let ((pa, wa), (pb, wb)) = (centroid(&a, split, 0, 640, 320), centroid(&b, split, 0, 640, 320));
    assert!(wa > 100.0 && (wa - wb).abs() < 0.01 * wa, "{wa} {wb}");
    assert!((pb[0] - pa[0]).abs() < 0.5 && (pb[1] - pa[1] + 40.0).abs() < 0.5, "{pa:?} {pb:?}");
    // an unknown part is an error of the layer
    let bad = doc_1_8(&f, r#"<layer id="l" asset="f" x="0" y="0"><formulaAnimator part="mid"/></layer>"#);
    let e = render_times(&bad, &[0.0]).unwrap();
    assert!(e.stats.errors.iter().any(|m| m.contains("mid")), "{:?}", e.stats.errors);

    // a formulaMorph layer: the from formula at progress 0, the to formula at 1, something of both between
    let m = format!(
        r##"{f}<formula id="g" tex="\part{{lhs}}{{E}} = \part{{rhs}}{{m^2}}" width="640" height="320" size="48"/>
            <formulaMorph id="m" from="f" to="g" width="640" height="320" size="48" color="#FFFFFF"><animate property="progress"><key time="0" value="0"/><key time="1" value="1"/></animate></formulaMorph>"##
    );
    let morph = doc_1_8(&m, r#"<layer id="l" asset="m" x="0" y="0"/>"#);
    let m0 = render_times(&morph, &[0.0]).unwrap();
    assert!(
        m0.stats.errors.is_empty() && m0.stats.unsupported.is_empty(),
        "{:?} {:?}",
        m0.stats.errors,
        m0.stats.unsupported
    );
    assert_eq!(m0.px, a.px, "the from formula at progress 0");
    let to = doc_1_8(
        r##"<formula id="g" tex="\part{lhs}{E} = \part{rhs}{m^2}" width="640" height="320" size="48" color="#FFFFFF"/>"##,
        r#"<layer id="l" asset="g" x="0" y="0"/>"#,
    );
    let (m1, g1) = (render_times(&morph, &[1.0]).unwrap(), render_times(&to, &[0.0]).unwrap());
    assert_eq!(m1.px, g1.px, "the to formula at progress 1");
    let mid = render_times(&morph, &[0.5]).unwrap();
    let ink = |r: &Rendered| sum(r, 0, 0, 640, 320, 0);
    assert!(ink(&mid) > 0.5 * ink(&g1).min(ink(&a)), "{} {} {}", ink(&mid), ink(&a), ink(&g1));
    assert!(mid.px != a.px && mid.px != g1.px);
}
