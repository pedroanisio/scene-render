/* SPDX-License-Identifier: Apache-2.0 */
/* Frame<->sample mapping, the sample-exact mixer, and the audio length of a
 * range render. */
#include <libavformat/avformat.h>

#include <stdlib.h>

#include "harness.h"
#include "scene_render/audio.h"
#include "scene_render/renderer.h"
#include "scene_render/xml.h"

static void frame_to_sample_is_exact(sr_test_ctx *t) {
    /* 30000/1001 at 48 kHz: floor(f * 48048000 / 30000), which fits in 64
     * bits for these frames, compared for a million frames. */
    uint64_t mismatches = 0, previous = 0, total = 0;
    for (uint64_t f = 0; f <= 1000000; ++f) {
        uint64_t expected = f * 48000u * 1001u / 30000u;
        uint64_t got = sr_frame_to_sample(f, 48000, 30000, 1001);
        mismatches += got != expected;
        if (f) total += got - previous;   /* per-frame blocks tile the range */
        previous = got;
    }
    CHECK_INT(t, mismatches, 0);
    CHECK_INT(t, total, sr_frame_to_sample(1000000, 48000, 30000, 1001));
    CHECK_INT(t, sr_frame_to_sample(1000000, 48000, 30000, 1001), 1601600000);
    /* Integer rates are exact multiples. */
    CHECK_INT(t, sr_frame_to_sample(30, 48000, 30, 1), 48000);
    /* Beyond 64-bit intermediate range: 2^40 frames at 384 kHz, 1001 den:
     * (2^40 * 384000 * 1001) / 60000 computed by hand in parts. */
    uint64_t f = (uint64_t)1 << 40;
    /* 384000 * 1001 / 60000 = 6406.4 exactly, so floor(f * 6406.4). */
    uint64_t expected = f * 6406u + (f * 2u) / 5u;
    CHECK_INT(t, sr_frame_to_sample(f, 384000, 60000, 1001), expected);
}

/* A scene with mono or stereo output and synthetic decoded assets. */
static SrAsset *pcm_asset(SrScene *scene, uint32_t channels, uint64_t frames,
                          float (*value)(uint64_t, uint32_t)) {
    SrAsset *asset = sr_scene_add_asset(scene);
    if (!asset) return NULL;
    asset->type = SR_ASSET_AUDIO;
    asset->audio_pcm = malloc(frames * channels * sizeof(float));
    asset->audio_frames = frames;
    asset->audio_decoded = true;
    for (uint64_t i = 0; i < frames; ++i)
        for (uint32_t c = 0; c < channels; ++c)
            asset->audio_pcm[i * channels + c] = value(i, c);
    return asset;
}

static SrAudioTrack *add_track(SrScene *scene, SrAsset *asset) {
    SrAudioMix *mix = &scene->audio;
    if (mix->track_count == mix->track_capacity) {
        size_t capacity = mix->track_capacity ? mix->track_capacity * 2 : 4;
        mix->tracks = realloc(mix->tracks, capacity * sizeof(*mix->tracks));
        mix->track_capacity = capacity;
    }
    SrAudioTrack *track = &mix->tracks[mix->track_count++];
    *track = (SrAudioTrack){.asset = asset, .clip_out = -1.0, .volume = 1.0,
                            .speed = 1.0};
    return track;
}

static float constant_half(uint64_t i, uint32_t c) {
    (void)i;
    return c == 0 ? 0.5f : 0.25f;
}

static float ramp(uint64_t i, uint32_t c) {
    (void)c;
    return (float)i / 1000.0f;
}

/* Two tracks: A (stereo 0.5/0.25) panned 0.5 with a 100-sample fade-in;
 * B (ramp) starting at sample 50, volume 0.5, 40-sample fade-out over its
 * 200-sample clip. Each output sample is computed here by hand. */
static void two_tracks_with_pan_and_fades(sr_test_ctx *t) {
    SrScene scene;
    sr_scene_init(&scene);
    scene.audio.sample_rate = 1000;
    scene.audio.channels = 2;
    SrAsset *a = pcm_asset(&scene, 2, 1000, constant_half);
    SrAsset *b = pcm_asset(&scene, 2, 1000, ramp);
    SrAudioTrack *ta = add_track(&scene, a);
    ta->pan = 0.5;
    ta->fade_in = 0.1;           /* 100 samples */
    ta->clip_out = 0.5;          /* 500 samples */
    SrAudioTrack *tb = add_track(&scene, b);
    tb->start = 0.05;            /* sample 50 */
    tb->volume = 0.5;
    tb->clip_in = 0.1;           /* source 100.. */
    tb->clip_out = 0.3;          /* ..300: 200 samples */
    tb->fade_out = 0.04;         /* 40 samples */
    SrMixer *mixer = NULL;
    CHECK_INT(t, sr_mixer_create(&scene, &mixer), SR_OK);
    enum { N = 600 };
    float out[N * 2];
    /* Split into uneven calls: results must not depend on the split. */
    sr_mixer_mix(mixer, 0, 7, out);
    sr_mixer_mix(mixer, 7, 293, out + 7 * 2);
    sr_mixer_mix(mixer, 300, N - 300, out + 300 * 2);
    double angle = (0.5 + 1.0) * SR_PI / 4.0;
    double gl = sqrt(2.0) * cos(angle), gr = sqrt(2.0) * sin(angle);
    int bad = 0;
    for (int s = 0; s < N; ++s) {
        float expected[2] = {0.0f, 0.0f};
        if (s < 500) {
            double fade = s < 100 ? s / 100.0 : 1.0;
            expected[0] += 0.5f * (float)(gl * fade);
            expected[1] += 0.25f * (float)(gr * fade);
        }
        if (s >= 50 && s < 250) {
            int l = s - 50;
            double fade = 200 - l < 40 ? (200 - l) / 40.0 : 1.0;
            float v = (float)(100 + l) / 1000.0f;
            expected[0] += v * (float)(0.5 * fade);
            expected[1] += v * (float)(0.5 * fade);
        }
        for (int c = 0; c < 2; ++c) {
            float e = fmaxf(-1.0f, fminf(1.0f, expected[c]));
            if (out[s * 2 + c] != e && bad++ < 5)
                SR_FAIL(t, "sample %d ch %d: %.9g, expected %.9g", s, c,
                        out[s * 2 + c], e);
        }
    }
    CHECK_INT(t, bad, 0);
    /* Fade-in starts from silence; hard-left pan of a mono-upmixed source
     * reaches unity on the left and silence on the right. */
    CHECK(t, out[0] == 0.0f && out[1] == 0.0f);
    sr_mixer_destroy(mixer);
    ta->pan = -1.0;
    ta->fade_in = 0.0;
    tb->volume = 0.0;
    CHECK_INT(t, sr_mixer_create(&scene, &mixer), SR_OK);
    sr_mixer_mix(mixer, 200, 1, out);
    CHECK_NEAR(t, out[0], 0.5 * sqrt(2.0), 1e-6);
    CHECK_NEAR(t, out[1], 0.0, 1e-6);
    sr_mixer_destroy(mixer);
    sr_scene_free(&scene);
}

static void reverse_and_speed_positions(sr_test_ctx *t) {
    SrScene scene;
    sr_scene_init(&scene);
    scene.audio.sample_rate = 1000;
    scene.audio.channels = 1;
    SrAsset *a = pcm_asset(&scene, 1, 1000, ramp);
    SrAudioTrack *track = add_track(&scene, a);
    track->clip_in = 0.1;        /* [100, 300) */
    track->clip_out = 0.3;
    track->loop_count = 2;
    track->reverse = true;
    SrMixer *mixer = NULL;
    float out[500];
    CHECK_INT(t, sr_mixer_create(&scene, &mixer), SR_OK);
    sr_mixer_mix(mixer, 0, 500, out);
    /* Reverse: output l plays source 299 - (l % 200), twice, then silence. */
    CHECK(t, out[0] == 299 / 1000.0f);
    CHECK(t, out[199] == 100 / 1000.0f);
    CHECK(t, out[200] == 299 / 1000.0f);
    CHECK(t, out[399] == 100 / 1000.0f);
    CHECK(t, out[400] == 0.0f);
    sr_mixer_destroy(mixer);
    /* speed=2 forward: output l reads source 100 + 2l (exact samples, no
     * interpolation weight); 2 plays of 200 source samples = 200 output. */
    track->reverse = false;
    track->speed = 2.0;
    CHECK_INT(t, sr_mixer_create(&scene, &mixer), SR_OK);
    sr_mixer_mix(mixer, 0, 500, out);
    int bad = 0;
    for (int l = 0; l < 200; ++l)
        bad += out[l] != (float)(100 + (2 * l) % 200) / 1000.0f;
    CHECK_INT(t, bad, 0);
    CHECK(t, out[200] == 0.0f);
    sr_mixer_destroy(mixer);
    /* speed=0.5 interpolates halfway between neighbours. */
    track->speed = 0.5;
    track->loop_count = 1;
    CHECK_INT(t, sr_mixer_create(&scene, &mixer), SR_OK);
    sr_mixer_mix(mixer, 0, 4, out);
    CHECK_NEAR(t, out[1], 100.5 / 1000.0, 1e-6);
    CHECK_NEAR(t, out[2], 101 / 1000.0, 1e-6);
    sr_mixer_destroy(mixer);
    /* speed=2 reversed: source 299 - 2l. */
    track->speed = 2.0;
    track->reverse = true;
    CHECK_INT(t, sr_mixer_create(&scene, &mixer), SR_OK);
    sr_mixer_mix(mixer, 0, 100, out);
    CHECK(t, out[0] == 299 / 1000.0f);
    CHECK(t, out[10] == 279 / 1000.0f);
    CHECK(t, out[99] == 101 / 1000.0f);
    sr_mixer_destroy(mixer);
    sr_scene_free(&scene);
}

/* Audio samples in the file's audio stream: last packet end minus first
 * packet pts, in 1/rate units. */
static int64_t audio_track_samples(const char *path, int *rate) {
    AVFormatContext *fmt = NULL;
    if (avformat_open_input(&fmt, path, NULL, NULL) < 0) return -1;
    avformat_find_stream_info(fmt, NULL);
    int si = av_find_best_stream(fmt, AVMEDIA_TYPE_AUDIO, -1, -1, NULL, 0);
    if (si < 0) {
        avformat_close_input(&fmt);
        return -1;
    }
    AVStream *st = fmt->streams[si];
    *rate = st->codecpar->sample_rate;
    AVPacket *pkt = av_packet_alloc();
    int64_t first = INT64_MAX, end = INT64_MIN;
    while (av_read_frame(fmt, pkt) >= 0) {
        if (pkt->stream_index == si && pkt->pts != AV_NOPTS_VALUE) {
            if (pkt->pts < first) first = pkt->pts;
            if (pkt->pts + pkt->duration > end) end = pkt->pts + pkt->duration;
        }
        av_packet_unref(pkt);
    }
    av_packet_free(&pkt);
    int64_t samples = av_rescale_q(end - first, st->time_base,
                                   (AVRational){1, *rate});
    avformat_close_input(&fmt);
    return samples;
}

/* A range render at 30000/1001 hands the encoder exactly S(end) - S(first)
 * samples, and the AAC track in the file agrees within one AAC frame. */
static void range_render_sample_count(sr_test_ctx *t) {
    FILE *sink = fopen("/dev/null", "w");
    SrDiagnostics diag;
    sr_diag_init(&diag, "audio-range", sink ? sink : stderr);
    SrScene scene;
    CHECK_INT(t, sr_scene_load_xml(sr_test_data_path("tests/data-media.xml"),
                                   &scene, &diag), SR_OK);
    scene.project.fps_num = 30000;
    scene.project.fps_den = 1001;
    char path[1024];
    snprintf(path, sizeof(path), "%s", sr_test_tmp_path("audio-range.mp4"));
    SrRenderOptions options = {.has_range = true, .first_frame = 10,
                               .end_frame = 25, .output_override = path,
                               .encoder_threads = 1};
    SrRenderMetrics metrics;
    CHECK_INT(t, sr_render(&scene, &options, &metrics, &diag), SR_OK);
    uint64_t expected = sr_frame_to_sample(25, 48000, 30000, 1001) -
                        sr_frame_to_sample(10, 48000, 30000, 1001);
    CHECK_INT(t, expected, 24024);
    CHECK_INT(t, metrics.audio_samples, expected);
    CHECK_INT(t, metrics.frames, 15);
    int rate = 0;
    int64_t in_file = audio_track_samples(path, &rate);
    CHECK_INT(t, rate, 48000);
    CHECK(t, llabs(in_file - (int64_t)expected) <= 1024);
    sr_scene_free(&scene);
    if (sink) fclose(sink);
}

const sr_test_case sr_tests_audio[] = {
    {"frame_to_sample_is_exact", frame_to_sample_is_exact},
    {"two_tracks_with_pan_and_fades", two_tracks_with_pan_and_fades},
    {"reverse_and_speed_positions", reverse_and_speed_positions},
    {"range_render_sample_count", range_render_sample_count},
    {NULL, NULL},
};
