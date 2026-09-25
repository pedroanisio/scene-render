/* SPDX-License-Identifier: Apache-2.0 */
/* Synthesizes small media files with chosen timestamps through libav, for
 * the timeline tests: FFV1 gray video frames of one flat level each, and
 * mono 16-bit PCM audio packets of one constant value each. */
#ifndef SR_TEST_MEDIA_SYNTH_H
#define SR_TEST_MEDIA_SYNTH_H

#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/channel_layout.h>

#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    const char *format;            /* muxer name: "matroska", "nut", ... */
    /* Video (frames = 0: none): frame k at pts video_pts[k] in video_tb. */
    AVRational video_tb;
    const int64_t *video_pts;
    const uint8_t *video_level;
    int video_frames;
    int width, height;
    /* Audio (packets = 0: none): packet k holds audio_samples[k] samples of
     * value audio_value[k] at pts audio_pts[k] in audio_tb. */
    AVRational audio_tb;
    int audio_rate;
    const int64_t *audio_pts;
    const int *audio_samples;
    const int16_t *audio_value;
    int audio_packets;
} SynthMedia;

static inline bool synth_write(AVFormatContext *fmt, AVPacket *pkt, AVStream *st,
                               AVRational tb) {
    av_packet_rescale_ts(pkt, tb, st->time_base);
    pkt->stream_index = st->index;
    return av_interleaved_write_frame(fmt, pkt) >= 0;
}

static inline bool synth_media(const char *path, const SynthMedia *m) {
    AVFormatContext *fmt = NULL;
    if (avformat_alloc_output_context2(&fmt, NULL, m->format, path) < 0) return false;
    bool ok = true;
    AVCodecContext *enc = NULL;
    AVStream *vs = NULL, *as = NULL;
    if (m->video_frames) {
        const AVCodec *codec = avcodec_find_encoder(AV_CODEC_ID_FFV1);
        enc = codec ? avcodec_alloc_context3(codec) : NULL;
        vs = avformat_new_stream(fmt, NULL);
        ok = enc && vs;
        if (ok) {
            enc->width = m->width;
            enc->height = m->height;
            enc->pix_fmt = AV_PIX_FMT_GRAY8;
            enc->time_base = m->video_tb;
            if (fmt->oformat->flags & AVFMT_GLOBALHEADER)
                enc->flags |= AV_CODEC_FLAG_GLOBAL_HEADER;
            ok = avcodec_open2(enc, codec, NULL) >= 0 &&
                 avcodec_parameters_from_context(vs->codecpar, enc) >= 0;
            vs->time_base = m->video_tb;
        }
    }
    if (ok && m->audio_packets) {
        as = avformat_new_stream(fmt, NULL);
        ok = as != NULL;
        if (ok) {
            AVCodecParameters *par = as->codecpar;
            par->codec_type = AVMEDIA_TYPE_AUDIO;
            par->codec_id = AV_CODEC_ID_PCM_S16LE;
            par->format = AV_SAMPLE_FMT_S16;
            par->sample_rate = m->audio_rate;
            av_channel_layout_default(&par->ch_layout, 1);
            par->bits_per_coded_sample = 16;
            par->block_align = 2;
            as->time_base = m->audio_tb;
        }
    }
    ok = ok && avio_open(&fmt->pb, path, AVIO_FLAG_WRITE) >= 0 &&
         avformat_write_header(fmt, NULL) >= 0;
    AVPacket *pkt = av_packet_alloc();
    AVFrame *frame = av_frame_alloc();
    ok = ok && pkt && frame;
    if (ok && enc) {
        frame->format = enc->pix_fmt;
        frame->width = enc->width;
        frame->height = enc->height;
        ok = av_frame_get_buffer(frame, 0) >= 0;
    }
    int v = 0, a = 0;
    while (ok && (v < m->video_frames || a < m->audio_packets)) {
        /* Interleave by time so the muxer sees nearly ordered input. */
        bool video = v < m->video_frames &&
                     (a >= m->audio_packets ||
                      av_compare_ts(m->video_pts[v], m->video_tb, m->audio_pts[a],
                                    m->audio_tb) <= 0);
        if (video) {
            ok = av_frame_make_writable(frame) >= 0;
            for (int y = 0; ok && y < frame->height; ++y)
                memset(frame->data[0] + (size_t)y * frame->linesize[0],
                       m->video_level[v], (size_t)frame->width);
            frame->pts = m->video_pts[v];
            ok = ok && avcodec_send_frame(enc, frame) >= 0;
            while (ok && avcodec_receive_packet(enc, pkt) == 0) {
                pkt->duration = 0;
                ok = synth_write(fmt, pkt, vs, enc->time_base);
            }
            ++v;
        } else {
            int n = m->audio_samples[a];
            ok = av_new_packet(pkt, n * 2) >= 0;
            for (int i = 0; ok && i < n; ++i) {
                int16_t value = m->audio_value[a];
                memcpy(pkt->data + 2 * i, &value, 2);
            }
            if (ok) {
                pkt->pts = pkt->dts = m->audio_pts[a];
                pkt->duration = av_rescale_q(n, (AVRational){1, m->audio_rate},
                                             m->audio_tb);
                pkt->flags |= AV_PKT_FLAG_KEY;
                ok = synth_write(fmt, pkt, as, m->audio_tb);
            }
            ++a;
        }
    }
    if (ok && enc) {
        avcodec_send_frame(enc, NULL);
        while (ok && avcodec_receive_packet(enc, pkt) == 0)
            ok = synth_write(fmt, pkt, vs, enc->time_base);
    }
    if (ok) ok = av_write_trailer(fmt) >= 0;
    av_frame_free(&frame);
    av_packet_free(&pkt);
    avcodec_free_context(&enc);
    if (fmt->pb) avio_closep(&fmt->pb);
    avformat_free_context(fmt);
    return ok;
}

#endif
