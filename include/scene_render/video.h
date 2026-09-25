#ifndef SCENE_RENDER_VIDEO_H
#define SCENE_RENDER_VIDEO_H

#include "scene_render/scene.h"

/* In-process libavformat/libavcodec decoding of video and still images.
 *
 * A video source keeps one demuxer and decoder open for the lifetime of the
 * asset. Frames are addressed by index at the DECLARED frame rate: a frame's
 * index is its pts, rebased to the first frame, in units of 1/declared fps.
 * Sequential requests decode forward; backward or far jumps seek to the
 * nearest earlier keyframe and decode forward, so both reach bit-identical
 * frames. An index with no frame of its own (variable-rate gaps) shows the
 * latest earlier frame; indices past the last frame show the last one.
 * Converted frames (Y'CbCr -> 8-bit RGBA with the stream's own matrix and
 * range, then the color.c input conversion to blend space) live in an LRU
 * cache bounded in bytes. Not thread-safe. */
typedef struct SrVideoSource SrVideoSource;

typedef struct {
    uint32_t width, height;        /* coded stream dimensions */
    int rate_num, rate_den;        /* stream's nominal frame rate */
    int64_t frame_count;           /* first to last frame at the declared fps */
} SrVideoInfo;

typedef struct {
    uint64_t requests;             /* sr_video_frame calls */
    uint64_t cache_hits;
    uint64_t decoded;              /* frames produced by the decoder */
    uint64_t seeks;
} SrVideoStats;

#define SR_VIDEO_CACHE_DEFAULT_BYTES ((size_t)256 << 20)
#define SR_VIDEO_CACHE_MIN_FRAMES 2

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
 * source's cache and stays valid until SR_VIDEO_CACHE_MIN_FRAMES - 1 more
 * distinct frames have been requested from the same source. */
SrStatus sr_video_frame(SrVideoSource *video, int64_t index,
                        const SrImage **out, char *err, size_t errlen);

/* Decodes the first frame of an image file (anything libavformat reads:
 * PNG, JPEG, PPM, WebP, ...) to freshly allocated 8-bit straight RGBA of
 * width x height. When `scale` is false the file must already have that
 * size; otherwise it is resampled (Lanczos). *rgba is owned by the caller. */
SrStatus sr_image_decode_rgba8(const char *path, uint32_t width,
                               uint32_t height, bool scale, uint8_t **rgba,
                               uint32_t *file_width, uint32_t *file_height,
                               char *err, size_t errlen);

#endif
