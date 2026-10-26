//! The star-fields draft at the renderer (sr-core `srep/srep-0000-star-fields.md`): a dome light's `starField` is
//! reported among what a frame does not draw as authored until the renderer draws the field.

mod common;
use common::*;

const DIGEST: &str = "cd18b1c9b8c009d922edfed020ba718aa14a577416fb625bf01ddd9cbdaab617";

fn media() -> std::path::PathBuf {
    std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/corpus/media"))
}

fn document() -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.8"><project width="64" height="64" fps="24" duration="1" background="#000000FF"/><assets><catalogue id="bsc" src="bsc5-compact.csv.gz" digest="sha256:{DIGEST}"/></assets><composition><camera id="eye" z="-10"/></composition><lights><light id="sky" type="dome"><starField catalogue="bsc"/></light></lights></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(media()) };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

#[test]
fn a_star_field_the_renderer_does_not_draw_is_reported() {
    let Some(r) = render(&document()) else { return };
    assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
    assert!(r.stats.unsupported.iter().any(|u| u.starts_with("sky: starField")), "{:?}", r.stats.unsupported);
}
