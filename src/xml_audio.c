#include "xml_internal.h"

#include <limits.h>
#include <stdlib.h>
#include <string.h>

static bool number(ParseContext *ctx, const char *element,
                   const XML_Char **attrs, const char *name, double *target) {
    const char *text = sr_xml_attr(attrs, name);
    if (!text) return true;
    if (!sr_parse_double(text, target)) {
        sr_xml_fail(ctx, element, name, "expected a finite decimal number");
        return false;
    }
    return true;
}

void sr_xml_start_audio_mix(ParseContext *ctx, const XML_Char **attrs) {
    const char *const allowed[] = {"sampleRate", "channels"};
    if (!sr_xml_attrs_allowed(ctx, "audioMix", attrs, allowed, 2)) return;
    const char *text = sr_xml_attr(attrs, "sampleRate");
    if (text && (!sr_parse_u32(text, &ctx->scene->audio.sample_rate) ||
                 ctx->scene->audio.sample_rate < 8000 ||
                 ctx->scene->audio.sample_rate > 384000))
        SR_XML_FAIL_RETURN(ctx, "audioMix", "sampleRate",
                           "expected an integer in [8000,384000]");
    text = sr_xml_attr(attrs, "channels");
    if (text && (!sr_parse_u32(text, &ctx->scene->audio.channels) ||
                 ctx->scene->audio.channels < 1 ||
                 ctx->scene->audio.channels > 2))
        SR_XML_FAIL_RETURN(ctx, "audioMix", "channels",
                           "this build supports one or two channels");
    ctx->seen_audio_mix = true;
}

void sr_xml_start_audio_track(ParseContext *ctx, const XML_Char **attrs) {
    const char *const allowed[] = {"id", "asset", "start", "clipIn",
                                    "clipOut", "loop", "volume", "pan"};
    if (!sr_xml_attrs_allowed(ctx, "audioTrack", attrs, allowed, 8)) return;
    const char *id = sr_xml_required(ctx, "audioTrack", attrs, "id");
    const char *asset = sr_xml_required(ctx, "audioTrack", attrs, "asset");
    if (ctx->failed) return;
    if (!sr_id_valid(id))
        SR_XML_FAIL_RETURN(ctx, "audioTrack", "id", "invalid identifier");
    if (sr_scene_id_exists(ctx->scene, id))
        SR_XML_FAIL_RETURN(ctx, "audioTrack", "id", "id must be globally unique");
    SrAudioMix *mix = &ctx->scene->audio;
    if (mix->track_count == mix->track_capacity) {
        size_t capacity = mix->track_capacity ? mix->track_capacity * 2 : 4;
        SrAudioTrack *tracks = sr_realloc(mix->tracks,
                                          capacity * sizeof(*tracks));
        if (!tracks) SR_XML_FAIL_RETURN(ctx, "audioTrack", NULL, "out of memory");
        memset(tracks + mix->track_capacity, 0,
               (capacity - mix->track_capacity) * sizeof(*tracks));
        mix->tracks = tracks;
        mix->track_capacity = capacity;
    }
    SrAudioTrack *track = &mix->tracks[mix->track_count++];
    track->clip_out = -1.0;
    track->volume = 1.0;
    track->id = sr_strdup(id);
    track->asset_id = sr_strdup(asset);
    track->source_line = sr_xml_line(ctx);
    if (!track->id || !track->asset_id)
        SR_XML_FAIL_RETURN(ctx, "audioTrack", NULL, "out of memory");
    if (!number(ctx, "audioTrack", attrs, "start", &track->start) ||
        !number(ctx, "audioTrack", attrs, "clipIn", &track->clip_in) ||
        !number(ctx, "audioTrack", attrs, "clipOut", &track->clip_out) ||
        !number(ctx, "audioTrack", attrs, "volume", &track->volume) ||
        !number(ctx, "audioTrack", attrs, "pan", &track->pan)) return;
    if (track->start < 0.0 || track->clip_in < 0.0 ||
        (track->clip_out >= 0.0 && track->clip_out <= track->clip_in) ||
        track->volume < 0.0 || track->volume > 1.0 ||
        track->pan < -1.0 || track->pan > 1.0)
        SR_XML_FAIL_RETURN(ctx, "audioTrack", "time/volume/pan",
                           "invalid track time, volume, or pan");
    const char *loops = sr_xml_attr(attrs, "loop");
    if (loops) {
        uint64_t value;
        if (!sr_parse_u64(loops, &value) || value > INT64_MAX)
            SR_XML_FAIL_RETURN(ctx, "audioTrack", "loop",
                               "expected a non-negative integer");
        track->loop_count = (int64_t)value;
    }
}
