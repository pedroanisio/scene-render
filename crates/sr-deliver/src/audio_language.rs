//! Where an audio source's language comes from when its track declares none: the selected representation of
//! its asset, or the asset itself. `metadata::audio_language` maps the tags to the stream's ISO 639-2 code.

use std::collections::BTreeSet;

use sr_eval::Program;
use sr_model::{
    assets::select_representation,
    model::{self as m, AssetsChild},
};

fn language<'a>(asset: &'a AssetsChild, wanted: Option<&str>) -> Option<&'a str> {
    match asset {
        AssetsChild::Audio(a) => select_representation(&a.representations, a.proxy.as_deref(), wanted)
            .and_then(|r| r.language)
            .or_else(|| a.language.as_ref().map(|l| l.0.as_str())),
        AssetsChild::Video(v) => {
            select_representation(&v.representations, v.proxy.as_deref(), wanted).and_then(|r| r.language)
        }
        AssetsChild::Generated(g) => g.language.as_ref().map(|l| l.0.as_str()),
        _ => None,
    }
}

pub(crate) fn audible_bus<'a>(p: &'a Program, mut bus: Option<&'a str>) -> bool {
    let mut seen = BTreeSet::new();
    while let Some(id) = bus {
        if !seen.insert(id) {
            return false;
        }
        let Some(b) = p.scene.audio_mix.iter().flat_map(|a| &a.children).find_map(|c| match c {
            m::AudioMixChild::Bus(b) if b.id == id => Some(b),
            _ => None,
        }) else {
            break;
        };
        if b.mute {
            return false;
        }
        bus = b.output.as_deref();
    }
    true
}

/// The language tag of asset `id`'s audio, as the representation selected for `wanted` declares it.
pub(crate) fn asset_language(p: &Program, id: &str, wanted: Option<&str>) -> Option<String> {
    crate::audio::find_asset(p, id).and_then(|(asset, _)| language(asset, wanted)).map(str::to_string)
}

/// The language tags of the audible video layers that carry sound into `output`.
pub(crate) fn layer_languages(p: &Program, output: &m::Output, wanted: Option<&str>) -> Vec<String> {
    let segmented = output.children.iter().any(|c| matches!(c, m::OutputChild::Segment(_)));
    let mut tags = Vec::new();
    for n in &p.nodes {
        let m::Node::Layer(l) = &*n.elem else { continue };
        if l.mute
            || !audible_bus(p, l.audio_bus.as_deref())
            || (segmented && !crate::segment_audio::selected(output, &n.id, "other", l.audio_bus.as_deref()))
        {
            continue;
        }
        if let Some((asset @ AssetsChild::Video(v), _)) =
            n.asset.as_deref().and_then(|id| crate::audio::find_asset(p, id))
        {
            if v.has_audio {
                tags.extend(language(asset, wanted).map(str::to_string));
            }
        }
    }
    tags
}
