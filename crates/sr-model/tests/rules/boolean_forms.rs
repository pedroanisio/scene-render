//! SREP 84: the rules read an `xs:boolean` attribute by its value, true for "true" and "1", false for "false"
//! and "0", whitespace collapsed (XSD 1.0 Part 2, 3.2.2), and an integer attribute by its value, whatever its sign,
//! leading zeros or padding (3.3.13), as the model does (`parse.rs`). Each document writes one attribute of C15, C43,
//! R15, R48-from, R48-to, VOL5, C80, GEO2, PYRO9, PYRO10 or PYRO11 in a form other than the one the rule
//! compared before; they are sr-core's kit cases srep-0084-* (`conformance/srep_cases/srep-0084.json`).

use std::collections::BTreeSet;

fn errors(xml: &str) -> BTreeSet<String> {
    sr_model::validate_str(xml, &sr_model::LoadOptions::without_assets())
        .diagnostics
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| d.code)
        .collect()
}

fn assert_errors(xml: &str, want: &[&str]) {
    let want: BTreeSet<String> = want.iter().map(|s| s.to_string()).collect();
    assert_eq!(errors(xml), want, "{xml}");
}

fn doc(version: &str, head: &str, body: &str, tail: &str) -> String {
    format!(
        r#"<scene version="{version}"><project width="64" height="64" fps="24" duration="1"/>{head}<composition>{body}</composition>{tail}</scene>"#
    )
}

fn c15(attrs: &str) -> String {
    doc(
        "1.2",
        r#"<assets><image id="img" src="tile.png" width="16" height="16"/></assets>"#,
        &format!(
            r#"<layer id="l" asset="img" {attrs}><timeRemap><key time="0" value="0"/><key time="1" value="0.5"/></timeRemap></layer>"#
        ),
        "",
    )
}

#[test]
fn c15_reads_reverse_and_loop_by_value() {
    for attrs in [r#"reverse="0""#, r#"reverse=" false ""#, r#"loop="00""#, r#"loop="+0""#, r#"loop=" 0 ""#] {
        assert_errors(&c15(attrs), &[]);
    }
    for attrs in [r#"reverse="1""#, r#"reverse=" true ""#, r#"loop="+2""#, r#"loop="1""#] {
        assert_errors(&c15(attrs), &["C15"]);
    }
}

fn c43(alpha: &str) -> String {
    format!(
        r##"<scene version="1.2"><project width="64" height="64" fps="24" duration="1"/><output id="o" path="out/o.mp4" codec="h264" alpha="{alpha}"/><composition><shape id="s" shape="rect" width="10" height="10" fill="#FFFFFF"/></composition></scene>"##
    )
}

#[test]
fn c43_reads_alpha_by_value() {
    for alpha in ["true", "1", " true ", " 1 "] {
        assert_errors(&c43(alpha), &["C43"]);
    }
    for alpha in ["false", "0", " 0 "] {
        assert_errors(&c43(alpha), &[]);
    }
}

fn r15(has_audio: &str) -> String {
    doc(
        "1.2",
        &format!(
            r#"<assets><video id="v" src="clip.mp4" width="64" height="64" fps="24" duration="1" hasAudio="{has_audio}"/></assets>"#
        ),
        r##"<shape id="s" shape="rect" width="10" height="10" fill="#FFFFFF"/>"##,
        r#"<audioMix><audioTrack id="t" asset="v"/></audioMix>"#,
    )
}

#[test]
fn r15_reads_has_audio_by_value() {
    for has_audio in ["true", "1", " true ", " 1 "] {
        assert_errors(&r15(has_audio), &[]);
    }
    for has_audio in ["false", "0", " false "] {
        assert_errors(&r15(has_audio), &["R15"]);
    }
}

fn r48(threed: &str, from_inside: bool) -> String {
    let ends = if from_inside { r#"from="k" to="b""# } else { r#"from="r" to="k""# };
    doc(
        "1.2",
        "",
        &format!(
            r##"<shape id="r" shape="rect" width="8" height="8" x="4" y="4" fill="#FF0000"/><shape id="b" shape="rect" width="8" height="8" x="40" y="40" fill="#0000FF"/><group id="g" threeD="{threed}"><shape id="k" shape="rect" width="5" height="5" fill="#00FF00"/></group><connector id="c" {ends} stroke="#FFFFFF" strokeWidth="2"/>"##
        ),
        "",
    )
}

#[test]
fn r48_reads_three_d_by_value() {
    for threed in ["true", "1", " true ", " 1 "] {
        assert_errors(&r48(threed, true), &["R48-from"]);
        assert_errors(&r48(threed, false), &["R48-to"]);
    }
    for threed in ["false", "0", " 0 "] {
        assert_errors(&r48(threed, true), &[]);
    }
}

fn vol5(blackbody: &str) -> String {
    doc(
        "1.3",
        r#"<assets><volume id="smoke" src="uniform.srvol" boundsMinX="0" boundsMinY="0" boundsMinZ="0" boundsMaxX="32" boundsMaxY="32" boundsMaxZ="2"/></assets>"#,
        &format!(
            r#"<object3D id="cloud" primitive="volume" volume="smoke"><medium blackbody="{blackbody}" emissionScale="0.1"/></object3D>"#
        ),
        "",
    )
}

#[test]
fn vol5_reads_blackbody_by_value() {
    for blackbody in ["true", "1", " true ", " 1 "] {
        assert_errors(&vol5(blackbody), &["VOL5"]);
    }
    for blackbody in ["false", "0", " 0 "] {
        assert_errors(&vol5(blackbody), &[]);
    }
}

fn c80(attrs: &str) -> String {
    doc(
        "1.2",
        "",
        &format!(
            r##"<repeat id="r" x="32" y="32" {attrs}><points type="grid" columns="3" rows="2"/><shape id="s" shape="rect" width="4" height="4" fill="#FF0000"/></repeat>"##
        ),
        "",
    )
}

#[test]
fn c80_reads_from_and_step_by_value() {
    for attrs in [r#"from="+0""#, r#"from="00""#, r#"from=" 0 ""#, r#"step="+1""#, r#"step="01""#] {
        assert_errors(&c80(attrs), &[]);
    }
    for attrs in [r#"from="+2""#, r#"from="2""#, r#"step="+3""#] {
        assert_errors(&c80(attrs), &["C80"]);
    }
}

fn geo2(size: &str) -> String {
    format!(
        r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/><assets><tiles id="dem" src="terrain.pmtiles"/><map id="m" width="64" height="32" background="#FFFFFF"/></assets><composition><object3D id="earth" primitive="globe" map="m" terrain="dem" terrainTileSize="{size}" terrainZoom="0" planetRadius="1000" radius="20" x="32" y="32" segments="32"/></composition></scene>"##
    )
}

#[test]
fn geo2_reads_the_tile_size_by_value() {
    for size in ["64", "+64", "064", " +64 ", "+4096"] {
        assert_errors(&geo2(size), &[]);
    }
    for size in ["+48", "48", " 48 "] {
        assert_errors(&geo2(size), &["GEO2"]);
    }
}

const PYRO_FOLLOW: &str = r#"<scene version="1.3"><project width="32" height="32" fps="30" duration="1"/><composition><object3D id="cloud" primitive="volume"><pyro boundary="open" follow="true" followMargin="2" followLoss="0.000001" width="8" height="8" depth="8" voxelSize="1" dt="0.1"><pyroSource shape="sphere" radius="2" densityRate="2" start="0.15" end="0.35"/><pyroImpulse shape="box" width="2" height="2" depth="2" time="0.5" density="1" temperature="3000"/></pyro><medium blackbody="true" emissionScale="0.1"/></object3D></composition></scene>"#;

#[test]
fn pyro9_and_pyro10_read_follow_by_value() {
    for follow in ["true", "1", " true ", " 1 "] {
        let xml = PYRO_FOLLOW.replace(r#"follow="true""#, &format!(r#"follow="{follow}""#));
        assert_errors(&xml, &[]);
        assert_errors(&xml.replace(r#"boundary="open" "#, ""), &["PYRO9"]);
    }
    for follow in ["false", "0", " 0 "] {
        let xml = PYRO_FOLLOW.replace(r#"follow="true""#, &format!(r#"follow="{follow}""#));
        assert_errors(&xml, &["PYRO10"]);
    }
}

#[test]
fn pyro11_reads_the_margin_by_value() {
    for margin in ["+2", "02", " 2 ", "+3"] {
        assert_errors(&PYRO_FOLLOW.replace(r#"followMargin="2""#, &format!(r#"followMargin="{margin}""#)), &[]);
    }
    for margin in ["+4", "4", "04"] {
        assert_errors(&PYRO_FOLLOW.replace(r#"followMargin="2""#, &format!(r#"followMargin="{margin}""#)), &["PYRO11"]);
    }
}
