//! Document version 1.8: the random-streams draft's (schema/UPSTREAM), which changes the draws of simulations
//! (crates/sr-eval/tests/scene/version_1_8.rs), and the media draft's, which adds the medium attributes phase,
//! phaseAlpha and dropletDiameter (MED4).

fn load(version: &str) -> Result<sr_model::Document, sr_model::LoadError> {
    let xml = format!(
        r##"<scene version="{version}"><project width="64" height="64" fps="10" duration="1"/><composition><shape id="s" shape="rect" width="10" height="10" fill="#FFFFFF"/></composition></scene>"##
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets())
}

#[test]
fn version_1_8_is_accepted_and_reported() {
    assert_eq!(sr_model::SCHEMA_VERSION, "1.8");
    let d = load("1.8").expect("a 1.8 document validates");
    assert_eq!(d.version().as_str(), "1.8");
    assert!(d.version() > sr_model::Version::V1_7);
    assert!(load("1.7").is_ok());
}

#[test]
fn a_version_after_1_8_is_not() {
    for v in ["1.9", "2.0", "1.80"] {
        assert!(load(v).is_err(), "{v} is accepted");
    }
}

/// Every document of the conformance corpus validates alike at 1.7 and at 1.8 but for MED4, the gate of the media
/// draft's medium attributes, and V17, the gate of the plots draft's elements.
#[test]
fn the_corpus_validates_alike_at_1_7_and_1_8() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus");
    let mut compared = 0;
    for dir in ["valid", "invalid"] {
        let mut files: Vec<_> = std::fs::read_dir(root.join(dir))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.to_string_lossy().ends_with(".scene.xml"))
            .collect();
        files.sort();
        for f in files {
            let xml = std::fs::read_to_string(&f).unwrap();
            let Some(at) = xml.find("<scene") else { continue };
            let head_end = at + xml[at..].find('>').unwrap();
            let set = |v: &str| {
                let head = &xml[at..head_end];
                let i = head.find("version=\"")?;
                let j = i + 9 + head[i + 9..].find('"').unwrap();
                Some(format!("{}{v}{}", &xml[..at + i + 9], &xml[at + j..]))
            };
            let (Some(a), Some(b)) = (set("1.7"), set("1.8")) else { continue };
            let opts = sr_model::LoadOptions { verify_assets: false, base_dir: f.parent().map(Into::into) };
            let codes = |x: &str| {
                let mut c: Vec<String> =
                    sr_model::validate_str(x, &opts).diagnostics.iter().map(|d| d.code.clone()).collect();
                c.sort();
                c
            };
            // MED4 and V17 are the differences: a 1.7 document may not carry the phase attributes, the plots or the space transforms 1.8 adds
            let phased = xml.split("<medium").skip(1).any(|tag| {
                let tag = &tag[..tag.find('>').unwrap_or(tag.len())];
                [" phase=", " phaseAlpha=", " dropletDiameter="].iter().any(|k| tag.contains(k))
            });
            let mut at_1_7 = codes(&a);
            let gated = at_1_7.iter().any(|c| c == "MED4");
            at_1_7.retain(|c| c != "MED4");
            assert_eq!(gated, phased && !codes(&b).iter().any(|c| c == "S01"), "{}", f.display());
            // and the space transforms: the three modifier types and a deform as a group's first child
            let group_deform =
                xml.split("<group").skip(1).any(|t| t[t.find('>').map_or(t.len(), |i| i + 1)..].starts_with("<deform"));
            let plotted = group_deform
                || ["<axes ", "<plot ", "<axes3D ", "<plot3D ", "primitive=\"axes\""].iter().any(|k| xml.contains(k))
                || ["matrix", "map", "complex"].iter().any(|t| xml.contains(&format!("<modifier type=\"{t}\"")))
                // and the semantic-tex draft's animator, morph asset, shape/@morph and \part tag
                || ["<formulaAnimator", "<formulaMorph", " morph=\"", "\\part{"].iter().any(|k| xml.contains(k))
                // and the reactive-values draft's value asset
                || xml.contains("<value id=")
                // and the fields draft's field node
                || xml.contains("<field ");
            let plot_gated = at_1_7.iter().any(|c| c == "V17");
            at_1_7.retain(|c| c != "V17");
            assert_eq!(plot_gated, plotted && !codes(&b).iter().any(|c| c == "S01"), "{}", f.display());
            assert_eq!(at_1_7, codes(&b), "{}", f.display());
            compared += 1;
        }
    }
    assert!(compared > 100, "{compared} documents compared");
}

/// MED4 (the media draft): phase, phaseAlpha and dropletDiameter need version 1.8; the media of earlier versions are
/// Henyey–Greenstein, as they were.
#[test]
fn the_phase_attributes_need_version_1_8() {
    let codes = |version: &str, attrs: &str| {
        let xml = format!(
            r#"<scene version="{version}"><project width="32" height="32" fps="30" duration="1"/><assets><volume id="smoke" src="smoke.srvol"/></assets><composition><object3D id="v" primitive="volume" volume="smoke"><medium {attrs} extinction="0.5"/></object3D></composition></scene>"#
        );
        let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
        sr_model::validate_str(&xml, &opts).diagnostics.into_iter().map(|d| d.code).collect::<Vec<_>>()
    };
    for attrs in [
        r#"phase="rayleigh""#,
        r#"phase="hg""#,
        r#"phase="draine" phaseAlpha="2""#,
        r#"phase="fog" dropletDiameter="20""#,
    ] {
        assert!(codes("1.8", attrs).is_empty(), "{attrs}: {:?}", codes("1.8", attrs));
        for v in ["1.3", "1.5", "1.6", "1.7"] {
            assert!(codes(v, attrs).contains(&"MED4".to_string()), "{v} {attrs}: {:?}", codes(v, attrs));
        }
    }
    assert!(codes("1.7", r#"anisotropy="0.5""#).is_empty());
}
