/* Stream inspection for tests/run-integration.sh, replacing ffprobe:
 *   sr-probe FILE
 * prints one line per stream:
 *   stream=<index> type=video codec=<name> width=<w> height=<h> frames=<n>
 *     pix_fmt=<fmt> range=<tv|pc> space=<..> transfer=<..> primaries=<..>
 *     duration=<seconds> spherical=<projection|none>
 *   stream=<index> type=audio codec=<name> rate=<hz> channels=<n>
 *     duration=<seconds>
 * Frame counts and durations come from demuxing every packet (pts + duration
 * of the last packet minus the first pts, from t=0 at the earliest), not
 * from container headers. */
#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/pixdesc.h>
#include <libavutil/spherical.h>

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

static const char *spherical(const AVStream *st) {
#if LIBAVCODEC_VERSION_INT >= AV_VERSION_INT(60, 31, 102)
    const AVPacketSideData *sd = av_packet_side_data_get(
        st->codecpar->coded_side_data, st->codecpar->nb_coded_side_data,
        AV_PKT_DATA_SPHERICAL);
    if (!sd) return "none";
    return av_spherical_projection_name(((const AVSphericalMapping *)sd->data)->projection);
#else
    size_t size = 0;
    const uint8_t *data = av_stream_get_side_data(st, AV_PKT_DATA_SPHERICAL, &size);
    return data ? av_spherical_projection_name(((const AVSphericalMapping *)data)->projection)
                : "none";
#endif
}

int main(int argc, char **argv) {
    if (argc != 2) {
        fprintf(stderr, "usage: %s FILE\n", argv[0]);
        return 2;
    }
    av_log_set_level(AV_LOG_ERROR);
    AVFormatContext *fmt = NULL;
    if (avformat_open_input(&fmt, argv[1], NULL, NULL) < 0 ||
        avformat_find_stream_info(fmt, NULL) < 0) {
        fprintf(stderr, "sr-probe: cannot read %s\n", argv[1]);
        return 1;
    }
    unsigned n = fmt->nb_streams;
    int64_t *count = calloc(n, sizeof(*count));
    int64_t *first = calloc(n, sizeof(*first));
    int64_t *end = calloc(n, sizeof(*end));
    AVPacket *pkt = av_packet_alloc();
    if (!count || !first || !end || !pkt) return 1;
    for (unsigned i = 0; i < n; ++i) { first[i] = INT64_MAX; end[i] = INT64_MIN; }
    while (av_read_frame(fmt, pkt) >= 0) {
        unsigned i = (unsigned)pkt->stream_index;
        if (i < n && pkt->pts != AV_NOPTS_VALUE) {
            ++count[i];
            if (pkt->pts < first[i]) first[i] = pkt->pts;
            if (pkt->pts + pkt->duration > end[i]) end[i] = pkt->pts + pkt->duration;
        }
        av_packet_unref(pkt);
    }
    for (unsigned i = 0; i < n; ++i) {
        const AVStream *st = fmt->streams[i];
        const AVCodecParameters *par = st->codecpar;
        /* Packets before t=0 are codec priming the edit list skips. */
        int64_t start = first[i] < 0 ? 0 : first[i];
        double duration = count[i] ? (double)(end[i] - start) * av_q2d(st->time_base) : 0.0;
        if (par->codec_type == AVMEDIA_TYPE_VIDEO) {
            printf("stream=%u type=video codec=%s width=%d height=%d frames=%lld "
                   "pix_fmt=%s range=%s space=%s transfer=%s primaries=%s "
                   "duration=%.6f spherical=%s\n",
                   i, avcodec_get_name(par->codec_id), par->width, par->height,
                   (long long)count[i],
                   av_get_pix_fmt_name((enum AVPixelFormat)par->format),
                   par->color_range == AVCOL_RANGE_JPEG ? "pc"
                       : par->color_range == AVCOL_RANGE_MPEG ? "tv" : "unknown",
                   av_color_space_name(par->color_space),
                   av_color_transfer_name(par->color_trc),
                   av_color_primaries_name(par->color_primaries), duration,
                   spherical(st));
        } else if (par->codec_type == AVMEDIA_TYPE_AUDIO) {
            printf("stream=%u type=audio codec=%s rate=%d channels=%d duration=%.6f\n",
                   i, avcodec_get_name(par->codec_id), par->sample_rate,
                   par->ch_layout.nb_channels, duration);
        }
    }
    av_packet_free(&pkt);
    free(count); free(first); free(end);
    avformat_close_input(&fmt);
    return 0;
}
