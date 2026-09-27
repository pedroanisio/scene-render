/* SPDX-License-Identifier: Apache-2.0 */
/* Multiple outputs, codecs, containers and stills (B1-6): loader checks,
 * pass planning, sequence patterns, the GIF palette, the EXR conversion,
 * 1-vs-4-thread byte identity of every added codec, shared passes against
 * outputs rendered alone, slices and resume, stills, and the lossless
 * codecs against the golden references. */
#include "scene_text.h"
#include "scene_render/encoder.h"
#include "scene_render/outputs.h"
#include "scene_render/renderer.h"
#include "../../src/gif_palette_internal.h"
#include "../../src/output_media_internal.h"
#include "../../src/output_plan_internal.h"

#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/imgutils.h>
#include <libswscale/swscale.h>

#include <dirent.h>
#include <errno.h>
#include <inttypes.h>
#include <sys/stat.h>

/* ------------------------------------------------------------ helpers */

static bool read_bytes(const char *path, uint8_t **data, size_t *size) {
    FILE *file = fopen(path, "rb");
    if (!file) return false;
    bool ok = fseek(file, 0, SEEK_END) == 0;
    long length = ok ? ftell(file) : -1;
    ok = ok && length >= 0 && fseek(file, 0, SEEK_SET) == 0;
    uint8_t *buffer = ok ? malloc((size_t)length + 1) : NULL;
    ok = buffer && fread(buffer, 1, (size_t)length, file) == (size_t)length;
    fclose(file);
    if (!ok) {
        free(buffer);
        return false;
    }
    *data = buffer;
    *size = (size_t)length;
    return true;
}

static bool same_file(const char *a, const char *b) {
    uint8_t *x = NULL, *y = NULL;
    size_t xs = 0, ys = 0;
    bool same = read_bytes(a, &x, &xs) && read_bytes(b, &y, &ys) && xs == ys &&
                memcmp(x, y, xs) == 0;
    free(x);
    free(y);
    return same;
}

/* Decodes frame `index` of `path` into `format` (tightly packed planes for
 * planar formats); returns malloc'd pixels, or NULL. */
static uint8_t *decode_frame(const char *path, int index, enum AVPixelFormat format,
                             int *width, int *height, int *frames) {
    AVFormatContext *fmt = NULL;
    if (avformat_open_input(&fmt, path, NULL, NULL) < 0) return NULL;
    uint8_t *out = NULL;
    AVCodecContext *dec = NULL;
    AVPacket *pkt = av_packet_alloc();
    AVFrame *frame = av_frame_alloc();
    int si = avformat_find_stream_info(fmt, NULL) >= 0
        ? av_find_best_stream(fmt, AVMEDIA_TYPE_VIDEO, -1, -1, NULL, 0) : -1;
    const AVCodec *codec = si >= 0
        ? avcodec_find_decoder(fmt->streams[si]->codecpar->codec_id) : NULL;
    dec = codec ? avcodec_alloc_context3(codec) : NULL;
    if (!dec || !pkt || !frame ||
        avcodec_parameters_to_context(dec, fmt->streams[si]->codecpar) < 0 ||
        avcodec_open2(dec, codec, NULL) < 0)
        goto done;
    int seen = 0;
    bool eof = false;
    while (!eof) {
        if (av_read_frame(fmt, pkt) < 0) {
            eof = true;
            avcodec_send_packet(dec, NULL);
        } else {
            if (pkt->stream_index == si) avcodec_send_packet(dec, pkt);
            av_packet_unref(pkt);
        }
        while (avcodec_receive_frame(dec, frame) == 0) {
            if (seen == index && !out) {
                *width = frame->width;
                *height = frame->height;
                int size = av_image_get_buffer_size(format, frame->width,
                                                    frame->height, 1);
                if (frame->format == format && size > 0) {
                    /* Same format: copy exactly (swscale would requantize
                     * floats). */
                    out = malloc((size_t)size);
                    if (out)
                        av_image_copy_to_buffer(out, size,
                            (const uint8_t *const *)frame->data, frame->linesize,
                            format, frame->width, frame->height, 1);
                    ++seen;
                    av_frame_unref(frame);
                    continue;
                }
                struct SwsContext *sws = sws_getContext(
                    frame->width, frame->height, frame->format, frame->width,
                    frame->height, format, SWS_POINT | SWS_BITEXACT |
                    SWS_ACCURATE_RND, NULL, NULL, NULL);
                out = sws && size > 0 ? malloc((size_t)size) : NULL;
                if (out) {
                    uint8_t *planes[4];
                    int strides[4];
                    av_image_fill_arrays(planes, strides, out, format, frame->width,
                                         frame->height, 1);
                    sws_scale(sws, (const uint8_t *const *)frame->data,
                              frame->linesize, 0, frame->height, planes, strides);
                }
                sws_freeContext(sws);
            }
            ++seen;
            av_frame_unref(frame);
        }
    }
    if (frames) *frames = seen;
done:
    av_frame_free(&frame);
    av_packet_free(&pkt);
    avcodec_free_context(&dec);
    avformat_close_input(&fmt);
    return out;
}

static void make_dir(const char *path) {
    if (mkdir(path, 0755) != 0 && errno != EEXIST)
        fprintf(stderr, "cannot create %s\n", path);
}

/* A scratch directory named `name` under the build tree, emptied. */
static const char *scratch_dir(const char *name) {
    static char path[4][1024];
    static int slot;
    slot = (slot + 1) % 4;
    snprintf(path[slot], sizeof(path[slot]), "%s", sr_test_tmp_path(name));
    make_dir(path[slot]);
    DIR *dir = opendir(path[slot]);
    struct dirent *entry;
    while (dir && (entry = readdir(dir))) {
        if (entry->d_name[0] == '.') continue;
        char file[1400];
        snprintf(file, sizeof(file), "%s/%s", path[slot], entry->d_name);
        remove(file);
    }
    if (dir) closedir(dir);
    return path[slot];
}

static char *log_text(FILE *log) {
    if (!log) return NULL;
    fflush(log);
    long size = ftell(log);
    rewind(log);
    char *text = calloc(1, (size_t)(size > 0 ? size : 0) + 1);
    if (text && size > 0 && fread(text, 1, (size_t)size, log) == 0) text[0] = '\0';
    fclose(log);
    return text;
}

#define BEGIN "<scene version=\"1.1\"><project width=\"32\" height=\"18\" " \
              "fps=\"6\" duration=\"1\" background=\"#204060\"/>"
#define BODY "<composition><shape id=\"s\" shape=\"ellipse\" x=\"2\" y=\"2\" " \
             "width=\"12\" height=\"12\" fill=\"#F0A030\"><animate " \
             "property=\"position.x\"><key time=\"0\" value=\"0\"/><key " \
             "time=\"1\" value=\"18\"/></animate></shape></composition></scene>"

static void expect_load(sr_test_ctx *t, const char *xml, SrStatus expected,
                        const char *diagnostic) {
    SrScene scene;
    char *message = NULL;
    SrStatus status = st_load(t, "outputs-load.xml", xml, &scene, &message);
    if (status != expected)
        SR_FAIL(t, "status %d, expected %d for %s\n%s", (int)status, (int)expected,
                xml, message ? message : "");
    if (diagnostic) CHECK_CONTAINS(t, message, diagnostic);
    if (status == SR_OK) sr_scene_free(&scene);
    free(message);
}

/* ------------------------------------------------------ pure helpers */

static void sequence_patterns(sr_test_ctx *t) {
    char name[64];
    CHECK(t, sr_sequence_pattern_valid("f-%d.png"));
    CHECK(t, sr_sequence_pattern_valid("dir/f-%04d.png"));
    CHECK(t, sr_sequence_pattern_valid("100%%/f%09d"));
    CHECK(t, !sr_sequence_pattern_valid("f.png"));
    CHECK(t, !sr_sequence_pattern_valid("f-%d-%d.png"));
    CHECK(t, !sr_sequence_pattern_valid("f-%s.png"));
    CHECK(t, !sr_sequence_pattern_valid("f-%00d.png"));
    CHECK(t, !sr_sequence_pattern_valid("f-%010d.png"));
    CHECK(t, !sr_sequence_pattern_valid("f-%x.png"));
    CHECK(t, !sr_sequence_pattern_valid("d%d/f.png"));
    CHECK(t, !sr_sequence_pattern_valid("f-%"));
    CHECK(t, !sr_sequence_pattern_valid(""));
    CHECK(t, sr_sequence_format("f-%04d.png", 12, name, sizeof(name)));
    CHECK_STR(t, name, "f-0012.png");
    CHECK(t, sr_sequence_format("100%%-%d", 123456, name, sizeof(name)));
    CHECK_STR(t, name, "100%-123456");
    CHECK(t, sr_sequence_format("%02d", 12345, name, sizeof(name)));
    CHECK_STR(t, name, "12345");
    CHECK(t, !sr_sequence_format("%d", 1000000000, name, sizeof(name)));
    CHECK(t, !sr_sequence_format("long-%d", 1, name, 6));
    /* Collisions: equal prefix/suffix, or a plain name the pattern makes. */
    CHECK(t, sr_output_paths_collide("f-%d.png", true, "f-%04d.png", true));
    CHECK(t, !sr_output_paths_collide("f-%d.png", true, "g-%d.png", true));
    CHECK(t, !sr_output_paths_collide("f-%d.png", true, "f-%d.tif", true));
    CHECK(t, sr_output_paths_collide("f-%03d.png", true, "f-7.png", false));
    CHECK(t, !sr_output_paths_collide("f-%03d.png", true, "f-x.png", false));
    CHECK(t, !sr_output_paths_collide("f-%03d.png", true, "f-.png", false));
    CHECK(t, sr_output_paths_collide("f-%01d.png", true, "f-1%01d.png", true));
    CHECK(t, sr_output_paths_collide("f-%01d.png", true, "f-%01d0.png", true));
    CHECK(t, !sr_output_paths_collide("f-%01d.png", true, "fx%01d.png", true));
    CHECK(t, !sr_output_paths_collide("f-%01d.png", true, "f-1%01dx.png", true));
    CHECK(t, sr_output_paths_collide("a.mp4", false, "a.mp4", false));
    CHECK(t, !sr_output_paths_collide("a.mp4", false, "b.mp4", false));
}

/* F(t) = ceil(t * n / d - 1e-12) against exact integer arithmetic for
 * times that are whole multiples of 1/1000 s. */
static void frame_ranges_closed_form(sr_test_ctx *t) {
    static const uint32_t rates[][2] = {{24, 1}, {30000, 1001}, {12, 1}, {1, 1},
                                        {1000, 1}, {25, 2}};
    for (size_t r = 0; r < sizeof(rates) / sizeof(rates[0]); ++r) {
        for (uint64_t ms = 0; ms <= 5000; ms += 7) {
            uint64_t num = ms * rates[r][0], den = 1000u * rates[r][1];
            uint64_t expected = (num + den - 1) / den;
            uint64_t got = sr_output_frame_at((double)ms / 1000.0, rates[r][0],
                                              rates[r][1]);
            if (got != expected) {
                SR_FAIL(t, "F(%" PRIu64 " ms at %u/%u) = %" PRIu64 ", expected %"
                        PRIu64, ms, rates[r][0], rates[r][1], got, expected);
                return;
            }
        }
    }
    CHECK_INT(t, sr_output_total_frames(2.0, 12, 1), 24);
    CHECK_INT(t, sr_output_total_frames(1.0, 30000, 1001), 30);
    CHECK_INT(t, sr_output_frame_at(-1.0, 24, 1), 0);
}

static void codec_table_and_muxers(sr_test_ctx *t) {
    SrOutput o;
    if (sr_output_init(&o) != SR_OK) {
        SR_FAIL(t, "init");
        return;
    }
    const char *why = NULL;
    CHECK_STR(t, sr_output_muxer(&o, "a.mp4", &why), "mp4");
    CHECK_STR(t, sr_output_muxer(&o, "a.MOV", &why), "mov");
    CHECK_STR(t, sr_output_muxer(&o, "a.mkv", &why), "matroska");
    CHECK(t, !sr_output_muxer(&o, "a.webm", &why) && why);
    CHECK(t, !sr_output_muxer(&o, "dir.mp4/a", &why));
    o.codec = SR_CODEC_VP9;
    CHECK_STR(t, sr_output_muxer(&o, "a.bin", &why), "webm");
    CHECK_STR(t, sr_output_muxer(&o, "a.mkv", &why), "matroska");
    CHECK(t, !sr_output_muxer(&o, "a.mov", &why));
    o.codec = SR_CODEC_PRORES;
    CHECK_STR(t, sr_output_muxer(&o, "a", &why), "mov");
    o.container = SR_CONTAINER_MKV;
    CHECK_STR(t, sr_output_muxer(&o, "a.mov", &why), "matroska");
    o.container = SR_CONTAINER_MP4;
    CHECK(t, !sr_output_muxer(&o, "a.mov", &why));
    o.container = SR_CONTAINER_AUTO;
    o.codec = SR_CODEC_GIF;
    CHECK_STR(t, sr_output_muxer(&o, "a.mp4", &why), "gif");
    o.codec = SR_CODEC_PNG_SEQUENCE;
    CHECK(t, !sr_output_muxer(&o, "a-%d.png", &why));
    CHECK_INT(t, sr_output_input_bits(&o), 8);
    o.codec = SR_CODEC_EXR_SEQUENCE;
    CHECK_INT(t, sr_output_input_bits(&o), 32);
    SrCodec codec;
    CHECK(t, sr_codec_parse("tiff-sequence", &codec) && codec == SR_CODEC_TIFF_SEQUENCE);
    CHECK(t, !sr_codec_parse("dnxhr", &codec));
    CHECK_INT(t, sr_output_preset_level(SR_CODEC_AV1, "medium"), 7);
    CHECK_INT(t, sr_output_preset_level(SR_CODEC_VP9, "veryslow"), 0);
    CHECK_INT(t, sr_output_preset_level(SR_CODEC_H264, "medium"), -1);
    CHECK_INT(t, sr_output_preset_level(SR_CODEC_VP9, "turbo"), -1);
    CHECK(t, sr_encoder_available(sr_codec_info(SR_CODEC_AV1)->encoder));
    CHECK(t, sr_encoder_available(sr_codec_info(SR_CODEC_VP9)->encoder));
    CHECK(t, sr_encoder_available(sr_codec_info(SR_CODEC_PRORES)->encoder));
    sr_output_free(&o);
}

/* ------------------------------------------------------------- loader */

static void loader_reads_fixture(sr_test_ctx *t) {
    SrScene scene;
    FILE *log;
    SrDiagnostics diag;
    st_diag(&diag, &log);
    SrStatus status = sr_scene_load_xml(sr_test_data_path("tests/data-outputs.xml"),
                                       &scene, &diag);
    if (log) fclose(log);
    CHECK_INT(t, status, SR_OK);
    if (status != SR_OK) return;
    CHECK_INT(t, sr_scene_output_count(&scene), 9);
    const SrOutput *main_output = sr_scene_output_const(&scene, 0);
    CHECK_STR(t, main_output->id, "main");
    CHECK_INT(t, main_output->still_count, 2);
    CHECK_INT(t, main_output->b_frames, 0);
    CHECK(t, main_output->faststart_authored && !main_output->faststart);
    CHECK_NEAR(t, main_output->keyframe_interval, 1.0, 0.0);
    CHECK_INT(t, main_output->stills[1].width, 48);
    CHECK(t, main_output->stills[1].format == SR_STILL_JPEG);
    const SrOutput *master = sr_scene_output_const(&scene, 1);
    CHECK_STR(t, master->pixel_format, "yuv422p10le");
    CHECK(t, master->prores_profile == SR_PRORES_HQ);
    const SrOutput *web = sr_scene_output_const(&scene, 2);
    CHECK_STR(t, web->audio_codec, "libopus");
    CHECK_NEAR(t, web->keyframe_interval, 2.0, 0.0);
    CHECK(t, web->has_end && web->end == 1.5 && web->start == 0.5);
    CHECK_STR(t, sr_scene_output_const(&scene, 4)->pixel_format, "pal8");
    CHECK_STR(t, sr_scene_output_const(&scene, 8)->pixel_format, "gbrpf32le");
    CHECK(t, !sr_output_is_legacy(main_output));
    sr_scene_free(&scene);
    /* A 1.0 output stays on every 1.0 path. */
    expect_load(t, "<scene version=\"1.0\"><project width=\"8\" height=\"8\" "
                "fps=\"1\" duration=\"1\"/><output path=\"a.avi\" codec=\"h264\" "
                "crf=\"51\"/><composition/></scene>", SR_OK, NULL);
    SrScene legacy;
    if (st_load(t, "outputs-legacy.xml", BEGIN "<output path=\"a.mp4\" codec=\"ffv1\" "
                "crf=\"3\" preset=\"slow\"/>" BODY, &legacy, NULL) == SR_OK) {
        CHECK(t, sr_output_is_legacy(&legacy.output));
        sr_scene_free(&legacy);
    }
}

static void loader_rejections(sr_test_ctx *t) {
    static const struct { const char *outputs, *message; } cases[] = {
        {"<output path=\"a.mp4\" codec=\"h264\"/><output path=\"b.mp4\" "
         "codec=\"h264\"/>", "needs an id"},
        {"<output id=\"a\" path=\"a.mp4\" codec=\"h264\"/><output id=\"a2\" "
         "path=\"a.mp4\" codec=\"h264\"/>", "can overwrite"},
        {"<output id=\"a\" path=\"f-%01d.png\" codec=\"png-sequence\"/><output "
         "id=\"b\" path=\"f-%04d.png\" codec=\"png-sequence\"/>", "can overwrite"},
        {"<output id=\"a\" path=\"f-%01d.png\" codec=\"png-sequence\"><poster "
         "path=\"f-3.png\"/></output>", "can overwrite"},
        {"<output path=\"a.mp4\" codec=\"h264\" start=\"1\"/>", "before the project"},
        {"<output path=\"a.mp4\" codec=\"h264\" start=\"0.5\" end=\"0.5\"/>",
         "start < end"},
        {"<output path=\"a.mp4\" codec=\"h264\" end=\"2\"/>", "start < end"},
        {"<output path=\"a.mov\" codec=\"prores\" crf=\"20\"/>", "crf does not apply"},
        {"<output path=\"a.gif\" codec=\"gif\" preset=\"slow\"/>", "preset does not"},
        {"<output path=\"a.png\" codec=\"apng\" bitrate=\"100\"/>", "bitrate does not"},
        {"<output path=\"a.mp4\" codec=\"h264\" loopCount=\"2\"/>", "loopCount applies"},
        {"<output path=\"a.webm\" codec=\"vp9\" bFrames=\"2\"/>", "bFrames applies"},
        {"<output path=\"a.mkv\" codec=\"h264\" faststart=\"true\"/>",
         "faststart applies"},
        {"<output path=\"a.gif\" codec=\"gif\" container=\"mp4\"/>",
         "container does not apply"},
        {"<output path=\"f.png\" codec=\"png-sequence\"/>", "exactly one %d"},
        {"<output path=\"a.mov\" codec=\"prores\" keyframeInterval=\"1\"/>",
         "keyframeInterval does not apply"},
        {"<output path=\"a.mp4\" codec=\"h264\" proresProfile=\"hq\"/>",
         "proresProfile applies"},
        {"<output path=\"a.mov\" codec=\"prores\" proresProfile=\"4444\" "
         "pixelFormat=\"yuv422p10le\"/>", "needs pixelFormat"},
        {"<output path=\"a.mp4\" codec=\"av1\" crf=\"0\"/>", "av1 crf"},
        {"<output path=\"a.webm\" codec=\"vp9\" preset=\"turbo\"/>", "expected ultrafast"},
        {"<output path=\"a.gif\" codec=\"gif\" pixelFormat=\"rgb24\"/>", "pal8"},
        {"<output path=\"f-%02d.exr\" codec=\"exr-sequence\" pixelFormat=\"rgb24\"/>",
         "gbrpf32le"},
        {"<output path=\"a.mp4\" codec=\"h264\" fps=\"1/2\"/>", "between 1 and 1000"},
        {"<output path=\"a.mp4\" codec=\"h264\" fps=\"1001\"/>", "between 1 and 1000"},
        {"<output path=\"a.mp4\" codec=\"h264\" width=\"16385\"/>", "at most 16384"},
        {"<output path=\"a.mp4\" codec=\"h264\" crf=\"52\"/>", "[0,51]"},
        {"<output path=\"a.webm\" codec=\"vp9\" crf=\"64\"/>", "[0,63]"},
        {"<output path=\"a.mp4\" codec=\"h264\" start=\"0.5\"><poster path=\"p.png\" "
         "time=\"0.25\"/></output>", "inside its output's range"},
        {"<output path=\"a.mp4\" codec=\"h264\"><poster path=\"p.png\" "
         "quality=\"2\"/></output>", "maximum value"},
        {"<output path=\"a.mp4\" codec=\"h264\"><poster path=\"p.png\" "
         "format=\"webp\"/></output>", "unsupported in this build"},
        {"<output path=\"a.mp4\" codec=\"h264\"><poster path=\"p.png\" "
         "marker=\"m\"/></output>", "unknown marker id 'm'"},
        {"<output path=\"a.mp4\" codec=\"h264\" container=\"mxf\"/>",
         "unsupported in this build"},
        {"<output path=\"a.mp4\" codec=\"dnxhr\"/>", "unsupported in this build"},
        {"<output path=\"a.mp4\" codec=\"h264\" alpha=\"true\"/>",
         "unsupported in this build"},
        {"<output path=\"a.mp4\" codec=\"h264\" loopCount=\"65536\"/>", "65535"},
    };
    char xml[4096];
    for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); ++i) {
        snprintf(xml, sizeof(xml), BEGIN "%s" BODY, cases[i].outputs);
        expect_load(t, xml, SR_ERR_XML, cases[i].message);
    }
    /* 1.0 documents keep exactly one output. */
    expect_load(t, "<scene version=\"1.0\"><project width=\"8\" height=\"8\" "
                "fps=\"1\" duration=\"1\"/><output path=\"a.mp4\" codec=\"h264\"/>"
                "<output path=\"b.mp4\" codec=\"h264\"/><composition/></scene>",
                SR_ERR_XML, "requires version=\"1.1\"");
    /* SR_MAX_OUTPUTS and SR_MAX_OUTPUT_STILLS. */
    char many[8192] = BEGIN;
    for (unsigned i = 0; i <= SR_MAX_OUTPUTS; ++i) {
        char one[160];
        snprintf(one, sizeof(one), "<output id=\"o%u\" path=\"o%u.mp4\" "
                 "codec=\"h264\"/>", i, i);
        strcat(many, one);
    }
    strcat(many, BODY);
    expect_load(t, many, SR_ERR_XML, "SR_MAX_OUTPUTS");
    char stills[8192] = BEGIN "<output path=\"a.mp4\" codec=\"h264\">";
    for (unsigned i = 0; i <= SR_MAX_OUTPUT_STILLS; ++i) {
        char one[96];
        snprintf(one, sizeof(one), "<poster path=\"p%u.png\"/>", i);
        strcat(stills, one);
    }
    strcat(stills, "</output>" BODY);
    expect_load(t, stills, SR_ERR_XML, "SR_MAX_OUTPUT_STILLS");
    expect_load(t, "<scene version=\"1.1\"><project width=\"64\" height=\"32\" "
                "fps=\"1\" duration=\"1\" mode=\"equirectangular\"/><output "
                "path=\"a.mp4\" codec=\"h264\" width=\"32\" height=\"16\"/>"
                "<scene360 width=\"64\" height=\"32\"/><composition/></scene>",
                SR_ERR_XML, "equirectangular");
}

/* Seeded mutations of the output/still attribute set: every document is
 * either accepted or rejected with a diagnostic, never a crash, and
 * accepted documents plan cleanly. */
static void fuzz_output_parser(sr_test_ctx *t) {
    static const char *const attributes[] = {
        "id=\"x\"", "container=\"mkv\"", "container=\"webm\"", "width=\"16\"",
        "height=\"0\"", "fps=\"3\"", "fps=\"0\"", "start=\"0.25\"", "end=\"0.75\"",
        "end=\"-1\"", "keyframeInterval=\"0.5\"", "keyframeInterval=\"0\"",
        "bFrames=\"1\"", "bFrames=\"17\"", "faststart=\"false\"",
        "faststart=\"maybe\"", "loopCount=\"3\"", "proresProfile=\"lt\"",
        "crf=\"30\"", "crf=\"-1\"", "preset=\"fast\"", "pixelFormat=\"rgb24\"",
        "bitrate=\"100000\"", "audioCodec=\"aac\""};
    static const char *const codecs[] = {"h264", "h265", "ffv1", "prores", "vp9",
                                         "av1", "gif", "apng", "png-sequence",
                                         "tiff-sequence", "exr-sequence"};
    static const char *const paths[] = {"a.mp4", "a.mkv", "a.webm", "f-%03d.x",
                                        "a", "f%d%d"};
    uint64_t state = 0x6B1D6ULL;
    unsigned accepted = 0;
    for (int round = 0; round < 216; ++round) {
        char xml[4096], attrs[1024] = "";
        state = state * 6364136223846793005ULL + 1442695040888963407ULL;
        size_t codec = (state >> 33) % (sizeof(codecs) / sizeof(codecs[0]));
        size_t path = (state >> 17) % (sizeof(paths) / sizeof(paths[0]));
        for (size_t k = 0; k < sizeof(attributes) / sizeof(attributes[0]); ++k) {
            state = state * 6364136223846793005ULL + 1442695040888963407ULL;
            if ((state >> 40) % 12 == 0) {
                strcat(attrs, " ");
                strcat(attrs, attributes[k]);
            }
        }
        const char *still = (state >> 20) % 3 == 0
            ? "<thumbnail path=\"t.jpg\" width=\"9\" time=\"0.5\"/>" : "";
        snprintf(xml, sizeof(xml), BEGIN "<output path=\"%s\" codec=\"%s\"%s>%s"
                 "</output>" BODY, paths[path], codecs[codec], attrs, still);
        /* Truncation at a seeded offset must fail as XML, never crash. */
        if (round % 9 == 8) xml[strlen(xml) * ((state >> 8) % 100) / 100] = '\0';
        SrScene scene;
        char *message = NULL;
        SrStatus status = st_load(t, "outputs-fuzz.xml", xml, &scene, &message);
        if (status == SR_OK) {
            ++accepted;
            SrOutputPlan plan;
            SrRenderOptions options = {.validate_only = true};
            FILE *log;
            SrDiagnostics diag;
            st_diag(&diag, &log);
            SrStatus planned = sr_output_plan_build(&plan, &scene, &options, &diag);
            CHECK(t, planned == SR_OK || planned == SR_ERR_ARGUMENT);
            sr_output_plan_free(&plan);
            if (log) fclose(log);
            sr_scene_free(&scene);
        } else if (status != SR_ERR_XML || !message || !strstr(message, "error")) {
            SR_FAIL(t, "round %d: status %d without a diagnostic:\n%s", round,
                    (int)status, xml);
        }
        free(message);
    }
    CHECK(t, accepted > 20);
}

/* --------------------------------------------------------------- plan */

static bool load_fixture(sr_test_ctx *t, SrScene *scene) {
    FILE *log;
    SrDiagnostics diag;
    st_diag(&diag, &log);
    SrStatus status = sr_scene_load_xml(sr_test_data_path("tests/data-outputs.xml"),
                                       scene, &diag);
    if (log) fclose(log);
    CHECK_INT(t, status, SR_OK);
    return status == SR_OK;
}

static SrStatus plan_with(const SrScene *scene, SrRenderOptions options,
                          SrOutputPlan *plan, char **message) {
    FILE *log;
    SrDiagnostics diag;
    st_diag(&diag, &log);
    SrStatus status = sr_output_plan_build(plan, scene, &options, &diag);
    char *text = log_text(log);
    if (message) *message = text;
    else free(text);
    return status;
}

static void plan_groups_passes(sr_test_ctx *t) {
    SrScene scene;
    if (!load_fixture(t, &scene)) return;
    SrOutputPlan plan;
    SrStatus status = plan_with(&scene, (SrRenderOptions){0}, &plan, NULL);
    CHECK_INT(t, status, SR_OK);
    /* 96x54@12 (main, master, web, av1, anim, png, tiff + poster),
     * 48x27@6 (loop), 48x27@12 (thumbnail), 96x54@4 (exr). */
    CHECK_INT(t, plan.pass_count, 4);
    if (status == SR_OK && plan.pass_count == 4) {
        const SrPlanPass *shared = &plan.passes[0];
        CHECK_INT(t, shared->width, 96);
        CHECK_INT(t, shared->sink_count, 7);
        CHECK_INT(t, shared->still_count, 1);
        CHECK_INT(t, shared->total_frames, 24);
        CHECK_STR(t, shared->sinks[2].output->id, "web");
        CHECK_INT(t, shared->sinks[2].first, 6);
        CHECK_INT(t, shared->sinks[2].end, 18);
        CHECK_INT(t, shared->sinks[4].end, 12);     /* anim ends at 1 s */
        CHECK_INT(t, shared->sinks[6].first, 12);   /* tiff starts at 1 s */
        CHECK_INT(t, shared->stills[0].frame, 6);
        CHECK_INT(t, plan.passes[1].still_count, 1);    /* thumbnail */
        CHECK_INT(t, plan.passes[1].sink_count, 0);
        CHECK_INT(t, plan.passes[1].height, 27);
        CHECK_INT(t, plan.passes[1].fps_num, 12);
        CHECK_INT(t, plan.passes[2].fps_num, 6);         /* gif */
        CHECK_INT(t, plan.passes[2].total_frames, 12);
        CHECK_INT(t, plan.passes[3].fps_num, 4);
        CHECK_INT(t, plan.passes[3].total_frames, 8);
    }
    sr_output_plan_free(&plan);
    /* --frame-range applies per output, in its own frames. */
    SrRenderOptions sliced = {.has_range = true, .first_frame = 3, .end_frame = 7};
    CHECK_INT(t, plan_with(&scene, sliced, &plan, NULL), SR_OK);
    if (plan.pass_count >= 2) {
        CHECK_INT(t, plan.passes[0].sinks[2].first, 6);
        CHECK_INT(t, plan.passes[0].sinks[2].end, 7);
        CHECK(t, plan.passes[0].sinks[6].first >= plan.passes[0].sinks[6].end);
        CHECK_INT(t, plan.passes[0].still_count, 1);  /* poster frame 6 */
        CHECK_INT(t, plan.passes[2].sinks[0].end, 7);
    }
    sr_output_plan_free(&plan);
    /* --resume: one pass per output. */
    SrRenderOptions resume = {.resume = true, .output_ids = "main,master,web"};
    CHECK_INT(t, plan_with(&scene, resume, &plan, NULL), SR_OK);
    size_t with_sinks = 0;
    for (size_t p = 0; p < plan.pass_count; ++p) {
        CHECK(t, plan.passes[p].sink_count <= 1);
        with_sinks += plan.passes[p].sink_count;
    }
    CHECK_INT(t, with_sinks, 3);
    sr_output_plan_free(&plan);
    sr_scene_free(&scene);
}

static void plan_cli_rules(sr_test_ctx *t) {
    SrScene scene;
    if (!load_fixture(t, &scene)) return;
    static const struct {
        SrRenderOptions options;
        SrStatus status;
        const char *message;
    } cases[] = {
        {{.output_ids = "nope"}, SR_ERR_ARGUMENT, "no output has id 'nope'"},
        {{.output_ids = "main,main"}, SR_ERR_ARGUMENT, "listed twice"},
        {{.output_override = "x.mp4"}, SR_ERR_ARGUMENT, "exactly one selected"},
        {{.output_ids = "main,web", .output_override = "x.mp4"}, SR_ERR_ARGUMENT,
         "exactly one selected"},
        {{.output_ids = "main", .output_override = "x.mp4"}, SR_OK, NULL},
        {{.preview = true, .output_ids = "main,web"}, SR_ERR_ARGUMENT,
         "renders one output"},
        {{.hash = true, .output_ids = "web"}, SR_OK, NULL},
        {{.resume = true, .output_ids = "loop"}, SR_ERR_ARGUMENT, "cannot resume"},
        {{.resume = true, .output_ids = "png"}, SR_ERR_ARGUMENT, "cannot resume"},
        {{.output_ids = "main", .output_override = "../build/test-artifacts/outputs"
                                                    "/poster.png"},
         SR_OK, NULL},
    };
    for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); ++i) {
        SrOutputPlan plan;
        char *message = NULL;
        SrStatus status = plan_with(&scene, cases[i].options, &plan, &message);
        if (status != cases[i].status)
            SR_FAIL(t, "case %zu: status %d\n%s", i, (int)status, message);
        if (cases[i].message) CHECK_CONTAINS(t, message, cases[i].message);
        free(message);
        sr_output_plan_free(&plan);
    }
    /* The override collides with the poster once both are joined. */
    char override[1024];
    snprintf(override, sizeof(override), "%s", sr_test_data_path(
        "tests/../build/test-artifacts/outputs/poster.png"));
    SrOutputPlan plan;
    char *message = NULL;
    SrRenderOptions collide = {.output_ids = "main", .output_override = override};
    CHECK_INT(t, plan_with(&scene, collide, &plan, &message), SR_ERR_ARGUMENT);
    CHECK_CONTAINS(t, message, "same file");
    free(message);
    sr_output_plan_free(&plan);
    /* The hash of one output is the hash of its pass. */
    SrRenderOptions hash = {.hash = true, .output_ids = "loop"};
    CHECK_INT(t, plan_with(&scene, hash, &plan, NULL), SR_OK);
    CHECK(t, plan.pass_count == 1 && plan.passes[0].width == 48 &&
             plan.passes[0].still_count == 0);
    sr_output_plan_free(&plan);
    sr_scene_free(&scene);
}

/* ------------------------------------------------------------ palette */

static void reference_quantize(const uint8_t *rgba, size_t pixels,
                               uint32_t palette[256], unsigned *used,
                               uint8_t *indices);

static void gif_palette_exact_and_median_cut(sr_test_ctx *t) {
    enum { W = 37, H = 23 };
    uint8_t rgba[W * H * 4], indices[W * H], expected[W * H];
    uint32_t palette[256], reference[256];
    /* At most 256 colours: exact and lossless. */
    for (size_t i = 0; i < W * H; ++i) {
        uint32_t c = (uint32_t)(i * 2654435761u) % 200u;
        rgba[i * 4] = (uint8_t)(c * 3);
        rgba[i * 4 + 1] = (uint8_t)(255 - c);
        rgba[i * 4 + 2] = (uint8_t)(c * 7);
        rgba[i * 4 + 3] = 255;
    }
    SrGifPalette *p = NULL;
    CHECK_INT(t, sr_gif_palette_create(&p, W, H), SR_OK);
    if (!p) return;
    unsigned used = sr_gif_palette_quantize(p, rgba, indices, W, palette);
    CHECK(t, used <= 200 && used > 100);
    for (size_t i = 0; i < W * H; ++i) {
        uint32_t c = palette[indices[i]];
        if ((c >> 16 & 255) != rgba[i * 4] || (c >> 8 & 255) != rgba[i * 4 + 1] ||
            (c & 255) != rgba[i * 4 + 2]) {
            SR_FAIL(t, "pixel %zu not exact", i);
            break;
        }
    }
    /* More colours: median cut equals the independent reference. */
    for (size_t i = 0; i < W * H; ++i) {
        rgba[i * 4] = (uint8_t)(i * 7);
        rgba[i * 4 + 1] = (uint8_t)(i * 13 / 3);
        rgba[i * 4 + 2] = (uint8_t)((i * i) >> 3);
    }
    used = sr_gif_palette_quantize(p, rgba, indices, W, palette);
    unsigned reference_used = 0;
    reference_quantize(rgba, W * H, reference, &reference_used, expected);
    CHECK_INT(t, used, reference_used);
    CHECK(t, memcmp(palette, reference, sizeof(palette)) == 0);
    CHECK(t, memcmp(indices, expected, sizeof(indices)) == 0);
    /* A single colour and a strided destination. */
    memset(rgba, 9, sizeof(rgba));
    uint8_t wide[H * (W + 5)];
    memset(wide, 0xEE, sizeof(wide));
    used = sr_gif_palette_quantize(p, rgba, wide, W + 5, palette);
    CHECK_INT(t, used, 1);
    CHECK_INT(t, wide[W + 5], 0);
    CHECK_INT(t, wide[W + 1], 0xEE);
    CHECK_INT(t, palette[0], 0xFF090909u);
    CHECK_INT(t, palette[1], 0xFF000000u);
    sr_gif_palette_free(p);
    CHECK_INT(t, sr_gif_palette_create(&p, 0, 4), SR_ERR_ARGUMENT);
}

/* Independent median cut: qsort over explicit colour records, the same
 * documented rules (widest box, widest channel r<g<b, weighted median,
 * count-weighted mean rounded half up). */
typedef struct { uint32_t key, count; } Rec;
static int ref_channel;
static int ref_compare(const void *a, const void *b) {
    const Rec *x = a, *y = b;
    int cx = (int)(x->key >> (16 - 8 * ref_channel) & 255);
    int cy = (int)(y->key >> (16 - 8 * ref_channel) & 255);
    if (cx != cy) return cx - cy;
    return x->key < y->key ? -1 : x->key > y->key;
}
static int key_compare(const void *a, const void *b) {
    uint32_t x = *(const uint32_t *)a, y = *(const uint32_t *)b;
    return x < y ? -1 : x > y;
}

static void reference_quantize(const uint8_t *rgba, size_t pixels,
                               uint32_t palette[256], unsigned *used,
                               uint8_t *indices) {
    uint32_t *keys = malloc(pixels * sizeof(*keys));
    Rec *recs = malloc(pixels * sizeof(*recs));
    size_t *box_lo = malloc(256 * sizeof(size_t)), *box_hi = malloc(256 * sizeof(size_t));
    for (size_t i = 0; i < pixels; ++i)
        keys[i] = (uint32_t)rgba[i * 4] << 16 | (uint32_t)rgba[i * 4 + 1] << 8 |
                  rgba[i * 4 + 2];
    qsort(keys, pixels, sizeof(*keys), key_compare);
    size_t n = 0;
    for (size_t i = 0; i < pixels; ++i) {
        if (n && recs[n - 1].key == keys[i]) ++recs[n - 1].count;
        else recs[n++] = (Rec){keys[i], 1};
    }
    unsigned boxes = 1;
    box_lo[0] = 0;
    box_hi[0] = n;
    for (;;) {
        if (boxes == 256) break;
        int best = -1, best_range = -1, best_channel = 0;
        for (unsigned b = 0; b < boxes; ++b) {
            if (box_hi[b] - box_lo[b] < 2) continue;
            int range = -1, channel = 0;
            for (int c = 0; c < 3; ++c) {
                int lo = 255, hi = 0;
                for (size_t i = box_lo[b]; i < box_hi[b]; ++i) {
                    int v = (int)(recs[i].key >> (16 - 8 * c) & 255);
                    if (v < lo) lo = v;
                    if (v > hi) hi = v;
                }
                if (hi - lo > range) {
                    range = hi - lo;
                    channel = c;
                }
            }
            if (range > best_range) {
                best = (int)b;
                best_range = range;
                best_channel = channel;
            }
        }
        if (best < 0 || best_range == 0) break;
        ref_channel = best_channel;
        qsort(recs + box_lo[best], box_hi[best] - box_lo[best], sizeof(Rec),
              ref_compare);
        uint64_t weight = 0, sum = 0;
        for (size_t i = box_lo[best]; i < box_hi[best]; ++i) weight += recs[i].count;
        size_t split = box_lo[best] + 1;
        for (size_t i = box_lo[best]; i + 1 < box_hi[best]; ++i) {
            sum += recs[i].count;
            split = i + 1;
            if (sum >= (weight + 1) / 2) break;
        }
        box_lo[boxes] = split;
        box_hi[boxes] = box_hi[best];
        box_hi[best] = split;
        ++boxes;
    }
    for (unsigned b = 0; b < 256; ++b) palette[b] = 0xFF000000u;
    if (n <= 256) {
        for (size_t i = 0; i < n; ++i) palette[i] = 0xFF000000u | recs[i].key;
        boxes = (unsigned)n;
    } else {
        for (unsigned b = 0; b < boxes; ++b) {
            uint64_t s[3] = {0, 0, 0}, w = 0;
            for (size_t i = box_lo[b]; i < box_hi[b]; ++i) {
                w += recs[i].count;
                for (int c = 0; c < 3; ++c)
                    s[c] += recs[i].count * (recs[i].key >> (16 - 8 * c) & 255);
            }
            palette[b] = 0xFF000000u | (uint32_t)((s[0] + w / 2) / w) << 16 |
                         (uint32_t)((s[1] + w / 2) / w) << 8 |
                         (uint32_t)((s[2] + w / 2) / w);
        }
    }
    for (size_t i = 0; i < pixels; ++i) {
        uint32_t key = (uint32_t)rgba[i * 4] << 16 | (uint32_t)rgba[i * 4 + 1] << 8 |
                       rgba[i * 4 + 2];
        for (unsigned b = 0; b < boxes; ++b) {
            size_t lo = n <= 256 ? b : box_lo[b], hi = n <= 256 ? b + 1 : box_hi[b];
            for (size_t k = lo; k < hi; ++k)
                if (recs[k].key == key) indices[i] = (uint8_t)b;
        }
    }
    *used = boxes;
    free(keys);
    free(recs);
    free(box_lo);
    free(box_hi);
}

/* ---------------------------------------------------------------- EXR */

static void exr_linear_conversion(sr_test_ctx *t) {
    for (int space = 0; space < 2; ++space) {
        SrColorSpace s = space ? SR_COLOR_REC709 : SR_COLOR_SRGB;
        for (int i = 0; i <= 100; ++i) {
            double v = i / 100.0;
            CHECK_NEAR(t, sr_output_decode_extended(v, s), sr_color_decode(v, s),
                       1e-15);
            CHECK_NEAR(t, sr_output_decode_extended(-v, s), -sr_color_decode(v, s),
                       1e-15);
        }
        CHECK(t, sr_output_decode_extended(1.5, s) > 1.0);
    }
    SrProject project = {.working_color_space = SR_COLOR_SRGB, .linear_light = false};
    SrFrame frame = {0};
    CHECK_INT(t, sr_frame_init(&frame, 3, 1), SR_OK);
    if (!frame.px) return;
    const float px[12] = {0.25f, 0.5f, 0.75f, 1.0f, 0.3f, 0.2f, 0.1f, 0.5f,
                          0.0f, 0.0f, 0.0f, 0.0f};
    memcpy(frame.px, px, sizeof(px));
    SrColorOutput color;
    CHECK_INT(t, sr_color_output_init_bits(&color, &project, SR_COLOR_SRGB, 16),
              SR_OK);
    float out[12];
    CHECK_INT(t, sr_output_convert_linear(&color, &project, &frame, out, 1), SR_OK);
    CHECK_NEAR(t, out[0], sr_color_decode(0.25, SR_COLOR_SRGB), 1e-6);
    CHECK_NEAR(t, out[4], sr_color_decode(0.6, SR_COLOR_SRGB) * 0.5, 1e-6);
    CHECK_NEAR(t, out[7], 0.5, 0.0);
    CHECK(t, out[8] == 0.0f && out[11] == 0.0f);
    sr_color_output_free(&color);
    /* A linear project is not decoded twice; the gamut matrix applies. */
    project.linear_light = true;
    CHECK_INT(t, sr_color_output_init_bits(&color, &project, SR_COLOR_REC2020, 16),
              SR_OK);
    CHECK_INT(t, sr_output_convert_linear(&color, &project, &frame, out, 2), SR_OK);
    double expected = color.matrix[0][0] * 0.25 + color.matrix[0][1] * 0.5 +
                      color.matrix[0][2] * 0.75;
    CHECK_NEAR(t, out[0], expected, 1e-6);
    sr_color_output_free(&color);
    sr_frame_free(&frame);
}

/* -------------------------------------------------------- rendering */

/* One output of `codec` over the small animated scene. */
static void write_scene(const char *path, const char *outputs) {
    FILE *file = fopen(path, "w");
    if (!file) return;
    fprintf(file, BEGIN "%s" "<assets><audio id=\"tone\" src=\"%s\"/></assets>"
            "<composition><shape id=\"s\" shape=\"ellipse\" x=\"2\" y=\"2\" "
            "width=\"12\" height=\"12\" fill=\"#F0A030\"><animate "
            "property=\"position.x\"><key time=\"0\" value=\"0\"/><key time=\"1\" "
            "value=\"18\"/></animate></shape></composition><audioMix><audioTrack "
            "id=\"track\" asset=\"tone\" clipIn=\"0\" clipOut=\"1\"/></audioMix></scene>",
            outputs, sr_test_data_path("examples/assets/tone.wav"));
    fclose(file);
}

static SrStatus render_file(const char *scene_path, SrRenderOptions options,
                            char **message) {
    SrScene scene;
    FILE *log;
    SrDiagnostics diag;
    st_diag(&diag, &log);
    SrStatus status = sr_scene_load_xml(scene_path, &scene, &diag);
    SrRenderMetrics metrics;
    if (status == SR_OK) {
        status = sr_render(&scene, &options, &metrics, &diag);
        sr_scene_free(&scene);
    }
    char *text = log_text(log);
    if (message) *message = text;
    else free(text);
    return status;
}

/* Every added codec: 1 and 4 threads give identical bytes, the file
 * decodes to the expected frame count and size. */
static void codecs_thread_identity(sr_test_ctx *t) {
    static const struct { const char *codec, *file, *extra; int frames; } cases[] = {
        {"prores", "p.mov", " proresProfile=\"proxy\"", 6},
        {"prores", "p4.mkv", " proresProfile=\"4444xq\"", 6},
        {"vp9", "v.webm", " preset=\"ultrafast\" crf=\"50\"", 6},
        {"vp9", "v.mp4", " preset=\"ultrafast\" bitrate=\"200000\" keyframeInterval=\"0.5\"", 6},
        {"av1", "a.mp4", " preset=\"ultrafast\" crf=\"55\"", 6},
        {"av1", "a.webm", " preset=\"fast\" crf=\"60\" keyframeInterval=\"0.5\"", 6},
        {"gif", "g.gif", " loopCount=\"1\"", 6},
        {"apng", "n.png", " loopCount=\"3\" pixelFormat=\"rgba\"", 6},
        {"h264", "h.mov", " keyframeInterval=\"0.5\" bFrames=\"0\" faststart=\"false\" "
                          "preset=\"ultrafast\"", 6},
        {"h265", "x.mp4", " bFrames=\"2\" preset=\"ultrafast\"", 6},
    };
    for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); ++i) {
        char dir_a[1024], dir_b[1024], outputs[512], scene_a[1100], scene_b[1100];
        char out_a[1100], out_b[1100];
        snprintf(dir_a, sizeof(dir_a), "%s", scratch_dir("codec-a"));
        snprintf(dir_b, sizeof(dir_b), "%s", scratch_dir("codec-b"));
        for (int k = 0; k < 2; ++k) {
            const char *dir = k ? dir_b : dir_a;
            char *out = k ? out_b : out_a, *scene = k ? scene_b : scene_a;
            snprintf(out, 1100, "%s/%s", dir, cases[i].file);
            snprintf(scene, 1100, "%s/scene.xml", dir);
            snprintf(outputs, sizeof(outputs), "<output path=\"%s\" codec=\"%s\"%s/>",
                     cases[i].file, cases[i].codec, cases[i].extra);
            write_scene(scene, outputs);
            char *message = NULL;
            SrStatus status = render_file(scene, (SrRenderOptions){
                .encoder_threads = k ? 4 : 1}, &message);
            if (status != SR_OK)
                SR_FAIL(t, "%s (%s): status %d\n%s", cases[i].codec, cases[i].file,
                        (int)status, message);
            free(message);
        }
        if (!same_file(out_a, out_b))
            SR_FAIL(t, "%s (%s): 1 and 4 threads differ", cases[i].codec,
                    cases[i].file);
        int w = 0, h = 0, frames = 0;
        uint8_t *px = decode_frame(out_a, 0, AV_PIX_FMT_RGBA, &w, &h, &frames);
        if (!px || w != 32 || h != 18 || frames != cases[i].frames)
            SR_FAIL(t, "%s (%s): decoded %dx%d, %d frames", cases[i].codec,
                    cases[i].file, w, h, frames);
        free(px);
    }
}

static void sequences_and_slices(sr_test_ctx *t) {
    static const struct { const char *codec, *pattern, *first, *last; } cases[] = {
        {"png-sequence", "f-%02d.png", "f-00.png", "f-05.png"},
        {"tiff-sequence", "f-%01d.tif", "f-0.tif", "f-5.tif"},
        {"exr-sequence", "f-%03d.exr", "f-000.exr", "f-005.exr"},
    };
    for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); ++i) {
        char full[1024], sliced[1024], scene_full[1100], scene_sliced[1100];
        char outputs[256], a[1200], b[1200];
        snprintf(full, sizeof(full), "%s", scratch_dir("seq-full"));
        snprintf(sliced, sizeof(sliced), "%s", scratch_dir("seq-sliced"));
        snprintf(outputs, sizeof(outputs), "<output path=\"%s\" codec=\"%s\"/>",
                 cases[i].pattern, cases[i].codec);
        snprintf(scene_full, sizeof(scene_full), "%s/scene.xml", full);
        snprintf(scene_sliced, sizeof(scene_sliced), "%s/scene.xml", sliced);
        write_scene(scene_full, outputs);
        write_scene(scene_sliced, outputs);
        CHECK_INT(t, render_file(scene_full, (SrRenderOptions){.encoder_threads = 4},
                                 NULL), SR_OK);
        /* Two slices in reverse order: the files are the same. */
        CHECK_INT(t, render_file(scene_sliced, (SrRenderOptions){.has_range = true,
                  .first_frame = 3, .end_frame = 6, .encoder_threads = 1}, NULL),
                  SR_OK);
        CHECK_INT(t, render_file(scene_sliced, (SrRenderOptions){.has_range = true,
                  .first_frame = 0, .end_frame = 3, .encoder_threads = 2}, NULL),
                  SR_OK);
        for (int f = 0; f < 6; ++f) {
            char name[64];
            sr_sequence_format(cases[i].pattern, (uint64_t)f, name, sizeof(name));
            snprintf(a, sizeof(a), "%s/%s", full, name);
            snprintf(b, sizeof(b), "%s/%s", sliced, name);
            if (!same_file(a, b)) SR_FAIL(t, "%s frame %d differs", cases[i].codec, f);
        }
        snprintf(a, sizeof(a), "%s/%s", full, cases[i].last);
        struct stat info;
        CHECK(t, stat(a, &info) == 0);
        (void)cases[i].first;
    }
}

/* Outputs sharing a pass produce the bytes each would produce alone, and
 * outputs in their own pass render at their own size and rate. */
static void shared_pass_equals_alone(sr_test_ctx *t) {
    const char *outputs =
        "<output id=\"h\" path=\"h.mkv\" codec=\"ffv1\" pixelFormat=\"yuv444p\"/>"
        "<output id=\"p\" path=\"p.mov\" codec=\"prores\" start=\"0.5\"/>"
        "<output id=\"n\" path=\"n-%01d.png\" codec=\"png-sequence\" end=\"0.5\"/>"
        "<output id=\"e\" path=\"e-%01d.exr\" codec=\"exr-sequence\" "
        "colorSpace=\"rec2020\"/>"
        "<output id=\"g\" path=\"g.gif\" codec=\"gif\" width=\"16\" height=\"9\" "
        "fps=\"3\"/>";
    const char *files[] = {"h.mkv", "p.mov", "n-0.png", "n-2.png", "e-0.exr",
                           "e-5.exr", "g.gif"};
    const char *alone[] = {"h", "p", "n", "n", "e", "e", "g"};
    char together[1024], scene_together[1100];
    snprintf(together, sizeof(together), "%s", scratch_dir("shared-all"));
    snprintf(scene_together, sizeof(scene_together), "%s/scene.xml", together);
    write_scene(scene_together, outputs);
    char *message = NULL;
    CHECK_INT(t, render_file(scene_together, (SrRenderOptions){.encoder_threads = 3},
                             &message), SR_OK);
    free(message);
    for (size_t i = 0; i < sizeof(files) / sizeof(files[0]); ++i) {
        char dir[1024], scene[1100], a[1200], b[1200];
        snprintf(dir, sizeof(dir), "%s", scratch_dir("shared-one"));
        snprintf(scene, sizeof(scene), "%s/scene.xml", dir);
        write_scene(scene, outputs);
        CHECK_INT(t, render_file(scene, (SrRenderOptions){.encoder_threads = 1,
                  .output_ids = alone[i]}, NULL), SR_OK);
        snprintf(a, sizeof(a), "%s/%s", together, files[i]);
        snprintf(b, sizeof(b), "%s/%s", dir, files[i]);
        if (!same_file(a, b)) SR_FAIL(t, "%s differs from a render alone", files[i]);
    }
    char path[1200];
    snprintf(path, sizeof(path), "%s/n-3.png", together);
    struct stat info;
    CHECK(t, stat(path, &info) != 0);       /* png ends at 0.5 s */
    snprintf(path, sizeof(path), "%s/g.gif", together);
    int w = 0, h = 0, frames = 0;
    free(decode_frame(path, 0, AV_PIX_FMT_RGBA, &w, &h, &frames));
    CHECK(t, w == 16 && h == 9 && frames == 3);
    snprintf(path, sizeof(path), "%s/p.mov", together);
    free(decode_frame(path, 0, AV_PIX_FMT_RGBA, &w, &h, &frames));
    CHECK_INT(t, frames, 3);
    /* The shared first-sink frame equals the exr sink's own pass. */
    char hash_out[1200];
    snprintf(hash_out, sizeof(hash_out), "%s/hash.txt", together);
    FILE *hashes = fopen(hash_out, "w+");
    SrScene scene;
    FILE *log;
    SrDiagnostics diag;
    st_diag(&diag, &log);
    if (hashes && sr_scene_load_xml(scene_together, &scene, &diag) == SR_OK) {
        SrRenderOptions options = {.hash = true, .hash_stream = hashes,
                                   .output_ids = "g", .encoder_threads = 2};
        SrRenderMetrics metrics;
        CHECK_INT(t, sr_render(&scene, &options, &metrics, &diag), SR_OK);
        CHECK_INT(t, metrics.frames, 3);
        sr_scene_free(&scene);
    }
    if (hashes) fclose(hashes);
    if (log) fclose(log);
}

static void resume_new_codecs(sr_test_ctx *t) {
    static const char *const cases[][2] = {
        {"<output path=\"r.mov\" codec=\"prores\"/>", "r.mov"},
        {"<output path=\"r.webm\" codec=\"vp9\" preset=\"ultrafast\" crf=\"50\" "
         "keyframeInterval=\"0.5\"/>", "r.webm"},
        {"<output path=\"r.mp4\" codec=\"av1\" preset=\"ultrafast\" crf=\"55\"/>",
         "r.mp4"},
    };
    for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); ++i) {
        char full[1024], resumed[1024], scene_a[1100], scene_b[1100];
        char a[1200], b[1200];
        snprintf(full, sizeof(full), "%s", scratch_dir("resume-full"));
        snprintf(resumed, sizeof(resumed), "%s", scratch_dir("resume-parts"));
        snprintf(scene_a, sizeof(scene_a), "%s/scene.xml", full);
        snprintf(scene_b, sizeof(scene_b), "%s/scene.xml", resumed);
        write_scene(scene_a, cases[i][0]);
        write_scene(scene_b, cases[i][0]);
        CHECK_INT(t, render_file(scene_a, (SrRenderOptions){.encoder_threads = 2},
                                 NULL), SR_OK);
        char *message = NULL;
        SrStatus status = render_file(scene_b, (SrRenderOptions){.resume = true,
            .segment_frames = 2, .encoder_threads = 2, .keep_parts = true}, &message);
        if (status != SR_OK) SR_FAIL(t, "resume %s: %d\n%s", cases[i][1], (int)status,
                                     message);
        free(message);
        snprintf(a, sizeof(a), "%s/%s", full, cases[i][1]);
        snprintf(b, sizeof(b), "%s/%s", resumed, cases[i][1]);
        int w, h, fa = 0, fb = 0;
        uint8_t *pa = decode_frame(a, 5, AV_PIX_FMT_RGBA, &w, &h, &fa);
        uint8_t *pb = decode_frame(b, 5, AV_PIX_FMT_RGBA, &w, &h, &fb);
        CHECK(t, pa && pb && fa == 6 && fb == 6);
        if (i == 0 && pa && pb)     /* intra-only: the same pictures */
            CHECK(t, memcmp(pa, pb, 32 * 18 * 4) == 0);
        free(pa);
        free(pb);
        /* A rerun reuses every segment and reproduces the bytes. */
        char copy[1300];
        snprintf(copy, sizeof(copy), "%s.first", b);
        rename(b, copy);
        CHECK_INT(t, render_file(scene_b, (SrRenderOptions){.resume = true,
                  .segment_frames = 2, .encoder_threads = 1}, NULL), SR_OK);
        CHECK(t, same_file(b, copy));
    }
}

static void stills_png_and_jpeg(sr_test_ctx *t) {
    char dir[1024], scene[1100], poster[1200], thumb[1200], preview[1200];
    snprintf(dir, sizeof(dir), "%s", scratch_dir("stills"));
    snprintf(scene, sizeof(scene), "%s/scene.xml", dir);
    write_scene(scene, "<output id=\"v\" path=\"v.mkv\" codec=\"ffv1\" "
                "colorSpace=\"display-p3\"><poster path=\"stills/poster.png\" "
                "format=\"png\" time=\"0.5\"/><thumbnail path=\"stills/t.jpg\" "
                "width=\"16\" time=\"0.9\" quality=\"0.5\"/></output>");
    CHECK_INT(t, render_file(scene, (SrRenderOptions){.encoder_threads = 2}, NULL),
              SR_OK);
    snprintf(poster, sizeof(poster), "%s/stills/poster.png", dir);
    snprintf(thumb, sizeof(thumb), "%s/stills/t.jpg", dir);
    snprintf(preview, sizeof(preview), "%s/preview.png", dir);
    CHECK_INT(t, render_file(scene, (SrRenderOptions){.preview = true,
              .preview_frame = 3, .preview_path = preview}, NULL), SR_OK);
    CHECK(t, same_file(poster, preview));   /* frame 3, output colour space */
    int w = 0, h = 0, frames = 0;
    uint8_t *px = decode_frame(thumb, 0, AV_PIX_FMT_RGBA, &w, &h, &frames);
    CHECK(t, px && w == 16 && h == 9);
    free(px);
    /* A slice without the still's frame does not write it. */
    remove(poster);
    CHECK_INT(t, render_file(scene, (SrRenderOptions){.has_range = true,
              .first_frame = 4, .end_frame = 6}, NULL), SR_OK);
    struct stat info;
    CHECK(t, stat(poster, &info) != 0);
    CHECK_INT(t, render_file(scene, (SrRenderOptions){.has_range = true,
              .first_frame = 3, .end_frame = 4}, NULL), SR_OK);
    CHECK(t, same_file(poster, preview));
}

/* The lossless codecs reproduce the golden reference exactly. */
static void lossless_codecs_match_golden(sr_test_ctx *t) {
    static const struct { SrCodec codec; const char *path, *file, *pixfmt; } cases[] = {
        {SR_CODEC_PNG_SEQUENCE, "g-%03d.png", "g-012.png", "rgb24"},
        {SR_CODEC_PNG_SEQUENCE, "r-%03d.png", "r-012.png", "rgba"},
        {SR_CODEC_TIFF_SEQUENCE, "g-%03d.tif", "g-012.tif", "rgb24"},
        {SR_CODEC_APNG, "g.apng", "g.apng", "rgb24"},
        {SR_CODEC_EXR_SEQUENCE, "g-%03d.exr", "g-012.exr", "gbrapf32le"},
    };
    int rw = 0, rh = 0;
    uint8_t *reference = decode_frame(
        sr_test_data_path("tests/golden/expected/outputs-f012.png"), 0,
        AV_PIX_FMT_RGBA, &rw, &rh, NULL);
    if (!reference) {
        SR_FAIL(t, "golden reference outputs-f012.png is missing");
        return;
    }
    for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); ++i) {
        SrScene scene;
        FILE *log;
        SrDiagnostics diag;
        st_diag(&diag, &log);
        if (sr_scene_load_xml(sr_test_data_path("tests/golden/outputs.xml"), &scene,
                              &diag) != SR_OK) {
            SR_FAIL(t, "load");
            break;
        }
        char dir[1024], path[1200], file[1200];
        snprintf(dir, sizeof(dir), "%s", scratch_dir("lossless"));
        snprintf(path, sizeof(path), "%s/%s", dir, cases[i].path);
        snprintf(file, sizeof(file), "%s/%s", dir, cases[i].file);
        scene.output.codec = cases[i].codec;
        free(scene.output.path);
        free(scene.output.pixel_format);
        scene.output.path = strdup(path);
        scene.output.pixel_format = strdup(cases[i].pixfmt);
        SrRenderOptions options = {.has_range = true, .first_frame = 12,
                                   .end_frame = 13, .encoder_threads = 4};
        SrRenderMetrics metrics;
        CHECK_INT(t, sr_render(&scene, &options, &metrics, &diag), SR_OK);
        if (cases[i].codec == SR_CODEC_EXR_SEQUENCE) {
            /* Linear floats: the golden frame's exact linear conversion. */
            SrColorOutput color;
            SrFrame frame = {0};
            int w = 0, h = 0;
            uint8_t *planes = decode_frame(file, 0, AV_PIX_FMT_GBRAPF32LE, &w, &h,
                                           NULL);
            CHECK(t, planes && w == rw && h == rh);
            bool exact = planes != NULL;
            for (size_t p = 0; exact && p < (size_t)w * h; ++p) {
                const float *g = (const float *)planes, *b = g + (size_t)w * h,
                            *r = b + (size_t)w * h, *a = r + (size_t)w * h;
                const uint8_t *ref = reference + p * 4;
                /* Re-encoding linear to sRGB 8-bit gives the reference. */
                double straight[3] = {r[p] / a[p], g[p] / a[p], b[p] / a[p]};
                for (int c = 0; c < 3; ++c) {
                    double code = floor(sr_color_encode(straight[c], SR_COLOR_SRGB) *
                                        255.0 + 0.5);
                    if (fabs(code - ref[c]) > 1.0) exact = false;
                }
                if (a[p] != 1.0f) exact = false;
            }
            CHECK(t, exact);
            /* And bit for bit the reviewed EXR reference (SR_UPDATE_GOLDEN=1
             * rewrites it from this render). */
            const char *expected = sr_test_data_path(
                "tests/golden/expected/outputs-f012.exr");
            const char *update = getenv("SR_UPDATE_GOLDEN");
            if (update && !strcmp(update, "1")) {
                uint8_t *data = NULL;
                size_t size = 0;
                FILE *copy = read_bytes(file, &data, &size) ? fopen(expected, "wb")
                                                            : NULL;
                CHECK(t, copy && fwrite(data, 1, size, copy) == size);
                if (copy) fclose(copy);
                free(data);
            }
            int ew = 0, eh = 0;
            uint8_t *want = decode_frame(expected, 0, AV_PIX_FMT_GBRAPF32LE, &ew, &eh,
                                         NULL);
            if (!want) SR_FAIL(t, "EXR reference outputs-f012.exr is missing");
            else if (planes && (ew != w || eh != h ||
                                memcmp(want, planes, (size_t)w * h * 16)))
                SR_FAIL(t, "EXR frame differs from outputs-f012.exr");
            free(want);
            free(planes);
            (void)color;
            (void)frame;
        } else {
            int w = 0, h = 0;
            uint8_t *px = decode_frame(file, 0, AV_PIX_FMT_RGBA, &w, &h, NULL);
            CHECK(t, px && w == rw && h == rh);
            if (px && w == rw && h == rh && memcmp(px, reference, (size_t)w * h * 4))
                SR_FAIL(t, "%s differs from the golden reference", cases[i].file);
            free(px);
        }
        sr_scene_free(&scene);
        if (log) fclose(log);
    }
    free(reference);
}

/* A GIF of at most 256 colours is lossless. */
static void gif_lossless_when_few_colours(sr_test_ctx *t) {
    char dir[1024], scene[1100], gif[1200], preview[1200];
    snprintf(dir, sizeof(dir), "%s", scratch_dir("gif-exact"));
    snprintf(scene, sizeof(scene), "%s/scene.xml", dir);
    FILE *file = fopen(scene, "w");
    if (!file) return;
    fputs(BEGIN "<output path=\"g.gif\" codec=\"gif\"/><composition><shape id=\"a\" "
          "shape=\"rect\" x=\"4\" y=\"2\" width=\"8\" height=\"8\" fill=\"#F0A030\"/>"
          "<shape id=\"b\" shape=\"rect\" x=\"16\" y=\"6\" width=\"10\" height=\"6\" "
          "fill=\"#30A0F0\"/></composition></scene>", file);
    fclose(file);
    CHECK_INT(t, render_file(scene, (SrRenderOptions){0}, NULL), SR_OK);
    snprintf(gif, sizeof(gif), "%s/g.gif", dir);
    snprintf(preview, sizeof(preview), "%s/p.png", dir);
    CHECK_INT(t, render_file(scene, (SrRenderOptions){.preview = true,
              .preview_frame = 2, .preview_path = preview}, NULL), SR_OK);
    int w, h;
    uint8_t *a = decode_frame(gif, 2, AV_PIX_FMT_RGBA, &w, &h, NULL);
    uint8_t *b = decode_frame(preview, 0, AV_PIX_FMT_RGBA, &w, &h, NULL);
    CHECK(t, a && b && memcmp(a, b, 32 * 18 * 4) == 0);
    free(a);
    free(b);
}

/* Legacy CLI failures keep their messages; new CLI conflicts are argument
 * errors before anything is written. */
static void render_errors(sr_test_ctx *t) {
    char dir[1024], scene[1100];
    snprintf(dir, sizeof(dir), "%s", scratch_dir("errors"));
    snprintf(scene, sizeof(scene), "%s/scene.xml", dir);
    write_scene(scene, "<output id=\"a\" path=\"a.mp4\" codec=\"h264\" start=\"0.5\"/>"
                "<output id=\"b\" path=\"b-%02d.png\" codec=\"png-sequence\" "
                "end=\"0.5\"/>");
    char *message = NULL;
    CHECK_INT(t, render_file(scene, (SrRenderOptions){.has_range = true,
              .first_frame = 6, .end_frame = 9}, &message), SR_ERR_ARGUMENT);
    CHECK_CONTAINS(t, message, "empty frame range");
    free(message);
    CHECK_INT(t, render_file(scene, (SrRenderOptions){.has_range = true,
              .first_frame = 4, .end_frame = 6}, &message), SR_OK);
    free(message);
    char skipped[1200];
    struct stat info;
    snprintf(skipped, sizeof(skipped), "%s/a.mp4", dir);
    CHECK(t, stat(skipped, &info) == 0);
    snprintf(skipped, sizeof(skipped), "%s/b-04.png", dir);
    CHECK(t, stat(skipped, &info) != 0);
    CHECK_INT(t, render_file(scene, (SrRenderOptions){.validate_only = true,
              .output_ids = "zz"}, &message), SR_ERR_ARGUMENT);
    CHECK_CONTAINS(t, message, "zz");
    free(message);
    /* An unwritable sequence directory is an I/O error naming the file. */
    write_scene(scene, "<output path=\"missing/../../nope/%02d.png\" "
                "codec=\"png-sequence\"/>");
    char blocker[1200];
    snprintf(blocker, sizeof(blocker), "%s/missing", dir);
    FILE *f = fopen(blocker, "w");
    if (f) fclose(f);
    SrStatus status = render_file(scene, (SrRenderOptions){0}, &message);
    CHECK(t, status == SR_ERR_IO);
    CHECK_CONTAINS(t, message, "cannot create directory");
    free(message);
    /* A frame file that cannot be written (a directory holds its name) is
     * an I/O error naming it. A failing second sink on the writer thread
     * stops the render with that status; the first sink's file is still
     * closed properly. */
    char blocked[1200];
    snprintf(blocked, sizeof(blocked), "%s/bad-01.png", dir);
    make_dir(blocked);
    write_scene(scene, "<output id=\"ok\" path=\"ok.mkv\" codec=\"ffv1\"/>"
                "<output id=\"bad\" path=\"bad-%02d.png\" "
                "codec=\"png-sequence\"/>");
    status = render_file(scene, (SrRenderOptions){.encoder_threads = 1,
                         .output_ids = "bad"}, &message);
    CHECK(t, status == SR_ERR_IO);
    CHECK_CONTAINS(t, message, "cannot write frame file");
    CHECK_CONTAINS(t, message, "bad-01.png");
    free(message);
    status = render_file(scene, (SrRenderOptions){.encoder_threads = 4}, &message);
    CHECK(t, status == SR_ERR_IO);
    free(message);
    char ok_file[1200];
    snprintf(ok_file, sizeof(ok_file), "%s/ok.mkv", dir);
    int w = 0, h = 0, frames = 0;
    free(decode_frame(ok_file, 0, AV_PIX_FMT_RGBA, &w, &h, &frames));
    CHECK(t, w == 32 && frames >= 1);
    /* Metadata cannot be embedded in a GIF. */
    write_scene(scene, "<output path=\"m.gif\" codec=\"gif\"/>");
    FILE *patch = fopen(scene, "r+");
    if (patch) fclose(patch);
    SrScene loaded;
    FILE *log;
    SrDiagnostics diag;
    st_diag(&diag, &log);
    if (sr_scene_load_xml(scene, &loaded, &diag) == SR_OK) {
        SrMetadataEntry entry = {strdup("title"), strdup("x"), 1};
        loaded.metadata = &entry;
        loaded.metadata_count = 1;
        SrRenderOptions options = {0};
        SrRenderMetrics metrics;
        CHECK_INT(t, sr_render(&loaded, &options, &metrics, &diag), SR_ERR_ARGUMENT);
        loaded.metadata = NULL;
        loaded.metadata_count = 0;
        free(entry.name);
        free(entry.value);
        sr_scene_free(&loaded);
    }
    char *text = log_text(log);
    CHECK_CONTAINS(t, text, "embedMetadata");
    free(text);
}

/* Regressions for the Codex diff review of B1-6. */
static void review_regressions(sr_test_ctx *t) {
    char dir[1024], scene_path[1100];
    snprintf(dir, sizeof(dir), "%s", scratch_dir("review"));
    snprintf(scene_path, sizeof(scene_path), "%s/scene.xml", dir);
    SrScene scene;
    FILE *log;
    SrDiagnostics diag;
    SrOutputPlan plan;
    char *message = NULL;
    /* 1: a malformed sequence override is an argument error (no scan past
     * the pattern). */
    write_scene(scene_path, "<output id=\"s\" path=\"s-%02d.png\" "
                "codec=\"png-sequence\"><poster path=\"p.png\"/></output>");
    st_diag(&diag, &log);
    if (sr_scene_load_xml(scene_path, &scene, &diag) == SR_OK) {
        SrRenderOptions bad = {.output_override = "bad-%s.png"};
        CHECK_INT(t, plan_with(&scene, bad, &plan, &message), SR_ERR_ARGUMENT);
        CHECK_CONTAINS(t, message, "not a sequence pattern");
        free(message);
        sr_output_plan_free(&plan);
        SrRenderOptions trailing = {.output_override = "bad-%"};
        CHECK_INT(t, plan_with(&scene, trailing, &plan, NULL), SR_ERR_ARGUMENT);
        sr_output_plan_free(&plan);
        sr_scene_free(&scene);
    }
    if (log) fclose(log);
    /* 4: an output skipped by the range in its own pass does not fail the
     * render; 5: disjoint ranges render only their own frames. */
    write_scene(scene_path, "<output id=\"a\" path=\"a.mkv\" codec=\"ffv1\" "
                "end=\"0.2\"/><output id=\"b\" path=\"b.mov\" codec=\"prores\" "
                "start=\"0.8\"/><output id=\"c\" path=\"c.mkv\" codec=\"ffv1\" "
                "width=\"16\" height=\"9\" end=\"0.5\"/>");
    CHECK_INT(t, render_file(scene_path, (SrRenderOptions){.has_range = true,
              .first_frame = 4, .end_frame = 6}, NULL), SR_OK);
    st_diag(&diag, &log);
    if (sr_scene_load_xml(scene_path, &scene, &diag) == SR_OK) {
        SrRenderOptions options = {.output_ids = "a,b", .encoder_threads = 2};
        SrRenderMetrics metrics;
        CHECK_INT(t, sr_render(&scene, &options, &metrics, &diag), SR_OK);
        CHECK_INT(t, metrics.frames, 3);    /* frames 0, 1 and 5 */
        /* 7: inherited sizes and rates obey the output limits. */
        scene.project.width = 20000;
        CHECK_INT(t, plan_with(&scene, (SrRenderOptions){0}, &plan, &message),
                  SR_ERR_ARGUMENT);
        CHECK_CONTAINS(t, message, "at most 16384");
        free(message);
        sr_output_plan_free(&plan);
        sr_scene_free(&scene);
    }
    if (log) fclose(log);
    /* 6: --output cannot silently change a 1.1 output's container or make
     * an authored faststart meaningless. */
    write_scene(scene_path, "<output path=\"w.webm\" codec=\"vp9\"/>");
    CHECK_INT(t, render_file(scene_path, (SrRenderOptions){.output_override =
              "w.mp4"}, &message), SR_ERR_ARGUMENT);
    CHECK_CONTAINS(t, message, "changes the container");
    free(message);
    write_scene(scene_path, "<output path=\"h.mp4\" codec=\"h264\" "
                "faststart=\"false\"/>");
    CHECK_INT(t, render_file(scene_path, (SrRenderOptions){.output_override =
              "h.mkv"}, &message), SR_ERR_ARGUMENT);
    CHECK_CONTAINS(t, message, "faststart applies only");
    free(message);
    /* 8: 1.0 lexical crf forms stay accepted. */
    expect_load(t, "<scene version=\"1.0\"><project width=\"8\" height=\"8\" "
                "fps=\"1\" duration=\"1\"/><output path=\"a.mp4\" "
                "codec=\"h264\" crf=\"+18\"/><composition/></scene>", SR_OK, NULL);
    expect_load(t, "<scene version=\"1.0\"><project width=\"8\" height=\"8\" "
                "fps=\"1\" duration=\"1\"/><output path=\"a.mp4\" "
                "codec=\"h264\" crf=\"-0\"/><composition/></scene>", SR_OK, NULL);
}

const sr_test_case sr_tests_outputs[] = {
    {"review_regressions", review_regressions},
    {"sequence_patterns", sequence_patterns},
    {"frame_ranges_closed_form", frame_ranges_closed_form},
    {"codec_table_and_muxers", codec_table_and_muxers},
    {"loader_reads_fixture", loader_reads_fixture},
    {"loader_rejections", loader_rejections},
    {"fuzz_output_parser", fuzz_output_parser},
    {"plan_groups_passes", plan_groups_passes},
    {"plan_cli_rules", plan_cli_rules},
    {"gif_palette_exact_and_median_cut", gif_palette_exact_and_median_cut},
    {"exr_linear_conversion", exr_linear_conversion},
    {"codecs_thread_identity", codecs_thread_identity},
    {"sequences_and_slices", sequences_and_slices},
    {"shared_pass_equals_alone", shared_pass_equals_alone},
    {"resume_new_codecs", resume_new_codecs},
    {"stills_png_and_jpeg", stills_png_and_jpeg},
    {"lossless_codecs_match_golden", lossless_codecs_match_golden},
    {"gif_lossless_when_few_colours", gif_lossless_when_few_colours},
    {"render_errors", render_errors},
    {NULL, NULL}};
