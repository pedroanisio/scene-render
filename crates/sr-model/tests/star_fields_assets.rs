//! The star-fields draft's file check (assets.rs): SF1, a catalogue asset's file is the gzipped nine-column CSV of
//! rule 1 whose sha256 equals its digest.

use std::path::PathBuf;

const DIGEST: &str = "cd18b1c9b8c009d922edfed020ba718aa14a577416fb625bf01ddd9cbdaab617";
const EXCERPT_DIGEST: &str = "d113bb5117946244cf63de9d06287d5aac008acd1a561d28b760bcc7d4c3d1fd";

fn media() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/corpus/media"))
}

fn codes(src: &str, digest: &str) -> Vec<String> {
    codes_in(media(), src, digest)
}

fn codes_in(dir: PathBuf, src: &str, digest: &str) -> Vec<String> {
    let xml = format!(
        r#"<scene version="1.8"><project width="64" height="64" fps="24" duration="1"/><assets><catalogue id="bsc" src="{src}" digest="sha256:{digest}"/></assets><composition/><lights><light id="sky" type="dome"><starField catalogue="bsc"/></light></lights></scene>"#
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(dir) };
    sr_model::validate_str(&xml, &opts).diagnostics.into_iter().map(|d| d.code).collect()
}

#[test]
fn the_bright_star_catalogue_with_its_digest_is_valid() {
    let got = codes("bsc5-compact.csv.gz", DIGEST);
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn a_file_of_another_digest_or_kind_is_refused_sf1() {
    let mut wrong = DIGEST.to_string();
    wrong.replace_range(0..1, "0");
    assert_eq!(codes("bsc5-compact.csv.gz", &wrong), ["SF1"]);
    // an SPK file pinned by its own digest is not a gzipped CSV
    assert_eq!(codes("de440s-excerpt.bsp", EXCERPT_DIGEST), ["SF1"]);
}

#[test]
fn a_missing_or_remote_file_is_the_asset_stages_as_for_any_asset() {
    assert_eq!(codes("absent.csv.gz", DIGEST), ["A01"]);
    assert_eq!(codes("https://example.com/bsc5.csv.gz", DIGEST), ["A03"]);
}

#[test]
fn a_malformed_digest_is_the_structure_stages_alone() {
    let got = codes("bsc5-compact.csv.gz", &DIGEST.to_uppercase());
    assert_eq!(got, ["S06"]);
}

/// A catalogue of `rows` under the header, gzipped into a directory of its own, with its sha256.
fn catalogue(name: &str, rows: &str) -> (PathBuf, String) {
    use sha2::Digest;
    use std::io::Write;
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("star_fields_assets").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(format!("HR,Name,RA,Dec,V,B-V,pmRA,pmDE,SpType\n{rows}").as_bytes()).unwrap();
    let raw = e.finish().unwrap();
    std::fs::write(dir.join("c.csv.gz"), &raw).unwrap();
    (dir, sha2::Sha256::digest(&raw).iter().map(|b| format!("{b:02x}")).collect())
}

#[test]
fn a_proper_motion_beyond_a_hundred_arcseconds_a_year_is_refused_sf1() {
    let (dir, digest) = catalogue("fastest", "1,a,10,5,4,0.5,100,-100,G0\n");
    let got = codes_in(dir, "c.csv.gz", &digest);
    assert!(got.is_empty(), "{got:?}");
    for (name, row) in [("ra", "1,a,10,5,4,0.5,100.5,0,G0\n"), ("dec", "1,a,10,5,4,0.5,0,-101,G0\n")] {
        let (dir, digest) = catalogue(name, row);
        assert_eq!(codes_in(dir, "c.csv.gz", &digest), ["SF1"], "{row}");
    }
}
