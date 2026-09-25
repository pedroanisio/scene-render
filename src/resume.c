#define _POSIX_C_SOURCE 200809L
#include "scene_render/resume.h"

#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

static uint64_t hash_bytes(uint64_t hash, const void *data, size_t size) {
    const uint8_t *bytes = data;
    for (size_t i = 0; i < size; ++i) {
        hash ^= bytes[i];
        hash *= UINT64_C(1099511628211);
    }
    return hash;
}

static uint64_t hash_file(uint64_t hash, const char *path) {
    FILE *file = fopen(path, "rb");
    if (!file) return hash_bytes(hash, path, strlen(path));
    uint8_t buffer[16384];
    size_t count;
    while ((count = fread(buffer, 1, sizeof(buffer), file)) > 0)
        hash = hash_bytes(hash, buffer, count);
    fclose(file);
    return hash;
}

static uint64_t scene_hash(const SrScene *scene) {
    uint64_t hash = UINT64_C(1469598103934665603);
    hash = hash_bytes(hash, SR_VERSION, strlen(SR_VERSION));
    hash = hash_file(hash, scene->source_path);
    for (size_t i = 0; i < scene->asset_count; ++i) {
        if (!scene->assets[i].source) continue;
        char *path = sr_path_join(scene->base_dir, scene->assets[i].source);
        if (path) { hash = hash_file(hash, path); free(path); }
    }
    hash = hash_bytes(hash, &scene->project.width, sizeof(scene->project.width));
    hash = hash_bytes(hash, &scene->project.height, sizeof(scene->project.height));
    hash = hash_bytes(hash, &scene->project.fps_num, sizeof(scene->project.fps_num));
    hash = hash_bytes(hash, &scene->project.fps_den, sizeof(scene->project.fps_den));
    hash = hash_bytes(hash, &scene->project.duration, sizeof(scene->project.duration));
    hash = hash_bytes(hash, &scene->project.seed, sizeof(scene->project.seed));
    hash = hash_bytes(hash, &scene->project.linear_light,
                      sizeof(scene->project.linear_light));
    hash = hash_bytes(hash, &scene->project.background.r, sizeof(double));
    hash = hash_bytes(hash, &scene->project.background.g, sizeof(double));
    hash = hash_bytes(hash, &scene->project.background.b, sizeof(double));
    hash = hash_bytes(hash, &scene->project.background.a, sizeof(double));
    hash = hash_bytes(hash, &scene->project.mode, sizeof(scene->project.mode));
    return hash;
}

static SrStatus make_dirs(const char *path, SrDiagnostics *diag) {
    char *copy = sr_strdup(path);
    if (!copy) return SR_ERR_MEMORY;
    for (char *at = copy + 1;; ++at) {
        if (*at != '/' && *at != '\0') continue;
        char saved = *at;
        *at = '\0';
        if (mkdir(copy, 0775) != 0 && errno != EEXIST) {
            sr_diag_error(diag, 0, NULL, NULL, "cannot create resume cache '%s': %s",
                          copy, strerror(errno));
            free(copy);
            return SR_ERR_IO;
        }
        *at = saved;
        if (!saved) break;
    }
    free(copy);
    return SR_OK;
}

SrStatus sr_resume_open(SrResumeCache *cache, const SrScene *scene,
                        const char *output_path, bool enabled,
                        SrDiagnostics *diag) {
    *cache = (SrResumeCache){0};
    if (!enabled) return SR_OK;
    size_t pixels = (size_t)scene->project.width * scene->project.height;
    if (scene->project.height && pixels / scene->project.height !=
        scene->project.width) return SR_ERR_MEMORY;
    uint64_t signature = scene_hash(scene);
    size_t length = strlen(output_path) + 40;
    cache->directory = sr_alloc(length);
    if (!cache->directory) return SR_ERR_MEMORY;
    snprintf(cache->directory, length, "%s.resume/%016llx", output_path,
             (unsigned long long)signature);
    SrStatus status = make_dirs(cache->directory, diag);
    if (status != SR_OK) { sr_resume_close(cache); return status; }
    cache->frame_bytes = pixels * 4;
    cache->enabled = true;
    char manifest[4096];
    snprintf(manifest, sizeof(manifest), "%s/manifest.txt", cache->directory);
    FILE *file = fopen(manifest, "wb");
    if (!file) { sr_resume_close(cache); return SR_ERR_IO; }
    fprintf(file, "scene-render=%s\nsignature=%016llx\nsize=%ux%u\n",
            SR_VERSION, (unsigned long long)signature, scene->project.width,
            scene->project.height);
    if (fclose(file) != 0) { sr_resume_close(cache); return SR_ERR_IO; }
    sr_diag_info(diag, "resume cache: %s", cache->directory);
    return SR_OK;
}

static bool frame_path(const SrResumeCache *cache, uint64_t frame, char *path,
                       size_t capacity) {
    int count = snprintf(path, capacity, "%s/%012llu.rgba", cache->directory,
                         (unsigned long long)frame);
    return count >= 0 && (size_t)count < capacity;
}

bool sr_resume_load(const SrResumeCache *cache, uint64_t frame,
                    uint8_t *rgba, SrDiagnostics *diag) {
    if (!cache->enabled) return false;
    char path[4096];
    if (!frame_path(cache, frame, path, sizeof(path))) return false;
    struct stat info;
    if (stat(path, &info) != 0 || info.st_size < 0 ||
        (uint64_t)info.st_size != cache->frame_bytes) return false;
    FILE *file = fopen(path, "rb");
    if (!file) return false;
    bool ok = fread(rgba, 1, cache->frame_bytes, file) == cache->frame_bytes &&
              fclose(file) == 0;
    if (ok) sr_diag_info(diag, "reused cached frame %llu",
                         (unsigned long long)frame);
    return ok;
}

SrStatus sr_resume_store(const SrResumeCache *cache, uint64_t frame,
                         const uint8_t *rgba, SrDiagnostics *diag) {
    if (!cache->enabled) return SR_OK;
    char path[4096], temporary[4096];
    if (!frame_path(cache, frame, path, sizeof(path))) return SR_ERR_IO;
    int count = snprintf(temporary, sizeof(temporary), "%s.tmp.%ld", path,
                         (long)getpid());
    if (count < 0 || (size_t)count >= sizeof(temporary)) return SR_ERR_IO;
    FILE *file = fopen(temporary, "wb");
    if (!file) return SR_ERR_IO;
    bool ok = fwrite(rgba, 1, cache->frame_bytes, file) == cache->frame_bytes &&
              fclose(file) == 0 && rename(temporary, path) == 0;
    if (!ok) {
        unlink(temporary);
        sr_diag_error(diag, 0, NULL, NULL, "cannot store resume frame %llu",
                      (unsigned long long)frame);
        return SR_ERR_IO;
    }
    return SR_OK;
}

void sr_resume_close(SrResumeCache *cache) {
    if (!cache) return;
    free(cache->directory);
    *cache = (SrResumeCache){0};
}
