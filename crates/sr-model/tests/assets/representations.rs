use sr_model::{assets::select_representation, model::AssetsChild, LoadOptions};

#[test]
fn representation_priority_and_color_metadata_are_shared() {
    let xml = r#"<scene version="1.1"><project width="32" height="32" fps="24" duration="1"/>
<assets><image id="i" src="base.png" width="32" height="32" proxy="implicit.png">
<representation name="de" src="named.png" language="en"/>
<representation name="regional" src="regional.png" language="de-AT"/>
<representation name="exact" src="exact.png" language="de-DE" colorSpace="linear-srgb"/>
<representation name="proxy" src="explicit.png"/>
</image></assets><composition/></scene>"#;
    let doc = sr_model::load_str(xml, &LoadOptions::without_assets()).unwrap();
    let AssetsChild::Image(a) = &doc.scene.assets.as_ref().unwrap().children[0] else { panic!() };
    let pick = |s| select_representation(&a.representations, a.proxy.as_deref(), s);
    assert!(pick(None).is_none());
    assert!(pick(Some("fr")).is_none());
    assert!(pick(Some("de-DE_proxy")).is_none());
    assert!(pick(Some("de_proxy")).is_none(), "a representation name is not a language tag");
    assert_eq!(pick(Some("de")).unwrap().src, "named.png");
    assert_eq!(pick(Some("DE-de")).unwrap().src, "exact.png");
    assert!(pick(Some("DE-de")).unwrap().color_space.is_some());
    assert_eq!(pick(Some("de-CH")).unwrap().src, "regional.png");
    assert_eq!(pick(Some("proxy")).unwrap().src, "explicit.png");
    assert_eq!(select_representation(&[], Some("implicit.png"), Some("proxy")).unwrap().src, "implicit.png");
}
