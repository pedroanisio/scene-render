/* SPDX-License-Identifier: Apache-2.0 */
#include "scene_text.h"
#include "scene_render/encoder.h"
#include "scene_render/renderer.h"

#include <libavformat/avformat.h>
#include <time.h>

#define PROJECT "<project width=\"16\" height=\"16\" fps=\"4\" duration=\"1\"/>"
#define BEGIN "<scene version=\"1.1\">" PROJECT
#define END "<composition/></scene>"

static void expect_xml(sr_test_ctx *t, const char *xml, SrStatus expected,
                        const char *diagnostic) {
    SrScene scene;
    char *message = NULL;
    SrStatus status = st_load(t, "metadata.xml", xml, &scene, &message);
    CHECK_INT(t, status, expected);
    if (diagnostic) CHECK_CONTAINS(t, message, diagnostic);
    if (status == SR_OK) sr_scene_free(&scene);
    free(message);
}

static bool load_fixture(sr_test_ctx *t, SrScene *scene) {
    FILE *sink;
    SrDiagnostics diag;
    st_diag(&diag, &sink);
    SrStatus status = sr_scene_load_xml(sr_test_data_path("tests/data-metadata.xml"),
                                       scene, &diag);
    CHECK_INT(t, status, SR_OK);
    if (sink) fclose(sink);
    return status == SR_OK;
}

static void parser_and_scope(sr_test_ctx *t) {
    static const struct { const char *name, *value; } expected[] = {
        {"title", ""}, {"author", "Author"},
        {"description", "Résumé ☀\nSecond line"},
        {"keywords", "schema,metadata"}, {"copyright", "© 2026"},
        {"revision", "r2"}, {"created", "2026-01-02T03:04:05Z"},
        {"modified", "2026-02-03T04:05:06-03:00"}, {"generator", "fixture"},
        {"language", "pt-BR"}, {"timecode", "01:02:03:04"}, {"empty", ""},
        {"作品", "日本語"}, {"x-en", "English suffix"},
        {"x-eng", "Long English suffix"}, {"x", "No suffix"},
        {"custom note", "Spaces stay in values"}, {"performer", "Performer"},
        {"track", "007"},
    };
    SrScene scene;
    if (load_fixture(t, &scene)) {
        CHECK(t, scene.output.embed_metadata);
        CHECK_INT(t, scene.metadata_count, sizeof(expected) / sizeof(expected[0]));
        for (size_t i = 0; i < scene.metadata_count; ++i) {
            CHECK_STR(t, scene.metadata[i].name, expected[i].name);
            CHECK_STR(t, scene.metadata[i].value, expected[i].value);
            CHECK(t, scene.metadata[i].source_line >= 3);
        }
        sr_scene_free(&scene);
        CHECK(t, !scene.metadata && !scene.metadata_count);
    }
    static const struct { const char *xml, *error; } errors[] = {
        {BEGIN "<metadata><meta name=\"\" value=\"x\"/></metadata>" END,
         "<meta> @name: metadata name"},
        {BEGIN "<metadata title=\"x\"><meta name=\"TITLE\" value=\"y\"/></metadata>" END,
         "duplicate metadata name 'TITLE'"},
        {BEGIN "<metadata><meta name=\"x\" value=\"1\"/>\n"
               "<meta name=\"X\" value=\"2\"/></metadata>" END,
         ":2: error: <meta> @name: duplicate"},
        {"<scene version=\"1.0\">" PROJECT "<metadata/>" END,
         "requires version=\"1.1\": <metadata>"},
        {BEGIN "<metadata><accessibility/></metadata>" END,
         "unsupported in this build: <accessibility>"},
        {BEGIN "<metadata unknown=\"x\"/>" END, "@unknown:"},
        {BEGIN "<metadata><meta name=\"x\"/></metadata>" END, "value"},
        {BEGIN "<metadata/><metadata/>" END, "metadata"},
        {BEGIN "<meta name=\"x\" value=\"x\"/>" END, "meta"},
        {BEGIN "<output path=\"x.mp4\" codec=\"h264\" embedMetadata=\"yes\"/>" END,
         "embedMetadata"},
    };
    for (size_t i = 0; i < sizeof(errors) / sizeof(errors[0]); ++i)
        expect_xml(t, errors[i].xml, SR_ERR_XML, errors[i].error);
    static const char *const flags[] = {"true", "false", "1", "0"};
    for (size_t i = 0; i < 4; ++i) {
        char xml[512];
        snprintf(xml, sizeof(xml), BEGIN "<output path=\"x.mp4\" codec=\"h264\" "
                 "embedMetadata=\"%s\"/>" END, flags[i]);
        if (st_load(t, "metadata.xml", xml, &scene, NULL) == SR_OK) {
            CHECK_INT(t, scene.output.embed_metadata, !(i % 2));
            sr_scene_free(&scene);
        } else CHECK(t, false);
    }
    expect_xml(t, "<scene version=\"1.0\">" PROJECT
               "<output path=\"x.mp4\" codec=\"h264\" embedMetadata=\"false\"/>" END,
               SR_OK, NULL);
}

static void resource_limits(sr_test_ctx *t) {
    char xml[20000], name[SR_MAX_METADATA_NAME + 2], value[SR_MAX_METADATA_VALUE + 2];
    memset(name, 'n', sizeof(name) - 1);
    name[sizeof(name) - 1] = '\0';
    memset(value, 'v', sizeof(value) - 1);
    value[sizeof(value) - 1] = '\0';
    for (unsigned extra = 0; extra < 2; ++extra) {
        name[SR_MAX_METADATA_NAME + extra] = '\0';
        snprintf(xml, sizeof(xml), BEGIN "<metadata><meta name=\"%s\" value=\"x\"/>"
                 "</metadata>" END, name);
        expect_xml(t, xml, extra ? SR_ERR_XML : SR_OK, extra ? "1..128 bytes" : NULL);
        name[SR_MAX_METADATA_NAME] = 'n';
        value[SR_MAX_METADATA_VALUE + extra] = '\0';
        snprintf(xml, sizeof(xml), BEGIN "<metadata title=\"%s\"/>" END, value);
        expect_xml(t, xml, extra ? SR_ERR_XML : SR_OK, extra ? "4096 bytes" : NULL);
        snprintf(xml, sizeof(xml), BEGIN "<metadata><meta name=\"x\" value=\"%s\"/>"
                 "</metadata>" END, value);
        expect_xml(t, xml, extra ? SR_ERR_XML : SR_OK, extra ? "4096 bytes" : NULL);
        value[SR_MAX_METADATA_VALUE] = 'v';
        size_t used = (size_t)snprintf(xml, sizeof(xml), BEGIN "<metadata>");
        for (size_t i = 0; i < SR_MAX_METADATA_ENTRIES + extra; ++i)
            used += (size_t)snprintf(xml + used, sizeof(xml) - used,
                                    "<meta name=\"n%zu\" value=\"\"/>", i);
        snprintf(xml + used, sizeof(xml) - used, "</metadata>" END);
        expect_xml(t, xml, extra ? SR_ERR_XML : SR_OK, extra ? "count exceeds 256" : NULL);
    }
}

static void container_policy(sr_test_ctx *t) {
    static const struct { const char *name; unsigned rejected; } names[] = {
        {"encoder", 7}, {"EnCoDeR-eng", 7}, {"Creation_Time", 7},
        {"location", 3}, {"com.apple.quicktime.artwork", 3},
        {"duration", 4}, {"encoding_tool", 4}, {"stereo_mode", 4},
        {"alpha_mode", 4}, {"creation time", 4}, {"Encoding Tool", 4},
        {"timecode", 0}, {"created", 0}, {"x-en", 0},
    };
    static const char *const paths[] = {"out.mp4", "out.mov", "out.mkv"};
    FILE *sink;
    SrDiagnostics diag;
    st_diag(&diag, &sink);
    SrScene scene;
    sr_scene_init(&scene);
    SrMetadataEntry entries[2] = {{.value = "value", .source_line = 7},
                                  {.value = "second", .source_line = 8}};
    scene.metadata = entries;
    scene.metadata_count = 1;
    for (size_t i = 0; i < sizeof(names) / sizeof(names[0]); ++i) {
        entries[0].name = (char *)names[i].name;
        for (size_t j = 0; j < 3; ++j) {
            CHECK_INT(t, sr_encoder_validate_metadata(&scene, paths[j], &diag),
                      names[i].rejected & (1u << j) ? SR_ERR_ARGUMENT : SR_OK);
            scene.output.embed_metadata = false;
            CHECK_INT(t, sr_encoder_validate_metadata(&scene, paths[j], &diag), SR_OK);
            scene.output.embed_metadata = true;
        }
    }
    static const char *const collisions[][2] = {
        {"custom note", "CUSTOM_NOTE"}, {"performer", "lead_performer"},
        {"track", "PART_NUMBER"},
    };
    scene.metadata_count = 2;
    for (size_t i = 0; i < sizeof(collisions) / sizeof(collisions[0]); ++i) {
        entries[0].name = (char *)collisions[i][0];
        entries[1].name = (char *)collisions[i][1];
        CHECK_INT(t, sr_encoder_validate_metadata(&scene, paths[0], &diag), SR_OK);
        CHECK_INT(t, sr_encoder_validate_metadata(&scene, paths[2], &diag), SR_ERR_ARGUMENT);
    }
    CHECK_INT(t, sr_encoder_validate_metadata(&scene, "out.unknown", &diag), SR_ERR_ARGUMENT);
    scene.metadata = NULL;
    scene.metadata_count = 0;
    sr_scene_free(&scene);
    if (sink) fclose(sink);
}

static bool files_equal(const char *left, const char *right) {
    FILE *a = fopen(left, "rb"), *b = fopen(right, "rb");
    bool equal = a && b;
    int ca = 0, cb = 0;
    while (equal && ca != EOF) {
        ca = fgetc(a);
        cb = fgetc(b);
        equal = ca == cb;
    }
    if (a) fclose(a);
    if (b) fclose(b);
    return equal;
}

static void check_tags(sr_test_ctx *t, const SrScene *scene, const char *path,
                        bool matroska, bool authored) {
    AVFormatContext *format = NULL;
    int rc = avformat_open_input(&format, path, NULL, NULL);
    CHECK_INT(t, rc, 0);
    if (rc < 0) return;
    CHECK(t, avformat_find_stream_info(format, NULL) >= 0);
    CHECK_INT(t, format->nb_streams, 1);
    for (size_t i = 0; i < scene->metadata_count; ++i) {
        const SrMetadataEntry *entry = &scene->metadata[i];
        const char *key = entry->name;
        if (matroska && !strcmp(key, "custom note")) key = "CUSTOM_NOTE";
        const AVDictionaryEntry *actual = av_dict_get(format->metadata, key, NULL, 0);
        if (authored) {
            if (!actual) SR_FAIL(t, "%s: missing tag %s", path, key);
            else CHECK_STR(t, actual->value, entry->value);
        } else if (actual) {
            SR_FAIL(t, "%s: unexpected authored tag %s", path, key);
        }
    }
    avformat_close_input(&format);
}

static SrStatus render_file(SrScene *scene, const char *path, unsigned threads,
                             bool resume, bool keep, SrRenderMetrics *metrics,
                             SrDiagnostics *diag) {
    SrRenderOptions options = {.output_override = path, .encoder_threads = threads,
        .resume = resume, .segment_frames = 2, .keep_parts = keep};
    return sr_render(scene, &options, metrics, diag);
}

static void container_roundtrips(sr_test_ctx *t) {
    const char *old = getenv("TZ");
    char *saved = old ? sr_strdup(old) : NULL;
    FILE *sink;
    SrDiagnostics diag;
    st_diag(&diag, &sink);
    SrScene scene;
    if (load_fixture(t, &scene)) {
        static const char *const suffixes[] = {"mp4", "mov", "mkv"};
        for (size_t i = 0; i < 3; ++i) {
            char name[64], first[1024], second[1024];
            snprintf(name, sizeof(name), "metadata-a.%s", suffixes[i]);
            snprintf(first, sizeof(first), "%s", sr_test_tmp_path(name));
            snprintf(name, sizeof(name), "metadata-b.%s", suffixes[i]);
            snprintf(second, sizeof(second), "%s", sr_test_tmp_path(name));
            SrRenderMetrics metrics;
            scene.output.embed_metadata = true;
            setenv("TZ", "UTC0", 1);
            tzset();
            CHECK_INT(t, render_file(&scene, first, 1, false, false, &metrics, &diag), SR_OK);
            check_tags(t, &scene, first, i == 2, true);
            setenv("TZ", "EST5EDT", 1);
            tzset();
            CHECK_INT(t, render_file(&scene, second, 4, false, false, &metrics, &diag), SR_OK);
            CHECK(t, files_equal(first, second));
            scene.output.embed_metadata = false;
            CHECK_INT(t, render_file(&scene, second, 1, false, false, &metrics, &diag), SR_OK);
            check_tags(t, &scene, second, i == 2, false);
        }
        sr_scene_free(&scene);
    }
    if (saved) setenv("TZ", saved, 1);
    else unsetenv("TZ");
    tzset();
    free(saved);
    if (sink) fclose(sink);
}

static void final_path_preflight(sr_test_ctx *t) {
    SrScene scene;
    if (st_load(t, "metadata.xml", BEGIN "<metadata><meta name=\"location\" "
                "value=\"arbitrary\"/></metadata>" END, &scene, NULL) != SR_OK) {
        CHECK(t, false);
        return;
    }
    FILE *sink;
    SrDiagnostics diag;
    st_diag(&diag, &sink);
    char path[1024], parts[1100];
    snprintf(path, sizeof(path), "%s", sr_test_tmp_path("metadata-preflight.mp4"));
    snprintf(parts, sizeof(parts), "%s.parts", path);
    unlink(path);
    SrRenderMetrics metrics;
    CHECK_INT(t, render_file(&scene, path, 1, true, true, &metrics, &diag), SR_ERR_ARGUMENT);
    CHECK(t, access(path, F_OK) != 0 && access(parts, F_OK) != 0);
    SrRenderOptions preview = {.preview = true,
        .preview_path = sr_test_tmp_path("metadata-preview.png"), .encoder_threads = 1};
    CHECK_INT(t, sr_render(&scene, &preview, &metrics, &diag), SR_OK);
    SrRenderOptions hash = {.hash = true, .hash_stream = sink, .encoder_threads = 1};
    CHECK_INT(t, sr_render(&scene, &hash, &metrics, &diag), SR_OK);
    snprintf(path, sizeof(path), "%s", sr_test_tmp_path("metadata-override.mkv"));
    CHECK_INT(t, render_file(&scene, path, 1, false, false, &metrics, &diag), SR_OK);
    check_tags(t, &scene, path, true, true);
    scene.output.embed_metadata = false;
    snprintf(path, sizeof(path), "%s", sr_test_tmp_path("metadata-disabled.mp4"));
    CHECK_INT(t, render_file(&scene, path, 1, false, false, &metrics, &diag), SR_OK);
    check_tags(t, &scene, path, false, false);
    sr_scene_free(&scene);
    if (sink) fclose(sink);
}

/* Simulate a cache made before 1.1 codec threads were pinned. */
static void remove_codec_policy(sr_test_ctx *t, const char *output) {
    char path[1400];
    snprintf(path, sizeof(path), "%s.parts/manifest", output);
    FILE *file = fopen(path, "rb");
    CHECK(t, file != NULL);
    if (!file) return;
    CHECK_INT(t, fseek(file, 0, SEEK_END), 0);
    long size = ftell(file);
    CHECK(t, size > 0);
    rewind(file);
    char *text = size > 0 ? malloc((size_t)size + 1) : NULL;
    CHECK(t, text != NULL);
    if (text) {
        CHECK_INT(t, fread(text, 1, (size_t)size, file), size);
        text[size] = '\0';
    }
    fclose(file);
    if (!text) return;
    const char *marker = "codec_policy=1 threads=1\n";
    char *at = strstr(text, marker);
    if (at) memmove(at, at + strlen(marker), strlen(at + strlen(marker)) + 1);
    file = fopen(path, "wb");
    CHECK(t, file != NULL);
    if (file) {
        CHECK(t, fputs(text, file) >= 0);
        CHECK_INT(t, fclose(file), 0);
    }
    free(text);
}

static void resumed_tags_and_fingerprint(sr_test_ctx *t) {
    static const char *const suffixes[] = {"mp4", "mov", "mkv"};
    char directory[1024];
    snprintf(directory, sizeof(directory), "%s", sr_test_tmp_path("metadata-resume-XXXXXX"));
    CHECK(t, mkdtemp(directory) != NULL);
    FILE *sink;
    SrDiagnostics diag;
    st_diag(&diag, &sink);
    for (size_t i = 0; i < 3; ++i) {
        char xml[1024], first[1200], second[1200], segment[1400];
        snprintf(xml, sizeof(xml), BEGIN
            "<metadata title=\"first\"><meta name=\"%s\" value=\"literal\"/>"
            "</metadata><output path=\"default.mp4\" codec=\"h264\" "
            "preset=\"ultrafast\"/>" END, i == 2 ? "location" : "custom");
        SrScene scene;
        SrStatus status = st_load(t, "metadata-resume.xml", xml, &scene, NULL);
        CHECK_INT(t, status, SR_OK);
        if (status != SR_OK) continue;
        uint64_t original_hash = scene.source_hash;
        snprintf(first, sizeof(first), "%s/first.%s", directory, suffixes[i]);
        snprintf(second, sizeof(second), "%s/second.%s", directory, suffixes[i]);
        SrRenderMetrics metrics;
        CHECK_INT(t, render_file(&scene, first, 1, true, true, &metrics, &diag), SR_OK);
        CHECK_INT(t, metrics.segments_rendered, 2);
        CHECK_INT(t, metrics.segments_reused, 0);
        check_tags(t, &scene, first, i == 2, true);
        for (unsigned k = 0; k < 2; ++k) {
            snprintf(segment, sizeof(segment), "%s.parts/seg-%06u.mp4", first, k);
            check_tags(t, &scene, segment, false, false);
        }
        CHECK_INT(t, render_file(&scene, first, 1, true, true, &metrics, &diag), SR_OK);
        CHECK_INT(t, metrics.segments_rendered, 0);
        CHECK_INT(t, metrics.segments_reused, 2);
        remove_codec_policy(t, first);
        CHECK_INT(t, render_file(&scene, first, 1, true, true, &metrics, &diag), SR_OK);
        CHECK_INT(t, metrics.segments_rendered, 2);
        CHECK_INT(t, metrics.segments_reused, 0);
        CHECK_INT(t, render_file(&scene, second, 4, true, false, &metrics, &diag), SR_OK);
        CHECK(t, files_equal(first, second));
        check_tags(t, &scene, second, i == 2, true);
        sr_scene_free(&scene);
        char *title = strstr(xml, "title=\"first\"");
        CHECK(t, title != NULL);
        if (title) memcpy(title + strlen("title=\""), "other", 5);
        status = st_load(t, "metadata-resume.xml", xml, &scene, NULL);
        CHECK_INT(t, status, SR_OK);
        if (status != SR_OK) continue;
        CHECK(t, scene.source_hash != original_hash);
        CHECK_INT(t, render_file(&scene, first, 1, true, false, &metrics, &diag), SR_OK);
        CHECK_INT(t, metrics.segments_rendered, 2);
        CHECK_INT(t, metrics.segments_reused, 0);
        check_tags(t, &scene, first, i == 2, true);
        sr_scene_free(&scene);
        unlink(first);
        unlink(second);
    }
    CHECK_INT(t, rmdir(directory), 0);
    if (sink) fclose(sink);
}

static uint32_t random_next(uint32_t *state) {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    return *state;
}

static void seeded_mutations(sr_test_ctx *t) {
    const char *input = BEGIN "<metadata title=\"title\"><meta name=\"custom\" "
        "value=\"value&#10;第二行\"/></metadata>" END;
    const size_t length = strlen(input);
    static const uint32_t seeds[] = {19, 73, 20260926};
    for (size_t i = 0; i < sizeof(seeds) / sizeof(seeds[0]); ++i) {
        uint32_t seed = seeds[i];
        for (unsigned j = 0; j < 72; ++j) {
            char xml[512];
            memcpy(xml, input, length + 1);
            size_t index = random_next(&seed) % length;
            xml[index] = j % 3 ? (char)(32 + random_next(&seed) % 95) : '\0';
            SrScene scene;
            SrStatus status = st_load(t, "metadata-fuzz.xml", xml, &scene, NULL);
            CHECK(t, status == SR_OK || status == SR_ERR_XML);
            if (status == SR_OK) {
                CHECK(t, scene.metadata_count <= SR_MAX_METADATA_ENTRIES);
                sr_scene_free(&scene);
            }
        }
    }
}

const sr_test_case sr_tests_metadata[] = {
    {"parser_and_scope", parser_and_scope},
    {"resource_limits", resource_limits},
    {"container_policy", container_policy},
    {"container_roundtrips", container_roundtrips},
    {"final_path_preflight", final_path_preflight},
    {"resumed_tags_and_fingerprint", resumed_tags_and_fingerprint},
    {"seeded_mutations", seeded_mutations},
    {NULL, NULL},
};
