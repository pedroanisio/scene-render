#define _POSIX_C_SOURCE 200809L
#include "scene_render/audio.h"

#include <errno.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

static char *temp_path(void) {
    char *path = sr_strdup("/tmp/scene-render-audio-XXXXXX");
    if (!path) return NULL;
    int fd = mkstemp(path);
    if (fd < 0) {
        free(path);
        return NULL;
    }
    close(fd);
    return path;
}

static SrStatus decode_asset(SrScene *scene, SrAsset *asset,
                             SrDiagnostics *diag) {
    if (asset->audio_cache_path) return SR_OK;
    char *source = sr_path_join(scene->base_dir, asset->source);
    char *output = temp_path();
    if (!source || !output) {
        free(source); free(output);
        return SR_ERR_MEMORY;
    }
    pid_t pid = fork();
    if (pid < 0) {
        free(source); unlink(output); free(output);
        return SR_ERR_IO;
    }
    if (pid == 0) {
        char rate[32], channels[16];
        snprintf(rate, sizeof(rate), "%u", scene->audio.sample_rate);
        snprintf(channels, sizeof(channels), "%u", scene->audio.channels);
        const char *argv[] = {"ffmpeg", "-nostdin", "-v", "error", "-y",
            "-i", source, "-vn", "-ar", rate, "-ac", channels, "-f",
            "f32le", "-acodec", "pcm_f32le", output, NULL};
        execvp(argv[0], (char *const *)argv);
        _exit(127);
    }
    int status = 0;
    pid_t waited;
    do {
        waited = waitpid(pid, &status, 0);
    } while (waited < 0 && errno == EINTR);
    free(source);
    if (waited < 0 || !WIFEXITED(status) || WEXITSTATUS(status) != 0) {
        sr_diag_error(diag, asset->source_line, "audio", "src",
                      "FFmpeg could not decode audio asset '%s'", asset->id);
        unlink(output); free(output);
        return SR_ERR_ASSET;
    }
    struct stat info;
    uint64_t frame_bytes = sizeof(float) * scene->audio.channels;
    if (stat(output, &info) != 0 || info.st_size < 0 ||
        (uint64_t)info.st_size % frame_bytes != 0) {
        unlink(output); free(output);
        return SR_ERR_ASSET;
    }
    asset->audio_cache_path = output;
    asset->audio_frames = (uint64_t)info.st_size / frame_bytes;
    return SR_OK;
}

static void add_samples(float *mix, const float *input, size_t frames,
                        uint32_t channels, double volume, double pan) {
    double left = volume * (pan > 0.0 ? 1.0 - pan : 1.0);
    double right = volume * (pan < 0.0 ? 1.0 + pan : 1.0);
    for (size_t i = 0; i < frames; ++i) {
        if (channels == 1) {
            mix[i] += input[i] * (float)volume;
        } else {
            mix[i * 2] += input[i * 2] * (float)left;
            mix[i * 2 + 1] += input[i * 2 + 1] * (float)right;
        }
    }
}

static SrStatus mix_track_block(const SrScene *scene, const SrAudioTrack *track,
                                FILE *source, uint64_t scene_first,
                                uint64_t block_first, size_t block_frames,
                                float *mix, float *scratch) {
    const uint32_t rate = scene->audio.sample_rate;
    const uint32_t channels = scene->audio.channels;
    uint64_t start = (uint64_t)llround(track->start * rate);
    uint64_t clip_first = (uint64_t)llround(track->clip_in * rate);
    uint64_t clip_end = track->clip_out >= 0.0
        ? (uint64_t)llround(track->clip_out * rate) : track->asset->audio_frames;
    if (clip_end > track->asset->audio_frames) clip_end = track->asset->audio_frames;
    if (clip_first >= clip_end) return SR_OK;
    uint64_t span = clip_end - clip_first;
    uint64_t plays = track->loop_count > 0 ? (uint64_t)track->loop_count : 1;
    uint64_t total = plays > UINT64_MAX / span ? UINT64_MAX : span * plays;
    uint64_t global = scene_first + block_first;
    size_t offset = 0;
    if (global < start) {
        uint64_t silent = start - global;
        if (silent >= block_frames) return SR_OK;
        offset = (size_t)silent;
    }
    uint64_t local = global + offset - start;
    while (offset < block_frames && local < total) {
        uint64_t source_frame = clip_first + local % span;
        uint64_t until_wrap = span - local % span;
        uint64_t until_end = total - local;
        size_t count = block_frames - offset;
        if ((uint64_t)count > until_wrap) count = (size_t)until_wrap;
        if ((uint64_t)count > until_end) count = (size_t)until_end;
        off_t byte_offset = (off_t)(source_frame * channels * sizeof(float));
        if (fseeko(source, byte_offset, SEEK_SET) != 0 ||
            fread(scratch, sizeof(float) * channels, count, source) != count)
            return SR_ERR_IO;
        add_samples(mix + offset * channels, scratch, count, channels,
                    track->volume, track->pan);
        offset += count;
        local += count;
    }
    return SR_OK;
}

SrStatus sr_audio_mix_to_file(SrScene *scene, uint64_t first_frame,
                              uint64_t end_frame, char **mixed_path,
                              SrDiagnostics *diag) {
    *mixed_path = NULL;
    if (!scene->audio.track_count) return SR_OK;
    for (size_t i = 0; i < scene->audio.track_count; ++i) {
        SrStatus status = decode_asset(scene, scene->audio.tracks[i].asset, diag);
        if (status != SR_OK) return status;
    }
    char *path = temp_path();
    if (!path) return SR_ERR_IO;
    FILE *output = fopen(path, "wb");
    if (!output) { unlink(path); free(path); return SR_ERR_IO; }
    const uint32_t channels = scene->audio.channels;
    uint64_t first_sample = (uint64_t)llround((double)first_frame *
        scene->project.fps_den * scene->audio.sample_rate /
        scene->project.fps_num);
    uint64_t end_sample = (uint64_t)llround((double)end_frame *
        scene->project.fps_den * scene->audio.sample_rate /
        scene->project.fps_num);
    const size_t block_capacity = 4096;
    float *mix = sr_alloc(block_capacity * channels * sizeof(float));
    float *scratch = sr_alloc(block_capacity * channels * sizeof(float));
    FILE **inputs = sr_alloc(scene->audio.track_count * sizeof(*inputs));
    if (!mix || !scratch || !inputs) {
        free(mix); free(scratch); free(inputs); fclose(output);
        unlink(path); free(path); return SR_ERR_MEMORY;
    }
    SrStatus status = SR_OK;
    for (size_t i = 0; i < scene->audio.track_count; ++i) {
        inputs[i] = fopen(scene->audio.tracks[i].asset->audio_cache_path, "rb");
        if (!inputs[i]) { status = SR_ERR_IO; break; }
    }
    for (uint64_t at = 0; status == SR_OK && at < end_sample - first_sample;) {
        size_t count = block_capacity;
        if ((uint64_t)count > end_sample - first_sample - at)
            count = (size_t)(end_sample - first_sample - at);
        memset(mix, 0, count * channels * sizeof(float));
        for (size_t i = 0; i < scene->audio.track_count; ++i) {
            status = mix_track_block(scene, &scene->audio.tracks[i], inputs[i],
                                     first_sample, at, count, mix, scratch);
            if (status != SR_OK) break;
        }
        for (size_t i = 0; i < count * channels; ++i)
            mix[i] = fmaxf(-1.0f, fminf(1.0f, mix[i]));
        if (status == SR_OK && fwrite(mix, sizeof(float) * channels, count,
                                     output) != count) status = SR_ERR_IO;
        at += count;
    }
    for (size_t i = 0; i < scene->audio.track_count; ++i)
        if (inputs[i]) fclose(inputs[i]);
    free(inputs); free(scratch); free(mix);
    if (fclose(output) != 0) status = SR_ERR_IO;
    if (status != SR_OK) { unlink(path); free(path); return status; }
    *mixed_path = path;
    return SR_OK;
}

void sr_audio_remove_temporary(SrScene *scene, char **mixed_path) {
    if (mixed_path && *mixed_path) { unlink(*mixed_path); free(*mixed_path); *mixed_path = NULL; }
    for (size_t i = 0; i < scene->asset_count; ++i) {
        if (scene->assets[i].audio_cache_path) {
            unlink(scene->assets[i].audio_cache_path);
            free(scene->assets[i].audio_cache_path);
            scene->assets[i].audio_cache_path = NULL;
        }
    }
}
