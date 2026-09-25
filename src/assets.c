#define _POSIX_C_SOURCE 200809L
#include "scene_render/assets.h"
#include "scene_render/color.h"
#include "scene_render/mesh.h"
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

/* Wraps decoded 8-bit straight RGBA (consumed) as a blend-space image. */
static SrImage *sr_image_from_rgba8(const SrScene *scene, SrColorSpace space,
                                    uint8_t *rgba, uint32_t width,
                                    uint32_t height) {
    SrImage *image = sr_alloc(sizeof(*image));
    if (image && sr_color_image_from_rgba8(&scene->project, space, rgba,
                                           (size_t)width * 4, width, height,
                                           image) != SR_OK) {
        free(image);
        image = NULL;
    }
    free(rgba);
    return image;
}

static SrStatus sr_load_ppm(const SrScene *scene, const char *path,
                            SrAsset *asset, SrDiagnostics *diag) {
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
    uint8_t *rgba = sr_alloc(pixels * 4);
    if (!rgba) {
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
        return SR_ERR_ASSET;
    }
    asset->decoded = sr_image_from_rgba8(scene, asset->source_color_space,
                                         rgba, width, height);
    return asset->decoded ? SR_OK : SR_ERR_MEMORY;
}

static SrImage *sr_decode_ffmpeg(const SrScene *scene, const char *path,
                                 SrAsset *asset, double timestamp,
                                 SrDiagnostics *diag) {
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
    uint8_t *rgba = sr_alloc(size);
    if (!rgba) {
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
        free(rgba);
        return NULL;
    }
    return sr_image_from_rgba8(scene, asset->source_color_space, rgba,
                               asset->width, asset->height);
}

static bool sr_font_name_valid(const char *name) {
    if (!name || !*name) return false;
    for (const unsigned char *p=(const unsigned char *)name;*p;++p)
        if (!isalnum(*p) && *p!=' ' && *p!='_' && *p!='-' && *p!='.')
            return false;
    return true;
}

static char *sr_filter_escape(const char *text) {
    size_t length=0;
    for(const char *p=text;*p;++p)length+=(*p==':'||*p=='\\'||*p=='\'')?2:1;
    char *result=sr_alloc(length+1);if(!result)return NULL;char *out=result;
    for(const char *p=text;*p;++p){if(*p==':'||*p=='\\'||*p=='\'')*out++='\\';*out++=*p;}
    *out='\0';return result;
}

static SrImage *sr_render_text_ffmpeg(const SrScene *scene,SrAsset *asset,
                                      SrDiagnostics *diag){
    char temporary[]="/tmp/scene-render-text-XXXXXX";int text_fd=mkstemp(temporary);
    if(text_fd<0)return NULL;
    size_t length=strlen(asset->text),written=0;
    while(written<length){ssize_t amount=write(text_fd,asset->text+written,length-written);
        if(amount>0)written+=(size_t)amount;else if(amount<0&&errno==EINTR)continue;else break;}
    close(text_fd);if(written!=length){unlink(temporary);return NULL;}
    char *font_path=asset->font_file?sr_path_join(scene->base_dir,asset->font_file):NULL;
    char *font_value=sr_filter_escape(font_path?font_path:asset->font_family);
    char *text_path=sr_filter_escape(temporary);free(font_path);
    if(!font_value||!text_path){free(font_value);free(text_path);unlink(temporary);return NULL;}
    char input[128],filter[2048],color[64];
    snprintf(input,sizeof(input),"color=c=black@0.0:s=%ux%u:r=1,format=rgba",
             asset->width,asset->height);
    snprintf(color,sizeof(color),"0x%02X%02X%02X@%.9f",
             (unsigned)lrint(asset->color.r*255.0),
             (unsigned)lrint(asset->color.g*255.0),
             (unsigned)lrint(asset->color.b*255.0),asset->color.a);
    snprintf(filter,sizeof(filter),
             "drawtext=%s='%s':textfile='%s':fontsize=%.9g:fontcolor=%s:"
             "x=0:y=0:text_shaping=1:fix_bounds=1",
             asset->font_file?"fontfile":"font",font_value,text_path,
             asset->text_size,color);
    free(font_value);free(text_path);
    int output[2];if(pipe(output)!=0){unlink(temporary);return NULL;}
    pid_t pid=fork();if(pid<0){close(output[0]);close(output[1]);unlink(temporary);return NULL;}
    if(pid==0){dup2(output[1],STDOUT_FILENO);close(output[0]);close(output[1]);
        const char *argv[]={"ffmpeg","-nostdin","-v","error","-f","lavfi",
            "-i",input,"-vf",filter,"-frames:v","1","-f","rawvideo",
            "-pix_fmt","rgba","pipe:1",NULL};
        execvp(argv[0],(char *const *)argv);_exit(127);}
    close(output[1]);size_t pixels=(size_t)asset->width*asset->height;
    if ((asset->height && pixels / asset->height != asset->width) ||
        pixels > SIZE_MAX / 4) {
        close(output[0]);waitpid(pid,NULL,0);unlink(temporary);return NULL;
    }
    size_t size=pixels*4;uint8_t *rgba=sr_alloc(size);
    if(!rgba){close(output[0]);waitpid(pid,NULL,0);unlink(temporary);return NULL;}
    size_t received=0;while(received<size){ssize_t amount=read(output[0],rgba+received,size-received);
        if(amount>0)received+=(size_t)amount;else if(amount<0&&errno==EINTR)continue;else break;}
    close(output[0]);int status=0;pid_t waited;do{waited=waitpid(pid,&status,0);}while(waited<0&&errno==EINTR);
    unlink(temporary);if(received!=size||waited<0||!WIFEXITED(status)||WEXITSTATUS(status)!=0){
        sr_diag_error(diag,asset->source_line,"text",asset->font_file?"fontFile":"font",
                      "FFmpeg drawtext could not shape the UTF-8 text");free(rgba);return NULL;}
    /* Text colors are working-space values, like every XML color. */
    return sr_image_from_rgba8(scene,scene->project.working_color_space,rgba,
                               asset->width,asset->height);
}

SrStatus sr_assets_load(SrScene *scene, SrDiagnostics *diag) {
    for (size_t i = 0; i < scene->asset_count; ++i) {
        SrAsset *asset = &scene->assets[i];
        if (asset->decoded || asset->type == SR_ASSET_VIDEO ||
            asset->type == SR_ASSET_AUDIO) {
            continue;
        }
        if(asset->type==SR_ASSET_MESH){char *path=sr_path_join(scene->base_dir,asset->source);
            if(!path)return SR_ERR_MEMORY;
            SrStatus status=sr_mesh_load_obj(path,&asset->mesh,asset->source_line,diag);
            free(path);if(status!=SR_OK)return status;continue;}
        if (asset->type == SR_ASSET_TEXT) {
            if (!sr_font_name_valid(asset->font_family)) {
                sr_diag_error(diag,asset->source_line,"text","font",
                              "font family contains unsupported characters");
                return SR_ERR_ASSET;
            }
            asset->decoded=sr_render_text_ffmpeg(scene,asset,diag);
            if(!asset->decoded)return SR_ERR_ASSET;
            continue;
        }
        if (asset->type == SR_ASSET_VECTOR) {
            SrStatus status = sr_procedural_asset(&scene->project, asset, diag);
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
                              ? sr_load_ppm(scene, path, asset, diag)
                              : SR_OK;
        if (status == SR_OK && !asset->decoded) {
            asset->decoded = sr_decode_ffmpeg(scene, path, asset, -1.0, diag);
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
            free(image->px);
            free(image);
            scene->assets[i].decoded = NULL;
        }
        for (size_t j = 0; j < 4; ++j) {
            image = scene->assets[i].video_cache[j].image;
            if (image) {
                free(image->px);
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
        free(entry->image->px);
        free(entry->image);
        entry->image = NULL;
    }
    char *path = sr_path_join(scene->base_dir, asset->source);
    if (!path) return NULL;
    double timestamp = (double)index * asset->fps_den / asset->fps_num;
    entry->image = sr_decode_ffmpeg(scene, path, asset, timestamp, diag);
    free(path);
    if (!entry->image) return NULL;
    entry->frame_index = index;
    entry->age = ++asset->cache_clock;
    return entry->image;
}
