/* Shared by video.c and audio.c: the timeline origin both measure a file's
 * streams from, so the video and the soundtrack of one file stay in sync.
 * Internal to the core; not installed. */
#ifndef SR_MEDIA_INTERNAL_H
#define SR_MEDIA_INTERNAL_H

#include <libavformat/avformat.h>
#include <libavutil/mathematics.h>

/* Timestamp `ts` of stream `st` in AV_TIME_BASE units, as a start of
 * content: audio codec priming that the demuxer leaves before zero with a
 * declared initial_padding (AAC or Opus in Matroska: CodecDelay) is not
 * content, so such a start moves forward by the padding, to zero at most.
 * MP4 edit lists already exclude priming and declare no padding. */
static inline int64_t sr_media_start_us(const AVStream *st, int64_t ts) {
    if (ts == AV_NOPTS_VALUE || st->time_base.num <= 0 || st->time_base.den <= 0)
        return AV_NOPTS_VALUE;
    int64_t us = av_rescale_q(ts, st->time_base, AV_TIME_BASE_Q);
    const AVCodecParameters *par = st->codecpar;
    if (us < 0 && par->codec_type == AVMEDIA_TYPE_AUDIO &&
        par->initial_padding > 0 && par->sample_rate > 0) {
        us += av_rescale(par->initial_padding, AV_TIME_BASE, par->sample_rate);
        if (us > 0) us = 0;
    }
    return us;
}

/* The container's start time (the earliest stream start, as
 * AVFormatContext.start_time) with priming excluded per sr_media_start_us,
 * in AV_TIME_BASE units. AV_NOPTS_VALUE when no stream start is known: the
 * caller then falls back to the earliest first timestamp of the streams it
 * uses (sr_media_start_us of each, minimum). */
static inline int64_t sr_media_origin_us(const AVFormatContext *fmt) {
    int64_t origin = AV_NOPTS_VALUE;
    for (unsigned i = 0; i < fmt->nb_streams; ++i) {
        const AVStream *st = fmt->streams[i];
        int64_t us = sr_media_start_us(st, st->start_time);
        if (us != AV_NOPTS_VALUE && (origin == AV_NOPTS_VALUE || us < origin))
            origin = us;
    }
    return origin != AV_NOPTS_VALUE ? origin : fmt->start_time;
}

/* The origin in `st`'s time base; exactly the stream's own start_time when
 * the origin is that start. */
static inline int64_t sr_media_origin_in(const AVStream *st, int64_t origin_us) {
    if (origin_us == AV_NOPTS_VALUE) return 0;
    if (st->start_time != AV_NOPTS_VALUE &&
        av_rescale_q(st->start_time, st->time_base, AV_TIME_BASE_Q) == origin_us)
        return st->start_time;
    return av_rescale_q(origin_us, AV_TIME_BASE_Q, st->time_base);
}

/* Fallback origin when sr_media_origin_us has none: demuxes the whole file
 * once and returns the earliest sr_media_start_us over the packets of the
 * streams `a` and `b` (negative = unused), or 0 when none has a timestamp.
 * The demuxer is left at the end. */
static inline int sr_media_scan_origin(AVFormatContext *fmt, AVPacket *pkt,
                                       int a, int b, int64_t *origin_us) {
    int64_t origin = AV_NOPTS_VALUE;
    int rc;
    while ((rc = av_read_frame(fmt, pkt)) >= 0) {
        int s = pkt->stream_index;
        if (s == a || s == b) {
            int64_t ts = pkt->pts != AV_NOPTS_VALUE ? pkt->pts : pkt->dts;
            int64_t us = sr_media_start_us(fmt->streams[s], ts);
            if (us != AV_NOPTS_VALUE && (origin == AV_NOPTS_VALUE || us < origin))
                origin = us;
        }
        av_packet_unref(pkt);
    }
    *origin_us = origin == AV_NOPTS_VALUE ? 0 : origin;
    return rc == AVERROR_EOF ? 0 : rc;
}

#endif
