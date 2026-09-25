#ifndef SCENE_RENDER_VIDEO_H
#define SCENE_RENDER_VIDEO_H

#include "scene_render/scene.h"

/* In-process libavformat/libavcodec decoding of video and still images.
 *
 * A video source keeps one demuxer and decoder open for the lifetime of the
 * asset. Frames are addressed by index at the DECLARED frame rate on the
 * file's timeline: time zero is the container's start time (the earliest
 * stream start, with declared audio codec priming before zero excluded),
 * the same origin sr_audio_decode_file uses for the file's soundtrack.
 * Output index i shows the LATEST frame whose presentation time is at most
 * i / fps (plus 1e-6 of a frame, or half a stream tick when timestamps are
 * coarser, e.g. Matroska milliseconds), never a later frame; indices before
 * the first frame show the first frame, indices past the last frame show
 * the last one. Sequential requests decode forward; backward jumps, and
 * forward jumps past a keyframe (positions recorded while indexing), seek
 * to the latest keyframe at or before the target time and decode forward,
 * so both choose the same frame and reach bit-identical pixels. A source
 * keeps up to four demuxer/decoder cursors (fewer for large frames),
 * opened on demand, so interleaved requests at different times (e.g. a
 * reversed duplicate layer) each decode forward from their own position.
 * A stream without packet timestamps is never seeked: its frames are
 * numbered from the start of the file (one per packet), and backward
 * access reopens the demuxer and decodes from the start.
 * Converted frames (Y'CbCr -> 8-bit RGBA with the stream's own matrix and
 * range, then the color.c input conversion to blend space) live in an LRU
 * cache bounded in bytes, except that at least SR_VIDEO_CACHE_MIN_FRAMES
 * (one: the frame returned) is always kept, so a single frame larger than
 * the budget is retained regardless (7680x4320 float RGBA is 530,841,600
 * bytes). Not thread-safe. */
typedef struct SrVideoSource SrVideoSource;

typedef struct {
    uint32_t width, height;        /* coded stream dimensions */
    int rate_num, rate_den;        /* stream's nominal frame rate */
    int64_t frame_count;           /* indices up to the last frame's, at the
                                      declared fps */
    bool matrix_approximated;      /* tagged BT.2020 constant luminance:
                                      decoded with the NCL matrix */
} SrVideoInfo;

typedef struct {
    uint64_t requests;             /* sr_video_frame calls */
    uint64_t cache_hits;
    uint64_t decoded;              /* frames produced by the decoder */
    uint64_t seeks;
} SrVideoStats;

#define SR_VIDEO_CACHE_DEFAULT_BYTES ((size_t)256 << 20)
/* Frames every source keeps whatever its budget: the one last returned. */
#define SR_VIDEO_CACHE_MIN_FRAMES 1

/* Opens `path` and indexes its frames at fps_num/fps_den. Converted frames
 * are `project` blend-space images of the stream's size, decoded from
 * `source_space`. `project` is borrowed and must outlive the source. */
SrStatus sr_video_open(const char *path, const SrProject *project,
                       SrColorSpace source_space, uint32_t fps_num,
                       uint32_t fps_den, size_t cache_bytes,
                       SrVideoSource **out, char *err, size_t errlen);
void sr_video_close(SrVideoSource *video);
const SrVideoInfo *sr_video_info(const SrVideoSource *video);
const SrVideoStats *sr_video_stats(const SrVideoSource *video);
/* Frame `index` (clamped to [0, frame_count-1]). *out is owned by the
 * source's cache and stays valid until the next request of a different
 * index from the same source (the cache may keep it longer). */
SrStatus sr_video_frame(SrVideoSource *video, int64_t index,
                        const SrImage **out, char *err, size_t errlen);

/* Bytes a source keeps regardless of its budget (SR_VIDEO_CACHE_MIN_FRAMES
 * converted frames); 0 for NULL. */
size_t sr_video_minimum_bytes(const SrVideoSource *video);
/* Sum of sr_video_minimum_bytes over the scene's open video assets: when
 * it exceeds the cache budget, the budget cannot hold even the frames that
 * are always kept (sr_assets_load warns once). */
size_t sr_video_scene_minimum_bytes(const SrScene *scene);

/* Decodes the first frame of an image file (anything libavformat reads:
 * PNG, JPEG, PPM, WebP, ...) to freshly allocated 8-bit straight RGBA of
 * width x height. When `scale` is false the file must already have that
 * size; otherwise it is resampled (Lanczos). *rgba is owned by the caller. */
SrStatus sr_image_decode_rgba8(const char *path, uint32_t width,
                               uint32_t height, bool scale, uint8_t **rgba,
                               uint32_t *file_width, uint32_t *file_height,
                               char *err, size_t errlen);

#endif
