/* SPDX-License-Identifier: Apache-2.0 */
/* Output-side pixel work of B1-6: the linear float conversion for EXR and
 * the poster/thumbnail still writers. */
#include "output_media_internal.h"

#include "scene_render/parallel.h"
#include "scene_render/renderer.h"

#include <libavcodec/avcodec.h>
#include <libswscale/swscale.h>

#include <errno.h>
#include <math.h>
#include <stdio.h>
#include <string.h>

/* The working-space transfer decode of sr_color_decode, but symmetric
 * about zero and not clamped, so over-range blend values survive. */
double sr_output_decode_extended(double value, SrColorSpace space) {
    double magnitude = fabs(value), linear;
    if (space == SR_COLOR_REC2020 || space == SR_COLOR_REC709) {
        const double alpha = space == SR_COLOR_REC2020 ? 1.09929682680944 : 1.099;
        const double beta = space == SR_COLOR_REC2020 ? 0.018053968510807 : 0.018;
        linear = magnitude < 4.5 * beta ? magnitude / 4.5
               : pow((magnitude + alpha - 1.0) / alpha, 1.0 / 0.45);
    } else {
        linear = magnitude <= .04045 ? magnitude / 12.92
               : pow((magnitude + .055) / 1.055, 2.4);
    }
    return value < 0.0 ? -linear : linear;
}

typedef struct {
    const SrColorOutput *color;
    SrColorSpace working;
    const SrFrame *frame;
    float *out;
} LinearContext;

static void linear_rows(void *opaque, size_t begin, size_t end) {
    const LinearContext *context = opaque;
    const SrColorOutput *color = context->color;
    uint32_t width = context->frame->width;
    for (size_t y = begin; y < end; ++y) {
        const float *s = context->frame->px + y * width * 4;
        float *d = context->out + y * width * 4;
        for (uint32_t x = 0; x < width; ++x, s += 4, d += 4) {
            float alpha = s[3];
            if (!(alpha > 0.0f)) {
                d[0] = d[1] = d[2] = d[3] = 0.0f;
                continue;
            }
            double c[3] = {s[0] / alpha, s[1] / alpha, s[2] / alpha};
            if (!color->linear_light)
                for (int i = 0; i < 3; ++i)
                    c[i] = sr_output_decode_extended(c[i], context->working);
            if (!color->identity_gamut) {
                double m[3];
                for (int row = 0; row < 3; ++row)
                    m[row] = color->matrix[row][0] * c[0] +
                             color->matrix[row][1] * c[1] +
                             color->matrix[row][2] * c[2];
                memcpy(c, m, sizeof(c));
            }
            for (int i = 0; i < 3; ++i) d[i] = (float)(c[i] * alpha);
            d[3] = alpha;
        }
    }
}

SrStatus sr_output_convert_linear(const SrColorOutput *color,
                                  const SrProject *project, const SrFrame *frame,
                                  float *rgba, unsigned threads) {
    if (!color || !project || !frame || !frame->px || !rgba) return SR_ERR_ARGUMENT;
    LinearContext context = {color, project->working_color_space, frame, rgba};
    return sr_parallel_for(frame->height, threads, linear_rows, &context);
}

/* Baseline JPEG through libavcodec's mjpeg encoder (bit-exact): 4:2:0
 * full-range Y'CbCr, BT.601 matrix as JFIF defines it, fixed qscale. */
static SrStatus write_jpeg(const char *path, uint32_t width, uint32_t height,
                           const uint8_t *rgba, double quality,
                           SrDiagnostics *diag) {
    const AVCodec *codec = avcodec_find_encoder(AV_CODEC_ID_MJPEG);
    AVCodecContext *c = codec ? avcodec_alloc_context3(codec) : NULL;
    AVFrame *frame = av_frame_alloc();
    AVPacket *packet = av_packet_alloc();
    struct SwsContext *sws = NULL;
    SrStatus status = SR_OK;
    int rc = c && frame && packet ? 0 : AVERROR(ENOMEM);
    int qscale = (int)floor(2.0 + (1.0 - quality) * 29.0 + 0.5);
    if (rc == 0) {
        c->width = (int)width;
        c->height = (int)height;
        c->pix_fmt = AV_PIX_FMT_YUVJ420P;
        c->color_range = AVCOL_RANGE_JPEG;
        c->time_base = (AVRational){1, 1};
        c->flags |= AV_CODEC_FLAG_BITEXACT | AV_CODEC_FLAG_QSCALE;
        c->global_quality = FF_QP2LAMBDA * qscale;
        c->thread_count = 1;
        rc = avcodec_open2(c, codec, NULL);
    }
    if (rc == 0) {
        frame->format = c->pix_fmt;
        frame->width = c->width;
        frame->height = c->height;
        frame->quality = c->global_quality;
        rc = av_frame_get_buffer(frame, 0);
    }
    if (rc == 0) {
        sws = sws_getContext((int)width, (int)height, AV_PIX_FMT_RGBA, (int)width,
                             (int)height, AV_PIX_FMT_YUVJ420P,
                             SWS_BICUBIC | SWS_ACCURATE_RND | SWS_BITEXACT |
                             SWS_FULL_CHR_H_INT, NULL, NULL, NULL);
        if (!sws) rc = AVERROR(ENOMEM);
    }
    if (rc == 0) {
        const uint8_t *source[4] = {rgba, NULL, NULL, NULL};
        const int strides[4] = {(int)width * 4, 0, 0, 0};
        if (sws_scale(sws, source, strides, 0, (int)height, frame->data,
                      frame->linesize) <= 0)
            rc = AVERROR(EINVAL);
    }
    if (rc == 0) rc = avcodec_send_frame(c, frame);
    if (rc == 0) rc = avcodec_send_frame(c, NULL);
    if (rc == 0) rc = avcodec_receive_packet(c, packet);
    if (rc == 0) {
        FILE *file = fopen(path, "wb");
        bool ok = file && fwrite(packet->data, 1, (size_t)packet->size, file) ==
                              (size_t)packet->size;
        if (file && fclose(file) != 0) ok = false;
        if (!ok) {
            sr_diag_error(diag, 0, NULL, NULL, "cannot write '%s': %s", path,
                          strerror(errno));
            status = SR_ERR_IO;
        }
    } else {
        char message[AV_ERROR_MAX_STRING_SIZE];
        av_strerror(rc, message, sizeof(message));
        sr_diag_error(diag, 0, NULL, NULL, "JPEG encoding of '%s' failed: %s",
                      path, message);
        status = rc == AVERROR(ENOMEM) ? SR_ERR_MEMORY : SR_ERR_ENCODER;
    }
    sws_freeContext(sws);
    av_packet_free(&packet);
    av_frame_free(&frame);
    avcodec_free_context(&c);
    return status;
}

SrStatus sr_output_write_still(const SrStill *still, const char *path,
                               uint32_t width, uint32_t height,
                               const uint8_t *rgba, SrDiagnostics *diag) {
    if (!still || !path || !rgba || !width || !height) return SR_ERR_ARGUMENT;
    if (still->format == SR_STILL_PNG)
        return sr_write_png(path, width, height, rgba, diag);
    return write_jpeg(path, width, height, rgba, still->quality, diag);
}
