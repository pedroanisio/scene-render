#ifndef SCENE_RENDER_RESUME_H
#define SCENE_RENDER_RESUME_H

#include "scene_render/diagnostics.h"
#include "scene_render/scene.h"

/* Segmented resume (--resume). The frame range [first, end) is split into
 * segments of segment_frames frames; segment k is an independently encoded,
 * video-only file OUTPUT.parts/seg-KKKKKK.<ext> (mkv for FFV1, mp4
 * otherwise) with its own leading keyframe. A segment is written under a
 * unique temporary name seg-KKKKKK.part.<pid>.<n>.<ext>, fsynced, and
 * committed by an atomic rename (directory fsynced after), so the final
 * name exists only for complete segments.
 *
 * Filesystem safety: OUTPUT.parts must be a real directory (not a symbolic
 * link) owned by the current user; every access goes through a descriptor
 * of it (openat/renameat/unlinkat), new files are created with
 * O_CREAT|O_EXCL|O_NOFOLLOW, and cleanup unlinks only names matching the
 * segment/manifest/lock patterns, never following links. The run holds an
 * exclusive flock on OUTPUT.parts/lock from open to close; a second render
 * on the same OUTPUT fails with SR_ERR_IO.
 *
 * OUTPUT.parts/manifest (at most 1 MiB; a larger one is stale) records
 * everything segment content depends on: SR_VERSION, the FNV-1a 64 hash of
 * the scene bytes the loader parsed, every input file's size, mtime and
 * full-content hash (assets, explicit font files and the font files that
 * font families resolved to), the frame range, the segment size, the
 * effective project/output video settings (CLI overrides included), the
 * resolved worker thread count, bit depth and the colour-conversion
 * backend actually used. Any difference discards every old segment. Audio
 * settings are not part of it: segments are video-only and audio is
 * encoded once at assembly. */
typedef struct {
    char *directory;            /* OUTPUT.parts */
    int dirfd;                  /* descriptor of the verified directory */
    int lockfd;                 /* OUTPUT.parts/lock, flocked */
    const char *extension;      /* "mp4" or "mkv" */
    uint64_t first, end;
    uint32_t segment_frames;
    uint64_t segment_count;
    unsigned long serial;       /* unique temporary names */
} SrResume;

typedef struct {
    unsigned threads;           /* resolved render/encoder worker count */
    unsigned bits;              /* bits per component fed to the encoder */
    const char *backend;        /* colour conversion: "cpu" or "opencl:<dev>" */
} SrResumeSettings;

/* One input file's identity: stat fields plus the FNV-1a 64 of its whole
 * content, read through one descriptor whose stat did not change while it
 * was read. */
typedef struct {
    char *key;                  /* manifest key: "asset", "font" */
    char *label;                /* name written to the manifest */
    char *path;                 /* path that is opened */
    bool present;
    uint64_t hash;
    long long size;
    long long mtime_sec;
    long mtime_nsec;
    unsigned long long device, inode;
} SrResumeFile;

typedef struct {
    SrResumeFile *files;
    size_t count, capacity;
} SrResumeInputs;

/* Fingerprints every asset source and explicit font file of the scene.
 * Called before the assets are loaded; sr_resume_inputs_verify after
 * loading proves the loaded bytes are the fingerprinted ones. */
SrStatus sr_resume_inputs_capture(SrResumeInputs *inputs, const SrScene *scene,
                                  SrDiagnostics *diag);
/* Adds the font files font families resolved to while the text assets
 * were rendered (scene->font_cache), when not already listed. */
SrStatus sr_resume_inputs_add_fonts(SrResumeInputs *inputs,
                                    const SrScene *scene, SrDiagnostics *diag);
/* Re-stats every input (size, mtime ns, device, inode); any change is
 * SR_ERR_ASSET "<key> '<label>' changed while <when>". */
SrStatus sr_resume_inputs_verify(const SrResumeInputs *inputs, const char *when,
                                 SrDiagnostics *diag);
void sr_resume_inputs_free(SrResumeInputs *inputs);

/* Creates or opens OUTPUT.parts, checks it (real directory, owned by the
 * current user) and takes its lock. */
SrStatus sr_resume_open(SrResume *resume, const SrScene *scene,
                        const char *output_path, uint64_t first, uint64_t end,
                        uint32_t segment_frames, SrDiagnostics *diag);
/* Removes stale temporary files, compares the manifest with the current
 * one and, when they differ, discards all old segments (diagnostic info)
 * and writes the new manifest. May be called again during a run (e.g.
 * after the backend changed), which discards everything rendered so far. */
SrStatus sr_resume_sync(SrResume *resume, const SrScene *scene,
                        const SrResumeInputs *inputs,
                        const SrResumeSettings *settings, SrDiagnostics *diag);
/* sr_resume_open followed by sr_resume_sync. */
SrStatus sr_resume_prepare(SrResume *resume, const SrScene *scene,
                           const char *output_path, uint64_t first,
                           uint64_t end, uint32_t segment_frames,
                           const SrResumeInputs *inputs,
                           const SrResumeSettings *settings,
                           SrDiagnostics *diag);
/* Frames [*from, *to) of segment k. */
void sr_resume_segment_range(const SrResume *resume, uint64_t k,
                             uint64_t *from, uint64_t *to);
/* Allocated path of committed segment k. NULL when out of memory. */
char *sr_resume_segment_path(const SrResume *resume, uint64_t k);
/* True when committed segment k exists as a regular file (not a link). */
bool sr_resume_segment_done(const SrResume *resume, uint64_t k);
/* Deletes committed segment k (it failed validation). */
SrStatus sr_resume_segment_discard(const SrResume *resume, uint64_t k,
                                   SrDiagnostics *diag);
/* Creates segment k's unique temporary file (O_EXCL|O_NOFOLLOW) and
 * returns its allocated path in *path. */
SrStatus sr_resume_segment_begin(SrResume *resume, uint64_t k, char **path,
                                 SrDiagnostics *diag);
/* fsyncs the temporary file `path` from sr_resume_segment_begin, renames it
 * to segment k's committed name and fsyncs the directory. */
SrStatus sr_resume_segment_commit(const SrResume *resume, uint64_t k,
                                  const char *path, SrDiagnostics *diag);
/* Unlinks a temporary file from sr_resume_segment_begin. */
void sr_resume_segment_abandon(const SrResume *resume, const char *path);
/* Deletes every segment, the manifest, the lock and the directory. */
SrStatus sr_resume_remove(SrResume *resume, SrDiagnostics *diag);
/* Releases the lock and the descriptors. */
void sr_resume_close(SrResume *resume);

/* Atomic replacement of a final output: creates a unique temporary file
 * (O_EXCL|O_NOFOLLOW) next to `output_path` with the same extension, and
 * returns its path in *temporary. */
SrStatus sr_output_temp_create(const char *output_path, char **temporary,
                               SrDiagnostics *diag);
/* fsyncs `temporary`, renames it over `output_path` and fsyncs the
 * directory; on failure the temporary file is removed. */
SrStatus sr_output_temp_commit(const char *temporary, const char *output_path,
                               SrDiagnostics *diag);

#endif
