/* SPDX-License-Identifier: Apache-2.0 */
/* --resume directory safety (src/resume.c): symbolic links are never
 * followed, the lock excludes a second run, oversized manifests are stale,
 * and input fingerprints notice changed files. */
#include "scene_render/resume.h"

#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/file.h>
#include <sys/stat.h>
#include <unistd.h>

#include "harness.h"

static void write_file(const char *path, const char *text)
{
    FILE *file = fopen(path, "wb");
    if (!file) return;
    fputs(text, file);
    fclose(file);
}

static bool file_is(const char *path, const char *text)
{
    char buffer[256] = {0};
    FILE *file = fopen(path, "rb");
    if (!file) return false;
    size_t count = fread(buffer, 1, sizeof(buffer) - 1, file);
    fclose(file);
    return count == strlen(text) && memcmp(buffer, text, count) == 0;
}

static bool exists_nofollow(const char *path)
{
    struct stat info;
    return lstat(path, &info) == 0;
}

/* A scratch directory WORK/<name> emptied of what earlier runs left. */
static void fresh_dir(char *out, size_t size, const char *name)
{
    snprintf(out, size, "%s", sr_test_tmp_path(name));
    char command[1024];
    snprintf(command, sizeof(command), "rm -rf '%s'", out);
    if (system(command) != 0) fprintf(stderr, "cannot clear %s\n", out);
    mkdir(out, 0755);
}

typedef struct {
    SrScene scene;
    SrDiagnostics diag;
    FILE *sink;
    SrResumeSettings settings;
} Fixture;

static bool fixture_open(Fixture *f)
{
    sr_scene_init(&f->scene);
    f->scene.source_hash = 0x1234;
    f->sink = tmpfile();
    sr_diag_init(&f->diag, "resume", f->sink ? f->sink : stderr);
    f->settings = (SrResumeSettings){1, 8, "cpu"};
    return f->scene.output.pixel_format && f->scene.output.preset;
}

static void fixture_close(Fixture *f)
{
    sr_scene_free(&f->scene);
    if (f->sink) fclose(f->sink);
}

static bool log_contains(Fixture *f, const char *needle)
{
    if (!f->sink) return false;
    fflush(f->sink);
    rewind(f->sink);
    char text[4096] = {0};
    size_t count = fread(text, 1, sizeof(text) - 1, f->sink);
    text[count] = '\0';
    return strstr(text, needle) != NULL;
}

static void parts_symlink_is_refused(sr_test_ctx *t)
{
    char root[256], victim[600], link[640], output[600], path[1024];
    fresh_dir(root, sizeof(root), "resume-symlink");
    snprintf(victim, sizeof(victim), "%s/victim", root);
    snprintf(output, sizeof(output), "%s/out.mp4", root);
    snprintf(link, sizeof(link), "%s.parts", output);
    mkdir(victim, 0755);
    snprintf(path, sizeof(path), "%s/seg-000000.mp4", victim);
    write_file(path, "precious segment");
    snprintf(path, sizeof(path), "%s/manifest", victim);
    write_file(path, "precious manifest");
    CHECK(t, symlink(victim, link) == 0);
    Fixture f;
    CHECK(t, fixture_open(&f));
    SrResume resume;
    SrStatus status = sr_resume_prepare(&resume, &f.scene, output, 0, 10, 5, NULL,
                                        &f.settings, &f.diag);
    CHECK_INT(t, status, SR_ERR_IO);
    CHECK(t, log_contains(&f, "symbolic link"));
    snprintf(path, sizeof(path), "%s/seg-000000.mp4", victim);
    CHECK(t, file_is(path, "precious segment"));
    snprintf(path, sizeof(path), "%s/manifest", victim);
    CHECK(t, file_is(path, "precious manifest"));
    snprintf(path, sizeof(path), "%s/lock", victim);
    CHECK(t, !exists_nofollow(path));
    fixture_close(&f);
}

static void planted_links_are_not_followed(sr_test_ctx *t)
{
    char root[256], victim[600], output[600], parts[640], path[1024];
    fresh_dir(root, sizeof(root), "resume-planted");
    snprintf(victim, sizeof(victim), "%s/victim.txt", root);
    snprintf(output, sizeof(output), "%s/out.mp4", root);
    snprintf(parts, sizeof(parts), "%s.parts", output);
    write_file(victim, "keep");
    mkdir(parts, 0755);
    /* Every name a run writes or reads, planted as a link to the victim. */
    const char *names[] = {"manifest.tmp", "manifest", "seg-000001.part.mp4"};
    for (size_t i = 0; i < sizeof names / sizeof names[0]; ++i) {
        snprintf(path, sizeof(path), "%s/%s", parts, names[i]);
        CHECK(t, symlink(victim, path) == 0);
    }
    snprintf(path, sizeof(path), "%s/manifest.tmp.%ld.0", parts, (long)getpid());
    CHECK(t, symlink(victim, path) == 0);
    /* A linked committed segment is not a segment. */
    snprintf(path, sizeof(path), "%s/seg-000000.mp4", parts);
    CHECK(t, symlink(victim, path) == 0);
    Fixture f;
    CHECK(t, fixture_open(&f));
    SrResume resume;
    SrStatus status = sr_resume_prepare(&resume, &f.scene, output, 0, 10, 5, NULL,
                                        &f.settings, &f.diag);
    CHECK_INT(t, status, SR_OK);
    if (status == SR_OK) {
        CHECK(t, !sr_resume_segment_done(&resume, 0));
        char *temporary = NULL;
        CHECK_INT(t, sr_resume_segment_begin(&resume, 1, &temporary, &f.diag), SR_OK);
        if (temporary) {
            struct stat info;
            CHECK(t, lstat(temporary, &info) == 0 && S_ISREG(info.st_mode));
            write_file(temporary, "segment");
            CHECK_INT(t, sr_resume_segment_commit(&resume, 1, temporary, &f.diag),
                      SR_OK);
            CHECK(t, sr_resume_segment_done(&resume, 1));
            free(temporary);
        }
        CHECK_INT(t, sr_resume_remove(&resume, &f.diag), SR_OK);
        sr_resume_close(&resume);
    }
    CHECK(t, file_is(victim, "keep"));
    CHECK(t, !exists_nofollow(parts));
    fixture_close(&f);
}

static void lock_excludes_second_run(sr_test_ctx *t)
{
    char root[256], output[600], parts[640], lock[700], segment[700];
    fresh_dir(root, sizeof(root), "resume-lock");
    snprintf(output, sizeof(output), "%s/out.mp4", root);
    snprintf(parts, sizeof(parts), "%s.parts", output);
    snprintf(lock, sizeof(lock), "%s/lock", parts);
    snprintf(segment, sizeof(segment), "%s/seg-000000.mp4", parts);
    mkdir(parts, 0755);
    write_file(segment, "other run's segment");
    int fd = open(lock, O_RDWR | O_CREAT, 0644);
    CHECK(t, fd >= 0 && flock(fd, LOCK_EX | LOCK_NB) == 0);
    Fixture f;
    CHECK(t, fixture_open(&f));
    SrResume resume;
    CHECK_INT(t, sr_resume_prepare(&resume, &f.scene, output, 0, 10, 5, NULL,
                                   &f.settings, &f.diag), SR_ERR_IO);
    CHECK(t, log_contains(&f, "another render is using"));
    CHECK(t, file_is(segment, "other run's segment"));
    if (fd >= 0) close(fd);
    /* Released: the next run proceeds (and discards the unknown segment). */
    CHECK_INT(t, sr_resume_prepare(&resume, &f.scene, output, 0, 10, 5, NULL,
                                   &f.settings, &f.diag), SR_OK);
    CHECK(t, !exists_nofollow(segment));
    sr_resume_close(&resume);
    fixture_close(&f);
}

static void oversized_manifest_is_stale(sr_test_ctx *t)
{
    char root[256], output[600], parts[640], path[1024];
    fresh_dir(root, sizeof(root), "resume-bigmanifest");
    snprintf(output, sizeof(output), "%s/out.mp4", root);
    snprintf(parts, sizeof(parts), "%s.parts", output);
    Fixture f;
    CHECK(t, fixture_open(&f));
    SrResume resume;
    CHECK_INT(t, sr_resume_prepare(&resume, &f.scene, output, 0, 10, 5, NULL,
                                   &f.settings, &f.diag), SR_OK);
    sr_resume_close(&resume);
    /* The real manifest followed by padding past 1 MiB: its prefix still
     * matches, but a file this large is never read. */
    snprintf(path, sizeof(path), "%s/manifest", parts);
    FILE *file = fopen(path, "ab");
    CHECK(t, file != NULL);
    if (file) {
        for (int i = 0; i < (1 << 20) / 64 + 1; ++i)
            fputs("# padding padding padding padding padding padding padding pad\n",
                  file);
        fclose(file);
    }
    snprintf(path, sizeof(path), "%s/seg-000000.mp4", parts);
    write_file(path, "segment");
    CHECK_INT(t, sr_resume_prepare(&resume, &f.scene, output, 0, 10, 5, NULL,
                                   &f.settings, &f.diag), SR_OK);
    CHECK(t, !exists_nofollow(path));
    snprintf(path, sizeof(path), "%s/manifest", parts);
    struct stat info;
    CHECK(t, stat(path, &info) == 0 && info.st_size < 4096);
    sr_resume_close(&resume);
    fixture_close(&f);
}

static void inputs_detect_changes(sr_test_ctx *t)
{
    char root[256], path[1024];
    fresh_dir(root, sizeof(root), "resume-inputs");
    snprintf(path, sizeof(path), "%s/image.ppm", root);
    write_file(path, "P6\n1 1\n255\nabc");
    Fixture f;
    CHECK(t, fixture_open(&f));
    free(f.scene.base_dir);
    f.scene.base_dir = sr_strdup(root);
    SrAsset asset = {.type = SR_ASSET_IMAGE, .source = "image.ppm"};
    f.scene.assets = &asset;
    f.scene.asset_count = 1;
    SrResumeInputs inputs;
    CHECK_INT(t, sr_resume_inputs_capture(&inputs, &f.scene, &f.diag), SR_OK);
    CHECK_INT(t, inputs.count, 1);
    if (inputs.count == 1) {
        CHECK(t, inputs.files[0].present);
        CHECK(t, inputs.files[0].hash ==
                 sr_fnv1a64(SR_FNV_OFFSET, "P6\n1 1\n255\nabc", 14));
    }
    CHECK_INT(t, sr_resume_inputs_verify(&inputs, "loading", &f.diag), SR_OK);
    /* Replaced by another file (new inode), as an editor's save does. */
    char other[1040];
    snprintf(other, sizeof(other), "%s.new", path);
    write_file(other, "P6\n1 1\n255\nxyz");
    CHECK(t, rename(other, path) == 0);
    CHECK_INT(t, sr_resume_inputs_verify(&inputs, "loading", &f.diag), SR_ERR_ASSET);
    CHECK(t, log_contains(&f, "asset 'image.ppm' changed while loading"));
    sr_resume_inputs_free(&inputs);
    f.scene.assets = NULL;
    f.scene.asset_count = 0;
    fixture_close(&f);
}

static void output_temp_is_atomic(sr_test_ctx *t)
{
    char root[256], output[600];
    fresh_dir(root, sizeof(root), "resume-output");
    snprintf(output, sizeof(output), "%s/final.mkv", root);
    write_file(output, "previous output");
    Fixture f;
    CHECK(t, fixture_open(&f));
    char *temporary = NULL;
    CHECK_INT(t, sr_output_temp_create(output, &temporary, &f.diag), SR_OK);
    if (temporary) {
        size_t length = strlen(temporary);
        CHECK(t, length > 4 && strcmp(temporary + length - 4, ".mkv") == 0);
        CHECK(t, strncmp(temporary, root, strlen(root)) == 0);
        CHECK(t, file_is(output, "previous output"));
        write_file(temporary, "new output");
        CHECK_INT(t, sr_output_temp_commit(temporary, output, &f.diag), SR_OK);
        CHECK(t, file_is(output, "new output"));
        CHECK(t, !exists_nofollow(temporary));
        free(temporary);
    }
    fixture_close(&f);
}

const sr_test_case sr_tests_resume[] = {
    {"parts_symlink_is_refused", parts_symlink_is_refused},
    {"planted_links_are_not_followed", planted_links_are_not_followed},
    {"lock_excludes_second_run", lock_excludes_second_run},
    {"oversized_manifest_is_stale", oversized_manifest_is_stale},
    {"inputs_detect_changes", inputs_detect_changes},
    {"output_temp_is_atomic", output_temp_is_atomic},
    {NULL, NULL},
};
