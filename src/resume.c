#define _POSIX_C_SOURCE 200809L
#include "scene_render/resume.h"

#include <dirent.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#define HEAD_BYTES (1024u * 1024u)

/* Hash of the file's first `limit` bytes (all of it when limit is 0);
 * false when it cannot be read. */
static bool hash_file(const char *path, size_t limit, uint64_t *hash) {
    FILE *file = fopen(path, "rb");
    if (!file) return false;
    uint64_t value = SR_FNV_OFFSET;
    uint8_t buffer[16384];
    size_t total = 0, count;
    while ((!limit || total < limit) &&
           (count = fread(buffer, 1, sizeof(buffer), file)) > 0) {
        if (limit && count > limit - total) count = limit - total;
        value = sr_fnv1a64(value, buffer, count);
        total += count;
    }
    bool ok = !ferror(file);
    fclose(file);
    *hash = value;
    return ok;
}

static void manifest_file(FILE *out, const char *key, const char *base_dir,
                          const char *relative) {
    char *path = sr_path_join(base_dir, relative);
    struct stat info;
    uint64_t hash = 0;
    if (!path || stat(path, &info) != 0 || !hash_file(path, HEAD_BYTES, &hash)) {
        fprintf(out, "%s=%s missing\n", key, relative);
    } else {
        fprintf(out, "%s=%s size=%lld mtime=%lld.%09ld head=%016llx\n", key,
                relative, (long long)info.st_size, (long long)info.st_mtim.tv_sec,
                (long)info.st_mtim.tv_nsec, (unsigned long long)hash);
    }
    free(path);
}

/* The manifest text; NULL when out of memory or the scene is unreadable. */
static char *build_manifest(const SrScene *scene, uint64_t first, uint64_t end,
                            uint32_t segment_frames,
                            const SrResumeSettings *settings, SrDiagnostics *diag) {
    uint64_t scene_hash;
    if (!hash_file(scene->source_path, 0, &scene_hash)) {
        sr_diag_error(diag, 0, NULL, NULL, "cannot read scene '%s' for the resume "
                      "manifest: %s", scene->source_path, strerror(errno));
        return NULL;
    }
    char *text = NULL;
    size_t size = 0;
    FILE *out = open_memstream(&text, &size);
    if (!out) return NULL;
    const SrProject *p = &scene->project;
    const SrOutput *o = &scene->output;
    fprintf(out, "scene-render-resume 1\nversion=%s\nscene=%016llx\n", SR_VERSION,
            (unsigned long long)scene_hash);
    for (size_t i = 0; i < scene->asset_count; ++i) {
        if (scene->assets[i].source)
            manifest_file(out, "asset", scene->base_dir, scene->assets[i].source);
        if (scene->assets[i].font_file)
            manifest_file(out, "font", scene->base_dir, scene->assets[i].font_file);
    }
    fprintf(out, "range=%llu:%llu\nsegment_frames=%u\n", (unsigned long long)first,
            (unsigned long long)end, segment_frames);
    fprintf(out, "project=%ux%u fps=%u/%u duration=%.17g seed=%llu linear=%d "
            "working=%d background=%.17g,%.17g,%.17g,%.17g mode=%d aa3d=%u\n",
            p->width, p->height, p->fps_num, p->fps_den, p->duration,
            (unsigned long long)p->seed, (int)p->linear_light,
            (int)p->working_color_space, p->background.r, p->background.g,
            p->background.b, p->background.a, (int)p->mode, p->antialias3d);
    fprintf(out, "scene360=%d %ux%u\n", (int)scene->scene360.enabled,
            scene->scene360.width, scene->scene360.height);
    fprintf(out, "video=codec:%d pixfmt:%s preset:%s crf:%d bitrate:%llu "
            "space:%d full:%d\n", (int)o->codec, o->pixel_format, o->preset,
            o->crf, (unsigned long long)o->bitrate, (int)o->color_space,
            (int)o->full_range);
    fprintf(out, "threads=%u gpu=%d bits=%u\n", settings->threads,
            (int)settings->gpu, settings->bits);
    if (fclose(out) != 0) {
        free(text);
        return NULL;
    }
    return text;
}

static char *read_text(const char *path) {
    FILE *file = fopen(path, "rb");
    if (!file) return NULL;
    char *text = NULL;
    size_t size = 0, used = 0;
    for (;;) {
        if (used + 4096 + 1 > size) {
            size_t grown = size ? size * 2 : 8192;
            char *next = sr_realloc(text, grown);
            if (!next) { free(text); fclose(file); return NULL; }
            text = next;
            size = grown;
        }
        size_t count = fread(text + used, 1, 4096, file);
        used += count;
        if (count < 4096) break;
    }
    bool ok = !ferror(file);
    fclose(file);
    if (!ok) { free(text); return NULL; }
    text[used] = '\0';
    return text;
}

static char *join(const char *directory, const char *name) {
    size_t length = strlen(directory) + strlen(name) + 2;
    char *path = sr_alloc(length);
    if (path) snprintf(path, length, "%s/%s", directory, name);
    return path;
}

static bool is_segment(const char *name) {
    return strncmp(name, "seg-", 4) == 0;
}

static bool is_partial(const char *name) {
    return is_segment(name) && strstr(name, ".part.") != NULL;
}

/* Unlinks every entry of `directory` accepted by `match`; counts removed
 * committed segments into *removed when non-NULL. */
static SrStatus remove_matching(const char *directory, bool (*match)(const char *),
                                size_t *removed, SrDiagnostics *diag) {
    DIR *dir = opendir(directory);
    if (!dir) {
        sr_diag_error(diag, 0, NULL, NULL, "cannot list '%s': %s", directory,
                      strerror(errno));
        return SR_ERR_IO;
    }
    SrStatus status = SR_OK;
    struct dirent *entry;
    while ((entry = readdir(dir)) != NULL) {
        if (!match(entry->d_name)) continue;
        char *path = join(directory, entry->d_name);
        if (!path) { status = SR_ERR_MEMORY; break; }
        if (unlink(path) != 0 && errno != ENOENT) {
            sr_diag_error(diag, 0, NULL, NULL, "cannot remove '%s': %s", path,
                          strerror(errno));
            status = SR_ERR_IO;
        } else if (removed && !is_partial(entry->d_name)) {
            ++*removed;
        }
        free(path);
    }
    closedir(dir);
    return status;
}

static SrStatus write_manifest(const char *path, const char *text,
                               SrDiagnostics *diag) {
    size_t length = strlen(path) + 8;
    char *temporary = sr_alloc(length);
    if (!temporary) return SR_ERR_MEMORY;
    snprintf(temporary, length, "%s.tmp", path);
    FILE *file = fopen(temporary, "wb");
    bool ok = file && fwrite(text, 1, strlen(text), file) == strlen(text);
    if (file && fclose(file) != 0) ok = false;
    if (ok && rename(temporary, path) != 0) ok = false;
    if (!ok) {
        sr_diag_error(diag, 0, NULL, NULL, "cannot write resume manifest '%s': %s",
                      path, strerror(errno));
        unlink(temporary);
    }
    free(temporary);
    return ok ? SR_OK : SR_ERR_IO;
}

SrStatus sr_resume_prepare(SrResume *resume, const SrScene *scene,
                           const char *output_path, uint64_t first,
                           uint64_t end, uint32_t segment_frames,
                           const SrResumeSettings *settings,
                           SrDiagnostics *diag) {
    *resume = (SrResume){0};
    if (!scene || !output_path || !settings || !segment_frames || first >= end)
        return SR_ERR_ARGUMENT;
    resume->first = first;
    resume->end = end;
    resume->segment_frames = segment_frames;
    resume->segment_count = (end - first + segment_frames - 1) / segment_frames;
    resume->extension = scene->output.codec == SR_CODEC_FFV1 ? "mkv" : "mp4";
    size_t length = strlen(output_path) + sizeof(".parts");
    resume->directory = sr_alloc(length);
    if (!resume->directory) return SR_ERR_MEMORY;
    snprintf(resume->directory, length, "%s.parts", output_path);
    if (mkdir(resume->directory, 0775) != 0 && errno != EEXIST) {
        sr_diag_error(diag, 0, NULL, NULL, "cannot create '%s': %s",
                      resume->directory, strerror(errno));
        sr_resume_close(resume);
        return SR_ERR_IO;
    }
    char *manifest = build_manifest(scene, first, end, segment_frames, settings, diag);
    char *path = join(resume->directory, "manifest");
    if (!manifest || !path) {
        free(manifest);
        free(path);
        sr_resume_close(resume);
        return manifest ? SR_ERR_MEMORY : SR_ERR_IO;
    }
    SrStatus status = remove_matching(resume->directory, is_partial, NULL, diag);
    char *old = status == SR_OK ? read_text(path) : NULL;
    if (status == SR_OK && (!old || strcmp(old, manifest) != 0)) {
        size_t removed = 0;
        status = remove_matching(resume->directory, is_segment, &removed, diag);
        if (status == SR_OK && (old || removed))
            sr_diag_info(diag, "resume manifest '%s' changed; discarded %zu old "
                         "segment(s)", path, removed);
        if (status == SR_OK) status = write_manifest(path, manifest, diag);
    }
    if (status == SR_OK)
        sr_diag_info(diag, "resume: %llu segment(s) of %u frames in %s",
                     (unsigned long long)resume->segment_count, segment_frames,
                     resume->directory);
    free(old);
    free(manifest);
    free(path);
    if (status != SR_OK) sr_resume_close(resume);
    return status;
}

void sr_resume_segment_range(const SrResume *resume, uint64_t k, uint64_t *from,
                             uint64_t *to) {
    *from = resume->first + k * resume->segment_frames;
    *to = *from + resume->segment_frames < resume->end
              ? *from + resume->segment_frames : resume->end;
}

char *sr_resume_segment_path(const SrResume *resume, uint64_t k, bool partial) {
    char name[64];
    snprintf(name, sizeof(name), "seg-%06llu%s.%s", (unsigned long long)k,
             partial ? ".part" : "", resume->extension);
    return join(resume->directory, name);
}

bool sr_resume_segment_done(const SrResume *resume, uint64_t k) {
    char *path = sr_resume_segment_path(resume, k, false);
    struct stat info;
    bool done = path && stat(path, &info) == 0 && S_ISREG(info.st_mode);
    free(path);
    return done;
}

SrStatus sr_resume_commit(const SrResume *resume, uint64_t k, SrDiagnostics *diag) {
    char *partial = sr_resume_segment_path(resume, k, true);
    char *final = sr_resume_segment_path(resume, k, false);
    SrStatus status = SR_OK;
    if (!partial || !final) {
        status = SR_ERR_MEMORY;
    } else if (rename(partial, final) != 0) {
        sr_diag_error(diag, 0, NULL, NULL, "cannot commit segment '%s': %s", final,
                      strerror(errno));
        status = SR_ERR_IO;
    }
    free(partial);
    free(final);
    return status;
}

static bool is_resume_file(const char *name) {
    return is_segment(name) || strcmp(name, "manifest") == 0 ||
           strcmp(name, "manifest.tmp") == 0;
}

SrStatus sr_resume_remove(const SrResume *resume, SrDiagnostics *diag) {
    SrStatus status = remove_matching(resume->directory, is_resume_file, NULL, diag);
    if (status == SR_OK && rmdir(resume->directory) != 0) {
        /* Something foreign lives there: keep it, the render succeeded. */
        sr_diag_warning(diag, 0, NULL, NULL, "cannot remove '%s': %s",
                        resume->directory, strerror(errno));
    }
    return status;
}

void sr_resume_close(SrResume *resume) {
    if (!resume) return;
    free(resume->directory);
    *resume = (SrResume){0};
}
