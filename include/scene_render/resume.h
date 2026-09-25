#ifndef SCENE_RENDER_RESUME_H
#define SCENE_RENDER_RESUME_H

#include "scene_render/diagnostics.h"
#include "scene_render/scene.h"

/* Segmented resume (--resume). The frame range [first, end) is split into
 * segments of segment_frames frames; segment k is an independently encoded,
 * video-only file OUTPUT.parts/seg-KKKKKK.<ext> (mkv for FFV1, mp4
 * otherwise) with its own leading keyframe. A segment is written as
 * seg-KKKKKK.part.<ext> and committed by an atomic rename, so the final
 * name exists only for complete segments.
 *
 * OUTPUT.parts/manifest records everything segment content depends on:
 * SR_VERSION, the FNV-1a hash of the scene file's bytes, every asset
 * file's size, mtime and hash of its first MiB, the frame range, the
 * segment size, the effective project/output video settings (CLI
 * overrides included), the encoder thread count and renderer backend. Any
 * difference discards every old segment. Audio settings are not part of
 * it: segments are video-only and audio is encoded once at assembly. */
typedef struct {
    char *directory;            /* OUTPUT.parts */
    const char *extension;      /* "mp4" or "mkv" */
    uint64_t first, end;
    uint32_t segment_frames;
    uint64_t segment_count;
} SrResume;

typedef struct {
    unsigned threads;           /* encoder/render threads as requested */
    bool gpu;                   /* --renderer gpu */
    unsigned bits;              /* bits per component fed to the encoder */
} SrResumeSettings;

/* Creates OUTPUT.parts, compares its manifest with the current one, and
 * discards all old segments (diagnostic info) when they differ; stale
 * *.part.* files are always removed. */
SrStatus sr_resume_prepare(SrResume *resume, const SrScene *scene,
                           const char *output_path, uint64_t first,
                           uint64_t end, uint32_t segment_frames,
                           const SrResumeSettings *settings,
                           SrDiagnostics *diag);
/* Frames [*from, *to) of segment k. */
void sr_resume_segment_range(const SrResume *resume, uint64_t k,
                             uint64_t *from, uint64_t *to);
/* Allocated path of segment k: the committed name, or the in-progress
 * .part name. NULL when out of memory. */
char *sr_resume_segment_path(const SrResume *resume, uint64_t k, bool partial);
/* True when segment k was committed (by this or an earlier run). */
bool sr_resume_segment_done(const SrResume *resume, uint64_t k);
/* Atomically renames segment k's .part file to its committed name. */
SrStatus sr_resume_commit(const SrResume *resume, uint64_t k,
                          SrDiagnostics *diag);
/* Deletes every segment, the manifest and the directory. */
SrStatus sr_resume_remove(const SrResume *resume, SrDiagnostics *diag);
void sr_resume_close(SrResume *resume);

#endif
