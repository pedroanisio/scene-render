#define _POSIX_C_SOURCE 200809L
#include "scene_render/assets.h"
#include "scene_render/procedural.h"

#include <ctype.h>
#include <errno.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

static bool sr_ppm_token(FILE *file, char *buffer, size_t capacity) {
    int value;
    do {
        value = fgetc(file);
        if (value == '#') {
            while (value != '\n' && value != EOF) {
                value = fgetc(file);
            }
        }
    } while (value != EOF && isspace((unsigned char)value));
    if (value == EOF) {
        return false;
    }
    size_t length = 0;
    do {
        if (length + 1 >= capacity) {
            return false;
        }
        buffer[length++] = (char)value;
        value = fgetc(file);
    } while (value != EOF && !isspace((unsigned char)value));
    buffer[length] = '\0';
    return true;
}

static bool sr_token_u32(FILE *file, uint32_t *value) {
    char token[64];
    return sr_ppm_token(file, token, sizeof(token)) && sr_parse_u32(token, value);
}

static SrStatus sr_load_ppm(const char *path, SrAsset *asset,
                            SrDiagnostics *diag) {
    FILE *file = fopen(path, "rb");
    if (!file) {
        return SR_ERR_ASSET;
    }
    char magic[8];
    uint32_t width, height, max_value;
    if (!sr_ppm_token(file, magic, sizeof(magic)) ||
        (strcmp(magic, "P6") != 0 && strcmp(magic, "P3") != 0) ||
        !sr_token_u32(file, &width) || !sr_token_u32(file, &height) ||
        !sr_token_u32(file, &max_value) || max_value == 0 || max_value > 255) {
        fclose(file);
        return SR_ERR_ASSET;
    }
    if (width != asset->width || height != asset->height) {
        sr_diag_error(diag, asset->source_line, "image", "width/height",
                      "declared dimensions %ux%u do not match PPM %ux%u",
                      asset->width, asset->height, width, height);
        fclose(file);
        return SR_ERR_ASSET;
    }
    size_t pixels = (size_t)width * height;
    if ((height && pixels / height != width) || pixels > SIZE_MAX / 4) {
        fclose(file);
        return SR_ERR_MEMORY;
    }
    SrImage *image = sr_alloc(sizeof(*image));
    uint8_t *rgba = sr_alloc(pixels * 4);
    if (!image || !rgba) {
        free(image);
        free(rgba);
        fclose(file);
        return SR_ERR_MEMORY;
    }
    bool ok = true;
    if (strcmp(magic, "P6") == 0) {
        uint8_t *rgb = sr_alloc(pixels * 3);
        if (!rgb || fread(rgb, 3, pixels, file) != pixels) {
            ok = false;
        } else {
            for (size_t i = 0; i < pixels; ++i) {
                rgba[i * 4] = rgb[i * 3];
                rgba[i * 4 + 1] = rgb[i * 3 + 1];
                rgba[i * 4 + 2] = rgb[i * 3 + 2];
                rgba[i * 4 + 3] = 255;
            }
        }
        free(rgb);
    } else {
        for (size_t i = 0; ok && i < pixels; ++i) {
            for (size_t channel = 0; channel < 3; ++channel) {
                uint32_t sample;
                if (!sr_token_u32(file, &sample) || sample > max_value) {
                    ok = false;
                    break;
                }
                rgba[i * 4 + channel] =
                    (uint8_t)((sample * 255U + max_value / 2U) / max_value);
            }
            rgba[i * 4 + 3] = 255;
        }
    }
    fclose(file);
    if (!ok) {
        free(rgba);
        free(image);
        return SR_ERR_ASSET;
    }
    image->width = width;
    image->height = height;
    image->rgba = rgba;
    asset->decoded = image;
    return SR_OK;
}

static SrImage *sr_decode_ffmpeg(const char *path, SrAsset *asset,
                                 double timestamp, SrDiagnostics *diag) {
    int output[2];
    if (pipe(output) != 0) {
        return NULL;
    }
    pid_t pid = fork();
    if (pid < 0) {
        close(output[0]);
        close(output[1]);
        return NULL;
    }
    if (pid == 0) {
        dup2(output[1], STDOUT_FILENO);
        close(output[0]);
        close(output[1]);
        char scale[96];
        snprintf(scale, sizeof(scale), "scale=%u:%u:flags=lanczos", asset->width,
                 asset->height);
        char seek[64];
        snprintf(seek, sizeof(seek), "%.9f", timestamp);
        const char *argv[24];
        size_t n = 0;
        argv[n++] = "ffmpeg"; argv[n++] = "-nostdin";
        argv[n++] = "-v"; argv[n++] = "error";
        argv[n++] = "-i"; argv[n++] = path;
        if (timestamp >= 0.0) {
            argv[n++] = "-ss"; argv[n++] = seek;
        }
        argv[n++] = "-vf"; argv[n++] = scale;
        argv[n++] = "-frames:v"; argv[n++] = "1";
        argv[n++] = "-f"; argv[n++] = "rawvideo";
        argv[n++] = "-pix_fmt"; argv[n++] = "rgba";
        argv[n++] = "pipe:1"; argv[n] = NULL;
        execvp(argv[0], (char *const *)argv);
        _exit(127);
    }
    close(output[1]);
    size_t pixels = (size_t)asset->width * asset->height;
    if ((asset->height && pixels / asset->height != asset->width) ||
        pixels > SIZE_MAX / 4) {
        close(output[0]);
        waitpid(pid, NULL, 0);
        return NULL;
    }
    size_t size = pixels * 4;
    SrImage *image = sr_alloc(sizeof(*image));
    uint8_t *rgba = sr_alloc(size);
    if (!image || !rgba) {
        free(image);
        free(rgba);
        close(output[0]);
        waitpid(pid, NULL, 0);
        return NULL;
    }
    size_t received = 0;
    while (received < size) {
        ssize_t amount = read(output[0], rgba + received, size - received);
        if (amount > 0) {
            received += (size_t)amount;
        } else if (amount < 0 && errno == EINTR) {
            continue;
        } else {
            break;
        }
    }
    close(output[0]);
    int status = 0;
    pid_t waited;
    do {
        waited = waitpid(pid, &status, 0);
    } while (waited < 0 && errno == EINTR);
    if (received != size || waited < 0 || !WIFEXITED(status) ||
        WEXITSTATUS(status) != 0) {
        sr_diag_error(diag, asset->source_line,
                      asset->type == SR_ASSET_VIDEO ? "video" : "image", "src",
                      "FFmpeg could not decode '%s' as %ux%u RGBA", path,
                      asset->width, asset->height);
        free(image);
        free(rgba);
        return NULL;
    }
    image->width = asset->width;
    image->height = asset->height;
    image->rgba = rgba;
    return image;
}

SrStatus sr_assets_load(SrScene *scene, SrDiagnostics *diag) {
    for (size_t i = 0; i < scene->asset_count; ++i) {
        SrAsset *asset = &scene->assets[i];
        if (asset->decoded || asset->type == SR_ASSET_VIDEO ||
            asset->type == SR_ASSET_AUDIO) {
            continue;
        }
        if (asset->type == SR_ASSET_TEXT || asset->type == SR_ASSET_VECTOR) {
            SrStatus status = sr_procedural_asset(asset, diag);
            if (status != SR_OK) return status;
            continue;
        }
        char *path = sr_path_join(scene->base_dir, asset->source);
        if (!path) {
            return SR_ERR_MEMORY;
        }
        sr_diag_info(diag, "decoding shared asset '%s' from %s", asset->id, path);
        const char *extension = strrchr(path, '.');
        SrStatus status = extension && asset->type == SR_ASSET_IMAGE &&
                                  (strcmp(extension, ".ppm") == 0 ||
                                   strcmp(extension, ".pnm") == 0)
                              ? sr_load_ppm(path, asset, diag)
                              : SR_OK;
        if (status == SR_OK && !asset->decoded) {
            asset->decoded = sr_decode_ffmpeg(path, asset, -1.0, diag);
            status = asset->decoded ? SR_OK : SR_ERR_ASSET;
        }
        if (status != SR_OK && diag->errors == 0) {
            sr_diag_error(diag, asset->source_line, "image", "src",
                          "unable to decode '%s'", path);
        }
        free(path);
        if (status != SR_OK) {
            return status;
        }
    }
    return SR_OK;
}

void sr_assets_unload(SrScene *scene) {
    for (size_t i = 0; i < scene->asset_count; ++i) {
        SrImage *image = scene->assets[i].decoded;
        if (image) {
            free(image->rgba);
            free(image);
            scene->assets[i].decoded = NULL;
        }
        for (size_t j = 0; j < 4; ++j) {
            image = scene->assets[i].video_cache[j].image;
            if (image) {
                free(image->rgba);
                free(image);
                scene->assets[i].video_cache[j].image = NULL;
            }
        }
    }
}

SrImage *sr_asset_get_frame(SrScene *scene, SrAsset *asset, double source_time,
                            SrDiagnostics *diag) {
    if (!asset) return NULL;
    if (asset->type != SR_ASSET_VIDEO) return asset->decoded;
    if (!asset->fps_num || !asset->fps_den || asset->duration <= 0.0) {
        sr_diag_error(diag, asset->source_line, "video", "fps/duration",
                      "video metadata must be positive");
        return NULL;
    }
    double fps = (double)asset->fps_num / asset->fps_den;
    int64_t total = (int64_t)ceil(asset->duration * fps - 1e-12);
    int64_t index = (int64_t)floor(fmax(0.0, source_time) * fps + 1e-9);
    if (total > 0 && index >= total) index = total - 1;
    for (size_t i = 0; i < 4; ++i) {
        SrVideoCacheEntry *entry = &asset->video_cache[i];
        if (entry->image && entry->frame_index == index) {
            entry->age = ++asset->cache_clock;
            return entry->image;
        }
    }
    size_t victim = 0;
    for (size_t i = 1; i < 4; ++i) {
        if (!asset->video_cache[i].image ||
            asset->video_cache[i].age < asset->video_cache[victim].age)
            victim = i;
    }
    SrVideoCacheEntry *entry = &asset->video_cache[victim];
    if (entry->image) {
        free(entry->image->rgba);
        free(entry->image);
    }
    char *path = sr_path_join(scene->base_dir, asset->source);
    if (!path) return NULL;
    double timestamp = (double)index * asset->fps_den / asset->fps_num;
    entry->image = sr_decode_ffmpeg(path, asset, timestamp, diag);
    free(path);
    if (!entry->image) return NULL;
    entry->frame_index = index;
    entry->age = ++asset->cache_clock;
    return entry->image;
}
