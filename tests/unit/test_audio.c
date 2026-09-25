/* SPDX-License-Identifier: Apache-2.0 */
/* Frame<->sample mapping, the sample-exact mixer, and the audio length of a
 * range render. */
#include <libavformat/avformat.h>

#include <stdlib.h>

#include "harness.h"
#include "media_synth.h"
#include "scene_render/audio.h"
#include "scene_render/encoder.h"
#include "scene_render/renderer.h"
#include "scene_render/xml.h"
#include "../../src/audio_internal.h"

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

/* Samples of the file's soundtrack as the core decodes it (default libav
 * decoding, so skip-samples and discard-padding side data are honoured). */
static int64_t decoded_samples(const char *path) {
    float *pcm = NULL;
    uint64_t samples = 0;
    char err[256] = "";
    if (sr_audio_decode_file(path, 48000, 2, 60.0, &pcm, &samples, err,
                             sizeof(err)) != SR_OK) {
        fprintf(stderr, "  decode %s: %s\n", path, err);
        return -1;
    }
    free(pcm);
    return (int64_t)samples;
}

/* A range render at 30000/1001 hands the encoder exactly S(end) - S(first)
 * samples. Decoded back, the Matroska track has exactly that many (the
 * padded tail of the last AAC frame carries DiscardPadding); libavformat's
 * MP4 demuxer does not trim a track's end, so the MP4 track may keep the
 * rest of the last AAC frame (fewer than 1024 extra samples). */
static void range_render_sample_count(sr_test_ctx *t) {
    FILE *sink = fopen("/dev/null", "w");
    static const char *const names[] = {"audio-range.mkv", "audio-range.mp4"};
    for (int k = 0; k < 2; ++k) {
        SrDiagnostics diag;
        sr_diag_init(&diag, "audio-range", sink ? sink : stderr);
        SrScene scene;
        CHECK_INT(t, sr_scene_load_xml(sr_test_data_path("tests/data-media.xml"),
                                       &scene, &diag), SR_OK);
        scene.project.fps_num = 30000;
        scene.project.fps_den = 1001;
        char path[1024];
        snprintf(path, sizeof(path), "%s", sr_test_tmp_path(names[k]));
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
        int64_t in_file = decoded_samples(path);
        if (k == 0) {
            CHECK_INT(t, in_file, expected);
        } else if (in_file < (int64_t)expected || in_file >= (int64_t)expected + 1024) {
            SR_FAIL(t, "mp4 decodes %lld samples, expected %llu (+ < 1024 tail)",
                    (long long)in_file, (unsigned long long)expected);
        }
        sr_scene_free(&scene);
    }
    if (sink) fclose(sink);
}

/* AAC priming in MP4 is signalled as skip samples; the decoder drops it
 * once (it needs pkt_timebase to move the frame timestamp past it), and
 * the core must not trim it again: a tone starting at 0.5 s decodes at
 * 0.5 s and the clip keeps its whole length. */
static void mp4_priming_trimmed_once(sr_test_ctx *t) {
    FILE *sink = fopen("/dev/null", "w");
    SrDiagnostics diag;
    sr_diag_init(&diag, "priming", sink ? sink : stderr);
    SrScene scene;
    sr_scene_init(&scene);
    scene.project.width = 16;
    scene.project.height = 16;
    scene.project.fps_num = 1;
    scene.project.fps_den = 1;
    char path[1024];
    snprintf(path, sizeof(path), "%s", sr_test_tmp_path("priming.mp4"));
    SrEncoderAudio audio = {48000, 2};
    SrEncoder *e = NULL;
    CHECK_INT(t, sr_encoder_open(&e, &scene, path, 1, &audio, &diag), SR_OK);
    enum { N = 48000, ONSET = 24000 };
    float *pcm = calloc((size_t)N * 2, sizeof(float));
    uint8_t rgba[16 * 16 * 4] = {0};
    if (e && pcm) {
        for (int i = ONSET; i < N; ++i)
            pcm[2 * i] = pcm[2 * i + 1] = (float)(0.5 * sin(2.0 * SR_PI * 1000.0 * i / 48000.0));
        CHECK_INT(t, sr_encoder_write_video(e, rgba, &diag), SR_OK);
        CHECK_INT(t, sr_encoder_write_audio(e, pcm, N, &diag), SR_OK);
        CHECK_INT(t, sr_encoder_finish(e, &diag), SR_OK);
    }
    sr_encoder_destroy(e);
    free(pcm);
    pcm = NULL;
    uint64_t samples = 0;
    CHECK_INT(t, sr_audio_decode_file(path, 48000, 2, 60.0, &pcm, &samples, NULL, 0),
              SR_OK);
    CHECK(t, samples >= N && samples < N + 1024);
    int64_t onset = -1;
    for (uint64_t i = 0; pcm && i < samples && onset < 0; ++i)
        if (fabsf(pcm[2 * i]) > 0.1f) onset = (int64_t)i;
    if (llabs(onset - ONSET) > 64)
        SR_FAIL(t, "tone decoded from sample %lld, expected %d", (long long)onset, ONSET);
    free(pcm);
    sr_scene_free(&scene);
    if (sink) fclose(sink);
}

/* One second of audio, a one-second timestamp gap, one more second: the
 * gap becomes silence and the second part plays at 2 s. */
static void timestamp_gap_is_silence(sr_test_ctx *t) {
    enum { PACKETS = 200, PER = 480 };
    int64_t pts[PACKETS];
    int counts[PACKETS];
    int16_t values[PACKETS];
    for (int k = 0; k < PACKETS; ++k) {
        pts[k] = k < 100 ? 10 * k : 2000 + 10 * (k - 100);   /* ms */
        counts[k] = PER;
        values[k] = k < 100 ? 8000 : -8000;
    }
    SynthMedia m = {.format = "matroska", .audio_tb = {1, 1000},
                    .audio_rate = 48000, .audio_pts = pts,
                    .audio_samples = counts, .audio_value = values,
                    .audio_packets = PACKETS};
    char path[1024];
    snprintf(path, sizeof(path), "%s", sr_test_tmp_path("audio-gap.mkv"));
    CHECK(t, synth_media(path, &m));
    float *pcm = NULL;
    uint64_t samples = 0;
    char err[256] = "";
    CHECK_INT(t, sr_audio_decode_file(path, 48000, 1, 60.0, &pcm, &samples, err,
                                      sizeof(err)), SR_OK);
    CHECK_INT(t, samples, 3 * 48000);
    if (pcm && samples == 3 * 48000) {
        const float high = 8000.0f / 32768.0f;
        CHECK_NEAR(t, pcm[100], high, 1e-4);
        CHECK_NEAR(t, pcm[47999], high, 1e-4);
        CHECK(t, pcm[48000] == 0.0f && pcm[72000] == 0.0f && pcm[95999] == 0.0f);
        CHECK_NEAR(t, pcm[96000 + 100], -high, 1e-4);
        CHECK_NEAR(t, pcm[3 * 48000 - 1], -high, 1e-4);
    }
    free(pcm);
}

/* Seconds -> samples saturates before llround; XML bounds track times. */
static void seconds_to_samples_saturates(sr_test_ctx *t) {
    CHECK(t, sr_audio_seconds_to_samples(2.5, 48000) == 120000);
    CHECK(t, sr_audio_seconds_to_samples(0x1p62, 1) == ((uint64_t)1 << 62));
    CHECK(t, sr_audio_seconds_to_samples(1e19, 1) == UINT64_MAX);
    CHECK(t, sr_audio_seconds_to_samples(1e15, 48000) == UINT64_MAX);
    CHECK(t, sr_audio_seconds_to_samples(0x1p63, 1) == UINT64_MAX);
    CHECK(t, sr_audio_seconds_to_samples(-1.0, 48000) == 0);
    CHECK(t, sr_audio_seconds_to_samples(NAN, 48000) == 0);
    static const char *const attrs[] = {"start", "clipIn", "clipOut", "fadeIn",
                                        "fadeOut"};
    FILE *sink = fopen("/dev/null", "w");
    for (int k = 0; k < 5; ++k) {
        for (int over = 0; over < 2; ++over) {
            char path[1024];
            snprintf(path, sizeof(path), "%s", sr_test_tmp_path("audio-limit.xml"));
            FILE *f = fopen(path, "w");
            if (!f) {
                SR_FAIL(t, "cannot write %s", path);
                continue;
            }
            fprintf(f,
                    "<?xml version=\"1.0\"?>\n<scene version=\"1.0\">\n"
                    "<project width=\"16\" height=\"16\" fps=\"1\" duration=\"1\"/>\n"
                    "<output path=\"out.mp4\" codec=\"h264\"/>\n"
                    "<assets><audio id=\"a\" src=\"a.wav\"/></assets>\n"
                    "<composition/>\n<audioMix>"
                    "<audioTrack id=\"t\" asset=\"a\" %s=\"%s\"/>"
                    "</audioMix>\n</scene>\n",
                    attrs[k], over ? "1.5e7" : "1e7");
            fclose(f);
            SrDiagnostics diag;
            sr_diag_init(&diag, "audio-limit", sink ? sink : stderr);
            SrScene scene;
            SrStatus st = sr_scene_load_xml(path, &scene, &diag);
            if (over) {
                if (st == SR_OK) SR_FAIL(t, "%s=1.5e7 accepted", attrs[k]);
                CHECK(t, diag.errors > 0);
            } else if (st != SR_OK) {
                SR_FAIL(t, "%s=1e7 rejected", attrs[k]);
            }
            if (st == SR_OK) sr_scene_free(&scene);
        }
    }
    if (sink) fclose(sink);
}

const sr_test_case sr_tests_audio[] = {
    {"frame_to_sample_is_exact", frame_to_sample_is_exact},
    {"two_tracks_with_pan_and_fades", two_tracks_with_pan_and_fades},
    {"reverse_and_speed_positions", reverse_and_speed_positions},
    {"range_render_sample_count", range_render_sample_count},
    {"mp4_priming_trimmed_once", mp4_priming_trimmed_once},
    {"timestamp_gap_is_silence", timestamp_gap_is_silence},
    {"seconds_to_samples_saturates", seconds_to_samples_saturates},
    {NULL, NULL},
};
