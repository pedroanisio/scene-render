#define _POSIX_C_SOURCE 200809L
#include "scene_render/encoder.h"

#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

static const char *sr_codec_encoder(SrCodec codec) {
    switch (codec) {
    case SR_CODEC_H265:
        return "libx265";
    case SR_CODEC_FFV1:
        return "ffv1";
    case SR_CODEC_H264:
    default:
        return "libx264";
    }
}

static void sr_color_tags(SrColorSpace space, const char **primaries,
                          const char **transfer, const char **matrix) {
    if (space == SR_COLOR_DISPLAY_P3) {
        *primaries = "smpte432";
        *transfer = "iec61966-2-1";
        *matrix = "bt709";
    } else if (space == SR_COLOR_REC2020) {
        *primaries = "bt2020";
        *transfer = "bt2020-10";
        *matrix = "bt2020nc";
    } else if (space == SR_COLOR_REC709) {
        *primaries = "bt709";
        *transfer = "bt709";
        *matrix = "bt709";
    } else {
        *primaries = "bt709";
        *transfer = "iec61966-2-1";
        *matrix = "bt709";
    }
}

bool sr_encoder_available(const char *name) {
    if (!name) return false;
    int output[2];
    if (pipe(output) != 0) return false;
    pid_t pid = fork();
    if (pid < 0) { close(output[0]); close(output[1]); return false; }
    if (pid == 0) {
        dup2(output[1], STDOUT_FILENO);
        int null_fd = open("/dev/null", O_WRONLY);
        if (null_fd >= 0) { dup2(null_fd, STDERR_FILENO); close(null_fd); }
        close(output[0]); close(output[1]);
        const char *argv[] = {"ffmpeg", "-hide_banner", "-loglevel", "error",
                              "-encoders", NULL};
        execvp(argv[0], (char *const *)argv);
        _exit(127);
    }
    close(output[1]);
    FILE *stream = fdopen(output[0], "r");
    bool found = strcmp(name, "ffmpeg") == 0;
    if (stream) {
        char *line = NULL; size_t capacity = 0;
        while (getline(&line, &capacity, stream) >= 0) {
            char flags[32], encoder_name[128];
            if (sscanf(line, " %31s %127s", flags, encoder_name) == 2 &&
                strcmp(encoder_name, name) == 0) found = true;
        }
        free(line);
        fclose(stream);
    } else close(output[0]);
    int status = 0;
    while (waitpid(pid, &status, 0) < 0 && errno == EINTR) {}
    return found && WIFEXITED(status) && WEXITSTATUS(status) == 0;
}

SrStatus sr_encoder_open(SrEncoder *encoder, const SrScene *scene,
                         const char *path, unsigned threads,
                         const char *audio_path,
                         SrDiagnostics *diag) {
    if (!encoder || !scene || !path) {
        return SR_ERR_ARGUMENT;
    }
    *encoder = (SrEncoder){.pid = -1, .input_fd = -1};
    if (!sr_encoder_available("ffmpeg")) {
        sr_diag_error(diag, 0, NULL, NULL,
                      "FFmpeg executable is unavailable; video output cannot start");
        return SR_ERR_ENCODER;
    }
    const char *video_encoder = sr_codec_encoder(scene->output.codec);
    if (!sr_encoder_available(video_encoder)) {
        sr_diag_error(diag, scene->output.source_line, "output", "codec",
                      "FFmpeg encoder '%s' is unavailable", video_encoder);
        return SR_ERR_ENCODER;
    }
    if (audio_path && !sr_encoder_available(scene->output.audio_codec)) {
        sr_diag_error(diag, scene->output.source_line, "output", "audioCodec",
                      "FFmpeg encoder '%s' is unavailable",
                      scene->output.audio_codec);
        return SR_ERR_ENCODER;
    }
    int input[2];
    if (pipe(input) != 0) {
        sr_diag_error(diag, 0, NULL, NULL, "cannot create encoder pipe: %s",
                      strerror(errno));
        return SR_ERR_IO;
    }
    pid_t pid = fork();
    if (pid < 0) {
        close(input[0]);
        close(input[1]);
        return SR_ERR_IO;
    }
    if (pid == 0) {
        dup2(input[0], STDIN_FILENO);
        close(input[0]);
        close(input[1]);
        char size[64], rate[64], crf[32], bitrate[32], thread_count[32];
        char audio_rate[32], audio_channels[16], audio_bitrate[32];
        char x265_params[64], scale_filter[96];
        snprintf(size, sizeof(size), "%ux%u", scene->project.width,
                 scene->project.height);
        snprintf(rate, sizeof(rate), "%u/%u", scene->project.fps_num,
                 scene->project.fps_den);
        snprintf(crf, sizeof(crf), "%d", scene->output.crf);
        snprintf(bitrate, sizeof(bitrate), "%llu",
                 (unsigned long long)scene->output.bitrate);
        snprintf(thread_count, sizeof(thread_count), "%u", threads);
        snprintf(audio_rate, sizeof(audio_rate), "%u", scene->audio.sample_rate);
        snprintf(audio_channels, sizeof(audio_channels), "%u",
                 scene->audio.channels);
        snprintf(audio_bitrate, sizeof(audio_bitrate), "%llu",
                 (unsigned long long)scene->output.audio_bitrate);
        snprintf(x265_params, sizeof(x265_params), "pools=%u:frame-threads=1",
                 threads);
        snprintf(scale_filter, sizeof(scale_filter),
                 "scale=in_range=pc:out_range=%s:flags=accurate_rnd+full_chroma_int",
                 scene->output.full_range ? "pc" : "tv");
        const char *primaries, *transfer, *matrix;
        sr_color_tags(scene->output.color_space, &primaries, &transfer, &matrix);
        const char *argv[88];
        size_t n = 0;
        argv[n++] = "ffmpeg";
        argv[n++] = "-nostdin";
        argv[n++] = "-hide_banner";
        argv[n++] = "-loglevel";
        argv[n++] = diag->verbose ? "info" : "error";
        argv[n++] = "-y";
        argv[n++] = "-f";
        argv[n++] = "rawvideo";
        argv[n++] = "-pixel_format";
        argv[n++] = "rgba";
        argv[n++] = "-video_size";
        argv[n++] = size;
        argv[n++] = "-framerate";
        argv[n++] = rate;
        argv[n++] = "-i";
        argv[n++] = "pipe:0";
        if (audio_path) {
            argv[n++] = "-f"; argv[n++] = "f32le";
            argv[n++] = "-ar"; argv[n++] = audio_rate;
            argv[n++] = "-ac"; argv[n++] = audio_channels;
            argv[n++] = "-i"; argv[n++] = audio_path;
            argv[n++] = "-map"; argv[n++] = "0:v:0";
            argv[n++] = "-map"; argv[n++] = "1:a:0";
        } else {
            argv[n++] = "-an";
        }
        argv[n++] = "-c:v";
        argv[n++] = sr_codec_encoder(scene->output.codec);
        if (scene->output.codec != SR_CODEC_FFV1) {
            argv[n++] = "-preset";
            argv[n++] = scene->output.preset;
            if (scene->output.bitrate) {
                argv[n++] = "-b:v";
                argv[n++] = bitrate;
            } else {
                argv[n++] = "-crf";
                argv[n++] = crf;
            }
        }
        if (scene->output.codec == SR_CODEC_H265 && threads > 0) {
            argv[n++] = "-x265-params";
            argv[n++] = x265_params;
        }
        argv[n++] = "-vf";
        argv[n++] = scale_filter;
        argv[n++] = "-pix_fmt";
        argv[n++] = scene->output.pixel_format;
        argv[n++] = "-color_primaries";
        argv[n++] = primaries;
        argv[n++] = "-color_trc";
        argv[n++] = transfer;
        argv[n++] = "-colorspace";
        argv[n++] = matrix;
        argv[n++] = "-color_range";
        argv[n++] = scene->output.full_range ? "pc" : "tv";
        argv[n++] = "-threads";
        argv[n++] = thread_count;
        argv[n++] = "-fflags";
        argv[n++] = "+bitexact";
        argv[n++] = "-flags:v";
        argv[n++] = "+bitexact";
        if (scene->project.mode == SR_MODE_EQUIRECTANGULAR &&
            scene->output.spherical_metadata) {
            argv[n++] = "-metadata:s:v:0";
            argv[n++] = "projection=equirectangular";
            argv[n++] = "-metadata:s:v:0";
            argv[n++] = "spherical-video=true";
        }
        if (audio_path) {
            argv[n++] = "-c:a";
            argv[n++] = scene->output.audio_codec;
            argv[n++] = "-b:a";
            argv[n++] = audio_bitrate;
            argv[n++] = "-shortest";
        }
        argv[n++] = "-map_metadata";
        argv[n++] = "-1";
        argv[n++] = path;
        argv[n] = NULL;
        execvp(argv[0], (char *const *)argv);
        _exit(127);
    }
    close(input[0]);
    signal(SIGPIPE, SIG_IGN);
    encoder->pid = pid;
    encoder->input_fd = input[1];
    encoder->active = true;
    encoder->write_seconds = 0.0;
    sr_diag_info(diag, "streaming RGBA frames to FFmpeg (%s)",
                 sr_codec_encoder(scene->output.codec));
    return SR_OK;
}

SrStatus sr_encoder_write(SrEncoder *encoder, const uint8_t *rgba,
                          size_t byte_count, SrDiagnostics *diag) {
    if (!encoder || !encoder->active || !rgba) {
        return SR_ERR_ARGUMENT;
    }
    double start = sr_monotonic_seconds();
    size_t written = 0;
    while (written < byte_count) {
        ssize_t amount = write(encoder->input_fd, rgba + written,
                               byte_count - written);
        if (amount > 0) {
            written += (size_t)amount;
        } else if (amount < 0 && errno == EINTR) {
            continue;
        } else {
            sr_diag_error(diag, 0, NULL, NULL,
                          "encoder pipe failed after %zu/%zu bytes: %s", written,
                          byte_count, strerror(errno));
            return SR_ERR_ENCODER;
        }
    }
    encoder->write_seconds += sr_monotonic_seconds() - start;
    return SR_OK;
}

SrStatus sr_encoder_close(SrEncoder *encoder, SrDiagnostics *diag) {
    if (!encoder || !encoder->active) {
        return SR_OK;
    }
    double start = sr_monotonic_seconds();
    close(encoder->input_fd);
    encoder->input_fd = -1;
    int status = 0;
    pid_t waited;
    do {
        waited = waitpid(encoder->pid, &status, 0);
    } while (waited < 0 && errno == EINTR);
    encoder->active = false;
    encoder->write_seconds += sr_monotonic_seconds() - start;
    if (waited < 0) {
        sr_diag_error(diag, 0, NULL, NULL, "cannot wait for FFmpeg: %s",
                      strerror(errno));
        return SR_ERR_ENCODER;
    }
    if (!WIFEXITED(status) || WEXITSTATUS(status) != 0) {
        sr_diag_error(diag, 0, NULL, NULL,
                      "FFmpeg exited unsuccessfully (status=%d)", status);
        return SR_ERR_ENCODER;
    }
    return SR_OK;
}
