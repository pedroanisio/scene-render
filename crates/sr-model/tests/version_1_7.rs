//! Document version 1.7: schema 1.7.0's. SREP 87 has no version gate, so 1.7 adds no syntax and no rule depends on it.

fn load(version: &str) -> Result<sr_model::Document, sr_model::LoadError> {
    let xml = format!(
        r##"<scene version="{version}"><project width="64" height="64" fps="10" duration="1"/><composition><shape id="s" shape="rect" width="10" height="10" fill="#FFFFFF"/></composition></scene>"##
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets())
}

#[test]
fn version_1_7_is_accepted_and_reported() {
    let d = load("1.7").expect("a 1.7 document validates");
    assert_eq!(d.version().as_str(), "1.7");
    assert!(load("1.6").is_ok());
}

/// Every document of the conformance corpus validates exactly alike at 1.6 and at 1.7: the new version gates nothing.
#[test]
fn the_corpus_validates_alike_at_1_6_and_1_7() {
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
            let (Some(a), Some(b)) = (set("1.6"), set("1.7")) else { continue };
            let opts = sr_model::LoadOptions { verify_assets: false, base_dir: f.parent().map(Into::into) };
            let codes = |x: &str| {
                let mut c: Vec<String> =
                    sr_model::validate_str(x, &opts).diagnostics.iter().map(|d| d.code.clone()).collect();
                c.sort();
                c
            };
            assert_eq!(codes(&a), codes(&b), "{}", f.display());
            compared += 1;
        }
    }
    assert!(compared > 100, "{compared} documents compared");
}
