#define _POSIX_C_SOURCE 200809L
#include "scene_render/resume.h"
#include "scene_render/text.h"

#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/file.h>
#include <sys/stat.h>
#include <unistd.h>

#define MANIFEST_LIMIT ((size_t)1 << 20)
#define CREATE_FLAGS (O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC)

/* ------------------------------------------------------------ inputs */

static bool same_identity(const struct stat *a, const struct stat *b) {
    return a->st_size == b->st_size && a->st_dev == b->st_dev &&
           a->st_ino == b->st_ino && a->st_mtim.tv_sec == b->st_mtim.tv_sec &&
           a->st_mtim.tv_nsec == b->st_mtim.tv_nsec;
}

static void record_stat(SrResumeFile *file, const struct stat *info) {
    file->size = (long long)info->st_size;
    file->mtime_sec = (long long)info->st_mtim.tv_sec;
    file->mtime_nsec = (long)info->st_mtim.tv_nsec;
    file->device = (unsigned long long)info->st_dev;
    file->inode = (unsigned long long)info->st_ino;
}

/* Hashes the whole file through one descriptor; the stat before and after
 * reading must agree, else the content was changing under us. */
static SrStatus fingerprint(SrResumeFile *file, SrDiagnostics *diag) {
    int fd = open(file->path, O_RDONLY | O_CLOEXEC);
    if (fd < 0) {
        file->present = false;   /* the loader reports a missing asset */
        return SR_OK;
    }
    struct stat before, after;
    SrStatus status = SR_OK;
    uint64_t hash = SR_FNV_OFFSET;
    if (fstat(fd, &before) != 0 || !S_ISREG(before.st_mode)) {
        sr_diag_error(diag, 0, NULL, NULL, "%s '%s' is not a regular file",
                      file->key, file->label);
        status = SR_ERR_ASSET;
    }
    unsigned char buffer[1 << 16];
    for (;;) {
        if (status != SR_OK) break;
        ssize_t count = read(fd, buffer, sizeof(buffer));
        if (count < 0 && errno == EINTR) continue;
        if (count < 0) {
            sr_diag_error(diag, 0, NULL, NULL, "cannot read %s '%s': %s",
                          file->key, file->label, strerror(errno));
            status = SR_ERR_IO;
            break;
        }
        if (count == 0) break;
        hash = sr_fnv1a64(hash, buffer, (size_t)count);
    }
    if (status == SR_OK &&
        (fstat(fd, &after) != 0 || !same_identity(&before, &after))) {
        sr_diag_error(diag, 0, NULL, NULL, "%s '%s' changed while it was hashed",
                      file->key, file->label);
        status = SR_ERR_ASSET;
    }
    close(fd);
    if (status != SR_OK) return status;
    file->present = true;
    file->hash = hash;
    record_stat(file, &before);
    return SR_OK;
}

static SrStatus add_input(SrResumeInputs *inputs, const char *key,
                          const char *label, char *path, SrDiagnostics *diag) {
    if (!path) return SR_ERR_MEMORY;
    for (size_t i = 0; i < inputs->count; ++i)
        if (strcmp(inputs->files[i].path, path) == 0) {
            free(path);
            return SR_OK;
        }
    if (inputs->count == inputs->capacity) {
        size_t capacity = inputs->capacity ? inputs->capacity * 2 : 8;
        SrResumeFile *files = capacity > SIZE_MAX / sizeof(*files)
            ? NULL : sr_realloc(inputs->files, capacity * sizeof(*files));
        if (!files) {
            free(path);
            return SR_ERR_MEMORY;
        }
        inputs->files = files;
        inputs->capacity = capacity;
    }
    SrResumeFile *file = &inputs->files[inputs->count];
    *file = (SrResumeFile){.path = path, .key = sr_strdup(key),
                           .label = sr_strdup(label)};
    ++inputs->count;
    if (!file->key || !file->label) return SR_ERR_MEMORY;
    return fingerprint(file, diag);
}

SrStatus sr_resume_inputs_capture(SrResumeInputs *inputs, const SrScene *scene,
                                  SrDiagnostics *diag) {
    *inputs = (SrResumeInputs){0};
    SrStatus status = SR_OK;
    for (size_t i = 0; status == SR_OK && i < scene->asset_count; ++i) {
        const SrAsset *asset = &scene->assets[i];
        if (asset->source)
            status = add_input(inputs, "asset", asset->source,
                               sr_path_join(scene->base_dir, asset->source), diag);
        if (status == SR_OK && asset->font_file)
            status = add_input(inputs, "font", asset->font_file,
                               sr_path_join(scene->base_dir, asset->font_file),
                               diag);
    }
    return status;
}

SrStatus sr_resume_inputs_add_fonts(SrResumeInputs *inputs,
                                    const SrScene *scene, SrDiagnostics *diag) {
    SrStatus status = SR_OK;
    size_t count = sr_font_cache_count(scene->font_cache);
    for (size_t i = 0; status == SR_OK && i < count; ++i) {
        const char *path = sr_font_cache_path(scene->font_cache, i);
        status = add_input(inputs, "font", path, sr_strdup(path), diag);
    }
    return status;
}

SrStatus sr_resume_inputs_verify(const SrResumeInputs *inputs, const char *when,
                                 SrDiagnostics *diag) {
    for (size_t i = 0; inputs && i < inputs->count; ++i) {
        const SrResumeFile *file = &inputs->files[i];
        struct stat info;
        bool present = stat(file->path, &info) == 0;
        SrResumeFile now = *file;
        if (present) record_stat(&now, &info);
        if (present != file->present ||
            (present && (now.size != file->size || now.mtime_sec != file->mtime_sec ||
                         now.mtime_nsec != file->mtime_nsec ||
                         now.device != file->device || now.inode != file->inode))) {
            sr_diag_error(diag, 0, NULL, NULL, "%s '%s' changed while %s",
                          file->key, file->label, when);
            return SR_ERR_ASSET;
        }
    }
    return SR_OK;
}

void sr_resume_inputs_free(SrResumeInputs *inputs) {
    if (!inputs) return;
    for (size_t i = 0; i < inputs->count; ++i) {
        free(inputs->files[i].key);
        free(inputs->files[i].label);
        free(inputs->files[i].path);
    }
    free(inputs->files);
    *inputs = (SrResumeInputs){0};
}

/* ---------------------------------------------------------- manifest */

/* The manifest text; NULL when out of memory. */
static char *build_manifest(const SrScene *scene, const SrResume *resume,
                            const SrResumeInputs *inputs,
                            const SrResumeSettings *settings) {
    char *text = NULL;
    size_t size = 0;
    FILE *out = open_memstream(&text, &size);
    if (!out) return NULL;
    const SrProject *p = &scene->project;
    const SrOutput *o = &scene->output;
    fprintf(out, "scene-render-resume 2\nversion=%s\nscene=%016llx\n", SR_VERSION,
            (unsigned long long)scene->source_hash);
    for (size_t i = 0; inputs && i < inputs->count; ++i) {
        const SrResumeFile *f = &inputs->files[i];
        if (!f->present)
            fprintf(out, "%s=%s missing\n", f->key, f->label);
        else
            fprintf(out, "%s=%s size=%lld mtime=%lld.%09ld hash=%016llx\n", f->key,
                    f->label, f->size, f->mtime_sec, f->mtime_nsec,
                    (unsigned long long)f->hash);
    }
    fprintf(out, "range=%llu:%llu\nsegment_frames=%u\n",
            (unsigned long long)resume->first, (unsigned long long)resume->end,
            resume->segment_frames);
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
    fprintf(out, "threads=%u bits=%u backend=%s\n", settings->threads,
            settings->bits, settings->backend ? settings->backend : "cpu");
    bool failed = ferror(out) != 0;
    if (fclose(out) != 0 || failed) {
        free(text);
        return NULL;
    }
    return text;
}

/* The manifest in the directory; NULL when absent, not a regular file,
 * larger than MANIFEST_LIMIT or unreadable (all mean "stale"). *oom is set
 * when reading failed for lack of memory. */
static char *read_manifest(const SrResume *resume, bool *oom) {
    *oom = false;
    int fd = openat(resume->dirfd, "manifest", O_RDONLY | O_NOFOLLOW | O_CLOEXEC);
    if (fd < 0) return NULL;
    struct stat info;
    if (fstat(fd, &info) != 0 || !S_ISREG(info.st_mode) ||
        (unsigned long long)info.st_size > MANIFEST_LIMIT) {
        close(fd);
        return NULL;
    }
    char *text = sr_alloc(MANIFEST_LIMIT + 2);
    if (!text) {
        *oom = true;
        close(fd);
        return NULL;
    }
    size_t used = 0;
    bool ok = true;
    while (used <= MANIFEST_LIMIT) {
        ssize_t count = read(fd, text + used, MANIFEST_LIMIT + 1 - used);
        if (count < 0 && errno == EINTR) continue;
        if (count <= 0) {
            ok = count == 0;
            break;
        }
        used += (size_t)count;
    }
    close(fd);
    if (!ok || used > MANIFEST_LIMIT) {
        free(text);
        return NULL;
    }
    text[used] = '\0';
    return text;
}

static SrStatus sync_directory(const SrResume *resume, SrDiagnostics *diag) {
    if (fsync(resume->dirfd) != 0) {
        sr_diag_error(diag, 0, NULL, NULL, "cannot sync '%s': %s",
                      resume->directory, strerror(errno));
        return SR_ERR_IO;
    }
    return SR_OK;
}

/* Creates a unique "<prefix>.<pid>.<n><suffix>" in the directory with
 * O_EXCL|O_NOFOLLOW; returns the descriptor (name in `name`) or -1. */
static int create_unique(SrResume *resume, const char *prefix, const char *suffix,
                         char *name, size_t size) {
    for (int attempt = 0; attempt < 1000; ++attempt) {
        snprintf(name, size, "%s.%ld.%lu%s", prefix, (long)getpid(),
                 resume->serial++, suffix);
        int fd = openat(resume->dirfd, name, CREATE_FLAGS, 0644);
        if (fd >= 0 || errno != EEXIST) return fd;
    }
    errno = EEXIST;
    return -1;
}

static SrStatus write_manifest(SrResume *resume, const char *text,
                               SrDiagnostics *diag) {
    char name[96];
    int fd = create_unique(resume, "manifest.tmp", "", name, sizeof(name));
    bool ok = fd >= 0;
    size_t length = strlen(text), done = 0;
    while (ok && done < length) {
        ssize_t count = write(fd, text + done, length - done);
        if (count < 0 && errno == EINTR) continue;
        if (count <= 0) ok = false;
        else done += (size_t)count;
    }
    if (ok && fsync(fd) != 0) ok = false;
    if (fd >= 0 && close(fd) != 0) ok = false;
    if (ok && renameat(resume->dirfd, name, resume->dirfd, "manifest") != 0)
        ok = false;
    if (!ok) {
        sr_diag_error(diag, 0, NULL, NULL,
                      "cannot write resume manifest in '%s': %s",
                      resume->directory, strerror(errno));
        if (fd >= 0) unlinkat(resume->dirfd, name, 0);
        return SR_ERR_IO;
    }
    return sync_directory(resume, diag);
}

/* ------------------------------------------------------ name patterns */

/* "seg-" followed by at least one digit; returns the rest or NULL. */
static const char *after_segment_number(const char *name) {
    if (strncmp(name, "seg-", 4) != 0) return NULL;
    const char *at = name + 4;
    if (*at < '0' || *at > '9') return NULL;
    while (*at >= '0' && *at <= '9') ++at;
    return at;
}

static bool is_committed(const char *name) {
    const char *rest = after_segment_number(name);
    return rest && (strcmp(rest, ".mp4") == 0 || strcmp(rest, ".mkv") == 0);
}

static bool is_partial(const char *name) {
    const char *rest = after_segment_number(name);
    return rest && strncmp(rest, ".part.", 6) == 0;
}

static bool is_manifest_temp(const char *name) {
    return strncmp(name, "manifest.tmp", 12) == 0;
}

static bool is_stale(const char *name) {
    return is_partial(name) || is_manifest_temp(name);
}

static bool is_resume_file(const char *name) {
    return is_committed(name) || is_stale(name) || strcmp(name, "manifest") == 0 ||
           strcmp(name, "lock") == 0;
}

/* Unlinks (never following links) every entry accepted by `match`; counts
 * removed committed segments into *removed when non-NULL. */
static SrStatus remove_matching(const SrResume *resume, bool (*match)(const char *),
                                size_t *removed, SrDiagnostics *diag) {
    int fd = dup(resume->dirfd);
    DIR *dir = fd >= 0 ? fdopendir(fd) : NULL;
    if (!dir) {
        sr_diag_error(diag, 0, NULL, NULL, "cannot list '%s': %s",
                      resume->directory, strerror(errno));
        if (fd >= 0) close(fd);
        return SR_ERR_IO;
    }
    rewinddir(dir);
    SrStatus status = SR_OK;
    struct dirent *entry;
    while ((entry = readdir(dir)) != NULL) {
        if (!match(entry->d_name)) continue;
        if (unlinkat(resume->dirfd, entry->d_name, 0) != 0 && errno != ENOENT) {
            sr_diag_error(diag, 0, NULL, NULL, "cannot remove '%s/%s': %s",
                          resume->directory, entry->d_name, strerror(errno));
            status = SR_ERR_IO;
        } else if (removed && is_committed(entry->d_name)) {
            ++*removed;
        }
    }
    closedir(dir);
    return status;
}

/* -------------------------------------------------------- directory */

SrStatus sr_resume_open(SrResume *resume, const SrScene *scene,
                        const char *output_path, uint64_t first, uint64_t end,
                        uint32_t segment_frames, SrDiagnostics *diag) {
    *resume = (SrResume){.dirfd = -1, .lockfd = -1};
    if (!scene || !output_path || !*output_path || !segment_frames || first >= end)
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
    const char *dir = resume->directory;
    if (mkdir(dir, 0755) != 0 && errno != EEXIST) {
        sr_diag_error(diag, 0, NULL, NULL, "cannot create '%s': %s", dir,
                      strerror(errno));
        sr_resume_close(resume);
        return SR_ERR_IO;
    }
    struct stat link_info, info;
    SrStatus status = SR_OK;
    if (lstat(dir, &link_info) != 0) {
        sr_diag_error(diag, 0, NULL, NULL, "cannot inspect '%s': %s", dir,
                      strerror(errno));
        status = SR_ERR_IO;
    } else if (S_ISLNK(link_info.st_mode)) {
        sr_diag_error(diag, 0, NULL, NULL,
                      "'%s' is a symbolic link; refusing to use it for --resume", dir);
        status = SR_ERR_IO;
    } else if (!S_ISDIR(link_info.st_mode) || link_info.st_uid != geteuid()) {
        sr_diag_error(diag, 0, NULL, NULL,
                      "'%s' is not a directory owned by the current user", dir);
        status = SR_ERR_IO;
    }
    if (status == SR_OK) {
        resume->dirfd = open(dir, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC);
        /* The descriptor must be the directory just inspected. */
        if (resume->dirfd < 0 || fstat(resume->dirfd, &info) != 0 ||
            info.st_dev != link_info.st_dev || info.st_ino != link_info.st_ino) {
            sr_diag_error(diag, 0, NULL, NULL, "cannot open '%s': %s", dir,
                          resume->dirfd < 0 ? strerror(errno)
                                            : "it was replaced while opening");
            status = SR_ERR_IO;
        }
    }
    if (status == SR_OK) {
        resume->lockfd = openat(resume->dirfd, "lock",
                                O_RDWR | O_CREAT | O_NOFOLLOW | O_CLOEXEC, 0644);
        if (resume->lockfd < 0 || fstat(resume->lockfd, &info) != 0 ||
            !S_ISREG(info.st_mode)) {
            sr_diag_error(diag, 0, NULL, NULL, "cannot open '%s/lock': %s", dir,
                          resume->lockfd < 0 ? strerror(errno) : "not a regular file");
            status = SR_ERR_IO;
        } else if (flock(resume->lockfd, LOCK_EX | LOCK_NB) != 0) {
            if (errno == EWOULDBLOCK)
                sr_diag_error(diag, 0, NULL, NULL,
                              "another render is using '%s'", dir);
            else
                sr_diag_error(diag, 0, NULL, NULL, "cannot lock '%s/lock': %s",
                              dir, strerror(errno));
            status = SR_ERR_IO;
        }
    }
    if (status != SR_OK) sr_resume_close(resume);
    return status;
}

SrStatus sr_resume_sync(SrResume *resume, const SrScene *scene,
                        const SrResumeInputs *inputs,
                        const SrResumeSettings *settings, SrDiagnostics *diag) {
    if (!resume || resume->dirfd < 0 || !scene || !settings) return SR_ERR_ARGUMENT;
    char *manifest = build_manifest(scene, resume, inputs, settings);
    if (!manifest) return SR_ERR_MEMORY;
    SrStatus status = remove_matching(resume, is_stale, NULL, diag);
    bool oom = false;
    char *old = status == SR_OK ? read_manifest(resume, &oom) : NULL;
    if (oom) status = SR_ERR_MEMORY;
    if (status == SR_OK && (!old || strcmp(old, manifest) != 0)) {
        size_t removed = 0;
        status = remove_matching(resume, is_committed, &removed, diag);
        if (status == SR_OK && (old || removed))
            sr_diag_info(diag, "resume manifest '%s/manifest' changed; discarded "
                         "%zu old segment(s)", resume->directory, removed);
        if (status == SR_OK) status = write_manifest(resume, manifest, diag);
    }
    if (status == SR_OK)
        sr_diag_info(diag, "resume: %llu segment(s) of %u frames in %s",
                     (unsigned long long)resume->segment_count,
                     resume->segment_frames, resume->directory);
    free(old);
    free(manifest);
    return status;
}

SrStatus sr_resume_prepare(SrResume *resume, const SrScene *scene,
                           const char *output_path, uint64_t first,
                           uint64_t end, uint32_t segment_frames,
                           const SrResumeInputs *inputs,
                           const SrResumeSettings *settings,
                           SrDiagnostics *diag) {
    SrStatus status = sr_resume_open(resume, scene, output_path, first, end,
                                     segment_frames, diag);
    if (status == SR_OK) {
        status = sr_resume_sync(resume, scene, inputs, settings, diag);
        if (status != SR_OK) sr_resume_close(resume);
    }
    return status;
}

/* --------------------------------------------------------- segments */

void sr_resume_segment_range(const SrResume *resume, uint64_t k, uint64_t *from,
                             uint64_t *to) {
    *from = resume->first + k * resume->segment_frames;
    *to = *from + resume->segment_frames < resume->end
              ? *from + resume->segment_frames : resume->end;
}

static void segment_name(const SrResume *resume, uint64_t k, char *name,
                         size_t size) {
    snprintf(name, size, "seg-%06llu.%s", (unsigned long long)k, resume->extension);
}

static char *join(const char *directory, const char *name) {
    size_t length = strlen(directory) + strlen(name) + 2;
    char *path = sr_alloc(length);
    if (path) snprintf(path, length, "%s/%s", directory, name);
    return path;
}

char *sr_resume_segment_path(const SrResume *resume, uint64_t k) {
    char name[64];
    segment_name(resume, k, name, sizeof(name));
    return join(resume->directory, name);
}

bool sr_resume_segment_done(const SrResume *resume, uint64_t k) {
    char name[64];
    segment_name(resume, k, name, sizeof(name));
    struct stat info;
    return fstatat(resume->dirfd, name, &info, AT_SYMLINK_NOFOLLOW) == 0 &&
           S_ISREG(info.st_mode);
}

SrStatus sr_resume_segment_discard(const SrResume *resume, uint64_t k,
                                   SrDiagnostics *diag) {
    char name[64];
    segment_name(resume, k, name, sizeof(name));
    if (unlinkat(resume->dirfd, name, 0) != 0 && errno != ENOENT) {
        sr_diag_error(diag, 0, NULL, NULL, "cannot remove '%s/%s': %s",
                      resume->directory, name, strerror(errno));
        return SR_ERR_IO;
    }
    return SR_OK;
}

SrStatus sr_resume_segment_begin(SrResume *resume, uint64_t k, char **path,
                                 SrDiagnostics *diag) {
    *path = NULL;
    char prefix[48], suffix[8], name[128];
    snprintf(prefix, sizeof(prefix), "seg-%06llu.part", (unsigned long long)k);
    snprintf(suffix, sizeof(suffix), ".%s", resume->extension);
    int fd = create_unique(resume, prefix, suffix, name, sizeof(name));
    if (fd < 0) {
        sr_diag_error(diag, 0, NULL, NULL, "cannot create a segment in '%s': %s",
                      resume->directory, strerror(errno));
        return SR_ERR_IO;
    }
    close(fd);
    *path = join(resume->directory, name);
    if (!*path) {
        unlinkat(resume->dirfd, name, 0);
        return SR_ERR_MEMORY;
    }
    return SR_OK;
}

static const char *base_name(const char *path) {
    const char *slash = strrchr(path, '/');
    return slash ? slash + 1 : path;
}

void sr_resume_segment_abandon(const SrResume *resume, const char *path) {
    if (resume && path) unlinkat(resume->dirfd, base_name(path), 0);
}

SrStatus sr_resume_segment_commit(const SrResume *resume, uint64_t k,
                                  const char *path, SrDiagnostics *diag) {
    const char *temporary = base_name(path);
    char name[64];
    segment_name(resume, k, name, sizeof(name));
    int fd = openat(resume->dirfd, temporary, O_RDONLY | O_NOFOLLOW | O_CLOEXEC);
    struct stat info;
    bool ok = fd >= 0 && fstat(fd, &info) == 0 && S_ISREG(info.st_mode) &&
              fsync(fd) == 0;
    if (fd >= 0) close(fd);
    if (ok) ok = renameat(resume->dirfd, temporary, resume->dirfd, name) == 0;
    if (!ok) {
        sr_diag_error(diag, 0, NULL, NULL, "cannot commit segment '%s/%s': %s",
                      resume->directory, name, strerror(errno));
        unlinkat(resume->dirfd, temporary, 0);
        return SR_ERR_IO;
    }
    return sync_directory(resume, diag);
}

SrStatus sr_resume_remove(SrResume *resume, SrDiagnostics *diag) {
    SrStatus status = remove_matching(resume, is_resume_file, NULL, diag);
    /* The lock goes last and is released by close: its name is gone, so a
     * newcomer creates a fresh one. */
    if (resume->lockfd >= 0) close(resume->lockfd);
    if (resume->dirfd >= 0) close(resume->dirfd);
    resume->lockfd = resume->dirfd = -1;
    if (status == SR_OK && rmdir(resume->directory) != 0) {
        /* Something foreign lives there: keep it, the render succeeded. */
        sr_diag_warning(diag, 0, NULL, NULL, "cannot remove '%s': %s",
                        resume->directory, strerror(errno));
    }
    return status;
}

void sr_resume_close(SrResume *resume) {
    if (!resume) return;
    if (resume->lockfd >= 0) close(resume->lockfd);
    if (resume->dirfd >= 0) close(resume->dirfd);
    free(resume->directory);
    *resume = (SrResume){.dirfd = -1, .lockfd = -1};
}

/* ----------------------------------------------------- final output */

static char *parent_of(const char *path) {
    const char *slash = strrchr(path, '/');
    if (!slash) return sr_strdup(".");
    if (slash == path) return sr_strdup("/");
    size_t length = (size_t)(slash - path);
    char *parent = sr_alloc(length + 1);
    if (parent) memcpy(parent, path, length);
    return parent;
}

SrStatus sr_output_temp_create(const char *output_path, char **temporary,
                               SrDiagnostics *diag) {
    *temporary = NULL;
    if (!output_path || !*output_path) return SR_ERR_ARGUMENT;
    const char *base = base_name(output_path);
    const char *dot = strrchr(base, '.');
    const char *extension = dot ? dot : "";
    size_t length = strlen(output_path) + strlen(extension) + 64;
    static unsigned long serial;
    for (int attempt = 0; attempt < 1000; ++attempt) {
        char *path = sr_alloc(length);
        if (!path) return SR_ERR_MEMORY;
        /* Same directory (rename stays atomic), same extension (the
         * container is chosen from it). */
        snprintf(path, length, "%.*s.%s.%ld.%lu.tmp%s", (int)(base - output_path),
                 output_path, base, (long)getpid(), serial++, extension);
        int fd = open(path, CREATE_FLAGS, 0644);
        if (fd >= 0) {
            close(fd);
            *temporary = path;
            return SR_OK;
        }
        int error = errno;
        free(path);
        if (error != EEXIST) {
            sr_diag_error(diag, 0, NULL, NULL,
                          "cannot create a temporary file next to '%s': %s",
                          output_path, strerror(error));
            return SR_ERR_IO;
        }
    }
    sr_diag_error(diag, 0, NULL, NULL,
                  "cannot create a temporary file next to '%s'", output_path);
    return SR_ERR_IO;
}

SrStatus sr_output_temp_commit(const char *temporary, const char *output_path,
                               SrDiagnostics *diag) {
    int fd = open(temporary, O_RDONLY | O_NOFOLLOW | O_CLOEXEC);
    bool ok = fd >= 0 && fsync(fd) == 0;
    if (fd >= 0) close(fd);
    if (ok) ok = rename(temporary, output_path) == 0;
    if (!ok) {
        sr_diag_error(diag, 0, NULL, NULL, "cannot replace '%s': %s", output_path,
                      strerror(errno));
        unlink(temporary);
        return SR_ERR_IO;
    }
    char *parent = parent_of(output_path);
    if (!parent) return SR_ERR_MEMORY;
    int dirfd = open(parent, O_RDONLY | O_DIRECTORY | O_CLOEXEC);
    ok = dirfd >= 0 && fsync(dirfd) == 0;
    if (dirfd >= 0) close(dirfd);
    SrStatus status = SR_OK;
    if (!ok) {
        sr_diag_error(diag, 0, NULL, NULL, "cannot sync '%s': %s", parent,
                      strerror(errno));
        status = SR_ERR_IO;
    }
    free(parent);
    return status;
}
